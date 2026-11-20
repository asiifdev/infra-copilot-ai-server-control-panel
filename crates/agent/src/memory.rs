//! The memory and search seam of the tools (ADR 0026, M2-e).
//!
//! Like [`crate::ProposalSink`] it is kept apart from [`crate::ReadStore`]: the read seam has no
//! write method, and this seam can only
//! * search what the org's tables and its ACTIVE memories hold,
//! * list ACTIVE memories, and
//! * create a PENDING memory.
//!
//! There is no approve, activate, edit or delete method here, so no tool and no model can make a
//! note visible to a later turn. Only a person can (the command line calls `db::approve_memory`).

use db::{Connection, ConversationId, Error as DbError, MemoryKind, OrgId, UserId};
use std::fmt;

/// At most this many memories may come from one conversation (pending or not).
pub const MAX_MEMORIES_PER_CONVERSATION: u32 = 5;
/// At most this many memories may wait for a person in one org.
pub const MAX_PENDING_PER_ORG: u32 = 50;
/// Most active memories `list_memories` shows.
pub const LIST_LIMIT: u32 = 50;

/// A memory as a tool proposes it. The text is cleaned again before it is stored.
#[derive(Debug)]
pub struct MemoryDraft<'a> {
    /// Kind.
    pub kind: MemoryKind,
    /// Text, already redacted and bounded by the tool.
    pub content: &'a str,
}

/// An active memory as tools see it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryNote {
    /// Id.
    pub id: String,
    /// Kind word (`org_fact`, `task`, `preference`).
    pub kind: &'static str,
    /// Text.
    pub content: String,
}

/// Why a memory call failed. Fixed codes, safe to show to a model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryError {
    /// This conversation already produced the maximum number of memories.
    TooManyInConversation,
    /// The org already has the maximum number of memories waiting for a person.
    TooManyPending,
    /// The org holds the maximum number of memories.
    OrgFull,
    /// The text is empty or too long after cleaning, or the kind needs a person this context
    /// does not have.
    Invalid(&'static str),
    /// This context has no memory store, or the store failed.
    Unavailable,
}

impl fmt::Display for MemoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TooManyInConversation => "too_many_memories_in_conversation",
            Self::TooManyPending => "too_many_pending_memories",
            Self::OrgFull => "memory_limit_reached",
            Self::Invalid(why) => why,
            Self::Unavailable => "memory_unavailable",
        })
    }
}

impl std::error::Error for MemoryError {}

/// Search and memory access for tools. Every call names the org, like [`crate::ReadStore`].
pub trait MemorySink {
    /// Ranked hits over the org's current inventory items, latest findings and active memories.
    ///
    /// # Errors
    /// [`MemoryError::Unavailable`] when nothing can be searched.
    fn search(
        &self,
        org: &OrgId,
        query: &str,
        limit: u32,
    ) -> Result<Vec<db::SearchHit>, MemoryError>;

    /// Active memories visible to the running turn (org notes and the asker's preferences).
    ///
    /// # Errors
    /// [`MemoryError::Unavailable`] when the store cannot answer.
    fn active(&self, org: &OrgId) -> Result<Vec<MemoryNote>, MemoryError>;

    /// How many memories wait for a person in this org.
    ///
    /// # Errors
    /// [`MemoryError::Unavailable`] when the store cannot answer.
    fn pending_count(&self, org: &OrgId) -> Result<u32, MemoryError>;

    /// Records a **pending** memory and returns its id. It stays invisible to models and search
    /// until a person approves it.
    ///
    /// # Errors
    /// [`MemoryError`] when a cap is reached or the text is unusable.
    fn create_pending(&self, org: &OrgId, draft: &MemoryDraft<'_>) -> Result<String, MemoryError>;
}

/// A sink with nothing in it: search finds nothing and nothing can be remembered.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoMemory;

impl MemorySink for NoMemory {
    fn search(
        &self,
        _org: &OrgId,
        _query: &str,
        _limit: u32,
    ) -> Result<Vec<db::SearchHit>, MemoryError> {
        Ok(Vec::new())
    }

    fn active(&self, _org: &OrgId) -> Result<Vec<MemoryNote>, MemoryError> {
        Ok(Vec::new())
    }

    fn pending_count(&self, _org: &OrgId) -> Result<u32, MemoryError> {
        Ok(0)
    }

    fn create_pending(
        &self,
        _org: &OrgId,
        _draft: &MemoryDraft<'_>,
    ) -> Result<String, MemoryError> {
        Err(MemoryError::Unavailable)
    }
}

/// [`MemorySink`] over the SQLite layer, bound to the person and conversation of the running turn
/// (the model cannot name another one).
pub struct DbMemorySink<'a> {
    conn: &'a Connection,
    conversation: Option<ConversationId>,
    user: Option<UserId>,
}

impl<'a> DbMemorySink<'a> {
    /// Reads and writes through `conn` on behalf of `user`, linking new memories to
    /// `conversation`.
    #[must_use]
    pub fn new(
        conn: &'a Connection,
        conversation: Option<ConversationId>,
        user: Option<UserId>,
    ) -> Self {
        Self {
            conn,
            conversation,
            user,
        }
    }
}

fn unavailable(_: DbError) -> MemoryError {
    MemoryError::Unavailable
}

impl MemorySink for DbMemorySink<'_> {
    fn search(
        &self,
        org: &OrgId,
        query: &str,
        limit: u32,
    ) -> Result<Vec<db::SearchHit>, MemoryError> {
        db::search(self.conn, org, query, limit).map_err(unavailable)
    }

    fn active(&self, org: &OrgId) -> Result<Vec<MemoryNote>, MemoryError> {
        // Without a person there are no preferences to show, and org notes need no person: use
        // the org-wide listing then.
        let rows = match &self.user {
            Some(user) => db::prompt_memories(self.conn, org, user, LIST_LIMIT),
            None => db::list_memories(self.conn, org, Some(db::MemoryStatus::Active))
                .map(|rows| rows.into_iter().filter(|r| r.user_id.is_none()).collect()),
        }
        .map_err(unavailable)?;
        Ok(rows
            .into_iter()
            .map(|r| MemoryNote {
                id: r.id.as_str().to_owned(),
                kind: r.kind.as_str(),
                content: r.content,
            })
            .collect())
    }

    fn pending_count(&self, org: &OrgId) -> Result<u32, MemoryError> {
        db::count_memories(self.conn, org, Some(db::MemoryStatus::Pending), None)
            .map_err(unavailable)
    }

    fn create_pending(&self, org: &OrgId, draft: &MemoryDraft<'_>) -> Result<String, MemoryError> {
        if let Some(conversation) = &self.conversation
            && db::count_memories(self.conn, org, None, Some(conversation)).map_err(unavailable)?
                >= MAX_MEMORIES_PER_CONVERSATION
        {
            return Err(MemoryError::TooManyInConversation);
        }
        if self.pending_count(org)? >= MAX_PENDING_PER_ORG {
            return Err(MemoryError::TooManyPending);
        }
        // A preference belongs to the person who is asking; without one there is nobody to own it.
        let user = if draft.kind == MemoryKind::Preference {
            Some(
                self.user
                    .as_ref()
                    .ok_or(MemoryError::Invalid("no_person_for_preference"))?,
            )
        } else {
            None
        };
        let id = db::save_memory(
            self.conn,
            org,
            &db::NewMemory {
                user,
                kind: draft.kind,
                content: draft.content,
                source: db::MemorySource::Assistant,
                conversation: self.conversation.as_ref(),
                status: db::MemoryStatus::Pending,
            },
        )
        .map_err(|e| match e {
            DbError::Invalid("memory_limit") => MemoryError::OrgFull,
            DbError::Invalid("memory_empty") => MemoryError::Invalid("empty_text"),
            DbError::Invalid("memory_too_long") => MemoryError::Invalid("text_too_long"),
            _ => MemoryError::Unavailable,
        })?;
        Ok(id.as_str().to_owned())
    }
}
