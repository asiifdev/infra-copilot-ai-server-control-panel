//! Standalone pull-only remote agent for Nodivara.
//!
//! Security invariants:
//! * Outbound-only: never opens a listening socket.
//! * Never runs arbitrary shell commands: every job is parsed into [`actions::Action`], verified
//!   against `args_hash`, and executed via [`actions::LocalExecutor`] + [`transport::ProcessRunner`].
//! * Never logs or exposes the bearer token: held in [`core_domain::secret::Secret`] and injected
//!   only into the `Authorization` header.
//! * Redacts and caps all command output before posting results back to the control plane.

use actions::{Action, Executor, LocalExecutor, OutcomeStatus};
use core_domain::redact::{REDACTED, redact};
use core_domain::secret::Secret;
use net::{Client, NetError, Policy};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;
use thiserror::Error;
use transport::ProcessRunner;

/// Default environment variable name for the agent bearer token.
pub const DEFAULT_TOKEN_ENV: &str = "NODIVARA_AGENT_TOKEN";

/// Default poll interval in seconds.
pub const DEFAULT_POLL_INTERVAL_SECS: u64 = 5;

/// Maximum bytes of command output retained in a job result.
pub const MAX_OUTPUT_BYTES: usize = 16 * 1024;

/// Maximum command execution timeout for a single action.
const ACTION_TIMEOUT: Duration = Duration::from_secs(30);

/// Errors returned by configuration parsing, transport, or agent cycle execution.
#[derive(Debug, Error)]
pub enum AgentError {
    /// Invalid CLI flag or configuration value.
    #[error("invalid configuration: {0}")]
    Config(&'static str),

    /// Missing or unreadable bearer token.
    #[error("missing or unreadable agent token: {0}")]
    MissingToken(&'static str),

    /// Outbound HTTP/TLS/network error (never contains credentials).
    #[error("outbound request failed: {0}")]
    Http(#[from] NetError),

    /// Control plane rejected the agent token.
    #[error("control plane rejected agent credentials (HTTP {0})")]
    Unauthenticated(u16),

    /// Control plane returned a non-2xx HTTP status.
    #[error("unexpected control plane HTTP status: {0}")]
    UnexpectedStatus(u16),

    /// Malformed JSON payload from control plane.
    #[error("invalid control plane JSON response")]
    BadResponseJson,
}

/// Runtime configuration for the pull-only agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentConfig {
    /// Base URL of the Nodivara control plane (no trailing slash).
    pub server_url: String,
    /// Environment variable containing the bearer token (used when `token_file` is `None`).
    pub token_env: String,
    /// Optional path to a file containing the bearer token.
    pub token_file: Option<PathBuf>,
    /// Delay between poll requests when no job is available.
    pub poll_interval: Duration,
    /// Run a single poll cycle and exit.
    pub once: bool,
    /// Allow `http://` control plane URLs (off by default; HTTPS required).
    pub allow_http: bool,
    /// Allow connecting to private/loopback IP addresses (off by default).
    pub allow_private_addresses: bool,
}

impl AgentConfig {
    /// Parses CLI arguments (excluding `argv[0]`).
    ///
    /// # Errors
    /// Returns [`AgentError::Config`] on unknown flags, missing values, or invalid URLs/intervals.
    pub fn from_args<I, S>(args: I) -> Result<Self, AgentError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut server: Option<String> = None;
        let mut token_env = DEFAULT_TOKEN_ENV.to_owned();
        let mut token_file: Option<PathBuf> = None;
        let mut poll_interval_secs = DEFAULT_POLL_INTERVAL_SECS;
        let mut once = false;
        let mut allow_http = false;
        let mut allow_private_addresses = false;

        let mut iter = args.into_iter();
        while let Some(arg) = iter.next() {
            match arg.as_ref() {
                "--server" => {
                    let val = iter
                        .next()
                        .ok_or(AgentError::Config("--server requires a value"))?;
                    server = Some(val.as_ref().to_owned());
                }
                "--token-env" => {
                    let val = iter
                        .next()
                        .ok_or(AgentError::Config("--token-env requires a value"))?;
                    let s = val.as_ref().trim();
                    if s.is_empty() {
                        return Err(AgentError::Config("--token-env cannot be empty"));
                    }
                    token_env = s.to_owned();
                }
                "--token-file" => {
                    let val = iter
                        .next()
                        .ok_or(AgentError::Config("--token-file requires a value"))?;
                    let s = val.as_ref().trim();
                    if s.is_empty() {
                        return Err(AgentError::Config("--token-file cannot be empty"));
                    }
                    token_file = Some(PathBuf::from(s));
                }
                "--poll-interval-secs" => {
                    let val = iter
                        .next()
                        .ok_or(AgentError::Config("--poll-interval-secs requires a value"))?;
                    let secs: u64 = val.as_ref().parse().map_err(|_| {
                        AgentError::Config("--poll-interval-secs must be an integer")
                    })?;
                    if secs == 0 {
                        return Err(AgentError::Config("--poll-interval-secs must be >= 1"));
                    }
                    poll_interval_secs = secs;
                }
                "--once" => once = true,
                "--allow-http" => allow_http = true,
                "--allow-private" => allow_private_addresses = true,
                _ => return Err(AgentError::Config("unknown CLI flag")),
            }
        }

        let raw_server = server.ok_or(AgentError::Config("--server <url> is required"))?;
        let normalized = normalize_server_url(&raw_server, allow_http)?;

        Ok(Self {
            server_url: normalized,
            token_env,
            token_file,
            poll_interval: Duration::from_secs(poll_interval_secs),
            once,
            allow_http,
            allow_private_addresses,
        })
    }

    /// Loads the bearer token from `--token-file` (if set) or `--token-env`.
    ///
    /// # Errors
    /// Returns [`AgentError::MissingToken`] if the file/env var is missing, unreadable, or empty.
    pub fn resolve_token(&self) -> Result<Secret, AgentError> {
        let raw = if let Some(path) = &self.token_file {
            std::fs::read_to_string(path)
                .map_err(|_| AgentError::MissingToken("failed to read token file"))?
        } else {
            std::env::var(&self.token_env)
                .map_err(|_| AgentError::MissingToken("token environment variable not set"))?
        };
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(AgentError::MissingToken("token is empty"));
        }
        Ok(Secret::new(trimmed.to_owned()))
    }

    /// Builds the [`net::Policy`] matching this configuration.
    #[must_use]
    pub fn net_policy(&self) -> Policy {
        Policy {
            allow_http: self.allow_http,
            block_private_addresses: !self.allow_private_addresses,
            ..Policy::default()
        }
    }
}

fn normalize_server_url(raw: &str, allow_http: bool) -> Result<String, AgentError> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(AgentError::Config("--server cannot be empty"));
    }
    if trimmed.bytes().any(|b| b.is_ascii_control() || b == b' ') {
        return Err(AgentError::Config("--server contains invalid characters"));
    }
    if trimmed.starts_with("https://") {
        if trimmed.len() <= "https://".len() {
            return Err(AgentError::Config("--server missing host"));
        }
        return Ok(trimmed.to_owned());
    }
    if trimmed.starts_with("http://") {
        if !allow_http {
            return Err(AgentError::Config(
                "--server must use https:// unless --allow-http is set",
            ));
        }
        if trimmed.len() <= "http://".len() {
            return Err(AgentError::Config("--server missing host"));
        }
        return Ok(trimmed.to_owned());
    }
    Err(AgentError::Config(
        "--server must start with https:// or http://",
    ))
}

/// Response body from `POST /api/agents/poll`.
#[derive(Debug, Deserialize)]
pub struct PollResponse {
    /// Claimed job, or `None` when the queue is empty.
    pub job: Option<ClaimedJob>,
}

/// Job record returned by `POST /api/agents/poll`.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ClaimedJob {
    /// Unique job identifier.
    pub id: String,
    /// Associated proposal identifier.
    #[serde(default)]
    pub proposal_id: String,
    /// Associated execution identifier.
    #[serde(default)]
    pub execution_id: String,
    /// Canonical JSON representation of the [`Action`].
    pub action_json: String,
    /// Expected SHA-256 hex digest of `Action::canonical_json()`.
    pub args_hash: String,
    /// Job status (`running` when claimed).
    #[serde(default)]
    pub status: String,
    /// Timestamp when claimed.
    #[serde(default)]
    pub claimed_at: Option<String>,
}

/// Request body sent to `POST /api/agents/result`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct JobResultPayload {
    /// Claimed job identifier.
    pub job_id: String,
    /// Whether the action succeeded.
    pub succeeded: bool,
    /// Whether post-execution verification passed.
    pub verified: bool,
    /// Fixed reason code.
    pub reason: String,
    /// Redacted, bounded command output.
    pub output: String,
}

/// Minimal HTTP response returned by [`AgentTransport`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpReply {
    /// HTTP status code.
    pub status: u16,
    /// Response body bytes.
    pub body: Vec<u8>,
}

/// Outbound-only HTTP transport seam (production uses [`NetTransport`]; tests use a mock).
pub trait AgentTransport: Send + Sync {
    /// Sends an outbound `POST` with JSON body and `Authorization: Bearer <token>`.
    fn post_json<'a>(
        &'a self,
        url: &'a str,
        body: &'a [u8],
        bearer_token: &'a Secret,
    ) -> Pin<Box<dyn Future<Output = Result<HttpReply, AgentError>> + Send + 'a>>;
}

/// Production [`AgentTransport`] backed by the hardened [`net::Client`].
#[derive(Debug)]
pub struct NetTransport {
    client: Client,
    policy: Policy,
}

impl NetTransport {
    /// Creates a new [`NetTransport`] with the given [`net::Policy`].
    ///
    /// # Errors
    /// Returns [`AgentError::Http`] if system TLS trust roots cannot be loaded.
    pub fn new(policy: Policy) -> Result<Self, AgentError> {
        Ok(Self {
            client: Client::new()?,
            policy,
        })
    }
}

impl AgentTransport for NetTransport {
    fn post_json<'a>(
        &'a self,
        url: &'a str,
        body: &'a [u8],
        bearer_token: &'a Secret,
    ) -> Pin<Box<dyn Future<Output = Result<HttpReply, AgentError>> + Send + 'a>> {
        Box::pin(async move {
            let auth = format!("Bearer {}", bearer_token.expose_secret());
            let resp = self
                .client
                .post_json_with(url, body, &[("authorization", &auth)], &self.policy)
                .await?;
            Ok(HttpReply {
                status: resp.status,
                body: resp.body,
            })
        })
    }
}

/// Outcome of a single poll-execute-report cycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CycleOutcome {
    /// No queued job was available.
    Idle,
    /// A job was claimed and its result was reported back to the control plane.
    Reported {
        /// Job ID.
        job_id: String,
        /// Whether execution succeeded.
        succeeded: bool,
        /// Whether verification passed.
        verified: bool,
        /// Fixed reason code.
        reason: String,
    },
}

/// Redacts secrets (including the agent's own bearer token) and caps output to [`MAX_OUTPUT_BYTES`].
#[must_use]
pub fn sanitize_output(raw: &str, token: &Secret) -> String {
    let mut cleaned = redact(raw).text;
    let raw_token = token.expose_secret();
    if !raw_token.is_empty() && cleaned.contains(raw_token) {
        cleaned = cleaned.replace(raw_token, REDACTED);
    }
    if cleaned.len() <= MAX_OUTPUT_BYTES {
        return cleaned;
    }
    let mut end = MAX_OUTPUT_BYTES;
    while end > 0 && !cleaned.is_char_boundary(end) {
        end -= 1;
    }
    cleaned.truncate(end);
    cleaned
}

fn check_http_status(status: u16) -> Result<(), AgentError> {
    if status == 401 || status == 403 {
        return Err(AgentError::Unauthenticated(status));
    }
    if !(200..300).contains(&status) {
        return Err(AgentError::UnexpectedStatus(status));
    }
    Ok(())
}

/// Runs one outbound poll -> verify `args_hash` -> execute -> redact -> post result cycle.
///
/// # Errors
/// Returns [`AgentError`] if polling, response decoding, or result submission fails.
pub async fn run_once<T: AgentTransport>(
    config: &AgentConfig,
    token: &Secret,
    transport: &T,
    executor: &dyn Executor,
) -> Result<CycleOutcome, AgentError> {
    let poll_url = format!("{}/api/agents/poll", config.server_url);
    let poll_reply = transport.post_json(&poll_url, b"{}", token).await?;
    check_http_status(poll_reply.status)?;

    let poll: PollResponse =
        serde_json::from_slice(&poll_reply.body).map_err(|_| AgentError::BadResponseJson)?;
    let Some(job) = poll.job else {
        return Ok(CycleOutcome::Idle);
    };

    let payload = execute_claimed_job(&job, token, executor);
    let result_url = format!("{}/api/agents/result", config.server_url);
    let result_bytes = serde_json::to_vec(&payload).map_err(|_| AgentError::BadResponseJson)?;
    let result_reply = transport
        .post_json(&result_url, &result_bytes, token)
        .await?;
    check_http_status(result_reply.status)?;

    Ok(CycleOutcome::Reported {
        job_id: payload.job_id,
        succeeded: payload.succeeded,
        verified: payload.verified,
        reason: payload.reason,
    })
}

fn execute_claimed_job(
    job: &ClaimedJob,
    token: &Secret,
    executor: &dyn Executor,
) -> JobResultPayload {
    let action = match Action::parse(&job.action_json) {
        Ok(a) => a,
        Err(_) => {
            return JobResultPayload {
                job_id: job.id.clone(),
                succeeded: false,
                verified: false,
                reason: "invalid_action".to_owned(),
                output: String::new(),
            };
        }
    };

    if action.hash() != job.args_hash {
        return JobResultPayload {
            job_id: job.id.clone(),
            succeeded: false,
            verified: false,
            reason: "args_hash_mismatch".to_owned(),
            output: String::new(),
        };
    }

    let outcome = executor.execute(&action);
    JobResultPayload {
        job_id: job.id.clone(),
        succeeded: outcome.status == OutcomeStatus::Succeeded,
        verified: outcome.verified,
        reason: outcome.reason.to_owned(),
        output: sanitize_output(&outcome.detail, token),
    }
}

/// Runs the pull-only agent loop until `shutdown` completes (or once if `config.once` is set).
///
/// # Errors
/// Returns [`AgentError`] if `NetTransport` initialization fails or if `config.once` is set and
/// the single cycle fails.
pub async fn run_agent<F>(config: AgentConfig, token: Secret, shutdown: F) -> Result<(), AgentError>
where
    F: Future<Output = ()>,
{
    let net_transport = NetTransport::new(config.net_policy())?;
    let executor = LocalExecutor::new(ProcessRunner::new(ACTION_TIMEOUT, MAX_OUTPUT_BYTES));
    run_agent_with(config, token, &net_transport, &executor, shutdown).await
}

/// Runs the pull-only agent loop with caller-supplied transport and executor seams.
///
/// # Errors
/// Returns [`AgentError`] when `config.once` is `true` and the single cycle fails.
pub async fn run_agent_with<T: AgentTransport, F: Future<Output = ()>>(
    config: AgentConfig,
    token: Secret,
    transport: &T,
    executor: &dyn Executor,
    shutdown: F,
) -> Result<(), AgentError> {
    tokio::pin!(shutdown);

    if config.once {
        let outcome = run_once(&config, &token, transport, executor).await?;
        log_cycle(&outcome);
        return Ok(());
    }

    loop {
        match run_once(&config, &token, transport, executor).await {
            Ok(outcome) => log_cycle(&outcome),
            Err(err) => {
                tracing::warn!(error = %err, "agent poll cycle failed");
            }
        }

        tokio::select! {
            () = &mut shutdown => {
                tracing::info!("shutdown signal received, stopping agent");
                break;
            }
            () = tokio::time::sleep(config.poll_interval) => {}
        }
    }

    Ok(())
}

fn log_cycle(outcome: &CycleOutcome) {
    match outcome {
        CycleOutcome::Idle => tracing::debug!("poll idle"),
        CycleOutcome::Reported {
            job_id,
            succeeded,
            verified,
            reason,
        } => {
            tracing::info!(
                job_id = %job_id,
                succeeded = *succeeded,
                verified = *verified,
                reason = %reason,
                "reported agent job result"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AgentConfig, AgentError, AgentTransport, CycleOutcome, DEFAULT_POLL_INTERVAL_SECS,
        DEFAULT_TOKEN_ENV, HttpReply, JobResultPayload, MAX_OUTPUT_BYTES, run_agent_with, run_once,
        sanitize_output,
    };
    use actions::{Action, LocalExecutor, ServiceManagerKind};
    use core_domain::secret::Secret;
    use std::collections::VecDeque;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Mutex;
    use std::time::Duration;
    use transport::{FakeRunner, RunOutput};

    struct RecordedRequest {
        url: String,
        body: Vec<u8>,
        auth_header: String,
    }

    struct MockTransport {
        replies: Mutex<VecDeque<Result<HttpReply, AgentError>>>,
        requests: Mutex<Vec<RecordedRequest>>,
    }

    impl MockTransport {
        fn new(replies: Vec<Result<HttpReply, AgentError>>) -> Self {
            Self {
                replies: Mutex::new(VecDeque::from(replies)),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn recorded(&self) -> Vec<RecordedRequest> {
            let guard = self.requests.lock().unwrap_or_else(|e| e.into_inner());
            guard
                .iter()
                .map(|r| RecordedRequest {
                    url: r.url.clone(),
                    body: r.body.clone(),
                    auth_header: r.auth_header.clone(),
                })
                .collect()
        }
    }

    impl AgentTransport for MockTransport {
        fn post_json<'a>(
            &'a self,
            url: &'a str,
            body: &'a [u8],
            bearer_token: &'a Secret,
        ) -> Pin<Box<dyn Future<Output = Result<HttpReply, AgentError>> + Send + 'a>> {
            Box::pin(async move {
                {
                    let mut reqs = self.requests.lock().unwrap_or_else(|e| e.into_inner());
                    reqs.push(RecordedRequest {
                        url: url.to_owned(),
                        body: body.to_vec(),
                        auth_header: format!("Bearer {}", bearer_token.expose_secret()),
                    });
                }
                let mut replies = self.replies.lock().unwrap_or_else(|e| e.into_inner());
                replies
                    .pop_front()
                    .unwrap_or(Err(AgentError::UnexpectedStatus(500)))
            })
        }
    }

    #[test]
    fn config_parses_defaults_and_validates_flags() -> Result<(), Box<dyn std::error::Error>> {
        let cfg = AgentConfig::from_args(["--server", "https://panel.example.com/"])?;
        assert_eq!(cfg.server_url, "https://panel.example.com");
        assert_eq!(cfg.token_env, DEFAULT_TOKEN_ENV);
        assert_eq!(cfg.token_file, None);
        assert_eq!(
            cfg.poll_interval,
            Duration::from_secs(DEFAULT_POLL_INTERVAL_SECS)
        );
        assert!(!cfg.once);
        assert!(!cfg.allow_http);
        assert!(!cfg.allow_private_addresses);

        // Rejects http:// without --allow-http
        assert!(matches!(
            AgentConfig::from_args(["--server", "http://127.0.0.1:3000"]),
            Err(AgentError::Config(_))
        ));

        // Accepts http:// with --allow-http and custom flags
        let custom = AgentConfig::from_args([
            "--server",
            "http://127.0.0.1:3000",
            "--allow-http",
            "--allow-private",
            "--token-env",
            "CUSTOM_TOK",
            "--poll-interval-secs",
            "10",
            "--once",
        ])?;
        assert_eq!(custom.token_env, "CUSTOM_TOK");
        assert_eq!(custom.poll_interval, Duration::from_secs(10));
        assert!(custom.once);
        assert!(custom.allow_http);
        assert!(custom.allow_private_addresses);

        // Rejects zero poll interval and unknown flags
        assert!(matches!(
            AgentConfig::from_args([
                "--server",
                "https://panel.example.com",
                "--poll-interval-secs",
                "0"
            ]),
            Err(AgentError::Config(_))
        ));
        assert!(matches!(
            AgentConfig::from_args(["--server", "https://panel.example.com", "--bogus"]),
            Err(AgentError::Config(_))
        ));
        Ok(())
    }

    #[test]
    fn token_is_never_leaked_in_debug_display_or_sanitized_output() {
        let raw_token = "super-secret-agent-token-9999";
        let secret = Secret::new(raw_token);

        assert!(!format!("{secret:?}").contains(raw_token));
        assert!(!format!("{secret}").contains(raw_token));

        let fake_api_key = ["sk", "1234567890abcdef1234567890"].join("-");
        let raw_output =
            format!("Authorization: Bearer {raw_token}\nAPI_KEY={fake_api_key}\nraw={raw_token}");
        let sanitized = sanitize_output(&raw_output, &secret);
        assert!(!sanitized.contains(raw_token));
        assert!(!sanitized.contains(&fake_api_key));
        assert!(sanitized.contains("[REDACTED]"));

        // Capping respects MAX_OUTPUT_BYTES
        let huge = "a".repeat(MAX_OUTPUT_BYTES + 500);
        let capped = sanitize_output(&huge, &secret);
        assert_eq!(capped.len(), MAX_OUTPUT_BYTES);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn rejects_args_hash_mismatch_and_reports_failure()
    -> Result<(), Box<dyn std::error::Error>> {
        let cfg = AgentConfig::from_args(["--server", "https://panel.example.com", "--once"])?;
        let token = Secret::new("tok_test_123");
        let action = Action::restart_service(ServiceManagerKind::Systemd, "nginx")?;

        let poll_body = serde_json::json!({
            "job": {
                "id": "job_1",
                "action_json": action.canonical_json(),
                "args_hash": "0000000000000000000000000000000000000000000000000000000000000000",
                "status": "running"
            }
        })
        .to_string()
        .into_bytes();

        let transport = MockTransport::new(vec![
            Ok(HttpReply {
                status: 200,
                body: poll_body,
            }),
            Ok(HttpReply {
                status: 200,
                body: b"{}".to_vec(),
            }),
        ]);
        let executor = LocalExecutor::new(FakeRunner::default());

        let outcome = run_once(&cfg, &token, &transport, &executor).await?;
        assert_eq!(
            outcome,
            CycleOutcome::Reported {
                job_id: "job_1".to_owned(),
                succeeded: false,
                verified: false,
                reason: "args_hash_mismatch".to_owned(),
            }
        );
        // FakeRunner was never invoked because hash verification failed first.
        let reqs = transport.recorded();
        assert_eq!(reqs.len(), 2);
        assert_eq!(reqs[0].url, "https://panel.example.com/api/agents/poll");
        assert_eq!(reqs[1].url, "https://panel.example.com/api/agents/result");
        assert_eq!(reqs[0].auth_header, "Bearer tok_test_123");

        let sent: serde_json::Value = serde_json::from_slice(&reqs[1].body)?;
        assert_eq!(sent["job_id"], "job_1");
        assert_eq!(sent["succeeded"], false);
        assert_eq!(sent["verified"], false);
        assert_eq!(sent["reason"], "args_hash_mismatch");
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn poll_execute_and_report_cycle_succeeds_and_redacts_output()
    -> Result<(), Box<dyn std::error::Error>> {
        let cfg = AgentConfig::from_args(["--server", "https://panel.example.com", "--once"])?;
        let token = Secret::new("agent_secret_xyz");
        let action = Action::restart_service(ServiceManagerKind::Systemd, "nginx")?;

        let poll_body = serde_json::json!({
            "job": {
                "id": "job_42",
                "action_json": action.canonical_json(),
                "args_hash": action.hash(),
                "status": "running"
            }
        })
        .to_string()
        .into_bytes();

        let transport = MockTransport::new(vec![
            Ok(HttpReply {
                status: 200,
                body: poll_body,
            }),
            Ok(HttpReply {
                status: 200,
                body: b"{}".to_vec(),
            }),
        ]);

        // LocalExecutor::execute(RestartService) runs `systemctl restart nginx` then `systemctl is-active nginx`.
        let runner = FakeRunner::new();
        runner.push(Ok(RunOutput::ok("restarted with TOKEN=agent_secret_xyz")));
        runner.push(Ok(RunOutput::ok("active\n")));
        let executor = LocalExecutor::new(runner);

        run_agent_with(cfg, token, &transport, &executor, std::future::pending()).await?;

        let reqs = transport.recorded();
        assert_eq!(reqs.len(), 2);
        let sent: serde_json::Value = serde_json::from_slice(&reqs[1].body)?;
        assert_eq!(sent["job_id"], "job_42");
        assert_eq!(sent["succeeded"], true);
        assert_eq!(sent["verified"], true);
        assert_eq!(sent["reason"], "ok");
        assert!(
            !sent["output"]
                .as_str()
                .unwrap_or("")
                .contains("agent_secret_xyz")
        );
        let _ = std::mem::size_of::<JobResultPayload>();
        Ok(())
    }
}
