//! The `infra` command line (decisions 0013 and 0026).
//!
//! * `infra collect` reads the machine **read-only** and prints an inventory as JSON
//!   (`--save --node N` also stores it in the local database).
//! * `infra compare SOURCE TARGET` compares two inventories and prints the parity report
//!   (`--save --source-node A --target-node B` compares the stored ones and stores the result).
//! * `infra plan draft|list|show|gates` and `infra transport add|list|show|remove|check` (decision
//!   0028): plans are only ever drafts, and `transport check` is a dry run unless `--run` is given.
//! * `infra serve` starts the local web interface on loopback (decision 0030).
//! * `infra init`, `node add`, `provider add|list|remove`, `ask` and `memory ...` work on the
//!   local database (`--db PATH`, `NODIVARA_DB`, or a per-user data directory; see `local`).
//!
//! Every message goes through the i18n catalog; nothing user-facing is written in code. Exit codes:
//! `0` ok, `1` the report contains a critical finding, `2` usage, configuration or I/O error,
//! `3` the assistant or its provider failed.

mod args;
mod ask;
mod env;
mod local;
mod memory;
mod plans;
mod provider;
mod saved;
mod serve;
mod setup;
mod time;
mod transports;

pub use env::Env;

use collectors::{CommandRunner, FakeRunner, HostRunner, RootFs, collect_inventory_with};
use i18n::{Catalog, Locale, MessageKey};
use inventory::{Inventory, Kind, Platform, Severity, Status};
use parity::{compare, render};
use std::ffi::OsString;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;

use args::{Globals, Parsed, parse, split_globals};

pub(crate) const USAGE_INIT: MessageKey = MessageKey::new("cli.usage.init");
pub(crate) const USAGE_NODE: MessageKey = MessageKey::new("cli.usage.node");
pub(crate) const USAGE_PROVIDER: MessageKey = MessageKey::new("cli.usage.provider");
pub(crate) const USAGE_ASK: MessageKey = MessageKey::new("cli.usage.ask");
pub(crate) const USAGE_MEMORY: MessageKey = MessageKey::new("cli.usage.memory");
pub(crate) const USAGE_PLAN: MessageKey = MessageKey::new("cli.usage.plan");
pub(crate) const USAGE_TRANSPORT: MessageKey = MessageKey::new("cli.usage.transport");
pub(crate) const USAGE_SERVE: MessageKey = MessageKey::new("cli.usage.serve");
const USAGE_COLLECT: MessageKey = MessageKey::new("cli.usage.collect");
const USAGE_COMPARE: MessageKey = MessageKey::new("cli.usage.compare");
const ERR_USAGE: MessageKey = MessageKey::new("cli.error.usage");
const ERR_READ: MessageKey = MessageKey::new("cli.error.read");
const ERR_INVENTORY: MessageKey = MessageKey::new("cli.error.inventory");
const ERR_COLLECT: MessageKey = MessageKey::new("cli.error.collect");
const ERR_WRITE: MessageKey = MessageKey::new("cli.error.write");
const REPORT_NONE: MessageKey = MessageKey::new("cli.report.none");
const REPORT_COUNT: MessageKey = MessageKey::new("cli.report.count");
const REPORT_HEAD: MessageKey = MessageKey::new("cli.report.head");
const REPORT_SUBJECT: MessageKey = MessageKey::new("cli.report.subject");
const REPORT_ADVICE: MessageKey = MessageKey::new("cli.report.advice");
const REASON_NOT_FILE: MessageKey = MessageKey::new("cli.reason.notfile");
const REASON_TOO_LARGE: MessageKey = MessageKey::new("cli.reason.toolarge");
const KIND_LISTENER: MessageKey = MessageKey::new("cli.kind.listener");
const KIND_PROCESS: MessageKey = MessageKey::new("cli.kind.process");
const KIND_CRON: MessageKey = MessageKey::new("cli.kind.cron");
const KIND_ENV_REF: MessageKey = MessageKey::new("cli.kind.env_ref");
const REPORT_PARTIAL: MessageKey = MessageKey::new("cli.report.partial");
const REPORT_LIMITS: MessageKey = MessageKey::new("cli.report.limits");

/// Replaces control characters (terminal escapes, newlines, NUL) in text that comes from an
/// inventory, which is untrusted input: it could repaint the terminal or forge report lines.
fn sanitize(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { '\u{FFFD}' } else { c })
        .collect()
}

/// Largest inventory file accepted, so a wrong path cannot exhaust memory.
const MAX_INVENTORY_BYTES: u64 = 64 * 1024 * 1024;

pub(crate) const EXIT_OK: u8 = 0;
const EXIT_CRITICAL: u8 = 1;
const EXIT_ERROR: u8 = 2;

/// Converts raw process arguments to strings without ever panicking: invalid UTF-8 is replaced
/// with U+FFFD (such an argument can only end up as a usage error or a file-not-found).
#[must_use]
pub fn args_from_os(args: impl IntoIterator<Item = OsString>) -> Vec<String> {
    args.into_iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

/// Why an inventory file could not be read.
#[derive(Debug)]
enum ReadError {
    /// Not a regular file (a FIFO, a device, a directory): its length says nothing about its size.
    NotFile,
    /// More than the allowed number of bytes.
    TooLarge,
    Io(std::io::Error),
}

/// Reads a regular file of at most `max` bytes. The limit is enforced while reading, not from the
/// metadata length, which is 0 for pipes, devices and `/proc` files.
fn read_limited(path: &Path, max: u64) -> Result<String, ReadError> {
    // Checked before opening: opening a FIFO would block until a writer appears.
    if !std::fs::metadata(path).map_err(ReadError::Io)?.is_file() {
        return Err(ReadError::NotFile);
    }
    let file = File::open(path).map_err(ReadError::Io)?;
    if !file.metadata().map_err(ReadError::Io)?.is_file() {
        return Err(ReadError::NotFile);
    }
    let mut bytes = Vec::new();
    file.take(max.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(ReadError::Io)?;
    if bytes.len() as u64 > max {
        return Err(ReadError::TooLarge);
    }
    String::from_utf8(bytes)
        .map_err(|e| ReadError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))
}

/// Runs the command line with the process environment and standard input. `lang_env` is the
/// `LANG` value (used when `--lang` is absent).
pub fn run(
    args: &[String],
    lang_env: Option<&str>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    run_with(
        args,
        lang_env,
        &Env::process(),
        &mut std::io::stdin().lock(),
        out,
        err,
    )
}

/// Like [`run`], with an explicit environment and standard input (tests).
pub fn run_with(
    args: &[String],
    lang_env: Option<&str>,
    env: &Env,
    stdin: &mut dyn Read,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    let catalog = match Catalog::builtin() {
        Ok(catalog) => catalog,
        // The catalog is embedded and covered by tests; this is a build defect, not user input.
        Err(error) => {
            let _ = writeln!(err, "{error:?}");
            return EXIT_ERROR;
        }
    };
    let mut ui = Ui {
        catalog,
        locale: lang_env.map_or(Locale::En, Locale::parse_or_default),
        env,
        stdin,
        out,
        err,
    };
    ui.dispatch(args)
}

pub(crate) struct Ui<'a> {
    catalog: Catalog,
    pub(crate) locale: Locale,
    pub(crate) env: &'a Env,
    pub(crate) stdin: &'a mut dyn Read,
    pub(crate) out: &'a mut dyn Write,
    pub(crate) err: &'a mut dyn Write,
}

impl Ui<'_> {
    /// Looks up `key`; every argument value is sanitized because it may come from an inventory
    /// or from a file system error message.
    pub(crate) fn tr(&self, key: MessageKey, args: &[(&str, &str)]) -> String {
        let clean: Vec<(&str, String)> = args.iter().map(|(n, v)| (*n, sanitize(v))).collect();
        let pairs: Vec<(&str, &str)> = clean.iter().map(|(n, v)| (*n, v.as_str())).collect();
        self.catalog.get(self.locale, key, &pairs).text
    }

    pub(crate) fn fail(&mut self, key: MessageKey, args: &[(&str, &str)]) -> u8 {
        self.fail_code(key, args, EXIT_ERROR)
    }

    /// Writes the message to stderr and returns `code`.
    pub(crate) fn fail_code(&mut self, key: MessageKey, args: &[(&str, &str)], code: u8) -> u8 {
        let text = self.tr(key, args);
        let _ = writeln!(self.err, "{text}");
        code
    }

    /// A line on stderr that is not a failure (progress, notes).
    pub(crate) fn note_err(&mut self, text: &str) {
        let _ = writeln!(self.err, "{text}");
    }

    /// Writes text and a newline to stdout; exit code `0`, or `2` when the write fails.
    pub(crate) fn write_out(&mut self, text: &str) -> u8 {
        match writeln!(self.out, "{text}") {
            Ok(()) => EXIT_OK,
            Err(error) => self.fail(ERR_WRITE, &[("reason", &error.to_string())]),
        }
    }

    /// Looks up `key` and writes it to stdout.
    pub(crate) fn say_out(&mut self, key: MessageKey, args: &[(&str, &str)]) -> u8 {
        let text = self.tr(key, args);
        self.write_out(&text)
    }

    /// Applies `--lang` when present; `false` for a language that does not exist.
    pub(crate) fn apply_lang(&mut self, parsed: &Parsed) -> bool {
        match parsed.option("--lang").map(Locale::parse) {
            None => true,
            Some(Some(locale)) => {
                self.locale = locale;
                true
            }
            Some(None) => false,
        }
    }

    pub(crate) fn usage(&mut self, detail: &str, key: Option<MessageKey>) -> u8 {
        let mut text = self.tr(ERR_USAGE, &[("detail", detail)]);
        let usages = match key {
            Some(key) => vec![key],
            None => vec![
                USAGE_COLLECT,
                USAGE_COMPARE,
                USAGE_INIT,
                USAGE_NODE,
                USAGE_PROVIDER,
                USAGE_ASK,
                USAGE_MEMORY,
                USAGE_PLAN,
                USAGE_TRANSPORT,
                USAGE_SERVE,
            ],
        };
        for key in usages {
            text.push('\n');
            text.push_str(&self.tr(key, &[]));
        }
        let _ = writeln!(self.err, "{text}");
        EXIT_ERROR
    }

    fn dispatch(&mut self, args: &[String]) -> u8 {
        let (globals, args) = match split_globals(args) {
            Ok(split) => split,
            Err(detail) => return self.usage(&detail, None),
        };
        let Some((command, rest)) = args.split_first() else {
            return self.usage("-", None);
        };
        match command.as_str() {
            "collect" => self.dispatch_collect(&globals, rest),
            "compare" => self.dispatch_compare(&globals, rest),
            "init" => match parse(rest, &["--slug", "--name", "--email", "--lang"], &[]) {
                Ok(parsed) if self.apply_lang(&parsed) => self.init(&globals, &parsed),
                Ok(_) => self.usage("--lang", Some(USAGE_INIT)),
                Err(detail) => self.usage(&detail, Some(USAGE_INIT)),
            },
            "node" => self.node_command(&globals, rest),
            "provider" => self.provider_command(&globals, rest),
            "ask" => self.ask(&globals, rest),
            "memory" => self.memory_command(&globals, rest),
            "plan" => self.plan_command(&globals, rest),
            "transport" => self.transport_command(&globals, rest),
            "serve" => self.serve(&globals, rest),
            other => self.usage(other, None),
        }
    }

    fn dispatch_collect(&mut self, globals: &Globals, rest: &[String]) -> u8 {
        let parsed = match parse(
            rest,
            &["--root", "--os", "--arch", "--lang", "--node"],
            &["--save"],
        ) {
            Ok(parsed) => parsed,
            Err(detail) => return self.usage(&detail, Some(USAGE_COLLECT)),
        };
        if !self.apply_lang(&parsed) {
            return self.usage("--lang", Some(USAGE_COLLECT));
        }
        if let Some(extra) = parsed.positional.first() {
            return self.usage(extra, Some(USAGE_COLLECT));
        }
        // `--node` names where `--save` stores the inventory; either alone makes no sense.
        if parsed.flag("--save") != parsed.option("--node").is_some() {
            return self.usage(
                if parsed.flag("--save") {
                    "--save"
                } else {
                    "--node"
                },
                Some(USAGE_COLLECT),
            );
        }
        self.collect(globals, &parsed)
    }

    fn dispatch_compare(&mut self, globals: &Globals, rest: &[String]) -> u8 {
        let parsed = match parse(
            rest,
            &["--lang", "--source-node", "--target-node"],
            &["--save"],
        ) {
            Ok(parsed) => parsed,
            Err(detail) => return self.usage(&detail, Some(USAGE_COMPARE)),
        };
        if !self.apply_lang(&parsed) {
            return self.usage("--lang", Some(USAGE_COMPARE));
        }
        if parsed.flag("--save") {
            return match (
                parsed.option("--source-node"),
                parsed.option("--target-node"),
                parsed.positional.first(),
            ) {
                (Some(source), Some(target), None) => self.compare_saved(globals, source, target),
                (_, _, Some(extra)) => self.usage(extra, Some(USAGE_COMPARE)),
                _ => self.usage("--save", Some(USAGE_COMPARE)),
            };
        }
        if let Some(detail) = parsed
            .option("--source-node")
            .map(|_| "--source-node")
            .or_else(|| parsed.option("--target-node").map(|_| "--target-node"))
        {
            return self.usage(detail, Some(USAGE_COMPARE));
        }
        match parsed.positional.as_slice() {
            [source, target] => self.compare_files(source, target),
            _ => self.usage("-", Some(USAGE_COMPARE)),
        }
    }

    fn collect(&mut self, globals: &Globals, parsed: &Parsed) -> u8 {
        let root_option = parsed.option("--root");
        let root = root_option.map_or_else(RootFs::host, RootFs::new);
        // Programs (docker, systemctl) run only when inspecting this machine; a `--root` tree is
        // a fixture or a mounted image, where they would describe the wrong machine.
        let runner: Arc<dyn CommandRunner> = if root_option.is_none() {
            Arc::new(HostRunner)
        } else {
            Arc::new(FakeRunner::new())
        };
        let host = host_platform();
        let platform = Platform {
            os: parsed.option("--os").map_or(host.os, str::to_owned),
            arch: parsed.option("--arch").map_or(host.arch, str::to_owned),
        };
        let inventory = match collect_inventory_with(&root, runner, time::now_iso_utc(), platform) {
            Ok(inventory) => inventory,
            Err(error) => return self.fail(ERR_COLLECT, &[("reason", &error.to_string())]),
        };
        let json = match inventory.to_json() {
            Ok(json) => json,
            Err(error) => return self.fail(ERR_COLLECT, &[("reason", &error.to_string())]),
        };
        let printed = self.write_out(&json);
        if printed != EXIT_OK {
            return printed;
        }
        match parsed.option("--node") {
            Some(node) => self.save_collected(globals, node, &inventory),
            None => EXIT_OK,
        }
    }

    pub(crate) fn load(&mut self, path: &str) -> Result<Inventory, u8> {
        let text = read_limited(Path::new(path), MAX_INVENTORY_BYTES).map_err(|error| {
            let reason = match error {
                ReadError::NotFile => self.tr(REASON_NOT_FILE, &[]),
                ReadError::TooLarge => self.tr(REASON_TOO_LARGE, &[]),
                ReadError::Io(error) => error.to_string(),
            };
            self.fail(ERR_READ, &[("path", path), ("reason", &reason)])
        })?;
        Inventory::from_json(&text)
            .map_err(|e| self.fail(ERR_INVENTORY, &[("path", path), ("reason", &e.to_string())]))
    }

    fn compare_files(&mut self, source_path: &str, target_path: &str) -> u8 {
        let source = match self.load(source_path) {
            Ok(inventory) => inventory,
            Err(code) => return code,
        };
        let target = match self.load(target_path) {
            Ok(inventory) => inventory,
            Err(code) => return code,
        };
        self.report(&source, &target).0
    }

    /// Compares two inventories and prints the report. Returns the exit code (`0`, `1` for a
    /// critical finding, `2` when the output cannot be written) and the findings.
    pub(crate) fn report(
        &mut self,
        source: &Inventory,
        target: &Inventory,
    ) -> (u8, Vec<inventory::Finding>) {
        let report = compare(source, target, &inventory::RuleContext::default());
        let rules = report.rules.len().to_string();
        let mut text = String::new();

        if report.findings.is_empty() {
            text.push_str(&self.tr(REPORT_NONE, &[("rules", &rules)]));
        } else {
            let count = report.findings.len().to_string();
            text.push_str(&self.tr(REPORT_COUNT, &[("count", &count), ("rules", &rules)]));
            for original in &report.findings {
                // Values in a finding come from the (untrusted) inventories.
                let finding = &parity::cleaned(original, &sanitize);
                let rendered = render(finding, &self.catalog, self.locale);
                let kind = self.tr(kind_key(finding.subject.kind), &[]);
                let head = self.tr(
                    REPORT_HEAD,
                    &[
                        ("severity", &rendered.severity_label),
                        ("rule", &finding.rule_id),
                        ("title", &rendered.title),
                    ],
                );
                let advice = self.tr(REPORT_ADVICE, &[("advice", &rendered.advice)]);
                let subject = self.tr(
                    REPORT_SUBJECT,
                    &[("kind", &kind), ("key", &finding.subject.key)],
                );
                text.push_str(&format!(
                    "\n\n{head}\n  {}\n  {advice}\n  {subject}",
                    rendered.explanation
                ));
                if let Some(note) = rendered.note {
                    text.push_str(&format!("\n  {note}"));
                }
            }
        }

        let warnings: Vec<&str> = [source, target]
            .into_iter()
            .filter(|inv| inv.status == Status::Partial)
            .flat_map(|inv| inv.warnings.iter().map(String::as_str))
            .collect();
        if source.status == Status::Partial || target.status == Status::Partial {
            text.push_str("\n\n");
            text.push_str(&self.tr(REPORT_PARTIAL, &[("warnings", &warnings.join("; "))]));
        }
        text.push_str("\n\n");
        let ids: Vec<&str> = report.rules.iter().map(|(id, _)| *id).collect();
        text.push_str(&self.tr(REPORT_LIMITS, &[("ids", &ids.join(", "))]));

        if let Err(error) = writeln!(self.out, "{text}") {
            let code = self.fail(ERR_WRITE, &[("reason", &error.to_string())]);
            return (code, report.findings);
        }
        let code = if report
            .findings
            .iter()
            .any(|f| f.severity == Severity::Critical)
        {
            EXIT_CRITICAL
        } else {
            EXIT_OK
        };
        (code, report.findings)
    }
}

/// Catalog key for the name of an item kind.
fn kind_key(kind: Kind) -> MessageKey {
    match kind {
        Kind::Listener => KIND_LISTENER,
        Kind::Process => KIND_PROCESS,
        Kind::Cron => KIND_CRON,
        Kind::EnvRef => KIND_ENV_REF,
        Kind::Vhost => MessageKey::new("inventory.kind.vhost"),
        Kind::Cert => MessageKey::new("inventory.kind.cert"),
        Kind::Database => MessageKey::new("inventory.kind.database"),
        Kind::Dns => MessageKey::new("inventory.kind.dns"),
        Kind::Check => MessageKey::new("inventory.kind.check"),
    }
}

/// The platform of the machine running this binary, in inventory vocabulary.
fn host_platform() -> Platform {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        other => other,
    };
    Platform {
        os: os.to_owned(),
        arch: arch.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{ReadError, read_limited};
    use std::path::PathBuf;

    fn temp(name: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
        let mut path = std::env::temp_dir();
        path.push(format!("infra-cli-{}-{name}", std::process::id()));
        std::fs::write(&path, bytes)?;
        Ok(path)
    }

    #[test]
    fn read_limited_enforces_the_limit_while_reading() -> Result<(), Box<dyn std::error::Error>> {
        let path = temp("limit", b"12345")?;
        assert_eq!(read_limited(&path, 5).ok().as_deref(), Some("12345"));
        assert!(matches!(read_limited(&path, 4), Err(ReadError::TooLarge)));
        std::fs::remove_file(&path)?;
        Ok(())
    }

    #[test]
    fn read_limited_rejects_non_files_and_bad_utf8() -> Result<(), Box<dyn std::error::Error>> {
        #[cfg(unix)]
        assert!(matches!(
            read_limited(std::path::Path::new("/dev/zero"), 10),
            Err(ReadError::NotFile)
        ));
        assert!(matches!(
            read_limited(&std::env::temp_dir(), 10),
            Err(ReadError::NotFile)
        ));
        let path = temp("utf8", &[0xff, 0xfe])?;
        assert!(matches!(read_limited(&path, 10), Err(ReadError::Io(_))));
        std::fs::remove_file(&path)?;
        Ok(())
    }
}
