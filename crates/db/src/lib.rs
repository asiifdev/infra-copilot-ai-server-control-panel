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
mod error;
mod identity;
mod ids;
mod nodes;
mod secret;
mod sha256;

use rusqlite::Connection;
use rusqlite_migration::{M, Migrations};
use std::path::Path;
use std::time::Duration;

pub use audit::{
    ActorKind, AuditEntry, AuditOutcome, ChainBreak, ChainStatus, NewAuditEntry, append_audit,
    list_audit, verify_audit_chain,
};
pub use error::{Error, Result};
pub use identity::{
    ApiToken, Member, NewApiToken, Org, Role, User, add_member, create_api_token, create_org,
    create_user, get_org, get_user_by_email, list_api_tokens, list_members, remove_member,
    revoke_api_token, set_member_role,
};
pub use ids::{AgentId, ApiTokenId, AuditId, MembershipId, NodeId, OrgId, SecretId, UserId};
pub use nodes::{
    Agent, NewAgent, NewNode, Node, NodeAccess, NodeKind, NodeSecretMeta, NodeStatus, create_node,
    delete_node, delete_node_secret, get_node, get_node_secret, list_agents, list_node_secrets,
    list_nodes, put_node_secret, register_agent, revoke_agent, set_node_status, touch_agent,
};
pub use secret::{SealedSecret, SecretText};

/// SQL migrations in order. Never edit an applied one; append a new file instead.
const MIGRATION_SQL: [&str; 3] = [
    include_str!("../migrations/0001_identity.sql"),
    include_str!("../migrations/0002_nodes.sql"),
    include_str!("../migrations/0003_audit_log.sql"),
];

/// How long a writer waits for a lock before failing with `SQLITE_BUSY`.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// The migration set, built from [`MIGRATION_SQL`].
#[must_use]
pub fn migrations() -> Migrations<'static> {
    Migrations::new(MIGRATION_SQL.iter().map(|sql| M::up(sql)).collect())
}

/// Opens (creating if needed) a database file, configures the connection and migrates it.
///
/// # Errors
/// Fails if the file cannot be opened, a pragma is refused or a migration fails.
pub fn open(path: &Path) -> Result<Connection> {
    let mut conn = Connection::open(path)?;
    configure(&conn, true)?;
    migrations().to_latest(&mut conn)?;
    Ok(conn)
}

/// Opens a private in-memory database, configured and migrated (tests, throwaway use).
///
/// # Errors
/// Fails if a pragma is refused or a migration fails.
pub fn open_in_memory() -> Result<Connection> {
    let mut conn = Connection::open_in_memory()?;
    configure(&conn, false)?;
    migrations().to_latest(&mut conn)?;
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
