//! Persistence layer: schema migrations and tenant-scoped queries (SQLite first).
//!
//! Depends on `core-domain` only (see docs/decisions/0007, 0009 and the rust-conventions skill).
//!
//! Tenant isolation rules enforced here:
//! * every function that touches an org-owned table takes an [`OrgId`] and filters on it in SQL;
//! * a row of another org is indistinguishable from a missing row ([`Error::NotFound`]);
//! * child rows reference `(org_id, id)` of their parent, so the schema itself refuses cross-org links.
//!
//! Only globally scoped data (organizations, users) is reachable without an [`OrgId`].

#[macro_use]
mod enums;
mod audit;
mod conversations;
mod error;
mod identity;
mod ids;
mod memories;
mod migrate;
mod nodes;
mod orgs;
mod plan_support;
mod plans;
mod providers;
mod search;
mod secret;
mod sha256;
mod snapshots;
mod transports;

use std::path::Path;
use std::time::Duration;

pub use audit::{
    ActorKind, AuditEntry, AuditOutcome, ChainBreak, ChainStatus, NewAuditEntry, append_audit,
    list_audit, verify_audit_chain,
};
pub use conversations::{
    ClaimOutcome, ConversationRow, MessageRole, MessageRow, NewMessage, NewProposal, NewToolCall,
    ProposalRow, ProposalStatus, ToolCallRow, TurnClaim, append_messages, archive_conversation,
    claim_turn, count_pending_proposals, create_conversation, create_proposal, delete_conversation,
    get_conversation, get_proposal, list_conversations, list_messages, list_proposals,
    list_tool_calls, release_turn, retitle_conversation, touch_turn,
};
pub use error::{Error, Result};
pub use identity::{
    ApiToken, Member, NewApiToken, Org, Role, User, add_member, create_api_token, create_org,
    create_user, get_member_user, get_org, get_user_by_email, is_member, list_api_tokens,
    list_members, remove_member, revoke_api_token, set_member_role,
};
pub use ids::{
    AgentId, AiProviderId, ApiTokenId, AuditId, ComparisonId, ConversationId, InventoryId,
    MembershipId, MemoryId, NodeId, OrgId, PlanId, ProposalId, SecretId, TransportId, UserId,
};
pub use memories::{
    MAX_MEMORIES_PER_ORG, MAX_MEMORY_CHARS, MemoryKind, MemoryRow, MemorySource, MemoryStatus,
    NewMemory, approve_memory, clean_memory_text, count_memories, delete_memory, get_memory,
    list_memories, prompt_memories, save_memory,
};
pub use migrate::{LATEST_VERSION, migrate, schema_version};
pub use nodes::{
    Agent, NewAgent, NewNode, Node, NodeAccess, NodeKind, NodeSecretMeta, NodeStatus, create_node,
    delete_node, delete_node_secret, get_node, get_node_secret, list_agents, list_node_secrets,
    list_nodes, put_node_secret, register_agent, revoke_agent, set_node_status, touch_agent,
};
pub use orgs::list_orgs;
pub use plan_support::{
    DraftError, create_draft_plan, find_plan_by_name, latest_comparison_between,
};
pub use plans::{
    NewPlan, NewPlanStep, PlanRow, PlanStatus, PlanStepKind, PlanStepRow, PlanStepStatus,
    create_plan, get_plan, list_plan_steps, list_plans, replace_plan_content, set_plan_status,
    set_plan_step_status,
};
pub use providers::{
    AiProvider, NewAiProvider, create_ai_provider, default_ai_provider, delete_ai_provider,
    delete_secret, get_ai_provider, get_secret, list_ai_providers, put_secret, replace_secret,
};
pub use search::{
    MAX_HITS, MAX_QUERY_TOKENS, MAX_TOKEN_CHARS, SearchHit, SearchKind, fts_query, search,
};
pub use secret::{SealedSecret, SecretText};
pub use sha256::sha256_hex;
pub use snapshots::{
    ComparisonRow, FindingRow, FindingSeverity, FindingStatus, InventoryRow, InventoryStatus,
    ItemKind, ItemRow, ItemSource, NewFinding, NewInventory, NewInventoryItem,
    count_inventory_items, get_inventory, latest_comparison, latest_inventory, list_findings,
    list_inventory_items, save_comparison, save_inventory,
};
pub use transports::{
    NewTransport, TransportRow, create_transport, delete_transport, get_transport, list_transports,
    record_transport_check,
};

/// The SQLite connection type returned by [`open`], re-exported so dependents need no direct
/// `rusqlite` dependency.
pub use rusqlite::Connection;

/// How long a writer waits for a lock before failing with `SQLITE_BUSY`.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Opens (creating if needed) a database file, configures the connection and migrates it.
///
/// # Errors
/// Fails if the file cannot be opened, a pragma is refused or a migration fails.
pub fn open(path: &Path) -> Result<Connection> {
    let mut conn = Connection::open(path)?;
    configure(&conn, true)?;
    migrate(&mut conn)?;
    Ok(conn)
}

/// Opens a private in-memory database, configured and migrated (tests, throwaway use).
///
/// # Errors
/// Fails if a pragma is refused or a migration fails.
pub fn open_in_memory() -> Result<Connection> {
    let mut conn = Connection::open_in_memory()?;
    configure(&conn, false)?;
    migrate(&mut conn)?;
    Ok(conn)
}

/// Per-connection settings. Foreign keys are off by default in SQLite and must be enabled on
/// every connection, so every connection must come from [`open`] or [`open_in_memory`].
fn configure(conn: &Connection, wal: bool) -> Result<()> {
    conn.pragma_update(None, "foreign_keys", true)?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    if wal {
        // The pragma returns the resulting mode; it must be WAL or we refuse to run.
        let mode: String =
            conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))?;
        if !mode.eq_ignore_ascii_case("wal") {
            return Err(Error::Config("journal_mode WAL was not applied"));
        }
    }
    Ok(())
}

/// Generates a random UUID v4 (lowercase text) using SQLite's CSPRNG, avoiding a new dependency.
pub(crate) fn new_uuid(conn: &Connection) -> Result<String> {
    Ok(conn.query_row(
        "SELECT lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-4' \
         || substr(lower(hex(randomblob(2))), 2) || '-' \
         || substr('89ab', abs(random()) % 4 + 1, 1) || substr(lower(hex(randomblob(2))), 2) \
         || '-' || lower(hex(randomblob(6)))",
        [],
        |row| row.get(0),
    )?)
}

/// Current UTC time as ISO-8601 with milliseconds.
pub(crate) fn now_utc(conn: &Connection) -> Result<String> {
    Ok(
        conn.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |row| {
            row.get(0)
        })?,
    )
}
