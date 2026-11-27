//! Read-only bounded service log reader: redacts secrets, strips control characters, and caps
//! output at 200 lines / 16 KiB.

use crate::ServiceName;
use crate::exec::validate_managed_path;
use core_domain::redact::redact;
use std::fs;
use std::path::Path;
use transport::{CommandPlan, Runner};

/// Maximum number of log lines returned per request.
pub const MAX_LOG_LINES: usize = 200;
/// Maximum byte length of returned log text (16 KiB).
pub const MAX_LOG_BYTES: usize = 16 * 1024;

/// Result of reading service logs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogExcerpt {
    /// Validated service name.
    pub service: String,
    /// Number of lines returned.
    pub line_count: usize,
    /// Whether the output was truncated to fit line or byte limits.
    pub truncated: bool,
    /// Redacted, control-character-cleaned log text.
    pub content: String,
}

/// Cleans, redacts, and bounds raw log bytes to at most `max_lines` (capped at [`MAX_LOG_LINES`])
/// and [`MAX_LOG_BYTES`].
#[must_use]
pub fn format_log_excerpt(service: &str, raw: &[u8], max_lines: usize) -> LogExcerpt {
    let limit_lines = max_lines.clamp(1, MAX_LOG_LINES);
    let text = String::from_utf8_lossy(raw);
    let all_lines: Vec<&str> = text.lines().collect();
    let total_lines = all_lines.len();
    let start = total_lines.saturating_sub(limit_lines);
    let tail = all_lines[start..].join("\n");

    let redacted = redact(&tail).text;
    let cleaned: String = redacted
        .chars()
        .map(|c| if c.is_control() && c != '\n' { ' ' } else { c })
        .collect();

    let mut truncated = start > 0;
    let content = if cleaned.len() > MAX_LOG_BYTES {
        truncated = true;
        let mut end = MAX_LOG_BYTES;
        while end > 0 && !cleaned.is_char_boundary(end) {
            end -= 1;
        }
        cleaned[..end].to_owned()
    } else {
        cleaned
    };

    let line_count = if content.is_empty() {
        0
    } else {
        content.lines().count()
    };

    LogExcerpt {
        service: service.to_owned(),
        line_count,
        truncated,
        content,
    }
}

/// Reads `<managed_dir>/<service>.log` safely (no path traversal, no symlinks).
///
/// # Errors
/// Returns a fixed error code (`"path_not_allowed"`, `"symlink_not_allowed"`, `"not_found"`, `"io"`).
pub fn read_managed_log(
    managed_dir: &Path,
    service: &ServiceName,
    max_lines: usize,
) -> Result<LogExcerpt, &'static str> {
    let candidate = managed_dir.join(format!("{}.log", service.as_str()));
    let candidate_str = candidate.to_str().ok_or("path_not_allowed")?;
    let validated = validate_managed_path(candidate_str, &[managed_dir.to_path_buf()])?;
    let bytes = match fs::read(&validated) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err("not_found"),
        Err(_) => return Err("io"),
    };
    Ok(format_log_excerpt(service.as_str(), &bytes, max_lines))
}

/// Reads service logs via `journalctl -u <service> -n <lines> --no-pager` through `runner`.
///
/// # Errors
/// Returns a fixed error code (`"not_installed"`, `"timeout"`, `"io"`, `"command_failed"`).
pub fn read_service_logs_via_runner(
    runner: &dyn Runner,
    service: &ServiceName,
    max_lines: usize,
) -> Result<LogExcerpt, &'static str> {
    let limit = max_lines.clamp(1, MAX_LOG_LINES);
    let plan = CommandPlan {
        program: "journalctl",
        args: vec![
            "-u".to_owned(),
            service.as_str().to_owned(),
            "-n".to_owned(),
            limit.to_string(),
            "--no-pager".to_owned(),
        ],
        env: Vec::new(),
        stdin: None,
    };
    let out = runner.run(&plan).map_err(|e| match e {
        transport::RunError::NotInstalled => "not_installed",
        transport::RunError::Timeout => "timeout",
        transport::RunError::Io => "io",
    })?;
    if out.exit_code != Some(0) {
        return Err("command_failed");
    }
    Ok(format_log_excerpt(service.as_str(), &out.stdout, limit))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDir;
    use std::fmt::Write as _;
    use std::time::Duration;

    #[test]
    fn managed_log_redacts_secrets_and_caps_lines_and_bytes()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = TestDir::new("logs")?;
        let log_path = dir.path().join("api.service.log");
        let mut body = String::new();
        for i in 1..=250 {
            let _ = writeln!(
                body,
                "line {i} DATABASE_URL=postgres://user:supersecret@10.0.0.2/db"
            );
        }
        fs::write(&log_path, body.as_bytes())?;

        let svc = ServiceName::new("api")?;
        let excerpt = read_managed_log(dir.path(), &svc, 500)
            .map_err(|e| format!("read_managed_log: {e}"))?;
        assert!(excerpt.truncated);
        assert!(excerpt.line_count <= MAX_LOG_LINES);
        assert!(excerpt.content.len() <= MAX_LOG_BYTES);
        assert!(!excerpt.content.contains("supersecret"));
        assert!(excerpt.content.contains("line 250"));
        Ok(())
    }

    #[test]
    fn journalctl_runner_reads_and_redacts() -> Result<(), Box<dyn std::error::Error>> {
        struct Fake;
        impl Runner for Fake {
            fn run(&self, plan: &CommandPlan) -> Result<transport::RunOutput, transport::RunError> {
                assert_eq!(plan.program, "journalctl");
                assert_eq!(
                    plan.args,
                    vec!["-u", "nginx.service", "-n", "10", "--no-pager"]
                );
                Ok(transport::RunOutput {
                    exit_code: Some(0),
                    stdout: b"started\nAPI_KEY=sk-proj-1234567890abcdef1234567890\n".to_vec(),
                    stderr: Vec::new(),
                    truncated: false,
                    elapsed: Duration::from_millis(2),
                })
            }
        }
        let svc = ServiceName::new("nginx")?;
        let excerpt =
            read_service_logs_via_runner(&Fake, &svc, 10).map_err(|e| format!("runner: {e}"))?;
        assert!(!excerpt.content.contains("sk-proj-1234567890"));
        assert!(excerpt.content.contains("started"));
        Ok(())
    }
}
