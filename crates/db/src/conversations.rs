//! Assistant conversations, their messages, the record of every tool call, and proposals
//! (docs/design.md 3.5 and 3.6, ADR 0023).
//!
//! Rules enforced here:
//! * every query filters on the org, and child rows carry `(org_id, id)` foreign keys;
//! * only the holder of the turn claim can append messages, so a second or stale turn cannot
//!   interleave with the running one;
//! * a message group (an assistant message, its tool results and their `tool_calls` rows) is
//!   written in one transaction, so history never contains half a round.

use crate::error::{Error, Result};
use crate::ids::{AiProviderId, ApprovalId, ConversationId, OrgId, ProposalId, UserId};
use crate::{new_uuid, now_utc};
use core_domain::redact::redact;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

text_enum!(
    /// Who wrote a stored message.
    MessageRole {
        /// Standing instructions.
        System => "system",
        /// The person.
        User => "user",
        /// The model.
        Assistant => "assistant",
        /// The result of a tool call.
        Tool => "tool",
    }
);

text_enum!(
    /// Lifecycle of a proposal. M2 only ever creates `pending`; the rest is reserved for the
    /// approval flow (docs/design.md 9) so the stored values do not change later.
    ProposalStatus {
        /// Waiting for a human.
        Pending => "pending",
        /// A human approved it.
        Approved => "approved",
        /// A human rejected it.
        Rejected => "rejected",
        /// It ran.
        Executed => "executed",
        /// It ran and failed.
        Failed => "failed",
        /// Nobody decided in time.
        Expired => "expired",
    }
);

/// How long a new proposal stays open.
const PROPOSAL_TTL_DAYS: i64 = 7;

/// A stored conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationRow {
    /// Id.
    pub id: ConversationId,
    /// The person who owns it.
    pub user_id: UserId,
    /// Title (redacted).
    pub title: String,
    /// Provider chosen for it; `None` means the org default.
    pub provider_id: Option<AiProviderId>,
    /// Creation time.
    pub created_at: String,
    /// Archive time, if archived.
    pub archived_at: Option<String>,
}

const CONV_COLUMNS: &str = "id, user_id, title, provider_id, created_at, archived_at";

fn map_conversation(row: &rusqlite::Row<'_>) -> rusqlite::Result<ConversationRow> {
    Ok(ConversationRow {
        id: ConversationId::from_db(row.get(0)?),
        user_id: UserId::from_db(row.get(1)?),
        title: row.get(2)?,
        provider_id: row.get::<_, Option<String>>(3)?.map(AiProviderId::from_db),
        created_at: row.get(4)?,
        archived_at: row.get(5)?,
    })
}

/// Starts a conversation for `user`. The title is redacted before it is stored and capped at 200
/// characters.
///
/// # Errors
/// [`Error::NotFound`] when `user` is not a member of `org`; [`Error::Invalid`] for an empty
/// title; a constraint violation for a provider that is not in this org.
pub fn create_conversation(
    conn: &Connection,
    org: &OrgId,
    user: &UserId,
    title: &str,
    provider: Option<&AiProviderId>,
) -> Result<ConversationId> {
    let title: String = redact(title).text.chars().take(200).collect();
    if title.trim().is_empty() {
        return Err(Error::Invalid("conversation title"));
    }
    let id = new_uuid(conn)?;
    let n = conn.execute(
        "INSERT INTO conversations (id, org_id, user_id, title, provider_id) \
         SELECT ?1, ?2, ?3, ?4, ?5 \
         WHERE EXISTS (SELECT 1 FROM memberships WHERE org_id = ?2 AND user_id = ?3)",
        params![
            id,
            org.as_str(),
            user.as_str(),
            title,
            provider.map(AiProviderId::as_str)
        ],
    )?;
    if n == 0 {
        return Err(Error::NotFound);
    }
    Ok(ConversationId::from_db(id))
}

/// One conversation of the org.
///
/// # Errors
/// [`Error::NotFound`] when it is missing or belongs to another org.
pub fn get_conversation(
    conn: &Connection,
    org: &OrgId,
    id: &ConversationId,
) -> Result<ConversationRow> {
    conn.query_row(
        &format!("SELECT {CONV_COLUMNS} FROM conversations WHERE org_id = ?1 AND id = ?2"),
        params![org.as_str(), id.as_str()],
        map_conversation,
    )
    .optional()?
    .ok_or(Error::NotFound)
}

/// Longest derived title, in characters (an ellipsis is added when the text was cut).
const DERIVED_TITLE_CHARS: usize = 60;

/// Names a conversation after the text of its first message, but only while the title is still one
/// of the `placeholders` (the localized "new conversation" texts): a title a person chose, or one
/// set before, is never replaced. The text is redacted, control characters and runs of white space
/// become single spaces, and it is cut at [`DERIVED_TITLE_CHARS`] characters. Returns whether the
/// title changed. The check and the change are one statement, so two concurrent first messages
/// cannot both win.
///
/// # Errors
/// [`Error::NotFound`] when the conversation is not in this org; database failure.
pub fn retitle_conversation(
    conn: &Connection,
    org: &OrgId,
    id: &ConversationId,
    placeholders: &[&str],
    from_text: &str,
) -> Result<bool> {
    let clean: String = redact(from_text)
        .text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let words: Vec<&str> = clean.split_whitespace().collect();
    let flat = words.join(" ");
    let mut title: String = flat.chars().take(DERIVED_TITLE_CHARS).collect();
    if title.is_empty() {
        return Ok(false);
    }
    if flat.chars().count() > DERIVED_TITLE_CHARS {
        title.push('\u{2026}');
    }
    let known = get_conversation(conn, org, id)?;
    if !placeholders.contains(&known.title.as_str()) {
        return Ok(false);
    }
    let changed = conn.execute(
        "UPDATE conversations SET title = ?3 WHERE org_id = ?1 AND id = ?2 AND title = ?4",
        params![org.as_str(), id.as_str(), title, known.title],
    )?;
    Ok(changed == 1)
}

/// The conversations of one user, newest first.
///
/// # Errors
/// Database failure.
pub fn list_conversations(
    conn: &Connection,
    org: &OrgId,
    user: &UserId,
    include_archived: bool,
    limit: u32,
) -> Result<Vec<ConversationRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {CONV_COLUMNS} FROM conversations \
         WHERE org_id = ?1 AND user_id = ?2 AND (?3 = 1 OR archived_at IS NULL) \
         ORDER BY created_at DESC, id LIMIT ?4"
    ))?;
    let rows = stmt.query_map(
        params![
            org.as_str(),
            user.as_str(),
            i64::from(include_archived),
            i64::from(limit)
        ],
        map_conversation,
    )?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Archives a conversation: it stays readable but no turn can run in it.
///
/// # Errors
/// [`Error::NotFound`] when it is not in this org.
pub fn archive_conversation(conn: &Connection, org: &OrgId, id: &ConversationId) -> Result<()> {
    let n = conn.execute(
        "UPDATE conversations SET archived_at = COALESCE(archived_at, ?3) \
         WHERE org_id = ?1 AND id = ?2",
        params![org.as_str(), id.as_str(), now_utc(conn)?],
    )?;
    if n == 0 { Err(Error::NotFound) } else { Ok(()) }
}

/// Deletes a conversation with its messages and tool call records.
///
/// # Errors
/// [`Error::NotFound`] when it is not in this org; a constraint violation while proposals still
/// refer to it (archive it instead).
pub fn delete_conversation(conn: &Connection, org: &OrgId, id: &ConversationId) -> Result<()> {
    let n = conn.execute(
        "DELETE FROM conversations WHERE org_id = ?1 AND id = ?2",
        params![org.as_str(), id.as_str()],
    )?;
    if n == 0 { Err(Error::NotFound) } else { Ok(()) }
}

// ---- the turn claim --------------------------------------------------------------------------

/// Proof that the holder is the one running a turn. Needed to append messages.
#[derive(Debug, PartialEq, Eq)]
pub struct TurnClaim(String);

impl TurnClaim {
    /// The opaque token.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Result of [`claim_turn`].
#[derive(Debug, PartialEq, Eq)]
pub enum ClaimOutcome {
    /// The caller may run the turn.
    Claimed(TurnClaim),
    /// Another turn is running (its claim is not stale).
    Busy,
}

/// Claims the right to run one turn, atomically: a single `UPDATE ... WHERE` succeeds for exactly
/// one caller. A claim whose last heartbeat is older than `stale_secs` is taken over, so a crashed
/// turn does not block the conversation forever.
///
/// # Errors
/// [`Error::NotFound`] when the conversation is not in this org; [`Error::Invalid`] when it is
/// archived.
pub fn claim_turn(
    conn: &Connection,
    org: &OrgId,
    id: &ConversationId,
    stale_secs: u32,
) -> Result<ClaimOutcome> {
    let token = new_uuid(conn)?;
    let n = conn.execute(
        "UPDATE conversations SET turn_token = ?3, turn_claimed_at = ?4 \
         WHERE org_id = ?1 AND id = ?2 AND archived_at IS NULL \
           AND (turn_token IS NULL \
                OR turn_claimed_at <= strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-' || ?5 || ' seconds'))",
        params![
            org.as_str(),
            id.as_str(),
            token,
            now_utc(conn)?,
            i64::from(stale_secs)
        ],
    )?;
    if n == 1 {
        return Ok(ClaimOutcome::Claimed(TurnClaim(token)));
    }
    let conversation = get_conversation(conn, org, id)?;
    if conversation.archived_at.is_some() {
        return Err(Error::Invalid("conversation archived"));
    }
    Ok(ClaimOutcome::Busy)
}

/// Refreshes the heartbeat of a held claim.
///
/// # Errors
/// [`Error::NotFound`] when the claim is no longer held (it went stale and was taken over).
pub fn touch_turn(
    conn: &Connection,
    org: &OrgId,
    id: &ConversationId,
    claim: &TurnClaim,
) -> Result<()> {
    let n = conn.execute(
        "UPDATE conversations SET turn_claimed_at = ?4 \
         WHERE org_id = ?1 AND id = ?2 AND turn_token = ?3",
        params![org.as_str(), id.as_str(), claim.as_str(), now_utc(conn)?],
    )?;
    if n == 0 { Err(Error::NotFound) } else { Ok(()) }
}

/// Releases a claim. Releasing a claim that is no longer held is a no-op, so a slow turn that was
/// taken over cannot free its successor's claim.
///
/// # Errors
/// Database failure.
pub fn release_turn(
    conn: &Connection,
    org: &OrgId,
    id: &ConversationId,
    claim: &TurnClaim,
) -> Result<()> {
    conn.execute(
        "UPDATE conversations SET turn_token = NULL, turn_claimed_at = NULL \
         WHERE org_id = ?1 AND id = ?2 AND turn_token = ?3",
        params![org.as_str(), id.as_str(), claim.as_str()],
    )?;
    Ok(())
}

// ---- messages and tool calls -----------------------------------------------------------------

/// A message to append.
#[derive(Debug, Clone)]
pub struct NewMessage<'a> {
    /// Author.
    pub role: MessageRole,
    /// Redacted text.
    pub content: &'a str,
    /// JSON array of calls, for an assistant message that calls tools.
    pub tool_calls_json: Option<&'a str>,
    /// For a tool message: the call it answers.
    pub tool_call_id: Option<&'a str>,
    /// JSON token usage and finish reason, if known.
    pub token_usage_json: Option<&'a str>,
    /// How many secrets were redacted from the content.
    pub redactions: u32,
}

/// The record of one tool call. It refers to the assistant message that asked for it.
#[derive(Debug, Clone)]
pub struct NewToolCall<'a> {
    /// Index, in the appended batch, of the assistant message that made the call.
    pub message_index: usize,
    /// Position of the call inside that message.
    pub idx: u32,
    /// The provider's call id.
    pub call_id: &'a str,
    /// Sanitised tool name.
    pub tool: &'a str,
    /// Tier of the tool, if the tool exists.
    pub tier: Option<u8>,
    /// Redacted, capped arguments.
    pub args: &'a str,
    /// Stable status code.
    pub status: &'a str,
    /// Wall time.
    pub duration_ms: u64,
    /// Secrets redacted from the result.
    pub redactions: u32,
    /// Whether the result was cut.
    pub truncated: bool,
}

/// A stored message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageRow {
    /// Position in the conversation, starting at 1.
    pub seq: i64,
    /// Author.
    pub role: MessageRole,
    /// Redacted text.
    pub content: String,
    /// JSON array of calls, if any.
    pub tool_calls_json: Option<String>,
    /// The call a tool message answers.
    pub tool_call_id: Option<String>,
    /// JSON usage, if any.
    pub token_usage_json: Option<String>,
    /// Secrets redacted from the content.
    pub redactions: i64,
    /// Time.
    pub created_at: String,
}

/// A stored tool call record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallRow {
    /// Sequence of the assistant message that asked.
    pub message_seq: i64,
    /// Position inside that message.
    pub idx: i64,
    /// The provider's call id.
    pub call_id: String,
    /// Tool name.
    pub tool: String,
    /// Tier, when the tool exists.
    pub tier: Option<u8>,
    /// Redacted arguments.
    pub args: String,
    /// Status code.
    pub status: String,
    /// Wall time.
    pub duration_ms: i64,
    /// Secrets redacted from the result.
    pub redactions: i64,
    /// Whether the result was cut.
    pub truncated: bool,
    /// Time.
    pub created_at: String,
}

/// Appends messages (and the tool call records that belong to them) in one transaction and
/// returns the sequence numbers given to the messages.
///
/// # Errors
/// [`Error::NotFound`] when the conversation is not in this org or `claim` is not the held claim
/// (a stale or foreign turn writes nothing); [`Error::Invalid`] for a bad `message_index`;
/// a constraint violation for out-of-range values.
pub fn append_messages(
    conn: &Connection,
    org: &OrgId,
    id: &ConversationId,
    claim: &TurnClaim,
    messages: &[NewMessage<'_>],
    calls: &[NewToolCall<'_>],
) -> Result<Vec<i64>> {
    if calls.iter().any(|c| c.message_index >= messages.len()) {
        return Err(Error::Invalid("tool call message index"));
    }
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let held: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM conversations \
         WHERE org_id = ?1 AND id = ?2 AND turn_token = ?3 AND archived_at IS NULL)",
        params![org.as_str(), id.as_str(), claim.as_str()],
        |row| row.get(0),
    )?;
    if !held {
        return Err(Error::NotFound);
    }
    let first: i64 = tx.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM messages WHERE org_id = ?1 AND conversation_id = ?2",
        params![org.as_str(), id.as_str()],
        |row| row.get(0),
    )?;
    let mut seqs = Vec::with_capacity(messages.len());
    for (next, m) in (first..).zip(messages) {
        tx.execute(
            "INSERT INTO messages (org_id, conversation_id, seq, role, content, tool_calls, \
             tool_call_id, token_usage, redactions) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                org.as_str(),
                id.as_str(),
                next,
                m.role.as_str(),
                m.content,
                m.tool_calls_json,
                m.tool_call_id,
                m.token_usage_json,
                i64::from(m.redactions)
            ],
        )?;
        seqs.push(next);
    }
    for c in calls {
        let message_seq = *seqs
            .get(c.message_index)
            .ok_or(Error::Invalid("tool call message index"))?;
        let row_id = new_uuid(&tx)?;
        tx.execute(
            "INSERT INTO tool_calls (id, org_id, conversation_id, message_seq, idx, call_id, tool, \
             tier, args, status, duration_ms, redactions, truncated) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                row_id,
                org.as_str(),
                id.as_str(),
                message_seq,
                i64::from(c.idx),
                c.call_id,
                c.tool,
                c.tier.map(i64::from),
                c.args,
                c.status,
                i64::try_from(c.duration_ms).unwrap_or(i64::MAX),
                i64::from(c.redactions),
                i64::from(c.truncated)
            ],
        )?;
    }
    tx.commit()?;
    Ok(seqs)
}

type RawMessage = (
    i64,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    i64,
    String,
);

/// The newest `last` messages of a conversation, oldest first.
///
/// # Errors
/// [`Error::NotFound`] when the conversation is not in this org.
pub fn list_messages(
    conn: &Connection,
    org: &OrgId,
    id: &ConversationId,
    last: u32,
) -> Result<Vec<MessageRow>> {
    get_conversation(conn, org, id)?;
    let mut stmt = conn.prepare(
        "SELECT seq, role, content, tool_calls, tool_call_id, token_usage, redactions, created_at \
         FROM (SELECT * FROM messages WHERE org_id = ?1 AND conversation_id = ?2 \
               ORDER BY seq DESC LIMIT ?3) ORDER BY seq",
    )?;
    let rows = stmt.query_map(
        params![org.as_str(), id.as_str(), i64::from(last)],
        |row| -> rusqlite::Result<RawMessage> {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
            ))
        },
    )?;
    let mut out = Vec::new();
    for r in rows {
        let r = r?;
        out.push(MessageRow {
            seq: r.0,
            role: MessageRole::parse(&r.1)?,
            content: r.2,
            tool_calls_json: r.3,
            tool_call_id: r.4,
            token_usage_json: r.5,
            redactions: r.6,
            created_at: r.7,
        });
    }
    Ok(out)
}

/// Tool call records of a conversation, in the order they were made.
///
/// # Errors
/// [`Error::NotFound`] when the conversation is not in this org.
pub fn list_tool_calls(
    conn: &Connection,
    org: &OrgId,
    id: &ConversationId,
) -> Result<Vec<ToolCallRow>> {
    get_conversation(conn, org, id)?;
    let mut stmt = conn.prepare(
        "SELECT message_seq, idx, call_id, tool, tier, args, status, duration_ms, redactions, \
         truncated, created_at FROM tool_calls WHERE org_id = ?1 AND conversation_id = ?2 \
         ORDER BY message_seq, idx",
    )?;
    let rows = stmt.query_map(params![org.as_str(), id.as_str()], |row| {
        Ok(ToolCallRow {
            message_seq: row.get(0)?,
            idx: row.get(1)?,
            call_id: row.get(2)?,
            tool: row.get(3)?,
            tier: row
                .get::<_, Option<i64>>(4)?
                .and_then(|t| u8::try_from(t).ok()),
            args: row.get(5)?,
            status: row.get(6)?,
            duration_ms: row.get(7)?,
            redactions: row.get(8)?,
            truncated: row.get::<_, i64>(9)? == 1,
            created_at: row.get(10)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

// ---- proposals -------------------------------------------------------------------------------

/// A proposal to store. Always created `pending`.
#[derive(Debug)]
pub struct NewProposal<'a> {
    /// The conversation it came from.
    pub conversation: Option<&'a ConversationId>,
    /// The tool that created it.
    pub tool: &'a str,
    /// Tier of that tool, 1 to 4.
    pub tier: u8,
    /// Short title.
    pub title: &'a str,
    /// Why.
    pub rationale: &'a str,
    /// What a person could do about it.
    pub suggested_action: &'a str,
    /// What the proposed action would change.
    pub impact: &'a str,
    /// Parity rule it relates to.
    pub rule_id: Option<&'a str>,
    /// Item it relates to.
    pub subject_key: Option<&'a str>,
    /// SHA-256 hex of the canonical arguments.
    pub args_hash: &'a str,
}

/// A stored proposal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposalRow {
    /// Id.
    pub id: ProposalId,
    /// Source conversation.
    pub conversation_id: Option<ConversationId>,
    /// Creating tool.
    pub tool: String,
    /// Tier.
    pub tier: u8,
    /// Title.
    pub title: String,
    /// Why.
    pub rationale: String,
    /// Suggested action.
    pub suggested_action: String,
    /// Impact.
    pub impact: String,
    /// Related rule.
    pub rule_id: Option<String>,
    /// Related item.
    pub subject_key: Option<String>,
    /// Hash of the arguments.
    pub args_hash: String,
    /// Status.
    pub status: ProposalStatus,
    /// Creation time.
    pub created_at: String,
    /// Expiry time.
    pub expires_at: String,
    /// The structured action (the `actions` crate's strict JSON), when the proposal has one.
    pub action_json: Option<String>,
    /// The node the action targets, when it has an action.
    pub node_id: Option<crate::ids::NodeId>,
    /// Cached redacted dry-run JSON, when a dry-run has been computed.
    pub dry_run_json: Option<String>,
    /// When `dry_run_json` was recorded.
    pub dry_run_at: Option<String>,
}

/// Stores a pending proposal that expires in seven days.
///
/// # Errors
/// A constraint violation for out-of-range text, a tier outside 1 to 4, or a conversation that is
/// not in this org.
pub fn create_proposal(
    conn: &Connection,
    org: &OrgId,
    new: &NewProposal<'_>,
) -> Result<ProposalId> {
    let id = new_uuid(conn)?;
    conn.execute(
        "INSERT INTO proposals (id, org_id, conversation_id, tool, tier, title, rationale, \
         suggested_action, impact, rule_id, subject_key, args_hash, expires_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, \
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '+' || ?13 || ' days'))",
        params![
            id,
            org.as_str(),
            new.conversation.map(ConversationId::as_str),
            new.tool,
            i64::from(new.tier),
            new.title,
            new.rationale,
            new.suggested_action,
            new.impact,
            new.rule_id,
            new.subject_key,
            new.args_hash,
            PROPOSAL_TTL_DAYS
        ],
    )?;
    Ok(ProposalId::from_db(id))
}

const PROPOSAL_COLUMNS: &str = "id, conversation_id, tool, tier, title, rationale, \
    suggested_action, impact, rule_id, subject_key, args_hash, status, created_at, expires_at, \
    action_json, node_id, dry_run_json, dry_run_at";

fn map_proposal(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProposalRow> {
    let status: String = row.get(11)?;
    Ok(ProposalRow {
        id: ProposalId::from_db(row.get(0)?),
        conversation_id: row
            .get::<_, Option<String>>(1)?
            .map(ConversationId::from_db),
        tool: row.get(2)?,
        tier: u8::try_from(row.get::<_, i64>(3)?).unwrap_or(u8::MAX),
        title: row.get(4)?,
        rationale: row.get(5)?,
        suggested_action: row.get(6)?,
        impact: row.get(7)?,
        rule_id: row.get(8)?,
        subject_key: row.get(9)?,
        args_hash: row.get(10)?,
        // The CHECK constraint guarantees a known value; fall back to the safest one.
        status: ProposalStatus::parse(&status).unwrap_or(ProposalStatus::Expired),
        created_at: row.get(12)?,
        expires_at: row.get(13)?,
        action_json: row.get(14)?,
        node_id: row
            .get::<_, Option<String>>(15)?
            .map(crate::ids::NodeId::from_db),
        dry_run_json: row.get(16)?,
        dry_run_at: row.get(17)?,
    })
}

/// One proposal.
///
/// # Errors
/// [`Error::NotFound`] when it is missing or belongs to another org.
pub fn get_proposal(conn: &Connection, org: &OrgId, id: &ProposalId) -> Result<ProposalRow> {
    conn.query_row(
        &format!("SELECT {PROPOSAL_COLUMNS} FROM proposals WHERE org_id = ?1 AND id = ?2"),
        params![org.as_str(), id.as_str()],
        map_proposal,
    )
    .optional()?
    .ok_or(Error::NotFound)
}

/// Proposals of the org, newest first, optionally of one status.
///
/// # Errors
/// Database failure.
pub fn list_proposals(
    conn: &Connection,
    org: &OrgId,
    status: Option<ProposalStatus>,
    limit: u32,
) -> Result<Vec<ProposalRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {PROPOSAL_COLUMNS} FROM proposals WHERE org_id = ?1 AND (?2 IS NULL OR status = ?2) \
         ORDER BY created_at DESC, id LIMIT ?3"
    ))?;
    let rows = stmt.query_map(
        params![
            org.as_str(),
            status.map(ProposalStatus::as_str),
            i64::from(limit)
        ],
        map_proposal,
    )?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// How many proposals of this conversation are still pending.
///
/// # Errors
/// Database failure.
pub fn count_pending_proposals(conn: &Connection, org: &OrgId, id: &ConversationId) -> Result<u32> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM proposals WHERE org_id = ?1 AND conversation_id = ?2 AND status = 'pending'",
        params![org.as_str(), id.as_str()],
        |row| row.get(0),
    )?;
    Ok(u32::try_from(n).unwrap_or(u32::MAX))
}

/// What a person decides about a pending proposal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProposalDecision {
    /// Approve: records the approval only. Nothing is executed (milestone M4 executes later).
    Approve,
    /// Reject.
    Reject,
}

/// Result of [`decide_proposal`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecisionOutcome {
    /// The proposal moved from `pending` to the new status.
    Decided(Box<ProposalRow>),
    /// It was no longer pending (someone decided first, or it ran): nothing changed.
    AlreadyDecided(ProposalStatus),
    /// It had expired: it is now marked `expired` and nothing else changed.
    Expired,
}

/// A stored approval/rejection decision for a proposal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRow {
    /// Id.
    pub id: ApprovalId,
    /// Proposal it belongs to.
    pub proposal_id: ProposalId,
    /// User who decided.
    pub user_id: UserId,
    /// Decision (`Approve` or `Reject`).
    pub decision: ProposalDecision,
    /// Whether tier-4 typed confirmation matched the target label.
    pub typed_confirmation: bool,
    /// Optional comment.
    pub comment: Option<String>,
    /// Creation time.
    pub created_at: String,
}

/// Returns whether `org` requires two distinct approvers for tier-4 proposals.
///
/// # Errors
/// [`Error::NotFound`] when `org` does not exist; database failure.
pub fn org_requires_two_approvers(conn: &Connection, org: &OrgId) -> Result<bool> {
    let raw: i64 = conn
        .query_row(
            "SELECT require_two_approvers FROM organizations WHERE id = ?1",
            params![org.as_str()],
            |row| row.get(0),
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    Ok(raw != 0)
}

/// Sets whether `org` requires two distinct approvers for tier-4 proposals.
///
/// # Errors
/// [`Error::NotFound`] when `org` does not exist; database failure.
pub fn set_org_require_two_approvers(conn: &Connection, org: &OrgId, required: bool) -> Result<()> {
    let changed = conn.execute(
        "UPDATE organizations SET require_two_approvers = ?2 WHERE id = ?1",
        params![org.as_str(), i64::from(required)],
    )?;
    if changed == 0 {
        return Err(Error::NotFound);
    }
    Ok(())
}

/// Counts `'approved'` rows in `approvals` for `(org, proposal)` and reports whether `user` has
/// already approved.
///
/// # Errors
/// Database failure.
pub fn count_approvals_for_proposal(
    conn: &Connection,
    org: &OrgId,
    proposal: &ProposalId,
    user: &UserId,
) -> Result<(u32, bool)> {
    let (count_i, user_approved_i): (i64, i64) = conn.query_row(
        "SELECT \
            COUNT(*), \
            COALESCE(MAX(CASE WHEN user_id = ?3 THEN 1 ELSE 0 END), 0) \
         FROM approvals \
         WHERE org_id = ?1 AND proposal_id = ?2 AND decision = 'approved'",
        params![org.as_str(), proposal.as_str(), user.as_str()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let count = u32::try_from(count_i).map_err(|_| Error::Config("approval count overflow"))?;
    Ok((count, user_approved_i != 0))
}

/// Returns the approval record for a proposal, if any.
///
/// # Errors
/// Database failure.
pub fn get_approval_for_proposal(
    conn: &Connection,
    org: &OrgId,
    proposal: &ProposalId,
) -> Result<Option<ApprovalRow>> {
    Ok(conn
        .query_row(
            "SELECT id, proposal_id, user_id, decision, typed_confirmation, comment, created_at \
             FROM approvals WHERE org_id = ?1 AND proposal_id = ?2 \
             ORDER BY created_at DESC LIMIT 1",
            params![org.as_str(), proposal.as_str()],
            |row| {
                let dec: String = row.get(3)?;
                Ok(ApprovalRow {
                    id: ApprovalId::from_db(row.get(0)?),
                    proposal_id: ProposalId::from_db(row.get(1)?),
                    user_id: UserId::from_db(row.get(2)?),
                    decision: if dec == "approved" {
                        ProposalDecision::Approve
                    } else {
                        ProposalDecision::Reject
                    },
                    typed_confirmation: row.get::<_, i64>(4)? != 0,
                    comment: row.get(5)?,
                    created_at: row.get(6)?,
                })
            },
        )
        .optional()?)
}

/// Stores the redacted dry-run JSON and timestamp on a proposal without touching its immutable
/// action fields.
///
/// # Errors
/// [`Error::NotFound`] when missing or in another org; database failure (including `> 8000` bytes).
pub fn set_proposal_dry_run(
    conn: &Connection,
    org: &OrgId,
    id: &ProposalId,
    dry_run_json: &str,
) -> Result<String> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let now = now_utc(&tx)?;
    let changed = tx.execute(
        "UPDATE proposals SET dry_run_json = ?3, dry_run_at = ?4 WHERE org_id = ?1 AND id = ?2",
        params![org.as_str(), id.as_str(), dry_run_json, now],
    )?;
    if changed == 0 {
        return Err(Error::NotFound);
    }
    tx.commit()?;
    Ok(now)
}

/// Records a person's decision on a proposal and appends an audit entry, in one transaction.
/// Only a `pending`, unexpired proposal can be decided; the status change is one conditional
/// UPDATE, so two decisions cannot both win.
///
/// # Errors
/// [`Error::NotFound`] when it is missing or belongs to another org; database failure.
pub fn decide_proposal(
    conn: &Connection,
    org: &OrgId,
    id: &ProposalId,
    decision: ProposalDecision,
    user: &UserId,
) -> Result<DecisionOutcome> {
    decide_proposal_with_details(conn, org, id, decision, user, false, None)
}

/// Records a person's decision on a proposal, stores a row in `approvals`, and appends an audit
/// entry, in one transaction.
///
/// # Errors
/// [`Error::NotFound`] when it is missing or belongs to another org; database failure.
pub fn decide_proposal_with_details(
    conn: &Connection,
    org: &OrgId,
    id: &ProposalId,
    decision: ProposalDecision,
    user: &UserId,
    typed_confirmation: bool,
    comment: Option<&str>,
) -> Result<DecisionOutcome> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let current = get_proposal(&tx, org, id)?;
    if current.status != ProposalStatus::Pending {
        return Ok(DecisionOutcome::AlreadyDecided(current.status));
    }
    let expired: bool = tx.query_row(
        "SELECT expires_at <= strftime('%Y-%m-%dT%H:%M:%fZ', 'now') FROM proposals \
         WHERE org_id = ?1 AND id = ?2",
        params![org.as_str(), id.as_str()],
        |row| row.get(0),
    )?;
    if expired {
        tx.execute(
            "UPDATE proposals SET status = 'expired' WHERE org_id = ?1 AND id = ?2 AND status = 'pending'",
            params![org.as_str(), id.as_str()],
        )?;
        tx.commit()?;
        return Ok(DecisionOutcome::Expired);
    }
    let (status, action) = match decision {
        ProposalDecision::Approve => (ProposalStatus::Approved, "proposal.approve"),
        ProposalDecision::Reject => (ProposalStatus::Rejected, "proposal.reject"),
    };
    let already_by_user: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM approvals WHERE org_id = ?1 AND proposal_id = ?2 AND user_id = ?3)",
        params![org.as_str(), id.as_str(), user.as_str()],
        |row| row.get(0),
    )?;
    if already_by_user {
        return Ok(DecisionOutcome::AlreadyDecided(current.status));
    }
    let two_approvers = org_requires_two_approvers(&tx, org)?;
    let requires_two = two_approvers && current.tier == 4 && decision == ProposalDecision::Approve;
    let approval_id = new_uuid(&tx)?;
    tx.execute(
        "INSERT INTO approvals (id, org_id, proposal_id, user_id, decision, typed_confirmation, comment) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            approval_id,
            org.as_str(),
            id.as_str(),
            user.as_str(),
            status.as_str(),
            i64::from(typed_confirmation),
            comment,
        ],
    )?;
    let should_transition = if requires_two {
        let approved_count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM approvals WHERE org_id = ?1 AND proposal_id = ?2 AND decision = 'approved'",
            params![org.as_str(), id.as_str()],
            |row| row.get(0),
        )?;
        approved_count >= 2
    } else {
        true
    };
    if should_transition {
        tx.execute(
            "UPDATE proposals SET status = ?3 WHERE org_id = ?1 AND id = ?2 AND status = 'pending'",
            params![org.as_str(), id.as_str(), status.as_str()],
        )?;
    }
    // Only an id, a number, a boolean and a hex digest: nothing that needs JSON escaping.
    let subject = format!(
        r#"{{"proposal":"{}","tier":{},"args_hash":"{}","typed_confirmation":{}}}"#,
        id.as_str(),
        current.tier,
        current.args_hash,
        typed_confirmation
    );
    crate::audit::append_audit_in_tx(
        &tx,
        org,
        &crate::audit::NewAuditEntry {
            actor_kind: crate::audit::ActorKind::User,
            actor_id: Some(user.as_str()),
            action,
            node_id: None,
            subject_json: &subject,
            result: crate::audit::AuditOutcome::Success,
        },
    )?;
    let row = get_proposal(&tx, org, id)?;
    tx.commit()?;
    Ok(DecisionOutcome::Decided(Box::new(row)))
}
