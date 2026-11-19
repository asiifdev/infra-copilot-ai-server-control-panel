//! Where the bot token comes from, behind a trait.
//!
//! **Stop-gap.** The product keeps secrets in the encrypted database (CLAUDE.md rule 10, ADR 0013).
//! [`EnvSecrets`] reads an environment variable so the standalone watcher can run before that
//! exists. A DB-backed source replaces it by implementing [`SecretSource`]; nothing else changes.
//! The config only ever holds the secret's NAME (`token_ref`).

use notifier::Secret;
use std::fmt;
use std::future::Future;

/// Why a secret could not be produced. Closed; never contains the name's value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretError {
    /// No such secret, or it is empty.
    NotFound,
    /// The secret exists but is not valid text.
    Invalid,
}

impl SecretError {
    /// Stable machine code.
    pub const fn code(self) -> &'static str {
        match self {
            SecretError::NotFound => "secret_not_found",
            SecretError::Invalid => "secret_invalid",
        }
    }
}

impl fmt::Display for SecretError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for SecretError {}

/// Resolves a secret by NAME at the moment it is needed (never cached by the engine).
pub trait SecretSource: Send + Sync {
    /// Returns the secret called `name`.
    fn get(&self, name: &str) -> impl Future<Output = Result<Secret, SecretError>> + Send;
}

/// Interim source: the secret is the value of the environment variable called `name`.
#[derive(Debug, Clone, Copy, Default)]
pub struct EnvSecrets;

impl SecretSource for EnvSecrets {
    async fn get(&self, name: &str) -> Result<Secret, SecretError> {
        match std::env::var(name) {
            Ok(value) if !value.is_empty() => Ok(Secret::new(value)),
            Ok(_) | Err(std::env::VarError::NotPresent) => Err(SecretError::NotFound),
            Err(std::env::VarError::NotUnicode(_)) => Err(SecretError::Invalid),
        }
    }
}
