//! The `AiTool` contract (docs/design.md 12.4, skill `ai-tool-tiers`).

use crate::memory::{MemorySink, NoMemory};
use crate::plans::{NoPlans, PlanSink, PlanStore};
use crate::proposals::ProposalSink;
use crate::schema::{ArgSchema, Args};
use crate::store::ReadStore;
use core_domain::tier::Tier;
use db::OrgId;
use i18n::{Catalog, Locale};
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
    /// What the tool changes, as an i18n catalog key (never the sentence itself, so it is shown in
    /// the reader's language). `None` for a read-only tool. Part of every Proposal's impact
    /// summary for tier 1 and above.
    pub impact: Option<&'static str>,
}

/// Who and what a call acts for. Built by the server from the logged-in session; a model never
/// supplies any of it.
#[non_exhaustive]
pub struct ToolContext<'a> {
    /// The organization every read is scoped to.
    pub org: &'a OrgId,
    /// Everything a tool may read. Every method takes the org again, so no read can forget it.
    pub store: &'a dyn ReadStore,
    /// The only write a tool has: recording pending proposals. It cannot approve or run one.
    pub proposals: &'a dyn ProposalSink,
    /// Search and memory: reads, plus creating PENDING memories. It cannot approve one.
    pub memory: &'a dyn MemorySink,
    /// What the planning tools may read besides inventories (transports, plans, comparisons).
    pub plans: &'a dyn PlanStore,
    /// The only plan write a tool has: one new **draft** plan. It cannot approve, schedule or run.
    pub plan_sink: &'a dyn PlanSink,
    /// Message catalog for localized explanations.
    pub catalog: &'a Catalog,
    /// The language of the person asking; text a tool writes follows it.
    pub locale: Locale,
}

/// Used when a caller supplies no proposal sink: tools that try to propose get a clean refusal.
static NO_PROPOSALS: crate::NoProposals = crate::NoProposals;

/// Used when a caller supplies no memory sink: search finds nothing and nothing is remembered.
static NO_MEMORY: NoMemory = NoMemory;
/// Used when a caller supplies no plan seams: no plans, no transports, and no way to save a draft.
static NO_PLANS: NoPlans = NoPlans;

impl<'a> ToolContext<'a> {
    /// A read-only context. Writes a tool may perform (proposals, and later memory or plans) are
    /// added with the `with_*` methods, so a caller never has to name every seam.
    #[must_use]
    pub fn new(
        org: &'a OrgId,
        store: &'a dyn ReadStore,
        catalog: &'a Catalog,
        locale: Locale,
    ) -> Self {
        Self {
            org,
            store,
            proposals: &NO_PROPOSALS,
            memory: &NO_MEMORY,
            plans: &NO_PLANS,
            plan_sink: &NO_PLANS,
            catalog,
            locale,
        }
    }

    /// Lets the planning tools read through `plans` and save drafts through `sink`.
    #[must_use]
    pub fn with_plans(mut self, plans: &'a dyn PlanStore, sink: &'a dyn PlanSink) -> Self {
        self.plans = plans;
        self.plan_sink = sink;
        self
    }

    /// Lets tools record pending proposals through `proposals`.
    #[must_use]
    pub fn with_proposals(mut self, proposals: &'a dyn ProposalSink) -> Self {
        self.proposals = proposals;
        self
    }

    /// Lets tools search and create pending memories through `memory`.
    #[must_use]
    pub fn with_memory(mut self, memory: &'a dyn MemorySink) -> Self {
        self.memory = memory;
        self
    }
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
    /// The request is well-formed but not acceptable (for example a limit was reached). The code
    /// is fixed text, safe to show to the model.
    Invalid(&'static str),
    /// Like [`ToolError::Invalid`], with an explanation in the reader's language from the i18n
    /// catalog. The code is fixed text; the explanation must not copy server data.
    Explained {
        /// Stable machine code.
        code: &'static str,
        /// The localized explanation.
        text: String,
    },
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => f.write_str("not_found"),
            Self::Unavailable(why) => write!(f, "unavailable: {why}"),
            Self::Invalid(why) => write!(f, "invalid: {why}"),
            Self::Explained { code, text } => write!(f, "invalid: {code}: {text}"),
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
