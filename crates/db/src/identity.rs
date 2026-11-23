//! Organizations, users, memberships and API tokens (docs/design.md 3.1).
//!
//! `organizations` and `users` are global tables. Everything else takes an [`OrgId`].

use crate::error::{Error, Result};
use crate::ids::{ApiTokenId, MembershipId, OrgId, UserId};
use crate::secret::SecretText;
use crate::{new_uuid, now_utc};
use rusqlite::{Connection, OptionalExtension, params};

text_enum!(
    /// Role of a user inside an org. Checked on the server, never trusted from the UI.
    Role {
        /// Full control of the org.
        Owner => "owner",
        /// Manages members and nodes.
        Admin => "admin",
        /// Runs approved actions.
        Operator => "operator",
        /// Read-only.
        Viewer => "viewer",
    }
);

/// An organization (tenant).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Org {
    /// Tenant id.
    pub id: OrgId,
    /// Display name.
    pub name: String,
    /// URL-safe unique name.
    pub slug: String,
    /// Plan name.
    pub plan: String,
    /// Creation time (ISO-8601 UTC).
    pub created_at: String,
}

/// A user. The password hash is never read back by this crate's queries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    /// User id.
    pub id: UserId,
    /// Lowercase email.
    pub email: String,
    /// Display name.
    pub name: String,
    /// Creation time.
    pub created_at: String,
}

/// A user's membership in one org.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// Membership id.
    pub id: MembershipId,
    /// The member.
    pub user_id: UserId,
    /// Their role.
    pub role: Role,
    /// Creation time.
    pub created_at: String,
}

/// Input for [`create_api_token`]. The caller hashes the token; only the hash is stored.
#[derive(Debug)]
pub struct NewApiToken<'a> {
    /// Owner of the token (must be a user; org membership is checked by the API layer).
    pub user_id: &'a UserId,
    /// Human label.
    pub name: &'a str,
    /// Hash of the token value.
    pub token_hash: &'a SecretText,
    /// JSON array of scopes.
    pub scopes_json: &'a str,
    /// Optional expiry (ISO-8601 UTC).
    pub expires_at: Option<&'a str>,
}

/// API token metadata. The hash is not returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiToken {
    /// Token id.
    pub id: ApiTokenId,
    /// Owner.
    pub user_id: UserId,
    /// Human label.
    pub name: String,
    /// JSON array of scopes.
    pub scopes_json: String,
    /// Expiry, if any.
    pub expires_at: Option<String>,
    /// Revocation time, if revoked.
    pub revoked_at: Option<String>,
}

/// Creates an organization. Global operation (there is no tenant yet).
///
/// # Errors
/// Constraint violation for a duplicate or malformed slug or name.
pub fn create_org(conn: &Connection, name: &str, slug: &str, plan: &str) -> Result<Org> {
    let id = new_uuid(conn)?;
    conn.execute(
        "INSERT INTO organizations (id, name, slug, plan) VALUES (?1, ?2, ?3, ?4)",
        params![id, name, slug, plan],
    )?;
    get_org(conn, &OrgId::from_db(id))
}

/// Reads one organization.
///
/// # Errors
/// [`Error::NotFound`] if it does not exist.
pub fn get_org(conn: &Connection, org: &OrgId) -> Result<Org> {
    conn.query_row(
        "SELECT id, name, slug, plan, created_at FROM organizations WHERE id = ?1",
        params![org.as_str()],
        |row| {
            Ok(Org {
                id: OrgId::from_db(row.get(0)?),
                name: row.get(1)?,
                slug: row.get(2)?,
                plan: row.get(3)?,
                created_at: row.get(4)?,
            })
        },
    )
    .optional()?
    .ok_or(Error::NotFound)
}

/// Creates a user. Global operation. `password_hash` is `None` for SSO users.
///
/// # Errors
/// Constraint violation for a duplicate or malformed email.
pub fn create_user(
    conn: &Connection,
    email: &str,
    name: &str,
    password_hash: Option<&SecretText>,
) -> Result<User> {
    let id = new_uuid(conn)?;
    conn.execute(
        "INSERT INTO users (id, email, password_hash, name) VALUES (?1, ?2, ?3, ?4)",
        params![id, email, password_hash.map(SecretText::expose), name],
    )?;
    user_by_id(conn, &id)
}

fn user_by_id(conn: &Connection, id: &str) -> Result<User> {
    conn.query_row(
        "SELECT id, email, name, created_at FROM users WHERE id = ?1",
        params![id],
        map_user,
    )
    .optional()?
    .ok_or(Error::NotFound)
}

fn map_user(row: &rusqlite::Row<'_>) -> rusqlite::Result<User> {
    Ok(User {
        id: UserId::from_db(row.get(0)?),
        email: row.get(1)?,
        name: row.get(2)?,
        created_at: row.get(3)?,
    })
}

/// Finds a user by email for login. Users are global, so this is the one lookup that spans orgs;
/// callers must not expose its result before authentication succeeds.
///
/// # Errors
/// [`Error::NotFound`] if there is no such user.
pub fn get_user_by_email(conn: &Connection, email: &str) -> Result<User> {
    conn.query_row(
        "SELECT id, email, name, created_at FROM users WHERE email = ?1",
        params![email.to_ascii_lowercase()],
        map_user,
    )
    .optional()?
    .ok_or(Error::NotFound)
}

/// The user `user`, only when they are a member of `org` (a user of another org is the same as a
/// missing one, so a caller never learns who exists elsewhere).
///
/// # Errors
/// [`Error::NotFound`] if the user is missing or not a member of this org.
pub fn get_member_user(conn: &Connection, org: &OrgId, user: &UserId) -> Result<User> {
    conn.query_row(
        "SELECT u.id, u.email, u.name, u.created_at FROM users u \
         JOIN memberships m ON m.user_id = u.id WHERE m.org_id = ?1 AND u.id = ?2",
        params![org.as_str(), user.as_str()],
        map_user,
    )
    .optional()?
    .ok_or(Error::NotFound)
}

/// Adds a user to an org.
///
/// # Errors
/// Constraint violation if the user is unknown or already a member.
pub fn add_member(
    conn: &Connection,
    org: &OrgId,
    user: &UserId,
    role: Role,
) -> Result<MembershipId> {
    let id = new_uuid(conn)?;
    conn.execute(
        "INSERT INTO memberships (id, org_id, user_id, role) VALUES (?1, ?2, ?3, ?4)",
        params![id, org.as_str(), user.as_str(), role.as_str()],
    )?;
    Ok(MembershipId::from_db(id))
}

/// Lists the members of an org.
///
/// # Errors
/// Database failure or an unknown stored role.
pub fn list_members(conn: &Connection, org: &OrgId) -> Result<Vec<Member>> {
    let mut stmt = conn.prepare(
        "SELECT id, user_id, role, created_at FROM memberships WHERE org_id = ?1 ORDER BY created_at, id",
    )?;
    let rows = stmt.query_map(params![org.as_str()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get(3)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, user_id, role, created_at) = row?;
        out.push(Member {
            id: MembershipId::from_db(id),
            user_id: UserId::from_db(user_id),
            role: Role::parse(&role)?,
            created_at,
        });
    }
    Ok(out)
}

/// Changes a member's role.
///
/// # Errors
/// [`Error::NotFound`] if the membership is not in this org.
pub fn set_member_role(
    conn: &Connection,
    org: &OrgId,
    id: &MembershipId,
    role: Role,
) -> Result<()> {
    let n = conn.execute(
        "UPDATE memberships SET role = ?3 WHERE org_id = ?1 AND id = ?2",
        params![org.as_str(), id.as_str(), role.as_str()],
    )?;
    found(n)
}

/// Whether `user` currently holds a membership in `org`. A removed member is not a member, so a
/// caller holding a stale session or id cannot keep acting for the org.
///
/// # Errors
/// Database failure.
pub fn is_member(conn: &Connection, org: &OrgId, user: &UserId) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM memberships WHERE org_id = ?1 AND user_id = ?2)",
        params![org.as_str(), user.as_str()],
        |row| row.get(0),
    )?)
}

/// Removes a member from an org.
///
/// # Errors
/// [`Error::NotFound`] if the membership is not in this org.
pub fn remove_member(conn: &Connection, org: &OrgId, id: &MembershipId) -> Result<()> {
    let n = conn.execute(
        "DELETE FROM memberships WHERE org_id = ?1 AND id = ?2",
        params![org.as_str(), id.as_str()],
    )?;
    found(n)
}

/// Creates an API token record.
///
/// # Errors
/// [`Error::NotFound`] if the user is not a member of this org; constraint violation for bad JSON,
/// bad expiry or duplicate hash.
pub fn create_api_token(
    conn: &Connection,
    org: &OrgId,
    new: &NewApiToken<'_>,
) -> Result<ApiTokenId> {
    let id = new_uuid(conn)?;
    let n = conn.execute(
        "INSERT INTO api_tokens (id, org_id, user_id, name, token_hash, scopes, expires_at) \
         SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7 \
         WHERE EXISTS (SELECT 1 FROM memberships WHERE org_id = ?2 AND user_id = ?3)",
        params![
            id,
            org.as_str(),
            new.user_id.as_str(),
            new.name,
            new.token_hash.expose(),
            new.scopes_json,
            new.expires_at
        ],
    )?;
    // Zero rows: the user is not a member of this org.
    found(n)?;
    Ok(ApiTokenId::from_db(id))
}

/// Lists the API tokens of an org (metadata only).
///
/// # Errors
/// Database failure.
pub fn list_api_tokens(conn: &Connection, org: &OrgId) -> Result<Vec<ApiToken>> {
    let mut stmt = conn.prepare(
        "SELECT id, user_id, name, scopes, expires_at, revoked_at FROM api_tokens \
         WHERE org_id = ?1 ORDER BY created_at, id",
    )?;
    let rows = stmt.query_map(params![org.as_str()], |row| {
        Ok(ApiToken {
            id: ApiTokenId::from_db(row.get(0)?),
            user_id: UserId::from_db(row.get(1)?),
            name: row.get(2)?,
            scopes_json: row.get(3)?,
            expires_at: row.get(4)?,
            revoked_at: row.get(5)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Revokes a token (idempotent on the first revocation time).
///
/// # Errors
/// [`Error::NotFound`] if the token is not in this org.
pub fn revoke_api_token(conn: &Connection, org: &OrgId, id: &ApiTokenId) -> Result<()> {
    let now = now_utc(conn)?;
    let n = conn.execute(
        "UPDATE api_tokens SET revoked_at = coalesce(revoked_at, ?3) WHERE org_id = ?1 AND id = ?2",
        params![org.as_str(), id.as_str(), now],
    )?;
    found(n)
}

pub(crate) fn found(changed: usize) -> Result<()> {
    if changed == 0 {
        Err(Error::NotFound)
    } else {
        Ok(())
    }
}
