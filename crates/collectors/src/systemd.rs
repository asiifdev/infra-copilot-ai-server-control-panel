//! Services and timers from systemd (kinds `process` and `cron`, seen via `systemd`).
//!
//! Runs two read-only queries through the [`CommandRunner`] (decision 0014): `Cmd::SystemctlServices`
//! and `Cmd::SystemctlTimers`. Nothing else is executed and no text from the machine ever becomes
//! an argument.
//!
//! * Services: one `process` item per unit that is `active` and `running` (oneshot units that
//!   finished are `active (exited)` and are skipped). Key `systemd:<unit without .service>`. Data
//!   (docs/design.md 4.1): `cmd` is the unit description (the real command line is not available
//!   from `list-units`), cleaned, redacted and bounded; `cwd` null; `mode` `"unknown"`;
//!   `supervised` true; `env_refs` empty; `instances` 1. Base-system units are skipped through the
//!   two explicit lists [`NOISE_PREFIXES`] and [`NOISE_NAMES`].
//! * Timers: one `cron` item per timer, key `system:root:timer:<timer unit>` (the cron key shape
//!   `<scope>:<owner>:<schedule>:<hint>`, docs/design.md 4.1), data `schedule` (the literal
//!   `"timer"`, the calendar expression is not in `list-timers`), `command_hint` (the activated
//!   unit), `owner` `"root"`, `source` `"timer"`.
//!
//! No systemd (container, macOS, missing binary) gives an empty collection without a warning. A
//! failed or timed out query gives one warning. Lines that cannot be parsed are counted in one
//! warning per query; their text is never copied.
//! Not covered: user-level (`--user`) units, transient units, timers' real calendar expressions.

use std::collections::BTreeMap;
use std::sync::Arc;

use core_domain::redact::redact;
use inventory::{CollectError, Collection, Collector, Item, Kind, SeenVia};
use serde_json::{Map, Value};

use crate::exec::{Cmd, CommandRunner, ExecError};
use crate::util::{MAX_CMD_CHARS, clean, truncate_chars};

/// Services whose name starts with one of these are base-system noise, not application services.
const NOISE_PREFIXES: [&str; 5] = [
    "systemd-",
    "user@",
    "user-runtime-dir@",
    "getty@",
    "serial-getty@",
];
/// Services with exactly one of these names are base-system noise.
const NOISE_NAMES: [&str; 6] = [
    "dbus",
    "dbus-broker",
    "polkit",
    "accounts-daemon",
    "udisks2",
    "upower",
];
const MAX_NAME_CHARS: usize = 128;
const MAX_LINES: usize = 10_000;
const SERVICE_SUFFIX: &str = ".service";
const TIMER_SUFFIX: &str = ".timer";

fn is_noise(name: &str) -> bool {
    NOISE_NAMES.contains(&name) || NOISE_PREFIXES.iter().any(|prefix| name.starts_with(prefix))
}

/// One parsed `list-units` row.
#[derive(Debug, PartialEq, Eq)]
struct ServiceRow<'a> {
    unit: &'a str,
    active: &'a str,
    sub: &'a str,
    description: &'a str,
}

/// Splits off the first whitespace separated token.
fn next_token(text: &str) -> Option<(&str, &str)> {
    let text = text.trim_start();
    if text.is_empty() {
        return None;
    }
    Some(match text.find(char::is_whitespace) {
        Some(index) => text.split_at(index),
        None => (text, ""),
    })
}

/// A leading status bullet (`●`, `*`, `x`-style markers are separate tokens) is dropped.
fn strip_bullet(line: &str) -> &str {
    let line = line.trim();
    for bullet in ["●", "○", "×", "*"] {
        if let Some(rest) = line.strip_prefix(bullet) {
            return rest.trim_start();
        }
    }
    line
}

/// `UNIT LOAD ACTIVE SUB DESCRIPTION`, the description being the rest of the line.
fn parse_service(line: &str) -> Option<ServiceRow<'_>> {
    let (unit, rest) = next_token(strip_bullet(line))?;
    let (_load, rest) = next_token(rest)?;
    let (active, rest) = next_token(rest)?;
    let (sub, rest) = next_token(rest)?;
    if !unit.ends_with(SERVICE_SUFFIX) || unit.len() == SERVICE_SUFFIX.len() {
        return None;
    }
    Some(ServiceRow {
        unit,
        active,
        sub,
        description: rest.trim(),
    })
}

/// `NEXT LEFT LAST PASSED UNIT ACTIVATES`: the dates hold spaces, so the last two tokens are read
/// from the end. Returns `(timer unit, activated unit)`.
fn parse_timer(line: &str) -> Option<(&str, &str)> {
    let mut tokens = line.split_whitespace().rev();
    let activates = tokens.next()?;
    let unit = tokens.next()?;
    // At least one more token must precede them (the date columns), and the unit must be a timer.
    tokens.next()?;
    if !unit.ends_with(TIMER_SUFFIX) || unit.len() == TIMER_SUFFIX.len() {
        return None;
    }
    Some((unit, activates))
}

fn process_item(name: &str, description: &str) -> Item {
    let cmd = truncate_chars(&redact(&clean(description, usize::MAX)).text, MAX_CMD_CHARS);
    let mut data = Map::new();
    data.insert("cmd".into(), Value::String(cmd));
    data.insert("cwd".into(), Value::Null);
    data.insert("mode".into(), Value::String("unknown".into()));
    data.insert("supervised".into(), Value::Bool(true));
    data.insert("env_refs".into(), Value::Array(Vec::new()));
    data.insert("instances".into(), Value::from(1_u64));
    Item {
        kind: Kind::Process,
        key: format!("systemd:{name}"),
        data,
        seen_via: SeenVia::Systemd,
    }
}

fn timer_item(unit: &str, activates: &str) -> Item {
    let hint = truncate_chars(&redact(&clean(activates, usize::MAX)).text, MAX_NAME_CHARS);
    let mut data = Map::new();
    data.insert("schedule".into(), Value::String("timer".into()));
    data.insert("command_hint".into(), Value::String(hint));
    data.insert("owner".into(), Value::String("root".into()));
    data.insert("source".into(), Value::String("timer".into()));
    Item {
        kind: Kind::Cron,
        key: format!("system:root:timer:{unit}"),
        data,
        seen_via: SeenVia::Systemd,
    }
}

/// Collector of systemd services and timers.
#[derive(Clone)]
pub struct Systemd {
    runner: Arc<dyn CommandRunner>,
}

impl std::fmt::Debug for Systemd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Systemd").finish_non_exhaustive()
    }
}

impl Systemd {
    /// A collector asking `runner` for the systemd listings.
    #[must_use]
    pub fn new(runner: Arc<dyn CommandRunner>) -> Self {
        Self { runner }
    }

    /// Runs `cmd`; `None` when there is nothing to parse (a warning was added if it is a failure).
    fn query(&self, cmd: &Cmd, label: &str, warnings: &mut Vec<String>) -> Option<String> {
        match self.runner.run(cmd) {
            Ok(text) => Some(text),
            // No systemd on this machine: not an error.
            Err(ExecError::NotAvailable) => None,
            Err(ExecError::TimedOut) => {
                warnings.push(format!("{label} timed out"));
                None
            }
            Err(ExecError::Failed(reason)) => {
                warnings.push(format!("{label} failed: {}", clean(&reason, 100)));
                None
            }
        }
    }
}

/// Lines to parse, capped; the flag tells whether lines were left out.
fn lines_of(text: &str) -> (impl Iterator<Item = &str>, bool) {
    let capped = text.lines().filter(|l| !l.trim().is_empty()).count() > MAX_LINES;
    (
        text.lines()
            .filter(|l| !l.trim().is_empty())
            .take(MAX_LINES),
        capped,
    )
}

impl Collector for Systemd {
    fn kind(&self) -> &'static str {
        "systemd"
    }

    fn run(&self) -> Result<Collection, CollectError> {
        let mut warnings = Vec::new();
        let mut items: BTreeMap<String, Item> = BTreeMap::new();

        if let Some(text) = self.query(
            &Cmd::SystemctlServices,
            "systemctl list-units",
            &mut warnings,
        ) {
            let (lines, capped) = lines_of(&text);
            let mut malformed = 0_usize;
            for line in lines {
                let Some(row) = parse_service(line) else {
                    malformed += 1;
                    continue;
                };
                if row.active != "active" || row.sub != "running" {
                    continue;
                }
                let name = row.unit.trim_end_matches(SERVICE_SUFFIX);
                if is_noise(name) {
                    continue;
                }
                let name = clean(name, MAX_NAME_CHARS);
                let item = process_item(&name, row.description);
                items.entry(item.key.clone()).or_insert(item);
            }
            if malformed > 0 {
                warnings.push(format!(
                    "{malformed} unparsable line(s) in systemctl list-units output"
                ));
            }
            if capped {
                warnings.push(format!(
                    "systemctl list-units output has more than {MAX_LINES} lines; the rest were not read"
                ));
            }
        }

        if let Some(text) = self.query(
            &Cmd::SystemctlTimers,
            "systemctl list-timers",
            &mut warnings,
        ) {
            let (lines, capped) = lines_of(&text);
            let mut malformed = 0_usize;
            for line in lines {
                // The footer of a listing ("12 timers listed.") is not a row.
                let Some((unit, activates)) = parse_timer(strip_bullet(line)) else {
                    malformed += 1;
                    continue;
                };
                let unit = clean(unit, MAX_NAME_CHARS);
                let item = timer_item(&unit, activates);
                items.entry(item.key.clone()).or_insert(item);
            }
            if malformed > 0 {
                warnings.push(format!(
                    "{malformed} unparsable line(s) in systemctl list-timers output"
                ));
            }
            if capped {
                warnings.push(format!(
                    "systemctl list-timers output has more than {MAX_LINES} lines; the rest were not read"
                ));
            }
        }

        Ok(Collection {
            items: items.into_values().collect(),
            warnings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::FakeRunner;

    const SERVICES: &str = "\
cron.service                  loaded active   running Regular Background Program Processing Daemon
dbus.service                  loaded active   running D-Bus System Message Bus
getty@tty1.service            loaded active   running Getty on tty1
nginx.service                 loaded active   running A high performance web server
ssh.service                   loaded active   running OpenBSD Secure Shell server
systemd-journald.service      loaded active   running Journal Service
user@1000.service             loaded active   running User Manager for UID 1000
apt-daily.service             loaded inactive dead    Daily apt download activities
cloud-init.service            loaded active   exited  Initial cloud-init job
ssh@.service                  not-found inactive dead ssh@.service
";

    const TIMERS: &str = "\
Mon 2026-11-23 00:00:00 UTC 3d left Mon 2026-11-16 00:00:00 UTC 3d ago logrotate.timer logrotate.service
n/a                         n/a    Thu 2026-11-19 06:00:00 UTC 1h ago backup.timer backup-run.service
";

    fn collect(runner: FakeRunner) -> Collection {
        let collector = Systemd::new(Arc::new(runner));
        assert_eq!(collector.kind(), "systemd");
        collector.run().unwrap_or_default()
    }

    fn full() -> FakeRunner {
        FakeRunner::new()
            .with(Cmd::SystemctlServices, Ok(SERVICES.into()))
            .with(Cmd::SystemctlTimers, Ok(TIMERS.into()))
    }

    #[test]
    fn running_services_become_process_items() {
        let found = collect(full());
        let keys: Vec<&str> = found
            .items
            .iter()
            .filter(|i| i.kind == Kind::Process)
            .map(|i| i.key.as_str())
            .collect();
        // cron, nginx and ssh run; dead and exited units and the noise units are skipped.
        assert_eq!(keys, ["systemd:cron", "systemd:nginx", "systemd:ssh"]);
        // `ssh@.service` is `not-found`, a template: skipped as not running, not a malformed line.
        assert!(found.warnings.is_empty(), "{:?}", found.warnings);
        let nginx = found.items.iter().find(|i| i.key == "systemd:nginx");
        let nginx = nginx.map(|i| (i.seen_via, i.str_field("cmd"), i.u64_field("instances")));
        assert_eq!(
            nginx,
            Some((
                SeenVia::Systemd,
                Some("A high performance web server"),
                Some(1)
            ))
        );
    }

    #[test]
    fn process_data_has_the_documented_shape() {
        let found = collect(full());
        let item = found.items.iter().find(|i| i.key == "systemd:ssh");
        let data = item.map(|i| &i.data);
        let get = |name: &str| data.and_then(|d| d.get(name));
        assert_eq!(get("cwd"), Some(&Value::Null));
        assert_eq!(get("mode"), Some(&Value::String("unknown".into())));
        assert_eq!(get("supervised"), Some(&Value::Bool(true)));
        assert_eq!(get("env_refs"), Some(&Value::Array(Vec::new())));
    }

    #[test]
    fn noise_is_filtered_only_by_the_explicit_lists() {
        for name in [
            "dbus",
            "dbus-broker",
            "systemd-logind",
            "user@0",
            "getty@tty2",
        ] {
            assert!(is_noise(name), "{name}");
        }
        for name in ["dbus-app", "mysystemd-x", "nginx", "getty", "user"] {
            assert!(!is_noise(name), "{name}");
        }
    }

    #[test]
    fn timers_become_cron_items() {
        let found = collect(full());
        let timers: Vec<(&str, Option<&str>)> = found
            .items
            .iter()
            .filter(|i| i.kind == Kind::Cron)
            .map(|i| (i.key.as_str(), i.str_field("command_hint")))
            .collect();
        assert_eq!(
            timers,
            [
                ("system:root:timer:backup.timer", Some("backup-run.service")),
                (
                    "system:root:timer:logrotate.timer",
                    Some("logrotate.service")
                ),
            ]
        );
        let item = found.items.iter().find(|i| i.kind == Kind::Cron);
        assert_eq!(item.and_then(|i| i.str_field("schedule")), Some("timer"));
        assert_eq!(item.and_then(|i| i.str_field("owner")), Some("root"));
        assert_eq!(item.and_then(|i| i.str_field("source")), Some("timer"));
        assert_eq!(item.map(|i| i.seen_via), Some(SeenVia::Systemd));
    }

    #[test]
    fn not_available_is_empty_without_a_warning() {
        let found = collect(FakeRunner::new());
        assert!(found.items.is_empty());
        assert!(found.warnings.is_empty());
    }

    #[test]
    fn failure_and_timeout_give_one_warning_each() {
        let runner = FakeRunner::new()
            .with(
                Cmd::SystemctlServices,
                Err(ExecError::Failed("exit status Some(1)".into())),
            )
            .with(Cmd::SystemctlTimers, Err(ExecError::TimedOut));
        let found = collect(runner);
        assert!(found.items.is_empty());
        assert_eq!(found.warnings.len(), 2, "{:?}", found.warnings);
        assert!(found.warnings[0].starts_with("systemctl list-units failed"));
        assert!(found.warnings[1].contains("timed out"));
    }

    #[test]
    fn a_failed_services_query_still_reads_timers() {
        let runner = FakeRunner::new()
            .with(Cmd::SystemctlServices, Err(ExecError::TimedOut))
            .with(Cmd::SystemctlTimers, Ok(TIMERS.into()));
        let found = collect(runner);
        assert_eq!(found.items.len(), 2);
        assert_eq!(found.warnings.len(), 1);
    }

    #[test]
    fn malformed_lines_are_skipped_and_counted_once() {
        let services = "\
nginx.service loaded active running Web
garbage
two tokens
notaunit loaded active running Not a service
.service loaded active running Empty name
";
        let timers = "\
12 timers listed.
only three tokens
a.timer
";
        let runner = FakeRunner::new()
            .with(Cmd::SystemctlServices, Ok(services.into()))
            .with(Cmd::SystemctlTimers, Ok(timers.into()));
        let found = collect(runner);
        assert_eq!(found.items.len(), 1);
        assert_eq!(found.warnings.len(), 2, "{:?}", found.warnings);
        assert!(found.warnings[0].starts_with("4 unparsable"));
        assert!(found.warnings[1].starts_with("3 unparsable"));
        // The text of a bad line is never copied into a warning.
        assert!(!found.warnings.iter().any(|w| w.contains("garbage")));
    }

    #[test]
    fn failed_unit_with_a_bullet_still_parses() {
        let row = parse_service("● broken.service loaded failed failed Broken thing");
        assert_eq!(
            row,
            Some(ServiceRow {
                unit: "broken.service",
                active: "failed",
                sub: "failed",
                description: "Broken thing",
            })
        );
    }

    #[test]
    fn hostile_text_is_neutralised() {
        let long_name = "a".repeat(5_000);
        let long_desc = "d".repeat(5_000);
        let services = format!(
            "evil\u{1b}[31m.service loaded active running Desc\u{7}with\u{0}ctl TOKEN=abcdef123456 password=hunter2\n\
             {long_name}.service loaded active running {long_desc}\n\
             \u{202e}rtl.service loaded active running \u{1b}]0;title\u{7}\n"
        );
        let timers = format!("n/a n/a n/a n/a {long_name}.timer evil\u{1b}[0m.service\n");
        let runner = FakeRunner::new()
            .with(Cmd::SystemctlServices, Ok(services))
            .with(Cmd::SystemctlTimers, Ok(timers));
        let found = collect(runner);
        assert_eq!(found.items.len(), 4);
        for item in &found.items {
            assert!(!item.key.chars().any(char::is_control), "{:?}", item.key);
            assert!(item.key.chars().count() <= 150);
            for value in item.data.values() {
                if let Some(text) = value.as_str() {
                    assert!(!text.chars().any(char::is_control), "{text:?}");
                    assert!(text.chars().count() <= MAX_CMD_CHARS);
                }
            }
        }
        let all: String = found
            .items
            .iter()
            .filter_map(|i| i.str_field("cmd"))
            .collect();
        assert!(!all.contains("hunter2"), "{all}");
        assert!(!all.contains("abcdef123456"), "{all}");
    }

    #[test]
    fn huge_output_is_capped_with_one_warning() {
        let mut text = String::new();
        for n in 0..(MAX_LINES + 5) {
            text.push_str(&format!("app{n}.service loaded active running App {n}\n"));
        }
        let runner = FakeRunner::new().with(Cmd::SystemctlServices, Ok(text));
        let found = collect(runner);
        assert_eq!(found.items.len(), MAX_LINES);
        assert_eq!(found.warnings.len(), 1);
    }
}
