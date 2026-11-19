//! The seam where a process would be started. [`Runner`] takes a [`CommandPlan`]; the real
//! [`ProcessRunner`] starts it with argv only (no shell), a cleared environment plus an
//! allow-list, a timeout and an output cap. [`FakeRunner`] records plans and replays scripted
//! results for tests.

use crate::plan::{CommandPlan, EnvValue};
use std::collections::VecDeque;
use std::fmt;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// What a finished process produced. `Debug` shows sizes only: output can carry tokens.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct RunOutput {
    /// Exit code, `None` when killed by a signal.
    pub exit_code: Option<i32>,
    /// Captured standard output (capped).
    pub stdout: Vec<u8>,
    /// Captured standard error (capped).
    pub stderr: Vec<u8>,
    /// Wall-clock time the process ran.
    pub elapsed: Duration,
    /// Output was longer than the cap and was cut.
    pub truncated: bool,
}

impl RunOutput {
    /// A successful result with this stdout.
    #[must_use]
    pub fn ok(stdout: &str) -> Self {
        Self {
            exit_code: Some(0),
            stdout: stdout.as_bytes().to_vec(),
            ..Self::default()
        }
    }

    /// A failed result with this exit code and stderr.
    #[must_use]
    pub fn failed(code: i32, stderr: &str) -> Self {
        Self {
            exit_code: Some(code),
            stderr: stderr.as_bytes().to_vec(),
            ..Self::default()
        }
    }

    /// Whether the exit code was 0.
    #[must_use]
    pub fn success(&self) -> bool {
        self.exit_code == Some(0)
    }
}

impl fmt::Debug for RunOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "RunOutput(exit={:?}, stdout={} bytes, stderr={} bytes, truncated={})",
            self.exit_code,
            self.stdout.len(),
            self.stderr.len(),
            self.truncated
        )
    }
}

/// Why a process could not be run to completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunError {
    /// The program is not installed (or not on `PATH`).
    NotInstalled,
    /// It did not finish in time and was killed.
    Timeout,
    /// Starting or talking to it failed.
    Io,
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotInstalled => "transport_run_not_installed",
            Self::Timeout => "transport_run_timeout",
            Self::Io => "transport_run_io",
        })
    }
}

impl std::error::Error for RunError {}

/// Starts a plan. Synchronous; async callers use a blocking task.
pub trait Runner {
    /// Runs the plan to completion.
    ///
    /// # Errors
    /// [`RunError`] when the process cannot be started, or times out.
    fn run(&self, plan: &CommandPlan) -> Result<RunOutput, RunError>;
}

/// Environment variables passed through to children. Everything else is dropped, so a stray
/// secret in the parent environment (the master key above all) never reaches a child.
pub const ENV_ALLOW_LIST: [&str; 8] = [
    "PATH",
    "HOME",
    "USER",
    "LANG",
    "LC_ALL",
    "TMPDIR",
    "SSH_AUTH_SOCK",
    "TERM",
];

/// Runs plans as real processes.
#[derive(Debug, Clone)]
pub struct ProcessRunner {
    timeout: Duration,
    max_output: usize,
    work_dir: Option<PathBuf>,
}

impl ProcessRunner {
    /// A runner with a timeout and a cap on captured bytes per stream.
    #[must_use]
    pub const fn new(timeout: Duration, max_output: usize) -> Self {
        Self {
            timeout,
            max_output,
            work_dir: None,
        }
    }

    /// Runs children in this directory.
    #[must_use]
    pub fn in_dir(mut self, dir: PathBuf) -> Self {
        self.work_dir = Some(dir);
        self
    }
}

/// Reads a stream fully but keeps at most `cap` bytes; keeps draining so the child never blocks.
fn drain(mut stream: impl Read, cap: usize) -> (Vec<u8>, bool) {
    let mut kept = Vec::new();
    let mut truncated = false;
    let mut buf = [0_u8; 8192];
    loop {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let room = cap.saturating_sub(kept.len());
                if n > room {
                    truncated = true;
                }
                kept.extend_from_slice(&buf[..n.min(room)]);
            }
        }
    }
    (kept, truncated)
}

impl Runner for ProcessRunner {
    fn run(&self, plan: &CommandPlan) -> Result<RunOutput, RunError> {
        let mut command = Command::new(plan.program);
        command.args(&plan.args).env_clear();
        for name in ENV_ALLOW_LIST {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        for (name, value) in &plan.env {
            command.env(name, value.expose());
        }
        if let Some(dir) = &self.work_dir {
            command.current_dir(dir);
        }
        command
            .stdin(if plan.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let started = Instant::now();
        let mut child = command.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                RunError::NotInstalled
            } else {
                RunError::Io
            }
        })?;
        let (Some(out), Some(err)) = (child.stdout.take(), child.stderr.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(RunError::Io);
        };
        let cap = self.max_output;
        let out_thread = std::thread::spawn(move || drain(out, cap));
        let err_thread = std::thread::spawn(move || drain(err, cap));
        let in_thread = match (child.stdin.take(), plan.stdin.as_ref()) {
            (Some(mut pipe), Some(data)) => {
                let bytes = data.bytes().to_vec();
                Some(std::thread::spawn(move || {
                    // A child that exits without reading is not an error here.
                    let _ = pipe.write_all(&bytes);
                }))
            }
            _ => None,
        };
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) if started.elapsed() >= self.timeout => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(RunError::Timeout);
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                Err(_) => break Err(RunError::Io),
            }
        };
        let elapsed = started.elapsed();
        let (stdout, cut_out) = out_thread.join().unwrap_or_default();
        let (stderr, cut_err) = err_thread.join().unwrap_or_default();
        if let Some(t) = in_thread {
            let _ = t.join();
        }
        let status = status?;
        Ok(RunOutput {
            exit_code: status.code(),
            stdout,
            stderr,
            elapsed,
            truncated: cut_out || cut_err,
        })
    }
}

/// What a [`FakeRunner`] saw: the plan with secret values kept apart. `Debug` hides them.
#[derive(Clone, PartialEq, Eq)]
pub struct RecordedCall {
    /// Program.
    pub program: String,
    /// Arguments.
    pub args: Vec<String>,
    /// Non-secret environment.
    pub plain_env: Vec<(String, String)>,
    /// Names of secret variables and their values (tests assert on where secrets go).
    pub secret_env: Vec<(String, String)>,
    /// Stdin bytes, if any.
    pub stdin: Option<Vec<u8>>,
    /// Whether stdin was marked secret.
    pub stdin_secret: bool,
}

impl fmt::Debug for RecordedCall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecordedCall")
            .field("program", &self.program)
            .field("args", &self.args)
            .field("plain_env", &self.plain_env)
            .field(
                "secret_env",
                &self.secret_env.iter().map(|(n, _)| n).collect::<Vec<_>>(),
            )
            .field("stdin_bytes", &self.stdin.as_ref().map(Vec::len))
            .finish()
    }
}

type Reply = Result<RunOutput, RunError>;

/// Records every plan and replays queued replies in order. With nothing queued it answers with
/// an empty success.
#[derive(Debug, Default)]
pub struct FakeRunner {
    calls: Mutex<Vec<RecordedCall>>,
    replies: Mutex<VecDeque<Reply>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl FakeRunner {
    /// A fake with no scripted replies.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues the next reply.
    pub fn push(&self, reply: Reply) {
        lock(&self.replies).push_back(reply);
    }

    /// Everything run so far.
    #[must_use]
    pub fn calls(&self) -> Vec<RecordedCall> {
        lock(&self.calls).clone()
    }
}

impl Runner for FakeRunner {
    fn run(&self, plan: &CommandPlan) -> Result<RunOutput, RunError> {
        let mut plain_env = Vec::new();
        let mut secret_env = Vec::new();
        for (name, value) in &plan.env {
            match value {
                EnvValue::Plain(v) => plain_env.push((name.clone(), v.clone())),
                EnvValue::Secret(s) => {
                    secret_env.push((name.clone(), s.expose_secret().to_owned()))
                }
            }
        }
        lock(&self.calls).push(RecordedCall {
            program: plan.program.to_owned(),
            args: plan.args.clone(),
            plain_env,
            secret_env,
            stdin: plan.stdin.as_ref().map(|s| s.bytes().to_vec()),
            stdin_secret: matches!(plan.stdin, Some(crate::plan::StdinData::Secret(_))),
        });
        lock(&self.replies)
            .pop_front()
            .unwrap_or_else(|| Ok(RunOutput::ok("")))
    }
}
