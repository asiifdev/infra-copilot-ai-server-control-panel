//! Organization memories (docs/design.md 7.4, ADR 0026): short notes a person wants the assistant
//! to know, or that the assistant proposes.
//!
//! Trust model, enforced here and in the schema:
//! * a memory written by the assistant is `pending`; the schema refuses an active one;
//! * only [`approve_memory`] (called by a person's action, never by a tool) activates it;
//! * pending memories are never returned by [`prompt_memories`] and never reach the search index;
//! * text is redacted, control characters are replaced, and the length is checked after that;
//! * an org holds at most [`MAX_MEMORIES_PER_ORG`] memories.
//!
//! Every function takes an [`OrgId`] and filters on it.

use crate::error::{Error, Result};
use crate::ids::{ConversationId, MemoryId, OrgId, UserId};
use crate::{new_uuid, now_utc};
use core_domain::redact::redact;
use rusqlite::{Connection, OptionalExtension, params};

/// Most memories (pending and active together) one org may hold.
pub const MAX_MEMORIES_PER_ORG: u32 = 200;
/// Longest memory text, in characters, after redaction.
pub const MAX_MEMORY_CHARS: usize = 1000;

text_enum!(
    /// What a memory is about.
    MemoryKind {
        /// A standing fact about the organization.
        OrgFact => "org_fact",
        /// Context of a task in progress.
        Task => "task",
        /// A personal preference of one person.
        Preference => "preference",
    }
);

text_enum!(
    /// Who wrote a memory.
    MemorySource {
        /// A person, through the command line or the panel.
        Person => "person",
        /// The assistant, through the `remember` tool.
        Assistant => "assistant",
    }
);

text_enum!(
    /// Whether a memory may be shown to a model and searched.
    MemoryStatus {
        /// Waiting for a person; invisible to models and search.
        Pending => "pending",
        /// Approved by a person.
        Active => "active",
    }
);

/// A memory to store.
#[derive(Debug)]
pub struct NewMemory<'a> {
    /// The owner of a personal preference; required for [`MemoryKind::Preference`], forbidden
    /// otherwise.
    pub user: Option<&'a UserId>,
    /// Kind.
    pub kind: MemoryKind,
    /// Text. Redacted and cleaned by [`save_memory`].
    pub content: &'a str,
    /// Who wrote it.
    pub source: MemorySource,
    /// The conversation it came from, if any.
    pub conversation: Option<&'a ConversationId>,
    /// Initial status. [`MemorySource::Assistant`] must use [`MemoryStatus::Pending`].
    pub status: MemoryStatus,
}

/// A stored memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryRow {
    /// Id.
    pub id: MemoryId,
    /// Owner of a personal preference.
    pub user_id: Option<UserId>,
    /// Kind.
    pub kind: MemoryKind,
    /// Text.
    pub content: String,
    /// Who wrote it.
    pub source: MemorySource,
    /// Pending or active.
    pub status: MemoryStatus,
    /// The conversation it came from.
    pub conversation_id: Option<ConversationId>,
    /// Creation time.
    pub created_at: String,
    /// Last change.
    pub updated_at: String,
}

/// Redacts, replaces control characters with spaces, trims, and checks the length (after
/// redaction, which can lengthen text).
///
/// # Errors
/// [`Error::Invalid`] when the result is empty or longer than [`MAX_MEMORY_CHARS`].
pub fn clean_memory_text(raw: &str) -> Result<String> {
    let clean: String = redact(raw)
        .text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let clean = clean.trim().to_owned();
    if clean.is_empty() {
        return Err(Error::Invalid("memory_empty"));
    }
    if clean.chars().count() > MAX_MEMORY_CHARS {
        return Err(Error::Invalid("memory_too_long"));
    }
    Ok(clean)
}

/// Stores a memory.
///
/// # Errors
/// [`Error::Invalid`] with `memory_empty`, `memory_too_long`, `memory_user_mismatch` (a
/// preference needs a user, other kinds must not have one), `memory_assistant_active` (the
/// assistant cannot write an active memory) or `memory_limit` (the org already holds
/// [`MAX_MEMORIES_PER_ORG`]); [`Error::NotFound`] when the user is not a member of the org;
/// a constraint violation for a conversation of another org.
pub fn save_memory(conn: &Connection, org: &OrgId, new: &NewMemory<'_>) -> Result<MemoryId> {
    let content = clean_memory_text(new.content)?;
    if (new.kind == MemoryKind::Preference) != new.user.is_some() {
        return Err(Error::Invalid("memory_user_mismatch"));
    }
    if new.source == MemorySource::Assistant && new.status != MemoryStatus::Pending {
        return Err(Error::Invalid("memory_assistant_active"));
    }
    let id = new_uuid(conn)?;
    // One statement: the cap and the membership check cannot race with another writer.
    let n = conn.execute(
        "INSERT INTO memories (id, org_id, user_id, kind, content, source, status, conversation_id) \
         SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8 \
         WHERE (SELECT count(*) FROM memories WHERE org_id = ?2) < ?9 \
           AND (?3 IS NULL OR EXISTS \
                (SELECT 1 FROM memberships WHERE org_id = ?2 AND user_id = ?3))",
        params![
            id,
            org.as_str(),
            new.user.map(UserId::as_str),
            new.kind.as_str(),
            content,
            new.source.as_str(),
            new.status.as_str(),
            new.conversation.map(ConversationId::as_str),
            MAX_MEMORIES_PER_ORG
        ],
    )?;
    if n == 0 {
        return if count_memories(conn, org, None, None)? >= MAX_MEMORIES_PER_ORG {
            Err(Error::Invalid("memory_limit"))
        } else {
            Err(Error::NotFound)
        };
    }
    Ok(MemoryId::from_db(id))
}

const COLUMNS: &str =
    "id, user_id, kind, content, source, status, conversation_id, created_at, updated_at";

type Raw = (
    String,
    Option<String>,
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    String,
);

fn map(row: &rusqlite::Row<'_>) -> rusqlite::Result<Raw> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
    ))
}

fn build(t: Raw) -> Result<MemoryRow> {
    Ok(MemoryRow {
        id: MemoryId::from_db(t.0),
        user_id: t.1.map(UserId::from_db),
        kind: MemoryKind::parse(&t.2)?,
        content: t.3,
        source: MemorySource::parse(&t.4)?,
        status: MemoryStatus::parse(&t.5)?,
        conversation_id: t.6.map(ConversationId::from_db),
        created_at: t.7,
        updated_at: t.8,
    })
}

/// One memory of the org.
///
/// # Errors
/// [`Error::NotFound`] when it is missing or belongs to another org.
pub fn get_memory(conn: &Connection, org: &OrgId, id: &MemoryId) -> Result<MemoryRow> {
    let raw = conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM memories WHERE org_id = ?1 AND id = ?2"),
            params![org.as_str(), id.as_str()],
            map,
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    build(raw)
}

/// The org's memories, oldest first, optionally only one status.
///
/// # Errors
/// Database failure.
pub fn list_memories(
    conn: &Connection,
    org: &OrgId,
    status: Option<MemoryStatus>,
) -> Result<Vec<MemoryRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM memories WHERE org_id = ?1 AND (?2 IS NULL OR status = ?2) \
         ORDER BY created_at, id LIMIT ?3"
    ))?;
    let rows = stmt.query_map(
        params![
            org.as_str(),
            status.map(MemoryStatus::as_str),
            MAX_MEMORIES_PER_ORG
        ],
        map,
    )?;
    rows.map(|r| build(r?)).collect()
}

/// The ACTIVE memories a prompt may show for `user`: org-wide notes plus that user's own
/// preferences, most recently changed first, at most `limit`. Pending memories are never
/// returned.
///
/// # Errors
/// Database failure.
pub fn prompt_memories(
    conn: &Connection,
    org: &OrgId,
    user: &UserId,
    limit: u32,
) -> Result<Vec<MemoryRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM memories \
         WHERE org_id = ?1 AND status = 'active' AND (user_id IS NULL OR user_id = ?2) \
         ORDER BY updated_at DESC, id LIMIT ?3"
    ))?;
    let rows = stmt.query_map(params![org.as_str(), user.as_str(), limit], map)?;
    rows.map(|r| build(r?)).collect()
}

/// Counts memories of the org, optionally of one status and/or one conversation.
///
/// # Errors
/// Database failure.
pub fn count_memories(
    conn: &Connection,
    org: &OrgId,
    status: Option<MemoryStatus>,
    conversation: Option<&ConversationId>,
) -> Result<u32> {
    Ok(conn.query_row(
        "SELECT count(*) FROM memories WHERE org_id = ?1 \
         AND (?2 IS NULL OR status = ?2) AND (?3 IS NULL OR conversation_id = ?3)",
        params![
            org.as_str(),
            status.map(MemoryStatus::as_str),
            conversation.map(ConversationId::as_str)
        ],
        |row| row.get(0),
    )?)
}

/// Activates a memory: the only way a pending one becomes visible to a model. This is a person's
/// decision; no tool may call it. Approving an active memory changes nothing.
///
/// # Errors
/// [`Error::NotFound`] when it is not in this org.
pub fn approve_memory(conn: &Connection, org: &OrgId, id: &MemoryId) -> Result<()> {
    let n = conn.execute(
        "UPDATE memories SET status = 'active', updated_at = ?3 \
         WHERE org_id = ?1 AND id = ?2 AND status = 'pending'",
        params![org.as_str(), id.as_str(), now_utc(conn)?],
    )?;
    if n == 0 {
        // Already active is fine; missing (or another org's) is not.
        get_memory(conn, org, id).map(|_| ())
    } else {
        Ok(())
    }
}

/// Deletes a memory (and its search document).
///
/// # Errors
/// [`Error::NotFound`] when it is not in this org.
pub fn delete_memory(conn: &Connection, org: &OrgId, id: &MemoryId) -> Result<()> {
    let n = conn.execute(
        "DELETE FROM memories WHERE org_id = ?1 AND id = ?2",
        params![org.as_str(), id.as_str()],
    )?;
    if n == 0 { Err(Error::NotFound) } else { Ok(()) }
}
