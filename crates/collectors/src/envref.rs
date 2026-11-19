//! Configuration references (`env_ref` items) from the environment of an application.
//!
//! Decision 0015: a variable's **value** is read only as a temporary string, parsed into a host and
//! an optional port, and dropped. Only `target_host` and `target_port` are kept. User names,
//! passwords, paths, query strings and values that do not parse never reach an item, a warning, a
//! key or any `Debug` output (the types here hold no raw value).
//!
//! Recognised shapes:
//! * a URL `scheme://[userinfo@]host[:port][/...]` (IPv6 as `[::1]`); without a port the default
//!   of a known scheme is used (redis 6379, postgres/postgresql 5432, mysql/mariadb 3306, mongodb
//!   27017, amqp 5672, http 80, https 443; also `rediss`, `amqps`, `ws`, `wss`), otherwise the
//!   port is `null`. A `+driver` suffix is ignored for the lookup (`postgresql+asyncpg`); `+srv`
//!   gives no port;
//! * a plain `host:port`;
//! * a pair `<PREFIX>_HOST` + `<PREFIX>_PORT` (`DB_HOST`, `REDIS_HOST`, `APP_DB_HOST`, ...). The
//!   item is named after the `_HOST` variable; a missing or invalid `_PORT` gives a `null` port.
//!
//! Anything else is discarded silently. Only values of variables whose name looks like a reference
//! (`*_URL`, `*_URI`, `*_DSN`, `*_HOST`, `*_ADDR`, `*_ENDPOINT`) are counted as "could not be
//! parsed", so ordinary variables (`PATH`, `NODE_ENV`) never make an inventory partial.

use std::collections::BTreeMap;

use inventory::{Item, Kind, SeenVia};
use serde_json::{Map, Value};

use crate::util::clean;

/// Most `env_ref` items per application.
pub(crate) const MAX_PER_APP: usize = 200;
/// Most `env_ref` items per inventory.
pub(crate) const MAX_PER_INVENTORY: usize = 5_000;
/// Longest variable name kept.
const MAX_NAME_CHARS: usize = 128;
/// Longest host kept (the DNS limit).
const MAX_HOST_CHARS: usize = 253;
/// Variables looked at per application (a bound on work, not on output).
const MAX_VARS: usize = 5_000;
/// Longest value looked at; a longer one cannot be a plain reference and is discarded.
const MAX_VALUE_BYTES: usize = 4_096;

/// Name endings that mark a variable as meant to point somewhere.
const REFERENCE_SUFFIXES: [&str; 6] = ["_URL", "_URI", "_DSN", "_HOST", "_ADDR", "_ENDPOINT"];

/// A parsed destination. Holds no secret by construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Target {
    pub(crate) host: String,
    pub(crate) port: Option<u16>,
}

/// Result of extracting the references of one application.
#[derive(Debug, Default)]
pub(crate) struct Extracted {
    pub(crate) items: Vec<Item>,
    /// Reference-named variables whose value did not parse.
    pub(crate) unparsed: usize,
    /// References beyond [`MAX_PER_APP`] that were not recorded.
    pub(crate) over_cap: usize,
}

fn default_port(scheme: &str) -> Option<u16> {
    let (base, driver) = scheme
        .split_once('+')
        .map_or((scheme, None), |(b, d)| (b, Some(d)));
    if driver == Some("srv") {
        return None;
    }
    match base {
        "redis" | "rediss" => Some(6379),
        "postgres" | "postgresql" => Some(5432),
        "mysql" | "mariadb" => Some(3306),
        "mongodb" => Some(27017),
        "amqp" => Some(5672),
        "amqps" => Some(5671),
        "http" | "ws" => Some(80),
        "https" | "wss" => Some(443),
        _ => None,
    }
}

fn parse_port(text: &str) -> Option<u16> {
    if text.is_empty() || text.len() > 5 || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse::<u16>().ok().filter(|p| *p != 0)
}

fn is_ipv4(host: &str) -> bool {
    let parts: Vec<&str> = host.split('.').collect();
    parts.len() == 4
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 3 && p.bytes().all(|b| b.is_ascii_digit()))
}

/// A host name, IPv4 address or (unbracketed) IPv6 address, lowercased; `None` when it is not one.
fn valid_host(host: &str, bracketed: bool) -> Option<String> {
    if host.is_empty() || host.len() > MAX_HOST_CHARS {
        return None;
    }
    if bracketed {
        let ok = host.contains(':')
            && host
                .bytes()
                .all(|b| b.is_ascii_hexdigit() || b == b':' || b == b'.');
        return ok.then(|| clean(&host.to_ascii_lowercase(), MAX_HOST_CHARS));
    }
    let chars_ok = host
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_');
    // A bare number such as `12` (from `12:30`) is not a host.
    let shaped = is_ipv4(host) || host.bytes().any(|b| b.is_ascii_alphabetic());
    (chars_ok && shaped).then(|| clean(&host.to_ascii_lowercase(), MAX_HOST_CHARS))
}

/// Splits `host[:port]` or `[v6][:port]`. `Err` when it is malformed; `Ok(None port)` when absent.
fn split_host_port(text: &str) -> Option<(String, Option<u16>)> {
    if let Some(rest) = text.strip_prefix('[') {
        let (inside, after) = rest.split_once(']')?;
        let host = valid_host(inside, true)?;
        return match after {
            "" => Some((host, None)),
            _ => Some((host, Some(parse_port(after.strip_prefix(':')?)?))),
        };
    }
    match text.split_once(':') {
        None => Some((valid_host(text, false)?, None)),
        Some((host, port)) => Some((valid_host(host, false)?, Some(parse_port(port)?))),
    }
}

/// Splits `scheme://rest`; the scheme is lowercased.
fn split_scheme(value: &str) -> Option<(String, &str)> {
    let (scheme, rest) = value.split_once("://")?;
    let mut chars = scheme.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_alphabetic());
    let rest_ok = chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'));
    (first_ok && rest_ok && scheme.len() <= 32).then(|| (scheme.to_ascii_lowercase(), rest))
}

/// Parses a URL value; userinfo, path and query are dropped here.
fn parse_url(value: &str) -> Option<Target> {
    let (scheme, rest) = split_scheme(value)?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = rest.get(..end)?;
    // An `@` after the first `/`, `?` or `#` with none before it means the userinfo (a password
    // with an unencoded `/`, `?` or `#`) was cut in half: what is left of it would be read as
    // host and port. Ambiguous, so discarded.
    if !authority.contains('@') && rest.get(end..)?.contains('@') {
        return None;
    }
    let hostport = authority.rsplit_once('@').map_or(authority, |(_, hp)| hp);
    // Replica lists (`mongodb://a:1,b:2`): the first member is the reference.
    let first = hostport.split(',').next().unwrap_or(hostport);
    let (host, port) = split_host_port(first)?;
    Some(Target {
        host,
        port: port.or_else(|| default_port(&scheme)),
    })
}

/// Parses one value as a URL or a plain `host:port`. `None` when it is neither.
pub(crate) fn parse_value(value: &str) -> Option<Target> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_VALUE_BYTES || value.chars().any(char::is_whitespace) {
        return None;
    }
    if value.contains("://") {
        return parse_url(value);
    }
    let (host, port) = split_host_port(value)?;
    port.map(|p| Target {
        host,
        port: Some(p),
    })
}

/// The value of a `_HOST` variable: a bare host, or anything [`parse_value`] understands.
fn parse_host_value(value: &str) -> Option<Target> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_VALUE_BYTES || value.chars().any(char::is_whitespace) {
        return None;
    }
    if let Some(target) = parse_value(value) {
        return Some(target);
    }
    let bracketed = value.starts_with('[');
    let host = if bracketed {
        valid_host(value.strip_prefix('[')?.strip_suffix(']')?, true)?
    } else {
        valid_host(value, false)?
    };
    Some(Target { host, port: None })
}

/// Names that hold credentials: skipped entirely, whatever their value looks like.
const SECRET_NAME_PARTS: [&str; 7] = ["PASS", "SECRET", "TOKEN", "KEY", "AUTH", "CRED", "PRIVATE"];

fn looks_secret(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    SECRET_NAME_PARTS.iter().any(|part| upper.contains(part))
}

fn looks_like_reference(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    REFERENCE_SUFFIXES.iter().any(|s| upper.ends_with(s))
}

fn env_ref_item(app: &str, name: &str, target: &Target, in_container: bool, via: SeenVia) -> Item {
    let name = clean(name, MAX_NAME_CHARS);
    let mut data = Map::new();
    data.insert("app".into(), Value::from(app));
    data.insert("name".into(), Value::from(name.clone()));
    data.insert("target_host".into(), Value::from(target.host.clone()));
    data.insert(
        "target_port".into(),
        target.port.map_or(Value::Null, Value::from),
    );
    data.insert("in_container".into(), Value::Bool(in_container));
    data.insert("resolves_to".into(), Value::Null);
    Item {
        kind: Kind::EnvRef,
        key: format!("{app}:{name}"),
        data,
        seen_via: via,
    }
}

/// Builds the `env_ref` items of the application whose process key is `app` from its variables
/// (`name`, `value` pairs; values are only inspected here). Items come out sorted by name.
pub(crate) fn extract<'a>(
    app: &str,
    vars: impl Iterator<Item = (&'a str, &'a str)>,
    in_container: bool,
    via: SeenVia,
) -> Extracted {
    let vars: BTreeMap<&str, &str> = vars
        .filter(|(name, _)| !name.is_empty())
        .take(MAX_VARS)
        .collect();
    let mut out = Extracted::default();
    for (name, value) in &vars {
        if looks_secret(name) {
            continue;
        }
        let target = if let Some(prefix) = name.strip_suffix("_HOST") {
            let port = vars
                .get(format!("{prefix}_PORT").as_str())
                .and_then(|p| parse_port(p.trim()));
            parse_host_value(value).map(|mut t| {
                if t.port.is_none() {
                    t.port = port;
                }
                t
            })
        } else if value.contains("://") || looks_like_reference(name) {
            // A bare `host:port` is only trusted for variables named like a reference: in any
            // other variable it can just as well be `user:1234` or a secret.
            parse_value(value)
        } else {
            None
        };
        match target {
            Some(target) => {
                if out.items.len() >= MAX_PER_APP {
                    out.over_cap += 1;
                } else {
                    out.items
                        .push(env_ref_item(app, name, &target, in_container, via));
                }
            }
            None if !value.trim().is_empty() && looks_like_reference(name) => out.unparsed += 1,
            None => {}
        }
    }
    out
}

/// Warnings for the counters of [`Extracted`], summed over all applications of one collector.
pub(crate) fn warnings(unparsed: usize, over_cap: usize) -> Vec<String> {
    let mut out = Vec::new();
    if unparsed > 0 {
        out.push(format!(
            "{unparsed} environment variable(s) that look like references could not be parsed and were skipped"
        ));
    }
    if over_cap > 0 {
        out.push(format!(
            "{over_cap} environment reference(s) beyond the limit of {MAX_PER_APP} per application were not recorded"
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn ambiguous_userinfo_and_secret_names_never_yield_a_target() {
        for value in [
            "redis://admin:1234/x@db.example.test:6379",
            "postgres://u:p#ss@db.example.test",
            "redis://u:9?z@h",
        ] {
            assert!(super::parse_value(value).is_none(), "{value}");
        }
        let vars = [
            ("DB_PASSWORD", "hunter:8080"),
            ("NOTE", "user:1234"),
            ("CACHE_ADDR", "cache.example.test:6379"),
            ("API_KEY", "redis://h:1"),
        ];
        let out = super::extract(
            "pm2:a",
            vars.iter().copied(),
            false,
            inventory::SeenVia::Config,
        );
        let keys: Vec<_> = out.items.iter().map(|i| i.key.as_str()).collect();
        assert_eq!(keys, ["pm2:a:CACHE_ADDR"]);
    }

    use super::*;

    fn t(value: &str) -> Option<(String, Option<u16>)> {
        parse_value(value).map(|t| (t.host, t.port))
    }

    fn hp(host: &str, port: Option<u16>) -> Option<(String, Option<u16>)> {
        Some((host.to_owned(), port))
    }

    #[test]
    fn urls() {
        assert_eq!(
            t("redis://:pw@127.0.0.1:6379/0"),
            hp("127.0.0.1", Some(6379))
        );
        assert_eq!(
            t("redis://cache.internal"),
            hp("cache.internal", Some(6379))
        );
        assert_eq!(t("postgres://u:p@db/app?sslmode=x"), hp("db", Some(5432)));
        assert_eq!(
            t("postgresql+asyncpg://u@Db.Example"),
            hp("db.example", Some(5432))
        );
        assert_eq!(t("mysql://u:p@h:3307/x"), hp("h", Some(3307)));
        assert_eq!(t("mongodb://a:1,b:2/x"), hp("a", Some(1)));
        assert_eq!(
            t("mongodb+srv://u:p@cluster.example/x"),
            hp("cluster.example", None)
        );
        assert_eq!(t("amqp://guest:guest@mq"), hp("mq", Some(5672)));
        assert_eq!(t("http://svc"), hp("svc", Some(80)));
        assert_eq!(t("https://svc/path"), hp("svc", Some(443)));
        assert_eq!(t("custom://svc"), hp("svc", None));
        assert_eq!(t("redis://[::1]:6380/1"), hp("::1", Some(6380)));
        assert_eq!(t("redis://[::1]"), hp("::1", Some(6379)));
        assert_eq!(t("redis://u:p@w@host:1"), hp("host", Some(1)));
    }

    #[test]
    fn plain_host_port() {
        assert_eq!(t("localhost:3000"), hp("localhost", Some(3000)));
        assert_eq!(t("127.0.0.1:80"), hp("127.0.0.1", Some(80)));
        assert_eq!(t("[::1]:9"), hp("::1", Some(9)));
    }

    #[test]
    fn garbage_is_discarded() {
        for bad in [
            "",
            "production",
            "/usr/bin:/bin",
            "12:30",
            "host:0",
            "host:70000",
            "host:port",
            "a b:80",
            "redis://",
            "redis://:pw@/var/run/x.sock",
            "redis://[::1",
            "://x",
            "1x://h",
            "host:80:81",
            "http://ho st",
        ] {
            assert_eq!(t(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn host_port_pairs() {
        let vars = [
            ("DB_HOST", "127.0.0.1"),
            ("DB_PORT", "5432"),
            ("APP_REDIS_HOST", "cache"),
            ("APP_REDIS_PORT", "oops"),
            ("MAIL_HOST", "smtp.example"),
            ("NODE_ENV", "production"),
            ("BROKEN_URL", "not a url"),
        ];
        let out = extract("pm2:web", vars.iter().copied(), false, SeenVia::Config);
        let got: Vec<(String, Option<u64>)> = out
            .items
            .iter()
            .map(|i| (i.key.clone(), i.u64_field("target_port")))
            .collect();
        assert_eq!(
            got,
            [
                ("pm2:web:APP_REDIS_HOST".to_owned(), None),
                ("pm2:web:DB_HOST".to_owned(), Some(5432)),
                ("pm2:web:MAIL_HOST".to_owned(), None),
            ]
        );
        assert_eq!(out.unparsed, 1);
    }

    #[test]
    fn caps_per_app() {
        let names: Vec<String> = (0..MAX_PER_APP + 5)
            .map(|i| format!("V{i:04}_URL"))
            .collect();
        let vars = names.iter().map(|n| (n.as_str(), "redis://h"));
        let out = extract("pm2:a", vars, false, SeenVia::Config);
        assert_eq!(out.items.len(), MAX_PER_APP);
        assert_eq!(out.over_cap, 5);
    }

    #[test]
    fn items_carry_no_value() {
        let out = extract(
            "docker:x",
            [(
                "DATABASE_URL",
                "postgres://admin:s3cr3t-pw@db:5432/prod?token=abc",
            )]
            .into_iter(),
            true,
            SeenVia::Docker,
        );
        let text = serde_json::to_string(&out.items).unwrap_or_default();
        assert!(text.contains("\"target_host\":\"db\""));
        for secret in ["admin", "s3cr3t", "prod", "token=abc"] {
            assert!(!text.contains(secret), "{secret}");
        }
    }
}
