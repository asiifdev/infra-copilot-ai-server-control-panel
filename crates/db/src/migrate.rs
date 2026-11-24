//! Forward-only schema migrations, tracked in SQLite's `user_version` (no extra table, no
//! dependency). Migration `n` (1-based) is applied in its own immediate transaction together with
//! the version bump, so a failure leaves the database at the previous version.

use crate::error::{Error, Result};
use rusqlite::{Connection, TransactionBehavior};

/// SQL migrations in order. Never edit an applied one; append a new file instead.
const MIGRATION_SQL: [&str; 10] = [
    include_str!("../migrations/0001_identity.sql"),
    include_str!("../migrations/0002_nodes.sql"),
    include_str!("../migrations/0003_audit_log.sql"),
    include_str!("../migrations/0004_inventories.sql"),
    include_str!("../migrations/0005_secrets_ai_providers.sql"),
    include_str!("../migrations/0006_conversations.sql"),
    include_str!("../migrations/0007_transports.sql"),
    include_str!("../migrations/0008_migration_plans.sql"),
    include_str!("../migrations/0009_memories.sql"),
    include_str!("../migrations/0010_executions.sql"),
];

/// The schema version this build produces (the number of migrations).
pub const LATEST_VERSION: usize = MIGRATION_SQL.len();

/// The schema version of an open database.
///
/// # Errors
/// Database failure, or a negative `user_version`.
pub fn schema_version(conn: &Connection) -> Result<usize> {
    let raw: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    usize::try_from(raw).map_err(|_| Error::Config("negative user_version"))
}

/// Applies every pending migration. Safe to call repeatedly.
///
/// # Errors
/// [`Error::DatabaseNewer`] if the database is ahead of this build (never downgrade silently);
/// [`Error::Migration`] if a migration fails.
pub fn migrate(conn: &mut Connection) -> Result<()> {
    let current = schema_version(conn)?;
    if current > LATEST_VERSION {
        return Err(Error::DatabaseNewer {
            found: current,
            supported: LATEST_VERSION,
        });
    }
    for (index, sql) in MIGRATION_SQL.iter().enumerate().skip(current) {
        let version = index + 1;
        let version_value =
            i64::try_from(version).map_err(|_| Error::Config("migration number too large"))?;
        let apply = |conn: &mut Connection| -> rusqlite::Result<()> {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch(sql)?;
            // `user_version` is part of the transaction: it rolls back with a failed migration.
            tx.pragma_update(None, "user_version", version_value)?;
            tx.commit()
        };
        apply(conn).map_err(|source| Error::Migration { version, source })?;
    }
    Ok(())
}
