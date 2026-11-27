//! One thing found on a machine.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// What an [`Item`] describes. Stable wire names; new kinds are added, never renamed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A TCP port something listens on (docs/design.md 4.1).
    ///
    /// Key `tcp:<port>`. Data: `proto` (`tcp`), `port` (number), `bind` (address string), `process`
    /// (name or null), `pid_hint` (number or null, informational only), `exposed`
    /// (`public` | `loopback` | `lan`).
    Listener,
    /// A running process (docs/design.md 4.1).
    ///
    /// Key `<manager>:<name>`, e.g. `pm2:web-app`; `proc:<name>` when no supervisor is known. Data:
    /// `cmd` (redacted command line), `cwd`, `mode` (`dev` | `prod` | `unknown`), `supervised`
    /// (bool, or null when not determined), `env_refs` (names only, never values).
    Process,
    /// A scheduled job (docs/design.md 4.1).
    ///
    /// Key `<scope>:<owner>:<schedule>:<command hint>`, e.g. `user:root:0 3 * * *:backup`. Data:
    /// `schedule`, `command_hint` (redacted), `owner`, `source` (`crontab` | `timer` | `panel`).
    Cron,
    /// A configuration reference from an application to a host and port (docs/design.md 4.1).
    ///
    /// Key `<process key>:<NAME>`, e.g. `pm2:web:DATABASE_URL`. Data: `app` (the process key),
    /// `name` (variable name), `target_host` (host string), `target_port` (number or null),
    /// `in_container` (bool: the reference lives inside a container, so `localhost` means the
    /// container itself), `resolves_to` (`listener:tcp:<port>` of the same inventory, or null).
    /// Never contains the variable's value, a user name, a password or a path (decision 0015).
    EnvRef,
    /// A web virtual host (`nginx` or `caddy`), read-only from configuration.
    Vhost,
    /// TLS certificate metadata (path, subject, expiry); never a private key.
    Cert,
    /// Database metadata (engine, version, name, row count); never credentials.
    Database,
    /// Public DNS resolution check for a domain against a target address.
    Dns,
    /// Functional check result (e.g. process DB connection or test login).
    Check,
}

impl Kind {
    /// The wire name (`listener`, `process`, `cron`, `env_ref`, `vhost`, `cert`, `database`,
    /// `dns`, `check`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Listener => "listener",
            Self::Process => "process",
            Self::Cron => "cron",
            Self::EnvRef => "env_ref",
            Self::Vhost => "vhost",
            Self::Cert => "cert",
            Self::Database => "database",
            Self::Dns => "dns",
            Self::Check => "check",
        }
    }
}

/// How an item was discovered, so a finding can explain itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeenVia {
    /// Read from the process filesystem.
    Proc,
    /// Read from the systemd service manager.
    Systemd,
    /// Read from the container runtime.
    Docker,
    /// Read from a configuration or schedule file.
    Config,
}

/// One inventory entry. `key` is stable and comparable across machines (a port, not a PID).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    /// What it is.
    pub kind: Kind,
    /// Normalised identifier, unique per kind within an inventory.
    pub key: String,
    /// Kind-specific fields. Never contains secret values.
    pub data: Map<String, Value>,
    /// How it was found.
    pub seen_via: SeenVia,
}

impl Item {
    /// A string field of `data`, if present.
    #[must_use]
    pub fn str_field(&self, name: &str) -> Option<&str> {
        self.data.get(name).and_then(Value::as_str)
    }

    /// An unsigned integer field of `data`, if present.
    #[must_use]
    pub fn u64_field(&self, name: &str) -> Option<u64> {
        self.data.get(name).and_then(Value::as_u64)
    }

    /// A boolean field of `data`, if present.
    #[must_use]
    pub fn bool_field(&self, name: &str) -> Option<bool> {
        self.data.get(name).and_then(Value::as_bool)
    }
}
