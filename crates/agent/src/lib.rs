//! The AI tool layer (ADR 0020, M2-a): everything that stands between a model's tool call and
//! the server, with no model and no network in sight.
//!
//! Rules enforced by code here, not by prompt (CLAUDE.md rules 1-4, skill `ai-tool-tiers`):
//! * the tier of a tool is declared by the tool's own code ([`ToolSpec::tier`]); a model cannot
//!   pass or lower it, and the registry refuses a tool above its ceiling (M2: tier 1, "plan");
//! * arguments are validated against a closed, typed schema before the tool runs: unknown
//!   fields, wrong types, out-of-range values and identity fields (`org_id`, `user_id`, ...) are
//!   refused, so a model can never name the tenant it acts for;
//! * every result is redacted ([`core_domain::redact`]), size-capped and wrapped as untrusted
//!   data before it goes back to the model;
//! * every call leaves a [`CallRecord`] (no secrets, no raw output) for `tool_calls` and the
//!   audit log.
//!
//! Tools are synchronous and read through the [`ToolContext`]; the async model loop (M2-d) calls
//! the [`Executor`] from a blocking task.

mod assistant;
mod exec;
mod memory;
mod memory_tools;
mod persist;
mod plan_flow;
mod plan_tools;
mod plans;
mod proposals;
mod registry;
mod schema;
mod store;
pub mod testing;
mod tool;
mod tools;
mod untrusted;

pub use assistant::{
    Assistant, DbProviderLoader, LoadError, ProviderLoader, Turn, TurnConfig, TurnError, TurnReport,
};
pub use exec::{CallRecord, CallStatus, Executor, Limits, Outcome};
pub use memory::{
    DbMemorySink, MAX_MEMORIES_PER_CONVERSATION, MAX_PENDING_PER_ORG, MemoryDraft, MemoryError,
    MemoryNote, MemorySink, NoMemory,
};
pub use persist::{persist_comparison, persist_inventory};
pub use plan_flow::{
    DraftContext, DraftError, DraftRequest, Drafted, Evidence, collect_evidence,
    draft_migration_plan, gates_for_stored,
};
pub use plans::{
    ComparisonView, DbPlanSink, DbPlanStore, DefaultPlanStepRunner, MAX_DRAFT_PLANS, NoPlans,
    PlanRunError, PlanRunReport, PlanSink, PlanSinkError, PlanStepOutcome, PlanStepRunner,
    PlanStore, PlanSummary, ResolvedTransport, StoredPlan, TransportInfo, approve_plan, run_plan,
};
pub use proposals::{
    ActionDraft, DbProposalSink, MAX_PENDING_PER_CONVERSATION, NoProposals, ProposalDraft,
    ProposalSink, SinkError,
};
pub use registry::{RegisterError, Registry, ToolDefinition};
pub use schema::{ArgError, ArgSchema, Args, FORBIDDEN_FIELD_NAMES, Field, Kind};
pub use store::{DbStore, FindingsView, NodeInfo, ReadStore, StoreError};
pub use tool::{AiTool, ToolContext, ToolError, ToolOutput, ToolSpec};
pub use tools::assistant_registry;
pub use untrusted::{CLOSE_TAG, OPEN_TAG, wrap_untrusted};
