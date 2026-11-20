//! The assistant loop (ADR 0023, M2-d): one person message in, one honest answer out, with tool
//! calls in between. docs/design.md 7.1 is the flow; this module is where its limits live.
//!
//! What the code guarantees, whatever a model says:
//! * the person's message is redacted before it is stored or sent anywhere;
//! * only one turn runs per conversation (an atomic claim in the database);
//! * every tool call of a response is answered, goes through the [`crate::Executor`] (tier
//!   ceiling, closed argument schemas, redaction, fencing), and leaves a `tool_calls` row and a
//!   hash-chained audit entry;
//! * a turn makes at most [`TurnConfig::max_rounds`] model calls with tools, then one call
//!   without tools, and stops at the token budget;
//! * a provider error is returned as an error. Nothing is invented in its place, and no
//!   assistant message is stored for it.

mod history;
mod loader;
mod notes;
mod prompt;
mod turn;

pub use loader::{DbProviderLoader, LoadError, ProviderLoader};

use crate::exec::Executor;
use ai::{AiError, AiProvider};
use db::{Connection, ConversationId, OrgId, UserId};
use i18n::{Catalog, Locale};
use std::fmt;

/// Limits of one turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TurnConfig {
    /// Model calls that may offer tools. After them comes one call without tools.
    pub max_rounds: u8,
    /// Stored messages replayed as context (the new message comes on top).
    pub history_messages: u32,
    /// Tokens one turn may spend. Counts what the provider reports, else a bytes/4 estimate.
    pub token_budget: u64,
    /// Longest person message accepted, in bytes.
    pub max_message_bytes: usize,
    /// Tool calls executed from one model response; the rest are answered with a refusal.
    pub max_calls_per_round: usize,
    /// Seconds after which a turn's claim counts as abandoned.
    pub claim_stale_secs: u32,
    /// Output token cap per model call; the provider default applies when `None`.
    pub max_tokens: Option<u32>,
}

impl Default for TurnConfig {
    fn default() -> Self {
        Self {
            max_rounds: 6,
            history_messages: 20,
            token_budget: 100_000,
            max_message_bytes: 16 * 1024,
            max_calls_per_round: 8,
            claim_stale_secs: 600,
            max_tokens: None,
        }
    }
}

/// One person message to answer.
#[derive(Debug)]
pub struct Turn<'a> {
    /// The organization. Every read and write is scoped to it.
    pub org: &'a OrgId,
    /// The signed-in person. Only the owner of a conversation can continue it.
    pub user: &'a UserId,
    /// The conversation to continue.
    pub conversation: &'a ConversationId,
    /// What the person typed. Redacted before use.
    pub message: &'a str,
    /// The person's language: selects the system prompt and the language of tool text.
    pub locale: Locale,
}

/// What one finished turn did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnReport {
    /// The answer, as stored (redacted).
    pub reply: String,
    /// Model calls made, including a final tool-less one.
    pub model_calls: u32,
    /// Tool calls the model asked for (executed or refused).
    pub tool_calls: u32,
    /// Tokens counted against the budget.
    pub tokens_used: u64,
    /// Whether any of them were estimated (bytes/4) because the provider did not report usage.
    pub tokens_estimated: bool,
    /// The model hit its output limit: the answer may be cut off.
    pub cut_off: bool,
    /// The round limit was reached and the answer came from the tool-less final call.
    pub forced_final: bool,
    /// Secrets redacted from the person's message.
    pub redactions: u32,
}

/// Why a turn did not produce an answer. Fixed codes: nothing here carries message text, server
/// text, keys or database detail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnError {
    /// No such conversation for this person in this org (a conversation of another org or user
    /// is indistinguishable from a missing one).
    NotFound,
    /// Another turn is running in this conversation.
    Busy,
    /// The conversation is archived.
    Archived,
    /// The message is empty.
    EmptyMessage,
    /// The message is longer than the limit.
    MessageTooLarge,
    /// The message contains a NUL character.
    BadMessage,
    /// This turn lost its claim (it went stale and another turn took over).
    ClaimLost,
    /// The provider failed. Nothing was stored in place of an answer.
    Provider(AiError),
    /// The token budget of the turn is spent.
    BudgetExceeded {
        /// Tokens counted.
        used: u64,
        /// The budget.
        budget: u64,
        /// Whether the count includes estimates.
        estimated: bool,
    },
    /// The model ended the turn without any text to show.
    NoAnswer,
    /// The database failed.
    Store,
}

impl TurnError {
    /// Stable machine code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::NotFound => "conversation_not_found",
            Self::Busy => "turn_busy",
            Self::Archived => "conversation_archived",
            Self::EmptyMessage => "message_empty",
            Self::MessageTooLarge => "message_too_large",
            Self::BadMessage => "message_invalid",
            Self::ClaimLost => "turn_claim_lost",
            Self::Provider(e) => e.code(),
            Self::BudgetExceeded { .. } => "token_budget_exceeded",
            Self::NoAnswer => "no_answer",
            Self::Store => "store_failed",
        }
    }
}

impl fmt::Display for TurnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for TurnError {}

/// Runs turns for one provider and one tool set.
pub struct Assistant<'a, P: AiProvider> {
    pub(crate) conn: &'a Connection,
    pub(crate) provider: &'a P,
    pub(crate) executor: &'a Executor,
    pub(crate) catalog: &'a Catalog,
    pub(crate) config: TurnConfig,
}

impl<'a, P: AiProvider> Assistant<'a, P> {
    /// An assistant with the default limits.
    #[must_use]
    pub fn new(
        conn: &'a Connection,
        provider: &'a P,
        executor: &'a Executor,
        catalog: &'a Catalog,
    ) -> Self {
        Self {
            conn,
            provider,
            executor,
            catalog,
            config: TurnConfig::default(),
        }
    }

    /// Replaces the limits.
    #[must_use]
    pub fn with_config(mut self, config: TurnConfig) -> Self {
        self.config = config;
        self
    }
}
