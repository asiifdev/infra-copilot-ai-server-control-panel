//! Alert events: closed enums, numbers and sanitised names only.

use crate::Severity;

/// What happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AlertKind {
    /// A monitor went down.
    Down,
    /// A monitor is still down (repeat reminder).
    StillDown,
    /// A monitor recovered.
    Recovered,
    /// A certificate is close to expiry.
    CertExpiring,
    /// A certificate is fine again (renewed).
    CertOk,
}

impl AlertKind {
    /// Every kind (for tests and exhaustive checks).
    pub const ALL: [AlertKind; 5] = [
        AlertKind::Down,
        AlertKind::StillDown,
        AlertKind::Recovered,
        AlertKind::CertExpiring,
        AlertKind::CertOk,
    ];

    /// Default urgency of the kind.
    pub const fn severity(self) -> Severity {
        match self {
            AlertKind::Down | AlertKind::StillDown => Severity::Critical,
            AlertKind::CertExpiring => Severity::Warning,
            AlertKind::Recovered | AlertKind::CertOk => Severity::Info,
        }
    }
}

/// Failure class of a check. Closed: the probed server's own words never get in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorClass {
    /// Name resolution failed.
    Dns,
    /// Connection refused or unreachable.
    Connect,
    /// TLS handshake or certificate problem.
    Tls,
    /// No answer in time.
    Timeout,
    /// Unexpected HTTP status (see `AlertDetail::http_status`).
    HttpStatus,
    /// The page was reachable but the expected keyword was missing.
    KeywordMissing,
    /// The target resolved to an address the checker refuses to contact.
    BlockedAddress,
    /// The peer broke the protocol (malformed reply, redirect loop).
    Protocol,
    /// Anything else.
    Other,
}

/// Structured details: enums and numbers only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AlertDetail {
    /// Failure class, when there was a failure.
    pub error: Option<ErrorClass>,
    /// HTTP status that was observed.
    pub http_status: Option<u16>,
    /// Days until certificate expiry (may be negative).
    pub days_left: Option<i64>,
    /// Consecutive failed checks.
    pub repeat_count: Option<u32>,
    /// Set when the alert was held back and is sent late: when it was first raised (unix seconds).
    pub delayed_since: Option<i64>,
}

/// One alert, ready to render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertEvent {
    /// What happened.
    pub kind: AlertKind,
    /// Monitor name as the owner typed it (sanitised by [`AlertEvent::new`]).
    pub monitor_name: String,
    /// Host or URL without userinfo, query or fragment (sanitised by [`AlertEvent::new`]).
    pub target: String,
    /// When the problem started (unix seconds), if known.
    pub since: Option<i64>,
    /// End of the state: recovery time, or certificate expiry (unix seconds).
    pub until: Option<i64>,
    /// Closed-enum and numeric details.
    pub detail: AlertDetail,
}

impl AlertEvent {
    /// Builds an event, sanitising the name and target.
    pub fn new(
        kind: AlertKind,
        monitor_name: &str,
        target: &str,
        since: Option<i64>,
        until: Option<i64>,
        detail: AlertDetail,
    ) -> Self {
        AlertEvent {
            kind,
            monitor_name: sanitise_text(monitor_name, MAX_NAME_CHARS),
            target: sanitise_target(target),
            since,
            until,
            detail,
        }
    }
}

const MAX_NAME_CHARS: usize = 100;
const MAX_TARGET_CHARS: usize = 200;

fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}'
    )
}

/// Makes `s` a single line of at most `max_chars` characters: control characters become spaces,
/// invisible and bidirectional-override characters are dropped, the ends are trimmed.
pub fn sanitise_text(s: &str, max_chars: usize) -> String {
    let cleaned: String = s
        .chars()
        .filter(|c| !is_invisible(*c))
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(max_chars)
        .collect();
    cleaned.trim().to_owned()
}

/// Removes userinfo (`user:pass@`), query and fragment from a host or URL, then applies
/// [`sanitise_text`]. Idempotent.
pub fn sanitise_target(s: &str) -> String {
    let s = s.trim();
    let (scheme, rest) = match s.find("://") {
        Some(i) => (&s[..i + 3], &s[i + 3..]),
        None => ("", s),
    };
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let host = authority.rsplit('@').next().unwrap_or("");
    sanitise_text(&format!("{scheme}{host}{path}"), MAX_TARGET_CHARS)
}
