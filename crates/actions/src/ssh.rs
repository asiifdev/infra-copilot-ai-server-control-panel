//! Remote SSH executor: runs approved [`Action`]s on a remote SSH node via argv-only `ssh`
//! commands through a [`Runner`]. Never uses shell interpolation.

use crate::exec::{
    acme_delete_plan, acme_issue_plan, acme_status_plan, db_backup_plan, decode_hex, encode_hex,
    failure, first_line, read_verified_backup_json, run_error_reason, safe_excerpt,
    write_backup_artifact,
};
use crate::sites::render_site_vhost_config;
use crate::{Action, BackupSnapshot, DryRun, Executor, Outcome, OutcomeStatus, ServiceManagerKind};
use serde_json::json;
use std::path::{Path, PathBuf};
use transport::{CommandPlan, RunOutput, Runner, Secret, StdinData};

/// Connection target for [`SshExecutor`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshTarget {
    /// Remote hostname or IP (validated: non-empty, no whitespace, no leading `-`).
    pub host: String,
    /// SSH port (default `22`).
    pub port: u16,
    /// Remote SSH user (validated: non-empty, no whitespace, no leading `-`).
    pub user: String,
    /// Optional private key path (`-i`).
    pub identity_file: Option<PathBuf>,
}

impl SshTarget {
    /// Builds a validated [`SshTarget`].
    ///
    /// # Errors
    /// Returns `"invalid_ssh_target"` if `host` or `user` is empty, starts with `-`, or contains
    /// whitespace or control characters, or if `port == 0`.
    pub fn new(
        host: impl Into<String>,
        port: u16,
        user: impl Into<String>,
        identity_file: Option<PathBuf>,
    ) -> Result<Self, &'static str> {
        let host = host.into();
        let user = user.into();
        if !is_safe_token(&host) || !is_safe_token(&user) || port == 0 {
            return Err("invalid_ssh_target");
        }
        Ok(Self {
            host,
            port,
            user,
            identity_file,
        })
    }

    /// Parses an [`SshTarget`] from a node's `name` and `labels_json`.
    ///
    /// Looks for `ssh_host` (falls back to `node_name`), `ssh_port` (default `22`), and
    /// `ssh_user` (default `"root"`).
    ///
    /// # Errors
    /// Returns `"invalid_ssh_target"` if the resolved values fail validation.
    pub fn from_node_labels(node_name: &str, labels_json: &str) -> Result<Self, &'static str> {
        let parsed = serde_json::from_str::<serde_json::Value>(labels_json)
            .unwrap_or(serde_json::Value::Null);
        let host = parsed
            .get("ssh_host")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(node_name);
        let user = parsed
            .get("ssh_user")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("root");
        let port = parsed
            .get("ssh_port")
            .and_then(serde_json::Value::as_u64)
            .and_then(|p| u16::try_from(p).ok())
            .unwrap_or(22);
        Self::new(host, port, user, None)
    }

    fn destination(&self) -> String {
        format!("{}@{}", self.user, self.host)
    }
}

fn is_safe_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 253
        && !value.starts_with('-')
        && !value
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '@' | ':' | '/' | '\\'))
}

/// Runs approved actions on a remote SSH host via argv-only `ssh` invocations.
#[derive(Debug)]
pub struct SshExecutor<R> {
    runner: R,
    target: SshTarget,
}

impl<R: Runner + Send + Sync> SshExecutor<R> {
    /// Creates an [`SshExecutor`] over `runner` targeting `target`.
    pub fn new(runner: R, target: SshTarget) -> Self {
        Self { runner, target }
    }

    /// Builds an argv-only `ssh` [`CommandPlan`] with hardened non-interactive options.
    #[must_use]
    pub fn ssh_plan(&self, remote_argv: &[&str], stdin: Option<StdinData>) -> CommandPlan {
        let mut args = vec![
            "-o".to_owned(),
            "BatchMode=yes".to_owned(),
            "-o".to_owned(),
            "StrictHostKeyChecking=accept-new".to_owned(),
            "-o".to_owned(),
            "ConnectTimeout=10".to_owned(),
            "-p".to_owned(),
            self.target.port.to_string(),
        ];
        if let Some(ref key) = self.target.identity_file {
            args.push("-i".to_owned());
            args.push(key.to_string_lossy().into_owned());
        }
        args.push("--".to_owned());
        args.push(self.target.destination());
        for arg in remote_argv {
            args.push((*arg).to_owned());
        }
        CommandPlan {
            program: "ssh",
            args,
            env: Vec::new(),
            stdin,
        }
    }

    fn run_remote(
        &self,
        remote_argv: &[&str],
        stdin: Option<StdinData>,
    ) -> Result<RunOutput, &'static str> {
        self.runner
            .run(&self.ssh_plan(remote_argv, stdin))
            .map_err(run_error_reason)
    }

    fn remote_service_state(&self, manager: ServiceManagerKind, name: &str) -> String {
        match manager {
            ServiceManagerKind::Systemd => {
                match self.run_remote(&["systemctl", "is-active", name], None) {
                    Ok(out) => {
                        let line = first_line(&out);
                        if line.is_empty() {
                            "unknown".to_owned()
                        } else {
                            line
                        }
                    }
                    Err(code) => code.to_owned(),
                }
            }
        }
    }

    fn run_remote_plan(&self, plan: CommandPlan) -> Result<RunOutput, &'static str> {
        let mut argv: Vec<&str> = Vec::with_capacity(1 + plan.args.len());
        argv.push(plan.program);
        for arg in &plan.args {
            argv.push(arg.as_str());
        }
        self.run_remote(&argv, plan.stdin)
    }

    fn verify_service_active(&self, name: &str) -> Outcome {
        match self.run_remote(&["systemctl", "is-active", name], None) {
            Err(reason) => failure(reason, String::new()),
            Ok(out) if out.exit_code == Some(0) && first_line(&out) == "active" => Outcome {
                status: OutcomeStatus::Succeeded,
                verified: true,
                reason: "ok",
                detail: String::new(),
            },
            Ok(out) => failure("not_active_after", first_line(&out)),
        }
    }
}

impl<R: Runner + Send + Sync> Executor for SshExecutor<R> {
    fn dry_run(&self, action: &Action) -> DryRun {
        let dest = self.target.destination();
        match action {
            Action::RestartService { manager, name } => DryRun {
                steps: vec![
                    format!("ssh {dest} systemctl restart {}", name.as_str()),
                    format!("ssh {dest} systemctl is-active {}", name.as_str()),
                ],
                current_state: self.remote_service_state(*manager, name.as_str()),
                diff: None,
            },
            Action::UpdateEnvVar { path, key, .. } => {
                let exists = self
                    .run_remote(&["test", "-f", path.as_str()], None)
                    .is_ok_and(|o| o.exit_code == Some(0));
                DryRun {
                    steps: vec![format!(
                        "ssh {dest} update_env_var {} {}=[REDACTED]",
                        path.as_str(),
                        key.as_str()
                    )],
                    current_state: if exists {
                        "present".to_owned()
                    } else {
                        "missing".to_owned()
                    },
                    diff: Some(format!("~ {}=[REDACTED]", key.as_str())),
                }
            }
            Action::DeleteManagedFile { path } => {
                let exists = self
                    .run_remote(&["test", "-f", path.as_str()], None)
                    .is_ok_and(|o| o.exit_code == Some(0));
                DryRun {
                    steps: vec![
                        format!("ssh {dest} rm -f -- {}", path.as_str()),
                        format!("ssh {dest} test ! -e {}", path.as_str()),
                    ],
                    current_state: if exists {
                        "present".to_owned()
                    } else {
                        "missing".to_owned()
                    },
                    diff: Some(format!("- {}", path.as_str())),
                }
            }
            Action::ConfigureSite {
                domain,
                upstream_port,
                php_version,
                tls_enabled,
                root_dir,
            } => {
                let conf_path = format!("/etc/nodivara/sites/{}.conf", domain.as_str());
                let exists = self
                    .run_remote(&["test", "-f", &conf_path], None)
                    .is_ok_and(|o| o.exit_code == Some(0));
                let rendered = render_site_vhost_config(
                    domain,
                    *upstream_port,
                    *php_version,
                    *tls_enabled,
                    root_dir,
                );
                DryRun {
                    steps: vec![format!("ssh {dest} tee {conf_path}")],
                    current_state: if exists {
                        "present".to_owned()
                    } else {
                        "missing".to_owned()
                    },
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
                let exists = self
                    .run_remote_plan(status_plan)
                    .is_ok_and(|o| o.exit_code == Some(0));
                DryRun {
                    steps: vec![
                        format!("ssh {dest} {dry_plan}"),
                        format!("ssh {dest} {live_plan}"),
                    ],
                    current_state: if exists {
                        "present".to_owned()
                    } else {
                        "missing".to_owned()
                    },
                    diff: None,
                }
            }
            Action::BackupDatabase {
                engine,
                db_name,
                output_path,
            } => {
                let plan = db_backup_plan(*engine, db_name.as_str(), output_path.as_str());
                let exists = self
                    .run_remote(&["test", "-f", output_path.as_str()], None)
                    .is_ok_and(|o| o.exit_code == Some(0));
                DryRun {
                    steps: vec![format!("ssh {dest} {plan}")],
                    current_state: if exists {
                        "present".to_owned()
                    } else {
                        "missing".to_owned()
                    },
                    diff: None,
                }
            }
        }
    }

    fn backup(&self, action: &Action, backup_dir: &Path) -> Result<BackupSnapshot, &'static str> {
        let payload = match action {
            Action::RestartService { manager, name } => json!({
                "kind": "ssh_restart_service",
                "host": self.target.host,
                "service": name.as_str(),
                "state": self.remote_service_state(*manager, name.as_str()),
            })
            .to_string()
            .into_bytes(),
            Action::UpdateEnvVar { path, .. }
            | Action::DeleteManagedFile { path }
            | Action::BackupDatabase {
                output_path: path, ..
            } => {
                let out = self.run_remote(&["cat", path.as_str()], None)?;
                let existed = out.exit_code == Some(0);
                let hex = if existed {
                    encode_hex(&out.stdout)
                } else {
                    String::new()
                };
                json!({
                    "kind": "ssh_file",
                    "host": self.target.host,
                    "path": path.as_str(),
                    "existed": existed,
                    "hex": hex,
                })
                .to_string()
                .into_bytes()
            }
            Action::ConfigureSite { domain, .. } => {
                let conf_path = format!("/etc/nodivara/sites/{}.conf", domain.as_str());
                let out = self.run_remote(&["cat", &conf_path], None)?;
                let existed = out.exit_code == Some(0);
                let hex = if existed {
                    encode_hex(&out.stdout)
                } else {
                    String::new()
                };
                json!({
                    "kind": "ssh_file",
                    "host": self.target.host,
                    "path": conf_path,
                    "existed": existed,
                    "hex": hex,
                })
                .to_string()
                .into_bytes()
            }
            Action::IssueTlsCert {
                domain,
                acme_provider,
            } => {
                let status_plan = acme_status_plan(*acme_provider, domain.as_str());
                let existed = self
                    .run_remote_plan(status_plan)
                    .is_ok_and(|o| o.exit_code == Some(0));
                json!({
                    "kind": "ssh_tls_cert",
                    "host": self.target.host,
                    "domain": domain.as_str(),
                    "existed": existed,
                })
                .to_string()
                .into_bytes()
            }
        };
        write_backup_artifact(backup_dir, &payload)
    }

    fn execute(&self, action: &Action) -> Outcome {
        match action {
            Action::RestartService { manager, name } => match manager {
                ServiceManagerKind::Systemd => {
                    match self.run_remote(&["systemctl", "restart", name.as_str()], None) {
                        Err(reason) => return failure(reason, String::new()),
                        Ok(out) if out.exit_code != Some(0) => {
                            return failure("command_failed", safe_excerpt(&out.stderr));
                        }
                        Ok(_) => {}
                    }
                    self.verify_service_active(name.as_str())
                }
            },
            Action::UpdateEnvVar { path, key, value } => {
                match self.run_remote(
                    &["nodivara-remote", "update-env", path.as_str(), key.as_str()],
                    Some(StdinData::Secret(Secret::new(value.0.clone()))),
                ) {
                    Err(reason) => failure(reason, String::new()),
                    Ok(out) if out.exit_code != Some(0) => {
                        failure("command_failed", safe_excerpt(&out.stderr))
                    }
                    Ok(_) => Outcome {
                        status: OutcomeStatus::Succeeded,
                        verified: true,
                        reason: "ok",
                        detail: String::new(),
                    },
                }
            }
            Action::DeleteManagedFile { path } => {
                match self.run_remote(&["rm", "-f", "--", path.as_str()], None) {
                    Err(reason) => return failure(reason, String::new()),
                    Ok(out) if out.exit_code != Some(0) => {
                        return failure("command_failed", safe_excerpt(&out.stderr));
                    }
                    Ok(_) => {}
                }
                match self.run_remote(&["test", "!", "-e", path.as_str()], None) {
                    Err(reason) => failure(reason, String::new()),
                    Ok(out) if out.exit_code == Some(0) => Outcome {
                        status: OutcomeStatus::Succeeded,
                        verified: true,
                        reason: "ok",
                        detail: String::new(),
                    },
                    Ok(out) => failure("verify_failed", safe_excerpt(&out.stderr)),
                }
            }
            Action::ConfigureSite {
                domain,
                upstream_port,
                php_version,
                tls_enabled,
                root_dir,
            } => {
                let conf_path = format!("/etc/nodivara/sites/{}.conf", domain.as_str());
                let rendered = render_site_vhost_config(
                    domain,
                    *upstream_port,
                    *php_version,
                    *tls_enabled,
                    root_dir,
                );
                match self.run_remote(
                    &["tee", &conf_path],
                    Some(StdinData::Bytes(rendered.into_bytes())),
                ) {
                    Err(reason) => return failure(reason, String::new()),
                    Ok(out) if out.exit_code != Some(0) => {
                        return failure("command_failed", safe_excerpt(&out.stderr));
                    }
                    Ok(_) => {}
                }
                match self.run_remote(&["test", "-s", &conf_path], None) {
                    Err(reason) => failure(reason, String::new()),
                    Ok(out) if out.exit_code == Some(0) => Outcome {
                        status: OutcomeStatus::Succeeded,
                        verified: true,
                        reason: "ok",
                        detail: String::new(),
                    },
                    Ok(out) => failure("verify_failed", safe_excerpt(&out.stderr)),
                }
            }
            Action::IssueTlsCert {
                domain,
                acme_provider,
            } => {
                let plan = acme_issue_plan(*acme_provider, domain.as_str(), false);
                match self.run_remote_plan(plan) {
                    Err(reason) => return failure(reason, String::new()),
                    Ok(out) if out.exit_code != Some(0) => {
                        return failure("command_failed", safe_excerpt(&out.stderr));
                    }
                    Ok(_) => {}
                }
                let verify = acme_status_plan(*acme_provider, domain.as_str());
                match self.run_remote_plan(verify) {
                    Err(reason) => failure(reason, String::new()),
                    Ok(out) if out.exit_code == Some(0) => Outcome {
                        status: OutcomeStatus::Succeeded,
                        verified: true,
                        reason: "ok",
                        detail: safe_excerpt(&out.stdout),
                    },
                    Ok(out) => failure("verify_failed", safe_excerpt(&out.stderr)),
                }
            }
            Action::BackupDatabase {
                engine,
                db_name,
                output_path,
            } => {
                let plan = db_backup_plan(*engine, db_name.as_str(), output_path.as_str());
                match self.run_remote_plan(plan) {
                    Err(reason) => return failure(reason, String::new()),
                    Ok(out) if out.exit_code != Some(0) => {
                        return failure("command_failed", safe_excerpt(&out.stderr));
                    }
                    Ok(_) => {}
                }
                match self.run_remote(&["test", "-s", output_path.as_str()], None) {
                    Err(reason) => failure(reason, String::new()),
                    Ok(out) if out.exit_code == Some(0) => Outcome {
                        status: OutcomeStatus::Succeeded,
                        verified: true,
                        reason: "ok",
                        detail: String::new(),
                    },
                    Ok(out) => failure("verify_failed", safe_excerpt(&out.stderr)),
                }
            }
        }
    }

    fn rollback(&self, action: &Action, backup: &BackupSnapshot) -> Outcome {
        let value = match read_verified_backup_json(backup) {
            Ok(v) => v,
            Err(reason) => return failure(reason, String::new()),
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
                        match self.run_remote(&["systemctl", cmd, name.as_str()], None) {
                            Err(reason) => return failure(reason, String::new()),
                            Ok(out) if out.exit_code != Some(0) => {
                                return failure("command_failed", safe_excerpt(&out.stderr));
                            }
                            Ok(_) => {}
                        }
                        if prev_state == "active" {
                            self.verify_service_active(name.as_str())
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
            } => self.rollback_remote_file(path.as_str(), &value),
            Action::ConfigureSite { domain, .. } => {
                let conf_path = format!("/etc/nodivara/sites/{}.conf", domain.as_str());
                self.rollback_remote_file(&conf_path, &value)
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
                match self.run_remote_plan(del_plan) {
                    Err(reason) => failure(reason, String::new()),
                    Ok(out) if out.exit_code == Some(0) => Outcome {
                        status: OutcomeStatus::Succeeded,
                        verified: true,
                        reason: "ok",
                        detail: String::new(),
                    },
                    Ok(out) => failure("command_failed", safe_excerpt(&out.stderr)),
                }
            }
        }
    }
}

impl<R: Runner + Send + Sync> SshExecutor<R> {
    fn rollback_remote_file(&self, path: &str, value: &serde_json::Value) -> Outcome {
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
            match self.run_remote(&["tee", path], Some(StdinData::Bytes(orig_bytes))) {
                Err(reason) => failure(reason, String::new()),
                Ok(out) if out.exit_code != Some(0) => {
                    failure("command_failed", safe_excerpt(&out.stderr))
                }
                Ok(_) => Outcome {
                    status: OutcomeStatus::Succeeded,
                    verified: true,
                    reason: "ok",
                    detail: String::new(),
                },
            }
        } else {
            match self.run_remote(&["rm", "-f", "--", path], None) {
                Err(reason) => failure(reason, String::new()),
                Ok(out) if out.exit_code != Some(0) => {
                    failure("command_failed", safe_excerpt(&out.stderr))
                }
                Ok(_) => Outcome {
                    status: OutcomeStatus::Succeeded,
                    verified: true,
                    reason: "ok",
                    detail: String::new(),
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDir;
    use crate::{EnvKey, EnvValue, ManagedPath, ServiceName};
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::time::Duration;
    use transport::{RunError, RunOutput};

    #[derive(Clone)]
    struct RecordedPlan {
        program: &'static str,
        args: Vec<String>,
        stdin: Option<Vec<u8>>,
    }

    struct FakeRunner {
        plans: Mutex<Vec<RecordedPlan>>,
        replies: Mutex<VecDeque<Result<RunOutput, RunError>>>,
    }

    impl FakeRunner {
        fn with_replies(replies: Vec<Result<RunOutput, RunError>>) -> Self {
            Self {
                plans: Mutex::new(Vec::new()),
                replies: Mutex::new(replies.into()),
            }
        }

        fn recorded(&self) -> Vec<RecordedPlan> {
            self.plans.lock().map(|g| g.clone()).unwrap_or_default()
        }
    }

    impl Runner for FakeRunner {
        fn run(&self, plan: &CommandPlan) -> Result<RunOutput, RunError> {
            if let Ok(mut guard) = self.plans.lock() {
                guard.push(RecordedPlan {
                    program: plan.program,
                    args: plan.args.clone(),
                    stdin: plan.stdin.as_ref().map(|s| s.bytes().to_vec()),
                });
            }
            self.replies
                .lock()
                .ok()
                .and_then(|mut q| q.pop_front())
                .unwrap_or(Err(RunError::Io))
        }
    }

    fn ok_out(stdout: &str) -> Result<RunOutput, RunError> {
        Ok(RunOutput {
            exit_code: Some(0),
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
            truncated: false,
            elapsed: Duration::from_millis(2),
        })
    }

    #[test]
    fn ssh_target_rejects_flag_injection_and_parses_labels() {
        assert!(SshTarget::new("-oProxyCommand=evil", 22, "root", None).is_err());
        assert!(SshTarget::new("node.example.test", 22, "-bad", None).is_err());
        assert!(SshTarget::new("node.example.test", 0, "deploy", None).is_err());

        let target = SshTarget::from_node_labels(
            "vps-1",
            r#"{"ssh_host":"203.0.113.10","ssh_port":2222,"ssh_user":"deploy"}"#,
        );
        assert!(target.is_ok());
        if let Ok(t) = target {
            assert_eq!(t.host, "203.0.113.10");
            assert_eq!(t.port, 2222);
            assert_eq!(t.user, "deploy");
        }
    }

    #[test]
    fn ssh_executor_runs_dry_run_backup_execute_rollback_with_argv_only()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = TestDir::new("ssh-restart")?;
        let target = SshTarget::new("203.0.113.10", 2222, "deploy", None)
            .map_err(|e| format!("target: {e}"))?;
        let runner = FakeRunner::with_replies(vec![
            ok_out("active\n"), // dry_run: is-active
            ok_out("active\n"), // backup: is-active
            ok_out(""),         // execute: restart
            ok_out("active\n"), // execute: is-active
            ok_out(""),         // rollback: restart
            ok_out("active\n"), // rollback: is-active
        ]);
        let exec = SshExecutor::new(runner, target);
        let action = Action::RestartService {
            manager: ServiceManagerKind::Systemd,
            name: ServiceName::new("nginx")?,
        };

        let dry = exec.dry_run(&action);
        assert_eq!(dry.current_state, "active");
        assert_eq!(dry.steps.len(), 2);

        let snap = exec
            .backup(&action, dir.path())
            .map_err(|e| format!("backup: {e}"))?;
        assert_eq!(snap.sha256.len(), 64);

        let out = exec.execute(&action);
        assert_eq!(out.status, OutcomeStatus::Succeeded);
        assert!(out.verified);

        let rb = exec.rollback(&action, &snap);
        assert_eq!(rb.status, OutcomeStatus::Succeeded);
        assert!(rb.verified);

        let plans = exec.runner.recorded();
        assert_eq!(plans.len(), 6);
        assert_eq!(plans[0].program, "ssh");
        assert!(plans[0].args.contains(&"BatchMode=yes".to_owned()));
        assert!(
            plans[0]
                .args
                .contains(&"StrictHostKeyChecking=accept-new".to_owned())
        );
        assert!(plans[0].args.contains(&"deploy@203.0.113.10".to_owned()));
        assert!(!plans[0].args.iter().any(|a| a == "sh" || a == "-c"));
        Ok(())
    }

    #[test]
    fn ssh_executor_update_env_keeps_secret_out_of_argv() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = TestDir::new("ssh-env")?;
        let target = SshTarget::new("203.0.113.20", 22, "deploy", None)
            .map_err(|e| format!("target: {e}"))?;
        let runner = FakeRunner::with_replies(vec![
            ok_out("OLD_KEY=old\n"), // backup: cat
            ok_out(""),              // execute: nodivara-remote update-env (stdin)
            ok_out(""),              // rollback: tee (stdin)
        ]);
        let exec = SshExecutor::new(runner, target);
        let action = Action::UpdateEnvVar {
            path: ManagedPath::new_env("/etc/nodivara/.env")?,
            key: EnvKey::new("API_TOKEN")?,
            value: EnvValue::new("sk-secret-value-12345")?,
        };

        let snap = exec
            .backup(&action, dir.path())
            .map_err(|e| format!("backup: {e}"))?;
        let out = exec.execute(&action);
        assert_eq!(out.status, OutcomeStatus::Succeeded);
        let rb = exec.rollback(&action, &snap);
        assert_eq!(rb.status, OutcomeStatus::Succeeded);

        let plans = exec.runner.recorded();
        for p in &plans {
            assert!(
                !p.args.iter().any(|a| a.contains("sk-secret-value-12345")),
                "secret leaked into ssh argv"
            );
        }
        assert_eq!(
            plans[1].stdin.as_deref(),
            Some(b"sk-secret-value-12345".as_slice())
        );
        Ok(())
    }
}
