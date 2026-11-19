//! Saving and loading transports. The row holds validated non-secret settings; credentials are
//! sealed by the vault and referenced by id. Every call is scoped to an org.

use crate::plan::Credentials;
use crate::spec::{SpecError, TransportSpec};
use crate::validate::{self, ValidationError};
use crate::wizard::Report;
use db::{Connection, NewTransport, OrgId, SecretId, TransportId, TransportRow};
use std::fmt;
use vault::{Vault, VaultError};

/// The vault purpose credentials are sealed under. A sealed value is bound to its purpose, so
/// it cannot be opened as another kind of secret.
pub const CREDENTIALS_PURPOSE: &str = "transport_credentials";

/// Why a store operation failed. Closed set, no values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreError {
    /// The name or a spec field is invalid.
    Invalid(ValidationError),
    /// Managed storage cannot be saved: it is not implemented.
    UnsupportedManaged,
    /// Credentials were given for a kind that takes none.
    CredentialsNotAccepted,
    /// A stored row no longer passes validation.
    CorruptRow(SpecError),
    /// The stored credentials cannot be read back.
    CorruptCredentials,
    /// The vault refused.
    Vault(VaultError),
    /// The database refused (duplicate name, foreign key, ...).
    Database,
    /// Not in this org.
    NotFound,
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(e) => e.fmt(f),
            Self::UnsupportedManaged => f.write_str("transport_unsupported_managed"),
            Self::CredentialsNotAccepted => f.write_str("transport_credentials_not_accepted"),
            Self::CorruptRow(e) => write!(f, "transport_corrupt_row_{e}"),
            Self::CorruptCredentials => f.write_str("transport_corrupt_credentials"),
            Self::Vault(e) => e.fmt(f),
            Self::Database => f.write_str("transport_database"),
            Self::NotFound => f.write_str("transport_not_found"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<db::Error> for StoreError {
    fn from(e: db::Error) -> Self {
        match e {
            db::Error::NotFound => Self::NotFound,
            _ => Self::Database,
        }
    }
}

impl From<VaultError> for StoreError {
    fn from(e: VaultError) -> Self {
        match e {
            VaultError::NotFound => Self::NotFound,
            other => Self::Vault(other),
        }
    }
}

/// A transport read back from the database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredTransport {
    /// Id.
    pub id: TransportId,
    /// Name.
    pub name: String,
    /// The re-validated spec.
    pub spec: TransportSpec,
    /// The sealed credentials, if any.
    pub secret_id: Option<SecretId>,
    /// Last successful verification.
    pub verified_at: Option<String>,
    /// JSON text of the last check.
    pub last_check_json: Option<String>,
}

fn from_row(row: TransportRow) -> Result<StoredTransport, StoreError> {
    let spec = TransportSpec::from_config_json(&row.kind, &row.config_json)
        .map_err(StoreError::CorruptRow)?;
    Ok(StoredTransport {
        id: row.id,
        name: row.name,
        spec,
        secret_id: row.secret_id,
        verified_at: row.verified_at,
        last_check_json: row.last_check_json,
    })
}

/// Saves a transport: seals the credentials (if any) with the vault, then stores the row. If the
/// row cannot be stored the sealed secret is removed again.
///
/// # Errors
/// [`StoreError`].
pub fn save(
    conn: &Connection,
    vault: &Vault,
    org: &OrgId,
    name: &str,
    spec: &TransportSpec,
    credentials: Option<&Credentials>,
    report: Option<&Report>,
) -> Result<TransportId, StoreError> {
    validate::transport_name(name).map_err(StoreError::Invalid)?;
    if matches!(spec, TransportSpec::Managed) {
        return Err(StoreError::UnsupportedManaged);
    }
    let credentials = credentials.filter(|c| !c.is_empty());
    if credentials.is_some() && !spec.kind().accepts_credentials() {
        return Err(StoreError::CredentialsNotAccepted);
    }
    let secret = match credentials {
        Some(c) => {
            let text = c.to_vault_text();
            Some(vault.store(
                conn,
                org,
                CREDENTIALS_PURPOSE,
                text.expose_secret().as_bytes(),
            )?)
        }
        None => None,
    };
    let config = spec.to_config_json();
    let created = db::create_transport(
        conn,
        org,
        &NewTransport {
            name,
            kind: spec.kind().as_str(),
            config_json: &config,
            secret: secret.as_ref(),
        },
    );
    let id = match created {
        Ok(id) => id,
        Err(e) => {
            if let Some(secret) = &secret {
                // Best effort: do not leave an unreferenced sealed secret behind.
                let _ = db::delete_secret(conn, org, secret);
            }
            return Err(e.into());
        }
    };
    if let Some(report) = report {
        db::record_transport_check(conn, org, &id, &report.to_json(), report.listing_ok)?;
    }
    Ok(id)
}

/// One transport of the org.
///
/// # Errors
/// [`StoreError::NotFound`] when it is not in this org; [`StoreError::CorruptRow`] when the row
/// fails validation.
pub fn load(
    conn: &Connection,
    org: &OrgId,
    id: &TransportId,
) -> Result<StoredTransport, StoreError> {
    from_row(db::get_transport(conn, org, id)?)
}

/// The org's transports.
///
/// # Errors
/// As [`load`], for the first bad row.
pub fn list(conn: &Connection, org: &OrgId) -> Result<Vec<StoredTransport>, StoreError> {
    db::list_transports(conn, org)?
        .into_iter()
        .map(from_row)
        .collect()
}

/// Opens a transport's credentials. `None` when it has none.
///
/// # Errors
/// [`StoreError::Vault`] when the vault cannot open them (wrong key, tampering, another org);
/// [`StoreError::CorruptCredentials`] when the opened text is not valid credentials.
pub fn load_credentials(
    conn: &Connection,
    vault: &Vault,
    org: &OrgId,
    stored: &StoredTransport,
) -> Result<Option<Credentials>, StoreError> {
    let Some(secret_id) = &stored.secret_id else {
        return Ok(None);
    };
    // `Vault::load` trusts the purpose stored on the row. A transport must only ever open a
    // secret sealed for transport credentials, never e.g. an AI provider key it was pointed at.
    let (purpose, _) = db::get_secret(conn, org, secret_id)?;
    if purpose != CREDENTIALS_PURPOSE {
        return Err(StoreError::CorruptCredentials);
    }
    let text = vault.load(conn, org, secret_id)?;
    Credentials::from_vault_text(&text)
        .map(Some)
        .ok_or(StoreError::CorruptCredentials)
}

/// Deletes a transport and its sealed credentials.
///
/// # Errors
/// [`StoreError::NotFound`] when it is not in this org.
pub fn delete(conn: &Connection, org: &OrgId, id: &TransportId) -> Result<(), StoreError> {
    let row = db::get_transport(conn, org, id)?;
    db::delete_transport(conn, org, id)?;
    if let Some(secret) = &row.secret_id {
        db::delete_secret(conn, org, secret)?;
    }
    Ok(())
}
