//! Shared test support: the contract suite every [`TransportSpec`] kind must pass, plus fixtures
//! and the hostile-input list. Plain `pub` on purpose (no feature flag), like
//! `notifier::testing`: other crates run it on their own specs.

use crate::plan::{CommandPlan, CopyOptions, Credentials, EnvValue, Operation, PlanError, plan};
use crate::spec::{RcloneSpec, RemoteKind, SpecError, SshTarget, TransportKind, TransportSpec};
use core_domain::secret::Secret;
use std::collections::BTreeMap;

/// A recognisable secret value. If it shows up anywhere but an environment value, a test fails.
pub const SENTINEL: &str = "S3CR3T-sentinel-0123456789abcdef";

/// A spec with the credentials that go with it.
#[derive(Debug)]
pub struct Fixture {
    /// Label for failure messages.
    pub label: String,
    /// The spec.
    pub spec: TransportSpec,
    /// Credentials, for kinds that take them.
    pub credentials: Option<Credentials>,
}

fn rclone_credentials(kind: RemoteKind) -> Option<Credentials> {
    let key = match kind {
        RemoteKind::S3 => "secret_access_key",
        RemoteKind::B2 => "key",
        RemoteKind::Sftp | RemoteKind::Webdav => "pass",
        RemoteKind::Drive | RemoteKind::Dropbox | RemoteKind::Onedrive => "client_secret",
    };
    let mut pairs = vec![(key.to_owned(), Secret::new(format!("{SENTINEL}-key")))];
    if kind.needs_oauth() {
        pairs.push((
            "token".to_owned(),
            Secret::new(format!("{{\"access_token\":\"{SENTINEL}-token\"}}")),
        ));
    }
    Credentials::new(pairs).ok()
}

/// One fixture per transport kind, one rclone fixture per backend, and an IPv6 SSH target.
#[must_use]
pub fn fixtures() -> Vec<Fixture> {
    let mut out = Vec::new();
    for kind in RemoteKind::ALL {
        let mut options = BTreeMap::new();
        let (name, value) = match kind {
            RemoteKind::S3 | RemoteKind::Onedrive => ("region", "eu-west-1"),
            RemoteKind::B2 => ("endpoint", "https://b2.example.org"),
            RemoteKind::Sftp => ("host", "backup.example.org"),
            RemoteKind::Webdav => ("url", "https://dav.example.org"),
            RemoteKind::Drive => ("scope", "drive"),
            RemoteKind::Dropbox => ("client_id", "app-id"),
        };
        options.insert(name.to_owned(), value.to_owned());
        if let Ok(spec) = RcloneSpec::new(kind, "backup_remote", "nodivara/some dir", options) {
            out.push(Fixture {
                label: format!("rclone-{}", kind.as_str()),
                spec: TransportSpec::Rclone(spec),
                credentials: rclone_credentials(kind),
            });
        }
    }
    for host in ["backup.example.test", "2001:db8::7"] {
        if let Ok(t) = SshTarget::new(host, "deploy", Some(2222), "/srv/backups/in") {
            for (label, spec) in [
                ("rsync", TransportSpec::Rsync(t.clone())),
                ("scp", TransportSpec::Scp(t.clone())),
                ("ssh_pipe", TransportSpec::SshPipe(t.clone())),
            ] {
                out.push(Fixture {
                    label: format!("{label}-{host}"),
                    spec,
                    credentials: None,
                });
            }
        }
    }
    out.push(Fixture {
        label: "managed".to_owned(),
        spec: TransportSpec::Managed,
        credentials: None,
    });
    out
}

/// Every operation, with and without tuning.
#[must_use]
pub fn operations() -> Vec<Operation> {
    let tuned = CopyOptions {
        excludes: vec!["*.log".to_owned(), "node_modules/".to_owned()],
        dry_run: true,
        transfers: Some(4),
    };
    vec![
        Operation::Probe,
        Operation::Verify,
        Operation::Quota,
        Operation::Authorize,
        Operation::SpeedWrite { bytes: 1024 },
        Operation::SpeedCleanup,
        Operation::Push {
            local: "/var/backups/site".to_owned(),
            opts: CopyOptions::default(),
        },
        Operation::Pull {
            local: "/var/restore/site".to_owned(),
            opts: CopyOptions::default(),
        },
        Operation::Push {
            local: "/var/backups/site".to_owned(),
            opts: tuned.clone(),
        },
        Operation::Pull {
            local: "/var/restore/site".to_owned(),
            opts: tuned,
        },
    ]
}

/// Inputs a misbehaving caller or a hostile server could supply. Each must be rejected or end up
/// as one inert argument after `--`.
#[must_use]
pub fn hostile_inputs() -> Vec<String> {
    let mut v: Vec<String> = [
        "-oProxyCommand=evil",
        "--rsh=evil",
        "-",
        "--",
        "-rf",
        "a\nb",
        "a\rb",
        "a\0b",
        "a\tb",
        "a;b",
        "a|b",
        "a&b",
        "a&&b",
        "$(id)",
        "${HOME}",
        "`id`",
        "a b",
        " a",
        "a'b",
        "a\"b",
        "a\\b",
        "../etc",
        "a/../b",
        "x\u{202e}gnp.exe",
        "a\u{200b}b",
        "a\u{85}b",
        "a\u{2028}b",
        "\u{feff}a",
        "\u{ad}a",
        "caf\u{e9}",
        "\u{ff0d}x",
        "\u{2010}x",
        "a:b",
        "%s",
        "{a,b}",
        "*",
        "~",
        "a=b",
        "user@evil",
        "host:22",
        "[::1]",
        "",
        "/tmp/$(id)",
        "/tmp/a`id`",
        "/tmp/ a;b",
        "/tmp/-x",
        "/tmp/a\nb",
        "/tmp/../x",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    v.push("a".repeat(5000));
    v.push(format!("/{}", "a".repeat(5000)));
    v
}

fn risky_for_shell(s: &str) -> bool {
    s.starts_with('-')
        || s.is_empty()
        || !s.is_ascii()
        || s.chars()
            .any(|c| c.is_control() || " ;|&$`'\"\\<>(){}*?[]!#~".contains(c))
}

fn separator(args: &[String]) -> Option<usize> {
    args.iter().position(|a| a == "--")
}

fn secret_values(credentials: Option<&Credentials>) -> Vec<String> {
    credentials
        .map(|c| {
            c.pairs()
                .iter()
                .map(|(_, v)| v.expose_secret().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

fn check_plan(label: &str, op: &Operation, p: &CommandPlan, secrets: &[String]) {
    let ctx = format!("{label} {op:?}");
    assert!(
        ["rclone", "rsync", "scp", "ssh"].contains(&p.program),
        "{ctx}: program {} is not a fixed tool (no shell)",
        p.program
    );
    for arg in &p.args {
        assert!(!arg.contains('\0'), "{ctx}: NUL in an argument");
        assert!(
            !arg.contains('\n') && !arg.contains('\r'),
            "{ctx}: newline in an argument"
        );
        for secret in secrets {
            assert!(!arg.contains(secret.as_str()), "{ctx}: secret in args");
        }
    }
    assert!(
        !p.args.iter().any(|a| a == "-c"),
        "{ctx}: looks like a shell invocation"
    );
    if !matches!(op, Operation::Probe) {
        assert!(
            separator(&p.args).is_some(),
            "{ctx}: no `--` in {:?}",
            p.args
        );
    }
    // Everything after `--` must be a positional operand we built from validated fields.
    for (name, value) in &p.env {
        assert!(
            name.starts_with("RCLONE_CONFIG_"),
            "{ctx}: unexpected environment variable {name}"
        );
        if let EnvValue::Plain(v) = value {
            for secret in secrets {
                assert!(!v.contains(secret.as_str()), "{ctx}: secret in plain env");
            }
        }
    }
    for rendered in [format!("{p}"), format!("{p:?}")] {
        for secret in secrets {
            assert!(!rendered.contains(secret.as_str()), "{ctx}: secret printed");
        }
    }
}

fn with_path(spec: &TransportSpec, value: &str) -> Option<Result<TransportSpec, SpecError>> {
    Some(match spec {
        TransportSpec::Rclone(s) => {
            RcloneSpec::new(s.kind(), s.remote(), value, s.options().clone())
                .map(TransportSpec::Rclone)
        }
        TransportSpec::Rsync(t) => {
            SshTarget::new(t.host(), t.user(), Some(t.port()), value).map(TransportSpec::Rsync)
        }
        TransportSpec::Scp(t) => {
            SshTarget::new(t.host(), t.user(), Some(t.port()), value).map(TransportSpec::Scp)
        }
        TransportSpec::SshPipe(t) => {
            SshTarget::new(t.host(), t.user(), Some(t.port()), value).map(TransportSpec::SshPipe)
        }
        TransportSpec::Managed => return None,
    })
}

fn with_host(spec: &TransportSpec, value: &str) -> Option<Result<TransportSpec, SpecError>> {
    let (t, wrap): (&SshTarget, fn(SshTarget) -> TransportSpec) = match spec {
        TransportSpec::Rsync(t) => (t, TransportSpec::Rsync),
        TransportSpec::Scp(t) => (t, TransportSpec::Scp),
        TransportSpec::SshPipe(t) => (t, TransportSpec::SshPipe),
        _ => return None,
    };
    Some(SshTarget::new(value, t.user(), Some(t.port()), t.path()).map(wrap))
}

fn with_user(spec: &TransportSpec, value: &str) -> Option<Result<TransportSpec, SpecError>> {
    let (t, wrap): (&SshTarget, fn(SshTarget) -> TransportSpec) = match spec {
        TransportSpec::Rsync(t) => (t, TransportSpec::Rsync),
        TransportSpec::Scp(t) => (t, TransportSpec::Scp),
        TransportSpec::SshPipe(t) => (t, TransportSpec::SshPipe),
        _ => return None,
    };
    Some(SshTarget::new(t.host(), value, Some(t.port()), t.path()).map(wrap))
}

fn with_remote(spec: &TransportSpec, value: &str) -> Option<Result<TransportSpec, SpecError>> {
    match spec {
        TransportSpec::Rclone(s) => Some(
            RcloneSpec::new(s.kind(), value, s.path(), s.options().clone())
                .map(TransportSpec::Rclone),
        ),
        _ => None,
    }
}

/// An accepted hostile value must travel as part of exactly one argument that comes after `--`.
fn assert_inert(label: &str, field: &str, hostile: &str, spec: &TransportSpec) {
    let ctx = format!("{label} {field} {hostile:?}");
    let op = Operation::Push {
        local: "/var/backups/site".to_owned(),
        opts: CopyOptions::default(),
    };
    let Ok(p) = plan(spec, None, &op) else {
        // Rejecting at plan time is also safe.
        return;
    };
    let Some(sep) = separator(&p.args) else {
        panic!("{ctx}: no `--`");
    };
    let carriers: Vec<usize> = p
        .args
        .iter()
        .enumerate()
        .filter(|(_, a)| a.contains(hostile))
        .map(|(i, _)| i)
        .collect();
    assert!(!carriers.is_empty(), "{ctx}: value vanished from the plan");
    for i in carriers {
        assert!(
            i > sep,
            "{ctx}: user-derived value before `--` in {:?}",
            p.args
        );
    }
    // The value never splits into extra arguments: same count as with a benign value.
    if field == "path"
        && let Some(Ok(benign)) = with_path(spec, "benign")
        && let Ok(b) = plan(&benign, None, &op)
    {
        assert_eq!(b.args.len(), p.args.len(), "{ctx}: argument count changed");
    }
}

/// The contract every transport must satisfy.
///
/// # Panics
/// On the first violated rule, with the fixture label and operation in the message.
pub fn contract_suite(fx: &Fixture) {
    let label = fx.label.as_str();
    let secrets = secret_values(fx.credentials.as_ref());
    let kind = fx.spec.kind();

    // Debug and Display of the spec, its JSON and the credentials never carry a secret.
    for rendered in [
        format!("{:?}", fx.spec),
        fx.spec.to_config_json(),
        format!("{:?}", fx.credentials),
    ] {
        for secret in &secrets {
            assert!(
                !rendered.contains(secret.as_str()),
                "{label}: secret printed"
            );
        }
    }
    // The stored config round-trips and is re-validated.
    let back = TransportSpec::from_config_json(kind.as_str(), &fx.spec.to_config_json());
    assert_eq!(back.as_ref(), Ok(&fx.spec), "{label}: config round trip");

    if kind == TransportKind::Managed {
        for op in operations() {
            assert_eq!(
                plan(&fx.spec, None, &op).err(),
                Some(PlanError::UnsupportedManaged),
                "{label}: managed must be Unsupported for {op:?}"
            );
        }
        return;
    }

    // Credentials are refused where the kind takes none.
    if !kind.accepts_credentials() {
        let creds = Credentials::new(vec![("password".to_owned(), Secret::new(SENTINEL))]);
        if let Ok(c) = creds {
            assert_eq!(
                plan(&fx.spec, Some(&c), &Operation::Verify).err(),
                Some(PlanError::CredentialsNotAccepted(kind)),
                "{label}: credentials must be refused"
            );
        }
    }

    for op in operations() {
        let first = plan(&fx.spec, fx.credentials.as_ref(), &op);
        let p = match first {
            Ok(p) => p,
            Err(PlanError::UnsupportedOperation(k)) => {
                assert_eq!(k, kind, "{label}: wrong kind in Unsupported");
                continue;
            }
            Err(e) => panic!("{label} {op:?}: unexpected error {e}"),
        };
        check_plan(label, &op, &p, &secrets);
        // Identical input gives an identical plan.
        let second = plan(&fx.spec, fx.credentials.as_ref(), &op);
        assert_eq!(
            second.as_ref().ok(),
            Some(&p),
            "{label} {op:?}: plan not deterministic"
        );
        // Rclone: the secret is injected through the environment, not left out.
        if kind == TransportKind::Rclone && matches!(op, Operation::Verify | Operation::Quota) {
            for secret in &secrets {
                assert!(
                    p.env
                        .iter()
                        .any(|(_, v)| v.is_secret() && v.expose() == secret),
                    "{label}: secret missing from the environment"
                );
            }
        }
    }

    // Hostile inputs for every field.
    for hostile in hostile_inputs() {
        let h = hostile.as_str();
        let fields: [(&str, Option<Result<TransportSpec, SpecError>>); 4] = [
            ("path", with_path(&fx.spec, h)),
            ("host", with_host(&fx.spec, h)),
            ("user", with_user(&fx.spec, h)),
            ("remote", with_remote(&fx.spec, h)),
        ];
        for (field, built) in fields {
            let Some(built) = built else { continue };
            let strict = match field {
                "path" => kind != TransportKind::Rclone,
                "host" | "user" | "remote" => true,
                _ => false,
            };
            match built {
                Err(_) => {}
                Ok(spec) => {
                    assert!(
                        !(strict && risky_for_shell(h)),
                        "{label} {field}: {h:?} must be rejected"
                    );
                    // Never a leading dash or control character, even where spaces are fine.
                    assert!(
                        !h.starts_with('-') && !h.chars().any(char::is_control),
                        "{label} {field}: {h:?} must be rejected"
                    );
                    if !h.is_empty() {
                        assert_inert(label, field, h, &spec);
                    }
                }
            }
        }
        // Local paths and excludes.
        let ops = [
            Operation::Push {
                local: hostile.clone(),
                opts: CopyOptions::default(),
            },
            Operation::Pull {
                local: hostile.clone(),
                opts: CopyOptions::default(),
            },
        ];
        for op in ops {
            if let Ok(p) = plan(&fx.spec, fx.credentials.as_ref(), &op) {
                assert!(
                    h.starts_with('/'),
                    "{label}: relative local path {h:?} accepted"
                );
                assert!(
                    !h.chars().any(char::is_control),
                    "{label}: control char accepted"
                );
                let sep = separator(&p.args);
                assert!(sep.is_some(), "{label}: no `--`");
                for (i, a) in p.args.iter().enumerate() {
                    if a == h {
                        assert!(Some(i) > sep, "{label}: local path before `--`");
                    }
                }
            }
        }
        let with_exclude = Operation::Push {
            local: "/var/backups/site".to_owned(),
            opts: CopyOptions {
                excludes: vec![hostile.clone()],
                ..CopyOptions::default()
            },
        };
        if let Ok(p) = plan(&fx.spec, fx.credentials.as_ref(), &with_exclude) {
            assert!(
                !h.starts_with('-') && !h.chars().any(char::is_control) && !h.is_empty(),
                "{label}: exclude {h:?} accepted"
            );
            let want = format!("--exclude={h}");
            assert_eq!(
                p.args.iter().filter(|a| **a == want).count(),
                1,
                "{label}: exclude must be exactly one argument"
            );
            assert!(
                !p.args.iter().any(|a| a == h),
                "{label}: bare exclude value became its own argument"
            );
        }
    }
}
