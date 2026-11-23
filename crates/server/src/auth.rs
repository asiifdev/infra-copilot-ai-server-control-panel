//! One-time sign-in tokens and sessions (ADR 0029, 0030).
//!
//! A sign-in token is 256 random bits, valid once for ten minutes, and kept only as a SHA-256
//! hash. Exchanging it creates a session: 256 random bits, kept only as a hash in memory (a
//! restart ends every session), expiring after eight hours without use, at most five at a time
//! (the least recently used one is dropped for a sixth). Failed exchanges are counted: after five
//! failures every pending token is destroyed, and more than ten attempts in a minute are refused
//! outright, so guessing a 256-bit token is hopeless and also noisy.

use ring::rand::{SecureRandom, SystemRandom};
use std::collections::VecDeque;

/// Name of the session cookie.
pub(crate) const COOKIE: &str = "nv_session";
/// How long a sign-in token lives.
const TOKEN_TTL_SECS: u64 = 10 * 60;
/// Idle time after which a session ends.
const SESSION_IDLE_SECS: u64 = 8 * 60 * 60;
/// Sessions kept at once.
const MAX_SESSIONS: usize = 5;
/// Tokens that may be pending at once.
const MAX_TOKENS: usize = 5;
/// Failed exchanges after which all pending tokens are destroyed.
const MAX_FAILURES: u32 = 5;
/// The window of the attempt limit, in seconds.
const ATTEMPT_WINDOW_SECS: u64 = 60;
/// Attempts allowed inside the window.
const MAX_ATTEMPTS: usize = 10;

struct PendingToken {
    hash: String,
    expires: u64,
}

struct Session {
    hash: String,
    last_seen: u64,
}

/// Why an exchange failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExchangeError {
    /// Unknown, expired or already used.
    Invalid,
    /// Too many attempts; try again later.
    RateLimited,
    /// The system random source failed.
    Random,
}

/// The state of sign-in: pending tokens and live sessions.
#[derive(Default)]
pub(crate) struct Auth {
    tokens: Vec<PendingToken>,
    sessions: Vec<Session>,
    attempts: VecDeque<u64>,
    failures: u32,
}

/// 256 random bits as 64 lowercase hex characters. `None` when the system random source fails.
fn random_hex() -> Option<String> {
    let mut bytes = [0_u8; 32];
    SystemRandom::new().fill(&mut bytes).ok()?;
    let mut out = String::with_capacity(64);
    for byte in bytes {
        out.push(char::from(b"0123456789abcdef"[usize::from(byte >> 4)]));
        out.push(char::from(b"0123456789abcdef"[usize::from(byte & 0x0f)]));
    }
    Some(out)
}

fn hash(secret: &str) -> String {
    db::sha256_hex(secret.as_bytes())
}

/// Compares two strings without stopping at the first difference. Both are hashes of the same
/// length here; a different length is simply unequal.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0_u8, |diff, (x, y)| diff | (x ^ y))
            == 0
}

impl Auth {
    /// Makes a sign-in token. The plaintext is returned once and never kept. `None` when the
    /// random source fails.
    pub(crate) fn issue_token(&mut self, now: u64) -> Option<String> {
        self.tokens.retain(|t| t.expires > now);
        let token = random_hex()?;
        if self.tokens.len() >= MAX_TOKENS {
            self.tokens.remove(0);
        }
        self.tokens.push(PendingToken {
            hash: hash(&token),
            expires: now.saturating_add(TOKEN_TTL_SECS),
        });
        Some(token)
    }

    /// Trades a token for a session id (the cookie value).
    pub(crate) fn exchange(&mut self, token: &str, now: u64) -> Result<String, ExchangeError> {
        while self
            .attempts
            .front()
            .is_some_and(|t| now.saturating_sub(*t) >= ATTEMPT_WINDOW_SECS)
        {
            self.attempts.pop_front();
        }
        if self.attempts.len() >= MAX_ATTEMPTS {
            return Err(ExchangeError::RateLimited);
        }
        self.attempts.push_back(now);
        let wanted = hash(token);
        // Every pending token is compared, so timing does not say which one matched.
        let mut found = None;
        for (index, pending) in self.tokens.iter().enumerate() {
            if same(&wanted, &pending.hash) && pending.expires > now {
                found = Some(index);
            }
        }
        let Some(index) = found else {
            self.failures = self.failures.saturating_add(1);
            if self.failures >= MAX_FAILURES {
                self.tokens.clear();
            }
            return Err(ExchangeError::Invalid);
        };
        // Single use: gone before anything else can happen.
        self.tokens.remove(index);
        let session = random_hex().ok_or(ExchangeError::Random)?;
        self.failures = 0;
        self.sessions
            .retain(|s| now.saturating_sub(s.last_seen) < SESSION_IDLE_SECS);
        if self.sessions.len() >= MAX_SESSIONS
            && let Some(oldest) = self
                .sessions
                .iter()
                .enumerate()
                .min_by_key(|(_, s)| s.last_seen)
                .map(|(i, _)| i)
        {
            self.sessions.remove(oldest);
        }
        self.sessions.push(Session {
            hash: hash(&session),
            last_seen: now,
        });
        Ok(session)
    }

    /// Whether `cookie` names a live session; a hit counts as activity.
    pub(crate) fn check(&mut self, cookie: &str, now: u64) -> bool {
        // Only well-formed values are looked up: 64 lowercase hex characters.
        if cookie.len() != 64
            || !cookie
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        {
            return false;
        }
        let wanted = hash(cookie);
        self.sessions
            .retain(|s| now.saturating_sub(s.last_seen) < SESSION_IDLE_SECS);
        let mut hit = false;
        for session in &mut self.sessions {
            if same(&wanted, &session.hash) {
                session.last_seen = now;
                hit = true;
            }
        }
        hit
    }

    /// Ends the session named by `cookie` (nothing happens for an unknown value).
    pub(crate) fn logout(&mut self, cookie: &str) {
        let wanted = hash(cookie);
        self.sessions.retain(|s| !same(&wanted, &s.hash));
    }

    /// Live sessions (tests).
    #[cfg(test)]
    pub(crate) fn session_count(&self) -> usize {
        self.sessions.len()
    }
}

/// The value of the session cookie in a `Cookie` header value, if present.
pub(crate) fn session_cookie(header: &str) -> Option<&str> {
    header.split(';').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name.trim() == COOKIE).then(|| value.trim())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_works_once() {
        let mut auth = Auth::default();
        let token = auth.issue_token(0).unwrap_or_default();
        assert!(auth.exchange(&token, 1).is_ok());
        assert_eq!(auth.exchange(&token, 2), Err(ExchangeError::Invalid));
    }

    #[test]
    fn a_token_expires_after_ten_minutes() {
        let mut auth = Auth::default();
        let token = auth.issue_token(0).unwrap_or_default();
        assert_eq!(auth.exchange(&token, 600), Err(ExchangeError::Invalid));
        let fresh = auth.issue_token(1000).unwrap_or_default();
        assert!(auth.exchange(&fresh, 1599).is_ok());
    }

    #[test]
    fn repeated_failures_destroy_the_token() {
        let mut auth = Auth::default();
        let token = auth.issue_token(0).unwrap_or_default();
        for _ in 0..5 {
            assert_eq!(auth.exchange("nope", 1), Err(ExchangeError::Invalid));
        }
        assert_eq!(auth.exchange(&token, 2), Err(ExchangeError::Invalid));
    }

    #[test]
    fn attempts_are_limited_per_minute() {
        let mut auth = Auth::default();
        let mut last = Ok(String::new());
        for _ in 0..11 {
            last = auth.exchange("nope", 5);
        }
        assert_eq!(last, Err(ExchangeError::RateLimited));
        // After the window the counter is free again.
        assert_eq!(auth.exchange("nope", 70), Err(ExchangeError::Invalid));
    }

    #[test]
    fn sessions_expire_when_idle_and_activity_keeps_them() {
        let mut auth = Auth::default();
        let token = auth.issue_token(0).unwrap_or_default();
        let session = auth.exchange(&token, 0).unwrap_or_default();
        assert!(auth.check(&session, SESSION_IDLE_SECS - 1));
        assert!(auth.check(&session, 2 * SESSION_IDLE_SECS - 2));
        assert!(!auth.check(&session, 4 * SESSION_IDLE_SECS));
    }

    #[test]
    fn the_session_cap_drops_the_least_recently_used() {
        let mut auth = Auth::default();
        let mut sessions = Vec::new();
        for i in 0..6_u64 {
            let token = auth.issue_token(i).unwrap_or_default();
            sessions.push(auth.exchange(&token, i).unwrap_or_default());
        }
        assert_eq!(auth.session_count(), MAX_SESSIONS);
        assert!(!auth.check(&sessions[0], 10));
        assert!(auth.check(&sessions[5], 10));
    }

    #[test]
    fn malformed_cookies_never_match() {
        let mut auth = Auth::default();
        assert!(!auth.check("", 0));
        assert!(!auth.check(&"A".repeat(64), 0));
        assert!(!auth.check("../../etc", 0));
    }

    #[test]
    fn the_cookie_is_found_among_others() {
        assert_eq!(session_cookie("a=1; nv_session=abc; b=2"), Some("abc"));
        assert_eq!(session_cookie("nv_session_x=abc"), None);
    }
}
