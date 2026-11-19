//! The executor: the only way a model's tool call reaches a tool.
//!
//! Order of checks (each one refuses before the next can run): tool exists, tier within the
//! ceiling, arguments valid, tool runs, output redacted, capped and fenced. The tier is read
//! from the registered tool, never from the call.

use crate::registry::Registry;
use crate::tool::{ToolContext, ToolError};
use crate::untrusted::wrap_untrusted;
use core_domain::redact::redact;
use core_domain::tier::Tier;
use serde_json::json;
use std::time::Instant;

/// Size limits of one call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Largest raw argument string accepted.
    pub max_args_bytes: usize,
    /// Largest result handed to the model (after redaction). Longer output is cut with a note
    /// so the model narrows its query instead of paging blindly.
    pub max_output_bytes: usize,
    /// Largest result a tool may produce at all; beyond it the tail is dropped before
    /// redaction runs.
    pub max_raw_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_args_bytes: 16 * 1024,
            max_output_bytes: 12 * 1024,
            max_raw_bytes: 1024 * 1024,
        }
    }
}

/// How a call ended. Codes are stable: they are stored in `tool_calls`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallStatus {
    /// The tool ran and returned a result.
    Ok,
    /// No tool has that name.
    UnknownTool,
    /// The tool is above the executor's ceiling (cannot normally happen: the registry refuses
    /// such tools; this is the second lock).
    TierDenied,
    /// The arguments failed validation; carries the rule code.
    BadArguments(&'static str),
    /// The tool ran and failed; carries the failure code.
    ToolFailed(&'static str),
}

impl CallStatus {
    /// Stable machine code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::UnknownTool => "unknown_tool",
            Self::TierDenied => "tier_denied",
            Self::BadArguments(code) | Self::ToolFailed(code) => code,
        }
    }
}

/// What is stored about a call (`tool_calls`, audit). No raw output, no secrets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallRecord {
    /// The tool name, sanitised (model-supplied text is reduced to `[a-z0-9_]`, 64 chars).
    pub tool: String,
    /// The tool's tier, when the tool exists.
    pub tier: Option<Tier>,
    /// The arguments as sent, redacted and capped at 1000 characters.
    pub args: String,
    /// How the call ended.
    pub status: CallStatus,
    /// Wall time of the whole call.
    pub duration_ms: u64,
    /// How many secrets were redacted from the result.
    pub redactions: usize,
    /// Whether the result was cut to the output limit.
    pub truncated: bool,
}

/// The result of one call: text for the model and the record for the audit trail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// What goes back to the model as the tool message.
    pub content: String,
    /// What is stored.
    pub record: CallRecord,
}

/// Runs tool calls against a [`Registry`].
pub struct Executor {
    registry: Registry,
    limits: Limits,
}

impl Executor {
    /// An executor over `registry` with default limits.
    #[must_use]
    pub fn new(registry: Registry) -> Self {
        Self {
            registry,
            limits: Limits::default(),
        }
    }

    /// Replaces the limits.
    #[must_use]
    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// The registry, for building the tool list offered to the model.
    #[must_use]
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Runs one call. Never panics and never returns an error: every refusal becomes an
    /// [`Outcome`] the model can read and the audit trail can store.
    #[must_use]
    pub fn call(&self, ctx: &ToolContext<'_>, name: &str, raw_args: &str) -> Outcome {
        let started = Instant::now();
        let args_record = cap_chars(&redact(raw_args).text, 1000);
        let finish = |status: CallStatus,
                      tier: Option<Tier>,
                      content: String,
                      redactions: usize,
                      truncated: bool| Outcome {
            content,
            record: CallRecord {
                tool: sanitise_name(name),
                tier,
                args: args_record.clone(),
                status,
                duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                redactions,
                truncated,
            },
        };

        let Some(tool) = self.registry.get(name) else {
            return finish(
                CallStatus::UnknownTool,
                None,
                error_text("unknown_tool"),
                0,
                false,
            );
        };
        let spec = tool.spec();
        if spec.tier > self.registry.ceiling() {
            let text = error_text("tier_denied");
            return finish(CallStatus::TierDenied, Some(spec.tier), text, 0, false);
        }
        let args = match spec.schema.validate(raw_args, self.limits.max_args_bytes) {
            Ok(args) => args,
            Err(error) => {
                let text = error_text(&error.to_string());
                return finish(
                    CallStatus::BadArguments(error.code()),
                    Some(spec.tier),
                    text,
                    0,
                    false,
                );
            }
        };
        match tool.run(ctx, &args) {
            Ok(output) => {
                let (text, redactions, truncated) = self.finalise(output.into_string());
                finish(
                    CallStatus::Ok,
                    Some(spec.tier),
                    wrap_untrusted(spec.name, &text),
                    redactions,
                    truncated,
                )
            }
            Err(error) => {
                let code = match error {
                    ToolError::NotFound => "not_found",
                    ToolError::Unavailable(_) => "unavailable",
                };
                // The failure text may carry server data: redact and fence it like output.
                let (text, redactions, truncated) = self.finalise(error.to_string());
                let content = wrap_untrusted(spec.name, &text);
                finish(
                    CallStatus::ToolFailed(code),
                    Some(spec.tier),
                    content,
                    redactions,
                    truncated,
                )
            }
        }
    }

    /// Redact first, cut second: cutting first could leave half of a secret that no longer
    /// matches a pattern.
    fn finalise(&self, mut text: String) -> (String, usize, bool) {
        if text.len() > self.limits.max_raw_bytes {
            cut_at_boundary(&mut text, self.limits.max_raw_bytes);
            // Drop the last (possibly half-cut) word so no secret is left as a bare prefix.
            if let Some(i) = text.rfind(char::is_whitespace) {
                text.truncate(i);
            }
        }
        let redaction = redact(&text);
        let mut text = redaction.text;
        let truncated = text.len() > self.limits.max_output_bytes;
        if truncated {
            cut_at_boundary(&mut text, self.limits.max_output_bytes);
            text.push_str("\n[output cut: narrow the query]");
        }
        (text, redaction.count, truncated)
    }
}

fn error_text(detail: &str) -> String {
    json!({ "error": detail }).to_string()
}

fn sanitise_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_')
        .take(64)
        .collect();
    if cleaned.is_empty() {
        "?".to_owned()
    } else {
        cleaned
    }
}

fn cap_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

fn cut_at_boundary(text: &mut String, max: usize) {
    if text.len() <= max {
        return;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
}
