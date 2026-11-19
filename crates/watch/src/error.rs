//! Closed error types of this crate. None carries text from a server, a token or a URL.

use std::fmt;

/// Why a [`crate::WatchConfig`] was rejected or an engine could not be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchError {
    /// The JSON could not be parsed (details are dropped on purpose).
    Parse,
    /// Monitor `index` failed `MonitorConfig::validate`.
    Monitor {
        /// Position in `WatchConfig::monitors`.
        index: usize,
        /// The closed reason.
        error: monitor::ConfigError,
    },
    /// Two monitors share an id.
    DuplicateId,
    /// More monitors than [`crate::MAX_MONITORS`].
    TooManyMonitors,
    /// The channel settings were rejected by the channel.
    Channel(notifier::ConfigError),
    /// Unknown locale tag.
    Locale,
    /// `token_ref` is not a plain environment-variable style name.
    TokenRef,
    /// A throttle or concurrency setting is zero.
    Limits,
    /// The built-in message catalog failed to load.
    Catalog,
}

impl WatchError {
    /// Stable machine code, safe to log.
    pub const fn code(self) -> &'static str {
        match self {
            WatchError::Parse => "watch_parse",
            WatchError::Monitor { .. } => "watch_monitor",
            WatchError::DuplicateId => "watch_duplicate_id",
            WatchError::TooManyMonitors => "watch_too_many_monitors",
            WatchError::Channel(_) => "watch_channel",
            WatchError::Locale => "watch_locale",
            WatchError::TokenRef => "watch_token_ref",
            WatchError::Limits => "watch_limits",
            WatchError::Catalog => "watch_catalog",
        }
    }
}

impl fmt::Display for WatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for WatchError {}
