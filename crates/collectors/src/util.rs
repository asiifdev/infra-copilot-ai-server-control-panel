//! Small helpers shared by the collectors.

use core_domain::redact::{REDACTED, redact};

use crate::rootfs::FsError;

/// Longest redacted command line or command stored in an item.
pub(crate) const MAX_CMD_CHARS: usize = 512;

/// Replaces control characters with `?` and keeps at most `max` characters. Used for anything that
/// ends up inside an item key (keys may not contain control characters).
pub(crate) fn clean(text: &str, max: usize) -> String {
    text.chars()
        .map(|c| if c.is_control() { '?' } else { c })
        .take(max)
        .collect()
}

/// Keeps at most `max` characters.
pub(crate) fn truncate_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// Warning text for a failed read. Only the root-relative path is named, never file content.
pub(crate) fn read_warning(path: &str, err: &FsError) -> String {
    format!("cannot read {path}: {err}")
}

/// Name parts that make a flag look like it carries a secret (matched case-insensitively as a
/// substring of the flag name, so `--db-pass`, `--apikey` and `--passwd` all count).
const SENSITIVE_FLAG_PARTS: [&str; 8] = [
    "pass",
    "pwd",
    "secret",
    "token",
    "key",
    "auth",
    "credential",
    "cred",
];

fn is_sensitive_flag_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    SENSITIVE_FLAG_PARTS.iter().any(|part| name.contains(part))
}

/// What to hide in the argument after a flag.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Hide {
    /// Nothing pending.
    No,
    /// The next argument, unless it is a flag or a shell operator.
    NextValue,
    /// The next argument if it contains `:` (`curl -u user:password`).
    NextIfColon,
}

fn is_shell_operator(arg: &str) -> bool {
    matches!(arg, "&&" | "||" | ";" | "|" | "&" | ";;")
}

/// Joins `args` with spaces after hiding secrets, for a command line or a cron command. The result
/// is not truncated: callers truncate **after** this call, so a cut can never leave half a secret.
///
/// Arguments are separate tokens here, which plain text redaction cannot see across, so these
/// shapes are handled on top of [`redact`] (which still runs on the joined text and covers
/// `NAME=value`, `scheme://user:pass@host` and well-known token formats):
/// * `--name=value` where `name` contains a sensitive part (`pass`, `pwd`, `secret`, `token`, `key`,
///   `auth`, `cred`) becomes `--name=[REDACTED]`;
/// * such a flag without `=` hides the next argument unless that is a flag or a shell operator;
/// * `-p<value>` (value attached) becomes `-p[REDACTED]`; `-p <value>` and `-a <value>` hide the
///   next argument; `-u <x>` hides `x` when it contains `:` (`user:password`).
///
/// This deliberately over-redacts: `-p 8080` (a port) is hidden too, and so is the value of any
/// flag whose name merely contains a sensitive part (`--keyboard`). Hiding a harmless value is
/// acceptable, leaking a secret is not. A secret in a bare positional argument that matches none
/// of these shapes (`tool secret`) cannot be recognised.
pub(crate) fn redact_command_args<'a>(args: impl Iterator<Item = &'a str>) -> String {
    redact_args_at(args, 0)
}

/// How many levels of quoted command lines are looked into (`sh -c 'bash -c "..."'`).
const MAX_NESTING: usize = 3;

/// A quoted argument that holds a whole command line (`'mysql -p secret'`): its words are split
/// again and redacted like top-level arguments. Nesting beyond [`MAX_NESTING`] is left as it is
/// (the plain-text pass in [`redact`] still runs on it).
fn redact_nested(text: &str, depth: usize) -> String {
    let quote = text.chars().next().filter(|c| matches!(c, '\'' | '"'));
    let body = match quote {
        Some(q) => {
            let inner = &text[q.len_utf8()..];
            inner.strip_suffix(q).unwrap_or(inner)
        }
        None => text,
    };
    let inner = redact_args_at(split_shell_words(body).into_iter(), depth + 1);
    match quote {
        Some(q) => format!("{q}{inner}{q}"),
        None => inner,
    }
}

fn has_whitespace(text: &str) -> bool {
    text.chars().any(char::is_whitespace)
}

/// `--user`, `--proxy-user`: their value is often `name:password`.
fn is_user_flag(name: &str) -> bool {
    name == "user" || name.ends_with("-user")
}

fn redact_args_at<'a>(args: impl Iterator<Item = &'a str>, depth: usize) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut pending = Hide::No;
    for arg in args {
        match std::mem::replace(&mut pending, Hide::No) {
            Hide::NextValue if !arg.starts_with('-') && !is_shell_operator(arg) => {
                out.push(REDACTED.to_owned());
                continue;
            }
            Hide::NextIfColon if arg.contains(':') && !arg.starts_with('-') => {
                out.push(REDACTED.to_owned());
                continue;
            }
            _ => {}
        }
        if !arg.starts_with('-') && has_whitespace(arg) && depth < MAX_NESTING {
            out.push(redact_nested(arg, depth));
            continue;
        }
        if let Some(long) = arg.strip_prefix("--") {
            out.push(match long.split_once('=') {
                Some((name, value)) if is_sensitive_flag_name(name) && !value.is_empty() => {
                    format!("--{name}={REDACTED}")
                }
                Some((name, value)) if is_user_flag(name) && value.contains(':') => {
                    format!("--{name}={REDACTED}")
                }
                Some((name, value)) if has_whitespace(value) && depth < MAX_NESTING => {
                    format!("--{name}={}", redact_nested(value, depth))
                }
                Some(_) => arg.to_owned(),
                None => {
                    if is_sensitive_flag_name(long) {
                        pending = Hide::NextValue;
                    } else if is_user_flag(long) {
                        pending = Hide::NextIfColon;
                    }
                    arg.to_owned()
                }
            });
        } else if let Some(short) = arg.strip_prefix('-').filter(|s| !s.is_empty()) {
            out.push(redact_short_flag(arg, short, &mut pending));
        } else {
            out.push(arg.to_owned());
        }
    }
    redact(&out.join(" ")).text
}

/// One single-dash argument (`short` is the text after the dash).
fn redact_short_flag(arg: &str, short: &str, pending: &mut Hide) -> String {
    if let Some((name, value)) = short.split_once('=') {
        if is_sensitive_flag_name(name) && !value.is_empty() {
            return format!("-{name}={REDACTED}");
        }
        return arg.to_owned();
    }
    match short {
        "p" | "a" => *pending = Hide::NextValue,
        "u" => *pending = Hide::NextIfColon,
        _ if short.starts_with('p') => {
            // `-psecret` (attached value) or `-password x` (single-dash long name): hide both
            // the attached text and, for a sensitive name, the next argument too.
            if is_sensitive_flag_name(short) {
                *pending = Hide::NextValue;
            }
            return format!("-p{REDACTED}");
        }
        _ if short.starts_with('u') && short.contains(':') => return format!("-u{REDACTED}"),
        // `-password hunter2`, `-token x`: single-dash long names.
        _ if is_sensitive_flag_name(short) => *pending = Hide::NextValue,
        _ => {}
    }
    arg.to_owned()
}

/// Splits a shell command line into words on unquoted spaces and tabs. Quotes stay in the words
/// (so a quoted secret with spaces remains one argument for [`redact_command_args`]); an
/// unterminated quote runs to the end. Backslash escapes outside single quotes are honoured.
pub(crate) fn split_shell_words(text: &str) -> Vec<&str> {
    let mut words = Vec::new();
    let mut start: Option<usize> = None;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (index, c) in text.char_indices() {
        if start.is_none() {
            if c == ' ' || c == '\t' {
                continue;
            }
            start = Some(index);
        }
        if escaped {
            escaped = false;
            continue;
        }
        match (quote, c) {
            (None | Some('"'), '\\') => escaped = true,
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, ' ' | '\t') => {
                if let Some(begin) = start.take() {
                    words.push(&text[begin..index]);
                }
            }
            (None, _) => {}
        }
    }
    if let Some(begin) = start {
        words.push(&text[begin..]);
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str]) -> String {
        redact_command_args(args.iter().copied())
    }

    #[test]
    fn hides_every_secret_shape() {
        let cases: [(&[&str], &str); 15] = [
            (&["m", "-p", "s"], "m -p [REDACTED]"),
            (&["m", "-ps"], "m -p[REDACTED]"),
            (&["r", "-a", "s"], "r -a [REDACTED]"),
            (&["c", "-u", "u:p", "h"], "c -u [REDACTED] h"),
            (&["c", "-u", "root", "h"], "c -u root h"),
            (&["c", "-uu:p"], "c -u[REDACTED]"),
            (&["a", "--pass=x"], "a --pass=[REDACTED]"),
            (
                &["a", "--db-pass=x", "--auth=y"],
                "a --db-pass=[REDACTED] --auth=[REDACTED]",
            ),
            (
                &["a", "--password", "x", "--port", "1"],
                "a --password [REDACTED] --port 1",
            ),
            (&["a", "--token", "--next"], "a --token --next"),
            (&["a", "--token", "&&", "b"], "a --token && b"),
            (&["a", "-Dpassword=x"], "a -Dpassword=[REDACTED]"),
            (&["a", "-password", "x"], "a -p[REDACTED] [REDACTED]"),
            (&["a", "-token", "x"], "a -token [REDACTED]"),
            (&["a", "--password"], "a --password"),
        ];
        for (args, expected) in cases {
            assert_eq!(run(args), expected, "{args:?}");
        }
    }

    #[test]
    fn already_safe_shapes_stay_redacted() {
        assert_eq!(run(&["TOKEN=abc", "cmd"]), "TOKEN=[REDACTED] cmd");
        assert_eq!(run(&["c", "--password=x"]), "c --password=[REDACTED]");
        assert_eq!(
            run(&["c", "https://u:pw@h/x"]),
            "c https://u:[REDACTED]@h/x"
        );
    }

    #[test]
    fn harmless_arguments_survive() {
        assert_eq!(
            run(&["app", "--port", "8080", "--host=example.test", "-v", "file"]),
            "app --port 8080 --host=example.test -v file"
        );
    }

    #[test]
    fn shell_words_keep_quoted_groups() {
        assert_eq!(
            split_shell_words("a  \"b c\"\td 'e f'g h\\ i"),
            ["a", "\"b c\"", "d", "'e f'g", "h\\ i"]
        );
        assert_eq!(
            split_shell_words("x \"unterminated y"),
            ["x", "\"unterminated y"]
        );
        assert!(split_shell_words("   ").is_empty());
    }
}
