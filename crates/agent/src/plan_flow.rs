//! The drafting and gate logic behind the planning tools, shared with the command line (ADR 0028).
//!
//! The AI tool `draft_migration_plan` and `infra plan draft` call the same [`draft_migration_plan`];
//! `show_plan`, `check_plan_gates` and `infra plan show|gates` read gates through the same
//! [`gates_for_stored`]. Nothing here is specific to a model: callers translate [`DraftError`] into
//! their own wording (a tool error for the model, a catalog message for a person).
//!
//! Names, not ids: callers name nodes and transports; everything is resolved inside the org given.
//! The only write is one new plan with status `draft`, through [`PlanSink`].

use crate::plans::{PlanSink, PlanSinkError, PlanStore, StoredPlan};
use crate::store::{ReadStore, StoreError};
use crate::tool::ToolError;
use crate::tools::clean_text;
use db::OrgId;
use i18n::{Catalog, Locale, MessageKey};
use inventory::Finding;
use plan::{
    AcceptedFinding, BackupLocation, CutoverMethod, Draft, DraftOptions, GateReport, draft_plan,
    gate_report,
};

/// Longest plan name accepted.
pub const NAME_MAX: usize = 100;

/// What the latest comparison between two nodes lets a gate say.
#[derive(Debug, Clone)]
pub struct Evidence {
    findings: Vec<Finding>,
    accepted: Vec<AcceptedFinding>,
    /// The comparison exists and covers the newest inventories of both nodes. Without it an empty
    /// finding list proves nothing, so the report is marked incomplete.
    current: bool,
    partial: bool,
}

impl Evidence {
    /// Marks `report` incomplete unless a current, complete comparison stands behind it.
    #[must_use]
    pub fn gates(&self, mut report: GateReport) -> GateReport {
        report.inventory_incomplete |= !self.current || self.partial;
        report
    }

    /// The comparison covers the newest inventories of both nodes.
    #[must_use]
    pub const fn is_current(&self) -> bool {
        self.current
    }

    /// `current` or `missing_or_stale` (machine text).
    #[must_use]
    pub const fn word(&self) -> &'static str {
        if self.current {
            "current"
        } else {
            "missing_or_stale"
        }
    }
}

/// Reads the evidence for the pair `source` (reference) and `target` (checked machine).
///
/// A comparison that is missing or stale gives empty evidence marked not current, never an error.
///
/// # Errors
/// [`StoreError::Unavailable`] when the store cannot answer.
pub fn collect_evidence(
    plans: &dyn PlanStore,
    catalog: &Catalog,
    locale: Locale,
    org: &OrgId,
    source: &str,
    target: &str,
) -> Result<Evidence, StoreError> {
    let view = match plans.comparison(org, source, target) {
        Ok(view) if view.fresh => view,
        Ok(_) | Err(StoreError::NotFound) => {
            return Ok(Evidence {
                findings: Vec::new(),
                accepted: Vec::new(),
                current: false,
                partial: false,
            });
        }
        Err(StoreError::Unavailable) => return Err(StoreError::Unavailable),
    };
    let reason = catalog
        .get(
            locale,
            MessageKey::new("assistant.plan.accepted_reason"),
            &[],
        )
        .text;
    // A person's acceptance only; an acceptance that no longer matches a finding is ignored by
    // `gate_report`, and one that cannot be built leaves the finding open.
    let accepted = view
        .accepted
        .iter()
        .filter_map(|key| AcceptedFinding::new(&key.rule_id, key.subject.clone(), &reason).ok())
        .collect();
    Ok(Evidence {
        findings: view.findings,
        accepted,
        current: true,
        partial: view.inventory_partial,
    })
}

/// The gate report of a stored plan, over the latest comparison of its two nodes.
///
/// # Errors
/// [`StoreError::Unavailable`] when the store cannot answer.
pub fn gates_for_stored(
    plans: &dyn PlanStore,
    catalog: &Catalog,
    locale: Locale,
    org: &OrgId,
    stored: &StoredPlan,
) -> Result<(GateReport, Evidence), StoreError> {
    let (source, target) = (&stored.summary.source_node, &stored.summary.target_node);
    let evidence = collect_evidence(plans, catalog, locale, org, source, target)?;
    let report = evidence.gates(gate_report(&evidence.findings, &evidence.accepted));
    Ok((report, evidence))
}

/// Everything a draft needs to read and write, for one org.
pub struct DraftContext<'a> {
    /// The org every lookup and the write are scoped to.
    pub org: &'a OrgId,
    /// Inventories.
    pub store: &'a dyn ReadStore,
    /// Nodes, transports and comparisons.
    pub plans: &'a dyn PlanStore,
    /// Where the draft is saved.
    pub sink: &'a dyn PlanSink,
    /// For the wording stored with acceptances.
    pub catalog: &'a Catalog,
    /// Language of the person asking.
    pub locale: Locale,
}

/// What to draft. Everything is a name, never an id.
#[derive(Debug, Clone, Copy, Default)]
pub struct DraftRequest<'a> {
    /// Node the data comes from.
    pub source: &'a str,
    /// Node the data goes to.
    pub target: &'a str,
    /// A verified transport; optional when exactly one verified transport exists.
    pub transport: Option<&'a str>,
    /// Where the backup is kept; default source.
    pub backup_location: Option<BackupLocation>,
    /// How traffic moves; default tunnel.
    pub cutover_method: Option<CutoverMethod>,
    /// Plan name; default is the two node names.
    pub name: Option<&'a str>,
}

/// A saved draft.
#[derive(Debug, Clone)]
pub struct Drafted {
    /// Id of the new plan.
    pub id: String,
    /// The (cleaned) plan name.
    pub name: String,
    /// The transport the plan uses.
    pub transport_name: String,
    /// What the gates were computed from.
    pub evidence: Evidence,
    /// The plan, its gates and warnings.
    pub draft: Draft,
}

/// Why no draft was saved. Closed set; callers choose the wording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftError {
    /// Source and target are the same name.
    SameNode,
    /// No source node of that name in this org.
    SourceMissing,
    /// No target node of that name in this org.
    TargetMissing,
    /// The source has no inventory yet.
    SourceNoInventory,
    /// The target has no inventory yet.
    TargetNoInventory,
    /// No transport was named and there is not exactly one verified one.
    TransportRequired,
    /// No transport of that name in this org.
    TransportMissing,
    /// The transport never passed a verification.
    TransportUnverified,
    /// The name is empty or too long after redaction (the code is machine text).
    Name(&'static str),
    /// The plan crate refused the inputs.
    Rejected,
    /// A plan with that name exists.
    NameTaken,
    /// The org holds the maximum number of drafts.
    TooManyDrafts,
    /// A node vanished between lookup and write.
    NodeMissing,
    /// The store failed; the word says which seam (`store` or `plans`).
    Unavailable(&'static str),
}

impl DraftError {
    /// The tool-facing error of this failure: the model sees the same codes and catalog texts as
    /// before the command line shared this code.
    #[must_use]
    pub fn into_tool_error(self, catalog: &Catalog, locale: Locale) -> ToolError {
        let (code, key): (&'static str, &'static str) = match self {
            Self::SameNode => ("same_node", "assistant.plan.err_same_node"),
            Self::SourceMissing | Self::NodeMissing => {
                ("source_node_not_found", "assistant.plan.err_source_missing")
            }
            Self::TargetMissing => ("target_node_not_found", "assistant.plan.err_target_missing"),
            Self::SourceNoInventory => ("no_inventory", "assistant.plan.err_source_no_inventory"),
            Self::TargetNoInventory => ("no_inventory", "assistant.plan.err_target_no_inventory"),
            Self::TransportRequired => (
                "transport_required",
                "assistant.plan.err_transport_required",
            ),
            Self::TransportMissing => (
                "transport_not_found",
                "assistant.plan.err_transport_missing",
            ),
            Self::TransportUnverified => (
                "transport_not_verified",
                "assistant.plan.err_transport_unverified",
            ),
            Self::Rejected => ("plan_rejected", "assistant.plan.err_rejected"),
            Self::NameTaken => ("plan_name_taken", "assistant.plan.err_name_taken"),
            Self::TooManyDrafts => ("too_many_draft_plans", "assistant.plan.err_too_many"),
            Self::Name(code) => return ToolError::Invalid(code),
            Self::Unavailable(seam) => return ToolError::Unavailable(seam.to_owned()),
        };
        ToolError::Explained {
            code,
            text: catalog.get(locale, MessageKey::new(key), &[]).text,
        }
    }
}

fn node_id(ctx: &DraftContext<'_>, name: &str, missing: DraftError) -> Result<String, DraftError> {
    ctx.plans.node_id(ctx.org, name).map_err(|e| match e {
        StoreError::NotFound => missing,
        StoreError::Unavailable => DraftError::Unavailable("store"),
    })
}

fn inventory(
    ctx: &DraftContext<'_>,
    name: &str,
    missing: DraftError,
) -> Result<inventory::Inventory, DraftError> {
    ctx.store
        .latest_inventory(ctx.org, name)
        .map_err(|e| match e {
            StoreError::NotFound => missing,
            StoreError::Unavailable => DraftError::Unavailable("store"),
        })
}

/// The transport to use: the named one, or the only verified one. Unverified is refused.
fn resolve_transport(
    ctx: &DraftContext<'_>,
    requested: Option<&str>,
) -> Result<(String, String), DraftError> {
    let name = if let Some(name) = requested {
        name.to_owned()
    } else {
        let all = ctx
            .plans
            .transports(ctx.org)
            .map_err(|_| DraftError::Unavailable("store"))?;
        let mut verified = all.iter().filter(|t| t.verified_at.is_some());
        match (verified.next(), verified.next()) {
            (Some(only), None) => only.name.clone(),
            _ => return Err(DraftError::TransportRequired),
        }
    };
    let resolved = ctx.plans.transport(ctx.org, &name).map_err(|e| match e {
        StoreError::NotFound => DraftError::TransportMissing,
        StoreError::Unavailable => DraftError::Unavailable("store"),
    })?;
    if !resolved.verified {
        return Err(DraftError::TransportUnverified);
    }
    Ok((resolved.id, name))
}

/// Drafts a migration plan from the latest inventories and the latest comparison of the two
/// nodes and saves it with status `draft`. Nothing is approved, scheduled or run.
///
/// # Errors
/// [`DraftError`].
pub fn draft_migration_plan(
    ctx: &DraftContext<'_>,
    request: &DraftRequest<'_>,
) -> Result<Drafted, DraftError> {
    let (source, target) = (request.source, request.target);
    if source == target {
        return Err(DraftError::SameNode);
    }
    let source_id = node_id(ctx, source, DraftError::SourceMissing)?;
    let target_id = node_id(ctx, target, DraftError::TargetMissing)?;
    let source_inventory = inventory(ctx, source, DraftError::SourceNoInventory)?;
    let target_inventory = inventory(ctx, target, DraftError::TargetNoInventory)?;

    let (transport_id, transport_name) = resolve_transport(ctx, request.transport)?;

    // Redacted, control characters stripped, length checked after redaction. The default name is
    // built from the two node names and goes through the same path.
    let clean = |raw: &str| {
        clean_text(raw, NAME_MAX).map_err(|e| match e {
            ToolError::Invalid(code) => DraftError::Name(code),
            _ => DraftError::Name("text_too_long"),
        })
    };
    let name = match request.name {
        Some(raw) => clean(raw)?,
        None => clean(&format!("{source} -> {target}"))?,
    };
    if name.trim().is_empty() {
        return Err(DraftError::Name("empty_text"));
    }

    let evidence = collect_evidence(ctx.plans, ctx.catalog, ctx.locale, ctx.org, source, target)
        .map_err(|_| DraftError::Unavailable("plans"))?;
    let mut options = DraftOptions::new(&name, &source_id, &target_id, &transport_id);
    if let Some(location) = request.backup_location {
        options.backup_location = location;
    }
    if let Some(method) = request.cutover_method {
        options.cutover_method = method;
    }
    options.accepted.clone_from(&evidence.accepted);

    let mut draft = draft_plan(
        &source_inventory,
        Some(&target_inventory),
        &evidence.findings,
        &options,
    )
    .map_err(|_| DraftError::Rejected)?;
    // The gate can only be passed with a comparison of the newest inventories behind it.
    draft.gates = evidence.gates(draft.gates);

    let id = ctx
        .sink
        .create_draft(ctx.org, &draft.plan)
        .map_err(|e| match e {
            PlanSinkError::NameTaken => DraftError::NameTaken,
            PlanSinkError::TooManyDrafts => DraftError::TooManyDrafts,
            PlanSinkError::NodeMissing => DraftError::NodeMissing,
            PlanSinkError::Unavailable => DraftError::Unavailable("plans"),
        })?;
    Ok(Drafted {
        id,
        name,
        transport_name,
        evidence,
        draft,
    })
}
