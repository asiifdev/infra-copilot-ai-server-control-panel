//! The only way a collector runs an external program (decision 0014).
//!
//! Not a shell: a collector picks one [`Cmd`] from a closed list; each maps to a fixed program and
//! fixed arguments (the only variable part, a container id, is validated as hexadecimal). The
//! program is started directly (no shell, no globbing, no user-supplied argument strings), with a
//! cleared environment except `PATH` and `HOME`, no stdin, a time limit and an output cap. All
//! commands in the list are read-only queries. Tests use [`FakeRunner`], so they never touch a real
//! Docker or service manager.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Most output bytes kept from one command.
pub const MAX_OUTPUT_BYTES: usize = 2 * 1024 * 1024;
const TIME_LIMIT: Duration = Duration::from_secs(10);

/// A read-only query a collector may run. Closed list: adding one needs review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cmd {
    /// `docker ps --no-trunc --format {{json .}}`: one JSON object per running container.
    DockerPs,
    /// `docker inspect <id>` for one container (the id is validated).
    DockerInspect(String),
    /// `systemctl list-units --type=service --all --plain --no-legend --no-pager`.
    SystemctlServices,
    /// `systemctl list-timers --all --plain --no-legend --no-pager`.
    SystemctlTimers,
}

impl Cmd {
    /// The fixed program and arguments, or `None` when a variable part is invalid.
    #[must_use]
    pub fn argv(&self) -> Option<(&'static str, Vec<String>)> {
        let own = |args: &[&str]| args.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>();
        match self {
            Self::DockerPs => Some((
                "docker",
                own(&["ps", "--no-trunc", "--format", "{{json .}}"]),
            )),
            Self::DockerInspect(id) => {
                let valid =
                    (12..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_hexdigit());
                valid.then(|| ("docker", vec!["inspect".to_owned(), id.clone()]))
            }
            Self::SystemctlServices => Some((
                "systemctl",
                own(&[
                    "list-units",
                    "--type=service",
                    "--all",
                    "--plain",
                    "--no-legend",
                    "--no-pager",
                ]),
            )),
            Self::SystemctlTimers => Some((
                "systemctl",
                own(&[
                    "list-timers",
                    "--all",
                    "--plain",
                    "--no-legend",
                    "--no-pager",
                ]),
            )),
        }
    }
}

/// Why a command gave no output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecError {
    /// The program is not installed or not allowed to run.
    NotAvailable,
    /// It ran longer than the time limit and was stopped.
    TimedOut,
    /// It exited with an error (for example no permission to use the Docker socket).
    Failed(String),
}

/// Runs a [`Cmd`] and returns its standard output (capped, lossily decoded as UTF-8).
pub trait CommandRunner: Send + Sync {
    /// # Errors
    /// [`ExecError`] when the program is missing, times out or fails. Collectors turn this into a
    /// warning, never into "nothing there".
    fn run(&self, cmd: &Cmd) -> Result<String, ExecError>;
}

/// Runs commands on this machine.
#[derive(Debug, Clone, Copy, Default)]
pub struct HostRunner;

impl CommandRunner for HostRunner {
    fn run(&self, cmd: &Cmd) -> Result<String, ExecError> {
        let (program, args) = cmd
            .argv()
            .ok_or_else(|| ExecError::Failed("invalid argument".into()))?;
        let mut command = Command::new(program);
        command
            .args(&args)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for name in ["PATH", "HOME"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        let mut child = command.spawn().map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied => {
                ExecError::NotAvailable
            }
            _ => ExecError::Failed(err.kind().to_string()),
        })?;
        let mut stdout = child.stdout.take();
        let reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(out) = stdout.as_mut() {
                let _ = out.take(MAX_OUTPUT_BYTES as u64).read_to_end(&mut buf);
            }
            buf
        });
        let start = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if start.elapsed() > TIME_LIMIT => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(ExecError::TimedOut);
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(err) => return Err(ExecError::Failed(err.kind().to_string())),
            }
        };
        let bytes = reader.join().unwrap_or_default();
        if status.success() {
            Ok(String::from_utf8_lossy(&bytes).into_owned())
        } else {
            Err(ExecError::Failed(format!(
                "exit status {:?}",
                status.code()
            )))
        }
    }
}

/// A runner for tests and for fixture roots: returns canned output per command, and
/// [`ExecError::NotAvailable`] for anything else.
#[derive(Debug, Clone, Default)]
pub struct FakeRunner {
    answers: Vec<(Cmd, Result<String, ExecError>)>,
}

impl FakeRunner {
    /// An empty runner (every command is not available).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds the answer for `cmd`.
    #[must_use]
    pub fn with(mut self, cmd: Cmd, answer: Result<String, ExecError>) -> Self {
        self.answers.push((cmd, answer));
        self
    }
}

impl CommandRunner for FakeRunner {
    fn run(&self, cmd: &Cmd) -> Result<String, ExecError> {
        self.answers
            .iter()
            .find(|(known, _)| known == cmd)
            .map_or(Err(ExecError::NotAvailable), |(_, answer)| answer.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspect_accepts_only_hex_ids() {
        for bad in [
            "",
            "abc",
            "--all",
            "zzzzzzzzzzzz",
            "a b c d e f 1 2 3 4 5 6",
            "../etc/passwd",
        ] {
            assert!(Cmd::DockerInspect(bad.into()).argv().is_none(), "{bad}");
        }
        let ok = Cmd::DockerInspect("0123456789ab".into()).argv();
        assert_eq!(ok.map(|(p, a)| (p, a.len())), Some(("docker", 2)));
    }

    #[test]
    fn fake_runner_answers_only_what_it_was_given() {
        let fake = FakeRunner::new().with(Cmd::DockerPs, Ok("x".into()));
        assert_eq!(fake.run(&Cmd::DockerPs), Ok("x".into()));
        assert_eq!(
            fake.run(&Cmd::SystemctlTimers),
            Err(ExecError::NotAvailable)
        );
    }
}
