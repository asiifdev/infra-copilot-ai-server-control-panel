//! ProcessRunner, exercised only with harmless local programs. No test here may point it at
//! rclone, rsync, scp, ssh or the network.

#![cfg(unix)]

use std::time::Duration;
use transport::{CommandPlan, EnvValue, ProcessRunner, RunError, Runner, Secret, StdinData};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn runner() -> ProcessRunner {
    ProcessRunner::new(Duration::from_secs(5), 64 * 1024).in_dir(std::env::temp_dir())
}

fn plan(program: &'static str, args: &[&str]) -> CommandPlan {
    CommandPlan {
        program,
        args: args.iter().map(|a| (*a).to_owned()).collect(),
        env: Vec::new(),
        stdin: None,
    }
}

#[test]
fn runs_argv_without_a_shell() -> TestResult {
    // If a shell were involved, `;` and `$(...)` would be interpreted. They must come back verbatim.
    let out = runner().run(&plan("/bin/echo", &["a;b", "$(id)", "`id`", "*"]))?;
    assert!(out.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "a;b $(id) `id` *\n");
    Ok(())
}

#[test]
fn reports_exit_codes_and_missing_programs() -> TestResult {
    let out = runner().run(&plan("/usr/bin/true", &[]))?;
    assert!(out.success());
    let out = runner().run(&plan("/usr/bin/false", &[]))?;
    assert_eq!(out.exit_code, Some(1));
    assert_eq!(
        runner().run(&plan("/nonexistent/nodivara-tool", &[])).err(),
        Some(RunError::NotInstalled)
    );
    Ok(())
}

#[test]
fn secrets_go_through_stdin_and_the_environment_only() -> TestResult {
    let mut p = plan("/bin/cat", &[]);
    p.stdin = Some(StdinData::Secret(Secret::new("token-from-stdin")));
    let out = runner().run(&p)?;
    assert_eq!(out.stdout, b"token-from-stdin");
    // `Debug` of the result shows sizes only.
    assert!(!format!("{out:?}").contains("token-from-stdin"));

    let mut p = plan("/usr/bin/env", &[]);
    p.env.push((
        "RCLONE_CONFIG_X_TYPE".to_owned(),
        EnvValue::Plain("s3".to_owned()),
    ));
    p.env.push((
        "RCLONE_CONFIG_X_SECRET".to_owned(),
        EnvValue::Secret(Secret::new("env-secret")),
    ));
    let out = runner().run(&p)?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(text.contains("RCLONE_CONFIG_X_TYPE=s3"));
    assert!(text.contains("RCLONE_CONFIG_X_SECRET=env-secret"));
    Ok(())
}

#[test]
fn the_parent_environment_is_not_inherited_except_the_allow_list() -> TestResult {
    // CARGO_PKG_NAME is set by cargo for this test process and is not on the allow-list.
    assert!(std::env::var_os("CARGO_PKG_NAME").is_some());
    let out = runner().run(&plan("/usr/bin/env", &[]))?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(!text.contains("CARGO_PKG_NAME"), "{text}");
    assert!(!text.contains("NODIVARA_MASTER_KEY"));
    assert!(text.contains("PATH="));
    Ok(())
}

#[test]
fn output_is_capped_and_a_hung_process_is_killed() -> TestResult {
    let small = ProcessRunner::new(Duration::from_secs(5), 16);
    let out = small.run(&plan(
        "/bin/echo",
        &["0123456789012345678901234567890123456789"],
    ))?;
    assert_eq!(out.stdout.len(), 16);
    assert!(out.truncated);

    let quick = ProcessRunner::new(Duration::from_millis(200), 1024);
    let started = std::time::Instant::now();
    assert_eq!(
        quick.run(&plan("/bin/sleep", &["30"])).err(),
        Some(RunError::Timeout)
    );
    assert!(started.elapsed() < Duration::from_secs(10));
    Ok(())
}

/// A grandchild that inherited the pipes can keep them open after its parent is killed or has
/// exited. The run must still end by its own deadline, not by the grandchild's.
#[test]
fn a_grandchild_holding_the_pipes_cannot_defeat_the_timeout() -> TestResult {
    let quick = ProcessRunner::new(Duration::from_millis(200), 1024).in_dir(std::env::temp_dir());
    let started = std::time::Instant::now();
    let result = quick.run(&plan("/bin/sh", &["-c", "sleep 3 & sleep 3"]));
    assert_eq!(result.err(), Some(RunError::Timeout));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "returned after {:?}",
        started.elapsed()
    );
    Ok(())
}

#[test]
fn output_is_returned_even_when_a_background_process_keeps_the_pipe_open() -> TestResult {
    let started = std::time::Instant::now();
    let out = runner().run(&plan("/bin/sh", &["-c", "sleep 3 & echo done"]))?;
    assert!(out.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "done\n");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "returned after {:?}",
        started.elapsed()
    );
    Ok(())
}
