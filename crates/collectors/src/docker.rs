//! Running Docker containers, from `docker ps` and `docker inspect` (kind `process`, seen via
//! `docker`).
//!
//! One item per running container, key `docker:<name>`. Data (docs/design.md 4.1): `cmd` (the image
//! reference, redacted and bounded), `cwd` (null), `mode` (`"unknown"`), `supervised` (true: the
//! container runtime owns it), `env_refs` (variable **names** from `Config.Env`, never values),
//! `instances` (1), `version` (see below), plus `restart_policy` and `ports` (published `host/proto` strings) when
//! `docker inspect` answered.
//!
//! `version` is the image **tag**: the text after the last `:` of the image reference when that
//! colon comes after the last `/` (so a registry port is not a tag), after any `@sha256:...` digest
//! was removed. It is `null` for no tag, for `latest`, for a digest-only reference and for a tag
//! with characters Docker does not allow. The image reference itself stays redacted in `cmd`, so
//! registry credentials are never stored.
//!
//! Each `Config.Env` entry `NAME=value` (split at the first `=`) that parses as a reference (see
//! `crate::envref`, decision 0015) also becomes an `env_ref` item, key `docker:<name>:<NAME>`,
//! `in_container` true. Only host and port are kept, never the value.
//!
//! No Docker installed (or not allowed to run) is normal and gives an empty collection without a
//! warning. A failed or timed-out `docker ps` (for example no access to the socket) is a warning, so
//! the inventory becomes partial. Malformed lines, failed inspects and containers beyond the cap are
//! each counted in one warning that never contains their content. Commands go through
//! [`CommandRunner`] only.

use std::collections::BTreeSet;
use std::sync::Arc;

use core_domain::redact::redact;
use inventory::{CollectError, Collection, Collector, Item, Kind, SeenVia};
use serde_json::{Map, Value, json};

use crate::envref::{self, Extracted};
use crate::exec::{Cmd, CommandRunner, ExecError};
use crate::util::{MAX_CMD_CHARS, clean, truncate_chars};

/// Most containers turned into items; the rest are counted in a warning.
const MAX_CONTAINERS: usize = 100;
/// Longest container name kept, in characters.
const MAX_NAME_CHARS: usize = 100;
/// Most environment variable names and published ports kept per container.
const MAX_LIST: usize = 200;
/// Characters of the container id used for `docker inspect`.
const ID_CHARS: usize = 12;
/// Longest image tag Docker allows.
const MAX_TAG_CHARS: usize = 128;

/// Collector of running Docker containers.
#[derive(Clone)]
pub struct Docker {
    runner: Arc<dyn CommandRunner>,
}

impl std::fmt::Debug for Docker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Docker").finish_non_exhaustive()
    }
}

impl Docker {
    /// A collector that runs its queries through `runner`.
    #[must_use]
    pub fn new(runner: Arc<dyn CommandRunner>) -> Self {
        Self { runner }
    }
}

/// What `docker inspect` added for one container.
#[derive(Default)]
struct Inspected {
    restart_policy: Option<String>,
    ports: Vec<String>,
    env_names: Vec<String>,
    refs: Extracted,
}

fn text<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field).and_then(Value::as_str)
}

/// A variable name from `NAME=value` (or a bare `NAME`); the value is never looked at.
fn env_name(entry: &str) -> Option<String> {
    let name = entry.split_once('=').map_or(entry, |(name, _)| name);
    (!name.is_empty()).then(|| clean(name, MAX_NAME_CHARS))
}

fn parse_inspect(output: &str, app: &str) -> Option<Inspected> {
    let parsed: Value = serde_json::from_str(output).ok()?;
    let first = parsed.as_array()?.first()?;
    let mut out = Inspected {
        restart_policy: first
            .get("HostConfig")
            .and_then(|h| h.get("RestartPolicy"))
            .and_then(|r| text(r, "Name"))
            .filter(|name| !name.is_empty())
            .map(|name| clean(name, 32)),
        ..Inspected::default()
    };
    let names: BTreeSet<String> = first
        .get("Config")
        .and_then(|c| c.get("Env"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(env_name)
        .collect();
    out.env_names = names.into_iter().take(MAX_LIST).collect();
    let entries: Vec<(&str, &str)> = first
        .get("Config")
        .and_then(|c| c.get("Env"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(|entry| entry.split_once('='))
        .collect();
    out.refs = envref::extract(app, entries.into_iter(), true, SeenVia::Docker);
    let mut ports = BTreeSet::new();
    if let Some(map) = first
        .get("NetworkSettings")
        .and_then(|n| n.get("Ports"))
        .and_then(Value::as_object)
    {
        for (container_port, bindings) in map {
            let proto = container_port.split_once('/').map_or("tcp", |(_, p)| p);
            for binding in bindings.as_array().into_iter().flatten() {
                if let Some(host_port) = text(binding, "HostPort").filter(|p| !p.is_empty()) {
                    ports.insert(format!("{}/{}", clean(host_port, 8), clean(proto, 8)));
                }
            }
        }
    }
    out.ports = ports.into_iter().take(MAX_LIST).collect();
    Some(out)
}

/// Image reference with secrets hidden. The reference is redacted as a URL (a scheme is added and
/// removed again) so `user:password@registry/app` loses its password; `NAME=value` and known token
/// shapes are covered by [`redact`] as well.
fn redacted_image(image: &str) -> String {
    const SCHEME: &str = "docker://";
    let redacted = redact(&format!("{SCHEME}{image}")).text;
    let shown = redacted.strip_prefix(SCHEME).unwrap_or(&redacted);
    truncate_chars(shown, MAX_CMD_CHARS)
}

/// The tag of an image reference, or `None` (no tag, `latest`, digest only, odd characters).
fn image_tag(image: &str) -> Option<String> {
    // A digest follows the last `@` and has no `/`; an `@` followed by a `/` is registry userinfo.
    let without_digest = match image.rsplit_once('@') {
        Some((head, tail)) if !tail.contains('/') => head,
        _ => image,
    };
    let after_slash = without_digest
        .rsplit_once('/')
        .map_or(without_digest, |(_, rest)| rest);
    let (_, tag) = after_slash.rsplit_once(':')?;
    let valid = !tag.is_empty()
        && tag.len() <= MAX_TAG_CHARS
        && tag
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'));
    (valid && tag != "latest").then(|| tag.to_owned())
}

fn build_item(name: &str, image: &str, inspected: Option<&Inspected>) -> Item {
    let mut data = Map::new();
    data.insert("cmd".into(), Value::from(redacted_image(image)));
    data.insert("cwd".into(), Value::Null);
    data.insert("mode".into(), json!("unknown"));
    data.insert("supervised".into(), Value::Bool(true));
    data.insert(
        "env_refs".into(),
        Value::from(inspected.map_or_else(Vec::new, |i| i.env_names.clone())),
    );
    data.insert("instances".into(), Value::from(1u64));
    data.insert(
        "version".into(),
        image_tag(image).map_or(Value::Null, Value::from),
    );
    if let Some(i) = inspected {
        if let Some(policy) = &i.restart_policy {
            data.insert("restart_policy".into(), Value::from(policy.clone()));
        }
        data.insert("ports".into(), Value::from(i.ports.clone()));
    }
    Item {
        kind: Kind::Process,
        key: format!("docker:{name}"),
        data,
        seen_via: SeenVia::Docker,
    }
}

impl Collector for Docker {
    fn kind(&self) -> &'static str {
        "docker"
    }

    fn run(&self) -> Result<Collection, CollectError> {
        let output = match self.runner.run(&Cmd::DockerPs) {
            Ok(output) => output,
            Err(ExecError::NotAvailable) => return Ok(Collection::default()),
            Err(ExecError::TimedOut) => {
                return Ok(Collection {
                    items: Vec::new(),
                    warnings: vec!["docker ps timed out".to_owned()],
                });
            }
            Err(ExecError::Failed(reason)) => {
                return Ok(Collection {
                    items: Vec::new(),
                    warnings: vec![format!("docker ps failed: {}", clean(&reason, 120))],
                });
            }
        };
        let mut items = Vec::new();
        let mut refs: Vec<Item> = Vec::new();
        let (mut unparsed, mut refs_over_cap) = (0usize, 0usize);
        let (mut malformed, mut inspect_failed, mut over_cap) = (0usize, 0usize, 0usize);
        for line in output.lines().filter(|l| !l.trim().is_empty()) {
            let Ok(entry) = serde_json::from_str::<Value>(line) else {
                malformed += 1;
                continue;
            };
            let name = text(&entry, "Names")
                .and_then(|n| n.split(',').next())
                .map(str::trim)
                .filter(|n| !n.is_empty());
            let (Some(name), Some(id)) = (name, text(&entry, "ID")) else {
                malformed += 1;
                continue;
            };
            if text(&entry, "State").is_some_and(|s| s != "running") {
                continue;
            }
            if items.len() >= MAX_CONTAINERS {
                over_cap += 1;
                continue;
            }
            let name = clean(name, MAX_NAME_CHARS);
            let key = format!("docker:{name}");
            let short: String = id.chars().take(ID_CHARS).collect();
            let mut inspected = match self.runner.run(&Cmd::DockerInspect(short)) {
                Ok(out) => {
                    let parsed = parse_inspect(&out, &key);
                    if parsed.is_none() {
                        inspect_failed += 1;
                    }
                    parsed
                }
                Err(_) => {
                    inspect_failed += 1;
                    None
                }
            };
            let image = text(&entry, "Image").unwrap_or_default();
            if let Some(found) = inspected.as_mut().map(|i| std::mem::take(&mut i.refs)) {
                unparsed += found.unparsed;
                refs_over_cap += found.over_cap;
                refs.extend(found.items);
            }
            items.push(build_item(&name, image, inspected.as_ref()));
        }
        let mut warnings = Vec::new();
        if malformed > 0 {
            warnings.push(format!("{malformed} docker ps line(s) could not be parsed"));
        }
        if inspect_failed > 0 {
            warnings.push(format!(
                "docker inspect gave no usable answer for {inspect_failed} container(s); restart policy, ports and env names are missing for them"
            ));
        }
        if over_cap > 0 {
            warnings.push(format!(
                "{over_cap} container(s) beyond the limit of {MAX_CONTAINERS} were not recorded"
            ));
        }
        warnings.extend(envref::warnings(unparsed, refs_over_cap));
        items.extend(refs);
        Ok(Collection { items, warnings })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::FakeRunner;

    const ID1: &str = "0123456789abcdef0123456789abcdef";
    const ID2: &str = "fedcba9876543210fedcba9876543210";

    fn ps(id: &str, name: &str, image: &str) -> String {
        json!({"ID": id, "Names": name, "Image": image, "State": "running", "Status": "Up"})
            .to_string()
    }

    fn inspect_cmd(id: &str) -> Cmd {
        Cmd::DockerInspect(id.chars().take(ID_CHARS).collect())
    }

    fn collect(runner: FakeRunner) -> Collection {
        Docker::new(Arc::new(runner)).run().unwrap_or_default()
    }

    const INSPECT: &str = r#"[{"Config":{"Env":["DB_PASSWORD=hunter2","PATH=/bin","TOKEN"]},
        "HostConfig":{"RestartPolicy":{"Name":"always"}},
        "NetworkSettings":{"Ports":{"80/tcp":[{"HostIp":"0.0.0.0","HostPort":"8080"}],"443/tcp":null}}}]"#;

    #[test]
    fn image_tags_become_versions() {
        let cases: [(&str, Option<&str>); 11] = [
            ("redis:7.2", Some("7.2")),
            ("nginx", None),
            ("nginx:latest", None),
            ("repo/app@sha256:0123abcd", None),
            ("repo/app:1.4@sha256:0123abcd", Some("1.4")),
            ("registry:5000/app", None),
            ("registry:5000/app:2.0-rc1", Some("2.0-rc1")),
            ("user:pw@registry.example/app", None),
            ("user:pw@registry.example/app:3", Some("3")),
            ("app:", None),
            ("app:bad tag", None),
        ];
        for (image, expected) in cases {
            assert_eq!(image_tag(image).as_deref(), expected, "{image}");
        }
    }

    #[test]
    fn version_is_on_the_item() {
        let out = collect(
            FakeRunner::new()
                .with(Cmd::DockerPs, Ok(ps(ID1, "cache", "redis:7.2")))
                .with(inspect_cmd(ID1), Ok(INSPECT.into())),
        );
        assert_eq!(out.items[0].str_field("version"), Some("7.2"));
        let out = collect(FakeRunner::new().with(Cmd::DockerPs, Ok(ps(ID1, "web", "nginx"))));
        assert_eq!(out.items[0].data.get("version"), Some(&Value::Null));
    }

    #[test]
    fn env_references_become_env_ref_items() {
        // Built at run time so no token-shaped literal sits in the source (secret scanners).
        let token = format!("ghp_{}", "q".repeat(36));
        let inspect = format!(
            r#"[{{"Config":{{"Env":[
            "DATABASE_URL=postgres://admin:s3cr3t@localhost/prod?x=1",
            "CACHE_HOST=redis","CACHE_PORT=6380","TOKEN={token}",
            "PATH=/usr/bin:/bin","BROKEN_URL=@@@ nope"]}}}}]"#
        );
        let out = collect(
            FakeRunner::new()
                .with(Cmd::DockerPs, Ok(ps(ID1, "web", "app:1")))
                .with(inspect_cmd(ID1), Ok(inspect.clone())),
        );
        let refs: Vec<&Item> = out
            .items
            .iter()
            .filter(|i| i.kind == Kind::EnvRef)
            .collect();
        let keys: Vec<&str> = refs.iter().map(|i| i.key.as_str()).collect();
        assert_eq!(keys, ["docker:web:CACHE_HOST", "docker:web:DATABASE_URL"]);
        assert_eq!(refs[0].u64_field("target_port"), Some(6380));
        assert_eq!(refs[1].str_field("target_host"), Some("localhost"));
        assert_eq!(refs[1].u64_field("target_port"), Some(5432));
        assert_eq!(refs[1].data.get("in_container"), Some(&json!(true)));
        assert_eq!(refs[1].seen_via, SeenVia::Docker);
        assert_eq!(out.warnings.len(), 1, "{:?}", out.warnings);
        let all = format!("{:?}{:?}", out.items, out.warnings);
        for secret in ["admin", "s3cr3t", "ghp_", "nope", "prod"] {
            assert!(!all.contains(secret), "{secret}");
        }
    }

    #[test]
    fn normal_container_with_inspect() {
        let out = collect(
            FakeRunner::new()
                .with(Cmd::DockerPs, Ok(ps(ID1, "web", "nginx:1.27")))
                .with(inspect_cmd(ID1), Ok(INSPECT.into())),
        );
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        assert_eq!(out.items.len(), 1);
        let item = &out.items[0];
        assert_eq!(item.key, "docker:web");
        assert_eq!(item.kind, Kind::Process);
        assert_eq!(item.seen_via, SeenVia::Docker);
        assert_eq!(item.str_field("cmd"), Some("nginx:1.27"));
        assert_eq!(item.str_field("mode"), Some("unknown"));
        assert_eq!(item.str_field("restart_policy"), Some("always"));
        assert_eq!(item.data.get("supervised"), Some(&json!(true)));
        assert_eq!(item.data.get("cwd"), Some(&Value::Null));
        assert_eq!(item.data.get("ports"), Some(&json!(["8080/tcp"])));
        assert_eq!(item.u64_field("instances"), Some(1));
    }

    #[test]
    fn two_containers() {
        let lines = format!("{}\n{}\n", ps(ID1, "a", "x"), ps(ID2, "b", "y"));
        let out = collect(FakeRunner::new().with(Cmd::DockerPs, Ok(lines)));
        let keys: Vec<_> = out.items.iter().map(|i| i.key.as_str()).collect();
        assert_eq!(keys, ["docker:a", "docker:b"]);
        // Inspect was not answered: one summary warning, items still present.
        assert_eq!(out.warnings.len(), 1);
    }

    #[test]
    fn not_available_is_silent() {
        let out = collect(FakeRunner::new());
        assert!(out.items.is_empty() && out.warnings.is_empty());
    }

    #[test]
    fn failed_and_timed_out_warn() {
        let failed = collect(FakeRunner::new().with(
            Cmd::DockerPs,
            Err(ExecError::Failed("permission denied".into())),
        ));
        assert!(failed.items.is_empty());
        assert_eq!(failed.warnings.len(), 1);
        let slow = collect(FakeRunner::new().with(Cmd::DockerPs, Err(ExecError::TimedOut)));
        assert_eq!(slow.warnings.len(), 1);
    }

    #[test]
    fn malformed_lines_counted_once_without_content() {
        let lines = format!("SECRET-garbage\n{{broken\n{}\n", ps(ID1, "ok", "i"));
        let out = collect(FakeRunner::new().with(Cmd::DockerPs, Ok(lines)));
        assert_eq!(out.items.len(), 1);
        let malformed: Vec<_> = out
            .warnings
            .iter()
            .filter(|w| w.contains("parsed"))
            .collect();
        assert_eq!(malformed.len(), 1);
        assert!(malformed[0].starts_with("2 "));
        assert!(!out.warnings.iter().any(|w| w.contains("SECRET")));
    }

    #[test]
    fn hostile_name_is_cleaned() {
        let name = format!("ev\u{1b}[31mil\n{}", "x".repeat(1000));
        let line = ps(ID1, &name, "img");
        let out = collect(FakeRunner::new().with(Cmd::DockerPs, Ok(line)));
        assert_eq!(out.items.len(), 1);
        let key = &out.items[0].key;
        assert!(!key.chars().any(char::is_control));
        assert!(key.chars().count() <= "docker:".len() + MAX_NAME_CHARS);
    }

    #[test]
    fn env_names_without_values() {
        let out = collect(
            FakeRunner::new()
                .with(Cmd::DockerPs, Ok(ps(ID1, "web", "nginx")))
                .with(inspect_cmd(ID1), Ok(INSPECT.into())),
        );
        let refs = out.items[0].data.get("env_refs");
        assert_eq!(refs, Some(&json!(["DB_PASSWORD", "PATH", "TOKEN"])));
        assert!(!format!("{:?}", out.items[0]).contains("hunter2"));
    }

    #[test]
    fn secrets_in_image_are_redacted() {
        let image = "user:hunter2pw@registry.example.test/app:1";
        let out = collect(FakeRunner::new().with(Cmd::DockerPs, Ok(ps(ID1, "app", image))));
        let rendered = format!("{:?}", out.items);
        assert!(!rendered.contains("hunter2pw"), "{rendered}");
    }

    #[test]
    fn caps_container_count_and_garbage_inspect_is_survived() {
        let lines: String = (0..105)
            .map(|n| ps(&format!("{n:012x}"), &format!("c{n}"), "i") + "\n")
            .collect();
        let out = collect(FakeRunner::new().with(Cmd::DockerPs, Ok(lines)));
        assert_eq!(out.items.len(), MAX_CONTAINERS);
        assert!(out.warnings.iter().any(|w| w.starts_with("5 ")));
        let bad = collect(
            FakeRunner::new()
                .with(Cmd::DockerPs, Ok(ps(ID1, "a", "i")))
                .with(inspect_cmd(ID1), Ok("[[[null".into())),
        );
        assert_eq!(bad.items.len(), 1);
    }
}
