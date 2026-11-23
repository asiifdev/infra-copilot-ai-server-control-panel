//! The shared state of the server and the part of it that is configuration.

use crate::auth::Auth;
use crate::clock::Clock;
use bootstrap::{Environment, Scope, SetupError};
use i18n::Catalog;
use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;
use vault::Vault;

/// How long a provider connection test waits for the provider by default. It must stay clearly
/// below the guard's request limit, or the guard would answer with a generic timeout first and
/// the person would never see the advice that goes with the failure.
pub const DEFAULT_PROVIDER_TEST_TIMEOUT: Duration = Duration::from_secs(20);

const _: () =
    assert!(DEFAULT_PROVIDER_TEST_TIMEOUT.as_secs() + 5 <= crate::guard::REQUEST_TIMEOUT.as_secs());

/// Default port of `infra serve`.
pub const DEFAULT_PORT: u16 = 7878;

/// Everything the server needs to start. Build it with [`Config::resolve`] (the command line) or
/// by hand (tests).
pub struct Config {
    /// The address to listen on. Must be a loopback address.
    pub bind: SocketAddr,
    /// The database file.
    pub db_path: PathBuf,
    /// The organization and person the server acts for.
    pub scope: Scope,
    /// The vault, when `NODIVARA_MASTER_KEY` is set. Without it the server still runs, but
    /// refuses to store provider keys (`409 master_key_missing`).
    pub vault: Option<Arc<Vault>>,
    /// The clock sessions and tokens expire by.
    pub clock: Clock,
    /// The text catalog. The embedded one unless a test supplies its own.
    pub catalog: Catalog,
    /// How long a provider connection test waits ([`DEFAULT_PROVIDER_TEST_TIMEOUT`]). Values too
    /// close to the request limit are lowered so the test always ends first.
    pub provider_test_timeout: Duration,
}

impl Config {
    /// Resolves the database, the organization and the vault the way every other command does
    /// (the `bootstrap` crate), for `127.0.0.1:port`.
    ///
    /// # Errors
    /// [`SetupError`] when the database is missing or unusable, the organization cannot be
    /// chosen, or the master key is set but invalid. A missing master key is not an error.
    pub fn resolve(
        env: &dyn Environment,
        port: u16,
        db: Option<&str>,
        org: Option<&str>,
    ) -> Result<Self, SetupError> {
        let (conn, db_path) = bootstrap::open_database(env, db, false)?;
        let scope = bootstrap::resolve_scope(&conn, org)?;
        let vault = match bootstrap::vault_from_env(env) {
            Ok(vault) => Some(Arc::new(vault)),
            Err(SetupError::MasterKeyMissing) => None,
            Err(other) => return Err(other),
        };
        let catalog = Catalog::builtin().map_err(|_| SetupError::Store {
            reason: "catalog".to_owned(),
        })?;
        Ok(Self {
            bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
            db_path,
            scope,
            vault,
            clock: Clock::system(),
            catalog,
            provider_test_timeout: DEFAULT_PROVIDER_TEST_TIMEOUT,
        })
    }
}

/// The state shared by every request.
pub(crate) struct App {
    /// `127.0.0.1:PORT`: the only `Host` the server answers to.
    pub(crate) authority: String,
    /// `http://127.0.0.1:PORT`: the only `Origin` a changing request may carry.
    pub(crate) origin: String,
    pub(crate) db_path: PathBuf,
    pub(crate) scope: Scope,
    pub(crate) vault: Option<Arc<Vault>>,
    pub(crate) catalog: Catalog,
    pub(crate) clock: Clock,
    pub(crate) provider_test_timeout: Duration,
    pub(crate) auth: Mutex<Auth>,
    /// Conversations with a turn running in this process.
    pub(crate) turns: Mutex<HashSet<String>>,
}

impl App {
    pub(crate) fn new(config: Config, local: SocketAddr) -> Self {
        let authority = local.to_string();
        Self {
            origin: format!("http://{authority}"),
            authority,
            db_path: config.db_path,
            scope: config.scope,
            vault: config.vault,
            catalog: config.catalog,
            clock: config.clock,
            provider_test_timeout: config
                .provider_test_timeout
                .min(crate::guard::REQUEST_TIMEOUT.saturating_sub(Duration::from_secs(5))),
            auth: Mutex::new(Auth::default()),
            turns: Mutex::new(HashSet::new()),
        }
    }

    /// The sign-in state. A poisoned lock (a panic while holding it) is recovered: the state is
    /// plain data and stays consistent between statements.
    pub(crate) fn auth(&self) -> MutexGuard<'_, Auth> {
        self.auth.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn turns(&self) -> MutexGuard<'_, HashSet<String>> {
        self.turns.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
