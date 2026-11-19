//! Send-only Telegram channel (Bot API `sendMessage`).
//!
//! M1 sends messages only. Commands, buttons and approvals come later with Proposals; the only
//! inbound piece here is the pure [`parse_update`] used to link a chat with `/start <code>`.
//!
//! The token lives in the request URL, so the [`HttpTransport`] must never log URLs. Errors
//! are the closed [`SendError`] set: the server's description text is read only to classify a
//! `400` and is never echoed.

use crate::{
    ChannelConfig, ConfigError, HttpTransport, Message, NotificationChannel, Secret, SendError,
    SendReceipt,
};
use serde_json::{Value, json};
use std::fmt;

/// Telegram's maximum message length in characters.
pub const MAX_MESSAGE_CHARS: usize = 4096;
const TITLE_BUDGET_CHARS: usize = 600;
const API_BASE: &str = "https://api.telegram.org/bot";
const MAX_RETRY_AFTER_SECS: u64 = 86_400;
const MAX_UPDATE_BYTES: usize = 64 * 1024;

/// Characters that MarkdownV2 requires to be escaped outside entities.
const SPECIAL: [char; 19] = [
    '_', '*', '[', ']', '(', ')', '~', '`', '>', '#', '+', '-', '=', '|', '{', '}', '.', '!', '\\',
];

/// Escapes every MarkdownV2 special character in `s` with a backslash. Control characters other
/// than `\n` become spaces.
pub fn escape_markdown_v2(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + s.len() / 4);
    let mut count = 0;
    push_escaped(&mut out, &mut count, s, usize::MAX);
    out
}

/// Appends the escaped form of `s` while keeping the escaped length (in chars) within `limit`.
/// Never splits an escape pair or a character. Returns false if `s` was cut.
fn push_escaped(out: &mut String, count: &mut usize, s: &str, limit: usize) -> bool {
    for c in s.chars() {
        let c = if c.is_control() && c != '\n' { ' ' } else { c };
        let width = if SPECIAL.contains(&c) { 2 } else { 1 };
        if count.saturating_add(width) > limit {
            return false;
        }
        if width == 2 {
            out.push('\\');
        }
        out.push(c);
        *count += width;
    }
    true
}

/// Builds the MarkdownV2 text: bold title, newline, body; at most [`MAX_MESSAGE_CHARS`] chars,
/// cut on a character boundary with an ellipsis.
pub fn format_text(msg: &Message) -> String {
    let mut out = String::from("*");
    let mut count = 1;
    if !push_escaped(&mut out, &mut count, &msg.title, TITLE_BUDGET_CHARS) {
        out.push('…');
        count += 1;
    }
    out.push_str("*\n");
    count += 2;
    // One char is kept for the ellipsis.
    if !push_escaped(&mut out, &mut count, &msg.body, MAX_MESSAGE_CHARS - 1) {
        out.push('…');
    }
    out
}

fn valid_token(token: &str) -> bool {
    (3..=200).contains(&token.len())
        && token.contains(':')
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'_' | b'-'))
}

fn valid_chat_id(id: &str) -> bool {
    if let Some(name) = id.strip_prefix('@') {
        return (5..=32).contains(&name.len())
            && name.starts_with(|c: char| c.is_ascii_alphabetic())
            && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    }
    let digits = id.strip_prefix('-').unwrap_or(id);
    (1..=20).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_digit())
}

/// The Telegram channel, generic over the network transport.
#[derive(Debug, Clone)]
pub struct Telegram<T: HttpTransport> {
    transport: T,
}

impl<T: HttpTransport> Telegram<T> {
    /// Wraps a transport.
    pub fn new(transport: T) -> Self {
        Telegram { transport }
    }
}

fn classify(status: u16, body: &[u8]) -> Result<SendReceipt, SendError> {
    let parsed: Option<Value> = serde_json::from_slice(body).ok();
    let ok = parsed
        .as_ref()
        .and_then(|v| v.get("ok"))
        .and_then(Value::as_bool);
    if status == 200 && ok == Some(true) {
        let message_id = parsed
            .as_ref()
            .and_then(|v| v.get("result"))
            .and_then(|r| r.get("message_id"))
            .and_then(Value::as_i64);
        return Ok(SendReceipt { message_id });
    }
    if status == 200 {
        return Err(SendError::InvalidResponse);
    }
    let code = parsed
        .as_ref()
        .and_then(|v| v.get("error_code"))
        .and_then(Value::as_u64)
        .and_then(|c| u16::try_from(c).ok())
        .unwrap_or(status);
    match code {
        429 => {
            let secs = parsed
                .as_ref()
                .and_then(|v| v.get("parameters"))
                .and_then(|p| p.get("retry_after"))
                .and_then(Value::as_u64)
                .unwrap_or(30)
                .min(MAX_RETRY_AFTER_SECS);
            Err(SendError::RateLimited {
                retry_after_secs: u32::try_from(secs).unwrap_or(u32::MAX),
            })
        }
        401 | 404 => Err(SendError::Unauthorized),
        403 => Err(SendError::Forbidden),
        400 => {
            // The description only classifies the error; it is never echoed.
            let chat_missing = parsed
                .as_ref()
                .and_then(|v| v.get("description"))
                .and_then(Value::as_str)
                .is_some_and(|d| d.to_ascii_lowercase().contains("chat not found"));
            Err(if chat_missing {
                SendError::ChatNotFound
            } else {
                SendError::BadRequest
            })
        }
        500..=599 => Err(SendError::ServerError),
        402..=499 => Err(SendError::BadRequest),
        _ => Err(SendError::InvalidResponse),
    }
}

impl<T: HttpTransport> NotificationChannel for Telegram<T> {
    fn kind(&self) -> &'static str {
        "telegram"
    }

    fn validate_config(&self, cfg: &ChannelConfig) -> Result<(), ConfigError> {
        if !valid_chat_id(&cfg.chat_id) {
            return Err(ConfigError::ChatId);
        }
        if let Some(q) = cfg.quiet_hours
            && (q.start_min >= 1440
                || q.end_min >= 1440
                || !(-1080..=1080).contains(&q.utc_offset_min))
        {
            return Err(ConfigError::QuietHours);
        }
        Ok(())
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
        let token = secret.expose_secret();
        if !valid_token(token) {
            return Err(SendError::Unauthorized);
        }
        let url = format!("{API_BASE}{token}/sendMessage");
        let body = serde_json::to_vec(&json!({
            "chat_id": cfg.chat_id,
            "text": format_text(msg),
            "parse_mode": "MarkdownV2",
            "disable_web_page_preview": true,
        }))
        .map_err(|_| SendError::BadRequest)?;
        let (status, reply) = self
            .transport
            .post_json(&url, &body)
            .await
            .map_err(SendError::Transport)?;
        classify(status, &reply)
    }
}

/// A request to link a private chat, parsed from `/start <code>`.
#[derive(Clone, PartialEq, Eq)]
pub struct LinkCommand {
    /// The private chat to link.
    pub chat_id: i64,
    /// The one-time linking code (6 to 64 chars of `A-Za-z0-9_-`).
    pub code: String,
}

impl fmt::Debug for LinkCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LinkCommand")
            .field("chat_id", &self.chat_id)
            .field("code", &"***")
            .finish()
    }
}

/// Extracts a link request from a Telegram update, or `None` for anything else: other update
/// kinds, edited messages, group or channel chats, other commands, malformed codes, malformed
/// or oversized JSON. Pure; the caller verifies the code against the stored one-time code.
pub fn parse_update(json: &[u8]) -> Option<LinkCommand> {
    if json.len() > MAX_UPDATE_BYTES {
        return None;
    }
    let v: Value = serde_json::from_slice(json).ok()?;
    let message = v.get("message")?;
    let chat = message.get("chat")?;
    if chat.get("type")?.as_str()? != "private" {
        return None;
    }
    let chat_id = chat.get("id")?.as_i64().filter(|id| *id > 0)?;
    let text = message.get("text")?.as_str()?;
    let mut parts = text.split_whitespace();
    let command = parts.next()?;
    let code = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    // `/start` or `/start@botname`.
    let (name, bot) = command.split_once('@').unwrap_or((command, ""));
    if name != "/start" || bot.len() > 64 {
        return None;
    }
    let valid = (6..=64).contains(&code.len())
        && code
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'));
    valid.then(|| LinkCommand {
        chat_id,
        code: code.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{self, FakeTransport};
    use crate::{Severity, TransportError};
    use i18n::{Catalog, Locale};

    fn cfg() -> ChannelConfig {
        ChannelConfig {
            chat_id: "-100123".into(),
            min_severity: Severity::Info,
            quiet_hours: None,
            locale: Locale::En,
        }
    }

    fn fake_token() -> String {
        format!("{}:{}", "1".repeat(9), "Z".repeat(35))
    }

    fn msg(title: &str, body: &str) -> Message {
        Message {
            severity: Severity::Critical,
            title: title.into(),
            body: body.into(),
        }
    }

    fn catalog() -> Result<Catalog, String> {
        Catalog::builtin().map_err(|e| e.to_string())
    }

    #[test]
    fn escapes_all_special_characters() {
        let all = "_*[]()~`>#+-=|{}.!\\";
        let escaped = escape_markdown_v2(all);
        assert_eq!(escaped.chars().count(), all.chars().count() * 2);
        let mut it = escaped.chars();
        while let Some(a) = it.next() {
            assert_eq!(a, '\\');
            assert!(SPECIAL.contains(&it.next().unwrap_or('x')));
        }
        assert_eq!(escape_markdown_v2("a\tb\nc"), "a b\nc");
    }

    #[test]
    fn hostile_names_never_leave_a_dangling_escape() {
        let hostile = "`[x](http://e)` \\ *_~ \n # + - = | { } . ! > \u{1F600} \u{0}";
        let text = format_text(&msg(hostile, hostile));
        // Every backslash is followed by a special character: pairs are never broken.
        let mut it = text.chars();
        while let Some(c) = it.next() {
            if c == '\\' {
                assert!(SPECIAL.contains(&it.next().unwrap_or('x')));
            }
        }
        assert!(!text.contains('\u{0}'));
    }

    #[test]
    fn huge_input_is_truncated_on_char_boundaries() {
        for unit in ["a", "\u{1F600}", ".", "\u{00E9}"] {
            let huge = unit.repeat(10_000);
            let text = format_text(&msg(&huge, &huge));
            assert!(text.chars().count() <= MAX_MESSAGE_CHARS, "unit {unit:?}");
            assert!(text.ends_with('…'));
            // Valid UTF-8 by construction; check no dangling escape at the cut.
            let body = text.trim_end_matches('…');
            let trailing = body.chars().rev().take_while(|c| *c == '\\').count();
            assert!(trailing % 2 == 0 || unit == ".");
        }
        let short = format_text(&msg("t", "b"));
        assert_eq!(short, "*t*\nb");
    }

    #[test]
    fn builds_the_expected_request() -> Result<(), String> {
        let transport = FakeTransport::default();
        let tg = Telegram::new(transport);
        let token = fake_token();
        let r = testing::block_on(tg.send(&msg("a.b", "c"), &cfg(), &Secret::new(token.clone())));
        assert_eq!(
            r,
            Ok(SendReceipt {
                message_id: Some(1)
            })
        );
        let reqs = tg.transport.requests();
        assert_eq!(reqs.len(), 1);
        assert_eq!(
            reqs[0].0,
            format!("https://api.telegram.org/bot{token}/sendMessage")
        );
        let v: Value = serde_json::from_slice(&reqs[0].1).map_err(|e| e.to_string())?;
        assert_eq!(v["chat_id"], "-100123");
        assert_eq!(v["parse_mode"], "MarkdownV2");
        assert_eq!(v["disable_web_page_preview"], true);
        assert_eq!(v["text"], "*a\\.b*\nc");
        Ok(())
    }

    fn reply(status: u16, body: &str) -> Result<SendReceipt, SendError> {
        let transport = FakeTransport::default();
        transport.push(Ok((status, body.as_bytes().to_vec())));
        let tg = Telegram::new(transport);
        testing::block_on(tg.send(&msg("t", "b"), &cfg(), &Secret::new(fake_token())))
    }

    #[test]
    fn maps_telegram_errors_without_echoing_text() {
        let planted = "SECRET-DESCRIPTION-TEXT";
        let cases = [
            (
                429,
                format!(
                    r#"{{"ok":false,"error_code":429,"description":"{planted}","parameters":{{"retry_after":7}}}}"#
                ),
                SendError::RateLimited {
                    retry_after_secs: 7,
                },
            ),
            (
                429,
                r#"{"ok":false,"error_code":429,"parameters":{"retry_after":99999999999}}"#
                    .to_owned(),
                SendError::RateLimited {
                    retry_after_secs: 86_400,
                },
            ),
            (
                401,
                format!(r#"{{"ok":false,"error_code":401,"description":"{planted}"}}"#),
                SendError::Unauthorized,
            ),
            (
                403,
                format!(r#"{{"ok":false,"error_code":403,"description":"{planted} blocked"}}"#),
                SendError::Forbidden,
            ),
            (
                400,
                r#"{"ok":false,"error_code":400,"description":"Bad Request: chat not found"}"#
                    .to_owned(),
                SendError::ChatNotFound,
            ),
            (
                400,
                format!(r#"{{"ok":false,"error_code":400,"description":"{planted}"}}"#),
                SendError::BadRequest,
            ),
            (
                502,
                "<html>gateway</html>".to_owned(),
                SendError::ServerError,
            ),
            (200, "not json".to_owned(), SendError::InvalidResponse),
            (
                200,
                r#"{"ok":false}"#.to_owned(),
                SendError::InvalidResponse,
            ),
        ];
        for (status, body, want) in cases {
            let got = reply(status, &body);
            assert_eq!(got, Err(want), "status {status}");
            let shown = format!("{got:?}{}", want);
            assert!(!shown.contains(planted));
        }
        assert_eq!(
            reply(200, r#"{"ok":true,"result":{"message_id":42}}"#),
            Ok(SendReceipt {
                message_id: Some(42)
            })
        );
    }

    #[test]
    fn transport_failure_and_bad_secret_never_leak_the_token() {
        let token = fake_token();
        let transport = FakeTransport::default();
        transport.push(Err(TransportError::Tls));
        let tg = Telegram::new(transport);
        let secret = Secret::new(token.clone());
        let err = testing::block_on(tg.send(&msg("t", "b"), &cfg(), &secret));
        assert_eq!(err, Err(SendError::Transport(TransportError::Tls)));
        for text in [
            format!("{err:?}"),
            format!("{secret:?}"),
            format!("{secret}"),
        ] {
            assert!(!text.contains(&token));
        }
        // Token-shaped but unsafe values are rejected before any request is made.
        for bad in ["", "no-colon", "1:a/../b", "1:a?x=y", "1:a b", "1:\u{e9}"] {
            let r = testing::block_on(tg.send(&msg("t", "b"), &cfg(), &Secret::new(bad)));
            assert_eq!(r, Err(SendError::Unauthorized), "{bad:?}");
            assert!(!format!("{r:?}").contains(bad) || bad.is_empty());
        }
        assert_eq!(tg.transport.requests().len(), 1);
    }

    #[test]
    fn validates_config() {
        let tg = Telegram::new(FakeTransport::default());
        for ok in ["123", "-100123456", "@my_channel"] {
            let c = ChannelConfig {
                chat_id: ok.into(),
                ..cfg()
            };
            assert_eq!(tg.validate_config(&c), Ok(()), "{ok}");
        }
        for bad in [
            "",
            "-",
            "12a",
            "@ab",
            "@1abcdef",
            "123456789012345678901",
            " 1",
        ] {
            let c = ChannelConfig {
                chat_id: bad.into(),
                ..cfg()
            };
            assert_eq!(tg.validate_config(&c), Err(ConfigError::ChatId), "{bad}");
        }
    }

    #[test]
    fn parse_update_accepts_only_private_start_with_code() {
        let ok = r#"{"update_id":1,"message":{"chat":{"id":55,"type":"private"},"text":"/start abc123XYZ"}}"#;
        let got = parse_update(ok.as_bytes());
        assert_eq!(
            got,
            Some(LinkCommand {
                chat_id: 55,
                code: "abc123XYZ".into()
            })
        );
        assert!(!format!("{got:?}").contains("abc123XYZ"));
        let with_bot =
            r#"{"message":{"chat":{"id":55,"type":"private"},"text":"/start@my_bot abc123XYZ"}}"#;
        assert!(parse_update(with_bot.as_bytes()).is_some());

        let hostile: Vec<String> = vec![
            r#"{"message":{"chat":{"id":-55,"type":"group"},"text":"/start abc123XYZ"}}"#.into(),
            r#"{"message":{"chat":{"id":-55,"type":"private"},"text":"/start abc123XYZ"}}"#.into(),
            r#"{"message":{"chat":{"id":55,"type":"supergroup"},"text":"/start abc123XYZ"}}"#
                .into(),
            r#"{"edited_message":{"chat":{"id":55,"type":"private"},"text":"/start abc123XYZ"}}"#
                .into(),
            r#"{"message":{"chat":{"id":55,"type":"private"},"text":"/start"}}"#.into(),
            r#"{"message":{"chat":{"id":55,"type":"private"},"text":"/start a"}}"#.into(),
            r#"{"message":{"chat":{"id":55,"type":"private"},"text":"/start abc123XYZ extra"}}"#
                .into(),
            r#"{"message":{"chat":{"id":55,"type":"private"},"text":"/stop abc123XYZ"}}"#.into(),
            r#"{"message":{"chat":{"id":55,"type":"private"},"text":"/start ab;c123XYZ"}}"#.into(),
            r#"{"message":{"chat":{"id":"55","type":"private"},"text":"/start abc123XYZ"}}"#.into(),
            r#"{"message":{"chat":{"id":55,"type":"private"},"text":["/start abc123XYZ"]}}"#.into(),
            r#"{"message":null}"#.into(),
            "[]".into(),
            "null".into(),
            "".into(),
            "{".into(),
            "[".repeat(50_000),
            format!(
                r#"{{"message":{{"chat":{{"id":55,"type":"private"}},"text":"/start {}"}}}}"#,
                "a".repeat(70_000)
            ),
        ];
        for h in hostile {
            assert_eq!(
                parse_update(h.as_bytes()),
                None,
                "{}",
                h.chars().take(60).collect::<String>()
            );
        }
        assert_eq!(parse_update(&[0xff, 0xfe, 0x00]), None);
    }

    #[test]
    fn passes_the_shared_contract() -> Result<(), String> {
        let tg = Telegram::new(FakeTransport::default());
        let secret = Secret::new(fake_token());
        testing::contract_suite(&tg, &cfg(), &secret, &catalog()?)
    }
}
