//! The `AiTool` contract (docs/design.md 12.4, skill `ai-tool-tiers`).

use crate::schema::{ArgSchema, Args};
use core_domain::tier::Tier;
use db::OrgId;
use std::fmt;

/// What a tool declares about itself. All of it is fixed in the tool's code.
#[derive(Clone, Copy, Debug)]
pub struct ToolSpec {
    /// Unique name, `[a-z][a-z0-9_]*`, at most 64 characters.
    pub name: &'static str,
    /// What the tool does, for the model. Keep it terse: it costs input tokens on every turn.
    pub description: &'static str,
    /// Risk tier. Decided here; neither the model nor the user can change it.
    pub tier: Tier,
    /// Closed argument schema.
    pub schema: ArgSchema,
    /// What the tool changes, in one sentence. `None` for a read-only tool. Part of every
    /// Proposal's impact summary for tier 1 and above.
    pub impact: Option<&'static str>,
}

/// Who and what a call acts for. Built by the server from the logged-in session; a model never
/// supplies any of it.
#[derive(Debug)]
pub struct ToolContext<'a> {
    /// The organization every read is scoped to.
    pub org: &'a OrgId,
}

/// Text a tool returns. It is redacted, capped and fenced by the executor, so a tool does not
/// need to (and cannot) skip those steps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolOutput(String);

impl ToolOutput {
    /// Wraps result text.
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    /// The text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn into_string(self) -> String {
        self.0
    }
}

/// Why a tool failed. The text goes back to the model after redaction, so it may name the
/// problem but should not copy server data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolError {
    /// The requested object does not exist, or belongs to another organization (the two are
    /// indistinguishable on purpose).
    NotFound,
    /// The tool cannot answer right now (database busy, collector failed).
    Unavailable(String),
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => f.write_str("not_found"),
            Self::Unavailable(why) => write!(f, "unavailable: {why}"),
        }
    }
}

impl std::error::Error for ToolError {}

/// A function a model may call.
///
/// Implementations must be pure with respect to authority: they read what `ctx.org` may read
/// and nothing else, and they never take an organization, user or role from `args`.
pub trait AiTool: Send + Sync {
    /// The tool's fixed declaration.
    fn spec(&self) -> &ToolSpec;

    /// Runs the tool with validated arguments.
    ///
    /// # Errors
    /// [`ToolError`] when the tool cannot produce a result.
    fn run(&self, ctx: &ToolContext<'_>, args: &Args) -> Result<ToolOutput, ToolError>;
}
