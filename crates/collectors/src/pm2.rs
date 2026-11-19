//! PM2 applications, from the saved process list `~/.pm2/dump.pm2` (kind `process`, seen via
//! `config`).
//!
//! PM2 is read from **files only**: `pm2 jlist` and friends can start the PM2 daemon, so no
//! command is ever run. The dump files are found by listing `/home/*/.pm2/dump.pm2` and
//! `/root/.pm2/dump.pm2` through [`RootFs`] (`read_dir` / `read_to_string`, nothing else).
//!
//! **A dump lists the apps that were saved (`pm2 save`), not necessarily the apps that are
//! running.** It can be stale in either direction. Every item therefore carries
//! `saved_only: true`: this collector only proves "PM2 would resurrect this app", never "it runs
//! now". Pair it with the process collector for liveness.
//!
//! Item key `pm2:<name>`. Data (docs/design.md 4.1): `cmd` (redacted exec path and arguments, at
//! most 512 characters, or null when the dump has no exec path), `cwd` (path or null), `mode`
//! (`"dev"` when `env.NODE_ENV` is `development` or the command is a known development server;
//! `"prod"` only when `NODE_ENV` is `production`; otherwise `"unknown"`), `supervised` (always
//! true: PM2 restarts it), `env_refs` (sorted names of the app's `env` variables, **never
//! values**), plus the additive fields `instances` (PM2 `instances` when it is a positive number,
//! else 1), `saved_only` (true) and `owner` (the account whose home holds the dump).
//!
//! Apps with the same name across several users' dumps are merged into one item; `instances` are
//! summed and the first dump supplies the other fields; `/root` is read first. Links at `.pm2` or
//! at the dump are skipped with a warning (never followed). File ownership is not checked: it needs
//! metadata that `RootFs` does not expose.
//!
//! For each variable of the app's `env` that parses as a reference (URL, `host:port` or a
//! `<PREFIX>_HOST` + `<PREFIX>_PORT` pair) the collector also emits an `env_ref` item, key
//! `pm2:<name>:<VARIABLE>`, `in_container` false (see `crate::envref`, decision 0015). Only host
//! and port are kept, never the value, user name, password or path. An app listed in several dumps
//! takes its references from the first dump.
//!
//! Failure handling: no dump anywhere gives an empty collection without a warning (PM2 is simply
//! not used). Unreadable, oversized or invalid dumps, and malformed app entries, each produce one
//! counting warning per problem class. A warning never contains dump content. Environment values
//! are never copied into any item, warning or key.

use std::collections::BTreeMap;

use inventory::{CollectError, Collection, Collector, Item, Kind, SeenVia};
use serde_json::{Map, Value};

use crate::envref::{self, Extracted};
use crate::rootfs::{FsError, RootFs};
use crate::supervision::is_dev_command;
use crate::util::{MAX_CMD_CHARS, clean, redact_command_args, split_shell_words, truncate_chars};

/// Largest dump read; a longer file is reported, not parsed (a cut JSON document is invalid).
const MAX_DUMP_BYTES: usize = 4 * 1024 * 1024;
/// Most home directories listed.
const MAX_HOMES: usize = 4096;
/// Most apps taken from one dump.
const MAX_APPS_PER_DUMP: usize = 10_000;
/// Longest app name kept (it becomes part of the item key).
const MAX_NAME_CHARS: usize = 200;
const MAX_CWD_CHARS: usize = 512;
const MAX_ENV_NAME_CHARS: usize = 128;
const MAX_ENV_NAMES: usize = 500;
/// Most arguments looked at per app.
const MAX_ARGS: usize = 256;

/// Collector of PM2 apps saved in `dump.pm2` files.
#[derive(Debug, Clone)]
pub struct Pm2 {
    root: RootFs,
}

impl Pm2 {
    /// A collector reading from `root`.
    #[must_use]
    pub fn new(root: RootFs) -> Self {
        Self { root }
    }
}

/// Counters behind the per-class warnings.
#[derive(Default)]
struct Problems {
    denied: usize,
    read_failed: usize,
    too_large: usize,
    invalid: usize,
    bad_apps: usize,
    symlinks: usize,
    dirs_denied: bool,
    truncated_homes: bool,
}

impl Problems {
    fn warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut add = |count: usize, what: &str| {
            if count > 0 {
                out.push(format!("pm2: {count} dump file(s) {what}"));
            }
        };
        add(self.denied, "could not be read (permission denied)");
        add(self.read_failed, "could not be read (I/O error)");
        add(
            self.too_large,
            "were larger than the read limit and were skipped",
        );
        add(self.invalid, "are not a valid PM2 dump (JSON array)");
        add(
            self.symlinks,
            "were skipped because `.pm2` or the dump is a symbolic link",
        );
        if self.bad_apps > 0 {
            out.push(format!(
                "pm2: {} app entr(ies) without a usable name were skipped",
                self.bad_apps
            ));
        }
        if self.dirs_denied {
            out.push("pm2: /home could not be listed (permission denied)".to_owned());
        }
        if self.truncated_homes {
            out.push("pm2: /home has too many entries; some were not checked".to_owned());
        }
        out
    }
}

impl Collector for Pm2 {
    fn kind(&self) -> &'static str {
        "pm2"
    }

    fn run(&self) -> Result<Collection, CollectError> {
        self.root
            .validate()
            .map_err(|err| CollectError(format!("root filesystem: {err}")))?;
        let mut problems = Problems::default();
        // (owner, home directory). `/root` first, so on a name clash the administrator's entry
        // supplies the fields; a user can only ever add to the instance count of another's app.
        let mut homes: Vec<(String, String)> = vec![("root".to_owned(), "/root".to_owned())];
        match self.root.read_dir("/home", MAX_HOMES) {
            Ok(listing) => {
                problems.truncated_homes = listing.truncated;
                homes.extend(
                    listing
                        .names
                        .iter()
                        .map(|name| (clean(name, MAX_NAME_CHARS), format!("/home/{name}"))),
                );
            }
            Err(FsError::PermissionDenied) => problems.dirs_denied = true,
            Err(_) => {} // no /home (or not a directory): nothing to list
        }

        let mut apps: BTreeMap<String, Item> = BTreeMap::new();
        let mut refs: Vec<Item> = Vec::new();
        let (mut unparsed, mut over_cap) = (0usize, 0usize);
        for (owner, home) in &homes {
            let dir = format!("{home}/.pm2");
            let path = format!("{dir}/dump.pm2");
            // A user-writable home must not be able to point the collector at another file (for
            // example a root-only JSON file): links at `.pm2` and at the dump are refused.
            if self.root.read_link(&dir).is_ok() || self.root.read_link(&path).is_ok() {
                problems.symlinks += 1;
                continue;
            }
            let file = match self.root.read_to_string(&path, MAX_DUMP_BYTES) {
                Ok(file) => file,
                Err(FsError::NotFound) => continue,
                Err(FsError::PermissionDenied) => {
                    problems.denied += 1;
                    continue;
                }
                Err(FsError::InvalidPath(_)) => continue, // a home name that cannot be addressed
                Err(_) => {
                    problems.read_failed += 1;
                    continue;
                }
            };
            if file.truncated {
                problems.too_large += 1;
                continue;
            }
            let Ok(Value::Array(entries)) = serde_json::from_str::<Value>(&file.text) else {
                problems.invalid += 1;
                continue;
            };
            for entry in entries.iter().take(MAX_APPS_PER_DUMP) {
                match app_item(entry, owner) {
                    Some((item, found)) => {
                        // The first dump that names an app supplies its references too.
                        if !apps.contains_key(&item.key) {
                            unparsed += found.unparsed;
                            over_cap += found.over_cap;
                            refs.extend(found.items);
                        }
                        merge(&mut apps, item);
                    }
                    None => problems.bad_apps += 1,
                }
            }
        }
        let mut warnings = problems.warnings();
        warnings.extend(envref::warnings(unparsed, over_cap));
        let mut items: Vec<Item> = apps.into_values().collect();
        items.extend(refs);
        Ok(Collection { items, warnings })
    }
}

/// Adds `item`, or sums `instances` into the one already there.
fn merge(apps: &mut BTreeMap<String, Item>, item: Item) {
    match apps.get_mut(&item.key) {
        Some(first) => {
            let total = first
                .u64_field("instances")
                .unwrap_or(1)
                .saturating_add(item.u64_field("instances").unwrap_or(1));
            first.data.insert("instances".into(), Value::from(total));
        }
        None => {
            apps.insert(item.key.clone(), item);
        }
    }
}

/// One app object of the dump as an item, or `None` when it has no usable name.
fn app_item(app: &Value, owner: &str) -> Option<(Item, Extracted)> {
    let name = clean(app.get("name")?.as_str()?, MAX_NAME_CHARS);
    if name.trim().is_empty() {
        return None;
    }
    let exec = app.get("pm_exec_path").and_then(Value::as_str);
    let mut argv: Vec<&str> = exec.into_iter().collect();
    match app.get("args") {
        Some(Value::Array(list)) => {
            argv.extend(list.iter().filter_map(Value::as_str).take(MAX_ARGS));
        }
        Some(Value::String(line)) => {
            argv.extend(split_shell_words(line).into_iter().take(MAX_ARGS));
        }
        _ => {}
    }
    let cmd = if argv.is_empty() {
        Value::Null
    } else {
        Value::from(truncate_chars(
            &redact_command_args(argv.iter().copied()),
            MAX_CMD_CHARS,
        ))
    };
    let cwd = app
        .get("pm_cwd")
        .and_then(Value::as_str)
        .map_or(Value::Null, |c| Value::from(clean(c, MAX_CWD_CHARS)));

    let env = app.get("env").and_then(Value::as_object);
    let node_env = env
        .and_then(|e| e.get("NODE_ENV"))
        .and_then(Value::as_str)
        .map(|v| v.trim().to_ascii_lowercase());
    let mode = if node_env.as_deref() == Some("development") || is_dev_command(&argv) {
        "dev"
    } else if node_env.as_deref() == Some("production") {
        "prod"
    } else {
        "unknown"
    };
    // Names only: the values of `env` are never read into any item.
    let mut env_refs: Vec<String> = env
        .map(|e| {
            e.keys()
                .filter(|k| !k.is_empty())
                .map(|k| clean(k, MAX_ENV_NAME_CHARS))
                .collect()
        })
        .unwrap_or_default();
    env_refs.sort();
    env_refs.dedup();
    env_refs.truncate(MAX_ENV_NAMES);

    let key = format!("pm2:{name}");
    // Values are inspected only inside `extract`, which keeps host and port and drops the rest.
    let texts: Vec<(&str, String)> = env
        .into_iter()
        .flatten()
        .filter_map(|(k, v)| match v {
            Value::String(text) => Some((k.as_str(), text.clone())),
            Value::Number(n) => Some((k.as_str(), n.to_string())),
            _ => None,
        })
        .collect();
    let found = envref::extract(
        &key,
        texts.iter().map(|(k, v)| (*k, v.as_str())),
        false,
        SeenVia::Config,
    );

    let instances = app
        .get("instances")
        .and_then(Value::as_u64)
        .filter(|n| *n >= 1)
        .unwrap_or(1);

    let mut data = Map::new();
    data.insert("cmd".into(), cmd);
    data.insert("cwd".into(), cwd);
    data.insert("mode".into(), Value::from(mode));
    data.insert("supervised".into(), Value::Bool(true));
    data.insert(
        "env_refs".into(),
        Value::Array(env_refs.into_iter().map(Value::from).collect()),
    );
    data.insert("instances".into(), Value::from(instances));
    data.insert("saved_only".into(), Value::Bool(true));
    data.insert("owner".into(), Value::from(owner));
    let item = Item {
        kind: Kind::Process,
        key,
        data,
        seen_via: SeenVia::Config,
    };
    Some((item, found))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    /// A scratch tree under the system temp directory, removed on drop.
    struct Tree(PathBuf);

    impl Tree {
        fn new() -> Result<Self, std::io::Error> {
            let dir = std::env::temp_dir().join(format!(
                "collectors-pm2-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir)?;
            Ok(Self(dir))
        }

        fn write(&self, rel: &str, bytes: &[u8]) -> Result<(), std::io::Error> {
            let path = self.0.join(rel);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, bytes)
        }

        fn path(&self, rel: &str) -> std::path::PathBuf {
            self.0.join(rel)
        }

        fn run(&self) -> Result<Collection, CollectError> {
            Pm2::new(RootFs::new(&self.0)).run()
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn find<'a>(c: &'a Collection, key: &str) -> Result<&'a Item, String> {
        c.items
            .iter()
            .find(|i| i.key == key)
            .ok_or_else(|| format!("missing {key}"))
    }

    const NORMAL: &str = r#"[
      {"name":"web","pm_exec_path":"/srv/web/server.js","pm_cwd":"/srv/web",
       "args":["--port","3000","--db-password","hunter2"],"exec_mode":"cluster_mode",
       "instances":4,"env":{"NODE_ENV":"production","DATABASE_URL":"mysql://u:SECRETPW@h/db","PORT":"3000"}},
      {"name":"dev-ui","pm_exec_path":"/usr/bin/vite","pm_cwd":"/srv/ui","args":[],
       "env":{"NODE_ENV":"development"}},
      {"name":"worker","pm_exec_path":"/srv/w.js","env":{}}
    ]"#;

    #[test]
    fn reads_apps_from_a_dump() -> TestResult {
        let t = Tree::new()?;
        t.write("home/deploy/.pm2/dump.pm2", NORMAL.as_bytes())?;
        let c = t.run()?;
        assert!(c.warnings.is_empty(), "{:?}", c.warnings);
        let processes = c.items.iter().filter(|i| i.kind == Kind::Process).count();
        assert_eq!(processes, 3);
        let web = find(&c, "pm2:web")?;
        assert_eq!(web.seen_via, SeenVia::Config);
        assert_eq!(web.str_field("mode"), Some("prod"));
        assert_eq!(web.str_field("cwd"), Some("/srv/web"));
        assert_eq!(web.u64_field("instances"), Some(4));
        assert_eq!(web.data.get("supervised"), Some(&Value::Bool(true)));
        assert_eq!(web.data.get("saved_only"), Some(&Value::Bool(true)));
        let cmd = web.str_field("cmd").ok_or("cmd")?;
        assert!(cmd.starts_with("/srv/web/server.js --port 3000"), "{cmd}");
        assert_eq!(
            web.data.get("env_refs"),
            Some(&serde_json::json!(["DATABASE_URL", "NODE_ENV", "PORT"]))
        );
        assert_eq!(find(&c, "pm2:dev-ui")?.str_field("mode"), Some("dev"));
        let worker = find(&c, "pm2:worker")?;
        assert_eq!(worker.str_field("mode"), Some("unknown"));
        assert_eq!(worker.u64_field("instances"), Some(1));
        assert_eq!(worker.data.get("cwd"), Some(&Value::Null));
        Ok(())
    }

    #[test]
    fn env_references_become_env_ref_items() -> TestResult {
        let t = Tree::new()?;
        t.write(
            "root/.pm2/dump.pm2",
            br#"[{"name":"web","pm_exec_path":"/srv/web/s.js","env":{
              "REDIS_URL":"redis://:topsecret@127.0.0.1:6379/0",
              "DB_HOST":"127.0.0.1","DB_PORT":5432,
              "BROKEN_URL":"not a url at all, hunter2",
              "NODE_ENV":"production","PATH":"/usr/bin:/bin",
              "API":"https://api.example.test/v1?token=ABCDEF"}}]"#,
        )?;
        let c = t.run()?;
        let refs: Vec<&Item> = c.items.iter().filter(|i| i.kind == Kind::EnvRef).collect();
        let keys: Vec<&str> = refs.iter().map(|i| i.key.as_str()).collect();
        assert_eq!(
            keys,
            ["pm2:web:API", "pm2:web:DB_HOST", "pm2:web:REDIS_URL"]
        );
        let redis = find(&c, "pm2:web:REDIS_URL")?;
        assert_eq!(redis.seen_via, SeenVia::Config);
        assert_eq!(redis.str_field("app"), Some("pm2:web"));
        assert_eq!(redis.str_field("name"), Some("REDIS_URL"));
        assert_eq!(redis.str_field("target_host"), Some("127.0.0.1"));
        assert_eq!(redis.u64_field("target_port"), Some(6379));
        assert_eq!(redis.data.get("in_container"), Some(&Value::Bool(false)));
        assert_eq!(redis.data.get("resolves_to"), Some(&Value::Null));
        assert_eq!(
            find(&c, "pm2:web:DB_HOST")?.u64_field("target_port"),
            Some(5432)
        );
        assert_eq!(find(&c, "pm2:web:API")?.u64_field("target_port"), Some(443));
        assert_eq!(c.warnings.len(), 1, "{:?}", c.warnings);
        let all = format!("{:?}{:?}", c.items, c.warnings);
        for secret in ["topsecret", "hunter2", "ABCDEF", "/v1"] {
            assert!(!all.contains(secret), "{secret}");
        }
        Ok(())
    }

    #[test]
    fn secrets_never_reach_items() -> TestResult {
        let t = Tree::new()?;
        t.write("root/.pm2/dump.pm2", NORMAL.as_bytes())?;
        let c = t.run()?;
        let text = format!("{:?}{}", c.warnings, serde_json::to_string(&c.items)?);
        assert!(!text.contains("SECRETPW"), "{text}");
        assert!(!text.contains("hunter2"), "{text}");
        Ok(())
    }

    #[test]
    fn no_dump_is_empty_without_warning() -> TestResult {
        let t = Tree::new()?;
        t.write("home/alice/readme", b"x")?;
        let c = t.run()?;
        assert!(c.items.is_empty());
        assert!(c.warnings.is_empty());
        Ok(())
    }

    #[test]
    fn same_name_in_two_users_is_merged() -> TestResult {
        let t = Tree::new()?;
        t.write(
            "home/a/.pm2/dump.pm2",
            br#"[{"name":"web","pm_exec_path":"/a.js","instances":2}]"#,
        )?;
        t.write(
            "root/.pm2/dump.pm2",
            br#"[{"name":"web","pm_exec_path":"/b.js"},{"name":"only-root"}]"#,
        )?;
        let c = t.run()?;
        assert_eq!(c.items.len(), 2);
        let web = find(&c, "pm2:web")?;
        assert_eq!(web.u64_field("instances"), Some(3));
        // `/root` is read first, so the administrator's entry supplies the fields.
        assert_eq!(web.str_field("cmd"), Some("/b.js"));
        assert_eq!(web.str_field("owner"), Some("root"));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_dump_or_pm2_dir_is_skipped_and_never_read() -> TestResult {
        let t = Tree::new()?;
        t.write("secret.json", br#"[{"name":"LEAKED"}]"#)?;
        t.write("home/b/real/dump.pm2", br#"[{"name":"LEAKED2"}]"#)?;
        t.write("home/a/.pm2/.keep", b"")?;
        std::os::unix::fs::symlink("/secret.json", t.path("home/a/.pm2/dump.pm2"))?;
        std::os::unix::fs::symlink("/home/b/real", t.path("home/b/.pm2"))?;
        t.write("root/.pm2/dump.pm2", br#"[{"name":"ok"}]"#)?;
        let c = t.run()?;
        let names: Vec<_> = c.items.iter().map(|i| i.key.as_str()).collect();
        assert_eq!(names, ["pm2:ok"]);
        assert!(
            c.warnings.iter().any(|w| w.contains("2 dump file(s)")),
            "{:?}",
            c.warnings
        );
        assert!(!c.warnings.iter().any(|w| w.contains("LEAKED")));
        Ok(())
    }

    #[test]
    fn invalid_json_warns_once_without_content() -> TestResult {
        let t = Tree::new()?;
        t.write("home/a/.pm2/dump.pm2", b"{not json SECRETLEAK")?;
        t.write("home/b/.pm2/dump.pm2", br#"{"name":"object-not-array"}"#)?;
        t.write("root/.pm2/dump.pm2", br#"[{"name":"ok"}]"#)?;
        let c = t.run()?;
        assert_eq!(c.items.len(), 1);
        assert_eq!(c.warnings.len(), 1, "{:?}", c.warnings);
        assert!(c.warnings[0].contains("2 dump file(s)"));
        assert!(!c.warnings[0].contains("SECRETLEAK"));
        Ok(())
    }

    #[test]
    fn huge_dump_is_skipped_with_a_warning() -> TestResult {
        let t = Tree::new()?;
        let mut big = String::from(r#"[{"name":"x","args":[""#);
        big.push_str(&"a".repeat(MAX_DUMP_BYTES + 10));
        big.push_str(r#""]}]"#);
        t.write("root/.pm2/dump.pm2", big.as_bytes())?;
        let c = t.run()?;
        assert!(c.items.is_empty());
        assert_eq!(c.warnings.len(), 1);
        assert!(c.warnings[0].contains("larger than the read limit"));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_dump_warns_once() -> TestResult {
        use std::os::unix::fs::PermissionsExt;
        let t = Tree::new()?;
        t.write("home/a/.pm2/dump.pm2", br#"[{"name":"x"}]"#)?;
        let path = t.0.join("home/a/.pm2/dump.pm2");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000))?;
        if fs::read(&path).is_ok() {
            return Ok(()); // running as root: permissions do not apply
        }
        let c = t.run()?;
        assert!(c.items.is_empty());
        assert_eq!(c.warnings.len(), 1, "{:?}", c.warnings);
        assert!(c.warnings[0].contains("permission denied"));
        Ok(())
    }

    #[test]
    fn hostile_entries_are_cleaned_or_skipped() -> TestResult {
        let t = Tree::new()?;
        let long = "n".repeat(5000);
        let dump = format!(
            r#"[{{"name":"a\nb\u0000c"}},{{"name":"{long}","pm_cwd":"/x\ty"}},
                {{"name":123}},{{"name":""}},{{"pm_exec_path":"/no-name"}},"str",null,
                {{"name":"envy","env":{{"BAD\nNAME":"v","":"w"}}}}]"#
        );
        t.write("root/.pm2/dump.pm2", dump.as_bytes())?;
        let c = t.run()?;
        assert_eq!(c.items.len(), 3, "{:?}", c.items);
        assert!(find(&c, "pm2:a?b?c").is_ok());
        let longest = c.items.iter().find(|i| i.key.len() > 100).ok_or("long")?;
        assert_eq!(longest.key.chars().count(), 4 + MAX_NAME_CHARS);
        assert_eq!(longest.str_field("cwd"), Some("/x?y"));
        let envy = find(&c, "pm2:envy")?;
        assert_eq!(
            envy.data.get("env_refs"),
            Some(&serde_json::json!(["BAD?NAME"]))
        );
        assert_eq!(c.warnings.len(), 1);
        assert!(c.warnings[0].contains("without a usable name"));
        Ok(())
    }

    #[test]
    fn dev_servers_and_instances_edge_cases() -> TestResult {
        let t = Tree::new()?;
        t.write(
            "root/.pm2/dump.pm2",
            br#"[{"name":"a","pm_exec_path":"/n/nodemon","args":"app.js"},
                {"name":"b","instances":"max"},
                {"name":"c","instances":0,"env":{"NODE_ENV":"Production "}}]"#,
        )?;
        let c = t.run()?;
        assert_eq!(find(&c, "pm2:a")?.str_field("mode"), Some("dev"));
        assert_eq!(find(&c, "pm2:b")?.u64_field("instances"), Some(1));
        assert_eq!(find(&c, "pm2:c")?.u64_field("instances"), Some(1));
        assert_eq!(find(&c, "pm2:c")?.str_field("mode"), Some("prod"));
        Ok(())
    }
}
