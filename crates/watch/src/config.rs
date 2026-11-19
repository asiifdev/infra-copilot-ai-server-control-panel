//! The standalone watch configuration (serde). It never contains the Telegram token.

use crate::error::WatchError;
use i18n::Locale;
use monitor::MonitorConfig;
use notifier::{ChannelConfig, QuietHours, Severity};
use serde::{Deserialize, Serialize};

/// Upper bound on monitors per engine.
pub const MAX_MONITORS: usize = 1000;
const MAX_TOKEN_REF_LEN: usize = 128;

fn default_throttle() -> u32 {
    20
}
fn default_in_flight() -> usize {
    16
}
fn default_locale() -> String {
    "en".to_owned()
}

/// One monitor plus the per-monitor SSRF opt-out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorSpec {
    /// The check itself (flattened into the same JSON object).
    #[serde(flatten)]
    pub config: MonitorConfig,
    /// Allow this monitor to contact loopback, LAN and other private addresses.
    ///
    /// Default `false`: the prober blocks private ranges (SSRF guard, decision 0016). Turn it on
    /// only for targets on the owner's own machines (dogfooding); never for URLs that other
    /// people can set. The address check still runs on the address actually connected to.
    #[serde(default)]
    pub allow_private: bool,
}

/// Lowest severity that is delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum MinSeverity {
    /// Everything, including recoveries.
    #[default]
    Info,
    /// Warnings and critical.
    Warning,
    /// Only critical (down).
    Critical,
}

/// Quiet hours in minutes since local midnight, with a fixed UTC offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuietSpec {
    /// Start minute of day (0..1440).
    pub start_min: u16,
    /// End minute of day (0..1440).
    pub end_min: u16,
    /// Offset from UTC in minutes.
    #[serde(default)]
    pub utc_offset_min: i16,
    /// Critical alerts still go through.
    #[serde(default)]
    pub critical_bypasses: bool,
}

/// Channel settings, without the secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelSpec {
    /// Destination chat.
    pub chat_id: String,
    /// Lowest severity delivered.
    #[serde(default)]
    pub min_severity: MinSeverity,
    /// Optional quiet hours.
    #[serde(default)]
    pub quiet_hours: Option<QuietSpec>,
    /// Locale tag (`en`, `id`, ...).
    #[serde(default = "default_locale")]
    pub locale: String,
}

impl ChannelSpec {
    /// Converts to the notifier's config; an unknown locale is an error, never a silent fallback.
    pub fn to_channel_config(&self) -> Result<ChannelConfig, WatchError> {
        let locale = Locale::parse(&self.locale).ok_or(WatchError::Locale)?;
        Ok(ChannelConfig {
            chat_id: self.chat_id.clone(),
            min_severity: match self.min_severity {
                MinSeverity::Info => Severity::Info,
                MinSeverity::Warning => Severity::Warning,
                MinSeverity::Critical => Severity::Critical,
            },
            quiet_hours: self.quiet_hours.map(|q| QuietHours {
                start_min: q.start_min,
                end_min: q.end_min,
                utc_offset_min: q.utc_offset_min,
                critical_bypasses: q.critical_bypasses,
            }),
            locale,
        })
    }
}

/// Everything the standalone watcher needs. Contains **no secret**.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchConfig {
    /// The checks.
    pub monitors: Vec<MonitorSpec>,
    /// Where alerts go.
    pub channel: ChannelSpec,
    /// NAME of the secret that holds the bot token (for the interim source: an environment
    /// variable name). Never the token itself.
    pub token_ref: String,
    /// At most this many alerts per minute (sliding window). Default 20.
    #[serde(default = "default_throttle")]
    pub max_alerts_per_minute: u32,
    /// At most this many probes run at the same time. Default 16.
    #[serde(default = "default_in_flight")]
    pub max_in_flight: usize,
}

impl WatchConfig {
    /// Parses JSON. The error is closed: parser text could echo input.
    pub fn from_json(text: &str) -> Result<WatchConfig, WatchError> {
        serde_json::from_str(text).map_err(|_| WatchError::Parse)
    }

    /// Validates everything that needs no channel: monitors, ids, `token_ref`, locale, limits.
    pub fn validate(&self) -> Result<(), WatchError> {
        if self.monitors.len() > MAX_MONITORS {
            return Err(WatchError::TooManyMonitors);
        }
        for (index, spec) in self.monitors.iter().enumerate() {
            spec.config
                .validate()
                .map_err(|error| WatchError::Monitor { index, error })?;
            let seen_before = self
                .monitors
                .iter()
                .take(index)
                .any(|other| other.config.id == spec.config.id);
            if seen_before {
                return Err(WatchError::DuplicateId);
            }
        }
        if !valid_token_ref(&self.token_ref) {
            return Err(WatchError::TokenRef);
        }
        if self.max_alerts_per_minute == 0 || self.max_in_flight == 0 {
            return Err(WatchError::Limits);
        }
        self.channel.to_channel_config().map(|_| ())
    }
}

fn valid_token_ref(name: &str) -> bool {
    let mut chars = name.chars();
    let first_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    first_ok
        && name.len() <= MAX_TOKEN_REF_LEN
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}
