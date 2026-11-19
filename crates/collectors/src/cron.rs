//! Scheduled jobs from crontab files (kind `cron`, seen via `config`).
//!
//! Read: `/etc/crontab` and `/etc/cron.d/*` (system tables, with a user column), and
//! `/var/spool/cron/crontabs/<user>` and `/var/spool/cron/<user>` (user tables, no user column;
//! the owner is the file name). Names starting with `.` or ending with `~` in the directories are
//! skipped, as cron does.
//!
//! Item key `<scope>:<owner>:<schedule>:<hint>` with scope `system` or `user`, the schedule with
//! single spaces, and the hint a short label of what the command runs (see `command_hint`). Data (docs/design.md 4.1): `schedule`,
//! `command_hint` (the full command, redacted, at most 512 characters), `owner`, `source` (`"crontab"`), plus `occurrences` (how
//! many identical lines were merged). Comments, blank lines and `NAME=value` lines are ignored;
//! `@reboot`, `@daily` and the other `@` shortcuts and five-field schedules are understood.
//!
//! Lines that fit neither form are counted in a warning (their text is never copied, it might hold
//! a secret). A missing location is not a warning on its own (hosts differ in which of them exist);
//! it is when none of the known locations exists at all, and unreadable locations always warn.
//! Not covered yet: systemd timers, anacron, `cron.daily` scripts, panel-managed jobs.

use std::collections::BTreeMap;

use core_domain::redact::redact;
use inventory::{CollectError, Collection, Collector, Item, Kind, SeenVia};
use serde_json::{Map, Value};

use crate::rootfs::{FsError, RootFs};
use crate::util::{
    MAX_CMD_CHARS, clean, read_warning, redact_command_args, split_shell_words, truncate_chars,
};

const SYSTEM_TAB: &str = "/etc/crontab";
const SYSTEM_DIR: &str = "/etc/cron.d";
const USER_DIRS: [&str; 2] = ["/var/spool/cron/crontabs", "/var/spool/cron"];
const MAX_FILE_BYTES: usize = 1024 * 1024;
const MAX_DIR_ENTRIES: usize = 4_096;
const MAX_JOBS: usize = 10_000;
const SPECIALS: [&str; 8] = [
    "@reboot",
    "@yearly",
    "@annually",
    "@monthly",
    "@weekly",
    "@daily",
    "@midnight",
    "@hourly",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope {
    System,
    User,
}

impl Scope {
    const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Job {
    pub(crate) schedule: String,
    /// Present for system tables only.
    pub(crate) user: Option<String>,
    pub(crate) command: String,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ParsedTab {
    pub(crate) jobs: Vec<Job>,
    pub(crate) malformed: usize,
}

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

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `NAME=value` (optionally with spaces around `=`).
fn is_assignment(line: &str) -> bool {
    line.split_once('=')
        .is_some_and(|(name, _)| is_identifier(name.trim_end()))
}

/// Minute, hour, day-of-month hold digits and `*,/-?`; month and weekday may also hold names.
fn valid_field(field: &str, index: usize) -> bool {
    let allows_names = index >= 3;
    !field.is_empty()
        && field.chars().all(|c| {
            c.is_ascii_digit()
                || matches!(c, '*' | ',' | '/' | '-' | '?')
                || (allows_names && c.is_ascii_alphabetic())
        })
}

/// Parses one crontab's text. Never panics.
pub(crate) fn parse_crontab(text: &str, scope: Scope) -> ParsedTab {
    let mut parsed = ParsedTab::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || is_assignment(line) {
            continue;
        }
        match parse_job(line, scope) {
            Some(job) => parsed.jobs.push(job),
            None => parsed.malformed += 1,
        }
    }
    parsed
}

fn parse_job(line: &str, scope: Scope) -> Option<Job> {
    let (first, mut rest) = next_token(line)?;
    let schedule = if first.starts_with('@') {
        let lower = first.to_ascii_lowercase();
        if !SPECIALS.contains(&lower.as_str()) {
            return None;
        }
        lower
    } else {
        let mut fields = vec![first];
        for _ in 0..4 {
            let (field, tail) = next_token(rest)?;
            fields.push(field);
            rest = tail;
        }
        if !fields
            .iter()
            .enumerate()
            .all(|(index, field)| valid_field(field, index))
        {
            return None;
        }
        fields.join(" ")
    };
    let (user, rest) = match scope {
        Scope::System => {
            let (user, tail) = next_token(rest)?;
            (Some(user.to_owned()), tail)
        }
        Scope::User => (None, rest),
    };
    let command = rest.trim();
    if command.is_empty() {
        return None;
    }
    Some(Job {
        schedule,
        user,
        command: command.to_owned(),
    })
}

const INTERPRETERS: [&str; 10] = [
    "php", "python", "python3", "bash", "sh", "dash", "zsh", "node", "perl", "ruby",
];
const SHELLS: [&str; 4] = ["bash", "sh", "dash", "zsh"];
/// Interpreter flags that take a separate argument we do not want as the hint.
const INTERPRETER_FLAGS_WITH_ARG: [&str; 9] =
    ["-d", "-W", "-X", "-I", "-r", "-e", "-E", "-c", "-f"];
const MAX_HINT_DEPTH: usize = 3;
const MAX_HINT_CHARS: usize = 100;

fn unquote(word: &str) -> &str {
    let word = word.trim_end_matches(';');
    for q in ['"', '\''] {
        if let Some(inner) = word.strip_prefix(q) {
            return inner.strip_suffix(q).unwrap_or(inner);
        }
    }
    word
}

fn basename(word: &str) -> &str {
    let base = word.rsplit('/').next().unwrap_or(word);
    if base.is_empty() { word } else { base }
}

/// Interpreter name without a version suffix (`python3.11` -> `python`, `php8.2` -> `php`).
fn interpreter_name(base: &str) -> Option<&str> {
    let name = base.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    INTERPRETERS.contains(&name).then_some(name)
}

/// Flags (with their argument counts) of the wrappers skipped in front of the real command.
/// Returns `(flags taking an argument, positional arguments to skip)`.
fn wrapper(base: &str) -> Option<(&'static [&'static str], usize)> {
    Some(match base {
        "env" => (&["-u", "-C", "-S"], 0),
        "nice" => (&["-n"], 0),
        "ionice" => (&["-c", "-n", "-p"], 0),
        "sudo" => (&["-u", "-g", "-h", "-p", "-C", "-D", "-R", "-T", "-U"], 0),
        "flock" => (&["-w", "-E"], 1),
        "timeout" => (&["-s", "-k"], 1),
        _ => return None,
    })
}

/// Index of the real command in `words`: skips `NAME=value` words, `cd <dir> &&` / `cd <dir> ;`
/// prefixes and the wrappers `env`, `nice`, `ionice`, `sudo`, `flock <file>`, `timeout <n>`.
fn command_start(words: &[&str]) -> usize {
    let mut i = 0;
    while let Some(word) = words.get(i).copied() {
        if is_assignment(word) {
            i += 1;
        } else if word == "cd" {
            // `cd <dir>` is only a prefix when a separator follows (`&&` or `;`).
            let dir = words.get(i + 1).copied().unwrap_or("");
            if dir.ends_with(';') {
                i += 2;
            } else if matches!(words.get(i + 2).copied(), Some("&&" | ";")) {
                i += 3;
            } else {
                break;
            }
        } else if let Some((flags_with_arg, positional)) = wrapper(basename(word)) {
            i += 1;
            while let Some(flag) = words.get(i).copied() {
                if flags_with_arg.contains(&flag) {
                    i += 2;
                } else if flag.starts_with('-') || is_assignment(flag) {
                    i += 1;
                } else {
                    break;
                }
            }
            i += positional;
        } else {
            break;
        }
    }
    i.min(words.len())
}

fn hint_word(command: &str, depth: usize) -> Option<String> {
    let words = split_shell_words(command);
    let rest = words.get(command_start(&words)..)?;
    let first = *rest.first()?;
    let base = basename(first);
    let Some(interpreter) = interpreter_name(base) else {
        return Some(base.to_owned());
    };
    let mut i = 1;
    while let Some(arg) = rest.get(i).copied() {
        if SHELLS.contains(&interpreter) && arg == "-c" {
            // `bash -c 'cd /x && php /srv/job.php'`: the hint comes from the inner command.
            if let Some(script) = rest.get(i + 1)
                && depth < MAX_HINT_DEPTH
                && let Some(inner) = hint_word(unquote(script), depth + 1)
            {
                return Some(inner);
            }
            break;
        }
        if arg == "-m" {
            // `python -m http.server`: the module names the job.
            if let Some(module) = rest.get(i + 1) {
                return Some(unquote(module).to_owned());
            }
            break;
        }
        if INTERPRETER_FLAGS_WITH_ARG.contains(&arg) {
            i += 2;
        } else if arg.starts_with('-') {
            i += 1;
        } else {
            return Some(basename(unquote(arg)).to_owned());
        }
    }
    Some(interpreter.to_owned())
}

/// A short, stable label for what a cron command runs, used in the item key.
///
/// Skips leading `NAME=value` words, `cd <dir> &&` / `cd <dir> ;`, and the wrappers `env`,
/// `nice`, `ionice`, `sudo`, `flock <file>` and `timeout <n>`. The hint is the basename of the
/// first remaining word; when that is an interpreter (`php`, `python`, `python3`, `bash`, `sh`,
/// `dash`, `zsh`, `node`, `perl`, `ruby`) it is the basename of the first non-flag argument
/// instead (`php /srv/app/artisan schedule:run` -> `artisan`; `bash -c '...'` uses the inner
/// command). Redaction is applied to the hint.
///
/// Limit: jobs with the same schedule and the same script but different arguments
/// (`artisan queue:work` and `artisan schedule:run`) still get one key and are merged, counted in
/// `occurrences`; PAR-008 therefore cannot tell them apart.
pub(crate) fn command_hint(command: &str) -> String {
    match hint_word(command, 0) {
        Some(hint) if !hint.is_empty() => clean(&redact(&hint).text, MAX_HINT_CHARS),
        _ => "unknown".to_owned(),
    }
}

struct Merged {
    schedule: String,
    command: String,
    owner: String,
    occurrences: u64,
}

/// Collector of cron jobs.
#[derive(Debug, Clone)]
pub struct Cron {
    root: RootFs,
}

impl Cron {
    /// A collector reading from `root`.
    #[must_use]
    pub fn new(root: RootFs) -> Self {
        Self { root }
    }
}

struct Run<'a> {
    root: &'a RootFs,
    jobs: BTreeMap<String, Merged>,
    warnings: Vec<String>,
    missing: Vec<&'static str>,
    any_location: bool,
    capped: bool,
}

impl Run<'_> {
    fn add_file(&mut self, path: &str, scope: Scope, file_owner: &str) {
        let file = match self.root.read_to_string(path, MAX_FILE_BYTES) {
            Ok(file) => file,
            // Vanished, or a subdirectory: not a table.
            Err(FsError::NotFound | FsError::WrongType) => return,
            Err(err) => {
                self.warnings.push(read_warning(path, &err));
                return;
            }
        };
        if file.truncated {
            self.warnings.push(format!(
                "{path} is larger than {MAX_FILE_BYTES} bytes; the rest was not read"
            ));
        }
        let parsed = parse_crontab(&file.text, scope);
        if parsed.malformed > 0 {
            self.warnings
                .push(format!("{} unparsable line(s) in {path}", parsed.malformed));
        }
        for job in parsed.jobs {
            let owner = clean(job.user.as_deref().unwrap_or(file_owner), 64);
            let hint = command_hint(&job.command);
            let schedule = clean(&job.schedule, 100);
            let key = format!("{}:{owner}:{schedule}:{hint}", scope.as_str());
            if let Some(existing) = self.jobs.get_mut(&key) {
                existing.occurrences += 1;
            } else if self.jobs.len() >= MAX_JOBS {
                self.capped = true;
            } else {
                let command = truncate_chars(
                    &redact_command_args(split_shell_words(&job.command).into_iter()),
                    MAX_CMD_CHARS,
                );
                self.jobs.insert(
                    key,
                    Merged {
                        schedule,
                        command,
                        owner,
                        occurrences: 1,
                    },
                );
            }
        }
    }

    fn add_dir(&mut self, dir: &'static str, scope: Scope) {
        let listing = match self.root.read_dir(dir, MAX_DIR_ENTRIES) {
            Ok(listing) => listing,
            Err(FsError::NotFound) => {
                self.missing.push(dir);
                return;
            }
            Err(err) => {
                self.any_location = true;
                self.warnings.push(read_warning(dir, &err));
                return;
            }
        };
        self.any_location = true;
        if listing.truncated {
            self.warnings.push(format!(
                "{dir} has more than {MAX_DIR_ENTRIES} entries; the rest were not read"
            ));
        }
        if listing.non_utf8 > 0 {
            self.warnings.push(format!(
                "{} file(s) in {dir} have non-UTF-8 names and were not read",
                listing.non_utf8
            ));
        }
        for name in listing.names {
            if name.starts_with('.') || name.ends_with('~') {
                continue;
            }
            self.add_file(&format!("{dir}/{name}"), scope, &name);
        }
    }
}

impl Collector for Cron {
    fn kind(&self) -> &'static str {
        "cron"
    }

    fn run(&self) -> Result<Collection, CollectError> {
        self.root
            .validate()
            .map_err(|err| CollectError(format!("root filesystem: {err}")))?;
        let mut run = Run {
            root: &self.root,
            jobs: BTreeMap::new(),
            warnings: Vec::new(),
            missing: Vec::new(),
            any_location: false,
            capped: false,
        };

        match self.root.read_to_string(SYSTEM_TAB, MAX_FILE_BYTES) {
            Err(FsError::NotFound) => run.missing.push(SYSTEM_TAB),
            Err(err) => {
                run.any_location = true;
                run.warnings.push(read_warning(SYSTEM_TAB, &err));
            }
            Ok(_) => {
                run.any_location = true;
                run.add_file(SYSTEM_TAB, Scope::System, "root");
            }
        }
        run.add_dir(SYSTEM_DIR, Scope::System);
        for dir in USER_DIRS {
            run.add_dir(dir, Scope::User);
        }

        if !run.any_location {
            run.warnings.push(format!(
                "no cron configuration found (checked {}); cron may not be installed",
                run.missing.join(", ")
            ));
        }
        if run.capped {
            run.warnings.push(format!(
                "more than {MAX_JOBS} cron jobs; the rest were not read"
            ));
        }

        let items = run
            .jobs
            .into_iter()
            .map(|(key, merged)| {
                let mut data = Map::new();
                data.insert("schedule".into(), Value::from(merged.schedule));
                data.insert("command_hint".into(), Value::from(merged.command));
                data.insert("owner".into(), Value::from(merged.owner));
                data.insert("source".into(), Value::from("crontab"));
                data.insert("occurrences".into(), Value::from(merged.occurrences));
                Item {
                    kind: Kind::Cron,
                    key,
                    data,
                    seen_via: SeenVia::Config,
                }
            })
            .collect();
        Ok(Collection {
            items,
            warnings: run.warnings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_schedules_and_skips_noise() {
        let text = "# c\n\nSHELL=/bin/sh\nMAILTO = a@example.test\n\
                    */5  *\t* * 1-5 root /bin/a\n@Daily root /bin/b\n@weird root /bin/c\n\
                    this is not a cron line\n0 3 * * *\n";
        let parsed = parse_crontab(text, Scope::System);
        assert_eq!(parsed.jobs.len(), 2);
        assert_eq!(
            parsed.jobs.first().map(|j| j.schedule.as_str()),
            Some("*/5 * * * 1-5")
        );
        assert_eq!(
            parsed.jobs.get(1).map(|j| j.schedule.as_str()),
            Some("@daily")
        );
        assert_eq!(parsed.malformed, 3);
    }

    #[test]
    fn user_tables_have_no_user_column() {
        let parsed = parse_crontab("0 3 * * * /usr/bin/backup --now\n", Scope::User);
        let job = parsed.jobs.first();
        assert_eq!(job.map(|j| j.user.clone()), Some(None));
        assert_eq!(
            job.map(|j| j.command.as_str()),
            Some("/usr/bin/backup --now")
        );
    }

    #[test]
    fn hint_is_basename_of_first_word() {
        assert_eq!(command_hint("/usr/local/bin/backup --x"), "backup");
        assert_eq!(command_hint("FOO=1 BAR=2 /opt/run.sh"), "run.sh");
        assert_eq!(command_hint("FOO=1"), "unknown");
        assert_eq!(command_hint("/"), "/");
    }
}
