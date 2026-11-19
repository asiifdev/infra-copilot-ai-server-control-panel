//! Errors of the provider. Fixed text: no server text, URL or key ever appears.

use crate::transport::TransportError;
use std::fmt;

/// Why a chat call failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AiError {
    /// The configuration is unusable (bad URL, bad key characters).
    Config(&'static str),
    /// The provider is not configured for something the request needs (tools, streaming).
    Unsupported(&'static str),
    /// The connection failed before an answer arrived.
    Transport(TransportError),
    /// 401 or 403: the key is missing, wrong or not allowed.
    Auth,
    /// The account is out of credit or quota. Retrying does not help; tell an administrator.
    Quota,
    /// 429: slow down. Carries the server's `Retry-After` in seconds when it sent one.
    RateLimited {
        /// Seconds to wait, capped at one hour.
        retry_after_secs: Option<u32>,
    },
    /// 5xx: the provider failed.
    Server(u16),
    /// 4xx other than the above: the request itself was refused.
    Rejected(u16),
    /// The answer did not follow the protocol (bad JSON, missing fields, over the size caps).
    Malformed(&'static str),
    /// The answer had neither text nor tool calls.
    Empty,
    /// The circuit breaker is open after repeated failures.
    CircuitOpen,
}

impl AiError {
    /// Stable machine code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Config(_) => "ai_config",
            Self::Unsupported(_) => "ai_unsupported",
            Self::Transport(_) => "ai_transport",
            Self::Auth => "ai_auth",
            Self::Quota => "ai_quota",
            Self::RateLimited { .. } => "ai_rate_limited",
            Self::Server(_) => "ai_server",
            Self::Rejected(_) => "ai_rejected",
            Self::Malformed(_) => "ai_malformed",
            Self::Empty => "ai_empty",
            Self::CircuitOpen => "ai_circuit_open",
        }
    }

    /// Whether trying the same request again can succeed.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        match self {
            Self::Transport(t) => t.is_transient(),
            Self::RateLimited { .. } | Self::Server(_) => true,
            _ => false,
        }
    }

    /// The server's wait hint, if any.
    #[must_use]
    pub const fn retry_after_secs(&self) -> Option<u32> {
        match self {
            Self::RateLimited { retry_after_secs } => *retry_after_secs,
            _ => None,
        }
    }

    /// Whether this failure says the provider itself is unhealthy (counts toward the breaker).
    pub(crate) const fn counts_against_breaker(&self) -> bool {
        matches!(
            self,
            Self::Transport(_) | Self::Server(_) | Self::RateLimited { .. } | Self::Malformed(_)
        )
    }
}

impl fmt::Display for AiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(why) | Self::Unsupported(why) | Self::Malformed(why) => {
                write!(f, "{}: {why}", self.code())
            }
            Self::Transport(t) => write!(f, "{}: {t}", self.code()),
            Self::Server(s) | Self::Rejected(s) => write!(f, "{}: {s}", self.code()),
            _ => f.write_str(self.code()),
        }
    }
}

impl std::error::Error for AiError {}
