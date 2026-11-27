//! `infra transport add|list|show|remove|check` (ADR 0028).
//!
//! Credentials never travel on the command line: they come from standard input
//! (`--credentials-stdin`, one `name=value` per line), are sealed with the vault (master key from
//! `NODIVARA_MASTER_KEY`) and are stored only as ciphertext. Nothing here prints a credential or
//! a secret id; `show` prints the name, the kind and the last check only.
//!
//! `check` is a **dry run** unless `--run` is given: it prints the commands, with every secret
//! value masked, and starts nothing. With `--run` the commands run on this machine through the
//! `Runner` seam; a missing program gets advice and an offer, and nothing is ever installed.

use crate::Ui;
use crate::args::{Globals, Parsed, parse};
use crate::plans::safe_detail;
use db::{NewTransport, OrgId, TransportRow};
use i18n::MessageKey;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::Read;
use std::sync::Arc;
use std::time::Duration;
use transport::{
    Advice, CommandPlan, Credentials, EnvValue, Operation, Progress, RcloneSpec, RemoteKind,
    RunError, RunOutput, Runner, Secret, SpecError, TransportKind, TransportSpec, Wizard,
    WizardParams, classify_output, classify_run_error, plan as plan_command, store, validate,
};

const ADDED: MessageKey = MessageKey::new("cli.transport.added");
const EXISTS: MessageKey = MessageKey::new("cli.transport.exists");
const UNKNOWN: MessageKey = MessageKey::new("cli.transport.unknown");
const REMOVED: MessageKey = MessageKey::new("cli.transport.removed");
const INVALID: MessageKey = MessageKey::new("cli.transport.invalid");
const MANAGED: MessageKey = MessageKey::new("cli.transport.managed");
const NO_CREDENTIALS: MessageKey = MessageKey::new("cli.transport.no_credentials");
const BAD_CREDENTIALS: MessageKey = MessageKey::new("cli.transport.bad_credentials");
const BAD_CREDENTIAL_LINE: MessageKey = MessageKey::new("cli.transport.bad_credential_line");
const VAULT_MISMATCH: MessageKey = MessageKey::new("cli.transport.vault_mismatch");
const LIST_EMPTY: MessageKey = MessageKey::new("cli.transport.list_empty");
const LIST_HEAD: MessageKey = MessageKey::new("cli.transport.list_head");
const LIST_LINE: MessageKey = MessageKey::new("cli.transport.list_line");
const SHOW_NAME: MessageKey = MessageKey::new("cli.transport.show_name");
const SHOW_KIND: MessageKey = MessageKey::new("cli.transport.show_kind");
const SHOW_VERIFIED: MessageKey = MessageKey::new("cli.transport.show_verified");
const SHOW_CHECK: MessageKey = MessageKey::new("cli.transport.show_check");
const NEVER: MessageKey = MessageKey::new("cli.transport.never");
const NONE_YET: MessageKey = MessageKey::new("cli.transport.none_yet");
const CHECK_DRY: MessageKey = MessageKey::new("cli.transport.check_dry");
const CHECK_RUNNING: MessageKey = MessageKey::new("cli.transport.check_running");
const DRY_STEP: MessageKey = MessageKey::new("cli.transport.dry_step");
const DRY_NEEDS_TOKEN: MessageKey = MessageKey::new("cli.transport.dry_needs_token");
const DRY_DONE: MessageKey = MessageKey::new("cli.transport.dry_done");
const STEP_OK: MessageKey = MessageKey::new("cli.transport.step_ok");
const STEP_PROBE: MessageKey = MessageKey::new("cli.transport.step_probe");
const STEP_VERIFY: MessageKey = MessageKey::new("cli.transport.step_verify");
const NEEDS_TOKEN: MessageKey = MessageKey::new("cli.transport.needs_token");
const AUTHORIZE_HINT: MessageKey = MessageKey::new("transport.authorize.hint");
const DETAIL: MessageKey = MessageKey::new("cli.transport.detail");
const OFFER_INSTALL: MessageKey = MessageKey::new("cli.transport.offer_install");
const PASSED: MessageKey = MessageKey::new("cli.transport.passed");
const SPEED_LEFTOVER: MessageKey = MessageKey::new("cli.transport.speed_leftover");
const UNKNOWN_VALUE: MessageKey = MessageKey::new("cli.transport.unknown_value");
const CORRUPT: MessageKey = MessageKey::new("cli.transport.corrupt");
const REPORT: MessageKey = MessageKey::new("cli.transport.report");
const CREDENTIALS_UPDATED: MessageKey = MessageKey::new("cli.transport.credentials_updated");

/// Most bytes of credentials read from standard input.
const MAX_CREDENTIAL_BYTES: usize = 64 * 1024;
/// Longest a single command of `check --run` may take before it is stopped.
const RUN_TIMEOUT: Duration = Duration::from_secs(120);
/// Most output bytes kept per stream of a command.
const RUN_MAX_OUTPUT: usize = 1024 * 1024;
/// A wizard has six steps; this only guards against a loop if that ever changes.
const MAX_ADVANCES: usize = 12;

/// Prints the commands a check would run, masks every secret, and runs nothing.
struct DryRunner {
    lines: RefCell<Vec<String>>,
    secrets: Vec<String>,
}

impl DryRunner {
    fn new(credentials: &Credentials) -> Self {
        Self {
            lines: RefCell::new(Vec::new()),
            secrets: credentials
                .pairs()
                .iter()
                .map(|(_, v)| v.expose_secret().to_owned())
                .collect(),
        }
    }

    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.lines.borrow_mut())
    }
}

impl Runner for DryRunner {
    fn run(&self, plan: &CommandPlan) -> Result<RunOutput, RunError> {
        // Written here rather than through `Display`: that one runs the whole line through the
        // redactor, which also mangles harmless flags such as `--ask-password=false`. Secret
        // values are `***` by construction (they only exist in the environment and stdin of a
        // plan, and stdin is never printed); the pass over the known values is a second guard in
        // case one ever reached an argument.
        let mut line = String::new();
        for (name, value) in &plan.env {
            match value {
                EnvValue::Plain(v) => line.push_str(&format!("{name}={v:?} ")),
                EnvValue::Secret(_) => line.push_str(&format!("{name}=*** ")),
            }
        }
        line.push_str(plan.program);
        for arg in &plan.args {
            line.push_str(&format!(" {arg:?}"));
        }
        for secret in self.secrets.iter().filter(|s| !s.is_empty()) {
            line = line.replace(secret.as_str(), "***");
        }
        self.lines.borrow_mut().push(line);
        // An empty JSON object parses as a quota with every field unknown.
        Ok(RunOutput::ok("{}"))
    }
}

/// How a check ended.
enum Outcome {
    /// Every step passed; the JSON is what `transports.last_check` keeps.
    Passed(String),
    /// A step failed with this advice.
    Failed(Advice),
    /// The remote needs an OAuth token that is not stored.
    NeedsToken(String),
}

impl Ui<'_> {
    pub(crate) fn transport_command(&mut self, globals: &Globals, rest: &[String]) -> u8 {
        let usage = Some(crate::USAGE_TRANSPORT);
        let Some((sub, args)) = rest.split_first() else {
            return self.usage("-", usage);
        };
        let parsed = match sub.as_str() {
            "add" => parse(
                args,
                &[
                    "--name",
                    "--kind",
                    "--remote-kind",
                    "--remote",
                    "--path",
                    "--options",
                    "--host",
                    "--user",
                    "--port",
                    "--lang",
                ],
                &["--credentials-stdin"],
            ),
            "check" => parse(args, &["--lang"], &["--run"]),
            "update-credentials" => parse(args, &["--lang"], &["--credentials-stdin"]),
            "list" | "show" | "remove" => parse(args, &["--lang"], &[]),
            other => return self.usage(&safe_detail(other), usage),
        };
        let parsed = match parsed {
            Ok(parsed) => parsed,
            Err(detail) => return self.usage(&safe_detail(&detail), usage),
        };
        if !self.apply_lang(&parsed) {
            return self.usage("--lang", usage);
        }
        match (sub.as_str(), parsed.positional.as_slice()) {
            ("add", []) => self.transport_add(globals, &parsed),
            ("list", []) => self.transport_list(globals),
            ("show", [name]) => self.transport_show(globals, name),
            ("remove", [name]) => self.transport_remove(globals, name),
            ("check", [name]) => self.transport_check(globals, name, parsed.flag("--run")),
            ("update-credentials", [target]) if parsed.flag("--credentials-stdin") => {
                self.transport_update_credentials(globals, target)
            }
            _ => self.usage("-", usage),
        }
    }

    // ---- add ------------------------------------------------------------------------------

    /// Builds the spec from the kind-specific flags: every field goes through the transport
    /// crate's constructors. A flag that belongs to another kind is a usage error.
    fn spec_from_flags(
        &mut self,
        kind: TransportKind,
        parsed: &Parsed,
    ) -> Result<TransportSpec, u8> {
        let usage = Some(crate::USAGE_TRANSPORT);
        let rclone_flags = ["--remote-kind", "--remote", "--options"];
        let ssh_flags = ["--host", "--user", "--port"];
        let foreign: &[&str] = match kind {
            TransportKind::Rclone => &ssh_flags,
            _ => &rclone_flags,
        };
        if let Some(flag) = foreign.iter().find(|f| parsed.option(f).is_some()) {
            return Err(self.usage(flag, usage));
        }
        let invalid = |ui: &mut Self, e: SpecError| ui.fail(INVALID, &[("reason", &e.to_string())]);
        if matches!(kind, TransportKind::Rclone) {
            let (Some(remote_kind), Some(remote)) =
                (parsed.option("--remote-kind"), parsed.option("--remote"))
            else {
                return Err(self.usage("-", usage));
            };
            let Some(remote_kind) = RemoteKind::parse(remote_kind) else {
                return Err(self.usage("--remote-kind", usage));
            };
            let options = match parsed.option("--options").map(parse_options) {
                None => BTreeMap::new(),
                Some(Some(options)) => options,
                Some(None) => return Err(self.usage("--options", usage)),
            };
            let path = parsed.option("--path").unwrap_or_default();
            return RcloneSpec::new(remote_kind, remote, path, options)
                .map(TransportSpec::Rclone)
                .map_err(|e| invalid(self, e));
        }
        let (Some(host), Some(user), Some(path)) = (
            parsed.option("--host"),
            parsed.option("--user"),
            parsed.option("--path"),
        ) else {
            return Err(self.usage("-", usage));
        };
        let port = match parsed.option("--port").map(str::parse::<u16>) {
            None => None,
            Some(Ok(port)) => Some(port),
            Some(Err(_)) => return Err(self.usage("--port", usage)),
        };
        let target =
            transport::SshTarget::new(host, user, port, path).map_err(|e| invalid(self, e))?;
        Ok(match kind {
            TransportKind::Rsync => TransportSpec::Rsync(target),
            TransportKind::Scp => TransportSpec::Scp(target),
            _ => TransportSpec::SshPipe(target),
        })
    }

    /// Reads `name=value` lines from standard input. The error says which line is wrong, never
    /// what it holds.
    fn read_credentials(&mut self) -> Result<Credentials, u8> {
        let mut bytes = Vec::new();
        let read =
            Read::take(&mut *self.stdin, (MAX_CREDENTIAL_BYTES as u64) + 1).read_to_end(&mut bytes);
        let bad = |ui: &mut Self| ui.fail(BAD_CREDENTIAL_LINE, &[("line", "0")]);
        if read.is_err() || bytes.len() > MAX_CREDENTIAL_BYTES {
            return Err(bad(self));
        }
        let Ok(text) = String::from_utf8(bytes) else {
            return Err(bad(self));
        };
        let mut pairs = Vec::new();
        for (index, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let Some((name, value)) = line.split_once('=') else {
                return Err(self.fail(BAD_CREDENTIAL_LINE, &[("line", &(index + 1).to_string())]));
            };
            pairs.push((
                name.trim().to_owned(),
                Secret::new(value.trim_end_matches('\r')),
            ));
        }
        Credentials::new(pairs)
            .map_err(|e| self.fail(BAD_CREDENTIALS, &[("reason", &e.to_string())]))
    }

    fn find_transport(
        &mut self,
        conn: &db::Connection,
        org: &OrgId,
        name: &str,
    ) -> Result<TransportRow, u8> {
        match db::list_transports(conn, org) {
            Ok(rows) => rows
                .into_iter()
                .find(|r| r.name == name)
                .ok_or_else(|| self.fail(UNKNOWN, &[("name", name)])),
            Err(error) => Err(self.store_error(&error)),
        }
    }

    fn transport_add(&mut self, globals: &Globals, parsed: &Parsed) -> u8 {
        let usage = Some(crate::USAGE_TRANSPORT);
        let (Some(name), Some(kind)) = (parsed.option("--name"), parsed.option("--kind")) else {
            return self.usage("-", usage);
        };
        let Some(kind) = TransportKind::parse(kind) else {
            return self.usage("--kind", usage);
        };
        if matches!(kind, TransportKind::Managed) {
            return self.fail(MANAGED, &[]);
        }
        let spec = match self.spec_from_flags(kind, parsed) {
            Ok(spec) => spec,
            Err(code) => return code,
        };
        if let Err(error) = validate::transport_name(name) {
            return self.fail(INVALID, &[("reason", &error.to_string())]);
        }
        let credentials = if parsed.flag("--credentials-stdin") {
            if !kind.accepts_credentials() {
                return self.fail(NO_CREDENTIALS, &[("kind", kind.as_str())]);
            }
            match self.read_credentials() {
                Ok(credentials) if !credentials.is_empty() => Some(credentials),
                Ok(_) => None,
                Err(code) => return code,
            }
        } else {
            None
        };
        // Names and options a backend does not know are refused now, not at the first check.
        if let Some(credentials) = &credentials
            && let Err(error) = plan_command(&spec, Some(credentials), &Operation::Verify)
        {
            return self.fail(BAD_CREDENTIALS, &[("reason", &error.to_string())]);
        }
        let vault = if credentials.is_some() {
            match self.vault_from_env() {
                Ok(vault) => Some(vault),
                Err(code) => return code,
            }
        } else {
            None
        };
        let (conn, _) = match self.open_db(globals, false) {
            Ok(opened) => opened,
            Err(code) => return code,
        };
        let scope = match self.scope(&conn, globals) {
            Ok(scope) => scope,
            Err(code) => return code,
        };
        match db::list_transports(&conn, &scope.org) {
            Ok(rows) if rows.iter().any(|r| r.name == name) => {
                return self.fail(EXISTS, &[("name", name)]);
            }
            Ok(_) => {}
            Err(error) => return self.store_error(&error),
        }
        let saved = match &vault {
            Some(vault) => store::save(
                &conn,
                vault,
                &scope.org,
                name,
                &spec,
                credentials.as_ref(),
                None,
            )
            .map(|_| ())
            .map_err(|e| e.to_string()),
            // Nothing to seal, so no master key is needed: store the row directly.
            None => db::create_transport(
                &conn,
                &scope.org,
                &NewTransport {
                    name,
                    kind: kind.as_str(),
                    config_json: &spec.to_config_json(),
                    secret: None,
                },
            )
            .map(|_| ())
            .map_err(|e| e.to_string()),
        };
        match saved {
            Ok(()) => self.say_out(ADDED, &[("name", name), ("kind", kind.as_str())]),
            Err(reason) => self.store_reason(&reason),
        }
    }

    // ---- list, show, remove ---------------------------------------------------------------

    fn transport_list(&mut self, globals: &Globals) -> u8 {
        let (conn, _) = match self.open_db(globals, false) {
            Ok(opened) => opened,
            Err(code) => return code,
        };
        let scope = match self.scope(&conn, globals) {
            Ok(scope) => scope,
            Err(code) => return code,
        };
        let rows = match db::list_transports(&conn, &scope.org) {
            Ok(rows) => rows,
            Err(error) => return self.store_error(&error),
        };
        if rows.is_empty() {
            return self.say_out(LIST_EMPTY, &[]);
        }
        let mut text = self.tr(LIST_HEAD, &[]);
        for row in &rows {
            let verified = self.verified_text(row);
            text.push('\n');
            // Three columns only: the row's settings and secret id stay behind.
            text.push_str(&self.tr(
                LIST_LINE,
                &[
                    ("name", &row.name),
                    ("kind", &row.kind),
                    ("verified", &verified),
                ],
            ));
        }
        self.write_out(&text)
    }

    fn verified_text(&self, row: &TransportRow) -> String {
        row.verified_at
            .clone()
            .unwrap_or_else(|| self.tr(NEVER, &[]))
    }

    fn transport_show(&mut self, globals: &Globals, name: &str) -> u8 {
        let (conn, _) = match self.open_db(globals, false) {
            Ok(opened) => opened,
            Err(code) => return code,
        };
        let scope = match self.scope(&conn, globals) {
            Ok(scope) => scope,
            Err(code) => return code,
        };
        let row = match self.find_transport(&conn, &scope.org, name) {
            Ok(row) => row,
            Err(code) => return code,
        };
        let verified = self.verified_text(&row);
        // The last check holds numbers, booleans and a fixed problem code only.
        let check = row
            .last_check_json
            .clone()
            .unwrap_or_else(|| self.tr(NONE_YET, &[]));
        let text = [
            self.tr(SHOW_NAME, &[("value", &row.name)]),
            self.tr(SHOW_KIND, &[("value", &row.kind)]),
            self.tr(SHOW_VERIFIED, &[("value", &verified)]),
            self.tr(SHOW_CHECK, &[("value", &check)]),
        ]
        .join("\n");
        self.write_out(&text)
    }

    fn transport_remove(&mut self, globals: &Globals, name: &str) -> u8 {
        let (conn, _) = match self.open_db(globals, false) {
            Ok(opened) => opened,
            Err(code) => return code,
        };
        let scope = match self.scope(&conn, globals) {
            Ok(scope) => scope,
            Err(code) => return code,
        };
        let row = match self.find_transport(&conn, &scope.org, name) {
            Ok(row) => row,
            Err(code) => return code,
        };
        // The sealed credentials go with the transport.
        match store::delete(&conn, &scope.org, &row.id) {
            Ok(()) => self.say_out(REMOVED, &[("name", name)]),
            Err(error) => self.store_reason(&error.to_string()),
        }
    }

    // ---- check ----------------------------------------------------------------------------

    fn transport_check(&mut self, globals: &Globals, name: &str, run: bool) -> u8 {
        let (conn, _) = match self.open_db(globals, false) {
            Ok(opened) => opened,
            Err(code) => return code,
        };
        let scope = match self.scope(&conn, globals) {
            Ok(scope) => scope,
            Err(code) => return code,
        };
        let row = match self.find_transport(&conn, &scope.org, name) {
            Ok(row) => row,
            Err(code) => return code,
        };
        let stored = match store::load(&conn, &scope.org, &row.id) {
            Ok(stored) => stored,
            Err(error) => return self.fail(CORRUPT, &[("reason", &error.to_string())]),
        };
        // The master key is needed only when there are sealed credentials to open.
        let credentials = if stored.secret_id.is_some() {
            let vault = match self.vault_from_env() {
                Ok(vault) => vault,
                Err(code) => return code,
            };
            match store::load_credentials(&conn, &vault, &scope.org, &stored) {
                Ok(Some(credentials)) => credentials,
                Ok(None) => empty_credentials(),
                Err(store::StoreError::Vault(_)) => return self.fail(VAULT_MISMATCH, &[]),
                Err(error) => return self.store_reason(&error.to_string()),
            }
        } else {
            empty_credentials()
        };

        let dry = DryRunner::new(&credentials);
        let real: Arc<dyn Runner>;
        let runner: &dyn Runner = if run {
            real = self.env.runner().unwrap_or_else(|| {
                Arc::new(transport::ProcessRunner::new(RUN_TIMEOUT, RUN_MAX_OUTPUT))
            });
            real.as_ref()
        } else {
            &dry
        };
        let header = if run {
            self.tr(CHECK_RUNNING, &[("name", name)])
        } else {
            self.tr(CHECK_DRY, &[("name", name)])
        };
        let printed = self.write_out(&header);
        if printed != crate::EXIT_OK {
            return printed;
        }

        let dry_view = if run { None } else { Some(&dry) };
        let outcome = match &stored.spec {
            TransportSpec::Rclone(spec) => {
                self.check_rclone(spec.clone(), credentials, runner, dry_view)
            }
            other => self.check_ssh(other, runner, dry_view),
        };
        match outcome {
            Ok(Outcome::Passed(json)) => {
                if !run {
                    return self.say_out(DRY_DONE, &[("name", name)]);
                }
                if let Err(error) =
                    db::record_transport_check(&conn, &scope.org, &row.id, &json, true)
                {
                    return self.store_error(&error);
                }
                self.say_out(PASSED, &[("name", name)])
            }
            Ok(Outcome::NeedsToken(command)) => {
                if !run {
                    return self.say_out(DRY_NEEDS_TOKEN, &[("name", name)]);
                }
                let hint = self.tr(AUTHORIZE_HINT, &[("command", &command)]);
                self.note_err(&hint);
                self.fail(NEEDS_TOKEN, &[("name", name)])
            }
            Ok(Outcome::Failed(advice)) => {
                if run {
                    let json =
                        format!("{{\"ok\":false,\"problem\":\"{}\"}}", advice.problem.code());
                    if let Err(error) =
                        db::record_transport_check(&conn, &scope.org, &row.id, &json, false)
                    {
                        return self.store_error(&error);
                    }
                }
                self.report_advice(&advice)
            }
            Err(code) => code,
        }
    }

    /// Prints what a step did: the masked commands (dry run) or a confirmation (run).
    fn step_done(&mut self, label: &str, dry: Option<&DryRunner>) -> Result<(), u8> {
        let text = match dry {
            Some(dry) => {
                let mut text = self.tr(DRY_STEP, &[("step", label)]);
                for line in dry.take() {
                    text.push_str("\n  ");
                    text.push_str(&line);
                }
                text
            }
            None => self.tr(STEP_OK, &[("step", label)]),
        };
        match self.write_out(&text) {
            crate::EXIT_OK => Ok(()),
            code => Err(code),
        }
    }

    fn check_rclone(
        &mut self,
        spec: RcloneSpec,
        credentials: Credentials,
        runner: &dyn Runner,
        dry: Option<&DryRunner>,
    ) -> Result<Outcome, u8> {
        let mut wizard = Wizard::new(spec, credentials, WizardParams::default());
        for _ in 0..MAX_ADVANCES {
            match wizard.advance(runner) {
                Progress::Advanced(step) => {
                    let label = self.tr(step.label_key(), &[]);
                    self.step_done(&label, dry)?;
                }
                Progress::NeedsToken { command } => return Ok(Outcome::NeedsToken(command)),
                Progress::Failed(advice) => return Ok(Outcome::Failed(advice)),
                Progress::ReadyToSave => {
                    let Ok(done) = wizard.finish() else {
                        break;
                    };
                    if done.report.speed_leftover && dry.is_none() {
                        let note = self.tr(SPEED_LEFTOVER, &[]);
                        self.note_err(&note);
                    }
                    if dry.is_none() {
                        self.print_report(&done.report)?;
                    }
                    return Ok(Outcome::Passed(done.report.to_json()));
                }
            }
        }
        Ok(Outcome::Failed(Advice::of(transport::ProblemKind::Unknown)))
    }

    /// rsync, scp and ssh pipe have no wizard: the program must exist and the host must answer.
    fn check_ssh(
        &mut self,
        spec: &TransportSpec,
        runner: &dyn Runner,
        dry: Option<&DryRunner>,
    ) -> Result<Outcome, u8> {
        for (op, label_key) in [
            (Operation::Probe, STEP_PROBE),
            (Operation::Verify, STEP_VERIFY),
        ] {
            let Ok(command) = plan_command(spec, None, &op) else {
                return Ok(Outcome::Failed(Advice::of(
                    transport::ProblemKind::InvalidConfig,
                )));
            };
            match runner.run(&command) {
                Err(error) => return Ok(Outcome::Failed(classify_run_error(error))),
                Ok(output) if !output.success() => {
                    return Ok(Outcome::Failed(classify_output(&output, &[])));
                }
                Ok(_) => {}
            }
            let label = self.tr(label_key, &[]);
            self.step_done(&label, dry)?;
        }
        Ok(Outcome::Passed("{\"ok\":true}".to_owned()))
    }

    fn print_report(&mut self, report: &transport::Report) -> Result<(), u8> {
        let unknown = self.tr(UNKNOWN_VALUE, &[]);
        let free = report
            .quota
            .and_then(|q| q.free)
            .map_or_else(|| unknown.clone(), |n| n.to_string());
        let speed = report
            .speed_bytes_per_sec
            .map_or(unknown, |n| n.to_string());
        let text = self.tr(REPORT, &[("free", &free), ("speed", &speed)]);
        match self.write_out(&text) {
            crate::EXIT_OK => Ok(()),
            code => Err(code),
        }
    }

    /// The advice for a failed check, in the reader's language, on stderr; exit code 2.
    fn report_advice(&mut self, advice: &Advice) -> u8 {
        let (title, body) = advice.render(&self.catalog, self.locale);
        let mut text = format!(
            "{title}
{body}"
        );
        if !advice.detail.is_empty() {
            // The tool's own words: quoted, shortened and redacted by the transport crate.
            text.push('\n');
            text.push_str(&self.tr(DETAIL, &[("detail", &advice.detail)]));
        }
        if advice.offer_install() {
            text.push('\n');
            text.push_str(&self.tr(OFFER_INSTALL, &[]));
        }
        self.note_err(&text);
        crate::EXIT_ERROR
    }

    fn transport_update_credentials(&mut self, globals: &Globals, target: &str) -> u8 {
        let (conn, _) = match self.open_db(globals, false) {
            Ok(ok) => ok,
            Err(code) => return code,
        };
        let scope = match self.scope(&conn, globals) {
            Ok(scope) => scope,
            Err(code) => return code,
        };
        let row = if let Ok(id) = db::TransportId::parse(target) {
            match db::get_transport(&conn, &scope.org, &id) {
                Ok(r) => r,
                Err(db::Error::NotFound) => return self.fail(UNKNOWN, &[("name", target)]),
                Err(_) => return self.store_reason("transport_load"),
            }
        } else {
            match self.find_transport(&conn, &scope.org, target) {
                Ok(r) => r,
                Err(code) => return code,
            }
        };
        let vault = match self.vault_from_env() {
            Ok(v) => v,
            Err(code) => return code,
        };
        let creds = match self.read_credentials() {
            Ok(c) => c,
            Err(code) => return code,
        };
        match store::update_credentials(&conn, &vault, &scope.org, &row.id, &creds) {
            Ok(()) => {
                let subject = format!(
                    r#"{{"transport":"{}","name":"{}"}}"#,
                    row.id.as_str(),
                    row.name
                );
                let _ = db::append_audit(
                    &conn,
                    &scope.org,
                    &db::NewAuditEntry {
                        actor_kind: db::ActorKind::User,
                        actor_id: Some(scope.user.as_str()),
                        action: "transport.credentials.update",
                        node_id: None,
                        subject_json: &subject,
                        result: db::AuditOutcome::Success,
                    },
                );
                let msg = self.tr(CREDENTIALS_UPDATED, &[("name", &row.name)]);
                self.write_out(&msg)
            }
            Err(store::StoreError::CredentialsNotAccepted) => self.fail(NO_CREDENTIALS, &[]),
            Err(store::StoreError::CorruptCredentials) => self.fail(BAD_CREDENTIALS, &[]),
            Err(store::StoreError::NotFound) => self.fail(UNKNOWN, &[("name", target)]),
            Err(_) => self.store_reason("transport_update_credentials"),
        }
    }
}

fn empty_credentials() -> Credentials {
    // An empty list always passes validation.
    Credentials::new(Vec::new()).unwrap_or_else(|_| unreachable!("empty credentials are valid"))
}

/// `key=value,key=value` into a map; `None` for an empty piece, a missing `=` or a repeated key.
fn parse_options(text: &str) -> Option<BTreeMap<String, String>> {
    let mut map = BTreeMap::new();
    for piece in text.split(',') {
        let (key, value) = piece.split_once('=')?;
        if key.is_empty() || map.insert(key.to_owned(), value.to_owned()).is_some() {
            return None;
        }
    }
    Some(map)
}

#[cfg(test)]
mod tests {
    use super::parse_options;

    #[test]
    fn options_parse_strictly() {
        let ok = parse_options("region=eu,endpoint=x");
        assert_eq!(ok.map(|m| m.len()), Some(2));
        assert!(parse_options("a").is_none());
        assert!(parse_options("a=1,a=2").is_none());
        assert!(parse_options("=1").is_none());
        assert!(parse_options("a=1,").is_none());
        assert!(parse_options("").is_none());
    }
}
