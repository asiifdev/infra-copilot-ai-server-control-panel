//! The action list, its canonical form and the executor, run against a fake runner: no real
//! service is ever touched.

use actions::{Action, ActionError, Executor, LocalExecutor, OutcomeStatus, ServiceManagerKind};
use core_domain::tier::Tier;
use transport::{FakeRunner, RunError, RunOutput};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn restart(name: &str) -> Result<Action, ActionError> {
    Action::restart_service(ServiceManagerKind::Systemd, name)
}

#[test]
fn an_action_has_one_canonical_form_a_fixed_tier_and_a_stable_hash() -> TestResult {
    let action = restart("nginx.service")?;
    assert_eq!(
        action.canonical_json(),
        r#"{"action":"restart_service","manager":"systemd","service":"nginx.service"}"#
    );
    assert_eq!(action.tier(), Tier::LightChange);
    assert!(action.executable_now());
    assert_eq!(action.tool(), "propose_restart_service");
    // Whitespace and key order in storage do not matter; the parsed action is the same.
    let reordered =
        r#" {"service":"nginx.service", "manager":"systemd", "action":"restart_service"} "#;
    let parsed = Action::parse(reordered)?;
    assert_eq!(parsed, action);
    assert_eq!(parsed.hash(), action.hash());
    assert_eq!(action.hash().len(), 64);
    assert_ne!(action.hash(), restart("nginx-b.service")?.hash());
    Ok(())
}

#[test]
fn service_names_that_could_become_options_or_shell_are_refused() {
    for bad in [
        "",
        "-rf",
        ".hidden",
        "a b",
        "a;b",
        "a$(x)",
        "a`b`",
        "a/b",
        "a\nb",
        "a\0b",
        "ünï",
        &"x".repeat(101),
    ] {
        assert_eq!(restart(bad), Err(ActionError::BadServiceName), "{bad:?}");
    }
    for good in ["nginx", "php8.3-fpm.service", "user@1000.service", "a_b-c"] {
        assert!(restart(good).is_ok(), "{good}");
    }
}

#[test]
fn the_stored_form_is_strict() {
    for (json, expected) in [
        ("not json", ActionError::BadShape),
        ("[]", ActionError::BadShape),
        (r#"{"action":"restart_service"}"#, ActionError::BadShape),
        (
            r#"{"action":"restart_service","manager":"systemd","service":"a","extra":1}"#,
            ActionError::BadShape,
        ),
        (
            r#"{"action":"restart_service","manager":"systemd","service":1}"#,
            ActionError::BadShape,
        ),
        (r#"{"action":"drop_database"}"#, ActionError::UnknownAction),
        (
            r#"{"action":"restart_service","manager":"cron","service":"a"}"#,
            ActionError::UnknownManager,
        ),
        (
            r#"{"action":"restart_service","manager":"systemd","service":"-x"}"#,
            ActionError::BadServiceName,
        ),
    ] {
        assert_eq!(Action::parse(json), Err(expected), "{json}");
    }
}

#[test]
fn a_restart_runs_exactly_two_argv_commands_and_verifies() -> TestResult {
    let runner = FakeRunner::new();
    runner.push(Ok(RunOutput::ok("")));
    runner.push(Ok(RunOutput::ok("active\n")));
    let executor = LocalExecutor::new(runner);
    let outcome = executor.execute(&restart("nginx.service")?);
    assert_eq!(outcome.status, OutcomeStatus::Succeeded);
    assert!(outcome.verified);
    assert_eq!(outcome.reason, "ok");
    Ok(())
}

#[test]
fn the_commands_are_argv_only_with_the_name_as_one_argument() -> TestResult {
    let runner = FakeRunner::new();
    let executor = LocalExecutor::new(runner);
    let _ = executor.execute(&restart("php8.3-fpm.service")?);
    // The runner moved into the executor: look at what a second fake records instead.
    let probe = FakeRunner::new();
    let probing = LocalExecutor::new(probe);
    let dry = probing.dry_run(&restart("php8.3-fpm.service")?);
    assert_eq!(dry.steps.len(), 2);
    assert!(dry.steps[0].starts_with("systemctl \"restart\" \"php8.3-fpm.service\""));
    assert_eq!(dry.current_state, "unknown");
    Ok(())
}

#[test]
fn failures_are_reported_with_a_code_and_never_verified() -> TestResult {
    let action = restart("nginx.service")?;
    // The restart command fails: stderr is kept, redacted.
    let runner = FakeRunner::new();
    let leaked = format!("{}{}", "Bearer ", "abcdefghijklmnop1234567890");
    runner.push(Ok(RunOutput::failed(
        1,
        &format!("Failed to restart nginx.service\u{7}: {leaked}"),
    )));
    let out = LocalExecutor::new(runner).execute(&action);
    assert_eq!(
        (out.status, out.verified, out.reason),
        (OutcomeStatus::Failed, false, "command_failed")
    );
    assert!(out.detail.contains("Failed to restart"));
    assert!(
        !out.detail.contains("abcdefghijklmnop1234567890"),
        "{}",
        out.detail
    );
    assert!(!out.detail.contains('\u{7}'));

    // The restart works but the service is not active afterwards.
    let runner = FakeRunner::new();
    runner.push(Ok(RunOutput::ok("")));
    runner.push(Ok(RunOutput::failed(3, "")));
    let out = LocalExecutor::new(runner).execute(&action);
    assert_eq!(
        (out.status, out.reason),
        (OutcomeStatus::Failed, "not_active_after")
    );

    // systemctl is missing, or times out.
    for (error, code) in [
        (RunError::NotInstalled, "not_installed"),
        (RunError::Timeout, "timeout"),
    ] {
        let runner = FakeRunner::new();
        runner.push(Err(error));
        let out = LocalExecutor::new(runner).execute(&action);
        assert_eq!((out.status, out.reason), (OutcomeStatus::Failed, code));
    }
    Ok(())
}

#[test]
fn output_is_bounded() -> TestResult {
    let runner = FakeRunner::new();
    runner.push(Ok(RunOutput::failed(1, &"x".repeat(50_000))));
    let out = LocalExecutor::new(runner).execute(&restart("a")?);
    assert!(out.detail.chars().count() <= 1000);
    Ok(())
}
