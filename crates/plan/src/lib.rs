//! The migration plan model (docs/design.md 3.4 and 6; skill `migration-planning`; decision 0024).
//!
//! Pure: no I/O, no process execution, no network. A plan is data that a person reviews and that a
//! later executor turns into proposals; nothing in this crate changes a server.
//!
//! * [`PlanSpec`] is the strict, size-limited spec; [`Plan`] adds the ordered steps and can only be
//!   built validated (acyclic, required kinds, required order, safe cutover, source != target).
//! * [`gate_report`] and [`Plan::check_schedulable`] keep cutover and the tier-4 shutdown of the old
//!   server behind the gates.
//! * [`check_capacity`] gives advice before a backup is made; [`DEFAULT_EXCLUDES`] is what a backup
//!   leaves out, always shown in the written plan.
//! * [`draft_plan`] builds the standard sequence; [`render_plan`] writes it for a person, with all
//!   wording from the i18n catalog (`plan.*` keys).

mod capacity;
mod draft;
mod error;
mod exclude;
mod gates;
mod plan;
mod render;
mod spec;
mod steps;

pub use capacity::{
    CapacityAdvice, CapacityInput, CapacityIssue, CapacityIssueKind, CapacityVerdict,
    check_capacity,
};
pub use draft::{Draft, DraftOptions, DraftWarning, draft_plan};
pub use error::PlanError;
pub use exclude::{DEFAULT_EXCLUDES, default_excludes, describe_key, validate_pattern};
pub use gates::{AcceptedFinding, FindingKey, GateReport, gate_report};
pub use plan::{MAX_STEPS, Plan, ScheduleRefusal, StoredStep};
pub use render::{render_capacity, render_draft, render_plan};
pub use spec::{
    Backup, BackupLocation, BackupStrategy, ConflictPolicy, Cutover, CutoverMethod, Data,
    DatabaseEntry, DbMode, Encryption, Gates, MAX_DATABASES, MAX_EXCLUDES, MAX_JSON_BYTES, NodeRef,
    PlanSpec, TransportRef,
};
pub use steps::{Side, Step, StepKind};
