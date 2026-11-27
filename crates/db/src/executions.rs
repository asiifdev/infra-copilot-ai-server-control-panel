//! Proposals with an action, and their one execution record (ADR 0037).
//!
//! The rules live here, in one transaction each, so no caller can skip one:
//! * an action is attached when the proposal is created and never changes (a trigger refuses it);
//! * an execution starts only for an `approved`, unexpired proposal that has an action;
//! * a proposal has at most one execution (`UNIQUE`), so a failed one is never repeated by itself
//!   and two concurrent requests cannot both run;
//! * the start and the finish are audited in the same transaction as the change they record.

use crate::audit::{ActorKind, AuditOutcome, NewAuditEntry, append_audit_in_tx};
use crate::conversations::{
    NewProposal, ProposalRow, ProposalStatus, create_proposal, get_proposal,
};
use crate::error::{Error, Result};
use crate::ids::{
    ExecutionBackupId, ExecutionId, ExecutionRollbackId, NodeId, OrgId, ProposalId, UserId,
};
use crate::{new_uuid, now_utc};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

/// Longest output kept per execution.
pub const MAX_OUTPUT_CHARS: usize = 4000;

/// Creates a pending proposal that carries an action and a target node, in one transaction.
///
/// # Errors
/// [`Error::NotFound`] when the node is not in this org; constraint violations as in
/// [`create_proposal`].
pub fn create_action_proposal(
    conn: &Connection,
    org: &OrgId,
    new: &NewProposal<'_>,
    node: &NodeId,
    action_json: &str,
) -> Result<ProposalId> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let known: Option<i64> = tx
        .query_row(
            "SELECT 1 FROM nodes WHERE org_id = ?1 AND id = ?2",
            params![org.as_str(), node.as_str()],
            |row| row.get(0),
        )
        .optional()?;
    if known.is_none() {
        return Err(Error::NotFound);
    }
    let id = create_proposal(&tx, org, new)?;
    tx.execute(
        "UPDATE proposals SET action_json = ?3, node_id = ?4 WHERE org_id = ?1 AND id = ?2",
        params![org.as_str(), id.as_str(), action_json, node.as_str()],
    )?;
    tx.commit()?;
    Ok(id)
}

/// How an execution ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionStatus {
    /// Started, not finished.
    Running,
    /// Ran and verified.
    Succeeded,
    /// Failed, or did not verify.
    Failed,
}

impl ExecutionStatus {
    /// Stored text form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }

    fn parse(text: &str) -> Self {
        match text {
            "running" => Self::Running,
            "succeeded" => Self::Succeeded,
            _ => Self::Failed,
        }
    }
}

/// A stored execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionRow {
    /// Id.
    pub id: ExecutionId,
    /// The proposal it ran.
    pub proposal_id: ProposalId,
    /// Target node.
    pub node_id: NodeId,
    /// Status.
    pub status: ExecutionStatus,
    /// Whether the check after the action passed.
    pub verified: bool,
    /// Fixed code of what happened.
    pub reason: Option<String>,
    /// Redacted, bounded output.
    pub output: String,
    /// Start time.
    pub started_at: String,
    /// Finish time.
    pub finished_at: Option<String>,
}

const EXECUTION_COLUMNS: &str =
    "id, proposal_id, node_id, status, verified, reason, output, started_at, finished_at";

fn map_execution(row: &rusqlite::Row<'_>) -> rusqlite::Result<ExecutionRow> {
    let status: String = row.get(3)?;
    Ok(ExecutionRow {
        id: ExecutionId::from_db(row.get(0)?),
        proposal_id: ProposalId::from_db(row.get(1)?),
        node_id: NodeId::from_db(row.get(2)?),
        status: ExecutionStatus::parse(&status),
        verified: row.get::<_, i64>(4)? != 0,
        reason: row.get(5)?,
        output: row.get(6)?,
        started_at: row.get(7)?,
        finished_at: row.get(8)?,
    })
}

/// The execution of a proposal, if one was started.
///
/// # Errors
/// Database failure.
pub fn get_execution_for_proposal(
    conn: &Connection,
    org: &OrgId,
    proposal: &ProposalId,
) -> Result<Option<ExecutionRow>> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {EXECUTION_COLUMNS} FROM executions WHERE org_id = ?1 AND proposal_id = ?2"
            ),
            params![org.as_str(), proposal.as_str()],
            map_execution,
        )
        .optional()?)
}

/// What [`begin_execution`] decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BeginOutcome {
    /// The execution row exists in state `running`; the caller must run the action and then call
    /// [`finish_execution`].
    Started {
        /// The new execution.
        execution: ExecutionId,
        /// The proposal, as approved.
        proposal: Box<ProposalRow>,
    },
    /// The proposal is not `approved` (it is pending, rejected or expired).
    NotApproved(ProposalStatus),
    /// It was approved but not run before it expired; it is now marked `expired`.
    Expired,
    /// It has no action to run.
    NoAction,
    /// It was run already (or is running now).
    AlreadyRun,
}

/// Claims the one execution of an approved proposal and audits the start.
///
/// # Errors
/// [`Error::NotFound`] when the proposal is missing or in another org; database failure.
pub fn begin_execution(
    conn: &Connection,
    org: &OrgId,
    proposal: &ProposalId,
    user: &UserId,
) -> Result<BeginOutcome> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let row = get_proposal(&tx, org, proposal)?;
    if matches!(
        row.status,
        ProposalStatus::Executed | ProposalStatus::Failed
    ) {
        return Ok(BeginOutcome::AlreadyRun);
    }
    if row.status != ProposalStatus::Approved {
        return Ok(BeginOutcome::NotApproved(row.status));
    }
    let (Some(_), Some(node)) = (row.action_json.as_ref(), row.node_id.as_ref()) else {
        return Ok(BeginOutcome::NoAction);
    };
    let expired: bool = tx.query_row(
        "SELECT expires_at <= strftime('%Y-%m-%dT%H:%M:%fZ', 'now') FROM proposals \
         WHERE org_id = ?1 AND id = ?2",
        params![org.as_str(), proposal.as_str()],
        |r| r.get(0),
    )?;
    if expired {
        tx.execute(
            "UPDATE proposals SET status = 'expired' WHERE org_id = ?1 AND id = ?2 AND status = 'approved'",
            params![org.as_str(), proposal.as_str()],
        )?;
        tx.commit()?;
        return Ok(BeginOutcome::Expired);
    }
    let id = new_uuid(&tx)?;
    let inserted = tx.execute(
        "INSERT INTO executions (id, org_id, proposal_id, node_id, started_by) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            id,
            org.as_str(),
            proposal.as_str(),
            node.as_str(),
            user.as_str()
        ],
    );
    match inserted {
        Ok(_) => {}
        Err(rusqlite::Error::SqliteFailure(e, _))
            if e.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            return Ok(BeginOutcome::AlreadyRun);
        }
        Err(other) => return Err(other.into()),
    }
    let subject = format!(
        r#"{{"proposal":"{}","execution":"{id}","tier":{},"args_hash":"{}"}}"#,
        proposal.as_str(),
        row.tier,
        row.args_hash
    );
    append_audit_in_tx(
        &tx,
        org,
        &NewAuditEntry {
            actor_kind: ActorKind::User,
            actor_id: Some(user.as_str()),
            action: "proposal.execute.start",
            node_id: Some(node),
            subject_json: &subject,
            result: AuditOutcome::Success,
        },
    )?;
    tx.commit()?;
    Ok(BeginOutcome::Started {
        execution: ExecutionId::from_db(id),
        proposal: Box::new(row),
    })
}

/// Records how a started execution ended, sets the proposal to `executed` or `failed`, and
/// audits it, in one transaction. Output is cut to [`MAX_OUTPUT_CHARS`].
///
/// # Errors
/// [`Error::NotFound`] when the execution is missing, in another org, or already finished.
pub fn finish_execution(
    conn: &Connection,
    org: &OrgId,
    execution: &ExecutionId,
    success: bool,
    verified: bool,
    reason: &str,
    output: &str,
) -> Result<()> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let (proposal, node): (String, String) = tx
        .query_row(
            "SELECT proposal_id, node_id FROM executions \
             WHERE org_id = ?1 AND id = ?2 AND status = 'running'",
            params![org.as_str(), execution.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    let status = if success {
        ExecutionStatus::Succeeded
    } else {
        ExecutionStatus::Failed
    };
    let output: String = output.chars().take(MAX_OUTPUT_CHARS).collect();
    tx.execute(
        "UPDATE executions SET status = ?3, verified = ?4, reason = ?5, output = ?6, finished_at = ?7 \
         WHERE org_id = ?1 AND id = ?2",
        params![
            org.as_str(),
            execution.as_str(),
            status.as_str(),
            i64::from(verified),
            reason,
            output,
            now_utc(&tx)?
        ],
    )?;
    tx.execute(
        "UPDATE proposals SET status = ?3 WHERE org_id = ?1 AND id = ?2 AND status = 'approved'",
        params![
            org.as_str(),
            proposal,
            if success { "executed" } else { "failed" }
        ],
    )?;
    let subject = format!(
        r#"{{"proposal":"{proposal}","execution":"{}","status":"{}","verified":{verified},"reason":"{}"}}"#,
        execution.as_str(),
        status.as_str(),
        // A fixed code from the executor; anything else is neutralized rather than trusted.
        if reason.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
            reason
        } else {
            "other"
        }
    );
    let node_id = NodeId::from_db(node);
    append_audit_in_tx(
        &tx,
        org,
        &NewAuditEntry {
            actor_kind: ActorKind::System,
            actor_id: Some("executor"),
            action: "proposal.execute.finish",
            node_id: Some(&node_id),
            subject_json: &subject,
            result: if success {
                AuditOutcome::Success
            } else {
                AuditOutcome::Failure
            },
        },
    )?;
    tx.commit()?;
    Ok(())
}

/// Marks executions that were `running` when the process stopped as failed (`interrupted`), so
/// their proposals do not stay approved-and-stuck. Returns how many.
///
/// # Errors
/// Database failure.
pub fn fail_interrupted_executions(conn: &Connection, org: &OrgId) -> Result<usize> {
    let ids: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT id FROM executions \
             WHERE org_id = ?1 AND status = 'running' \
             AND id NOT IN (SELECT execution_id FROM agent_jobs WHERE org_id = ?1 AND status IN ('queued', 'running')) \
             ORDER BY started_at",
        )?;
        let rows = stmt.query_map(params![org.as_str()], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<Vec<String>>>()?
    };
    for id in &ids {
        finish_execution(
            conn,
            org,
            &ExecutionId::from_db(id.clone()),
            false,
            false,
            "interrupted",
            "",
        )?;
    }
    Ok(ids.len())
}

// ---- backups and rollbacks -------------------------------------------------------------------

/// A stored pre-execution backup snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionBackupRow {
    /// Id.
    pub id: ExecutionBackupId,
    /// Execution it belongs to.
    pub execution_id: ExecutionId,
    /// Storage reference (e.g. path to the backup artifact).
    pub storage_ref: String,
    /// SHA-256 hex digest (64 chars) of the backup payload.
    pub sha256: String,
    /// Creation time.
    pub created_at: String,
}

fn map_backup(row: &rusqlite::Row<'_>) -> rusqlite::Result<ExecutionBackupRow> {
    Ok(ExecutionBackupRow {
        id: ExecutionBackupId::from_db(row.get(0)?),
        execution_id: ExecutionId::from_db(row.get(1)?),
        storage_ref: row.get(2)?,
        sha256: row.get(3)?,
        created_at: row.get(4)?,
    })
}

/// Records a pre-execution backup for `execution` and appends an audit entry, in one transaction.
///
/// # Errors
/// [`Error::NotFound`] when the execution is missing or in another org; database failure.
pub fn record_execution_backup(
    conn: &Connection,
    org: &OrgId,
    execution: &ExecutionId,
    storage_ref: &str,
    sha256: &str,
) -> Result<ExecutionBackupId> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let (proposal, node): (String, String) = tx
        .query_row(
            "SELECT proposal_id, node_id FROM executions WHERE org_id = ?1 AND id = ?2",
            params![org.as_str(), execution.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    let id = new_uuid(&tx)?;
    tx.execute(
        "INSERT INTO execution_backups (id, org_id, execution_id, storage_ref, sha256) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, org.as_str(), execution.as_str(), storage_ref, sha256],
    )?;
    let subject = format!(
        r#"{{"proposal":"{proposal}","execution":"{}","backup":"{id}","sha256":"{sha256}"}}"#,
        execution.as_str()
    );
    let node_id = NodeId::from_db(node);
    append_audit_in_tx(
        &tx,
        org,
        &NewAuditEntry {
            actor_kind: ActorKind::System,
            actor_id: Some("executor"),
            action: "proposal.execute.backup",
            node_id: Some(&node_id),
            subject_json: &subject,
            result: AuditOutcome::Success,
        },
    )?;
    tx.commit()?;
    Ok(ExecutionBackupId::from_db(id))
}

/// The backup of an execution, if one was recorded.
///
/// # Errors
/// Database failure.
pub fn get_execution_backup(
    conn: &Connection,
    org: &OrgId,
    execution: &ExecutionId,
) -> Result<Option<ExecutionBackupRow>> {
    Ok(conn
        .query_row(
            "SELECT id, execution_id, storage_ref, sha256, created_at \
             FROM execution_backups WHERE org_id = ?1 AND execution_id = ?2",
            params![org.as_str(), execution.as_str()],
            map_backup,
        )
        .optional()?)
}

/// A stored execution rollback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionRollbackRow {
    /// Id.
    pub id: ExecutionRollbackId,
    /// Execution it rolls back.
    pub execution_id: ExecutionId,
    /// User who started the rollback.
    pub started_by: UserId,
    /// Status (`running`, `succeeded`, `failed`).
    pub status: ExecutionStatus,
    /// Whether the post-rollback verification passed.
    pub verified: bool,
    /// Fixed code of what happened.
    pub reason: Option<String>,
    /// Redacted, bounded output.
    pub output: String,
    /// Start time.
    pub started_at: String,
    /// Finish time.
    pub finished_at: Option<String>,
}

fn map_rollback(row: &rusqlite::Row<'_>) -> rusqlite::Result<ExecutionRollbackRow> {
    let status: String = row.get(3)?;
    Ok(ExecutionRollbackRow {
        id: ExecutionRollbackId::from_db(row.get(0)?),
        execution_id: ExecutionId::from_db(row.get(1)?),
        started_by: UserId::from_db(row.get(2)?),
        status: ExecutionStatus::parse(&status),
        verified: row.get::<_, i64>(4)? != 0,
        reason: row.get(5)?,
        output: row.get(6)?,
        started_at: row.get(7)?,
        finished_at: row.get(8)?,
    })
}

/// The rollback of an execution, if one was started.
///
/// # Errors
/// Database failure.
pub fn get_rollback_for_execution(
    conn: &Connection,
    org: &OrgId,
    execution: &ExecutionId,
) -> Result<Option<ExecutionRollbackRow>> {
    Ok(conn
        .query_row(
            "SELECT id, execution_id, started_by, status, verified, reason, output, started_at, finished_at \
             FROM execution_rollbacks WHERE org_id = ?1 AND execution_id = ?2",
            params![org.as_str(), execution.as_str()],
            map_rollback,
        )
        .optional()?)
}

/// What [`begin_rollback`] decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BeginRollbackOutcome {
    /// The rollback row exists in state `running`; the caller must run the rollback and then call
    /// [`finish_rollback`].
    Started {
        /// The new rollback.
        rollback: ExecutionRollbackId,
        /// The execution being rolled back.
        execution: Box<ExecutionRow>,
        /// The proposal that was executed.
        proposal: Box<ProposalRow>,
        /// The backup snapshot to restore from.
        backup: ExecutionBackupRow,
    },
    /// The proposal has not been executed.
    NoExecution,
    /// The execution is still running.
    ExecutionStillRunning,
    /// No backup exists for this execution.
    NoBackup,
    /// A rollback was already started or finished for this execution.
    AlreadyRolledBack,
}

/// Claims the one rollback of an executed proposal's execution and audits the start.
///
/// # Errors
/// [`Error::NotFound`] when the proposal is missing or in another org; database failure.
pub fn begin_rollback(
    conn: &Connection,
    org: &OrgId,
    proposal: &ProposalId,
    user: &UserId,
) -> Result<BeginRollbackOutcome> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let prop = get_proposal(&tx, org, proposal)?;
    let Some(exec) = get_execution_for_proposal(&tx, org, proposal)? else {
        return Ok(BeginRollbackOutcome::NoExecution);
    };
    if exec.status == ExecutionStatus::Running {
        return Ok(BeginRollbackOutcome::ExecutionStillRunning);
    }
    let Some(backup) = get_execution_backup(&tx, org, &exec.id)? else {
        return Ok(BeginRollbackOutcome::NoBackup);
    };
    let id = new_uuid(&tx)?;
    let inserted = tx.execute(
        "INSERT INTO execution_rollbacks (id, org_id, execution_id, started_by) \
         VALUES (?1, ?2, ?3, ?4)",
        params![id, org.as_str(), exec.id.as_str(), user.as_str()],
    );
    match inserted {
        Ok(_) => {}
        Err(rusqlite::Error::SqliteFailure(e, _))
            if e.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            return Ok(BeginRollbackOutcome::AlreadyRolledBack);
        }
        Err(other) => return Err(other.into()),
    }
    let subject = format!(
        r#"{{"proposal":"{}","execution":"{}","rollback":"{id}"}}"#,
        proposal.as_str(),
        exec.id.as_str()
    );
    append_audit_in_tx(
        &tx,
        org,
        &NewAuditEntry {
            actor_kind: ActorKind::User,
            actor_id: Some(user.as_str()),
            action: "proposal.rollback.start",
            node_id: Some(&exec.node_id),
            subject_json: &subject,
            result: AuditOutcome::Success,
        },
    )?;
    tx.commit()?;
    Ok(BeginRollbackOutcome::Started {
        rollback: ExecutionRollbackId::from_db(id),
        execution: Box::new(exec),
        proposal: Box::new(prop),
        backup,
    })
}

/// Records how a started rollback ended and audits it, in one transaction. Output is cut to
/// [`MAX_OUTPUT_CHARS`].
///
/// # Errors
/// [`Error::NotFound`] when the rollback is missing, in another org, or already finished.
pub fn finish_rollback(
    conn: &Connection,
    org: &OrgId,
    rollback: &ExecutionRollbackId,
    success: bool,
    verified: bool,
    reason: &str,
    output: &str,
) -> Result<()> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let (execution_id, proposal, node): (String, String, String) = tx
        .query_row(
            "SELECT r.execution_id, e.proposal_id, e.node_id \
             FROM execution_rollbacks r \
             JOIN executions e ON e.org_id = r.org_id AND e.id = r.execution_id \
             WHERE r.org_id = ?1 AND r.id = ?2 AND r.status = 'running'",
            params![org.as_str(), rollback.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    let status = if success {
        ExecutionStatus::Succeeded
    } else {
        ExecutionStatus::Failed
    };
    let output: String = output.chars().take(MAX_OUTPUT_CHARS).collect();
    tx.execute(
        "UPDATE execution_rollbacks \
         SET status = ?3, verified = ?4, reason = ?5, output = ?6, finished_at = ?7 \
         WHERE org_id = ?1 AND id = ?2",
        params![
            org.as_str(),
            rollback.as_str(),
            status.as_str(),
            i64::from(verified),
            reason,
            output,
            now_utc(&tx)?
        ],
    )?;
    let safe_reason = if reason.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
        reason
    } else {
        "other"
    };
    let subject = format!(
        r#"{{"proposal":"{proposal}","execution":"{execution_id}","rollback":"{}","status":"{}","verified":{verified},"reason":"{safe_reason}"}}"#,
        rollback.as_str(),
        status.as_str()
    );
    let node_id = NodeId::from_db(node);
    append_audit_in_tx(
        &tx,
        org,
        &NewAuditEntry {
            actor_kind: ActorKind::System,
            actor_id: Some("executor"),
            action: "proposal.rollback.finish",
            node_id: Some(&node_id),
            subject_json: &subject,
            result: if success {
                AuditOutcome::Success
            } else {
                AuditOutcome::Failure
            },
        },
    )?;
    tx.commit()?;
    Ok(())
}
