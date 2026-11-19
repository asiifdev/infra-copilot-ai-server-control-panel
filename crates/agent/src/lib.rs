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

mod exec;
mod registry;
mod schema;
pub mod testing;
mod tool;
mod untrusted;

pub use exec::{CallRecord, CallStatus, Executor, Limits, Outcome};
pub use registry::{RegisterError, Registry, ToolDefinition};
pub use schema::{ArgError, ArgSchema, Args, FORBIDDEN_FIELD_NAMES, Field, Kind};
pub use tool::{AiTool, ToolContext, ToolError, ToolOutput, ToolSpec};
pub use untrusted::{CLOSE_TAG, OPEN_TAG, wrap_untrusted};
