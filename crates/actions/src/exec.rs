//! The executor: turns an [`Action`] into argv-only commands and runs them through a
//! [`Runner`]. Output from the target is untrusted text: it is redacted, stripped of control
//! characters and cut before anything keeps it.

use crate::{Action, ServiceManagerKind};
use core_domain::redact::redact;
use transport::{CommandPlan, RunOutput, Runner};

/// Most characters of command output kept in an [`Outcome`] or a [`DryRun`].
pub const MAX_DETAIL: usize = 1000;

/// What running an action would do, found without changing anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DryRun {
    /// The commands the action would run, in a log-safe rendering.
    pub steps: Vec<String>,
    /// What a read-only check found (for a restart: the current state of the service).
    pub current_state: String,
}

/// How an execution ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeStatus {
    /// The commands ran and the verification passed.
    Succeeded,
    /// A command failed, could not start, or the verification did not pass.
    Failed,
}

/// The result of running an action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// How it ended.
    pub status: OutcomeStatus,
    /// Whether the check made after the action passed.
    pub verified: bool,
    /// A fixed code for what happened (`ok`, `not_installed`, `timeout`, `command_failed`,
    /// `not_active_after`, `io`).
    pub reason: &'static str,
    /// Redacted, bounded output of the failing or last command.
    pub detail: String,
}

/// Runs actions. Implementations must run only the commands of the closed action list.
pub trait Executor: Send + Sync {
    /// Describes what [`Executor::execute`] would do, with read-only checks only.
    fn dry_run(&self, action: &Action) -> DryRun;

    /// Runs the action and verifies the result.
    fn execute(&self, action: &Action) -> Outcome;
}

/// Runs actions on this machine through a [`Runner`].
#[derive(Debug)]
pub struct LocalExecutor<R> {
    runner: R,
}

impl<R: Runner + Send + Sync> LocalExecutor<R> {
    /// An executor over `runner` (a `ProcessRunner` in production, a `FakeRunner` in tests).
    pub const fn new(runner: R) -> Self {
        Self { runner }
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

/// Untrusted output made safe to keep: redacted, control characters (but newlines) replaced,
/// at most [`MAX_DETAIL`] characters.
fn safe_excerpt(bytes: &[u8]) -> String {
    let redacted = redact(&String::from_utf8_lossy(bytes)).text;
    redacted
        .chars()
        .map(|c| if c.is_control() && c != '\n' { ' ' } else { c })
        .take(MAX_DETAIL)
        .collect::<String>()
        .trim()
        .to_owned()
}

fn first_line(output: &RunOutput) -> String {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .chars()
        .take(40)
        .collect()
}

fn failure(reason: &'static str, detail: String) -> Outcome {
    Outcome {
        status: OutcomeStatus::Failed,
        verified: false,
        reason,
        detail,
    }
}

fn run_error_reason(error: transport::RunError) -> &'static str {
    match error {
        transport::RunError::NotInstalled => "not_installed",
        transport::RunError::Timeout => "timeout",
        transport::RunError::Io => "io",
    }
}

impl<R: Runner + Send + Sync> Executor for LocalExecutor<R> {
    fn dry_run(&self, action: &Action) -> DryRun {
        let Action::RestartService { manager, name } = action;
        match manager {
            ServiceManagerKind::Systemd => {
                let restart = systemctl(&["restart"], name.as_str());
                let check = systemctl(&["is-active"], name.as_str());
                let state = match self.runner.run(&check) {
                    Ok(output) => {
                        let line = first_line(&output);
                        if line.is_empty() {
                            "unknown".to_owned()
                        } else {
                            line
                        }
                    }
                    Err(error) => run_error_reason(error).to_owned(),
                };
                DryRun {
                    steps: vec![restart.to_string(), check.to_string()],
                    current_state: state,
                }
            }
        }
    }

    fn execute(&self, action: &Action) -> Outcome {
        let Action::RestartService { manager, name } = action;
        match manager {
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
        }
    }
}
