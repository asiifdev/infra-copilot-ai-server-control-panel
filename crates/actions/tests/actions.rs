//! The action list, its canonical form and the executor, run against a fake runner: no real
//! service is ever touched.

use actions::{
    AcmeProvider, Action, ActionError, DbEngine, Executor, LocalExecutor, OutcomeStatus,
    PhpVersion, ServiceManagerKind,
};
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
    // A bare name and its full unit name are one and the same action.
    assert_eq!(restart("nginx")?.hash(), action.hash());
    Ok(())
}

#[test]
fn only_service_units_can_be_addressed_and_critical_ones_are_tier_three() -> TestResult {
    // Other unit types are refused: restarting a target can power the machine off.
    for bad in [
        "poweroff.target",
        "reboot.target",
        "halt.target",
        "multi-user.target",
        "ctrl-alt-del.target",
        "dbus.socket",
        "boot.mount",
        "x.timer",
        "x.SLICE",
        "x.swap",
    ] {
        assert_eq!(restart(bad), Err(ActionError::BadServiceName), "{bad}");
    }
    // A bare name always becomes a service unit, so the command cannot address anything else.
    let bare = restart("php8.3-fpm")?;
    assert_eq!(
        bare.canonical_json(),
        r#"{"action":"restart_service","manager":"systemd","service":"php8.3-fpm.service"}"#
    );
    assert_eq!(bare, restart("php8.3-fpm.service")?);
    // Units that can cut the machine off or stop it are a risky change, never run today.
    for critical in [
        "sshd",
        "ssh.service",
        "systemd-logind",
        "systemd-networkd.service",
        "dbus",
        "NetworkManager",
        "networking",
        "getty@tty1",
        "user@1000",
        "reboot",
        "poweroff",
        "Rescue",
    ] {
        let action = restart(critical)?;
        assert_eq!(action.tier(), Tier::RiskyChange, "{critical}");
        assert!(action.executable_now(), "{critical}");
        assert!(action.requires_backup(), "{critical}");
        assert!(!action.requires_typed_confirmation(), "{critical}");
    }
    for ordinary in [
        "nginx",
        "php8.3-fpm",
        "redis-server",
        "my-app.service",
        "sshfs-helper-not",
    ] {
        let action = restart(ordinary)?;
        // `sshfs-helper-not` shares the prefix `ssh` and is conservatively critical too.
        let expected = if ordinary.starts_with("ssh") {
            Tier::RiskyChange
        } else {
            Tier::LightChange
        };
        assert_eq!(action.tier(), expected, "{ordinary}");
    }
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
        &"x".repeat(93),
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

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(prefix: &str) -> Result<Self, Box<dyn std::error::Error>> {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let base = std::env::temp_dir().canonicalize()?;
        let dir = base.join(format!("nodivara-{prefix}-{}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        Ok(Self(dir))
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn update_env_var_validation_and_redaction() -> TestResult {
    for bad_path in [
        "",
        "relative/.env",
        "/etc/nodivara/../.env",
        "/etc/nodivara/./.env",
        "/etc/nodivara/app.conf",
        "/etc/nodivara/.env/",
    ] {
        assert_eq!(
            Action::update_env_var(bad_path, "APP_KEY", "secret"),
            Err(ActionError::BadPath),
            "{bad_path}"
        );
    }
    for bad_key in ["", "1ABC", "A-B", "A B", "A=B"] {
        assert_eq!(
            Action::update_env_var("/etc/nodivara/.env", bad_key, "val"),
            Err(ActionError::BadEnvKey),
            "{bad_key}"
        );
    }
    for bad_val in ["a\nb", "a\rb", "a\0b"] {
        assert_eq!(
            Action::update_env_var("/etc/nodivara/.env", "APP_KEY", bad_val),
            Err(ActionError::BadEnvValue)
        );
    }

    let action =
        Action::update_env_var("/etc/nodivara/.env.prod", "API_SECRET", "super-secret-42")?;
    assert_eq!(action.tier(), Tier::RiskyChange);
    assert!(action.executable_now());
    assert!(action.requires_backup());
    assert!(!action.requires_typed_confirmation());
    assert_eq!(action.target_label(), "/etc/nodivara/.env.prod");
    assert_eq!(action.tool(), "propose_update_env_var");
    let dbg = format!("{action:?}");
    assert!(!dbg.contains("super-secret-42"), "{dbg}");

    let parsed = Action::parse(&action.canonical_json())?;
    assert_eq!(parsed, action);
    assert_eq!(parsed.hash(), action.hash());
    Ok(())
}

#[test]
#[cfg(unix)]
fn update_env_var_dry_run_backup_execute_and_rollback() -> TestResult {
    let root = TempDir::new("env")?;
    let backups = TempDir::new("env-bak")?;
    let env_path = root.path().join(".env");
    std::fs::write(&env_path, "EXISTING=old-secret-value\nOTHER=1\n")?;

    let action = Action::update_env_var(
        env_path.to_str().ok_or("utf8")?,
        "EXISTING",
        "brand-new-secret-value",
    )?;
    let exec =
        LocalExecutor::with_allowed_roots(FakeRunner::new(), vec![root.path().to_path_buf()]);

    // Dry-run redacts both old and new values.
    let dry = exec.dry_run(&action);
    assert_eq!(dry.current_state, "present");
    let diff = dry.diff.as_deref().ok_or("expected diff")?;
    assert!(diff.contains("EXISTING=[REDACTED]"), "{diff}");
    assert!(!diff.contains("old-secret-value"), "{diff}");
    assert!(!diff.contains("brand-new-secret-value"), "{diff}");
    assert!(!dry.steps.join(" ").contains("brand-new-secret-value"));

    // Backup, execute, verify.
    let snap = exec.backup(&action, backups.path())?;
    assert_eq!(snap.sha256.len(), 64);
    let out = exec.execute(&action);
    assert_eq!(
        (out.status, out.verified, out.reason),
        (OutcomeStatus::Succeeded, true, "ok")
    );
    assert!(!out.detail.contains("brand-new-secret-value"));
    let now = std::fs::read_to_string(&env_path)?;
    assert_eq!(now, "EXISTING=brand-new-secret-value\nOTHER=1\n");

    // Corrupted backup sha256 is refused on rollback.
    let mut bad_snap = snap.clone();
    bad_snap.sha256 = "0".repeat(64);
    let bad_rb = exec.rollback(&action, &bad_snap);
    assert_eq!(
        (bad_rb.status, bad_rb.reason),
        (OutcomeStatus::Failed, "backup_corrupted")
    );

    // Valid rollback restores original bytes.
    let rb = exec.rollback(&action, &snap);
    assert_eq!(
        (rb.status, rb.verified, rb.reason),
        (OutcomeStatus::Succeeded, true, "ok")
    );
    assert_eq!(
        std::fs::read_to_string(&env_path)?,
        "EXISTING=old-secret-value\nOTHER=1\n"
    );
    Ok(())
}

#[test]
#[cfg(unix)]
fn delete_managed_file_tier_4_backup_execute_rollback_and_symlinks() -> TestResult {
    let root = TempDir::new("del")?;
    let backups = TempDir::new("del-bak")?;
    let target = root.path().join("old-release.tar");
    std::fs::write(&target, b"archive-bytes-123")?;

    let action = Action::delete_managed_file(target.to_str().ok_or("utf8")?)?;
    assert_eq!(action.tier(), Tier::Destructive);
    assert!(action.executable_now());
    assert!(action.requires_backup());
    assert!(action.requires_typed_confirmation());
    assert_eq!(action.target_label(), target.to_str().ok_or("utf8")?);
    assert_eq!(action.tool(), "propose_delete_managed_file");

    let exec =
        LocalExecutor::with_allowed_roots(FakeRunner::new(), vec![root.path().to_path_buf()]);
    let dry = exec.dry_run(&action);
    assert!(dry.diff.is_some());

    let snap = exec.backup(&action, backups.path())?;
    let out = exec.execute(&action);
    assert_eq!(
        (out.status, out.verified, out.reason),
        (OutcomeStatus::Succeeded, true, "ok")
    );
    assert!(!target.exists());

    let rb = exec.rollback(&action, &snap);
    assert_eq!(
        (rb.status, rb.verified, rb.reason),
        (OutcomeStatus::Succeeded, true, "ok")
    );
    assert_eq!(std::fs::read(&target)?, b"archive-bytes-123");

    // Allowlist and symlink traversal are refused.
    let outside = Action::delete_managed_file("/etc/passwd")?;
    assert_eq!(exec.execute(&outside).reason, "path_not_allowed");

    #[cfg(unix)]
    {
        let link = root.path().join("escape-link");
        std::os::unix::fs::symlink(&target, &link)?;
        let sym_action = Action::delete_managed_file(link.to_str().ok_or("utf8")?)?;
        assert_eq!(exec.execute(&sym_action).reason, "symlink_not_allowed");
        assert_eq!(
            exec.backup(&sym_action, backups.path()),
            Err("symlink_not_allowed")
        );
    }
    Ok(())
}

#[test]
fn critical_restart_service_backup_and_rollback() -> TestResult {
    let backups = TempDir::new("svc-bak")?;
    let action = restart("sshd.service")?;
    assert_eq!(action.tier(), Tier::RiskyChange);
    assert!(action.requires_backup());
    assert_eq!(action.target_label(), "sshd.service");

    let runner = FakeRunner::new();
    // backup(): is-active -> active
    runner.push(Ok(RunOutput::ok("active\n")));
    // execute(): restart -> ok, is-active -> active
    runner.push(Ok(RunOutput::ok("")));
    runner.push(Ok(RunOutput::ok("active\n")));
    // rollback(): restart -> ok, is-active -> active
    runner.push(Ok(RunOutput::ok("")));
    runner.push(Ok(RunOutput::ok("active\n")));

    let exec = LocalExecutor::new(runner);
    let snap = exec.backup(&action, backups.path())?;
    let out = exec.execute(&action);
    assert_eq!((out.status, out.verified), (OutcomeStatus::Succeeded, true));
    let rb = exec.rollback(&action, &snap);
    assert_eq!((rb.status, rb.verified), (OutcomeStatus::Succeeded, true));
    Ok(())
}

#[test]
fn configure_site_tiers_canonical_json_parse_and_validation() -> TestResult {
    let local_site = Action::configure_site(
        "MyApp.Test",
        3000,
        Some(PhpVersion::V8_3),
        false,
        "/srv/myapp",
    )?;
    assert_eq!(local_site.tier(), Tier::LightChange);
    assert_eq!(local_site.tool(), "propose_configure_site");
    assert_eq!(local_site.target_label(), "myapp.test");
    assert_eq!(Action::parse(&local_site.canonical_json())?, local_site);

    let local_tls = Action::configure_site("myapp.test", 3000, None, true, "/srv/myapp")?;
    assert_eq!(local_tls.tier(), Tier::RiskyChange);

    let prod_site = Action::configure_site(
        "app.example.com",
        8080,
        Some(PhpVersion::V8_4),
        true,
        "/var/www/app",
    )?;
    assert_eq!(prod_site.tier(), Tier::RiskyChange);
    assert!(prod_site.requires_backup());
    assert_eq!(Action::parse(&prod_site.canonical_json())?, prod_site);

    // Reject bad domain, port 0, bad path, unknown php version.
    assert_eq!(
        Action::configure_site("-bad.com", 8080, None, false, "/srv/app"),
        Err(ActionError::BadDomain)
    );
    assert_eq!(
        Action::configure_site("app.example.com", 0, None, false, "/srv/app"),
        Err(ActionError::BadPort)
    );
    assert_eq!(
        Action::configure_site("app.example.com", 8080, None, false, "/srv/../etc"),
        Err(ActionError::BadPath)
    );
    let bad_php = r#"{"action":"configure_site","domain":"app.example.com","php_version":"7.4","root_dir":"/srv/app","tls_enabled":false,"upstream_port":8080}"#;
    assert_eq!(Action::parse(bad_php), Err(ActionError::UnknownPhpVersion));
    Ok(())
}

#[test]
fn configure_site_dry_run_backup_execute_and_rollback() -> TestResult {
    let dir = TempDir::new("site-cfg")?;
    let backups = TempDir::new("site-bak")?;
    let root_dir = dir.path().join("www");
    std::fs::create_dir_all(&root_dir)?;

    let action = Action::configure_site(
        "portal.example.com",
        4000,
        Some(PhpVersion::V8_2),
        true,
        &root_dir.to_string_lossy(),
    )?;
    let exec = LocalExecutor::with_allowed_roots(FakeRunner::new(), vec![dir.path().to_path_buf()]);

    let dry = exec.dry_run(&action);
    assert_eq!(dry.current_state, "absent");
    let diff = dry.diff.unwrap_or_default();
    assert!(diff.contains("server_name portal.example.com;"));
    assert!(diff.contains("php8.2-fpm.sock"));
    assert!(diff.contains("proxy_pass http://127.0.0.1:4000;"));

    let snap = exec.backup(&action, backups.path())?;
    let out = exec.execute(&action);
    assert_eq!((out.status, out.verified), (OutcomeStatus::Succeeded, true));

    let vhost_file = dir.path().join("sites").join("portal.example.com.conf");
    assert!(vhost_file.exists());
    let written = std::fs::read_to_string(&vhost_file)?;
    assert!(written.contains("listen 443 ssl http2;"));

    let rb = exec.rollback(&action, &snap);
    assert_eq!((rb.status, rb.verified), (OutcomeStatus::Succeeded, true));
    assert!(!vhost_file.exists());
    Ok(())
}

#[test]
fn issue_tls_cert_validation_argv_dry_run_backup_execute_and_rollback() -> TestResult {
    // Reject local/reserved TLDs and flag-injection.
    for bad in [
        "app.test",
        "dev.localhost",
        "printer.local",
        "-d evil.com",
        "bad;rm.com",
    ] {
        assert_eq!(
            Action::issue_tls_cert(bad, AcmeProvider::LetsEncrypt),
            Err(ActionError::BadDomain),
            "{bad}"
        );
    }
    let bad_provider =
        r#"{"acme_provider":"selfsigned","action":"issue_tls_cert","domain":"app.example.com"}"#;
    assert_eq!(
        Action::parse(bad_provider),
        Err(ActionError::UnknownAcmeProvider)
    );

    let backups = TempDir::new("tls-bak")?;
    let action = Action::issue_tls_cert("app.example.com", AcmeProvider::LetsEncrypt)?;
    assert_eq!(action.tier(), Tier::RiskyChange);
    assert!(action.requires_backup());
    assert_eq!(action.tool(), "propose_issue_tls_cert");
    assert_eq!(Action::parse(&action.canonical_json())?, action);

    let runner = FakeRunner::new();
    // dry_run: status check -> absent (exit 1)
    runner.push(Ok(RunOutput::failed(1, "")));
    // backup: status check -> absent (exit 1)
    runner.push(Ok(RunOutput::failed(1, "")));
    // execute: certbot certonly -> ok, certbot certificates -> ok
    runner.push(Ok(RunOutput::ok("Successfully received certificate.")));
    runner.push(Ok(RunOutput::ok("Certificate Name: app.example.com")));
    // rollback (existed == false): certbot delete -> ok
    runner.push(Ok(RunOutput::ok("Deleted")));

    let exec = LocalExecutor::new(runner);
    let dry = exec.dry_run(&action);
    assert_eq!(dry.current_state, "absent");
    assert_eq!(
        dry.steps[0],
        r#"certbot "certonly" "--non-interactive" "--agree-tos" "--nginx" "--dry-run" "-d" "app.example.com""#
    );
    assert_eq!(
        dry.steps[1],
        r#"certbot "certonly" "--non-interactive" "--agree-tos" "--nginx" "-d" "app.example.com""#
    );

    let snap = exec.backup(&action, backups.path())?;
    let out = exec.execute(&action);
    assert_eq!((out.status, out.verified), (OutcomeStatus::Succeeded, true));
    let rb = exec.rollback(&action, &snap);
    assert_eq!((rb.status, rb.verified), (OutcomeStatus::Succeeded, true));
    Ok(())
}

#[test]
fn backup_database_validation_argv_dry_run_backup_execute_and_rollback() -> TestResult {
    // Reject flag-injection and unsafe DB names.
    for bad in ["--all-databases", "-h", "db;drop", "db name", "db.name", ""] {
        assert_eq!(
            Action::backup_database(DbEngine::Mysql, bad, "/var/backups/nodivara/db.sql"),
            Err(ActionError::BadDbName),
            "{bad}"
        );
    }
    let bad_engine = r#"{"action":"backup_database","db_name":"app_prod","engine":"sqlite","output_path":"/var/backups/nodivara/app.sql"}"#;
    assert_eq!(Action::parse(bad_engine), Err(ActionError::UnknownDbEngine));

    let dir = TempDir::new("db-dump")?;
    let backups = TempDir::new("db-bak")?;
    let dump_path = dir.path().join("app_prod.dump");
    let action = Action::backup_database(
        DbEngine::Postgres,
        "app_prod_01",
        &dump_path.to_string_lossy(),
    )?;
    assert_eq!(action.tier(), Tier::LightChange);
    assert_eq!(action.tool(), "propose_backup_database");
    assert_eq!(Action::parse(&action.canonical_json())?, action);

    let runner = FakeRunner::new();
    // execute: pg_dump -> ok, test -s -> ok
    runner.push(Ok(RunOutput::ok("")));
    runner.push(Ok(RunOutput::ok("")));

    let exec = LocalExecutor::with_allowed_roots(runner, vec![dir.path().to_path_buf()]);
    let dry = exec.dry_run(&action);
    assert_eq!(dry.current_state, "absent");
    assert!(dry.steps[0].starts_with(r#"pg_dump "--format=custom""#));
    assert!(dry.steps[0].contains(r#""--" "app_prod_01""#));
    assert!(!dry.steps[0].contains("password") && !dry.steps[0].contains(" -p "));

    let snap = exec.backup(&action, backups.path())?;
    let out = exec.execute(&action);
    assert_eq!((out.status, out.verified), (OutcomeStatus::Succeeded, true));

    // Simulate the created dump file on disk and verify rollback removes it.
    std::fs::write(&dump_path, b"PGDMP")?;
    let rb = exec.rollback(&action, &snap);
    assert_eq!((rb.status, rb.verified), (OutcomeStatus::Succeeded, true));
    assert!(!dump_path.exists());
    Ok(())
}
