//! Provider settings. Parsed from the `params` JSON of an `ai_providers` row, strictly: an
//! unknown key is an error, so a typo cannot silently turn a safety setting off.

use crate::error::AiError;
use net::{Policy, StreamLimits};
use serde_json::Value;
use std::time::Duration;

/// How much a reasoning model should think before answering (sent only when configured).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReasoningEffort {
    /// As little as possible (fast and cheap; the default of the Muse reference setup).
    Minimal,
    /// Low.
    Low,
    /// Medium.
    Medium,
    /// High.
    High,
}

impl ReasoningEffort {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "minimal" => Some(Self::Minimal),
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            _ => None,
        }
    }
}

/// Everything about one provider that is not the key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderConfig {
    /// Base URL of the API, for example `https://api.example.test/v1`; `/chat/completions` is
    /// appended.
    pub base_url: String,
    /// Model name sent with every request.
    pub model: String,
    /// Output token cap when a request names none.
    pub max_tokens: u32,
    /// Time allowed for DNS, connect, TLS and the response head.
    pub connect_timeout: Duration,
    /// Longest silence while a response streams.
    pub stream_idle: Duration,
    /// Longest a whole response may take.
    pub total_timeout: Duration,
    /// Whether the model accepts function calling.
    pub supports_tools: bool,
    /// Whether to ask for Server-Sent Events.
    pub supports_stream: bool,
    /// Whether to send `stream_options.include_usage` (many compatible servers reject it).
    pub include_usage_in_stream: bool,
    /// Reasoning effort to request, if the model has the knob.
    pub reasoning_effort: Option<ReasoningEffort>,
    /// Allow `http://` and private or loopback addresses. Only for a model you run yourself.
    pub allow_private_network: bool,
    /// Retries after the first attempt for transient failures.
    pub max_retries: u8,
    /// Consecutive failures that open the circuit breaker.
    pub breaker_failures: u32,
    /// How long the breaker stays open.
    pub breaker_cooldown: Duration,
}

impl ProviderConfig {
    /// A validated configuration with the default limits.
    ///
    /// # Errors
    /// [`AiError::Config`] for a bad URL or model name.
    pub fn new(base_url: &str, model: &str) -> Result<Self, AiError> {
        let config = Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            model: model.to_owned(),
            max_tokens: 1024,
            connect_timeout: Duration::from_secs(30),
            stream_idle: Duration::from_secs(60),
            total_timeout: Duration::from_secs(300),
            supports_tools: true,
            supports_stream: true,
            include_usage_in_stream: false,
            reasoning_effort: None,
            allow_private_network: false,
            max_retries: 2,
            breaker_failures: 5,
            breaker_cooldown: Duration::from_secs(60),
        };
        config.validate()?;
        Ok(config)
    }

    /// Builds the configuration from a stored row: base URL, model and the `params` JSON.
    ///
    /// Keys: `max_tokens`, `supports_tools`, `supports_stream`, `include_usage_in_stream`,
    /// `reasoning_effort`, `allow_private_network`, `connect_timeout_secs`, `idle_timeout_secs`,
    /// `total_timeout_secs`, `max_retries`.
    ///
    /// # Errors
    /// [`AiError::Config`] for invalid JSON, an unknown key, a wrong type or a value out of range.
    pub fn from_params(base_url: &str, model: &str, params_json: &str) -> Result<Self, AiError> {
        let mut config = Self::new_unchecked(base_url, model);
        let Ok(Value::Object(params)) = serde_json::from_str::<Value>(params_json) else {
            return Err(AiError::Config("params must be a JSON object"));
        };
        for (key, value) in &params {
            let bool_of = || {
                value
                    .as_bool()
                    .ok_or(AiError::Config("param must be a boolean"))
            };
            let secs_of = |min: u64, max: u64| {
                value
                    .as_u64()
                    .filter(|n| (min..=max).contains(n))
                    .map(Duration::from_secs)
                    .ok_or(AiError::Config("timeout out of range"))
            };
            match key.as_str() {
                "max_tokens" => {
                    config.max_tokens = value
                        .as_u64()
                        .and_then(|n| u32::try_from(n).ok())
                        .filter(|n| (1..=200_000).contains(n))
                        .ok_or(AiError::Config("max_tokens out of range"))?;
                }
                "supports_tools" => config.supports_tools = bool_of()?,
                "supports_stream" => config.supports_stream = bool_of()?,
                "include_usage_in_stream" => config.include_usage_in_stream = bool_of()?,
                "allow_private_network" => config.allow_private_network = bool_of()?,
                "reasoning_effort" => {
                    config.reasoning_effort = Some(
                        value
                            .as_str()
                            .and_then(ReasoningEffort::parse)
                            .ok_or(AiError::Config("unknown reasoning_effort"))?,
                    );
                }
                "connect_timeout_secs" => config.connect_timeout = secs_of(1, 300)?,
                "idle_timeout_secs" => config.stream_idle = secs_of(1, 600)?,
                "total_timeout_secs" => config.total_timeout = secs_of(1, 3600)?,
                "max_retries" => {
                    config.max_retries = value
                        .as_u64()
                        .and_then(|n| u8::try_from(n).ok())
                        .filter(|n| *n <= 5)
                        .ok_or(AiError::Config("max_retries out of range"))?;
                }
                _ => return Err(AiError::Config("unknown param")),
            }
        }
        config.validate()?;
        Ok(config)
    }

    fn new_unchecked(base_url: &str, model: &str) -> Self {
        Self::new("https://placeholder.invalid", "m").map_or_else(
            |_| unreachable!("constant configuration is valid"),
            |mut c| {
                c.base_url = base_url.trim_end_matches('/').to_owned();
                c.model = model.to_owned();
                c
            },
        )
    }

    fn validate(&self) -> Result<(), AiError> {
        let plain = self.base_url.strip_prefix("http://");
        let secure = self.base_url.strip_prefix("https://");
        let rest = match (secure, plain) {
            (Some(rest), _) => rest,
            (None, Some(rest)) if self.allow_private_network => rest,
            _ => return Err(AiError::Config("base_url must be https")),
        };
        let authority = rest.split('/').next().unwrap_or_default();
        let bad = rest.is_empty()
            || authority.is_empty()
            || authority.contains('@')
            || self.base_url.len() > 500
            || self
                .base_url
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || c == '?' || c == '#');
        if bad {
            return Err(AiError::Config("base_url is malformed"));
        }
        if self.model.is_empty()
            || self.model.len() > 200
            || self.model.chars().any(char::is_control)
        {
            return Err(AiError::Config("model name is malformed"));
        }
        Ok(())
    }

    pub(crate) fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    /// The network policy this provider implies: strict unless `allow_private_network`.
    #[must_use]
    pub fn net_policy(&self) -> Policy {
        Policy {
            timeout: self.connect_timeout,
            block_private_addresses: !self.allow_private_network,
            allow_http: self.allow_private_network,
            ..Policy::default()
        }
    }

    /// The stream limits this provider implies.
    #[must_use]
    pub fn stream_limits(&self) -> StreamLimits {
        StreamLimits {
            idle: self.stream_idle,
            total: self.total_timeout,
            max_bytes: 8 * 1024 * 1024,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_http_and_private_networks_need_an_explicit_switch() {
        assert!(ProviderConfig::new("https://api.example.test/v1", "m").is_ok());
        assert!(ProviderConfig::new("http://localhost:11434/v1", "m").is_err());
        let local = ProviderConfig::from_params(
            "http://localhost:11434/v1",
            "m",
            r#"{"allow_private_network":true}"#,
        );
        assert!(local.is_ok_and(|c| !c.net_policy().block_private_addresses && c.net_policy().allow_http));
        let strict = ProviderConfig::new("https://api.example.test/v1/", "m");
        assert!(strict.is_ok_and(|c| c.endpoint()
            == "https://api.example.test/v1/chat/completions"
            && c.net_policy().block_private_addresses
            && !c.net_policy().allow_http));
    }

    #[test]
    fn malformed_urls_and_models_are_refused() {
        for url in [
            "",
            "ftp://x/v1",
            "https://",
            "https:///v1",
            "https://u:p@host/v1",
            "https://h/v1?x=1",
            "https://h/v 1",
            "https://h/v1#f",
            "api.example.test",
        ] {
            assert!(ProviderConfig::new(url, "m").is_err(), "{url:?}");
        }
        for model in ["", "a\nb"] {
            assert!(
                ProviderConfig::new("https://h/v1", model).is_err(),
                "{model:?}"
            );
        }
        assert!(ProviderConfig::new("https://h/v1", &"x".repeat(201)).is_err());
    }

    #[test]
    fn params_are_strict() {
        let ok = ProviderConfig::from_params(
            "https://h/v1",
            "m",
            r#"{"max_tokens":2048,"supports_tools":false,"reasoning_effort":"minimal","idle_timeout_secs":20,"max_retries":1}"#,
        );
        assert!(ok.is_ok_and(|c| c.max_tokens == 2048
            && !c.supports_tools
            && c.reasoning_effort == Some(ReasoningEffort::Minimal)
            && c.stream_idle == Duration::from_secs(20)
            && c.max_retries == 1));
        for bad in [
            "not json",
            "[]",
            r#"{"typo_param":true}"#,
            r#"{"max_tokens":0}"#,
            r#"{"max_tokens":"9"}"#,
            r#"{"supports_tools":"yes"}"#,
            r#"{"reasoning_effort":"max"}"#,
            r#"{"idle_timeout_secs":0}"#,
            r#"{"max_retries":9}"#,
        ] {
            assert!(
                ProviderConfig::from_params("https://h/v1", "m", bad).is_err(),
                "{bad}"
            );
        }
    }
}
