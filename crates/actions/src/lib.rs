//! Actions: the closed list of things an approved proposal may do (ADR 0037).
//!
//! An [`Action`] is data, never a command line. It is parsed from a strict JSON form, every field
//! is validated, its tier is fixed by this code (neither a model nor a person can lower it), and
//! it has one canonical form whose SHA-256 binds an approval to exactly this action. The
//! [`Executor`] turns an action into argv-only commands and runs them through a
//! `transport::Runner`, so tests never touch a real service.
//!
//! Only tier 2 actions exist so far. Tier 3 and 4 need dry-run with backup, typed confirmation
//! and rollback, and are refused by [`Action::executable_now`] until those exist.

mod exec;

pub use exec::{DryRun, Executor, LocalExecutor, Outcome, OutcomeStatus};

use core_domain::tier::Tier;
use std::fmt;

/// Longest service name accepted.
pub const MAX_SERVICE_NAME: usize = 100;

/// Why an action could not be built or read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionError {
    /// The text is not the strict JSON form (not an object, unknown or missing key, wrong type).
    BadShape,
    /// The action name is not on the list.
    UnknownAction,
    /// The service manager is not on the list.
    UnknownManager,
    /// The service name is empty, too long, starts with `-` or `.`, or has a character outside
    /// `A-Z a-z 0-9 _ . @ -`.
    BadServiceName,
}

impl fmt::Display for ActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::BadShape => "action_bad_shape",
            Self::UnknownAction => "action_unknown",
            Self::UnknownManager => "action_unknown_manager",
            Self::BadServiceName => "action_bad_service_name",
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

/// A validated service name. There is no way to build one that fails validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceName(String);

impl ServiceName {
    /// Validates a service name.
    ///
    /// # Errors
    /// [`ActionError::BadServiceName`].
    pub fn new(text: &str) -> Result<Self, ActionError> {
        let ok = !text.is_empty()
            && text.len() <= MAX_SERVICE_NAME
            && !text.starts_with(['-', '.'])
            && text
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '@' | '-'));
        if ok {
            Ok(Self(text.to_owned()))
        } else {
            Err(ActionError::BadServiceName)
        }
    }

    /// The name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One thing an approved proposal may do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Restart a service (tier 2). No undo exists: a restart cannot be reversed, only repeated.
    RestartService {
        /// How the service is managed.
        manager: ServiceManagerKind,
        /// Its name.
        name: ServiceName,
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

    /// The tool name stored in `proposals.tool`.
    #[must_use]
    pub const fn tool(&self) -> &'static str {
        match self {
            Self::RestartService { .. } => "propose_restart_service",
        }
    }

    /// The tier, fixed here.
    #[must_use]
    pub const fn tier(&self) -> Tier {
        match self {
            Self::RestartService { .. } => Tier::LightChange,
        }
    }

    /// Whether the executor can run this action today. False for tier 3 and 4 until dry-run with
    /// backup, typed confirmation and rollback exist.
    #[must_use]
    pub const fn executable_now(&self) -> bool {
        matches!(self.tier(), Tier::LightChange)
    }

    /// The one canonical JSON form: fixed key order, no spaces. Values are validated, so nothing
    /// here needs escaping.
    #[must_use]
    pub fn canonical_json(&self) -> String {
        match self {
            Self::RestartService { manager, name } => format!(
                r#"{{"action":"restart_service","manager":"{}","service":"{}"}}"#,
                manager.as_str(),
                name.as_str()
            ),
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
            _ => Err(ActionError::UnknownAction),
        }
    }
}
