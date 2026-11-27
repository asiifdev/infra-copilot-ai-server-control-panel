//! Transport definitions (docs/design.md 3.4). Credentials live in `secrets`; a row only
//! references them. Every query filters on the org.

use crate::error::{Error, Result};
use crate::ids::{OrgId, SecretId, TransportId};
use crate::new_uuid;
use rusqlite::{Connection, OptionalExtension, params};

/// A transport to store.
#[derive(Debug)]
pub struct NewTransport<'a> {
    /// Unique name inside the org.
    pub name: &'a str,
    /// `rclone`, `rsync`, `scp`, `ssh_pipe` or `managed` (the table refuses anything else).
    pub kind: &'a str,
    /// JSON object text of non-secret settings.
    pub config_json: &'a str,
    /// Sealed credentials, if the transport has any.
    pub secret: Option<&'a SecretId>,
}

/// A stored transport. The credentials are referenced, never included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportRow {
    /// Id.
    pub id: TransportId,
    /// Name.
    pub name: String,
    /// Kind text.
    pub kind: String,
    /// JSON object text.
    pub config_json: String,
    /// The sealed credentials, if any.
    pub secret_id: Option<SecretId>,
    /// When the last successful verification finished.
    pub verified_at: Option<String>,
    /// JSON text of the last check result.
    pub last_check_json: Option<String>,
    /// Creation time.
    pub created_at: String,
}

/// Stores a transport.
///
/// # Errors
/// Constraint violation for a duplicate name, an unknown kind, bad JSON, or a secret that is not
/// in this org (the composite foreign key refuses it).
pub fn create_transport(
    conn: &Connection,
    org: &OrgId,
    new: &NewTransport<'_>,
) -> Result<TransportId> {
    let id = new_uuid(conn)?;
    conn.execute(
        "INSERT INTO transports (id, org_id, name, kind, config, secret_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            id,
            org.as_str(),
            new.name,
            new.kind,
            new.config_json,
            new.secret.map(SecretId::as_str)
        ],
    )?;
    Ok(TransportId::from_db(id))
}

const COLUMNS: &str = "id, name, kind, config, secret_id, verified_at, last_check, created_at";

type Raw = (
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
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
    ))
}

fn build(t: Raw) -> TransportRow {
    TransportRow {
        id: TransportId::from_db(t.0),
        name: t.1,
        kind: t.2,
        config_json: t.3,
        secret_id: t.4.map(SecretId::from_db),
        verified_at: t.5,
        last_check_json: t.6,
        created_at: t.7,
    }
}

/// One transport by id.
///
/// # Errors
/// [`Error::NotFound`] if it is missing or belongs to another org.
pub fn get_transport(conn: &Connection, org: &OrgId, id: &TransportId) -> Result<TransportRow> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM transports WHERE org_id = ?1 AND id = ?2"),
        params![org.as_str(), id.as_str()],
        map,
    )
    .optional()?
    .map(build)
    .ok_or(Error::NotFound)
}

/// The org's transports by name.
///
/// # Errors
/// Database failure.
pub fn list_transports(conn: &Connection, org: &OrgId) -> Result<Vec<TransportRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM transports WHERE org_id = ?1 ORDER BY name, id"
    ))?;
    let rows = stmt.query_map(params![org.as_str()], map)?;
    Ok(rows
        .map(|r| r.map(build))
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Records the outcome of a check. `verified` sets `verified_at` to now; otherwise the previous
/// value stays.
///
/// # Errors
/// [`Error::NotFound`] if it is not in this org; a constraint violation for bad JSON.
pub fn record_transport_check(
    conn: &Connection,
    org: &OrgId,
    id: &TransportId,
    last_check_json: &str,
    verified: bool,
) -> Result<()> {
    let n = conn.execute(
        "UPDATE transports SET last_check = ?3, \
         verified_at = CASE WHEN ?4 = 1 THEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now') ELSE verified_at END \
         WHERE org_id = ?1 AND id = ?2",
        params![
            org.as_str(),
            id.as_str(),
            last_check_json,
            i64::from(verified)
        ],
    )?;
    if n == 0 { Err(Error::NotFound) } else { Ok(()) }
}

/// Updates the sealed credential reference of a transport and optionally clears its verification
/// state so it must be re-verified with the new credentials.
///
/// # Errors
/// [`Error::NotFound`] if it is not in this org; constraint violation if the secret belongs to
/// another org.
pub fn update_transport_secret(
    conn: &Connection,
    org: &OrgId,
    id: &TransportId,
    secret: Option<&SecretId>,
    reset_verified: bool,
) -> Result<()> {
    let n = if reset_verified {
        conn.execute(
            "UPDATE transports SET secret_id = ?3, verified_at = NULL, last_check = NULL \
             WHERE org_id = ?1 AND id = ?2",
            params![org.as_str(), id.as_str(), secret.map(SecretId::as_str)],
        )?
    } else {
        conn.execute(
            "UPDATE transports SET secret_id = ?3 WHERE org_id = ?1 AND id = ?2",
            params![org.as_str(), id.as_str(), secret.map(SecretId::as_str)],
        )?
    };
    if n == 0 { Err(Error::NotFound) } else { Ok(()) }
}

/// Deletes a transport (its secret is left; delete it separately).
///
/// # Errors
/// [`Error::NotFound`] if it is not in this org.
pub fn delete_transport(conn: &Connection, org: &OrgId, id: &TransportId) -> Result<()> {
    let n = conn.execute(
        "DELETE FROM transports WHERE org_id = ?1 AND id = ?2",
        params![org.as_str(), id.as_str()],
    )?;
    if n == 0 { Err(Error::NotFound) } else { Ok(()) }
}
