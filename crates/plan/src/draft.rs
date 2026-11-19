//! Builds the standard step sequence from what is known about the two machines. No AI is involved:
//! the same inputs always give the same plan, and the result goes through the same validation as a
//! plan written by hand.

use inventory::{Finding, Inventory, Status};

use crate::error::PlanError;
use crate::exclude::default_excludes;
use crate::gates::{AcceptedFinding, GateReport, gate_report};
use crate::plan::Plan;
use crate::spec::{
    Backup, BackupLocation, BackupStrategy, ConflictPolicy, Cutover, CutoverMethod, Data,
    DatabaseEntry, DbMode, Encryption, Gates, NodeRef, PlanSpec, TransportRef,
};
use crate::steps::{Side, Step, StepKind};

/// What the person chose. [`DraftOptions::new`] fills the safe defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftOptions {
    /// Plan name.
    pub name: String,
    /// Source node id.
    pub source_node_id: String,
    /// Target node id.
    pub target_node_id: String,
    /// Id of the configured transport.
    pub transport_id: String,
    /// Where the backup is kept.
    pub backup_location: BackupLocation,
    /// Encryption of the backup in transit.
    pub encryption: Encryption,
    /// Tunnel or DNS.
    pub cutover_method: CutoverMethod,
    /// Databases and what happens to each.
    pub databases: Vec<DatabaseEntry>,
    /// Exclusions; `None` uses the default list of rebuildable data.
    pub exclude: Option<Vec<String>>,
    /// Plan a delta between verification and cutover.
    pub include_delta: bool,
    /// Plan the shutdown of the old server as the last step (tier 4, still gated).
    pub include_decommission: bool,
    /// Critical findings the person accepted, each with a reason.
    pub accepted: Vec<AcceptedFinding>,
}

impl DraftOptions {
    /// Defaults: backup on the source, client-side encryption, tunnel cutover, default exclusions,
    /// a delta, the decommission step planned, no databases, nothing accepted.
    #[must_use]
    pub fn new(name: &str, source_node_id: &str, target_node_id: &str, transport_id: &str) -> Self {
        Self {
            name: name.to_owned(),
            source_node_id: source_node_id.to_owned(),
            target_node_id: target_node_id.to_owned(),
            transport_id: transport_id.to_owned(),
            backup_location: BackupLocation::Source,
            encryption: Encryption::ClientSide,
            cutover_method: CutoverMethod::Tunnel,
            databases: Vec::new(),
            exclude: None,
            include_delta: true,
            include_decommission: true,
            accepted: Vec::new(),
        }
    }
}

/// Something the person should know about a draft before relying on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftWarning {
    /// The source inventory is partial.
    SourceInventoryPartial,
    /// The target inventory is partial.
    TargetInventoryPartial,
    /// No target inventory was given: the plan starts by collecting it, and the findings may be
    /// incomplete.
    NoTargetInventory,
    /// Source and target differ in operating system or architecture.
    PlatformDiffers,
    /// The plan lists no database: only files and configuration move.
    NoDatabases,
}

/// A drafted plan with the gate report it was drafted against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    /// The validated plan.
    pub plan: Plan,
    /// Gates over the findings given to [`draft_plan`].
    pub gates: GateReport,
    /// Things to read before trusting the draft.
    pub warnings: Vec<DraftWarning>,
}

/// Drafts a plan.
///
/// Sequence: inventory of the source and of the target, backup, transfer, one restore for files and
/// configuration plus one per database that is not skipped, verify, an optional delta, cutover,
/// post-verify and an optional shutdown of the old server. The gate report is computed from
/// `findings` and the acceptances in `options`; a partial inventory makes it incomplete.
///
/// # Errors
/// Any [`PlanError`] of the spec or the steps, for example the same node twice, an unsafe DNS
/// cutover or a malformed exclusion.
pub fn draft_plan(
    source: &Inventory,
    target: Option<&Inventory>,
    findings: &[Finding],
    options: &DraftOptions,
) -> Result<Draft, PlanError> {
    let spec = PlanSpec {
        source: NodeRef {
            node_id: options.source_node_id.clone(),
        },
        target: NodeRef {
            node_id: options.target_node_id.clone(),
        },
        backup: Backup {
            location: options.backup_location,
            strategy: BackupStrategy::SnapshotDelta,
            exclude: options.exclude.clone().unwrap_or_else(default_excludes),
        },
        transport: TransportRef {
            transport_id: options.transport_id.clone(),
            encryption: options.encryption,
        },
        data: Data {
            databases: options.databases.clone(),
            conflict_policy: ConflictPolicy::NeverOverwrite,
        },
        cutover: Cutover {
            method: options.cutover_method,
            dns_snapshot: true,
            rollback: true,
        },
        gates: Gates {
            no_open_critical_findings: true,
            keep_source_until_verified: true,
        },
    };

    let mut steps = vec![
        Step::new(0, StepKind::Inventory, Vec::new()).on(Side::Source),
        Step::new(1, StepKind::Inventory, Vec::new()).on(Side::Target),
        Step::new(2, StepKind::Backup, vec![0, 1]),
        Step::new(3, StepKind::Transfer, vec![2]),
    ];
    let mut next = 4;
    let mut restores = vec![next];
    steps.push(Step::new(next, StepKind::Restore, vec![3]));
    next += 1;
    for db in options
        .databases
        .iter()
        .filter(|db| db.mode != DbMode::Skip)
    {
        restores.push(next);
        steps.push(Step::new(next, StepKind::Restore, vec![3]).for_database(&db.key));
        next += 1;
    }
    let verify = next;
    steps.push(Step::new(verify, StepKind::Verify, restores));
    next += 1;
    let mut last = verify;
    if options.include_delta {
        steps.push(Step::new(next, StepKind::Delta, vec![last]));
        last = next;
        next += 1;
    }
    steps.push(Step::new(next, StepKind::Cutover, vec![last]));
    last = next;
    next += 1;
    steps.push(Step::new(next, StepKind::PostVerify, vec![last]));
    last = next;
    next += 1;
    if options.include_decommission {
        steps.push(Step::new(next, StepKind::DecommissionSource, vec![last]));
    }

    let plan = Plan::new(options.name.clone(), spec, steps)?;
    let mut gates = gate_report(findings, &options.accepted).with_inventories(source, target);
    // Without a target inventory no comparison could have run: an empty finding list proves nothing.
    gates.inventory_incomplete |= target.is_none();

    let mut warnings = Vec::new();
    if source.status == Status::Partial {
        warnings.push(DraftWarning::SourceInventoryPartial);
    }
    match target {
        None => warnings.push(DraftWarning::NoTargetInventory),
        Some(inv) => {
            if inv.status == Status::Partial {
                warnings.push(DraftWarning::TargetInventoryPartial);
            }
            if inv.platform != source.platform {
                warnings.push(DraftWarning::PlatformDiffers);
            }
        }
    }
    if options.databases.is_empty() {
        warnings.push(DraftWarning::NoDatabases);
    }
    Ok(Draft {
        plan,
        gates,
        warnings,
    })
}
