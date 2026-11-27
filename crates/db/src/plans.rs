//! Migration plans and steps (docs/design.md 3.4). Every function takes an [`OrgId`] and filters on
//! it; a plan or node of another org is [`Error::NotFound`], and the schema itself refuses
//! cross-org links (composite foreign keys).
//!
//! The plan model and its rules live in the `plan` crate. This crate stores the spec and the step
//! documents as opaque JSON text and never parses them (the same split as inventories), so it does
//! not depend on `plan`.

use crate::error::{Error, Result};
use crate::ids::{NodeId, OrgId, PlanId};
use crate::new_uuid;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

text_enum!(
    /// Where a plan stands.
    PlanStatus {
        /// Being written; the only state in which its content may change.
        Draft => "draft",
        /// Read by a person.
        Reviewed => "reviewed",
        /// Approved to run.
        Approved => "approved",
        /// Executing.
        Running => "running",
        /// Stopped part-way, may continue.
        Paused => "paused",
        /// Finished.
        Done => "done",
        /// Given up.
        Aborted => "aborted",
    }
);

text_enum!(
    /// What a step does (mirrors `plan::StepKind`).
    PlanStepKind {
        /// Read what runs on a machine.
        Inventory => "inventory",
        /// Take the snapshot.
        Backup => "backup",
        /// Move the snapshot.
        Transfer => "transfer",
        /// Restore on the target.
        Restore => "restore",
        /// Verify the target.
        Verify => "verify",
        /// Copy what changed since the snapshot.
        Delta => "delta",
        /// Move traffic.
        Cutover => "cutover",
        /// Verify after cutover.
        PostVerify => "post_verify",
        /// Shut the old server down (tier 4).
        DecommissionSource => "decommission_source",
    }
);

text_enum!(
    /// Where a step stands.
    PlanStepStatus {
        /// Not started.
        Pending => "pending",
        /// Proposed, waiting for a decision.
        Proposed => "proposed",
        /// Approved.
        Approved => "approved",
        /// Executing.
        Running => "running",
        /// Finished well.
        Succeeded => "succeeded",
        /// Finished badly.
        Failed => "failed",
        /// Not needed.
        Skipped => "skipped",
        /// Undone.
        RolledBack => "rolled_back",
    }
);

impl PlanStatus {
    /// Whether a plan may move from `self` to `next`. `done` and `aborted` are final.
    #[must_use]
    pub const fn can_become(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Draft, Self::Reviewed | Self::Aborted)
                | (Self::Reviewed, Self::Draft | Self::Approved | Self::Aborted)
                | (
                    Self::Approved,
                    Self::Reviewed | Self::Running | Self::Aborted
                )
                | (Self::Running, Self::Paused | Self::Done | Self::Aborted)
                | (Self::Paused, Self::Running | Self::Aborted)
        )
    }
}

/// A plan to store (or the new content of a draft).
#[derive(Debug)]
pub struct NewPlan<'a> {
    /// Display name.
    pub name: &'a str,
    /// The machine data comes from.
    pub source_node: &'a NodeId,
    /// The machine data goes to.
    pub target_node: &'a NodeId,
    /// The spec as JSON text (validated by the caller; the database only checks it is JSON).
    pub spec_json: &'a str,
}

/// A step to store.
#[derive(Debug)]
pub struct NewPlanStep<'a> {
    /// Number inside the plan.
    pub seq: u32,
    /// Kind.
    pub kind: PlanStepKind,
    /// Step document as JSON text.
    pub spec_json: &'a str,
    /// Steps that must finish first.
    pub depends_on: &'a [u32],
}

/// A stored plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanRow {
    /// Id.
    pub id: PlanId,
    /// Name.
    pub name: String,
    /// Source node.
    pub source_node_id: NodeId,
    /// Target node.
    pub target_node_id: NodeId,
    /// Status.
    pub status: PlanStatus,
    /// The spec as JSON text.
    pub spec_json: String,
    /// When it was created.
    pub created_at: String,
}

/// A stored step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanStepRow {
    /// Number inside the plan.
    pub seq: u32,
    /// Kind.
    pub kind: PlanStepKind,
    /// Step document as JSON text.
    pub spec_json: String,
    /// Status.
    pub status: PlanStepStatus,
    /// Steps that must finish first.
    pub depends_on: Vec<u32>,
}

fn node_in_org(conn: &Connection, org: &OrgId, node: &NodeId) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM nodes WHERE org_id = ?1 AND id = ?2)",
        params![org.as_str(), node.as_str()],
        |row| row.get(0),
    )?)
}

pub(crate) fn check_new(conn: &Connection, org: &OrgId, new: &NewPlan<'_>) -> Result<()> {
    if new.source_node == new.target_node {
        return Err(Error::Invalid("plan source and target must differ"));
    }
    // Explicit checks first, so another org's node is NotFound rather than a constraint error that
    // would reveal it exists; the composite foreign keys still back this up.
    if !node_in_org(conn, org, new.source_node)? || !node_in_org(conn, org, new.target_node)? {
        return Err(Error::NotFound);
    }
    Ok(())
}

fn depends_json(depends_on: &[u32]) -> String {
    let list: Vec<String> = depends_on.iter().map(u32::to_string).collect();
    format!("[{}]", list.join(","))
}

fn parse_depends(text: &str) -> Result<Vec<u32>> {
    let inner = text
        .trim()
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .ok_or(Error::Invalid("depends_on"))?;
    if inner.trim().is_empty() {
        return Ok(Vec::new());
    }
    inner
        .split(',')
        .map(|part| {
            part.trim()
                .parse()
                .map_err(|_| Error::Invalid("depends_on"))
        })
        .collect()
}

pub(crate) fn insert_steps(
    tx: &rusqlite::Transaction<'_>,
    org: &OrgId,
    plan: &str,
    steps: &[NewPlanStep<'_>],
) -> Result<()> {
    let mut stmt = tx.prepare(
        "INSERT INTO migration_steps (org_id, plan_id, seq, kind, spec, depends_on) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for step in steps {
        stmt.execute(params![
            org.as_str(),
            plan,
            step.seq,
            step.kind.as_str(),
            step.spec_json,
            depends_json(step.depends_on)
        ])?;
    }
    Ok(())
}

/// Inserts the plan row (always `draft`: the column default is the only status a new plan gets).
pub(crate) fn insert_plan_row(
    conn: &Connection,
    org: &OrgId,
    id: &str,
    new: &NewPlan<'_>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO migration_plans (id, org_id, name, source_node_id, target_node_id, spec) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            id,
            org.as_str(),
            new.name,
            new.source_node.as_str(),
            new.target_node.as_str(),
            new.spec_json
        ],
    )?;
    Ok(())
}

/// Stores a draft plan and its steps atomically.
///
/// # Errors
/// [`Error::NotFound`] if a node is not in this org; [`Error::Invalid`] when source and target are
/// the same node; a constraint violation for malformed JSON or duplicate step numbers.
pub fn create_plan(
    conn: &mut Connection,
    org: &OrgId,
    new: &NewPlan<'_>,
    steps: &[NewPlanStep<'_>],
) -> Result<PlanId> {
    let id = new_uuid(conn)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    check_new(&tx, org, new)?;
    insert_plan_row(&tx, org, &id, new)?;
    insert_steps(&tx, org, &id, steps)?;
    tx.commit()?;
    Ok(PlanId::from_db(id))
}

pub(crate) const PLAN_COLUMNS: &str =
    "id, name, source_node_id, target_node_id, status, spec, created_at";

pub(crate) fn map_plan(row: &rusqlite::Row<'_>) -> rusqlite::Result<[String; 7]> {
    Ok([
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
    ])
}

pub(crate) fn build_plan(raw: [String; 7]) -> Result<PlanRow> {
    let [id, name, source, target, status, spec, created_at] = raw;
    Ok(PlanRow {
        id: PlanId::from_db(id),
        name,
        source_node_id: NodeId::from_db(source),
        target_node_id: NodeId::from_db(target),
        status: PlanStatus::parse(&status)?,
        spec_json: spec,
        created_at,
    })
}

/// One plan by id.
///
/// # Errors
/// [`Error::NotFound`] if it is missing or belongs to another org.
pub fn get_plan(conn: &Connection, org: &OrgId, id: &PlanId) -> Result<PlanRow> {
    let raw = conn
        .query_row(
            &format!("SELECT {PLAN_COLUMNS} FROM migration_plans WHERE org_id = ?1 AND id = ?2"),
            params![org.as_str(), id.as_str()],
            map_plan,
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    build_plan(raw)
}

/// The plans of an org, newest first, at most `limit`.
///
/// # Errors
/// Database failure.
pub fn list_plans(conn: &Connection, org: &OrgId, limit: u32) -> Result<Vec<PlanRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {PLAN_COLUMNS} FROM migration_plans WHERE org_id = ?1 \
         ORDER BY created_at DESC, id DESC LIMIT ?2"
    ))?;
    let rows = stmt.query_map(params![org.as_str(), limit], map_plan)?;
    rows.map(|raw| build_plan(raw?)).collect()
}

/// The steps of a plan, in step-number order.
///
/// # Errors
/// [`Error::NotFound`] if the plan is missing or belongs to another org.
pub fn list_plan_steps(conn: &Connection, org: &OrgId, plan: &PlanId) -> Result<Vec<PlanStepRow>> {
    get_plan(conn, org, plan)?;
    let mut stmt = conn.prepare(
        "SELECT seq, kind, spec, status, depends_on FROM migration_steps \
         WHERE org_id = ?1 AND plan_id = ?2 ORDER BY seq",
    )?;
    let rows = stmt.query_map(params![org.as_str(), plan.as_str()], |row| {
        Ok((
            row.get::<_, u32>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;
    rows.map(|row| {
        let (seq, kind, spec, status, depends) = row?;
        Ok(PlanStepRow {
            seq,
            kind: PlanStepKind::parse(&kind)?,
            spec_json: spec,
            status: PlanStepStatus::parse(&status)?,
            depends_on: parse_depends(&depends)?,
        })
    })
    .collect()
}

/// Replaces the content of a plan (name, nodes, spec and all steps) atomically. Only a plan in
/// `draft` may change; once it has been reviewed it must go back to draft first.
///
/// # Errors
/// [`Error::NotFound`] for a plan or node outside this org; [`Error::Invalid`] when the plan is not
/// a draft or the nodes are equal.
pub fn replace_plan_content(
    conn: &mut Connection,
    org: &OrgId,
    id: &PlanId,
    new: &NewPlan<'_>,
    steps: &[NewPlanStep<'_>],
) -> Result<()> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let current: String = tx
        .query_row(
            "SELECT status FROM migration_plans WHERE org_id = ?1 AND id = ?2",
            params![org.as_str(), id.as_str()],
            |row| row.get(0),
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    if PlanStatus::parse(&current)? != PlanStatus::Draft {
        return Err(Error::Invalid("plan is not a draft"));
    }
    check_new(&tx, org, new)?;
    tx.execute(
        "UPDATE migration_plans SET name = ?3, source_node_id = ?4, target_node_id = ?5, spec = ?6 \
         WHERE org_id = ?1 AND id = ?2",
        params![
            org.as_str(),
            id.as_str(),
            new.name,
            new.source_node.as_str(),
            new.target_node.as_str(),
            new.spec_json
        ],
    )?;
    tx.execute(
        "DELETE FROM migration_steps WHERE org_id = ?1 AND plan_id = ?2",
        params![org.as_str(), id.as_str()],
    )?;
    insert_steps(&tx, org, id.as_str(), steps)?;
    tx.commit()?;
    Ok(())
}

/// Moves a plan to a new status along the allowed transitions (see [`PlanStatus::can_become`]).
///
/// # Errors
/// [`Error::NotFound`] if the plan is not in this org; [`Error::Invalid`] for a forbidden
/// transition.
pub fn set_plan_status(
    conn: &mut Connection,
    org: &OrgId,
    id: &PlanId,
    next: PlanStatus,
) -> Result<()> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let current: String = tx
        .query_row(
            "SELECT status FROM migration_plans WHERE org_id = ?1 AND id = ?2",
            params![org.as_str(), id.as_str()],
            |row| row.get(0),
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    if !PlanStatus::parse(&current)?.can_become(next) {
        return Err(Error::Invalid("plan status transition"));
    }
    tx.execute(
        "UPDATE migration_plans SET status = ?3 WHERE org_id = ?1 AND id = ?2",
        params![org.as_str(), id.as_str(), next.as_str()],
    )?;
    tx.commit()?;
    Ok(())
}

/// Sets the status of one step. The database refuses to move `decommission_source` forward while
/// any other step has not succeeded (trigger in migration 0008).
///
/// # Errors
/// [`Error::NotFound`] if the plan or step is not in this org; a constraint violation from the
/// trigger.
pub fn set_plan_step_status(
    conn: &Connection,
    org: &OrgId,
    plan: &PlanId,
    seq: u32,
    status: PlanStepStatus,
) -> Result<()> {
    let n = conn.execute(
        "UPDATE migration_steps SET status = ?4 WHERE org_id = ?1 AND plan_id = ?2 AND seq = ?3",
        params![org.as_str(), plan.as_str(), seq, status.as_str()],
    )?;
    if n == 0 { Err(Error::NotFound) } else { Ok(()) }
}

/// Approves a `draft` or `reviewed` plan (moving `draft -> reviewed -> approved` when needed) and
/// appends an audit entry, in one transaction.
///
/// # Errors
/// [`Error::NotFound`] if the plan is not in this org; [`Error::Invalid`] if it has no steps or
/// is already `approved`, `running`, `paused`, `done`, or `aborted`.
pub fn approve_plan(
    conn: &Connection,
    org: &OrgId,
    id: &PlanId,
    actor_kind: crate::audit::ActorKind,
    actor_id: Option<&str>,
) -> Result<PlanStatus> {
    let tx = conn.unchecked_transaction()?;
    let current: String = tx
        .query_row(
            "SELECT status FROM migration_plans WHERE org_id = ?1 AND id = ?2",
            params![org.as_str(), id.as_str()],
            |row| row.get(0),
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    let status = PlanStatus::parse(&current)?;
    if !matches!(status, PlanStatus::Draft | PlanStatus::Reviewed) {
        return Err(Error::Invalid("plan cannot be approved"));
    }
    let step_count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM migration_steps WHERE org_id = ?1 AND plan_id = ?2",
        params![org.as_str(), id.as_str()],
        |row| row.get(0),
    )?;
    if step_count <= 0 {
        return Err(Error::Invalid("plan has no steps"));
    }
    if status == PlanStatus::Draft {
        tx.execute(
            "UPDATE migration_plans SET status = 'reviewed' WHERE org_id = ?1 AND id = ?2",
            params![org.as_str(), id.as_str()],
        )?;
    }
    tx.execute(
        "UPDATE migration_plans SET status = 'approved' WHERE org_id = ?1 AND id = ?2",
        params![org.as_str(), id.as_str()],
    )?;
    let subject = format!(
        r#"{{"plan":"{}","from":"{}","to":"approved"}}"#,
        id.as_str(),
        status.as_str()
    );
    crate::audit::append_audit_in_tx(
        &tx,
        org,
        &crate::audit::NewAuditEntry {
            actor_kind,
            actor_id,
            action: "plan.approve",
            node_id: None,
            subject_json: &subject,
            result: crate::audit::AuditOutcome::Success,
        },
    )?;
    tx.commit()?;
    Ok(PlanStatus::Approved)
}

/// Atomically claims an `approved` (or `paused`) plan for execution (`running`) and audits the
/// start.
///
/// # Errors
/// [`Error::NotFound`] if the plan is not in this org; [`Error::Invalid`] if it is not `approved`
/// or `paused`.
pub fn begin_plan_run(
    conn: &Connection,
    org: &OrgId,
    id: &PlanId,
    actor_kind: crate::audit::ActorKind,
    actor_id: Option<&str>,
) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    let current: String = tx
        .query_row(
            "SELECT status FROM migration_plans WHERE org_id = ?1 AND id = ?2",
            params![org.as_str(), id.as_str()],
            |row| row.get(0),
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    let status = PlanStatus::parse(&current)?;
    if !matches!(status, PlanStatus::Approved | PlanStatus::Paused) {
        return Err(Error::Invalid("plan is not approved"));
    }
    let n = tx.execute(
        "UPDATE migration_plans SET status = 'running' \
         WHERE org_id = ?1 AND id = ?2 AND status IN ('approved', 'paused')",
        params![org.as_str(), id.as_str()],
    )?;
    if n == 0 {
        return Err(Error::Invalid("plan is not approved"));
    }
    let subject = format!(
        r#"{{"plan":"{}","from":"{}"}}"#,
        id.as_str(),
        status.as_str()
    );
    crate::audit::append_audit_in_tx(
        &tx,
        org,
        &crate::audit::NewAuditEntry {
            actor_kind,
            actor_id,
            action: "plan.run.start",
            node_id: None,
            subject_json: &subject,
            result: crate::audit::AuditOutcome::Success,
        },
    )?;
    tx.commit()?;
    Ok(())
}

/// Finishes a `running` plan run (`done`, `paused`, or `aborted`) and audits it.
///
/// # Errors
/// [`Error::NotFound`] if missing; [`Error::Invalid`] if not `running` or `next` is invalid.
pub fn finish_plan_run(
    conn: &Connection,
    org: &OrgId,
    id: &PlanId,
    next: PlanStatus,
    actor_kind: crate::audit::ActorKind,
    actor_id: Option<&str>,
) -> Result<()> {
    if !matches!(
        next,
        PlanStatus::Done | PlanStatus::Paused | PlanStatus::Aborted
    ) {
        return Err(Error::Invalid("plan run finish status"));
    }
    let tx = conn.unchecked_transaction()?;
    let n = tx.execute(
        "UPDATE migration_plans SET status = ?3 \
         WHERE org_id = ?1 AND id = ?2 AND status = 'running'",
        params![org.as_str(), id.as_str(), next.as_str()],
    )?;
    if n == 0 {
        return Err(Error::Invalid("plan is not running"));
    }
    let subject = format!(
        r#"{{"plan":"{}","status":"{}"}}"#,
        id.as_str(),
        next.as_str()
    );
    crate::audit::append_audit_in_tx(
        &tx,
        org,
        &crate::audit::NewAuditEntry {
            actor_kind,
            actor_id,
            action: "plan.run.finish",
            node_id: None,
            subject_json: &subject,
            result: if next == PlanStatus::Done {
                crate::audit::AuditOutcome::Success
            } else {
                crate::audit::AuditOutcome::Failure
            },
        },
    )?;
    tx.commit()?;
    Ok(())
}

/// Updates one `migration_steps` row and appends an audit entry, in one transaction.
///
/// # Errors
/// [`Error::NotFound`] if missing; constraint violation if the `decommission_source` trigger
/// refuses the transition.
pub fn transition_plan_step(
    conn: &Connection,
    org: &OrgId,
    plan: &PlanId,
    seq: u32,
    status: PlanStepStatus,
    actor_kind: crate::audit::ActorKind,
    actor_id: Option<&str>,
) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    let n = tx.execute(
        "UPDATE migration_steps SET status = ?4 WHERE org_id = ?1 AND plan_id = ?2 AND seq = ?3",
        params![org.as_str(), plan.as_str(), seq, status.as_str()],
    )?;
    if n == 0 {
        return Err(Error::NotFound);
    }
    let subject = format!(
        r#"{{"plan":"{}","seq":{seq},"status":"{}"}}"#,
        plan.as_str(),
        status.as_str()
    );
    crate::audit::append_audit_in_tx(
        &tx,
        org,
        &crate::audit::NewAuditEntry {
            actor_kind,
            actor_id,
            action: "plan.step.update",
            node_id: None,
            subject_json: &subject,
            result: if status == PlanStepStatus::Failed {
                crate::audit::AuditOutcome::Failure
            } else {
                crate::audit::AuditOutcome::Success
            },
        },
    )?;
    tx.commit()?;
    Ok(())
}
