//! Listing organizations, for single-user tools (the command line) that must find "the" org of a
//! local database. `organizations` is a global table: these are the only reads without an
//! [`OrgId`], and they return the org rows only (no tenant data).

use crate::error::Result;
use crate::identity::Org;
use crate::ids::OrgId;
use rusqlite::{Connection, params};

fn map(row: &rusqlite::Row<'_>) -> rusqlite::Result<Org> {
    Ok(Org {
        id: OrgId::from_db(row.get(0)?),
        name: row.get(1)?,
        slug: row.get(2)?,
        plan: row.get(3)?,
        created_at: row.get(4)?,
    })
}

/// Every organization of the database, oldest first, at most `limit`.
///
/// # Errors
/// Database failure.
pub fn list_orgs(conn: &Connection, limit: u32) -> Result<Vec<Org>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, slug, plan, created_at FROM organizations ORDER BY created_at, id LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit], map)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}
