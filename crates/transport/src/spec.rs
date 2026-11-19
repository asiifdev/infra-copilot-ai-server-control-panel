//! The closed set of transports and their validated settings. Specs hold no secrets: they are
//! what the `transports.config` column stores. Credentials are separate ([`crate::Credentials`]).

use crate::validate::{self, ValidationError};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::fmt;

/// The kind of transport (the `transports.kind` column).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TransportKind {
    /// rclone to a cloud remote.
    Rclone,
    /// rsync over SSH.
    Rsync,
    /// scp.
    Scp,
    /// A stream piped through SSH.
    SshPipe,
    /// Product-managed storage (cloud edition). Declared, not implemented.
    Managed,
}

impl TransportKind {
    /// Every kind, in a stable order.
    pub const ALL: [Self; 5] = [
        Self::Rclone,
        Self::Rsync,
        Self::Scp,
        Self::SshPipe,
        Self::Managed,
    ];

    /// The database text.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rclone => "rclone",
            Self::Rsync => "rsync",
            Self::Scp => "scp",
            Self::SshPipe => "ssh_pipe",
            Self::Managed => "managed",
        }
    }

    /// Parses the database text.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == text)
    }

    /// Whether credentials may be attached. SSH kinds authenticate through the agent and the
    /// host's own key setup, never through a secret this product holds.
    #[must_use]
    pub const fn accepts_credentials(self) -> bool {
        matches!(self, Self::Rclone)
    }
}

/// The rclone backend (`type`) of a remote. A closed list: adding a backend is a code change
/// with its own review, not a free-text field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RemoteKind {
    /// S3 and S3-compatible storage.
    S3,
    /// Backblaze B2.
    B2,
    /// Google Drive.
    Drive,
    /// SFTP.
    Sftp,
    /// Dropbox.
    Dropbox,
    /// Microsoft OneDrive.
    Onedrive,
    /// WebDAV.
    Webdav,
}

impl RemoteKind {
    /// Every backend.
    pub const ALL: [Self; 7] = [
        Self::S3,
        Self::B2,
        Self::Drive,
        Self::Sftp,
        Self::Dropbox,
        Self::Onedrive,
        Self::Webdav,
    ];

    /// The rclone options this backend may carry, plain or secret. A closed allow-list of
    /// settings that only describe a service or hold a credential value. Options that run a
    /// program (`ssh`, `*_command`), read a local file (`key_file`, `known_hosts_file`,
    /// `pubkey_file`) or pull ambient credentials (`env_auth`) are deliberately absent.
    #[must_use]
    pub const fn allowed_options(self) -> &'static [&'static str] {
        match self {
            Self::S3 => &[
                "provider",
                "region",
                "endpoint",
                "location_constraint",
                "acl",
                "storage_class",
                "access_key_id",
                "secret_access_key",
                "session_token",
            ],
            Self::B2 => &["account", "key", "endpoint"],
            Self::Drive => &[
                "client_id",
                "client_secret",
                "token",
                "scope",
                "root_folder_id",
                "team_drive",
            ],
            Self::Sftp => &["host", "user", "port", "pass", "key_pem", "key_file_pass"],
            Self::Dropbox => &["client_id", "client_secret", "token"],
            Self::Onedrive => &[
                "client_id",
                "client_secret",
                "token",
                "drive_id",
                "drive_type",
                "region",
            ],
            Self::Webdav => &["url", "vendor", "user", "pass", "bearer_token"],
        }
    }

    /// The rclone backend name (the value of the `type` option).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::S3 => "s3",
            Self::B2 => "b2",
            Self::Drive => "drive",
            Self::Sftp => "sftp",
            Self::Dropbox => "dropbox",
            Self::Onedrive => "onedrive",
            Self::Webdav => "webdav",
        }
    }

    /// Parses the backend name.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == text)
    }

    /// Whether the remote is authorised by an OAuth flow that yields a `token`.
    #[must_use]
    pub const fn needs_oauth(self) -> bool {
        matches!(self, Self::Drive | Self::Dropbox | Self::Onedrive)
    }
}

/// An rclone remote: backend, the name used in environment variables and arguments, the path
/// inside it, and non-secret options (region, endpoint, ...).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RcloneSpec {
    kind: RemoteKind,
    remote: String,
    path: String,
    options: BTreeMap<String, String>,
}

/// An SSH destination for rsync, scp and ssh pipe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshTarget {
    host: String,
    user: String,
    port: u16,
    path: String,
}

/// Why a spec could not be built or read back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecError {
    /// A field failed validation.
    Invalid(ValidationError),
    /// The stored kind text is unknown.
    UnknownKind,
    /// The stored JSON is not the expected shape.
    BadConfig,
}

impl From<ValidationError> for SpecError {
    fn from(e: ValidationError) -> Self {
        Self::Invalid(e)
    }
}

impl fmt::Display for SpecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(e) => e.fmt(f),
            Self::UnknownKind => f.write_str("transport_unknown_kind"),
            Self::BadConfig => f.write_str("transport_bad_config"),
        }
    }
}

impl std::error::Error for SpecError {}

/// Default SSH port.
pub const SSH_PORT: u16 = 22;
/// Most options an rclone spec may carry.
const MAX_OPTIONS: usize = 32;

impl RcloneSpec {
    /// Builds and validates a spec.
    ///
    /// # Errors
    /// [`SpecError::Invalid`] naming the first field that fails.
    pub fn new(
        kind: RemoteKind,
        remote: &str,
        path: &str,
        options: BTreeMap<String, String>,
    ) -> Result<Self, SpecError> {
        validate::remote_name(remote)?;
        validate::rclone_path(path)?;
        if options.len() > MAX_OPTIONS {
            return Err(validate::ValidationError {
                field: validate::Field::OptionName,
                problem: validate::Problem::TooLong,
            }
            .into());
        }
        for (name, value) in &options {
            validate::option_name(name)?;
            validate::option_allowed(kind, name)?;
            validate::option_value(value)?;
        }
        Ok(Self {
            kind,
            remote: remote.to_owned(),
            path: path.to_owned(),
            options,
        })
    }

    /// The backend.
    #[must_use]
    pub const fn kind(&self) -> RemoteKind {
        self.kind
    }

    /// The remote name.
    #[must_use]
    pub fn remote(&self) -> &str {
        &self.remote
    }

    /// The path inside the remote (may be empty).
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Non-secret options, sorted by name.
    #[must_use]
    pub const fn options(&self) -> &BTreeMap<String, String> {
        &self.options
    }

    /// `remote:path` with `leaf` appended under the path.
    #[must_use]
    pub fn remote_arg(&self, leaf: Option<&str>) -> String {
        let mut joined = self.path.clone();
        if let Some(leaf) = leaf {
            if !joined.is_empty() && !joined.ends_with('/') {
                joined.push('/');
            }
            joined.push_str(leaf);
        }
        format!("{}:{joined}", self.remote)
    }
}

impl SshTarget {
    /// Builds and validates a target. `port` defaults to 22 when `None`.
    ///
    /// # Errors
    /// [`SpecError::Invalid`] naming the first field that fails.
    pub fn new(host: &str, user: &str, port: Option<u16>, path: &str) -> Result<Self, SpecError> {
        validate::host(host)?;
        validate::user(user)?;
        let port = port.unwrap_or(SSH_PORT);
        validate::port(port)?;
        validate::ssh_path(path)?;
        Ok(Self {
            host: host.to_owned(),
            user: user.to_owned(),
            port,
            path: path.to_owned(),
        })
    }

    /// Host name or IP literal.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Login name.
    #[must_use]
    pub fn user(&self) -> &str {
        &self.user
    }

    /// TCP port.
    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// Path on the host.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// `user@host` for ssh.
    #[must_use]
    pub fn login(&self) -> String {
        format!("{}@{}", self.user, self.host)
    }

    /// `user@host:path` for rsync and scp (IPv6 literals bracketed).
    #[must_use]
    pub fn remote_arg(&self) -> String {
        if validate::is_ipv6(&self.host) {
            format!("{}@[{}]:{}", self.user, self.host, self.path)
        } else {
            format!("{}@{}:{}", self.user, self.host, self.path)
        }
    }
}

/// A validated transport. There is no way to build one with an unchecked field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportSpec {
    /// rclone to a cloud remote.
    Rclone(RcloneSpec),
    /// rsync over SSH.
    Rsync(SshTarget),
    /// scp.
    Scp(SshTarget),
    /// A stream piped through SSH.
    SshPipe(SshTarget),
    /// Product-managed storage. Declared but not implemented: every plan is `Unsupported`.
    Managed,
}

impl TransportSpec {
    /// The kind.
    #[must_use]
    pub const fn kind(&self) -> TransportKind {
        match self {
            Self::Rclone(_) => TransportKind::Rclone,
            Self::Rsync(_) => TransportKind::Rsync,
            Self::Scp(_) => TransportKind::Scp,
            Self::SshPipe(_) => TransportKind::SshPipe,
            Self::Managed => TransportKind::Managed,
        }
    }

    /// The JSON object stored in `transports.config`. Never contains a secret.
    #[must_use]
    pub fn to_config_json(&self) -> String {
        let value = match self {
            Self::Rclone(s) => json!({
                "remote_kind": s.kind.as_str(),
                "remote": s.remote,
                "path": s.path,
                "options": s.options,
            }),
            Self::Rsync(t) | Self::Scp(t) | Self::SshPipe(t) => json!({
                "host": t.host,
                "user": t.user,
                "port": t.port,
                "path": t.path,
            }),
            Self::Managed => json!({}),
        };
        value.to_string()
    }

    /// Reads a stored row back, re-validating every field (the database is not trusted to hold
    /// only values this code wrote).
    ///
    /// # Errors
    /// [`SpecError::UnknownKind`], [`SpecError::BadConfig`] or [`SpecError::Invalid`].
    pub fn from_config_json(kind: &str, config: &str) -> Result<Self, SpecError> {
        let kind = TransportKind::parse(kind).ok_or(SpecError::UnknownKind)?;
        let value: Value = serde_json::from_str(config).map_err(|_| SpecError::BadConfig)?;
        let obj = value.as_object().ok_or(SpecError::BadConfig)?;
        let get = |key: &str| {
            obj.get(key)
                .and_then(Value::as_str)
                .ok_or(SpecError::BadConfig)
        };
        match kind {
            TransportKind::Rclone => {
                let remote_kind =
                    RemoteKind::parse(get("remote_kind")?).ok_or(SpecError::BadConfig)?;
                let mut options = BTreeMap::new();
                if let Some(raw) = obj.get("options") {
                    let map: &Map<String, Value> = raw.as_object().ok_or(SpecError::BadConfig)?;
                    for (k, v) in map {
                        options.insert(
                            k.clone(),
                            v.as_str().ok_or(SpecError::BadConfig)?.to_owned(),
                        );
                    }
                }
                Ok(Self::Rclone(RcloneSpec::new(
                    remote_kind,
                    get("remote")?,
                    get("path")?,
                    options,
                )?))
            }
            TransportKind::Rsync | TransportKind::Scp | TransportKind::SshPipe => {
                let port = match obj.get("port") {
                    None => None,
                    Some(v) => Some(
                        v.as_u64()
                            .and_then(|n| u16::try_from(n).ok())
                            .ok_or(SpecError::BadConfig)?,
                    ),
                };
                let target = SshTarget::new(get("host")?, get("user")?, port, get("path")?)?;
                Ok(match kind {
                    TransportKind::Rsync => Self::Rsync(target),
                    TransportKind::Scp => Self::Scp(target),
                    _ => Self::SshPipe(target),
                })
            }
            TransportKind::Managed => Ok(Self::Managed),
        }
    }
}
