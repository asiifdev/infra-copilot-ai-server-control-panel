//! Actions: the closed list of things an approved proposal may do (ADR 0037).
//!
//! An [`Action`] is data, never a command line. It is parsed from a strict JSON form, every field
//! is validated, its tier is fixed by this code (neither a model nor a person can lower it), and
//! it has one canonical form whose SHA-256 binds an approval to exactly this action. The
//! [`Executor`] turns an action into argv-only commands or validated local file operations and runs
//! commands through a `transport::Runner`, so tests never touch a real service.

mod exec;
mod logs;
mod sites;
mod ssh;

pub use exec::{
    BackupSnapshot, DryRun, Executor, LocalExecutor, Outcome, OutcomeStatus, dry_run_readonly,
};
pub use logs::{
    LogExcerpt, MAX_LOG_BYTES, MAX_LOG_LINES, format_log_excerpt, read_managed_log,
    read_service_logs_via_runner,
};
pub use sites::{
    MAX_SITE_DOMAIN_LEN, MAX_TEST_DOMAIN_LEN, PhpVersion, SiteDomain, render_site_vhost_config,
    render_test_vhost_config, validate_test_domain, write_managed_test_site,
};
pub use ssh::{SshExecutor, SshTarget};

#[cfg(test)]
pub(crate) mod test_support {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    pub(crate) struct TestDir(PathBuf);

    impl TestDir {
        pub(crate) fn new(prefix: &str) -> Result<Self, std::io::Error> {
            let seq = SEQ.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "nodivara-actions-{prefix}-{}-{seq}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir)?;
            Ok(Self(dir))
        }

        pub(crate) fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

use core_domain::tier::Tier;
use std::fmt;
use std::path::{Component, Path};

/// Longest service name accepted.
pub const MAX_SERVICE_NAME: usize = 100;
/// Longest managed file path accepted.
pub const MAX_PATH_LEN: usize = 300;
/// Longest environment variable key accepted.
pub const MAX_ENV_KEY: usize = 128;
/// Longest environment variable value accepted.
pub const MAX_ENV_VALUE: usize = 1000;

/// Why an action could not be built or read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionError {
    /// The text is not the strict JSON form (not an object, unknown or missing key, wrong type).
    BadShape,
    /// The action name is not on the list.
    UnknownAction,
    /// The service manager is not on the list.
    UnknownManager,
    /// The service name is empty, too long, starts with `-` or `.`, has a character outside
    /// `A-Z a-z 0-9 _ . @ -`, or names a unit that is not a service (`.target`, `.socket`,
    /// `.mount`, ...): restarting `poweroff.target` would power the machine off.
    BadServiceName,
    /// The path is empty, too long, relative, contains `.` or `..`, control characters, or does
    /// not match the required file naming (e.g. `.env` for env updates).
    BadPath,
    /// The environment variable key is empty, too long, or not `[A-Za-z_][A-Za-z0-9_]*`.
    BadEnvKey,
    /// The environment variable value is too long or contains NUL / newline characters.
    BadEnvValue,
    /// The site or certificate domain is empty, too long, malformed, or reserved.
    BadDomain,
    /// The upstream port is `0`.
    BadPort,
    /// The PHP version is not on the strict allowlist (`8.1`, `8.2`, `8.3`, `8.4`).
    UnknownPhpVersion,
    /// The ACME provider is not on the strict allowlist (`letsencrypt`, `zerossl`).
    UnknownAcmeProvider,
    /// The database engine is not on the strict allowlist (`mysql`, `postgres`).
    UnknownDbEngine,
    /// The database name is empty, too long, or outside `[a-zA-Z0-9_]+`.
    BadDbName,
}

impl fmt::Display for ActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::BadShape => "action_bad_shape",
            Self::UnknownAction => "action_unknown",
            Self::UnknownManager => "action_unknown_manager",
            Self::BadServiceName => "action_bad_service_name",
            Self::BadPath => "action_bad_path",
            Self::BadEnvKey => "action_bad_env_key",
            Self::BadEnvValue => "action_bad_env_value",
            Self::BadDomain => "action_bad_domain",
            Self::BadPort => "action_bad_port",
            Self::UnknownPhpVersion => "action_unknown_php_version",
            Self::UnknownAcmeProvider => "action_unknown_acme_provider",
            Self::UnknownDbEngine => "action_unknown_db_engine",
            Self::BadDbName => "action_bad_db_name",
        })
    }
}

impl std::error::Error for ActionError {}

/// How services are managed on the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceManagerKind {
    /// systemd (`systemctl`).
    Systemd,
}

impl ServiceManagerKind {
    /// Stored text form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Systemd => "systemd",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "systemd" => Some(Self::Systemd),
            _ => None,
        }
    }
}

/// Unit types systemd knows besides `service`. A name ending in one of them is refused:
/// `systemctl restart reboot.target` is a reboot, not a service restart.
const OTHER_UNIT_TYPES: [&str; 10] = [
    "socket",
    "target",
    "mount",
    "automount",
    "swap",
    "timer",
    "path",
    "slice",
    "scope",
    "device",
];

/// Units whose restart can cut the machine off, end every session or stop it: a restart of one of
/// these is a risky change (tier 3), never a light one. Matched on the lowercase name without the
/// `.service` suffix, as a prefix (`systemd-` covers logind, journald, udevd, networkd, ...).
const CRITICAL_PREFIXES: [&str; 17] = [
    "systemd-",
    "dbus",
    "getty",
    "console-getty",
    "serial-getty",
    "user@",
    "user-runtime-dir@",
    "sshd",
    "ssh",
    "networkmanager",
    "networking",
    "network",
    "polkit",
    "rc-local",
    "emergency",
    "rescue",
    "init",
];

/// Units that stop or reboot the machine, matched exactly.
const POWER_UNITS: [&str; 7] = [
    "poweroff",
    "reboot",
    "halt",
    "kexec",
    "shutdown",
    "suspend",
    "hibernate",
];

/// A validated service name, always the full unit name ending in `.service`. There is no way to
/// build one that fails validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceName(String);

impl ServiceName {
    /// Validates a service name. A name without a `.service` suffix gets it (`nginx` becomes
    /// `nginx.service`), so the command can only ever address a service unit.
    ///
    /// # Errors
    /// [`ActionError::BadServiceName`].
    pub fn new(text: &str) -> Result<Self, ActionError> {
        let charset_ok = !text.is_empty()
            && text.len() <= MAX_SERVICE_NAME
            && !text.starts_with(['-', '.'])
            && text
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '@' | '-'));
        if !charset_ok {
            return Err(ActionError::BadServiceName);
        }
        let other_unit_type = text.rsplit_once('.').is_some_and(|(_, suffix)| {
            OTHER_UNIT_TYPES.contains(&suffix.to_ascii_lowercase().as_str())
        });
        if other_unit_type {
            return Err(ActionError::BadServiceName);
        }
        let full = if text.ends_with(".service") {
            text.to_owned()
        } else {
            format!("{text}.service")
        };
        if full.len() > MAX_SERVICE_NAME || full == ".service" {
            return Err(ActionError::BadServiceName);
        }
        Ok(Self(full))
    }

    /// The full unit name, ending in `.service`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether restarting this unit can cut the machine off, end every session or stop it.
    #[must_use]
    pub fn is_critical(&self) -> bool {
        let lower = self.0.to_ascii_lowercase();
        let base = lower.strip_suffix(".service").unwrap_or(&lower);
        POWER_UNITS.contains(&base) || CRITICAL_PREFIXES.iter().any(|p| base.starts_with(p))
    }
}

/// A validated absolute lexical path without `.` or `..` components or control characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedPath(String);

impl ManagedPath {
    /// Validates a managed file path.
    ///
    /// # Errors
    /// [`ActionError::BadPath`].
    pub fn new(text: &str) -> Result<Self, ActionError> {
        if text.is_empty()
            || text.len() > MAX_PATH_LEN
            || !text.starts_with('/')
            || text.ends_with('/')
            || text.chars().any(char::is_control)
        {
            return Err(ActionError::BadPath);
        }
        if text[1..]
            .split('/')
            .any(|seg| seg.is_empty() || seg == "." || seg == "..")
        {
            return Err(ActionError::BadPath);
        }
        let p = Path::new(text);
        let mut normal_count = 0usize;
        for comp in p.components() {
            match comp {
                Component::RootDir => {}
                Component::Normal(seg) => {
                    if seg.is_empty() {
                        return Err(ActionError::BadPath);
                    }
                    normal_count += 1;
                }
                Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                    return Err(ActionError::BadPath);
                }
            }
        }
        if normal_count == 0 || text.ends_with('/') {
            return Err(ActionError::BadPath);
        }
        Ok(Self(text.to_owned()))
    }

    /// Validates a path to a `.env` file (filename must be `.env` or `.env.<suffix>`).
    ///
    /// # Errors
    /// [`ActionError::BadPath`].
    pub fn new_env(text: &str) -> Result<Self, ActionError> {
        let path = Self::new(text)?;
        let name = Path::new(&path.0)
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or(ActionError::BadPath)?;
        if name != ".env" && !name.starts_with(".env.") {
            return Err(ActionError::BadPath);
        }
        Ok(path)
    }

    /// The path string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A validated POSIX environment variable key (`[A-Za-z_][A-Za-z0-9_]*`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvKey(String);

impl EnvKey {
    /// Validates an environment variable key.
    ///
    /// # Errors
    /// [`ActionError::BadEnvKey`].
    pub fn new(text: &str) -> Result<Self, ActionError> {
        let mut chars = text.chars();
        let Some(first) = chars.next() else {
            return Err(ActionError::BadEnvKey);
        };
        if text.len() > MAX_ENV_KEY
            || !(first.is_ascii_alphabetic() || first == '_')
            || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(ActionError::BadEnvKey);
        }
        Ok(Self(text.to_owned()))
    }

    /// The key string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A validated single-line environment variable value. `Debug` and `Display` are redacted so the
/// value never leaks into logs.
#[derive(Clone, PartialEq, Eq)]
pub struct EnvValue(String);

impl EnvValue {
    /// Validates an environment variable value (no NUL or newlines, bounded length).
    ///
    /// # Errors
    /// [`ActionError::BadEnvValue`].
    pub fn new(text: &str) -> Result<Self, ActionError> {
        if text.len() > MAX_ENV_VALUE || text.contains(['\0', '\r', '\n']) {
            return Err(ActionError::BadEnvValue);
        }
        Ok(Self(text.to_owned()))
    }

    /// The underlying value (for executor application only; never log or display).
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for EnvValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("EnvValue([REDACTED])")
    }
}

impl fmt::Display for EnvValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

/// Longest database name accepted.
pub const MAX_DB_NAME: usize = 64;

/// Supported ACME certificate authorities for [`Action::IssueTlsCert`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcmeProvider {
    /// Let's Encrypt via `certbot`.
    LetsEncrypt,
    /// ZeroSSL via `acme.sh`.
    ZeroSsl,
}

impl AcmeProvider {
    /// Stored text form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LetsEncrypt => "letsencrypt",
            Self::ZeroSsl => "zerossl",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "letsencrypt" => Some(Self::LetsEncrypt),
            "zerossl" => Some(Self::ZeroSsl),
            _ => None,
        }
    }
}

/// Supported database engines for [`Action::BackupDatabase`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbEngine {
    /// MySQL / MariaDB (`mysqldump`).
    Mysql,
    /// PostgreSQL (`pg_dump`).
    Postgres,
}

impl DbEngine {
    /// Stored text form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mysql => "mysql",
            Self::Postgres => "postgres",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "mysql" => Some(Self::Mysql),
            "postgres" => Some(Self::Postgres),
            _ => None,
        }
    }
}

/// A validated database name matching `^[a-zA-Z0-9_]+$`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbName(String);

impl DbName {
    /// Validates a database name (`1..=64` ASCII alphanumeric or `_` characters).
    ///
    /// # Errors
    /// [`ActionError::BadDbName`].
    pub fn new(text: &str) -> Result<Self, ActionError> {
        if text.is_empty()
            || text.len() > MAX_DB_NAME
            || !text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(ActionError::BadDbName);
        }
        Ok(Self(text.to_owned()))
    }

    /// The database name string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One thing an approved proposal may do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Restart a service. Tier 2 for an ordinary service, tier 3 for a critical unit (see
    /// [`ServiceName::is_critical`]).
    RestartService {
        /// How the service is managed.
        manager: ServiceManagerKind,
        /// Its name.
        name: ServiceName,
    },
    /// Set or update an environment variable in an allowlisted `.env` file (tier 3).
    UpdateEnvVar {
        /// Path to the `.env` file.
        path: ManagedPath,
        /// Variable key.
        key: EnvKey,
        /// Variable value (redacted in logs, dry-run diff, and UI).
        value: EnvValue,
    },
    /// Delete a managed file inside an allowlisted directory (tier 4, requires typed confirmation).
    DeleteManagedFile {
        /// Path to the managed file.
        path: ManagedPath,
    },
    /// Configure an nginx site vhost (tier 2 for `.test` without TLS; tier 3 for production or TLS).
    ConfigureSite {
        /// Validated `.test` or production domain.
        domain: SiteDomain,
        /// Local upstream port (`1..=65535`).
        upstream_port: u16,
        /// Optional PHP-FPM version from the strict allowlist.
        php_version: Option<PhpVersion>,
        /// Whether TLS is enabled on the vhost.
        tls_enabled: bool,
        /// Site document root under an allowlisted managed directory.
        root_dir: ManagedPath,
    },
    /// Issue or renew a production TLS certificate via ACME (tier 3).
    IssueTlsCert {
        /// Validated production domain (`.test` / local TLDs refused).
        domain: SiteDomain,
        /// ACME provider (`letsencrypt` or `zerossl`).
        acme_provider: AcmeProvider,
    },
    /// Dump a MySQL or PostgreSQL database to a managed backup path without password in argv (tier 2).
    BackupDatabase {
        /// Database engine (`mysql` or `postgres`).
        engine: DbEngine,
        /// Strict `[a-zA-Z0-9_]+` database name.
        db_name: DbName,
        /// Destination dump file path under an allowlisted managed directory.
        output_path: ManagedPath,
    },
}

impl Action {
    /// A restart of `name` under `manager`.
    ///
    /// # Errors
    /// [`ActionError::BadServiceName`].
    pub fn restart_service(manager: ServiceManagerKind, name: &str) -> Result<Self, ActionError> {
        Ok(Self::RestartService {
            manager,
            name: ServiceName::new(name)?,
        })
    }

    /// An update of `key` in `.env` at `path`.
    ///
    /// # Errors
    /// [`ActionError::BadPath`], [`ActionError::BadEnvKey`], or [`ActionError::BadEnvValue`].
    pub fn update_env_var(path: &str, key: &str, value: &str) -> Result<Self, ActionError> {
        Ok(Self::UpdateEnvVar {
            path: ManagedPath::new_env(path)?,
            key: EnvKey::new(key)?,
            value: EnvValue::new(value)?,
        })
    }

    /// A deletion of a managed file at `path`.
    ///
    /// # Errors
    /// [`ActionError::BadPath`].
    pub fn delete_managed_file(path: &str) -> Result<Self, ActionError> {
        Ok(Self::DeleteManagedFile {
            path: ManagedPath::new(path)?,
        })
    }

    /// Configures a `.test` or production site vhost.
    ///
    /// # Errors
    /// [`ActionError::BadDomain`], [`ActionError::BadPort`], or [`ActionError::BadPath`].
    pub fn configure_site(
        domain: &str,
        upstream_port: u16,
        php_version: Option<PhpVersion>,
        tls_enabled: bool,
        root_dir: &str,
    ) -> Result<Self, ActionError> {
        if upstream_port == 0 {
            return Err(ActionError::BadPort);
        }
        Ok(Self::ConfigureSite {
            domain: SiteDomain::new(domain).map_err(|_| ActionError::BadDomain)?,
            upstream_port,
            php_version,
            tls_enabled,
            root_dir: ManagedPath::new(root_dir)?,
        })
    }

    /// Issues a TLS certificate for a production domain via `acme_provider`.
    ///
    /// # Errors
    /// [`ActionError::BadDomain`].
    pub fn issue_tls_cert(domain: &str, acme_provider: AcmeProvider) -> Result<Self, ActionError> {
        Ok(Self::IssueTlsCert {
            domain: SiteDomain::new_production(domain).map_err(|_| ActionError::BadDomain)?,
            acme_provider,
        })
    }

    /// Backs up a database to `output_path` without passing credentials in argv.
    ///
    /// # Errors
    /// [`ActionError::BadDbName`] or [`ActionError::BadPath`].
    pub fn backup_database(
        engine: DbEngine,
        db_name: &str,
        output_path: &str,
    ) -> Result<Self, ActionError> {
        Ok(Self::BackupDatabase {
            engine,
            db_name: DbName::new(db_name)?,
            output_path: ManagedPath::new(output_path)?,
        })
    }

    /// The tool name stored in `proposals.tool`.
    #[must_use]
    pub const fn tool(&self) -> &'static str {
        match self {
            Self::RestartService { .. } => "propose_restart_service",
            Self::UpdateEnvVar { .. } => "propose_update_env_var",
            Self::DeleteManagedFile { .. } => "propose_delete_managed_file",
            Self::ConfigureSite { .. } => "propose_configure_site",
            Self::IssueTlsCert { .. } => "propose_issue_tls_cert",
            Self::BackupDatabase { .. } => "propose_backup_database",
        }
    }

    /// The tier, fixed here.
    #[must_use]
    pub fn tier(&self) -> Tier {
        match self {
            Self::RestartService { name, .. } if name.is_critical() => Tier::RiskyChange,
            Self::RestartService { .. } | Self::BackupDatabase { .. } => Tier::LightChange,
            Self::ConfigureSite {
                domain,
                tls_enabled,
                ..
            } => {
                if domain.is_test_domain() && !*tls_enabled {
                    Tier::LightChange
                } else {
                    Tier::RiskyChange
                }
            }
            Self::UpdateEnvVar { .. } | Self::IssueTlsCert { .. } => Tier::RiskyChange,
            Self::DeleteManagedFile { .. } => Tier::Destructive,
        }
    }

    /// Human-readable target identifier (also used as the exact string required for tier-4 typed
    /// confirmation).
    #[must_use]
    pub fn target_label(&self) -> &str {
        match self {
            Self::RestartService { name, .. } => name.as_str(),
            Self::UpdateEnvVar { path, .. } | Self::DeleteManagedFile { path } => path.as_str(),
            Self::ConfigureSite { domain, .. } | Self::IssueTlsCert { domain, .. } => {
                domain.as_str()
            }
            Self::BackupDatabase { db_name, .. } => db_name.as_str(),
        }
    }

    /// Whether executing this action requires a pre-execution backup (`tier >= 3`).
    #[must_use]
    pub fn requires_backup(&self) -> bool {
        self.tier().requires_dry_run_and_backup()
    }

    /// Whether approving this action requires typing [`Action::target_label`] (`tier == 4`).
    #[must_use]
    pub fn requires_typed_confirmation(&self) -> bool {
        self.tier().requires_typed_confirmation()
    }

    /// Whether the executor can run this action today (tiers 2, 3, and 4).
    #[must_use]
    pub fn executable_now(&self) -> bool {
        self.tier().requires_proposal()
    }

    /// The one canonical JSON form: fixed alphabetical key order, no extra spaces.
    #[must_use]
    pub fn canonical_json(&self) -> String {
        match self {
            Self::RestartService { manager, name } => format!(
                r#"{{"action":"restart_service","manager":"{}","service":"{}"}}"#,
                manager.as_str(),
                name.as_str()
            ),
            Self::UpdateEnvVar { path, key, value } => serde_json::json!({
                "action": "update_env_var",
                "key": key.as_str(),
                "path": path.as_str(),
                "value": value.expose(),
            })
            .to_string(),
            Self::DeleteManagedFile { path } => serde_json::json!({
                "action": "delete_managed_file",
                "path": path.as_str(),
            })
            .to_string(),
            Self::ConfigureSite {
                domain,
                upstream_port,
                php_version,
                tls_enabled,
                root_dir,
            } => serde_json::json!({
                "action": "configure_site",
                "domain": domain.as_str(),
                "php_version": php_version.map(PhpVersion::as_str),
                "root_dir": root_dir.as_str(),
                "tls_enabled": tls_enabled,
                "upstream_port": upstream_port,
            })
            .to_string(),
            Self::IssueTlsCert {
                domain,
                acme_provider,
            } => serde_json::json!({
                "acme_provider": acme_provider.as_str(),
                "action": "issue_tls_cert",
                "domain": domain.as_str(),
            })
            .to_string(),
            Self::BackupDatabase {
                engine,
                db_name,
                output_path,
            } => serde_json::json!({
                "action": "backup_database",
                "db_name": db_name.as_str(),
                "engine": engine.as_str(),
                "output_path": output_path.as_str(),
            })
            .to_string(),
        }
    }

    /// SHA-256 hex of [`Action::canonical_json`]: what an approval binds to.
    #[must_use]
    pub fn hash(&self) -> String {
        db::sha256_hex(self.canonical_json().as_bytes())
    }

    /// Reads the strict JSON form. Re-validates every field (the database is not trusted to hold
    /// only values this code wrote).
    ///
    /// # Errors
    /// [`ActionError`].
    pub fn parse(json: &str) -> Result<Self, ActionError> {
        let value: serde_json::Value =
            serde_json::from_str(json).map_err(|_| ActionError::BadShape)?;
        let object = value.as_object().ok_or(ActionError::BadShape)?;
        let text = |key: &str| {
            object
                .get(key)
                .and_then(serde_json::Value::as_str)
                .ok_or(ActionError::BadShape)
        };
        match text("action")? {
            "restart_service" => {
                if object.len() != 3 {
                    return Err(ActionError::BadShape);
                }
                let manager = ServiceManagerKind::parse(text("manager")?)
                    .ok_or(ActionError::UnknownManager)?;
                Self::restart_service(manager, text("service")?)
            }
            "update_env_var" => {
                if object.len() != 4 {
                    return Err(ActionError::BadShape);
                }
                Self::update_env_var(text("path")?, text("key")?, text("value")?)
            }
            "delete_managed_file" => {
                if object.len() != 2 {
                    return Err(ActionError::BadShape);
                }
                Self::delete_managed_file(text("path")?)
            }
            "configure_site" => {
                if object.len() != 6 {
                    return Err(ActionError::BadShape);
                }
                let port_u64 = object
                    .get("upstream_port")
                    .and_then(serde_json::Value::as_u64)
                    .ok_or(ActionError::BadShape)?;
                let upstream_port = u16::try_from(port_u64).map_err(|_| ActionError::BadPort)?;
                let tls_enabled = object
                    .get("tls_enabled")
                    .and_then(serde_json::Value::as_bool)
                    .ok_or(ActionError::BadShape)?;
                let php_version = match object.get("php_version").ok_or(ActionError::BadShape)? {
                    serde_json::Value::Null => None,
                    serde_json::Value::String(s) => {
                        Some(PhpVersion::parse(s).ok_or(ActionError::UnknownPhpVersion)?)
                    }
                    _ => return Err(ActionError::BadShape),
                };
                Self::configure_site(
                    text("domain")?,
                    upstream_port,
                    php_version,
                    tls_enabled,
                    text("root_dir")?,
                )
            }
            "issue_tls_cert" => {
                if object.len() != 3 {
                    return Err(ActionError::BadShape);
                }
                let provider = AcmeProvider::parse(text("acme_provider")?)
                    .ok_or(ActionError::UnknownAcmeProvider)?;
                Self::issue_tls_cert(text("domain")?, provider)
            }
            "backup_database" => {
                if object.len() != 4 {
                    return Err(ActionError::BadShape);
                }
                let engine =
                    DbEngine::parse(text("engine")?).ok_or(ActionError::UnknownDbEngine)?;
                Self::backup_database(engine, text("db_name")?, text("output_path")?)
            }
            _ => Err(ActionError::UnknownAction),
        }
    }
}
