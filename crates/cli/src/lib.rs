//! The `infra` command line (decision 0013, plan M1-a).
//!
//! * `infra collect` reads the machine **read-only** and prints an inventory as JSON.
//! * `infra compare SOURCE TARGET` compares two inventories and prints the parity report.
//!
//! Every message goes through the i18n catalog; nothing user-facing is written in code. Exit codes:
//! `0` ok, `1` the report contains a critical finding, `2` usage or I/O error.

mod time;

use collectors::{CommandRunner, FakeRunner, HostRunner, RootFs, collect_inventory_with};
use i18n::{Catalog, Locale, MessageKey};
use inventory::{Inventory, Kind, Platform, Severity, Status};
use parity::{compare, render};
use std::ffi::OsString;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;

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

const EXIT_OK: u8 = 0;
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

/// Runs the command line. `lang_env` is the `LANG` value (used when `--lang` is absent).
pub fn run(
    args: &[String],
    lang_env: Option<&str>,
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
        out,
        err,
    };
    ui.dispatch(args)
}

struct Ui<'a> {
    catalog: Catalog,
    locale: Locale,
    out: &'a mut dyn Write,
    err: &'a mut dyn Write,
}

/// Parsed `--name value` options plus positional arguments.
#[derive(Default)]
struct Parsed {
    options: Vec<(String, String)>,
    positional: Vec<String>,
}

impl Parsed {
    fn option(&self, name: &str) -> Option<&str> {
        self.options
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

impl Ui<'_> {
    /// Looks up `key`; every argument value is sanitized because it may come from an inventory
    /// or from a file system error message.
    fn tr(&self, key: MessageKey, args: &[(&str, &str)]) -> String {
        let clean: Vec<(&str, String)> = args.iter().map(|(n, v)| (*n, sanitize(v))).collect();
        let pairs: Vec<(&str, &str)> = clean.iter().map(|(n, v)| (*n, v.as_str())).collect();
        self.catalog.get(self.locale, key, &pairs).text
    }

    fn fail(&mut self, key: MessageKey, args: &[(&str, &str)]) -> u8 {
        let text = self.tr(key, args);
        let _ = writeln!(self.err, "{text}");
        EXIT_ERROR
    }

    fn usage(&mut self, detail: &str, key: Option<MessageKey>) -> u8 {
        let mut text = self.tr(ERR_USAGE, &[("detail", detail)]);
        let usages = match key {
            Some(key) => vec![key],
            None => vec![USAGE_COLLECT, USAGE_COMPARE],
        };
        for key in usages {
            text.push('\n');
            text.push_str(&self.tr(key, &[]));
        }
        let _ = writeln!(self.err, "{text}");
        EXIT_ERROR
    }

    fn dispatch(&mut self, args: &[String]) -> u8 {
        let Some((command, rest)) = args.split_first() else {
            return self.usage("-", None);
        };
        let allowed: &[&str] = match command.as_str() {
            "collect" => &["--root", "--os", "--arch", "--lang"],
            "compare" => &["--lang"],
            other => return self.usage(other, None),
        };
        let key = if command == "collect" {
            USAGE_COLLECT
        } else {
            USAGE_COMPARE
        };
        let parsed = match parse(rest, allowed) {
            Ok(parsed) => parsed,
            Err(detail) => return self.usage(&detail, Some(key)),
        };
        if let Some(tag) = parsed.option("--lang") {
            match Locale::parse(tag) {
                Some(locale) => self.locale = locale,
                None => return self.usage(tag, Some(key)),
            }
        }
        if command == "collect" {
            if let Some(extra) = parsed.positional.first() {
                return self.usage(extra, Some(key));
            }
            self.collect(&parsed)
        } else {
            match parsed.positional.as_slice() {
                [source, target] => self.compare(source, target),
                _ => self.usage("-", Some(key)),
            }
        }
    }

    fn collect(&mut self, parsed: &Parsed) -> u8 {
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
        match writeln!(self.out, "{json}") {
            Ok(()) => EXIT_OK,
            Err(error) => self.fail(ERR_WRITE, &[("reason", &error.to_string())]),
        }
    }

    fn load(&mut self, path: &str) -> Result<Inventory, u8> {
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

    fn compare(&mut self, source_path: &str, target_path: &str) -> u8 {
        let source = match self.load(source_path) {
            Ok(inventory) => inventory,
            Err(code) => return code,
        };
        let target = match self.load(target_path) {
            Ok(inventory) => inventory,
            Err(code) => return code,
        };
        let report = compare(&source, &target, &inventory::RuleContext::default());
        let rules = report.rules.len().to_string();
        let mut text = String::new();

        if report.findings.is_empty() {
            text.push_str(&self.tr(REPORT_NONE, &[("rules", &rules)]));
        } else {
            let count = report.findings.len().to_string();
            text.push_str(&self.tr(REPORT_COUNT, &[("count", &count), ("rules", &rules)]));
            for original in &report.findings {
                // Values in a finding come from the (untrusted) inventories.
                let mut finding = original.clone();
                for value in finding.args.values_mut() {
                    *value = sanitize(value);
                }
                finding.subject.key = sanitize(&finding.subject.key);
                let finding = &finding;
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

        let warnings: Vec<&str> = [&source, &target]
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
            return self.fail(ERR_WRITE, &[("reason", &error.to_string())]);
        }
        if report
            .findings
            .iter()
            .any(|f| f.severity == Severity::Critical)
        {
            EXIT_CRITICAL
        } else {
            EXIT_OK
        }
    }
}

/// Splits `--name value` options (only those in `allowed`) from positional arguments.
fn parse(args: &[String], allowed: &[&str]) -> Result<Parsed, String> {
    let mut parsed = Parsed::default();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg.starts_with("--") {
            if !allowed.contains(&arg.as_str()) {
                return Err(arg.clone());
            }
            let Some(value) = iter.next() else {
                return Err(arg.clone());
            };
            if parsed.option(arg).is_some() {
                // A repeated option is ambiguous; refusing beats silently picking the first.
                return Err(arg.clone());
            }
            parsed.options.push((arg.clone(), value.clone()));
        } else {
            parsed.positional.push(arg.clone());
        }
    }
    Ok(parsed)
}

/// Catalog key for the name of an item kind.
fn kind_key(kind: Kind) -> MessageKey {
    match kind {
        Kind::Listener => KIND_LISTENER,
        Kind::Process => KIND_PROCESS,
        Kind::Cron => KIND_CRON,
        Kind::EnvRef => KIND_ENV_REF,
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
