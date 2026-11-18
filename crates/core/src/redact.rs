//! Secret redaction.
//!
//! Applied before anything is logged, stored in the audit trail, shown in an error, or sent to an AI
//! model (CLAUDE.md rule 2). The rule is deliberately biased towards over-redaction: hiding a
//! harmless value is acceptable, leaking a secret is not.
//!
//! What it redacts:
//! * private key blocks (`-----BEGIN ... PRIVATE KEY-----` to `-----END ...-----`);
//! * the password inside a URL (`scheme://user:password@host`), including passwords that contain `@`;
//! * `Bearer <token>` values;
//! * well-known token shapes (GitHub, `sk-` API keys, Slack, AWS access key ids, Telegram bot tokens);
//! * the value of any `name=value`, `name: value` or `"name": "value"` whose name has a sensitive
//!   segment (`SECRET`, `TOKEN`, `PASSWORD`, `KEY`, ...), including camelCase names such as `apiKey`.
//!
//! Redaction is idempotent: redacting already redacted text changes nothing.

/// Replacement text for a redacted value.
pub const REDACTED: &str = "[REDACTED]";

/// Replacement text for a redacted private key block.
pub const REDACTED_PRIVATE_KEY: &str = "[REDACTED PRIVATE KEY]";

/// Result of [`redact`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redaction {
    /// The text with every detected secret replaced.
    pub text: String,
    /// How many separate secrets were replaced.
    pub count: usize,
}

/// Redacts secrets in `input`.
#[must_use]
pub fn redact(input: &str) -> Redaction {
    let mut count = 0;
    let text = redact_private_keys(input, &mut count);
    let text = redact_url_passwords(&text, &mut count);
    let text = redact_bearer(&text, &mut count);
    let text = redact_telegram_tokens(&text, &mut count);
    let text = redact_prefixed_tokens(&text, &mut count);
    let text = redact_assignments(&text, &mut count);
    Redaction { text, count }
}

const BEGIN: &str = "-----BEGIN ";
const DASHES: &str = "-----";

fn redact_private_keys(input: &str, count: &mut usize) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find(BEGIN) {
        let after = &rest[start..];
        let header_area = &after[BEGIN.len()..];
        let Some(header_len) = header_area.find(DASHES) else {
            break;
        };
        let is_private_key = header_area
            .get(..header_len)
            .is_some_and(|header| header.contains("PRIVATE KEY"));
        if !is_private_key {
            out.push_str(&rest[..start + BEGIN.len()]);
            rest = &rest[start + BEGIN.len()..];
            continue;
        }
        out.push_str(&rest[..start]);
        out.push_str(REDACTED_PRIVATE_KEY);
        *count += 1;
        rest = match after.find("-----END ") {
            Some(end) => {
                let tail = &after[end + "-----END ".len()..];
                match tail.find(DASHES) {
                    Some(close) => &tail[close + DASHES.len()..],
                    None => "",
                }
            }
            // Unterminated block: drop the remainder, it can only be key material.
            None => "",
        };
    }
    out.push_str(rest);
    out
}

fn redact_url_passwords(input: &str, count: &mut usize) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(i) = rest.find("://") {
        let authority_start = i + 3;
        out.push_str(&rest[..authority_start]);
        let tail = &rest[authority_start..];
        let authority_end = tail
            .find(|c: char| matches!(c, '/' | '?' | '#' | '"' | '\'') || c.is_whitespace())
            .unwrap_or(tail.len());
        let authority = &tail[..authority_end];
        let mut handled = false;
        if let Some(at) = authority.rfind('@') {
            let userinfo = &authority[..at];
            if let Some(colon) = userinfo.find(':') {
                let user = &userinfo[..colon];
                let password = &userinfo[colon + 1..];
                if !password.is_empty() && password != REDACTED {
                    out.push_str(user);
                    out.push(':');
                    out.push_str(REDACTED);
                    out.push_str(&authority[at..]);
                    *count += 1;
                    handled = true;
                }
            }
        }
        if !handled {
            out.push_str(authority);
        }
        rest = &tail[authority_end..];
    }
    out.push_str(rest);
    out
}

fn redact_bearer(input: &str, count: &mut usize) -> String {
    const MARKER: &str = "bearer ";
    let lower = input.to_ascii_lowercase();
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0;
    while let Some(found) = lower[cursor..].find(MARKER) {
        let token_start = cursor + found + MARKER.len();
        let token_len = input[token_start..]
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ','))
            .unwrap_or(input.len() - token_start);
        let token = &input[token_start..token_start + token_len];
        out.push_str(&input[cursor..token_start]);
        if !token.is_empty() && token != REDACTED {
            out.push_str(REDACTED);
            *count += 1;
        } else {
            out.push_str(token);
        }
        cursor = token_start + token_len;
    }
    out.push_str(&input[cursor..]);
    out
}

fn is_token_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

/// A Telegram bot token looks like `123456789:AAH...` (8-10 digits, a colon, 30-40 token characters).
fn redact_telegram_tokens(input: &str, count: &mut usize) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut last = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b':' {
            let mut ds = i;
            while ds > 0 && bytes[ds - 1].is_ascii_digit() {
                ds -= 1;
            }
            let digits = i - ds;
            let boundary_before = ds == 0 || !is_token_byte(bytes[ds - 1]);
            let mut te = i + 1;
            while te < bytes.len() && is_token_byte(bytes[te]) {
                te += 1;
            }
            let token_len = te - (i + 1);
            if boundary_before && (8..=10).contains(&digits) && (30..=40).contains(&token_len) {
                out.push_str(&input[last..ds]);
                out.push_str(REDACTED);
                *count += 1;
                last = te;
                i = te;
                continue;
            }
        }
        i += 1;
    }
    out.push_str(&input[last..]);
    out
}

fn is_secret_prefixed(word: &str) -> bool {
    let long_enough = |min: usize| word.len() >= min;
    let github = ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"]
        .iter()
        .any(|p| word.starts_with(p))
        && long_enough(24);
    let github_pat = word.starts_with("github_pat_") && long_enough(30);
    let api_key = word.starts_with("sk-") && long_enough(20);
    let slack = ["xoxb-", "xoxp-", "xoxa-", "xoxs-"]
        .iter()
        .any(|p| word.starts_with(p))
        && long_enough(20);
    let aws = word.len() == 20
        && word.starts_with("AKIA")
        && word
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit());
    github || github_pat || api_key || slack || aws
}

fn redact_prefixed_tokens(input: &str, count: &mut usize) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut last = 0;
    let mut i = 0;
    while i < bytes.len() {
        if is_token_byte(bytes[i]) {
            let start = i;
            while i < bytes.len() && is_token_byte(bytes[i]) {
                i += 1;
            }
            if is_secret_prefixed(&input[start..i]) {
                out.push_str(&input[last..start]);
                out.push_str(REDACTED);
                *count += 1;
                last = i;
            }
        } else {
            i += 1;
        }
    }
    out.push_str(&input[last..]);
    out
}

/// Splits a key into upper-case segments on non-alphanumerics and lower-to-upper camelCase boundaries.
fn key_segments(key: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut prev_lower = false;
    for c in key.chars() {
        if !c.is_ascii_alphanumeric() {
            if !current.is_empty() {
                segments.push(std::mem::take(&mut current));
            }
            prev_lower = false;
            continue;
        }
        if c.is_ascii_uppercase() && prev_lower && !current.is_empty() {
            segments.push(std::mem::take(&mut current));
        }
        current.push(c.to_ascii_uppercase());
        prev_lower = c.is_ascii_lowercase();
    }
    if !current.is_empty() {
        segments.push(current);
    }
    segments
}

fn is_sensitive_key(key: &str) -> bool {
    const SENSITIVE: [&str; 11] = [
        "SECRET",
        "TOKEN",
        "PASSWORD",
        "PASSWD",
        "PASSPHRASE",
        "CREDENTIAL",
        "CREDENTIALS",
        "KEY",
        "APIKEY",
        "PWD",
        "AUTHORIZATION",
    ];
    // `key` segments that are not secrets: database keys.
    const NOT_SECRET_KEY_PREFIX: [&str; 5] = ["PRIMARY", "FOREIGN", "SORT", "PARTITION", "CACHE"];
    let segments = key_segments(key);
    for (index, segment) in segments.iter().enumerate() {
        if !SENSITIVE.contains(&segment.as_str()) {
            continue;
        }
        if segment == "KEY" && index > 0 {
            let previous = segments[index - 1].as_str();
            if NOT_SECRET_KEY_PREFIX.contains(&previous) {
                continue;
            }
        }
        return true;
    }
    false
}

fn is_key_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')
}

fn redact_assignments(input: &str, count: &mut usize) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut last = 0;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b != b'=' && b != b':' {
            i += 1;
            continue;
        }
        if b == b':' && input[i..].starts_with("://") {
            i += 3;
            continue;
        }
        // Key: the identifier before the delimiter, allowing one closing quote (JSON style).
        let mut key_end = i;
        if key_end > 0 && matches!(bytes[key_end - 1], b'"' | b'\'') {
            key_end -= 1;
        }
        let mut key_start = key_end;
        while key_start > 0 && is_key_byte(bytes[key_start - 1]) {
            key_start -= 1;
        }
        let key = &input[key_start..key_end];
        if key.is_empty() || !is_sensitive_key(key) {
            i += 1;
            continue;
        }
        let mut value_start = i + 1;
        while value_start < bytes.len() && matches!(bytes[value_start], b' ' | b'\t') {
            value_start += 1;
        }
        let quote = bytes
            .get(value_start)
            .copied()
            .filter(|q| matches!(q, b'"' | b'\''));
        let (start, end) = if let Some(q) = quote {
            let start = value_start + 1;
            let mut end = start;
            while end < bytes.len()
                && bytes[end] != b'\n'
                && !(bytes[end] == q && bytes[end - 1] != b'\\')
            {
                end += 1;
            }
            (start, end)
        } else {
            let word_end = |from: usize| {
                let mut end = from;
                while end < bytes.len() && !matches!(bytes[end], b' ' | b'\t' | b'\n' | b'\r') {
                    end += 1;
                }
                end
            };
            let mut start = value_start;
            let mut end = word_end(start);
            // `Authorization: Bearer <token>`: the secret is the credential after the scheme word.
            let scheme = &input[start..end];
            if ["bearer", "basic", "digest", "negotiate"]
                .iter()
                .any(|s| scheme.eq_ignore_ascii_case(s))
            {
                let mut next = end;
                while next < bytes.len() && matches!(bytes[next], b' ' | b'\t') {
                    next += 1;
                }
                if next > end && next < bytes.len() && !matches!(bytes[next], b'\n' | b'\r') {
                    start = next;
                    end = word_end(next);
                }
            }
            (start, end)
        };
        if end > start && &input[start..end] != REDACTED {
            out.push_str(&input[last..start]);
            out.push_str(REDACTED);
            *count += 1;
            last = end;
        }
        i = end.max(i + 1);
    }
    out.push_str(&input[last..]);
    out
}

#[cfg(test)]
mod tests {
    use super::{REDACTED, REDACTED_PRIVATE_KEY, redact};

    fn text(input: &str) -> String {
        redact(input).text
    }

    #[test]
    fn url_password_is_hidden_but_user_and_host_stay() {
        assert_eq!(
            text("mysql://app:hunter2@127.0.0.1:3307/appdb"),
            "mysql://app:[REDACTED]@127.0.0.1:3307/appdb"
        );
    }

    #[test]
    fn url_password_containing_at_sign_is_fully_hidden() {
        let out = text("postgresql://postgres:Pass@2025@localhost:5432/db");
        assert_eq!(out, "postgresql://postgres:[REDACTED]@localhost:5432/db");
        assert!(!out.contains("2025"));
    }

    #[test]
    fn url_without_password_is_untouched() {
        let input = "https://svelte.dev/docs/ai/skills and redis://127.0.0.1:6380";
        assert_eq!(text(input), input);
        assert_eq!(redact(input).count, 0);
    }

    #[test]
    fn env_style_assignments() {
        let input =
            "DB_HOST=127.0.0.1\nDB_PASSWORD=s3cret\nAPP_KEY=base64:abcDEF123=\nREVERB_APP_KEY=zzz";
        let out = text(input);
        assert!(out.contains("DB_HOST=127.0.0.1"));
        assert!(out.contains("DB_PASSWORD=[REDACTED]"));
        assert!(out.contains("APP_KEY=[REDACTED]"));
        assert!(out.contains("REVERB_APP_KEY=[REDACTED]"));
        assert!(!out.contains("s3cret"));
        assert!(!out.contains("abcDEF123"));
    }

    #[test]
    fn json_and_yaml_style() {
        let out = text(r#"{"user":"a","password": "p@ss w0rd","apiKey":"k-123","note":"ok"}"#);
        assert!(out.contains(r#""user":"a""#));
        assert!(out.contains(r#""password": "[REDACTED]""#));
        assert!(out.contains(r#""apiKey":"[REDACTED]""#));
        assert!(out.contains(r#""note":"ok""#));
        assert_eq!(text("token: abc123"), "token: [REDACTED]");
    }

    #[test]
    fn camel_case_and_segments() {
        assert_eq!(text("accessToken=abc"), "accessToken=[REDACTED]");
        assert_eq!(text("clientSecret=abc"), "clientSecret=[REDACTED]");
        assert_eq!(text("monkey=banana"), "monkey=banana");
        assert_eq!(text("keyboard=us"), "keyboard=us");
    }

    #[test]
    fn database_keys_are_not_secrets() {
        assert_eq!(text("primary_key=id"), "primary_key=id");
        assert_eq!(text("FOREIGN_KEY=user_id"), "FOREIGN_KEY=user_id");
    }

    #[test]
    fn bearer_tokens() {
        assert_eq!(
            text("Authorization: Bearer eyJhbGciOi.abc.def"),
            "Authorization: Bearer [REDACTED]"
        );
        assert_eq!(
            text("curl -H 'bearer abc123'"),
            "curl -H 'bearer [REDACTED]'"
        );
    }

    #[test]
    fn well_known_token_shapes() {
        let github = format!("ghp_{}", "a".repeat(30));
        let openai = format!("sk-{}", "b".repeat(30));
        // Built at run time so the source file holds no key-shaped literal for secret scanners.
        let aws = format!("AKIA{}", "ABCDEFGHIJKLMNOP");
        let line = format!("tokens {github} {openai} {aws} done");
        let result = redact(&line);
        assert_eq!(
            result.text,
            format!("tokens {REDACTED} {REDACTED} {REDACTED} done")
        );
        assert_eq!(result.count, 3);
    }

    #[test]
    fn short_lookalikes_are_not_redacted() {
        assert_eq!(text("sk-short and ghp_x"), "sk-short and ghp_x");
    }

    #[test]
    fn telegram_bot_token() {
        let token = format!("123456789:{}", "A".repeat(35));
        let out = text(&format!("bot {token} ready"));
        assert_eq!(out, format!("bot {REDACTED} ready"));
        assert_eq!(text("time 12:30:45 ok"), "time 12:30:45 ok");
    }

    #[test]
    fn private_key_block() {
        // The markers are assembled at run time so the source holds no key-shaped block for secret scanners.
        let (begin, end) = ("-----BEGIN", "-----END");
        let kind = "OPENSSH PRIVATE KEY";
        let input = format!(
            "before\n{begin} {kind}-----\nFAKE-BODY-1\nFAKE-BODY-2\n{end} {kind}-----\nafter"
        );
        assert_eq!(
            text(&input),
            format!("before\n{REDACTED_PRIVATE_KEY}\nafter")
        );
    }

    #[test]
    fn unterminated_private_key_drops_the_remainder() {
        let begin = "-----BEGIN";
        let out = text(&format!("x\n{begin} RSA PRIVATE KEY-----\nFAKE-BODY..."));
        assert_eq!(out, format!("x\n{REDACTED_PRIVATE_KEY}"));
    }

    #[test]
    fn public_certificates_are_kept() {
        let input = "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----";
        assert_eq!(text(input), input);
    }

    #[test]
    fn redaction_is_idempotent() {
        let input = "DB_PASSWORD=abc mysql://u:p@h/db Authorization: Bearer tok123 apiKey=\"x\"";
        let once = text(input);
        let twice = text(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn plain_text_and_unicode_pass_through() {
        let input = "Layanan restart berhasil: nginx, php-fpm. Waktu 03:15 \u{2713}";
        assert_eq!(text(input), input);
    }

    #[test]
    fn counts_every_secret() {
        let out = redact("A_TOKEN=1 B_SECRET=2 mysql://u:p@h/db");
        assert_eq!(out.count, 3);
    }
}
