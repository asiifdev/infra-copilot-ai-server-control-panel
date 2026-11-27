//! Outbound-only remote agent tokens, queued jobs, and managed `.test` local sites.
//! All queries are strictly scoped by `org_id`.

use crate::audit::{ActorKind, AuditOutcome, NewAuditEntry, append_audit_in_tx};
use crate::error::{Error, Result};
use crate::executions::MAX_OUTPUT_CHARS;
use crate::ids::{
    AgentJobId, AgentTokenId, ExecutionId, LocalSiteId, NodeId, OrgId, ProposalId, UserId,
};
use crate::{new_uuid, now_utc};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

text_enum!(
    /// Lifecycle state of a queued agent job.
    AgentJobStatus {
        /// Waiting for the remote agent to poll.
        Queued => "queued",
        /// Claimed by the remote agent.
        Running => "running",
        /// Finished successfully and verified.
        Succeeded => "succeeded",
        /// Failed or rejected.
        Failed => "failed",
    }
);

text_enum!(
    /// Status of a managed `.test` local site entry.
    LocalSiteStatus {
        /// Config proposed only (not written to a managed directory).
        Proposed => "proposed",
        /// Config written to the managed sites directory.
        Active => "active",
    }
);

/// A remote agent job record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentJobRow {
    /// Job id.
    pub id: AgentJobId,
    /// Organization id.
    pub org_id: OrgId,
    /// Target node id.
    pub node_id: NodeId,
    /// Proposal id.
    pub proposal_id: ProposalId,
    /// Execution id.
    pub execution_id: ExecutionId,
    /// Strict action JSON.
    pub action_json: String,
    /// Canonical SHA-256 hash of the action.
    pub args_hash: String,
    /// Current job status.
    pub status: AgentJobStatus,
    /// Whether post-execution verification passed.
    pub verified: bool,
    /// Reason code.
    pub reason: String,
    /// Redacted, bounded output.
    pub output: String,
    /// Creation timestamp.
    pub created_at: String,
    /// Claim timestamp.
    pub claimed_at: Option<String>,
    /// Finish timestamp.
    pub finished_at: Option<String>,
}

/// Authenticated agent identity derived from a bearer token hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedAgent {
    /// Token id.
    pub token_id: AgentTokenId,
    /// Organization id.
    pub org_id: OrgId,
    /// Node id.
    pub node_id: NodeId,
}

/// A managed `.test` local site entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalSiteRow {
    /// Site id.
    pub id: LocalSiteId,
    /// Organization id.
    pub org_id: OrgId,
    /// Domain ending in `.test`.
    pub domain: String,
    /// Local upstream port.
    pub upstream_port: u16,
    /// Managed config file path or virtual reference.
    pub config_path: String,
    /// Site status (`proposed` | `active`).
    pub status: LocalSiteStatus,
    /// Creation timestamp.
    pub created_at: String,
}

fn bounded_output(raw: &str) -> String {
    raw.chars().take(MAX_OUTPUT_CHARS).collect()
}

fn bounded_reason(raw: &str) -> String {
    let trimmed: String = raw.chars().take(40).collect();
    if trimmed.is_empty() {
        "unknown".to_owned()
    } else {
        trimmed
    }
}

fn row_to_agent_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentJobRow> {
    let status_s: String = row.get(7)?;
    let status = AgentJobStatus::parse(&status_s).map_err(|_| rusqlite::Error::InvalidQuery)?;
    let verified_i: i64 = row.get(8)?;
    Ok(AgentJobRow {
        id: AgentJobId::from_db(row.get(0)?),
        org_id: OrgId::from_db(row.get(1)?),
        node_id: NodeId::from_db(row.get(2)?),
        proposal_id: ProposalId::from_db(row.get(3)?),
        execution_id: ExecutionId::from_db(row.get(4)?),
        action_json: row.get(5)?,
        args_hash: row.get(6)?,
        status,
        verified: verified_i != 0,
        reason: row.get(9)?,
        output: row.get(10)?,
        created_at: row.get(11)?,
        claimed_at: row.get(12)?,
        finished_at: row.get(13)?,
    })
}

/// Revokes any active agent tokens for `(org, node)` and stores a new `token_hash` (64-char hex).
///
/// # Errors
/// [`Error::NotFound`] if the node does not belong to `org`.
pub fn rotate_agent_token(
    conn: &Connection,
    org: &OrgId,
    node: &NodeId,
    actor: &UserId,
    token_hash: &str,
) -> Result<AgentTokenId> {
    if token_hash.len() != 64 {
        return Err(Error::Invalid("token_hash"));
    }
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let exists: Option<i64> = tx
        .query_row(
            "SELECT 1 FROM nodes WHERE org_id = ?1 AND id = ?2",
            params![org.as_str(), node.as_str()],
            |r| r.get(0),
        )
        .optional()?;
    if exists.is_none() {
        return Err(Error::NotFound);
    }
    let now = now_utc(&tx)?;
    tx.execute(
        "UPDATE agent_tokens SET revoked_at = ?3 \
         WHERE org_id = ?1 AND node_id = ?2 AND revoked_at IS NULL",
        params![org.as_str(), node.as_str(), now],
    )?;
    let id = new_uuid(&tx)?;
    tx.execute(
        "INSERT INTO agent_tokens (id, org_id, node_id, token_hash, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, org.as_str(), node.as_str(), token_hash, now],
    )?;
    append_audit_in_tx(
        &tx,
        org,
        &NewAuditEntry {
            actor_kind: ActorKind::User,
            actor_id: Some(actor.as_str()),
            action: "agent.token.rotate",
            node_id: Some(node),
            subject_json: "{}",
            result: AuditOutcome::Success,
        },
    )?;
    tx.commit()?;
    Ok(AgentTokenId::from_db(id))
}

/// Looks up an active (non-revoked) agent token by its SHA-256 hex digest.
///
/// # Errors
/// Database failure.
pub fn authenticate_agent_token(
    conn: &Connection,
    token_hash: &str,
) -> Result<Option<AuthenticatedAgent>> {
    if token_hash.len() != 64 {
        return Ok(None);
    }
    conn.query_row(
        "SELECT id, org_id, node_id FROM agent_tokens \
         WHERE token_hash = ?1 AND revoked_at IS NULL",
        params![token_hash],
        |row| {
            Ok(AuthenticatedAgent {
                token_id: AgentTokenId::from_db(row.get(0)?),
                org_id: OrgId::from_db(row.get(1)?),
                node_id: NodeId::from_db(row.get(2)?),
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

/// Enqueues a remote agent job for an already-started `execution`.
///
/// # Errors
/// Constraint or database errors.
pub fn enqueue_agent_job(
    conn: &Connection,
    org: &OrgId,
    node: &NodeId,
    proposal: &ProposalId,
    execution: &ExecutionId,
    action_json: &str,
    args_hash: &str,
) -> Result<AgentJobId> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let id = new_uuid(&tx)?;
    let now = now_utc(&tx)?;
    tx.execute(
        "INSERT INTO agent_jobs \
         (id, org_id, node_id, proposal_id, execution_id, action_json, args_hash, status, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'queued', ?8)",
        params![
            id,
            org.as_str(),
            node.as_str(),
            proposal.as_str(),
            execution.as_str(),
            action_json,
            args_hash,
            now
        ],
    )?;
    let subject = format!(r#"{{"execution_id":"{}"}}"#, execution.as_str());
    append_audit_in_tx(
        &tx,
        org,
        &NewAuditEntry {
            actor_kind: ActorKind::System,
            actor_id: None,
            action: "agent.job.enqueue",
            node_id: Some(node),
            subject_json: &subject,
            result: AuditOutcome::Success,
        },
    )?;
    tx.commit()?;
    Ok(AgentJobId::from_db(id))
}

/// Atomically claims the oldest `queued` job for `(org, node)` (`queued` -> `running`).
///
/// # Errors
/// Database failure.
pub fn claim_next_agent_job(
    conn: &Connection,
    org: &OrgId,
    node: &NodeId,
) -> Result<Option<AgentJobRow>> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let next_id: Option<String> = tx
        .query_row(
            "SELECT id FROM agent_jobs \
             WHERE org_id = ?1 AND node_id = ?2 AND status = 'queued' \
             ORDER BY created_at ASC LIMIT 1",
            params![org.as_str(), node.as_str()],
            |r| r.get(0),
        )
        .optional()?;
    let Some(id) = next_id else {
        return Ok(None);
    };
    let now = now_utc(&tx)?;
    let updated = tx.execute(
        "UPDATE agent_jobs SET status = 'running', claimed_at = ?3 \
         WHERE org_id = ?1 AND id = ?2 AND status = 'queued'",
        params![org.as_str(), id, now],
    )?;
    if updated == 0 {
        return Ok(None);
    }
    let row = tx.query_row(
        "SELECT id, org_id, node_id, proposal_id, execution_id, action_json, args_hash, \
                status, verified, reason, output, created_at, claimed_at, finished_at \
         FROM agent_jobs WHERE org_id = ?1 AND id = ?2",
        params![org.as_str(), id],
        row_to_agent_job,
    )?;
    tx.commit()?;
    Ok(Some(row))
}

/// Completion payload reported by a remote agent for a claimed job.
#[derive(Debug, Clone, Copy)]
pub struct AgentJobResult<'a> {
    /// Whether the action succeeded.
    pub succeeded: bool,
    /// Whether post-execution verification passed.
    pub verified: bool,
    /// Fixed reason code.
    pub reason: &'a str,
    /// Redacted, bounded command output.
    pub output: &'a str,
}

/// Completes a `running` agent job, updates its linked `executions` row, and appends an audit entry.
///
/// # Errors
/// [`Error::NotFound`] if the job does not exist for `(org, node)` or [`Error::Invalid`] if not `running`.
pub fn complete_agent_job(
    conn: &Connection,
    org: &OrgId,
    node: &NodeId,
    job_id: &AgentJobId,
    result: AgentJobResult<'_>,
) -> Result<AgentJobRow> {
    let AgentJobResult {
        succeeded,
        verified,
        reason,
        output,
    } = result;
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let job: Option<(String, String)> = tx
        .query_row(
            "SELECT execution_id, status FROM agent_jobs \
             WHERE org_id = ?1 AND node_id = ?2 AND id = ?3",
            params![org.as_str(), node.as_str(), job_id.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((exec_id, current_status)) = job else {
        return Err(Error::NotFound);
    };
    if current_status != "running" {
        return Err(Error::Invalid("job_not_running"));
    }

    let now = now_utc(&tx)?;
    let status = if succeeded { "succeeded" } else { "failed" };
    let clean_reason = bounded_reason(reason);
    let clean_output = bounded_output(output);

    tx.execute(
        "UPDATE agent_jobs \
         SET status = ?3, verified = ?4, reason = ?5, output = ?6, finished_at = ?7 \
         WHERE org_id = ?1 AND id = ?2",
        params![
            org.as_str(),
            job_id.as_str(),
            status,
            i64::from(verified),
            clean_reason,
            clean_output,
            now
        ],
    )?;

    tx.execute(
        "UPDATE executions \
         SET status = ?3, verified = ?4, reason = ?5, output = ?6, finished_at = ?7 \
         WHERE org_id = ?1 AND id = ?2 AND status = 'running'",
        params![
            org.as_str(),
            exec_id,
            status,
            i64::from(verified),
            clean_reason,
            clean_output,
            now
        ],
    )?;

    let subject = format!(
        r#"{{"execution_id":"{exec_id}","job_id":"{}","status":"{status}","verified":{verified},"reason":"{clean_reason}"}}"#,
        job_id.as_str()
    );
    append_audit_in_tx(
        &tx,
        org,
        &NewAuditEntry {
            actor_kind: ActorKind::Agent,
            actor_id: Some(node.as_str()),
            action: "agent.job.complete",
            node_id: Some(node),
            subject_json: &subject,
            result: if succeeded {
                AuditOutcome::Success
            } else {
                AuditOutcome::Failure
            },
        },
    )?;

    let row = tx.query_row(
        "SELECT id, org_id, node_id, proposal_id, execution_id, action_json, args_hash, \
                status, verified, reason, output, created_at, claimed_at, finished_at \
         FROM agent_jobs WHERE org_id = ?1 AND id = ?2",
        params![org.as_str(), job_id.as_str()],
        row_to_agent_job,
    )?;
    tx.commit()?;
    Ok(row)
}

/// Returns the agent job associated with `execution_id` in `org`, if any.
///
/// # Errors
/// Database failure.
pub fn get_agent_job_for_execution(
    conn: &Connection,
    org: &OrgId,
    execution_id: &ExecutionId,
) -> Result<Option<AgentJobRow>> {
    conn.query_row(
        "SELECT id, org_id, node_id, proposal_id, execution_id, action_json, args_hash, \
                status, verified, reason, output, created_at, claimed_at, finished_at \
         FROM agent_jobs WHERE org_id = ?1 AND execution_id = ?2",
        params![org.as_str(), execution_id.as_str()],
        row_to_agent_job,
    )
    .optional()
    .map_err(Into::into)
}

/// Records or proposes a managed `.test` local site in `org` and appends an audit entry.
///
/// # Errors
/// Constraint violation on duplicate domain or database failure.
pub fn create_local_site(
    conn: &Connection,
    org: &OrgId,
    actor: &UserId,
    domain: &str,
    upstream_port: u16,
    config_path: &str,
    status: LocalSiteStatus,
) -> Result<LocalSiteRow> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let id = new_uuid(&tx)?;
    let now = now_utc(&tx)?;
    tx.execute(
        "INSERT INTO local_sites (id, org_id, domain, upstream_port, config_path, status, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            id,
            org.as_str(),
            domain,
            i64::from(upstream_port),
            config_path,
            status.as_str(),
            now
        ],
    )?;
    let subject = format!(
        r#"{{"site_id":"{id}","domain":"{domain}","upstream_port":{upstream_port},"status":"{}"}}"#,
        status.as_str()
    );
    append_audit_in_tx(
        &tx,
        org,
        &NewAuditEntry {
            actor_kind: ActorKind::User,
            actor_id: Some(actor.as_str()),
            action: "site.create",
            node_id: None,
            subject_json: &subject,
            result: AuditOutcome::Success,
        },
    )?;
    tx.commit()?;
    Ok(LocalSiteRow {
        id: LocalSiteId::from_db(id),
        org_id: org.clone(),
        domain: domain.to_owned(),
        upstream_port,
        config_path: config_path.to_owned(),
        status,
        created_at: now,
    })
}

/// Lists all managed `.test` local sites in `org`.
///
/// # Errors
/// Database failure.
pub fn list_local_sites(conn: &Connection, org: &OrgId) -> Result<Vec<LocalSiteRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, org_id, domain, upstream_port, config_path, status, created_at \
         FROM local_sites WHERE org_id = ?1 ORDER BY domain ASC",
    )?;
    let rows = stmt
        .query_map(params![org.as_str()], |row| {
            let port_i: i64 = row.get(3)?;
            let status_s: String = row.get(5)?;
            let status =
                LocalSiteStatus::parse(&status_s).map_err(|_| rusqlite::Error::InvalidQuery)?;
            let upstream_port = u16::try_from(port_i).map_err(|_| rusqlite::Error::InvalidQuery)?;
            Ok(LocalSiteRow {
                id: LocalSiteId::from_db(row.get(0)?),
                org_id: OrgId::from_db(row.get(1)?),
                domain: row.get(2)?,
                upstream_port,
                config_path: row.get(4)?,
                status,
                created_at: row.get(6)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversations::NewProposal;
    use crate::executions::{begin_execution, create_action_proposal};
    use crate::nodes::{NewNode, NodeAccess, NodeKind, create_node};
    use crate::sha256_hex;
    use crate::{
        ProposalDecision, create_conversation, create_org, create_user, decide_proposal,
        open_in_memory,
    };

    #[test]
    fn agent_token_rotation_job_claim_and_completion_are_org_isolated() -> Result<()> {
        let mut conn = open_in_memory()?;
        crate::migrate(&mut conn)?;
        let org_a = create_org(&conn, "Org A", "org-a", "free")?.id;
        let org_b = create_org(&conn, "Org B", "org-b", "free")?.id;
        let user_a = create_user(&conn, "a@example.test", "A", None)?.id;
        crate::add_member(&conn, &org_a, &user_a, crate::Role::Admin)?;
        let node_a = create_node(
            &conn,
            &org_a,
            &NewNode {
                name: "agent-node",
                kind: NodeKind::Vps,
                access: NodeAccess::Agent,
                labels_json: "{}",
            },
        )?;

        let h1 = sha256_hex(b"token-1");
        let h2 = sha256_hex(b"token-2");
        rotate_agent_token(&conn, &org_a, &node_a, &user_a, &h1)?;
        assert!(authenticate_agent_token(&conn, &h1)?.is_some());
        rotate_agent_token(&conn, &org_a, &node_a, &user_a, &h2)?;
        assert!(authenticate_agent_token(&conn, &h1)?.is_none());
        let auth = authenticate_agent_token(&conn, &h2)?;
        assert!(auth.is_some());

        let conv = create_conversation(&conn, &org_a, &user_a, "t", None)?;
        let action_json = r#"{"action":"restart_service","manager":"systemd","name":"nginx"}"#;
        let args_hash = sha256_hex(action_json.as_bytes());
        let prop = create_action_proposal(
            &conn,
            &org_a,
            &NewProposal {
                conversation: Some(&conv),
                tool: "restart_service",
                tier: 2,
                title: "restart nginx",
                rationale: "test",
                suggested_action: "restart nginx",
                impact: "brief downtime",
                rule_id: None,
                subject_key: None,
                args_hash: &args_hash,
            },
            &node_a,
            action_json,
        )?;
        decide_proposal(&conn, &org_a, &prop, ProposalDecision::Approve, &user_a)?;
        let crate::BeginOutcome::Started { execution, .. } =
            begin_execution(&conn, &org_a, &prop, &user_a)?
        else {
            return Err(Error::Invalid("expected started"));
        };
        let job_id = enqueue_agent_job(
            &conn,
            &org_a,
            &node_a,
            &prop,
            &execution,
            action_json,
            &args_hash,
        )?;
        assert!(claim_next_agent_job(&conn, &org_b, &node_a)?.is_none());
        let claimed = claim_next_agent_job(&conn, &org_a, &node_a)?;
        assert_eq!(claimed.as_ref().map(|j| &j.id), Some(&job_id));
        assert!(claim_next_agent_job(&conn, &org_a, &node_a)?.is_none());

        let done = complete_agent_job(
            &conn,
            &org_a,
            &node_a,
            &job_id,
            AgentJobResult {
                succeeded: true,
                verified: true,
                reason: "ok",
                output: "restarted",
            },
        )?;
        assert_eq!(done.status, AgentJobStatus::Succeeded);
        assert!(done.verified);
        Ok(())
    }

    #[test]
    fn local_site_registry_is_org_isolated() -> Result<()> {
        let mut conn = open_in_memory()?;
        crate::migrate(&mut conn)?;
        let org_a = create_org(&conn, "Org A", "org-a", "free")?.id;
        let org_b = create_org(&conn, "Org B", "org-b", "free")?.id;
        let user_a = create_user(&conn, "a@example.test", "A", None)?.id;
        crate::add_member(&conn, &org_a, &user_a, crate::Role::Admin)?;
        create_local_site(
            &conn,
            &org_a,
            &user_a,
            "billing.test",
            3000,
            "/tmp/billing.test.conf",
            LocalSiteStatus::Active,
        )?;
        assert_eq!(list_local_sites(&conn, &org_a)?.len(), 1);
        assert_eq!(list_local_sites(&conn, &org_b)?.len(), 0);
        Ok(())
    }
}
