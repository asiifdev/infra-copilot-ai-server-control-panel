//! The executor: turns an [`Action`] into argv-only commands or validated local file
//! operations, and runs commands through a [`Runner`]. Output from the target is untrusted text:
//! it is redacted, stripped of control characters and cut before anything keeps it.

use crate::sites::render_site_vhost_config;
use crate::{AcmeProvider, Action, DbEngine, ServiceManagerKind};
use core_domain::redact::redact;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use transport::{CommandPlan, ProcessRunner, RunOutput, Runner};

/// Most characters of command output kept in an [`Outcome`] or a [`DryRun`].
pub const MAX_DETAIL: usize = 1000;

static BACKUP_SEQ: AtomicU64 = AtomicU64::new(0);

/// What running an action would do, found without changing anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DryRun {
    /// The commands or operations the action would run, in a log-safe, redacted rendering.
    pub steps: Vec<String>,
    /// What a read-only check found (for a restart: the current state of the service).
    pub current_state: String,
    /// Redacted preview diff when applicable (`None` for service restarts).
    pub diff: Option<String>,
}

/// Metadata of a pre-execution backup snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupSnapshot {
    /// Path or reference to the stored backup artifact.
    pub storage_ref: String,
    /// SHA-256 hex digest (64 chars) of the backup artifact.
    pub sha256: String,
}

/// How an execution or rollback ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeStatus {
    /// The commands ran and the verification passed.
    Succeeded,
    /// A command failed, could not start, or the verification did not pass.
    Failed,
}

/// The result of running an action or a rollback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// How it ended.
    pub status: OutcomeStatus,
    /// Whether the check made after the action passed.
    pub verified: bool,
    /// A fixed code for what happened (`ok`, `not_installed`, `timeout`, `command_failed`,
    /// `not_active_after`, `io`, `path_not_allowed`, `symlink_not_allowed`, `not_found`,
    /// `is_directory`, `verify_failed`, `backup_missing`, `backup_corrupted`).
    pub reason: &'static str,
    /// Redacted, bounded output of the failing or last command.
    pub detail: String,
}

/// Runs actions. Implementations must run only the commands of the closed action list.
pub trait Executor: Send + Sync {
    /// Describes what [`Executor::execute`] would do, with read-only checks only.
    fn dry_run(&self, action: &Action) -> DryRun;

    /// Takes a pre-execution backup snapshot in `backup_dir` for tier 3–4 actions.
    ///
    /// # Errors
    /// Returns a fixed reason code if the backup could not be created.
    fn backup(&self, action: &Action, backup_dir: &Path) -> Result<BackupSnapshot, &'static str>;

    /// Runs the action and verifies the result.
    fn execute(&self, action: &Action) -> Outcome;

    /// Restores the target from `backup` and verifies the result.
    fn rollback(&self, action: &Action, backup: &BackupSnapshot) -> Outcome;
}

/// Default allowlisted roots for local file mutations when no custom roots are supplied.
#[must_use]
pub fn default_allowed_roots() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/etc/nodivara"),
        PathBuf::from("/opt/nodivara"),
        PathBuf::from("/var/lib/nodivara"),
        PathBuf::from("/var/backups/nodivara"),
        PathBuf::from("/var/www"),
        PathBuf::from("/srv"),
    ]
}

/// Runs a read-only dry-run locally with default settings (used when execution is disabled but a
/// user previews a proposal).
#[must_use]
pub fn dry_run_readonly(action: &Action) -> DryRun {
    LocalExecutor::new(ProcessRunner::new(std::time::Duration::from_secs(5), 4096)).dry_run(action)
}

/// Runs actions on this machine through a [`Runner`] and allowlisted local paths.
#[derive(Debug)]
pub struct LocalExecutor<R> {
    runner: R,
    allowed_roots: Vec<PathBuf>,
}

impl<R: Runner + Send + Sync> LocalExecutor<R> {
    /// An executor over `runner` with the default allowlisted file roots.
    pub fn new(runner: R) -> Self {
        Self {
            runner,
            allowed_roots: default_allowed_roots(),
        }
    }

    /// An executor over `runner` with explicit allowlisted file roots (used in tests with
    /// `tempdir`).
    pub fn with_allowed_roots(runner: R, allowed_roots: Vec<PathBuf>) -> Self {
        Self {
            runner,
            allowed_roots,
        }
    }
}

fn systemctl(args: &[&str], name: &str) -> CommandPlan {
    let mut all: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
    all.push(name.to_owned());
    CommandPlan {
        program: "systemctl",
        args: all,
        env: Vec::new(),
        stdin: None,
    }
}

pub(crate) fn acme_issue_plan(provider: AcmeProvider, domain: &str, dry_run: bool) -> CommandPlan {
    match provider {
        AcmeProvider::LetsEncrypt => {
            let mut args = vec![
                "certonly".to_owned(),
                "--non-interactive".to_owned(),
                "--agree-tos".to_owned(),
                "--nginx".to_owned(),
            ];
            if dry_run {
                args.push("--dry-run".to_owned());
            }
            args.push("-d".to_owned());
            args.push(domain.to_owned());
            CommandPlan {
                program: "certbot",
                args,
                env: Vec::new(),
                stdin: None,
            }
        }
        AcmeProvider::ZeroSsl => {
            let mut args = vec![
                "--issue".to_owned(),
                "--server".to_owned(),
                "zerossl".to_owned(),
                "-d".to_owned(),
                domain.to_owned(),
                "--nginx".to_owned(),
            ];
            if dry_run {
                args.push("--staging".to_owned());
            }
            CommandPlan {
                program: "acme.sh",
                args,
                env: Vec::new(),
                stdin: None,
            }
        }
    }
}

pub(crate) fn acme_status_plan(provider: AcmeProvider, domain: &str) -> CommandPlan {
    match provider {
        AcmeProvider::LetsEncrypt => CommandPlan {
            program: "certbot",
            args: vec![
                "certificates".to_owned(),
                "-d".to_owned(),
                domain.to_owned(),
            ],
            env: Vec::new(),
            stdin: None,
        },
        AcmeProvider::ZeroSsl => CommandPlan {
            program: "acme.sh",
            args: vec!["--info".to_owned(), "-d".to_owned(), domain.to_owned()],
            env: Vec::new(),
            stdin: None,
        },
    }
}

pub(crate) fn acme_delete_plan(provider: AcmeProvider, domain: &str) -> CommandPlan {
    match provider {
        AcmeProvider::LetsEncrypt => CommandPlan {
            program: "certbot",
            args: vec![
                "delete".to_owned(),
                "--non-interactive".to_owned(),
                "--cert-name".to_owned(),
                domain.to_owned(),
            ],
            env: Vec::new(),
            stdin: None,
        },
        AcmeProvider::ZeroSsl => CommandPlan {
            program: "acme.sh",
            args: vec!["--remove".to_owned(), "-d".to_owned(), domain.to_owned()],
            env: Vec::new(),
            stdin: None,
        },
    }
}

pub(crate) fn db_backup_plan(engine: DbEngine, db_name: &str, output_path: &str) -> CommandPlan {
    match engine {
        DbEngine::Mysql => CommandPlan {
            program: "mysqldump",
            args: vec![
                "--single-transaction".to_owned(),
                "--quick".to_owned(),
                format!("--result-file={output_path}"),
                "--".to_owned(),
                db_name.to_owned(),
            ],
            env: Vec::new(),
            stdin: None,
        },
        DbEngine::Postgres => CommandPlan {
            program: "pg_dump",
            args: vec![
                "--format=custom".to_owned(),
                format!("--file={output_path}"),
                "--".to_owned(),
                db_name.to_owned(),
            ],
            env: Vec::new(),
            stdin: None,
        },
    }
}

pub(crate) fn validate_managed_dir(
    path_str: &str,
    allowed_roots: &[PathBuf],
) -> Result<PathBuf, &'static str> {
    match validate_managed_path(path_str, allowed_roots) {
        Ok(p) => Ok(p),
        Err("is_directory") => Ok(PathBuf::from(path_str)),
        Err(e) => Err(e),
    }
}

pub(crate) fn site_vhost_path(
    allowed_roots: &[PathBuf],
    domain: &str,
) -> Result<PathBuf, &'static str> {
    let base = allowed_roots.first().ok_or("path_not_allowed")?;
    let candidate = base.join("sites").join(format!("{domain}.conf"));
    let s = candidate.to_str().ok_or("path_not_allowed")?;
    validate_managed_path(s, allowed_roots)
}

/// Untrusted output made safe to keep: redacted, control characters (but newlines) replaced,
/// at most [`MAX_DETAIL`] characters.
pub(crate) fn safe_excerpt(bytes: &[u8]) -> String {
    let redacted = redact(&String::from_utf8_lossy(bytes)).text;
    redacted
        .chars()
        .map(|c| if c.is_control() && c != '\n' { ' ' } else { c })
        .take(MAX_DETAIL)
        .collect::<String>()
        .trim()
        .to_owned()
}

pub(crate) fn first_line(output: &RunOutput) -> String {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .chars()
        .take(40)
        .collect()
}

pub(crate) fn failure(reason: &'static str, detail: String) -> Outcome {
    Outcome {
        status: OutcomeStatus::Failed,
        verified: false,
        reason,
        detail,
    }
}

pub(crate) fn run_error_reason(error: transport::RunError) -> &'static str {
    match error {
        transport::RunError::NotInstalled => "not_installed",
        transport::RunError::Timeout => "timeout",
        transport::RunError::Io => "io",
    }
}

/// Validates that `path_str` is inside one of `allowed_roots`, has no `.` or `..` components,
/// and has no symlink along any component from the matching root down to the target.
pub(crate) fn validate_managed_path(
    path_str: &str,
    allowed_roots: &[PathBuf],
) -> Result<PathBuf, &'static str> {
    let path = Path::new(path_str);
    if !path.is_absolute() {
        return Err("path_not_allowed");
    }
    if path
        .components()
        .any(|c| matches!(c, Component::CurDir | Component::ParentDir))
    {
        return Err("path_not_allowed");
    }
    let Some(root) = allowed_roots
        .iter()
        .find(|r| path.starts_with(r) && path != r.as_path())
    else {
        return Err("path_not_allowed");
    };
    if let Ok(meta) = fs::symlink_metadata(root)
        && meta.file_type().is_symlink()
    {
        return Err("symlink_not_allowed");
    }
    let rel = path.strip_prefix(root).map_err(|_| "path_not_allowed")?;
    let mut cur = root.clone();
    for comp in rel.components() {
        cur.push(comp);
        match fs::symlink_metadata(&cur) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    return Err("symlink_not_allowed");
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("io"),
        }
    }
    if let Ok(meta) = fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            return Err("symlink_not_allowed");
        }
        if meta.is_dir() {
            return Err("is_directory");
        }
    }
    Ok(path.to_path_buf())
}

pub(crate) fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(char::from(HEX[(b >> 4) as usize]));
        out.push(char::from(HEX[(b & 0x0f) as usize]));
    }
    out
}

pub(crate) fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    let (chunks, _) = bytes.as_chunks::<2>();
    for &[a, b] in chunks {
        let hi = hex_nibble(a)?;
        let lo = hex_nibble(b)?;
        out.push((hi << 4) | lo);
    }
    Some(out)
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

pub(crate) fn write_backup_artifact(
    backup_dir: &Path,
    payload: &[u8],
) -> Result<BackupSnapshot, &'static str> {
    fs::create_dir_all(backup_dir).map_err(|_| "io")?;
    let seq = BACKUP_SEQ.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let file_path = backup_dir.join(format!("backup-{pid}-{seq}.json"));
    let mut file = fs::File::create(&file_path).map_err(|_| "io")?;
    file.write_all(payload).map_err(|_| "io")?;
    file.sync_all().map_err(|_| "io")?;
    let sha256 = db::sha256_hex(payload);
    Ok(BackupSnapshot {
        storage_ref: file_path.to_string_lossy().into_owned(),
        sha256,
    })
}

pub(crate) fn read_verified_backup_json(
    backup: &BackupSnapshot,
) -> Result<serde_json::Value, &'static str> {
    let payload = fs::read(&backup.storage_ref).map_err(|_| "backup_missing")?;
    if db::sha256_hex(&payload) != backup.sha256 {
        return Err("backup_corrupted");
    }
    serde_json::from_slice::<serde_json::Value>(&payload).map_err(|_| "backup_corrupted")
}

fn is_env_line_for_key(line: &str, key: &str) -> bool {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix("export ").unwrap_or(trimmed);
    rest.strip_prefix(key)
        .is_some_and(|after| after.starts_with('='))
}

fn env_key_present(content: &str, key: &str) -> bool {
    content.lines().any(|line| is_env_line_for_key(line, key))
}

fn apply_env_update(content: &str, key: &str, value: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut replaced = false;
    for line in content.lines() {
        if !replaced && is_env_line_for_key(line, key) {
            lines.push(format!("{key}={value}"));
            replaced = true;
        } else {
            lines.push(line.to_owned());
        }
    }
    if !replaced {
        lines.push(format!("{key}={value}"));
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

fn write_file_atomically(path: &Path, bytes: &[u8]) -> Result<(), &'static str> {
    let parent = path.parent().ok_or("io")?;
    fs::create_dir_all(parent).map_err(|_| "io")?;
    let seq = BACKUP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp_path = parent.join(format!(".nodivara-tmp-{}-{seq}", std::process::id()));
    {
        let mut f = fs::File::create(&tmp_path).map_err(|_| "io")?;
        f.write_all(bytes).map_err(|_| "io")?;
        f.sync_all().map_err(|_| "io")?;
    }
    fs::rename(&tmp_path, path).map_err(|_| {
        let _ = fs::remove_file(&tmp_path);
        "io"
    })
}

impl<R: Runner + Send + Sync> LocalExecutor<R> {
    fn service_state(&self, manager: ServiceManagerKind, name: &str) -> String {
        match manager {
            ServiceManagerKind::Systemd => {
                let check = systemctl(&["is-active"], name);
                match self.runner.run(&check) {
                    Ok(output) => {
                        let line = first_line(&output);
                        if line.is_empty() {
                            "unknown".to_owned()
                        } else {
                            line
                        }
                    }
                    Err(error) => run_error_reason(error).to_owned(),
                }
            }
        }
    }
}

impl<R: Runner + Send + Sync> Executor for LocalExecutor<R> {
    fn dry_run(&self, action: &Action) -> DryRun {
        match action {
            Action::RestartService { manager, name } => match manager {
                ServiceManagerKind::Systemd => {
                    let restart = systemctl(&["restart"], name.as_str());
                    let check = systemctl(&["is-active"], name.as_str());
                    let state = self.service_state(*manager, name.as_str());
                    DryRun {
                        steps: vec![restart.to_string(), check.to_string()],
                        current_state: state,
                        diff: None,
                    }
                }
            },
            Action::UpdateEnvVar { path, key, .. } => {
                let checked = validate_managed_path(path.as_str(), &self.allowed_roots);
                let (state, present) = match checked {
                    Ok(p) => match fs::read_to_string(&p) {
                        Ok(content) => {
                            let has_key = env_key_present(&content, key.as_str());
                            (
                                if has_key { "present" } else { "absent" }.to_owned(),
                                has_key,
                            )
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                            ("file_absent".to_owned(), false)
                        }
                        Err(_) => ("io".to_owned(), false),
                    },
                    Err(reason) => (reason.to_owned(), false),
                };
                let diff = if present {
                    format!("- {k}=[REDACTED]\n+ {k}=[REDACTED]", k = key.as_str())
                } else {
                    format!("+ {k}=[REDACTED]", k = key.as_str())
                };
                DryRun {
                    steps: vec![format!(
                        "update env {} in {} (value redacted)",
                        key.as_str(),
                        path.as_str()
                    )],
                    current_state: state,
                    diff: Some(diff),
                }
            }
            Action::DeleteManagedFile { path } => {
                let checked = validate_managed_path(path.as_str(), &self.allowed_roots);
                let state = match checked {
                    Ok(p) => match fs::symlink_metadata(&p) {
                        Ok(meta) => format!("present ({} bytes)", meta.len()),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => "absent".to_owned(),
                        Err(_) => "io".to_owned(),
                    },
                    Err(reason) => reason.to_owned(),
                };
                DryRun {
                    steps: vec![format!("delete managed file {}", path.as_str())],
                    current_state: state.clone(),
                    diff: Some(format!("--- {}\n+++ /dev/null\n({})", path.as_str(), state)),
                }
            }
            Action::ConfigureSite {
                domain,
                upstream_port,
                php_version,
                tls_enabled,
                root_dir,
            } => {
                let root_ok = validate_managed_dir(root_dir.as_str(), &self.allowed_roots);
                let vhost = site_vhost_path(&self.allowed_roots, domain.as_str());
                let state = match (&root_ok, &vhost) {
                    (Err(r), _) | (_, Err(r)) => (*r).to_owned(),
                    (Ok(_), Ok(p)) => match fs::symlink_metadata(p) {
                        Ok(_) => "present".to_owned(),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => "absent".to_owned(),
                        Err(_) => "io".to_owned(),
                    },
                };
                let rendered = render_site_vhost_config(
                    domain,
                    *upstream_port,
                    *php_version,
                    *tls_enabled,
                    root_dir,
                );
                let vhost_label = vhost.as_ref().map_or_else(
                    |_| domain.as_str().to_owned(),
                    |p| p.to_string_lossy().into_owned(),
                );
                DryRun {
                    steps: vec![format!("write site vhost config {vhost_label}")],
                    current_state: state,
                    diff: Some(rendered),
                }
            }
            Action::IssueTlsCert {
                domain,
                acme_provider,
            } => {
                let dry_plan = acme_issue_plan(*acme_provider, domain.as_str(), true);
                let live_plan = acme_issue_plan(*acme_provider, domain.as_str(), false);
                let status_plan = acme_status_plan(*acme_provider, domain.as_str());
                let state = match self.runner.run(&status_plan) {
                    Ok(out) if out.exit_code == Some(0) => "present".to_owned(),
                    Ok(_) => "absent".to_owned(),
                    Err(e) => run_error_reason(e).to_owned(),
                };
                DryRun {
                    steps: vec![dry_plan.to_string(), live_plan.to_string()],
                    current_state: state,
                    diff: None,
                }
            }
            Action::BackupDatabase {
                engine,
                db_name,
                output_path,
            } => {
                let plan = db_backup_plan(*engine, db_name.as_str(), output_path.as_str());
                let state = match validate_managed_path(output_path.as_str(), &self.allowed_roots) {
                    Ok(p) => match fs::symlink_metadata(&p) {
                        Ok(meta) => format!("present ({} bytes)", meta.len()),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => "absent".to_owned(),
                        Err(_) => "io".to_owned(),
                    },
                    Err(reason) => reason.to_owned(),
                };
                DryRun {
                    steps: vec![plan.to_string()],
                    current_state: state,
                    diff: None,
                }
            }
        }
    }

    fn backup(&self, action: &Action, backup_dir: &Path) -> Result<BackupSnapshot, &'static str> {
        match action {
            Action::RestartService { manager, name } => {
                let state = self.service_state(*manager, name.as_str());
                let payload = serde_json::json!({
                    "kind": "service",
                    "service": name.as_str(),
                    "state": state,
                })
                .to_string();
                write_backup_artifact(backup_dir, payload.as_bytes())
            }
            Action::UpdateEnvVar { path, .. } => {
                let p = validate_managed_path(path.as_str(), &self.allowed_roots)?;
                let (existed, hex) = match fs::read(&p) {
                    Ok(bytes) => (true, encode_hex(&bytes)),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => (false, String::new()),
                    Err(_) => return Err("io"),
                };
                let payload = serde_json::json!({
                    "kind": "file",
                    "existed": existed,
                    "hex": hex,
                })
                .to_string();
                write_backup_artifact(backup_dir, payload.as_bytes())
            }
            Action::DeleteManagedFile { path } => {
                let p = validate_managed_path(path.as_str(), &self.allowed_roots)?;
                let bytes = match fs::read(&p) {
                    Ok(bytes) => bytes,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err("not_found"),
                    Err(_) => return Err("io"),
                };
                let payload = serde_json::json!({
                    "kind": "file",
                    "existed": true,
                    "hex": encode_hex(&bytes),
                })
                .to_string();
                write_backup_artifact(backup_dir, payload.as_bytes())
            }
            Action::ConfigureSite {
                domain, root_dir, ..
            } => {
                let _ = validate_managed_dir(root_dir.as_str(), &self.allowed_roots)?;
                let p = site_vhost_path(&self.allowed_roots, domain.as_str())?;
                let (existed, hex) = match fs::read(&p) {
                    Ok(bytes) => (true, encode_hex(&bytes)),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => (false, String::new()),
                    Err(_) => return Err("io"),
                };
                let payload = serde_json::json!({
                    "kind": "file",
                    "existed": existed,
                    "hex": hex,
                })
                .to_string();
                write_backup_artifact(backup_dir, payload.as_bytes())
            }
            Action::IssueTlsCert {
                domain,
                acme_provider,
            } => {
                let status_plan = acme_status_plan(*acme_provider, domain.as_str());
                let existed = match self.runner.run(&status_plan) {
                    Ok(out) => out.exit_code == Some(0),
                    Err(_) => false,
                };
                let payload = serde_json::json!({
                    "kind": "tls_cert",
                    "domain": domain.as_str(),
                    "existed": existed,
                })
                .to_string();
                write_backup_artifact(backup_dir, payload.as_bytes())
            }
            Action::BackupDatabase { output_path, .. } => {
                let p = validate_managed_path(output_path.as_str(), &self.allowed_roots)?;
                let (existed, hex) = match fs::read(&p) {
                    Ok(bytes) => (true, encode_hex(&bytes)),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => (false, String::new()),
                    Err(_) => return Err("io"),
                };
                let payload = serde_json::json!({
                    "kind": "file",
                    "existed": existed,
                    "hex": hex,
                })
                .to_string();
                write_backup_artifact(backup_dir, payload.as_bytes())
            }
        }
    }

    fn execute(&self, action: &Action) -> Outcome {
        match action {
            Action::RestartService { manager, name } => match manager {
                ServiceManagerKind::Systemd => {
                    match self.runner.run(&systemctl(&["restart"], name.as_str())) {
                        Err(error) => return failure(run_error_reason(error), String::new()),
                        Ok(output) if output.exit_code != Some(0) => {
                            return failure("command_failed", safe_excerpt(&output.stderr));
                        }
                        Ok(_) => {}
                    }
                    match self.runner.run(&systemctl(&["is-active"], name.as_str())) {
                        Err(error) => failure(run_error_reason(error), String::new()),
                        Ok(output)
                            if output.exit_code == Some(0) && first_line(&output) == "active" =>
                        {
                            Outcome {
                                status: OutcomeStatus::Succeeded,
                                verified: true,
                                reason: "ok",
                                detail: String::new(),
                            }
                        }
                        Ok(output) => failure("not_active_after", first_line(&output)),
                    }
                }
            },
            Action::UpdateEnvVar { path, key, value } => {
                let p = match validate_managed_path(path.as_str(), &self.allowed_roots) {
                    Ok(p) => p,
                    Err(reason) => return failure(reason, String::new()),
                };
                let existing = match fs::read_to_string(&p) {
                    Ok(s) => s,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
                    Err(_) => return failure("io", String::new()),
                };
                let updated = apply_env_update(&existing, key.as_str(), value.expose());
                if let Err(reason) = write_file_atomically(&p, updated.as_bytes()) {
                    return failure(reason, String::new());
                }
                let expected_line = format!("{}={}", key.as_str(), value.expose());
                let verified = fs::read_to_string(&p)
                    .map(|after| after.lines().any(|line| line == expected_line))
                    .unwrap_or(false);
                if verified {
                    Outcome {
                        status: OutcomeStatus::Succeeded,
                        verified: true,
                        reason: "ok",
                        detail: String::new(),
                    }
                } else {
                    failure("verify_failed", String::new())
                }
            }
            Action::DeleteManagedFile { path } => {
                let p = match validate_managed_path(path.as_str(), &self.allowed_roots) {
                    Ok(p) => p,
                    Err(reason) => return failure(reason, String::new()),
                };
                match fs::symlink_metadata(&p) {
                    Ok(meta) if meta.file_type().is_symlink() => {
                        return failure("symlink_not_allowed", String::new());
                    }
                    Ok(meta) if meta.is_dir() => {
                        return failure("is_directory", String::new());
                    }
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        return failure("not_found", String::new());
                    }
                    Err(_) => return failure("io", String::new()),
                }
                if fs::remove_file(&p).is_err() {
                    return failure("io", String::new());
                }
                if p.exists() {
                    failure("verify_failed", String::new())
                } else {
                    Outcome {
                        status: OutcomeStatus::Succeeded,
                        verified: true,
                        reason: "ok",
                        detail: String::new(),
                    }
                }
            }
            Action::ConfigureSite {
                domain,
                upstream_port,
                php_version,
                tls_enabled,
                root_dir,
            } => {
                if let Err(reason) = validate_managed_dir(root_dir.as_str(), &self.allowed_roots) {
                    return failure(reason, String::new());
                }
                let p = match site_vhost_path(&self.allowed_roots, domain.as_str()) {
                    Ok(p) => p,
                    Err(reason) => return failure(reason, String::new()),
                };
                let content = render_site_vhost_config(
                    domain,
                    *upstream_port,
                    *php_version,
                    *tls_enabled,
                    root_dir,
                );
                if let Err(reason) = write_file_atomically(&p, content.as_bytes()) {
                    return failure(reason, String::new());
                }
                let expected = format!("server_name {};", domain.as_str());
                let verified = fs::read_to_string(&p)
                    .map(|after| after.contains(&expected))
                    .unwrap_or(false);
                if verified {
                    Outcome {
                        status: OutcomeStatus::Succeeded,
                        verified: true,
                        reason: "ok",
                        detail: String::new(),
                    }
                } else {
                    failure("verify_failed", String::new())
                }
            }
            Action::IssueTlsCert {
                domain,
                acme_provider,
            } => {
                let plan = acme_issue_plan(*acme_provider, domain.as_str(), false);
                match self.runner.run(&plan) {
                    Err(error) => return failure(run_error_reason(error), String::new()),
                    Ok(output) if output.exit_code != Some(0) => {
                        return failure("command_failed", safe_excerpt(&output.stderr));
                    }
                    Ok(_) => {}
                }
                let verify_plan = acme_status_plan(*acme_provider, domain.as_str());
                match self.runner.run(&verify_plan) {
                    Err(error) => failure(run_error_reason(error), String::new()),
                    Ok(output) if output.exit_code == Some(0) => Outcome {
                        status: OutcomeStatus::Succeeded,
                        verified: true,
                        reason: "ok",
                        detail: safe_excerpt(&output.stdout),
                    },
                    Ok(output) => failure("verify_failed", safe_excerpt(&output.stderr)),
                }
            }
            Action::BackupDatabase {
                engine,
                db_name,
                output_path,
            } => {
                let p = match validate_managed_path(output_path.as_str(), &self.allowed_roots) {
                    Ok(p) => p,
                    Err(reason) => return failure(reason, String::new()),
                };
                if let Some(parent) = p.parent()
                    && fs::create_dir_all(parent).is_err()
                {
                    return failure("io", String::new());
                }
                let plan = db_backup_plan(*engine, db_name.as_str(), output_path.as_str());
                match self.runner.run(&plan) {
                    Err(error) => return failure(run_error_reason(error), String::new()),
                    Ok(output) if output.exit_code != Some(0) => {
                        return failure("command_failed", safe_excerpt(&output.stderr));
                    }
                    Ok(_) => {}
                }
                let verify_plan = CommandPlan {
                    program: "test",
                    args: vec!["-s".to_owned(), output_path.as_str().to_owned()],
                    env: Vec::new(),
                    stdin: None,
                };
                match self.runner.run(&verify_plan) {
                    Err(error) => failure(run_error_reason(error), String::new()),
                    Ok(output) if output.exit_code == Some(0) => Outcome {
                        status: OutcomeStatus::Succeeded,
                        verified: true,
                        reason: "ok",
                        detail: String::new(),
                    },
                    Ok(output) => failure("verify_failed", safe_excerpt(&output.stderr)),
                }
            }
        }
    }

    fn rollback(&self, action: &Action, backup: &BackupSnapshot) -> Outcome {
        let Ok(payload) = fs::read(&backup.storage_ref) else {
            return failure("backup_missing", String::new());
        };
        if db::sha256_hex(&payload) != backup.sha256 {
            return failure("backup_corrupted", String::new());
        }
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&payload) else {
            return failure("backup_corrupted", String::new());
        };
        match action {
            Action::RestartService { manager, name } => {
                let prev_state = value
                    .get("state")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown");
                match manager {
                    ServiceManagerKind::Systemd => {
                        let cmd = if prev_state == "active" {
                            "restart"
                        } else {
                            "stop"
                        };
                        match self.runner.run(&systemctl(&[cmd], name.as_str())) {
                            Err(error) => return failure(run_error_reason(error), String::new()),
                            Ok(output) if output.exit_code != Some(0) => {
                                return failure("command_failed", safe_excerpt(&output.stderr));
                            }
                            Ok(_) => {}
                        }
                        if prev_state == "active" {
                            match self.runner.run(&systemctl(&["is-active"], name.as_str())) {
                                Err(error) => failure(run_error_reason(error), String::new()),
                                Ok(output)
                                    if output.exit_code == Some(0)
                                        && first_line(&output) == "active" =>
                                {
                                    Outcome {
                                        status: OutcomeStatus::Succeeded,
                                        verified: true,
                                        reason: "ok",
                                        detail: String::new(),
                                    }
                                }
                                Ok(output) => failure("not_active_after", first_line(&output)),
                            }
                        } else {
                            Outcome {
                                status: OutcomeStatus::Succeeded,
                                verified: true,
                                reason: "ok",
                                detail: String::new(),
                            }
                        }
                    }
                }
            }
            Action::UpdateEnvVar { path, .. }
            | Action::DeleteManagedFile { path }
            | Action::BackupDatabase {
                output_path: path, ..
            } => {
                let p = match validate_managed_path(path.as_str(), &self.allowed_roots) {
                    Ok(p) => p,
                    Err(reason) => return failure(reason, String::new()),
                };
                restore_managed_file(&p, &value)
            }
            Action::ConfigureSite { domain, .. } => {
                let p = match site_vhost_path(&self.allowed_roots, domain.as_str()) {
                    Ok(p) => p,
                    Err(reason) => return failure(reason, String::new()),
                };
                restore_managed_file(&p, &value)
            }
            Action::IssueTlsCert {
                domain,
                acme_provider,
            } => {
                let Some(existed) = value.get("existed").and_then(serde_json::Value::as_bool)
                else {
                    return failure("backup_corrupted", String::new());
                };
                if existed {
                    return Outcome {
                        status: OutcomeStatus::Succeeded,
                        verified: true,
                        reason: "ok",
                        detail: String::new(),
                    };
                }
                let del_plan = acme_delete_plan(*acme_provider, domain.as_str());
                match self.runner.run(&del_plan) {
                    Err(error) => failure(run_error_reason(error), String::new()),
                    Ok(output) if output.exit_code == Some(0) => Outcome {
                        status: OutcomeStatus::Succeeded,
                        verified: true,
                        reason: "ok",
                        detail: String::new(),
                    },
                    Ok(output) => failure("command_failed", safe_excerpt(&output.stderr)),
                }
            }
        }
    }
}

fn restore_managed_file(p: &Path, value: &serde_json::Value) -> Outcome {
    let Some(existed) = value.get("existed").and_then(serde_json::Value::as_bool) else {
        return failure("backup_corrupted", String::new());
    };
    if existed {
        let Some(hex) = value.get("hex").and_then(serde_json::Value::as_str) else {
            return failure("backup_corrupted", String::new());
        };
        let Some(orig_bytes) = decode_hex(hex) else {
            return failure("backup_corrupted", String::new());
        };
        if let Err(reason) = write_file_atomically(p, &orig_bytes) {
            return failure(reason, String::new());
        }
        let verified = fs::read(p).map(|now| now == orig_bytes).unwrap_or(false);
        if verified {
            Outcome {
                status: OutcomeStatus::Succeeded,
                verified: true,
                reason: "ok",
                detail: String::new(),
            }
        } else {
            failure("verify_failed", String::new())
        }
    } else {
        match fs::remove_file(p) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return failure("io", String::new()),
        }
        if p.exists() {
            failure("verify_failed", String::new())
        } else {
            Outcome {
                status: OutcomeStatus::Succeeded,
                verified: true,
                reason: "ok",
                detail: String::new(),
            }
        }
    }
}
