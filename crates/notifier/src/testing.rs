//! Shared test support: the contract suite every channel must pass, plus fakes.
//!
//! Plain `pub` on purpose (no feature flag): other crates run `contract_suite` on their own
//! channel in their tests.

use crate::{
    AlertDetail, AlertEvent, AlertKind, ChannelConfig, ConfigError, HttpTransport, Message,
    NotificationChannel, QuietHours, Secret, SendError, SendReceipt, Severity, TransportError,
    render,
};
use i18n::{Catalog, Locale};
use std::collections::VecDeque;
use std::future::Future;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, Waker};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Runs a future that completes without a reactor (fakes and pure code) to completion.
pub fn block_on<F: Future>(fut: F) -> F::Output {
    let mut fut = std::pin::pin!(fut);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(out) = fut.as_mut().poll(&mut cx) {
            return out;
        }
        std::thread::yield_now();
    }
}

type TransportReply = Result<(u16, Vec<u8>), TransportError>;

/// A transport that records requests and replays queued replies (default: Telegram success).
#[derive(Debug, Default)]
pub struct FakeTransport {
    requests: Mutex<Vec<(String, Vec<u8>)>>,
    replies: Mutex<VecDeque<TransportReply>>,
}

impl FakeTransport {
    /// Queues the next reply.
    pub fn push(&self, reply: TransportReply) {
        lock(&self.replies).push_back(reply);
    }

    /// Every `(url, body)` posted so far.
    pub fn requests(&self) -> Vec<(String, Vec<u8>)> {
        lock(&self.requests).clone()
    }
}

impl HttpTransport for FakeTransport {
    async fn post_json(&self, url: &str, body: &[u8]) -> Result<(u16, Vec<u8>), TransportError> {
        lock(&self.requests).push((url.to_owned(), body.to_vec()));
        match lock(&self.replies).pop_front() {
            Some(reply) => reply,
            None => Ok((200, br#"{"ok":true,"result":{"message_id":1}}"#.to_vec())),
        }
    }
}

/// An in-memory channel that records what it was asked to send.
#[derive(Debug, Default)]
pub struct FakeChannel {
    sent: Mutex<Vec<Message>>,
}

impl FakeChannel {
    /// Messages delivered so far.
    pub fn sent(&self) -> Vec<Message> {
        lock(&self.sent).clone()
    }
}

impl NotificationChannel for FakeChannel {
    fn kind(&self) -> &'static str {
        "fake"
    }

    fn validate_config(&self, cfg: &ChannelConfig) -> Result<(), ConfigError> {
        if cfg.chat_id.is_empty() {
            return Err(ConfigError::ChatId);
        }
        match cfg.quiet_hours {
            Some(q) if q.start_min >= 1440 || q.end_min >= 1440 => Err(ConfigError::QuietHours),
            _ => Ok(()),
        }
    }

    async fn send(
        &self,
        msg: &Message,
        cfg: &ChannelConfig,
        secret: &Secret,
    ) -> Result<SendReceipt, SendError> {
        if self.validate_config(cfg).is_err() {
            return Err(SendError::BadRequest);
        }
        if secret.expose_secret().is_empty() {
            return Err(SendError::Unauthorized);
        }
        let mut sent = lock(&self.sent);
        sent.push(msg.clone());
        Ok(SendReceipt {
            message_id: i64::try_from(sent.len()).ok(),
        })
    }
}

fn check(cond: bool, what: &str) -> Result<(), String> {
    if cond { Ok(()) } else { Err(what.to_owned()) }
}

/// The shared contract for every [`NotificationChannel`]. `cfg` and `secret` must be a valid
/// setup for the channel under test whose sends succeed. Returns the first violated rule.
///
/// Rules: a lowercase `kind`; `validate_config` accepts `cfg` and rejects an empty chat id and
/// out-of-range quiet hours; every alert kind renders and sends in every locale; the test
/// message sends; hostile and huge messages do not panic and either send or fail with a closed
/// error; an empty secret is refused; no `Debug` or `Display` output of any result contains the
/// secret.
pub fn contract_suite<C: NotificationChannel>(
    channel: &C,
    cfg: &ChannelConfig,
    secret: &Secret,
    catalog: &Catalog,
) -> Result<(), String> {
    let kind = channel.kind();
    check(
        !kind.is_empty() && kind.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'),
        "kind is lowercase ascii",
    )?;
    check(
        channel.validate_config(cfg).is_ok(),
        "valid config accepted",
    )?;
    let empty_chat = ChannelConfig {
        chat_id: String::new(),
        ..cfg.clone()
    };
    check(
        channel.validate_config(&empty_chat) == Err(ConfigError::ChatId),
        "empty chat id rejected",
    )?;
    let bad_quiet = ChannelConfig {
        quiet_hours: Some(QuietHours {
            start_min: 5_000,
            end_min: 0,
            utc_offset_min: 0,
            critical_bypasses: false,
        }),
        ..cfg.clone()
    };
    check(
        channel.validate_config(&bad_quiet) == Err(ConfigError::QuietHours),
        "out-of-range quiet hours rejected",
    )?;

    let mut outputs = Vec::new();
    for locale in Locale::ALL {
        let loc_cfg = ChannelConfig {
            locale,
            ..cfg.clone()
        };
        for kind in AlertKind::ALL {
            let event = AlertEvent::new(
                kind,
                "web `[1]` \\ *",
                "https://user:pw@example.test/path?token=x",
                Some(1_700_000_000),
                Some(1_700_003_600),
                AlertDetail::default(),
            );
            let msg = render(&event, locale, catalog);
            let r = block_on(channel.send(&msg, &loc_cfg, secret));
            check(r.is_ok(), "every alert kind sends in every locale")?;
            outputs.push(format!("{r:?}"));
        }
        let r = block_on(channel.test(&loc_cfg, secret, catalog));
        check(r.is_ok(), "test message sends")?;
        outputs.push(format!("{r:?}"));
    }

    let hostile = Message {
        severity: Severity::Critical,
        title: "`*_[](){}~>#+-=|.!\\\n".repeat(1_000),
        body: "\u{1F600}\n".repeat(10_000),
    };
    let r = block_on(channel.send(&hostile, cfg, secret));
    outputs.push(format!("{r:?}"));
    if let Err(e) = r {
        outputs.push(format!("{e} {e:?}"));
    }

    let r = block_on(channel.send(&hostile, cfg, &Secret::new("")));
    check(r.is_err(), "empty secret refused")?;

    let secret_text = secret.expose_secret();
    check(
        outputs.iter().all(|o| !o.contains(secret_text)),
        "secret never appears in results or errors",
    )?;
    check(
        !format!("{secret:?}{secret}").contains(secret_text),
        "secret Debug and Display are masked",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> ChannelConfig {
        ChannelConfig {
            chat_id: "1".into(),
            min_severity: Severity::Info,
            quiet_hours: None,
            locale: Locale::Id,
        }
    }

    #[test]
    fn fake_channel_passes_the_contract() -> Result<(), String> {
        let catalog = Catalog::builtin().map_err(|e| e.to_string())?;
        let ch = FakeChannel::default();
        contract_suite(&ch, &cfg(), &Secret::new("fake-secret-value"), &catalog)?;
        check(!ch.sent().is_empty(), "fake recorded messages")
    }

    #[test]
    fn contract_catches_a_noncompliant_channel() -> Result<(), String> {
        struct Leaky;
        impl NotificationChannel for Leaky {
            fn kind(&self) -> &'static str {
                "leaky"
            }
            fn validate_config(&self, _: &ChannelConfig) -> Result<(), ConfigError> {
                Ok(())
            }
            async fn send(
                &self,
                _: &Message,
                _: &ChannelConfig,
                _: &Secret,
            ) -> Result<SendReceipt, SendError> {
                Ok(SendReceipt::default())
            }
        }
        let catalog = Catalog::builtin().map_err(|e| e.to_string())?;
        let r = contract_suite(&Leaky, &cfg(), &Secret::new("s"), &catalog);
        check(
            r.is_err(),
            "a channel that accepts bad config fails the suite",
        )
    }
}
