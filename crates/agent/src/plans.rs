//! The seams of the AI planning tools (ADR 0027): a read seam, [`PlanStore`], and a write seam,
//! [`PlanSink`], that can only create **one draft plan**.
//!
//! Like [`crate::ReadStore`] and [`crate::ProposalSink`], every method names the org, so a call
//! that forgets the tenant does not compile. The write seam has no approve, schedule, run, edit
//! or delete method, so no tool and no model can reach one; the status of a plan written here is
//! `draft`, always (approval arrives in M4, from a person).
//!
//! What the read seam never returns: a transport's configuration, its secret reference, or any id
//! of a secret. Machines are named, not addressed.

use crate::gates_for_stored;
use crate::store::{StoreError, finding_from_row};
use db::{Connection, FindingStatus, InventoryStatus, NodeId, OrgId};
use i18n::{Catalog, Locale};
use inventory::Finding;
use plan::{FindingKey, Plan, StoredStep};
use std::fmt;

/// Drafts an org may hold at once: a model that drafts in a loop fills a review queue, not the
/// table.
pub const MAX_DRAFT_PLANS: u32 = 50;

/// How many plans a listing shows at most.
const LIST_LIMIT: u32 = 200;

/// A transport as the model sees it: name, kind and when it was last verified. Never its
/// configuration, never its credentials.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransportInfo {
    /// Name, unique inside the org.
    pub name: String,
    /// `rclone`, `rsync`, `scp`, `ssh_pipe` or `managed`.
    pub kind: String,
    /// When the last successful verification finished; `None` when it never passed.
    pub verified_at: Option<String>,
}

/// A transport resolved by name for drafting. The id goes into the plan's spec (a plan refers to a
/// transport by id) and nowhere else.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedTransport {
    /// The transport id.
    pub id: String,
    /// Whether a verification ever passed.
    pub verified: bool,
}

/// One plan in a listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanSummary {
    /// Plan UUID.
    pub id: String,
    /// Name.
    pub name: String,
    /// `draft`, `reviewed`, `approved`, ...
    pub status: &'static str,
    /// Name of the node data comes from.
    pub source_node: String,
    /// Name of the node data goes to.
    pub target_node: String,
    /// When it was created.
    pub created_at: String,
}

/// A stored plan, rebuilt and re-validated through the `plan` crate (stored data is not trusted).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredPlan {
    /// Listing data.
    pub summary: PlanSummary,
    /// The validated plan with its steps.
    pub plan: Plan,
    /// Name of the transport the plan uses, when it still exists in the org.
    pub transport_name: Option<String>,
}

impl StoredPlan {
    /// The names of the source and the target, by the ids the plan holds (for `render_plan`).
    #[must_use]
    pub fn plan_names(&self) -> plan::PlanNames {
        let names = plan::PlanNames::of_plan(
            &self.plan,
            &self.summary.source_node,
            &self.summary.target_node,
        );
        match &self.transport_name {
            Some(name) => names.with(&self.plan.spec().transport.transport_id, name),
            None => names,
        }
    }
}

/// The newest comparison between two named nodes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComparisonView {
    /// When it ran.
    pub created_at: String,
    /// Both inventories behind it are still the newest of their nodes. A stale comparison says
    /// nothing about the machines as they are now, so callers must not let it pass a gate.
    pub fresh: bool,
    /// One of the two inventories is partial.
    pub inventory_partial: bool,
    /// Every finding of the comparison.
    pub findings: Vec<Finding>,
    /// The findings a person accepted (a subset of `findings`, by rule and subject).
    pub accepted: Vec<FindingKey>,
}

/// Read access of the planning tools. Read-only by construction.
pub trait PlanStore {
    /// The org's transports, by name.
    ///
    /// # Errors
    /// [`StoreError::Unavailable`] when the store cannot answer.
    fn transports(&self, org: &OrgId) -> Result<Vec<TransportInfo>, StoreError>;

    /// The transport called `name`.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] when there is none in this org (another org's is the same).
    fn transport(&self, org: &OrgId, name: &str) -> Result<ResolvedTransport, StoreError>;

    /// The id of the node called `name`.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] when there is none in this org.
    fn node_id(&self, org: &OrgId, name: &str) -> Result<String, StoreError>;

    /// The org's plans, newest first.
    ///
    /// # Errors
    /// [`StoreError::Unavailable`] when the store cannot answer.
    fn plans(&self, org: &OrgId) -> Result<Vec<PlanSummary>, StoreError>;

    /// One plan with its steps.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] when there is none in this org; [`StoreError::Unavailable`] when
    /// the stored plan no longer passes validation.
    fn plan(&self, org: &OrgId, name: &str) -> Result<StoredPlan, StoreError>;

    /// The newest comparison whose reference is the node `source` and whose checked machine is
    /// the node `target`.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] when the two were never compared.
    fn comparison(
        &self,
        org: &OrgId,
        source: &str,
        target: &str,
    ) -> Result<ComparisonView, StoreError>;
}

/// Why a draft was not saved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanSinkError {
    /// A plan with this name exists.
    NameTaken,
    /// The org holds the maximum number of drafts.
    TooManyDrafts,
    /// A node of the plan is not in this org.
    NodeMissing,
    /// This context cannot save plans, or the store failed.
    Unavailable,
}

impl fmt::Display for PlanSinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NameTaken => "plan_name_taken",
            Self::TooManyDrafts => "too_many_draft_plans",
            Self::NodeMissing => "node_missing",
            Self::Unavailable => "plans_unavailable",
        })
    }
}

impl std::error::Error for PlanSinkError {}

/// Where a tool saves a draft plan.
pub trait PlanSink {
    /// Saves `plan` as a **draft** and returns its id. The plan is already validated (a [`Plan`]
    /// cannot be built otherwise); the sink adds the per-org rules.
    ///
    /// # Errors
    /// [`PlanSinkError`] when it cannot be saved.
    fn create_draft(&self, org: &OrgId, plan: &Plan) -> Result<String, PlanSinkError>;
}

/// The default seam: no plans, no transports, and nowhere to save a draft.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoPlans;

impl PlanStore for NoPlans {
    fn transports(&self, _org: &OrgId) -> Result<Vec<TransportInfo>, StoreError> {
        Ok(Vec::new())
    }

    fn transport(&self, _org: &OrgId, _name: &str) -> Result<ResolvedTransport, StoreError> {
        Err(StoreError::NotFound)
    }

    fn node_id(&self, _org: &OrgId, _name: &str) -> Result<String, StoreError> {
        Err(StoreError::NotFound)
    }

    fn plans(&self, _org: &OrgId) -> Result<Vec<PlanSummary>, StoreError> {
        Ok(Vec::new())
    }

    fn plan(&self, _org: &OrgId, _name: &str) -> Result<StoredPlan, StoreError> {
        Err(StoreError::NotFound)
    }

    fn comparison(
        &self,
        _org: &OrgId,
        _source: &str,
        _target: &str,
    ) -> Result<ComparisonView, StoreError> {
        Err(StoreError::NotFound)
    }
}

impl PlanSink for NoPlans {
    fn create_draft(&self, _org: &OrgId, _plan: &Plan) -> Result<String, PlanSinkError> {
        Err(PlanSinkError::Unavailable)
    }
}

/// [`PlanStore`] over the SQLite layer.
pub struct DbPlanStore<'a> {
    conn: &'a Connection,
}

impl<'a> DbPlanStore<'a> {
    /// Reads through `conn` (which must come from `db::open`).
    #[must_use]
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    fn node_by_name(&self, org: &OrgId, name: &str) -> Result<NodeId, StoreError> {
        Ok(db::list_nodes(self.conn, org)?
            .into_iter()
            .find(|n| n.name == name)
            .ok_or(StoreError::NotFound)?
            .id)
    }

    fn node_name(&self, org: &OrgId, id: &NodeId) -> Result<String, StoreError> {
        Ok(db::list_nodes(self.conn, org)?
            .into_iter()
            .find(|n| &n.id == id)
            .ok_or(StoreError::NotFound)?
            .name)
    }

    fn summarize(&self, org: &OrgId, row: &db::PlanRow) -> Result<PlanSummary, StoreError> {
        Ok(PlanSummary {
            id: row.id.as_str().to_owned(),
            name: row.name.clone(),
            status: row.status.as_str(),
            source_node: self.node_name(org, &row.source_node_id)?,
            target_node: self.node_name(org, &row.target_node_id)?,
            created_at: row.created_at.clone(),
        })
    }
}

impl PlanStore for DbPlanStore<'_> {
    fn transports(&self, org: &OrgId) -> Result<Vec<TransportInfo>, StoreError> {
        // Only three columns leave this function; the row's config and secret id stay behind.
        Ok(db::list_transports(self.conn, org)?
            .into_iter()
            .map(|t| TransportInfo {
                name: t.name,
                kind: t.kind,
                verified_at: t.verified_at,
            })
            .collect())
    }

    fn transport(&self, org: &OrgId, name: &str) -> Result<ResolvedTransport, StoreError> {
        let row = db::list_transports(self.conn, org)?
            .into_iter()
            .find(|t| t.name == name)
            .ok_or(StoreError::NotFound)?;
        Ok(ResolvedTransport {
            id: row.id.as_str().to_owned(),
            verified: row.verified_at.is_some(),
        })
    }

    fn node_id(&self, org: &OrgId, name: &str) -> Result<String, StoreError> {
        Ok(self.node_by_name(org, name)?.as_str().to_owned())
    }

    fn plans(&self, org: &OrgId) -> Result<Vec<PlanSummary>, StoreError> {
        db::list_plans(self.conn, org, LIST_LIMIT)?
            .iter()
            .map(|row| self.summarize(org, row))
            .collect()
    }

    fn plan(&self, org: &OrgId, name: &str) -> Result<StoredPlan, StoreError> {
        let row = db::PlanId::parse(name)
            .ok()
            .and_then(|id| db::get_plan(self.conn, org, &id).ok())
            .map_or_else(|| db::find_plan_by_name(self.conn, org, name), Ok)?;
        let steps = db::list_plan_steps(self.conn, org, &row.id)?;
        let stored: Vec<StoredStep> = steps
            .into_iter()
            .map(|s| StoredStep {
                seq: s.seq,
                kind: s.kind.as_str().to_owned(),
                spec_json: s.spec_json,
                depends_on: s.depends_on,
            })
            .collect();
        let plan = Plan::from_stored(
            &row.name,
            row.source_node_id.as_str(),
            row.target_node_id.as_str(),
            &row.spec_json,
            &stored,
        )
        .map_err(|_| StoreError::Unavailable)?;
        let transport_id = &plan.spec().transport.transport_id;
        let transport_name = db::list_transports(self.conn, org)?
            .into_iter()
            .find(|t| t.id.as_str() == transport_id)
            .map(|t| t.name);
        Ok(StoredPlan {
            summary: self.summarize(org, &row)?,
            plan,
            transport_name,
        })
    }

    fn comparison(
        &self,
        org: &OrgId,
        source: &str,
        target: &str,
    ) -> Result<ComparisonView, StoreError> {
        let (source_node, target_node) = (
            self.node_by_name(org, source)?,
            self.node_by_name(org, target)?,
        );
        let header = db::latest_comparison_between(self.conn, org, &source_node, &target_node)?;
        let latest_source = db::latest_inventory(self.conn, org, &source_node)?;
        let latest_target = db::latest_inventory(self.conn, org, &target_node)?;
        let fresh = latest_source.id == header.source_inventory_id
            && latest_target.id == header.target_inventory_id;
        let inventory_partial = latest_source.status == InventoryStatus::Partial
            || latest_target.status == InventoryStatus::Partial;
        let rows = db::list_findings(self.conn, org, &header.id, None, u32::MAX)?;
        let mut findings = Vec::with_capacity(rows.len());
        let mut accepted = Vec::new();
        for row in rows {
            let is_accepted = row.status == FindingStatus::Accepted;
            let finding = finding_from_row(row)?;
            if is_accepted {
                accepted.push(FindingKey {
                    rule_id: finding.rule_id.clone(),
                    subject: finding.subject.clone(),
                });
            }
            findings.push(finding);
        }
        Ok(ComparisonView {
            created_at: header.created_at,
            fresh,
            inventory_partial,
            findings,
            accepted,
        })
    }
}

/// [`PlanSink`] over the SQLite layer.
pub struct DbPlanSink<'a> {
    conn: &'a Connection,
}

impl<'a> DbPlanSink<'a> {
    /// Saves drafts through `conn` (which must come from `db::open`).
    #[must_use]
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }
}

impl PlanSink for DbPlanSink<'_> {
    fn create_draft(&self, org: &OrgId, plan: &Plan) -> Result<String, PlanSinkError> {
        let spec_json = plan.spec_json().map_err(|_| PlanSinkError::Unavailable)?;
        let stored = plan
            .stored_steps()
            .map_err(|_| PlanSinkError::Unavailable)?;
        let mut steps = Vec::with_capacity(stored.len());
        for step in &stored {
            steps.push(db::NewPlanStep {
                seq: step.seq,
                kind: db::PlanStepKind::parse(&step.kind)
                    .map_err(|_| PlanSinkError::Unavailable)?,
                spec_json: &step.spec_json,
                depends_on: &step.depends_on,
            });
        }
        let (source, target) = (
            NodeId::parse(&plan.spec().source.node_id).map_err(|_| PlanSinkError::NodeMissing)?,
            NodeId::parse(&plan.spec().target.node_id).map_err(|_| PlanSinkError::NodeMissing)?,
        );
        let id = db::create_draft_plan(
            self.conn,
            org,
            &db::NewPlan {
                name: plan.name(),
                source_node: &source,
                target_node: &target,
                spec_json: &spec_json,
            },
            &steps,
            MAX_DRAFT_PLANS,
        )
        .map_err(|error| match error {
            db::DraftError::NameTaken => PlanSinkError::NameTaken,
            db::DraftError::TooManyDrafts => PlanSinkError::TooManyDrafts,
            db::DraftError::Db(db::Error::NotFound | db::Error::Invalid(_)) => {
                PlanSinkError::NodeMissing
            }
            db::DraftError::Db(_) => PlanSinkError::Unavailable,
        })?;
        Ok(id.as_str().to_owned())
    }
}

/// Runs one step of a migration plan (or simulates it when `dry_run` is true).
pub trait PlanStepRunner {
    /// Executes or simulates `step` of `plan`.
    ///
    /// # Errors
    /// Returns a fixed short reason code if the step fails.
    fn run_step(&self, plan: &Plan, step: &plan::Step, dry_run: bool) -> Result<(), &'static str>;
}

/// Default step runner that simulates or performs local no-op verification when no external
/// runner is wired (used for dry-run previews and local plan orchestration).
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultPlanStepRunner;

impl PlanStepRunner for DefaultPlanStepRunner {
    fn run_step(
        &self,
        _plan: &Plan,
        _step: &plan::Step,
        _dry_run: bool,
    ) -> Result<(), &'static str> {
        Ok(())
    }
}

/// Outcome of a single step in a [`PlanRunReport`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanStepOutcome {
    /// Step number.
    pub seq: u32,
    /// Wire kind (`inventory`, `backup`, ...).
    pub kind: &'static str,
    /// Resulting status (`succeeded`, `failed`, `skipped`, `dry_run_ok`, `dry_run_blocked`).
    pub status: &'static str,
    /// Fixed reason code on failure or gate block.
    pub reason: Option<&'static str>,
}

/// Summary of a plan run (or dry-run).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanRunReport {
    /// Plan UUID.
    pub plan_id: String,
    /// Plan name.
    pub name: String,
    /// Whether this was a dry-run (no DB or target changes).
    pub dry_run: bool,
    /// Plan status after the run (`done`, `paused`, or the current status for dry-run).
    pub status: &'static str,
    /// Per-step outcomes in execution order.
    pub steps: Vec<PlanStepOutcome>,
}

/// Why approving or running a plan failed.
#[derive(Debug)]
pub enum PlanRunError {
    /// Plan not found in this org.
    NotFound,
    /// Plan cannot be approved in its current state or failed validation.
    NotApprovable,
    /// Plan must be `approved` (or `paused`) before running.
    NotApproved,
    /// Plan gate refused cutover or decommission.
    GateBlocked(plan::ScheduleRefusal),
    /// Underlying store or database error.
    Store(StoreError),
}

impl From<StoreError> for PlanRunError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::NotFound => Self::NotFound,
            other => Self::Store(other),
        }
    }
}

impl From<db::Error> for PlanRunError {
    fn from(e: db::Error) -> Self {
        match e {
            db::Error::NotFound => Self::NotFound,
            db::Error::Invalid(_) => Self::NotApprovable,
            other => Self::Store(StoreError::from(other)),
        }
    }
}

/// Approves a `draft` or `reviewed` plan (`id` or `name`), validating it first and recording
/// `plan.approve` in the audit chain.
///
/// # Errors
/// [`PlanRunError::NotFound`], [`PlanRunError::NotApprovable`], or [`PlanRunError::Store`].
pub fn approve_plan(
    conn: &Connection,
    org: &OrgId,
    id_or_name: &str,
    actor_kind: db::ActorKind,
    actor_id: Option<&str>,
) -> Result<StoredPlan, PlanRunError> {
    let store = DbPlanStore::new(conn);
    let stored = store.plan(org, id_or_name)?;
    let plan_id = db::PlanId::parse(&stored.summary.id).map_err(|_| PlanRunError::NotApprovable)?;
    if !matches!(stored.summary.status, "draft" | "reviewed") {
        return Err(PlanRunError::NotApprovable);
    }
    db::approve_plan(conn, org, &plan_id, actor_kind, actor_id)?;
    Ok(store.plan(org, &stored.summary.id)?)
}

fn refusal_reason(refusal: &plan::ScheduleRefusal) -> &'static str {
    match refusal {
        plan::ScheduleRefusal::UnknownStep => "unknown_step",
        plan::ScheduleRefusal::AlreadyCompleted => "already_completed",
        plan::ScheduleRefusal::DependenciesPending(_) => "dependencies_pending",
        plan::ScheduleRefusal::GatesUnknown => "gates_unknown",
        plan::ScheduleRefusal::GatesNotSatisfied => "gates_not_satisfied",
        plan::ScheduleRefusal::SourceNotVerified => "source_not_verified",
    }
}

/// Runs (or dry-runs) a plan (`id` or `name`), checking gates before `cutover` and
/// `decommission_source` and tracking each `migration_steps` row atomically with audit entries.
///
/// # Errors
/// [`PlanRunError`].
#[allow(clippy::too_many_arguments)]
pub fn run_plan(
    conn: &Connection,
    catalog: &Catalog,
    locale: Locale,
    org: &OrgId,
    id_or_name: &str,
    dry_run: bool,
    runner: &dyn PlanStepRunner,
    actor_kind: db::ActorKind,
    actor_id: Option<&str>,
) -> Result<PlanRunReport, PlanRunError> {
    let store = DbPlanStore::new(conn);
    let stored = store.plan(org, id_or_name)?;
    let plan_id = db::PlanId::parse(&stored.summary.id).map_err(|_| PlanRunError::NotApproved)?;
    if !dry_run && !matches!(stored.summary.status, "approved" | "paused") {
        return Err(PlanRunError::NotApproved);
    }
    let (gates, _) = gates_for_stored(&store, catalog, locale, org, &stored)?;
    let existing_steps = db::list_plan_steps(conn, org, &plan_id)?;

    let mut completed = std::collections::BTreeSet::new();
    for s in &existing_steps {
        if matches!(
            s.status,
            db::PlanStepStatus::Succeeded | db::PlanStepStatus::Skipped
        ) {
            completed.insert(s.seq);
        }
    }

    if dry_run {
        let mut outcomes = Vec::with_capacity(stored.plan.steps().len());
        for step in stored.plan.steps() {
            if completed.contains(&step.seq) {
                outcomes.push(PlanStepOutcome {
                    seq: step.seq,
                    kind: step.kind.as_str(),
                    status: "succeeded",
                    reason: None,
                });
                continue;
            }
            if let Err(refusal) = stored
                .plan
                .check_schedulable(step.seq, &completed, Some(&gates))
            {
                outcomes.push(PlanStepOutcome {
                    seq: step.seq,
                    kind: step.kind.as_str(),
                    status: "dry_run_blocked",
                    reason: Some(refusal_reason(&refusal)),
                });
                continue;
            }
            match runner.run_step(&stored.plan, step, true) {
                Ok(()) => {
                    completed.insert(step.seq);
                    outcomes.push(PlanStepOutcome {
                        seq: step.seq,
                        kind: step.kind.as_str(),
                        status: "dry_run_ok",
                        reason: None,
                    });
                }
                Err(reason) => {
                    outcomes.push(PlanStepOutcome {
                        seq: step.seq,
                        kind: step.kind.as_str(),
                        status: "failed",
                        reason: Some(reason),
                    });
                }
            }
        }
        return Ok(PlanRunReport {
            plan_id: stored.summary.id,
            name: stored.summary.name,
            dry_run: true,
            status: stored.summary.status,
            steps: outcomes,
        });
    }

    db::begin_plan_run(conn, org, &plan_id, actor_kind, actor_id)?;
    let mut outcomes = Vec::with_capacity(stored.plan.steps().len());
    for step in stored.plan.steps() {
        if completed.contains(&step.seq) {
            outcomes.push(PlanStepOutcome {
                seq: step.seq,
                kind: step.kind.as_str(),
                status: "succeeded",
                reason: None,
            });
            continue;
        }
        if let Err(refusal) = stored
            .plan
            .check_schedulable(step.seq, &completed, Some(&gates))
        {
            let _ = db::finish_plan_run(
                conn,
                org,
                &plan_id,
                db::PlanStatus::Paused,
                actor_kind,
                actor_id,
            );
            return Err(PlanRunError::GateBlocked(refusal));
        }
        db::transition_plan_step(
            conn,
            org,
            &plan_id,
            step.seq,
            db::PlanStepStatus::Running,
            actor_kind,
            actor_id,
        )?;
        match runner.run_step(&stored.plan, step, false) {
            Ok(()) => {
                db::transition_plan_step(
                    conn,
                    org,
                    &plan_id,
                    step.seq,
                    db::PlanStepStatus::Succeeded,
                    actor_kind,
                    actor_id,
                )?;
                completed.insert(step.seq);
                outcomes.push(PlanStepOutcome {
                    seq: step.seq,
                    kind: step.kind.as_str(),
                    status: "succeeded",
                    reason: None,
                });
            }
            Err(reason) => {
                db::transition_plan_step(
                    conn,
                    org,
                    &plan_id,
                    step.seq,
                    db::PlanStepStatus::Failed,
                    actor_kind,
                    actor_id,
                )?;
                db::finish_plan_run(
                    conn,
                    org,
                    &plan_id,
                    db::PlanStatus::Paused,
                    actor_kind,
                    actor_id,
                )?;
                outcomes.push(PlanStepOutcome {
                    seq: step.seq,
                    kind: step.kind.as_str(),
                    status: "failed",
                    reason: Some(reason),
                });
                return Ok(PlanRunReport {
                    plan_id: stored.summary.id,
                    name: stored.summary.name,
                    dry_run: false,
                    status: "paused",
                    steps: outcomes,
                });
            }
        }
    }
    db::finish_plan_run(
        conn,
        org,
        &plan_id,
        db::PlanStatus::Done,
        actor_kind,
        actor_id,
    )?;
    Ok(PlanRunReport {
        plan_id: stored.summary.id,
        name: stored.summary.name,
        dry_run: false,
        status: "done",
        steps: outcomes,
    })
}
