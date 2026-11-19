//! Turning a stored provider row into a working provider.
//!
//! The loop in [`super::Assistant`] is generic over [`AiProvider`]; this is the seam that builds
//! the real one. The API key is opened by the vault at this moment, handed to the provider as a
//! [`core_domain::secret::Secret`] and never stored in the loader.

use ai::{AiProvider, NetTransport, OpenAiCompatible, ProviderConfig};
use db::{AiProviderId, Connection, Error as DbError, OrgId};
use std::fmt;
use vault::Vault;

/// Why a provider could not be built. Fixed text: no URL, key or database text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadError {
    /// No such provider in this org (or no default is set).
    NotFound,
    /// The stored settings are invalid; carries the `ai` error code.
    Config(&'static str),
    /// The key could not be opened; carries the vault error code.
    Key(&'static str),
    /// The network client could not be built (no usable root certificates).
    Client,
    /// The database failed.
    Store,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => f.write_str("provider_not_found"),
            Self::Config(code) => write!(f, "provider_config: {code}"),
            Self::Key(code) => write!(f, "provider_key: {code}"),
            Self::Client => f.write_str("provider_client"),
            Self::Store => f.write_str("provider_store"),
        }
    }
}

impl std::error::Error for LoadError {}

/// Builds the provider a conversation uses.
pub trait ProviderLoader {
    /// The provider type produced.
    type Provider: AiProvider;

    /// The provider `provider` names, or the org default for `None`.
    ///
    /// # Errors
    /// [`LoadError`].
    fn load(
        &self,
        org: &OrgId,
        provider: Option<&AiProviderId>,
    ) -> Result<Self::Provider, LoadError>;
}

/// [`ProviderLoader`] over the SQLite layer, the vault and the hardened `net` client.
pub struct DbProviderLoader<'a> {
    conn: &'a Connection,
    vault: &'a Vault,
}

impl<'a> DbProviderLoader<'a> {
    /// A loader that reads rows through `conn` and opens keys with `vault`. Each loaded provider
    /// gets its own hardened `net` client (`net::Client` is built per load because a transport
    /// owns its client; nothing about the client is shared with the caller).
    #[must_use]
    pub fn new(conn: &'a Connection, vault: &'a Vault) -> Self {
        Self { conn, vault }
    }
}

impl ProviderLoader for DbProviderLoader<'_> {
    type Provider = OpenAiCompatible<NetTransport>;

    fn load(
        &self,
        org: &OrgId,
        provider: Option<&AiProviderId>,
    ) -> Result<Self::Provider, LoadError> {
        let row = match provider {
            Some(id) => db::get_ai_provider(self.conn, org, id),
            None => db::default_ai_provider(self.conn, org),
        }
        .map_err(|e| match e {
            DbError::NotFound => LoadError::NotFound,
            _ => LoadError::Store,
        })?;
        let config =
            ProviderConfig::from_params(&row.base_url, &row.default_model, &row.params_json)
                .map_err(|e| LoadError::Config(e.code()))?;
        let key = row
            .secret_id
            .as_ref()
            .map(|id| self.vault.load(self.conn, org, id))
            .transpose()
            .map_err(|e| LoadError::Key(e.code()))?;
        let client = net::Client::new().map_err(|_| LoadError::Client)?;
        let transport = NetTransport::new(client, config.net_policy(), config.stream_limits());
        Ok(OpenAiCompatible::new(config, key, transport))
    }
}
