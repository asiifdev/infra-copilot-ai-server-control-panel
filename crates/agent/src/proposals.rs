//! The only write a tool can make in M2: a pending proposal, through [`ProposalSink`].
//!
//! Kept apart from [`crate::ReadStore`] on purpose: the read seam has no write method at all, and
//! this seam can only *create pending records*. There is no approve, execute or edit method here,
//! so no tool and no model can reach one (ADR 0023; the approval flow is M3).

use core_domain::tier::Tier;
use db::{Connection, ConversationId, OrgId};
use std::fmt;

/// At most this many pending proposals per conversation: a model that proposes in a loop fills a
/// review queue, not the table.
pub const MAX_PENDING_PER_CONVERSATION: u32 = 20;

/// A proposal as a tool describes it. Text is already bounded by the tool's schema.
#[derive(Debug)]
pub struct ProposalDraft<'a> {
    /// The creating tool.
    pub tool: &'static str,
    /// Its tier.
    pub tier: Tier,
    /// Short title.
    pub title: &'a str,
    /// Why.
    pub rationale: &'a str,
    /// What a person could do.
    pub suggested_action: &'a str,
    /// What the action would change (from the tool's declaration, not from the model).
    pub impact: &'static str,
    /// Related parity rule.
    pub rule_id: Option<&'a str>,
    /// Related item.
    pub subject_key: Option<&'a str>,
    /// SHA-256 hex of the canonical arguments.
    pub args_hash: &'a str,
}

/// Why a proposal was not recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SinkError {
    /// The conversation already holds the maximum number of pending proposals.
    TooManyPending,
    /// This context cannot record proposals, or the store failed.
    Unavailable,
}

impl fmt::Display for SinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TooManyPending => "too_many_pending_proposals",
            Self::Unavailable => "proposals_unavailable",
        })
    }
}

impl std::error::Error for SinkError {}

/// Where a tool records a proposal. Every call names the org, like [`crate::ReadStore`].
pub trait ProposalSink {
    /// Records a **pending** proposal and returns its id.
    ///
    /// # Errors
    /// [`SinkError`] when the proposal cannot be recorded.
    fn create_pending(&self, org: &OrgId, draft: &ProposalDraft<'_>) -> Result<String, SinkError>;
}

/// A sink that records nothing: for contexts (tests, read-only callers) with nowhere to write.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoProposals;

impl ProposalSink for NoProposals {
    fn create_pending(
        &self,
        _org: &OrgId,
        _draft: &ProposalDraft<'_>,
    ) -> Result<String, SinkError> {
        Err(SinkError::Unavailable)
    }
}

/// [`ProposalSink`] over the SQLite layer, bound to the conversation of the running turn (the
/// model cannot name another one).
pub struct DbProposalSink<'a> {
    conn: &'a Connection,
    conversation: Option<ConversationId>,
}

impl<'a> DbProposalSink<'a> {
    /// Records proposals through `conn`, linked to `conversation`.
    #[must_use]
    pub fn new(conn: &'a Connection, conversation: Option<ConversationId>) -> Self {
        Self { conn, conversation }
    }
}

impl ProposalSink for DbProposalSink<'_> {
    fn create_pending(&self, org: &OrgId, draft: &ProposalDraft<'_>) -> Result<String, SinkError> {
        if let Some(conversation) = &self.conversation {
            let pending = db::count_pending_proposals(self.conn, org, conversation)
                .map_err(|_| SinkError::Unavailable)?;
            if pending >= MAX_PENDING_PER_CONVERSATION {
                return Err(SinkError::TooManyPending);
            }
        }
        let id = db::create_proposal(
            self.conn,
            org,
            &db::NewProposal {
                conversation: self.conversation.as_ref(),
                tool: draft.tool,
                tier: draft.tier.level(),
                title: draft.title,
                rationale: draft.rationale,
                suggested_action: draft.suggested_action,
                impact: draft.impact,
                rule_id: draft.rule_id,
                subject_key: draft.subject_key,
                args_hash: draft.args_hash,
            },
        )
        .map_err(|_| SinkError::Unavailable)?;
        Ok(id.as_str().to_owned())
    }
}
