//! The channel contract and its closed value and error types.

use i18n::{Catalog, Locale};
use std::fmt;
use std::future::Future;

/// How urgent a message is. Ordered: `Info < Warning < Critical`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// Good news or routine (recovery, certificate ok).
    Info,
    /// Needs attention soon (certificate expiring).
    Warning,
    /// Needs attention now (target down).
    Critical,
}

/// A rendered, channel-neutral message. Plain text, no markup; channels do their own escaping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// Urgency, used by policy helpers and channels.
    pub severity: Severity,
    /// One-line title.
    pub title: String,
    /// Body lines joined by `\n`; may be empty.
    pub body: String,
}

/// Quiet hours in the recipient's fixed UTC offset (no DST handling by design).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuietHours {
    /// Window start, minutes after local midnight (0..1440).
    pub start_min: u16,
    /// Window end (exclusive), minutes after local midnight (0..1440). `start == end` means no
    /// window; `start > end` wraps past midnight.
    pub end_min: u16,
    /// Fixed offset from UTC in minutes (-1080..=1080).
    pub utc_offset_min: i16,
    /// When true, `Critical` messages ignore quiet hours.
    pub critical_bypasses: bool,
}

/// Per-channel settings. Holds no secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelConfig {
    /// Destination chat (numeric id, or `@channelname` for public channels).
    pub chat_id: String,
    /// Messages below this severity are not sent.
    pub min_severity: Severity,
    /// Optional quiet hours.
    pub quiet_hours: Option<QuietHours>,
    /// Language for rendered text.
    pub locale: Locale,
}

/// Invalid channel configuration. Closed set, no echoed input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigError {
    /// The chat id is empty or malformed.
    ChatId,
    /// Quiet hours minutes or offset out of range.
    QuietHours,
}

impl ConfigError {
    /// Stable machine code.
    pub const fn code(self) -> &'static str {
        match self {
            ConfigError::ChatId => "config_chat_id",
            ConfigError::QuietHours => "config_quiet_hours",
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for ConfigError {}

pub use core_domain::secret::Secret;

/// Proof of delivery.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SendReceipt {
    /// Provider message id when known.
    pub message_id: Option<i64>,
}

/// Why a send failed. Closed set; never carries server text, URLs or the secret.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendError {
    /// The provider asks to slow down.
    RateLimited {
        /// Seconds to wait before retrying (capped at one day).
        retry_after_secs: u32,
    },
    /// The secret is rejected or malformed (401, bad token shape, empty secret).
    Unauthorized,
    /// The bot may not post to this chat (403: blocked, kicked).
    Forbidden,
    /// The chat does not exist or the bot has never seen it.
    ChatNotFound,
    /// Other 400-class rejection (bad request).
    BadRequest,
    /// Provider-side failure (5xx).
    ServerError,
    /// The provider answered with something unexpected.
    InvalidResponse,
    /// The network layer failed.
    Transport(TransportError),
}

impl SendError {
    /// Stable machine code (safe to log and show).
    pub const fn code(self) -> &'static str {
        match self {
            SendError::RateLimited { .. } => "send_rate_limited",
            SendError::Unauthorized => "send_unauthorized",
            SendError::Forbidden => "send_forbidden",
            SendError::ChatNotFound => "send_chat_not_found",
            SendError::BadRequest => "send_bad_request",
            SendError::ServerError => "send_server_error",
            SendError::InvalidResponse => "send_invalid_response",
            SendError::Transport(_) => "send_transport",
        }
    }

    /// Whether retrying later may succeed.
    pub const fn is_retryable(self) -> bool {
        matches!(
            self,
            SendError::RateLimited { .. } | SendError::ServerError | SendError::Transport(_)
        )
    }
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for SendError {}

/// Network failure classes. Implementations must map into this closed set and must not keep the
/// request URL (it contains the bot token) in any error, log or trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportError {
    /// The request timed out.
    Timeout,
    /// DNS or TCP connection failed.
    Connect,
    /// TLS handshake or certificate failed.
    Tls,
    /// Anything else.
    Other,
}

/// The only network access this crate needs. Implemented by the HTTP client crate; faked in
/// tests. The `url` may contain a secret: never log it.
pub trait HttpTransport: Send + Sync {
    /// POSTs `body` as `application/json` and returns the status code and response body.
    fn post_json(
        &self,
        url: &str,
        body: &[u8],
    ) -> impl Future<Output = Result<(u16, Vec<u8>), TransportError>> + Send;
}

impl<T: HttpTransport + ?Sized> HttpTransport for &T {
    fn post_json(
        &self,
        url: &str,
        body: &[u8],
    ) -> impl Future<Output = Result<(u16, Vec<u8>), TransportError>> + Send {
        (*self).post_json(url, body)
    }
}

/// A one-way notification channel (Telegram, email, webhook, ...).
///
/// Not object safe (methods return `impl Future`): use generics or an enum for dispatch.
/// Every implementation must run `testing::contract_suite`.
pub trait NotificationChannel: Sync {
    /// Stable lowercase kind, for example `telegram`.
    fn kind(&self) -> &'static str;

    /// Checks `cfg` without any network access.
    fn validate_config(&self, cfg: &ChannelConfig) -> Result<(), ConfigError>;

    /// Delivers `msg`. The secret is injected here and nowhere else.
    fn send(
        &self,
        msg: &Message,
        cfg: &ChannelConfig,
        secret: &Secret,
    ) -> impl Future<Output = Result<SendReceipt, SendError>> + Send;

    /// Sends the localized test message, proving the whole path works.
    fn test(
        &self,
        cfg: &ChannelConfig,
        secret: &Secret,
        catalog: &Catalog,
    ) -> impl Future<Output = Result<SendReceipt, SendError>> + Send {
        let msg = crate::render_test(cfg.locale, catalog);
        async move { self.send(&msg, cfg, secret).await }
    }
}
