//! Why a plan, a spec or a scheduling request was refused. Messages are for developers and logs;
//! they never echo free-form input values (only field names, step numbers and kinds). What a person
//! reads comes from the i18n catalog through [`crate::render_plan`].

use std::fmt;

use crate::steps::StepKind;

/// Everything that can be wrong with a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    /// The JSON could not be parsed (unknown field, wrong enum value, wrong type, ...).
    Parse(String),
    /// A list or the whole document exceeds its size limit.
    TooLarge {
        /// Which list or document.
        what: &'static str,
        /// The limit.
        max: usize,
    },
    /// A value is empty, too long, contains control characters or is otherwise malformed.
    Invalid(&'static str),
    /// Source and target are the same node.
    SameNode,
    /// The node ids inside the stored spec differ from the node columns of the row.
    NodeMismatch,
    /// Two database entries share a key.
    DuplicateDatabase,
    /// A gate that must always be on was switched off.
    GateDisabled(&'static str),
    /// DNS cutover without a DNS snapshot and rollback.
    UnsafeDnsCutover,
    /// A backup kept on an intermediary must be encrypted on the client first.
    IntermediaryNeedsEncryption,
    /// The plan has no steps.
    NoSteps,
    /// Two steps share a number.
    DuplicateSeq(u32),
    /// A step lists the same dependency twice.
    DuplicateDependency(u32),
    /// A step depends on itself.
    SelfDependency(u32),
    /// A step depends on a step that does not exist.
    UnknownDependency {
        /// The step with the bad dependency.
        step: u32,
        /// The missing step number.
        missing: u32,
    },
    /// The dependencies contain a cycle.
    Cycle,
    /// A required kind of step is missing.
    MissingKind(StepKind),
    /// A kind that may appear only once appears more than once.
    TooMany(StepKind),
    /// A step of a later kind does not (transitively) depend on a step of an earlier kind.
    OrderMissing {
        /// The kind that must come first.
        earlier: StepKind,
        /// The kind that must come after it.
        later: StepKind,
    },
    /// A step of an earlier kind (transitively) depends on a step of a later kind.
    OrderReversed {
        /// The kind that must come first.
        earlier: StepKind,
        /// The kind that wrongly runs first.
        later: StepKind,
    },
    /// An inventory step does not say whether it reads the source or the target.
    SideRequired(u32),
    /// A step that is not an inventory step names a side.
    SideNotAllowed(u32),
    /// There is no inventory step for the source.
    NoSourceInventory,
    /// A database is named by a step kind that cannot carry one.
    DatabaseNotAllowed(u32),
    /// A step names a database that the spec does not list.
    UnknownDatabase(u32),
    /// A restore step targets a database whose mode is `skip`.
    SkippedDatabaseRestored(u32),
    /// A stored step has a kind this build does not know.
    UnknownKind,
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(msg) => write!(f, "invalid plan JSON: {msg}"),
            Self::TooLarge { what, max } => write!(f, "{what} exceeds the limit of {max}"),
            Self::Invalid(field) => write!(f, "invalid value for {field}"),
            Self::NodeMismatch => f.write_str("spec node ids differ from the stored plan nodes"),
            Self::SameNode => f.write_str("source and target must be different nodes"),
            Self::DuplicateDatabase => f.write_str("a database is listed twice"),
            Self::GateDisabled(name) => write!(f, "gate {name} must stay on"),
            Self::UnsafeDnsCutover => {
                f.write_str("a DNS cutover needs a DNS snapshot and rollback")
            }
            Self::IntermediaryNeedsEncryption => {
                f.write_str("a backup on an intermediary must be encrypted on the client")
            }
            Self::NoSteps => f.write_str("the plan has no steps"),
            Self::DuplicateSeq(seq) => write!(f, "step {seq} is defined twice"),
            Self::DuplicateDependency(seq) => write!(f, "step {seq} lists a dependency twice"),
            Self::SelfDependency(seq) => write!(f, "step {seq} depends on itself"),
            Self::UnknownDependency { step, missing } => {
                write!(f, "step {step} depends on missing step {missing}")
            }
            Self::Cycle => f.write_str("the step dependencies contain a cycle"),
            Self::MissingKind(kind) => write!(f, "the plan has no {} step", kind.as_str()),
            Self::TooMany(kind) => write!(f, "the plan has more than one {} step", kind.as_str()),
            Self::OrderMissing { earlier, later } => write!(
                f,
                "every {} step must depend on every {} step",
                later.as_str(),
                earlier.as_str()
            ),
            Self::OrderReversed { earlier, later } => write!(
                f,
                "a {} step may not run before a {} step",
                later.as_str(),
                earlier.as_str()
            ),
            Self::SideRequired(seq) => write!(f, "inventory step {seq} must name a side"),
            Self::SideNotAllowed(seq) => write!(f, "step {seq} may not name a side"),
            Self::NoSourceInventory => f.write_str("the plan has no source inventory step"),
            Self::DatabaseNotAllowed(seq) => write!(f, "step {seq} may not name a database"),
            Self::UnknownDatabase(seq) => write!(f, "step {seq} names an unlisted database"),
            Self::SkippedDatabaseRestored(seq) => {
                write!(f, "step {seq} restores a database marked skip")
            }
            Self::UnknownKind => f.write_str("unknown step kind"),
        }
    }
}

impl std::error::Error for PlanError {}
