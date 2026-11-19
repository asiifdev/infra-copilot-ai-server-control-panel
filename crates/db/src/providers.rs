//! Org-level sealed secrets and AI provider settings. The API key of a provider lives in
//! `secrets`, sealed by the `vault` crate; this module never sees plaintext.

use crate::error::{Error, Result};
use crate::ids::{AiProviderId, OrgId, SecretId};
use crate::new_uuid;
use crate::secret::SealedSecret;
use rusqlite::{Connection, OptionalExtension, params};

/// Stores a sealed secret for the org.
///
/// # Errors
/// Constraint violation for an empty value, bad purpose length or unknown org.
pub fn put_secret(
    conn: &Connection,
    org: &OrgId,
    purpose: &str,
    sealed: &SealedSecret,
) -> Result<SecretId> {
    let id = new_uuid(conn)?;
    conn.execute(
        "INSERT INTO secrets (id, org_id, purpose, ciphertext, key_id) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, org.as_str(), purpose, sealed.ciphertext, sealed.key_id],
    )?;
    Ok(SecretId::from_db(id))
}

/// Reads a sealed secret and its purpose.
///
/// # Errors
/// [`Error::NotFound`] if it is missing or belongs to another org.
pub fn get_secret(conn: &Connection, org: &OrgId, id: &SecretId) -> Result<(String, SealedSecret)> {
    conn.query_row(
        "SELECT purpose, ciphertext, key_id FROM secrets WHERE org_id = ?1 AND id = ?2",
        params![org.as_str(), id.as_str()],
        |row| {
            Ok((
                row.get(0)?,
                SealedSecret {
                    ciphertext: row.get(1)?,
                    key_id: row.get(2)?,
                },
            ))
        },
    )
    .optional()?
    .ok_or(Error::NotFound)
}

/// Replaces the sealed value of a secret (key rotation). The purpose stays.
///
/// # Errors
/// [`Error::NotFound`] if it is not in this org.
pub fn replace_secret(
    conn: &Connection,
    org: &OrgId,
    id: &SecretId,
    sealed: &SealedSecret,
) -> Result<()> {
    let n = conn.execute(
        "UPDATE secrets SET ciphertext = ?3, key_id = ?4 WHERE org_id = ?1 AND id = ?2",
        params![org.as_str(), id.as_str(), sealed.ciphertext, sealed.key_id],
    )?;
    if n == 0 { Err(Error::NotFound) } else { Ok(()) }
}

/// Deletes a secret that nothing references.
///
/// # Errors
/// [`Error::NotFound`] if it is not in this org; a constraint violation while a provider still
/// uses it.
pub fn delete_secret(conn: &Connection, org: &OrgId, id: &SecretId) -> Result<()> {
    let n = conn.execute(
        "DELETE FROM secrets WHERE org_id = ?1 AND id = ?2",
        params![org.as_str(), id.as_str()],
    )?;
    if n == 0 { Err(Error::NotFound) } else { Ok(()) }
}

/// An AI provider to store.
#[derive(Debug)]
pub struct NewAiProvider<'a> {
    /// Unique name inside the org.
    pub name: &'a str,
    /// Base URL of the OpenAI-compatible endpoint.
    pub base_url: &'a str,
    /// Model used when a request names none.
    pub default_model: &'a str,
    /// Sealed API key, or `None` for a keyless local model.
    pub secret: Option<&'a SecretId>,
    /// JSON object text of provider settings (capabilities, limits).
    pub params_json: &'a str,
    /// Whether this is the org's default provider (at most one).
    pub is_default: bool,
}

/// A stored AI provider. The key is referenced, never included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiProvider {
    /// Id.
    pub id: AiProviderId,
    /// Name.
    pub name: String,
    /// Base URL.
    pub base_url: String,
    /// Default model.
    pub default_model: String,
    /// The sealed key, if any.
    pub secret_id: Option<SecretId>,
    /// JSON object text.
    pub params_json: String,
    /// Whether it is the default.
    pub is_default: bool,
}

/// Stores a provider.
///
/// # Errors
/// Constraint violation for a duplicate name, a second default, bad JSON, or a secret that is
/// not in this org (the composite foreign key refuses it).
pub fn create_ai_provider(
    conn: &Connection,
    org: &OrgId,
    new: &NewAiProvider<'_>,
) -> Result<AiProviderId> {
    let id = new_uuid(conn)?;
    conn.execute(
        "INSERT INTO ai_providers (id, org_id, name, base_url, default_model, secret_id, params, is_default) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            id,
            org.as_str(),
            new.name,
            new.base_url,
            new.default_model,
            new.secret.map(SecretId::as_str),
            new.params_json,
            i64::from(new.is_default)
        ],
    )?;
    Ok(AiProviderId::from_db(id))
}

const COLUMNS: &str = "id, name, base_url, default_model, secret_id, params, is_default";

type Raw = (String, String, String, String, Option<String>, String, i64);

fn map(row: &rusqlite::Row<'_>) -> rusqlite::Result<Raw> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
    ))
}

fn build(t: Raw) -> AiProvider {
    AiProvider {
        id: AiProviderId::from_db(t.0),
        name: t.1,
        base_url: t.2,
        default_model: t.3,
        secret_id: t.4.map(SecretId::from_db),
        params_json: t.5,
        is_default: t.6 == 1,
    }
}

/// One provider by id.
///
/// # Errors
/// [`Error::NotFound`] if it is missing or belongs to another org.
pub fn get_ai_provider(conn: &Connection, org: &OrgId, id: &AiProviderId) -> Result<AiProvider> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM ai_providers WHERE org_id = ?1 AND id = ?2"),
        params![org.as_str(), id.as_str()],
        map,
    )
    .optional()?
    .map(build)
    .ok_or(Error::NotFound)
}

/// The org's default provider.
///
/// # Errors
/// [`Error::NotFound`] if none is marked default.
pub fn default_ai_provider(conn: &Connection, org: &OrgId) -> Result<AiProvider> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM ai_providers WHERE org_id = ?1 AND is_default = 1"),
        params![org.as_str()],
        map,
    )
    .optional()?
    .map(build)
    .ok_or(Error::NotFound)
}

/// The org's providers by name.
///
/// # Errors
/// Database failure.
pub fn list_ai_providers(conn: &Connection, org: &OrgId) -> Result<Vec<AiProvider>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM ai_providers WHERE org_id = ?1 ORDER BY name, id"
    ))?;
    let rows = stmt.query_map(params![org.as_str()], map)?;
    Ok(rows
        .map(|r| r.map(build))
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Deletes a provider (its secret is left; delete it separately).
///
/// # Errors
/// [`Error::NotFound`] if it is not in this org.
pub fn delete_ai_provider(conn: &Connection, org: &OrgId, id: &AiProviderId) -> Result<()> {
    let n = conn.execute(
        "DELETE FROM ai_providers WHERE org_id = ?1 AND id = ?2",
        params![org.as_str(), id.as_str()],
    )?;
    if n == 0 { Err(Error::NotFound) } else { Ok(()) }
}
