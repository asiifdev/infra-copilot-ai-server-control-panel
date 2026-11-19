//! Failures mapped to advice (migration-planning skill: show the failure as it is, with a
//! suggestion, not a raw error). The tool's output is untrusted text: it is only matched against
//! fixed phrases, and the short detail kept for support is redacted and stripped of control
//! characters.

use crate::runner::{RunError, RunOutput};
use core_domain::redact::redact;
use i18n::{Catalog, Locale, MessageKey};

/// What went wrong, as a closed set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProblemKind {
    /// The tool is not installed. Nodivara offers to install it and never does so by itself.
    NotInstalled,
    /// No room left, or a transfer quota was hit.
    QuotaExceeded,
    /// The OAuth token expired or was revoked.
    TokenExpired,
    /// The remote or the path does not exist, or the name is wrong.
    WrongRemote,
    /// Name resolution, connection or TLS failed.
    Network,
    /// The command did not finish in time.
    Timeout,
    /// The settings did not pass validation.
    InvalidConfig,
    /// Anything else.
    Unknown,
}

impl ProblemKind {
    /// Stable machine code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::NotInstalled => "not_installed",
            Self::QuotaExceeded => "quota_exceeded",
            Self::TokenExpired => "token_expired",
            Self::WrongRemote => "wrong_remote",
            Self::Network => "network",
            Self::Timeout => "timeout",
            Self::InvalidConfig => "invalid_config",
            Self::Unknown => "unknown",
        }
    }

    /// Catalog key of the headline.
    #[must_use]
    pub const fn title_key(self) -> MessageKey {
        MessageKey::new(match self {
            Self::NotInstalled => "transport.not_installed.title",
            Self::QuotaExceeded => "transport.quota_exceeded.title",
            Self::TokenExpired => "transport.token_expired.title",
            Self::WrongRemote => "transport.wrong_remote.title",
            Self::Network => "transport.network.title",
            Self::Timeout => "transport.timeout.title",
            Self::InvalidConfig => "transport.invalid_config.title",
            Self::Unknown => "transport.unknown.title",
        })
    }

    /// Catalog key of the suggestion.
    #[must_use]
    pub const fn body_key(self) -> MessageKey {
        MessageKey::new(match self {
            Self::NotInstalled => "transport.not_installed.body",
            Self::QuotaExceeded => "transport.quota_exceeded.body",
            Self::TokenExpired => "transport.token_expired.body",
            Self::WrongRemote => "transport.wrong_remote.body",
            Self::Network => "transport.network.body",
            Self::Timeout => "transport.timeout.body",
            Self::InvalidConfig => "transport.invalid_config.body",
            Self::Unknown => "transport.unknown.body",
        })
    }

    /// Every kind, for tests that check the catalog covers them all.
    pub const ALL: [Self; 8] = [
        Self::NotInstalled,
        Self::QuotaExceeded,
        Self::TokenExpired,
        Self::WrongRemote,
        Self::Network,
        Self::Timeout,
        Self::InvalidConfig,
        Self::Unknown,
    ];
}

/// A classified failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advice {
    /// The class.
    pub problem: ProblemKind,
    /// A short excerpt of the tool's own message, redacted, one line, for support. Untrusted
    /// data: show it as quoted text, never act on it.
    pub detail: String,
}

impl Advice {
    /// A failure without tool output.
    #[must_use]
    pub const fn of(problem: ProblemKind) -> Self {
        Self {
            problem,
            detail: String::new(),
        }
    }

    /// Whether the product should offer to install the tool (never installs by itself).
    #[must_use]
    pub const fn offer_install(&self) -> bool {
        matches!(self.problem, ProblemKind::NotInstalled)
    }

    /// Headline and suggestion in the person's language.
    #[must_use]
    pub fn render(&self, catalog: &Catalog, locale: Locale) -> (String, String) {
        (
            catalog.get(locale, self.problem.title_key(), &[]).text,
            catalog.get(locale, self.problem.body_key(), &[]).text,
        )
    }
}

const DETAIL_MAX: usize = 300;
const MATCH_WINDOW: usize = 16 * 1024;

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
}

/// A short, safe excerpt of the tool's stderr: known secret values and token shapes removed,
/// control characters turned into spaces, cut at 300 characters.
fn detail(stderr: &str, secrets: &[&str]) -> String {
    let mut text = stderr.to_owned();
    for secret in secrets.iter().filter(|s| !s.is_empty()) {
        text = text.replace(secret, "[REDACTED]");
    }
    let text = redact(&text).text;
    let one_line: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    one_line.trim().chars().take(DETAIL_MAX).collect()
}

/// Classifies a finished process that did not succeed. `secrets` are the credential values in
/// use, scrubbed from the detail even if the tool echoed them.
#[must_use]
pub fn classify_output(output: &RunOutput, secrets: &[&str]) -> Advice {
    let raw = String::from_utf8_lossy(&output.stderr);
    let window: String = raw.chars().take(MATCH_WINDOW).collect();
    let lower = window.to_lowercase();
    let problem = if contains_any(
        &lower,
        &[
            "invalid_grant",
            "token expired",
            "token has expired",
            "token has been expired",
            "expired or revoked",
            "failed to refresh token",
            "couldn't fetch token",
            "401 unauthorized",
            "invalid_client",
        ],
    ) {
        ProblemKind::TokenExpired
    } else if output.exit_code == Some(8)
        || contains_any(
            &lower,
            &[
                "quota",
                "storagequotaexceeded",
                "insufficient storage",
                "insufficientstorage",
                "no space left",
                "usage limit",
            ],
        )
    {
        ProblemKind::QuotaExceeded
    } else if output.exit_code == Some(3)
        || contains_any(
            &lower,
            &[
                "didn't find section",
                "unknown remote",
                "nosuchbucket",
                "bucket does not exist",
                "directory not found",
                "couldn't find root",
                "unknown backend",
            ],
        )
    {
        ProblemKind::WrongRemote
    } else if contains_any(
        &lower,
        &[
            "no such host",
            "connection refused",
            "network is unreachable",
            "i/o timeout",
            "tls handshake",
            "could not resolve",
            "connection timed out",
            "host key verification failed",
        ],
    ) {
        ProblemKind::Network
    } else {
        ProblemKind::Unknown
    };
    Advice {
        problem,
        detail: detail(&window, secrets),
    }
}

/// Classifies a process that could not be run at all.
#[must_use]
pub const fn classify_run_error(error: RunError) -> Advice {
    Advice::of(match error {
        RunError::NotInstalled => ProblemKind::NotInstalled,
        RunError::Timeout => ProblemKind::Timeout,
        RunError::Io => ProblemKind::Unknown,
    })
}

/// Whether the tool's stderr says the remote cannot report quota (`rclone about` is an optional
/// backend feature).
#[must_use]
pub fn about_unsupported(output: &RunOutput) -> bool {
    let lower = String::from_utf8_lossy(&output.stderr).to_lowercase();
    contains_any(
        &lower,
        &[
            "doesn't support about",
            "does not support about",
            "about not supported",
        ],
    )
}
