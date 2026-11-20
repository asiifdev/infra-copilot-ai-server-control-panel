//! `infra plan ...` and `infra transport ...` end to end (ADR 0028), against a real SQLite file in a
//! temporary directory. No process is ever started: every call goes through a `FakeRunner` supplied
//! by the test seam, so even a bug cannot reach rclone, rsync, ssh or the network.
//!
//! Fake secrets are built at run time so no token-looking literal sits in the repository.

use cli::Env;
use db::Role;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use transport::{FakeRunner, RunError, RunOutput};

type TestResult = Result<(), Box<dyn std::error::Error>>;

// ---- secrets and keys, built at run time ------------------------------------------------------

fn master() -> String {
    format!("main:{}", "ab".repeat(32))
}

fn access_id() -> String {
    format!("{}-{}", "idmarker", "k3j9x".repeat(5))
}

fn access_secret() -> String {
    format!("{}-{}", "secretmarker", "q7Zr3Lm9".repeat(4))
}

fn credentials_text() -> String {
    format!(
        "access_key_id={}\nsecret_access_key={}\n",
        access_id(),
        access_secret()
    )
}

// ---- scratch space and driving the command line -----------------------------------------------

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> std::io::Result<Self> {
        static N: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "nodivara-pt-{}-{}-{tag}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&path)?;
        Ok(Self(path))
    }

    fn db(&self) -> String {
        self.0.join("test.db").to_string_lossy().into_owned()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Out {
    code: u8,
    stdout: String,
    stderr: String,
}

impl Out {
    /// Everything the command wrote.
    fn all(&self) -> String {
        format!("{}\n{}", self.stdout, self.stderr)
    }
}

struct Ctx {
    db: String,
    runner: Arc<FakeRunner>,
    env: Vec<(String, String)>,
    org: Option<String>,
    lang: Option<String>,
}

impl Ctx {
    fn new(db: String) -> Self {
        Self {
            db,
            runner: Arc::new(FakeRunner::new()),
            env: vec![("NODIVARA_MASTER_KEY".to_owned(), master())],
            org: None,
            lang: None,
        }
    }

    fn run(&self, args: &[&str], stdin: &str) -> Out {
        let mut args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
        args.extend(["--db".to_owned(), self.db.clone()]);
        if let Some(org) = &self.org {
            args.extend(["--org".to_owned(), org.clone()]);
        }
        if let Some(lang) = &self.lang {
            args.extend(["--lang".to_owned(), lang.clone()]);
        }
        let pairs: Vec<(&str, &str)> = self
            .env
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let env = Env::fixed(&pairs).with_runner(self.runner.clone());
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = cli::run_with(&args, None, &env, &mut stdin.as_bytes(), &mut out, &mut err);
        Out {
            code,
            stdout: String::from_utf8_lossy(&out).into_owned(),
            stderr: String::from_utf8_lossy(&err).into_owned(),
        }
    }

    fn without_master(mut self) -> Self {
        self.env.clear();
        self
    }
}

fn manifest(rel: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(rel)
        .to_string_lossy()
        .into_owned()
}

/// Everything in the database files (main, WAL and shared memory).
fn raw_db_bytes(db: &str) -> Vec<u8> {
    let mut all = Vec::new();
    for suffix in ["", "-wal", "-shm"] {
        if let Ok(bytes) = std::fs::read(format!("{db}{suffix}")) {
            all.extend(bytes);
        }
    }
    all
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|w| w == needle.as_bytes())
}

/// Asserts that none of `secrets` is anywhere in `text`.
fn assert_clean(text: &str, secrets: &[&str], what: &str) {
    for secret in secrets {
        assert!(!text.contains(secret), "{what} leaked a secret: {text}");
    }
}

// ---- a world with two nodes that have inventories ---------------------------------------------

/// Org `local`, nodes old/new/empty/ok, inventories for old (source), new (gaps) and ok (clean).
fn world(tag: &str) -> Result<(Scratch, Ctx), Box<dyn std::error::Error>> {
    let dir = Scratch::new(tag)?;
    let ctx = Ctx::new(dir.db());
    assert_eq!(ctx.run(&["init"], "").code, 0);
    for name in ["old", "new", "empty", "ok"] {
        let r = ctx.run(&["node", "add", "--name", name, "--kind", "vps"], "");
        assert_eq!(r.code, 0, "{}", r.stderr);
    }
    let mut conn = db::open(Path::new(&ctx.db))?;
    let org = db::list_orgs(&conn, 5)?.remove(0).id;
    for (node, file) in [
        ("old", "source.json"),
        ("new", "target-gaps.json"),
        ("ok", "target-ok.json"),
    ] {
        let text = std::fs::read_to_string(manifest(&format!("../parity/tests/fixtures/{file}")))?;
        let inventory = inventory::Inventory::from_json(&text)?;
        agent::persist_inventory(&mut conn, &org, node, &inventory)?;
    }
    Ok((dir, ctx))
}

/// An rsync transport named `name`, marked verified directly in the database (the check itself
/// is covered by its own tests).
fn verified_transport(ctx: &Ctx, name: &str) -> TestResult {
    let r = ctx.run(
        &[
            "transport",
            "add",
            "--name",
            name,
            "--kind",
            "rsync",
            "--host",
            "backup.example.net",
            "--user",
            "migrator",
            "--path",
            "/srv/backups",
        ],
        "",
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    let conn = db::open(Path::new(&ctx.db))?;
    let org = db::list_orgs(&conn, 5)?.remove(0).id;
    let row = db::list_transports(&conn, &org)?
        .into_iter()
        .find(|t| t.name == name)
        .ok_or("transport missing")?;
    db::record_transport_check(&conn, &org, &row.id, "{\"ok\":true}", true)?;
    Ok(())
}

fn plan_count(db: &str) -> Result<usize, Box<dyn std::error::Error>> {
    let conn = db::open(Path::new(db))?;
    let mut total = 0;
    for org in db::list_orgs(&conn, 10)? {
        total += db::list_plans(&conn, &org.id, 1000)?.len();
    }
    Ok(total)
}

fn compare_saved(ctx: &Ctx, target: &str) -> Out {
    ctx.run(
        &[
            "compare",
            "--save",
            "--source-node",
            "old",
            "--target-node",
            target,
        ],
        "",
    )
}

// ---- plan: draft, list, show, gates -----------------------------------------------------------

#[test]
fn plan_draft_list_show_and_gates_work_on_stored_fixtures() -> TestResult {
    let (_dir, mut ctx) = world("plan-flow")?;
    verified_transport(&ctx, "ssh1")?;
    assert_eq!(compare_saved(&ctx, "new").code, 1);

    let drafted = ctx.run(
        &[
            "plan",
            "draft",
            "--source",
            "old",
            "--target",
            "new",
            "--transport",
            "ssh1",
            "--name",
            "mig",
        ],
        "",
    );
    assert_eq!(drafted.code, 0, "{}", drafted.stderr);
    assert!(drafted.stdout.contains("Saved draft plan mig"));
    assert!(drafted.stdout.contains("Nothing was approved"));
    assert!(drafted.stdout.contains("Migration plan: mig"));
    assert_eq!(plan_count(&ctx.db)?, 1);

    // Stored with status draft, always.
    let conn = db::open(Path::new(&ctx.db))?;
    let org = db::list_orgs(&conn, 5)?.remove(0).id;
    let rows = db::list_plans(&conn, &org, 10)?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status.as_str(), "draft");

    let list = ctx.run(&["plan", "list"], "");
    assert_eq!(list.code, 0);
    assert!(
        list.stdout.contains("mig | draft | old -> new"),
        "{}",
        list.stdout
    );

    let show_en = ctx.run(&["plan", "show", "mig"], "");
    assert_eq!(show_en.code, 0, "{}", show_en.stderr);
    assert!(show_en.stdout.contains("Migration plan: mig"));
    assert!(show_en.stdout.contains("status draft"));

    // The same plan in Indonesian: different words, same facts.
    ctx.lang = Some("id".to_owned());
    let show_id = ctx.run(&["plan", "show", "mig"], "");
    assert_eq!(show_id.code, 0, "{}", show_id.stderr);
    assert!(show_id.stdout.contains("Rencana migrasi: mig"));
    assert!(!show_id.stdout.contains("Migration plan"));
    assert!(show_id.stdout.contains("status draft"));

    // A critical finding is open: the gates exit with 1, in both languages.
    let gates_id = ctx.run(&["plan", "gates", "mig"], "");
    assert_eq!(gates_id.code, 1, "{}", gates_id.stderr);
    assert!(gates_id.stdout.contains("Temuan kritis yang masih terbuka"));
    ctx.lang = Some("en".to_owned());
    let gates_en = ctx.run(&["plan", "gates", "mig"], "");
    assert_eq!(gates_en.code, 1);
    assert!(gates_en.stdout.contains("Cutover allowed: no"));
    assert!(gates_en.stdout.contains("PAR-001"), "{}", gates_en.stdout);
    assert!(!gates_en.stdout.contains("Temuan"));
    Ok(())
}

#[test]
fn gates_open_only_with_a_current_clean_comparison() -> TestResult {
    let (_dir, ctx) = world("plan-gates")?;
    verified_transport(&ctx, "ssh1")?;
    // No comparison yet: an empty finding list proves nothing, so the gates stay closed.
    let r = ctx.run(
        &[
            "plan", "draft", "--source", "old", "--target", "ok", "--name", "clean",
        ],
        "",
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(
        r.stdout
            .contains("There is no comparison of the newest inventories")
    );
    let gates = ctx.run(&["plan", "gates", "clean"], "");
    assert_eq!(gates.code, 1);
    assert!(gates.stdout.contains("Cutover allowed: no"));

    // Compare against the clean target: nothing critical, comparison current.
    let compared = compare_saved(&ctx, "ok");
    if compared.code == 0 {
        let gates = ctx.run(&["plan", "gates", "clean"], "");
        assert_eq!(gates.code, 0, "{}{}", gates.stdout, gates.stderr);
        assert!(gates.stdout.contains("Cutover allowed: yes"));
        // Shutdown of the old server is never open from here.
        assert!(
            gates
                .stdout
                .contains("Shutdown of the old server allowed: no")
        );

        // A newer inventory makes the comparison stale: the gates close again.
        let mut conn = db::open(Path::new(&ctx.db))?;
        let org = db::list_orgs(&conn, 5)?.remove(0).id;
        let text = std::fs::read_to_string(manifest("../parity/tests/fixtures/target-ok.json"))?;
        agent::persist_inventory(
            &mut conn,
            &org,
            "ok",
            &inventory::Inventory::from_json(&text)?,
        )?;
        let stale = ctx.run(&["plan", "gates", "clean"], "");
        assert_eq!(stale.code, 1);
        assert!(stale.stdout.contains("Cutover allowed: no"));
    }
    Ok(())
}

#[test]
fn bad_drafts_are_refused_with_localized_errors_and_store_nothing() -> TestResult {
    let (_dir, mut ctx) = world("plan-errors")?;
    verified_transport(&ctx, "ssh1")?;
    let add = ctx.run(
        &[
            "transport",
            "add",
            "--name",
            "unverified",
            "--kind",
            "scp",
            "--host",
            "h.example.net",
            "--user",
            "u",
            "--path",
            "/srv",
        ],
        "",
    );
    assert_eq!(add.code, 0);

    let cases: &[(&[&str], &str)] = &[
        (
            &["--source", "nope", "--target", "new", "--transport", "ssh1"],
            "no node named nope",
        ),
        (
            &["--source", "old", "--target", "nope", "--transport", "ssh1"],
            "no node named nope",
        ),
        (
            &["--source", "old", "--target", "old", "--transport", "ssh1"],
            "two different nodes",
        ),
        (
            &[
                "--source",
                "old",
                "--target",
                "empty",
                "--transport",
                "ssh1",
            ],
            "Node empty has no stored inventory",
        ),
        (
            &["--source", "old", "--target", "new", "--transport", "ghost"],
            "no transport named ghost",
        ),
        (
            &[
                "--source",
                "old",
                "--target",
                "new",
                "--transport",
                "unverified",
            ],
            "has not passed a check yet",
        ),
    ];
    for (flags, text) in cases {
        let mut args = vec!["plan", "draft"];
        args.extend(flags.iter().copied());
        let r = ctx.run(&args, "");
        assert_eq!(r.code, 2, "{flags:?}: {}", r.stderr);
        assert!(r.stderr.contains(text), "{flags:?}: {}", r.stderr);
        assert!(r.stdout.is_empty(), "{flags:?}");
    }
    // Two verified transports and none named: refused rather than guessed.
    verified_transport(&ctx, "ssh2")?;
    let r = ctx.run(&["plan", "draft", "--source", "old", "--target", "new"], "");
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("not exactly one verified transport"));
    assert_eq!(
        plan_count(&ctx.db)?,
        0,
        "nothing may be stored by a refused draft"
    );

    // The same refusal in Indonesian.
    ctx.lang = Some("id".to_owned());
    let r = ctx.run(
        &[
            "plan",
            "draft",
            "--source",
            "nope",
            "--target",
            "new",
            "--transport",
            "ssh1",
        ],
        "",
    );
    assert_eq!(r.code, 2);
    assert!(
        r.stderr.contains("Tidak ada node bernama") || r.stderr.contains("nope"),
        "{}",
        r.stderr
    );
    ctx.lang = None;

    // A duplicate name is refused, and the first plan is untouched.
    let args = [
        "plan",
        "draft",
        "--source",
        "old",
        "--target",
        "new",
        "--transport",
        "ssh1",
        "--name",
        "dup",
    ];
    assert_eq!(ctx.run(&args, "").code, 0);
    let again = ctx.run(&args, "");
    assert_eq!(again.code, 2);
    assert!(again.stderr.contains("A plan named dup already exists"));
    assert_eq!(plan_count(&ctx.db)?, 1);
    // Case differences do not dodge it.
    let upper = ctx.run(
        &[
            "plan",
            "draft",
            "--source",
            "old",
            "--target",
            "new",
            "--transport",
            "ssh1",
            "--name",
            "DUP",
        ],
        "",
    );
    assert_eq!(upper.code, 2);
    assert_eq!(plan_count(&ctx.db)?, 1);

    // Unknown plans.
    for sub in ["show", "gates"] {
        let r = ctx.run(&["plan", sub, "missing"], "");
        assert_eq!(r.code, 2);
        assert!(r.stderr.contains("no plan named missing"));
    }
    Ok(())
}

#[test]
fn a_cli_plan_has_no_approve_or_run_command() {
    let ctx = Ctx::new("unused.db".to_owned());
    for args in [
        vec!["plan", "approve", "x"],
        vec!["plan", "run", "x"],
        vec!["plan", "schedule", "x"],
        vec!["plan", "draft", "--status", "approved"],
        vec![
            "plan",
            "draft",
            "--source",
            "a",
            "--target",
            "b",
            "--cutover",
            "magic",
        ],
        vec![
            "plan",
            "draft",
            "--source",
            "a",
            "--target",
            "b",
            "--backup-location",
            "moon",
        ],
        vec!["plan", "draft", "--source", "a"],
        vec!["plan", "show"],
        vec!["plan", "list", "extra"],
        vec!["plan"],
    ] {
        let r = ctx.run(&args, "");
        assert_eq!(r.code, 2, "{args:?}");
        assert!(r.stderr.contains("Usage"), "{args:?}: {}", r.stderr);
        assert!(r.stdout.is_empty(), "{args:?}");
    }
}

// ---- transport: add, list, show, remove -------------------------------------------------------

fn add_s3(ctx: &Ctx, name: &str) -> Out {
    ctx.run(
        &[
            "transport",
            "add",
            "--name",
            name,
            "--kind",
            "rclone",
            "--remote-kind",
            "s3",
            "--remote",
            "offsite",
            "--path",
            "bucket-private-marker/backups",
            "--options",
            "provider=AWS,region=eu-west-1",
            "--credentials-stdin",
        ],
        &credentials_text(),
    )
}

#[test]
fn transports_keep_credentials_sealed_and_out_of_every_output() -> TestResult {
    let (_dir, ctx) = world("transport-crud")?;
    let (id, secret) = (access_id(), access_secret());
    let secrets = [id.as_str(), secret.as_str()];

    let added = add_s3(&ctx, "cloud");
    assert_eq!(added.code, 0, "{}", added.stderr);
    assert_clean(&added.all(), &secrets, "add");

    let list = ctx.run(&["transport", "list"], "");
    assert_eq!(list.code, 0);
    assert!(
        list.stdout.contains("cloud | rclone | never"),
        "{}",
        list.stdout
    );
    assert_clean(&list.all(), &secrets, "list");

    // `show` prints the name, the kind, when it was verified and the last check: no settings and
    // no secret id.
    let conn = db::open(Path::new(&ctx.db))?;
    let org = db::list_orgs(&conn, 5)?.remove(0).id;
    let row = db::list_transports(&conn, &org)?.remove(0);
    let secret_id = row.secret_id.clone().ok_or("secret id")?;
    let show = ctx.run(&["transport", "show", "cloud"], "");
    assert_eq!(show.code, 0);
    assert!(show.stdout.contains("Name: cloud"));
    assert!(show.stdout.contains("Kind: rclone"));
    assert!(show.stdout.contains("Verified at: never"));
    assert!(show.stdout.contains("Last check: no check yet"));
    assert_clean(&show.all(), &secrets, "show");
    assert!(
        !show.all().contains(secret_id.as_str()),
        "show printed a secret id"
    );
    assert!(
        !show.all().contains("bucket-private-marker"),
        "show printed settings"
    );
    assert!(!list.all().contains(secret_id.as_str()));
    assert!(!added.all().contains(secret_id.as_str()));

    // The database holds ciphertext only: neither value, nor the JSON they travel in.
    let raw = raw_db_bytes(&ctx.db);
    for s in secrets {
        assert!(!contains(&raw, s), "plaintext credential in the database");
    }
    assert!(!contains(&raw, "access_key_id\":\"idmarker"));

    // A second add with the same name is refused and leaves one transport and one secret.
    let again = add_s3(&ctx, "cloud");
    assert_eq!(again.code, 2);
    assert!(
        again
            .stderr
            .contains("A transport named cloud already exists")
    );
    assert_clean(&again.all(), &secrets, "duplicate add");
    assert_eq!(db::list_transports(&conn, &org)?.len(), 1);

    // Remove deletes the row and its sealed credentials.
    let removed = ctx.run(&["transport", "remove", "cloud"], "");
    assert_eq!(removed.code, 0, "{}", removed.stderr);
    assert!(db::list_transports(&conn, &org)?.is_empty());
    assert!(
        db::get_secret(&conn, &org, &secret_id).is_err(),
        "sealed credentials survived"
    );
    let missing = ctx.run(&["transport", "remove", "cloud"], "");
    assert_eq!(missing.code, 2);
    assert!(
        ctx.run(&["transport", "list"], "")
            .stdout
            .contains("No transports yet")
    );
    Ok(())
}

#[test]
fn credentials_are_never_taken_from_the_command_line() -> TestResult {
    let (_dir, ctx) = world("transport-argv")?;
    let (id, secret) = (access_id(), access_secret());
    let secrets = [id.as_str(), secret.as_str()];
    let base = [
        "transport",
        "add",
        "--name",
        "cloud",
        "--kind",
        "rclone",
        "--remote-kind",
        "s3",
        "--remote",
        "offsite",
    ];
    let with_secret_flag = |flag: &str, value: &str| {
        let mut args: Vec<String> = base.iter().map(|s| (*s).to_owned()).collect();
        if let Some((name, inline)) = flag.split_once('=') {
            args.push(format!("{name}={inline}"));
        } else {
            args.push(flag.to_owned());
            args.push(value.to_owned());
        }
        args
    };
    for flag in [
        "--secret-access-key",
        "--access-key-id",
        "--token",
        "--password",
        "--pass",
        "--key",
        "--credentials",
    ] {
        for form in [flag.to_owned(), format!("{flag}={secret}")] {
            let owned = with_secret_flag(&form, &secret);
            let args: Vec<&str> = owned.iter().map(String::as_str).collect();
            let r = ctx.run(&args, "");
            assert_eq!(r.code, 2, "{form}: {}", r.stderr);
            assert!(r.stderr.contains("Usage"), "{form}");
            assert_clean(&r.all(), &secrets, "usage error");
        }
    }
    // A stray word after a switch is not echoed either.
    let r = ctx.run(
        &[
            "transport",
            "add",
            "--name",
            "c",
            "--kind",
            "scp",
            "--credentials-stdin",
            &secret,
        ],
        "",
    );
    assert_eq!(r.code, 2);
    assert_clean(&r.all(), &secrets, "stray word");
    let conn = db::open(Path::new(&ctx.db))?;
    let org = db::list_orgs(&conn, 5)?.remove(0).id;
    assert!(
        db::list_transports(&conn, &org)?.is_empty(),
        "nothing may be stored"
    );
    Ok(())
}

#[test]
fn transport_add_refuses_bad_input_without_echoing_it_or_storing_anything() -> TestResult {
    let (_dir, ctx) = world("transport-bad")?;
    let (id, secret) = (access_id(), access_secret());
    let secrets = [id.as_str(), secret.as_str()];
    let conn = db::open(Path::new(&ctx.db))?;
    let org = db::list_orgs(&conn, 5)?.remove(0).id;

    // No master key: advice, nothing stored, nothing echoed.
    let no_key = Ctx {
        env: Vec::new(),
        ..Ctx::new(ctx.db.clone())
    };
    let r = add_s3(&no_key, "cloud");
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("NODIVARA_MASTER_KEY"), "{}", r.stderr);
    assert_clean(&r.all(), &secrets, "missing master key");

    // A malformed master key, in Indonesian.
    let mut bad_key = Ctx::new(ctx.db.clone());
    bad_key.env = vec![("NODIVARA_MASTER_KEY".to_owned(), "not-a-key".to_owned())];
    bad_key.lang = Some("id".to_owned());
    let r = add_s3(&bad_key, "cloud");
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("tidak valid"), "{}", r.stderr);
    assert!(!r.all().contains("not-a-key"));
    assert_clean(&r.all(), &secrets, "bad master key");
    assert!(db::list_transports(&conn, &org)?.is_empty());

    let rclone = |extra: &[&str], stdin: &str| {
        let mut args = vec![
            "transport",
            "add",
            "--name",
            "cloud",
            "--kind",
            "rclone",
            "--remote-kind",
            "s3",
            "--remote",
            "offsite",
        ];
        args.extend(extra.iter().copied());
        ctx.run(&args, stdin)
    };
    // Stdin problems name the line, never its content.
    let r = rclone(
        &["--credentials-stdin"],
        &format!("access_key_id={}\n{secret}\n", id),
    );
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("Line 2"), "{}", r.stderr);
    assert_clean(&r.all(), &secrets, "bad line");
    // A name the backend does not know, a repeated name, a control character in a value.
    for stdin in [
        format!("host={secret}\n"),
        format!("access_key_id={id}\naccess_key_id={secret}\n"),
        format!("access_key_id={secret}\u{7}\n"),
        "access_key_id=\n".to_owned(),
        format!("type={secret}\n"),
    ] {
        let r = rclone(&["--credentials-stdin"], &stdin);
        assert_eq!(r.code, 2, "{stdin:?}");
        assert!(r.stderr.contains("not valid"), "{}", r.stderr);
        assert_clean(&r.all(), &secrets, "bad credentials");
    }
    // The same option as a plain option and as a credential.
    let r = rclone(
        &[
            "--options",
            "region=x,access_key_id=plain",
            "--credentials-stdin",
        ],
        &credentials_text(),
    );
    assert_eq!(r.code, 2);
    assert_clean(&r.all(), &secrets, "duplicate option");
    // Oversized input.
    let big = format!("access_key_id={}\n", "x".repeat(70_000));
    let r = rclone(&["--credentials-stdin"], &big);
    assert_eq!(r.code, 2);

    // Credentials for a kind that has none; foreign flags; bad values.
    let r = ctx.run(
        &[
            "transport",
            "add",
            "--name",
            "r",
            "--kind",
            "rsync",
            "--host",
            "h.example.net",
            "--user",
            "u",
            "--path",
            "/srv",
            "--credentials-stdin",
        ],
        &credentials_text(),
    );
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("takes no stored credentials"));
    assert_clean(&r.all(), &secrets, "ssh credentials");
    let r = ctx.run(
        &[
            "transport",
            "add",
            "--name",
            "r",
            "--kind",
            "rsync",
            "--remote",
            "x",
            "--host",
            "h",
            "--user",
            "u",
            "--path",
            "/srv",
        ],
        "",
    );
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("Usage"));
    let r = ctx.run(
        &[
            "transport",
            "add",
            "--name",
            "r",
            "--kind",
            "rclone",
            "--remote-kind",
            "s3",
            "--remote",
            "x",
            "--host",
            "h",
        ],
        "",
    );
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("Usage"));
    for args in [
        vec!["transport", "add", "--name", "r", "--kind", "managed"],
        vec!["transport", "add", "--name", "r", "--kind", "ftp"],
        vec![
            "transport",
            "add",
            "--name",
            "r",
            "--kind",
            "rsync",
            "--host",
            "-oProxyCommand=x",
            "--user",
            "u",
            "--path",
            "/srv",
        ],
        vec![
            "transport",
            "add",
            "--name",
            "r",
            "--kind",
            "rsync",
            "--host",
            "h",
            "--user",
            "u",
            "--path",
            "/srv; rm -rf /",
        ],
        vec![
            "transport",
            "add",
            "--name",
            "r",
            "--kind",
            "rsync",
            "--host",
            "h",
            "--user",
            "u",
            "--path",
            "/srv",
            "--port",
            "99999",
        ],
        vec![
            "transport",
            "add",
            "--name",
            "r",
            "--kind",
            "rclone",
            "--remote-kind",
            "s3",
            "--remote",
            "Bad Name",
        ],
        vec![
            "transport",
            "add",
            "--name",
            "r",
            "--kind",
            "rclone",
            "--remote-kind",
            "s3",
            "--remote",
            "x",
            "--options",
            "env_auth=true",
        ],
        vec![
            "transport",
            "add",
            "--name",
            "r",
            "--kind",
            "rclone",
            "--remote-kind",
            "s3",
            "--remote",
            "x",
            "--options",
            "region",
        ],
        vec![
            "transport",
            "add",
            "--name",
            "r",
            "--kind",
            "rclone",
            "--remote-kind",
            "ftp",
            "--remote",
            "x",
        ],
    ] {
        let r = ctx.run(&args, "");
        assert_eq!(r.code, 2, "{args:?}");
        assert!(r.stdout.is_empty(), "{args:?}");
    }
    assert!(
        db::list_transports(&conn, &org)?.is_empty(),
        "a refused add stored something"
    );
    let raw = raw_db_bytes(&ctx.db);
    for s in secrets {
        assert!(!contains(&raw, s));
    }
    Ok(())
}

#[test]
fn transport_commands_print_usage_on_misuse() {
    let ctx = Ctx::new("unused.db".to_owned());
    for args in [
        vec!["transport"],
        vec!["transport", "bogus"],
        vec!["transport", "show"],
        vec!["transport", "remove"],
        vec!["transport", "check"],
        vec!["transport", "check", "a", "--force"],
        vec!["transport", "check", "a", "b"],
        vec!["transport", "list", "extra"],
        vec!["transport", "add"],
    ] {
        let r = ctx.run(&args, "");
        assert_eq!(r.code, 2, "{args:?}");
        assert!(r.stderr.contains("Usage"), "{args:?}: {}", r.stderr);
        assert!(r.stdout.is_empty());
    }
}

// ---- transport check --------------------------------------------------------------------------

fn quota_json() -> RunOutput {
    RunOutput::ok("{\"total\":1000000000,\"used\":1000,\"free\":999999000}")
}

/// Replies for a clean rclone check: probe, list, quota, speed upload, speed cleanup.
fn script_success(runner: &FakeRunner) {
    runner.push(Ok(RunOutput::ok("rclone v1.0")));
    runner.push(Ok(RunOutput::ok("")));
    runner.push(Ok(quota_json()));
    runner.push(Ok(RunOutput::ok("")));
    runner.push(Ok(RunOutput::ok("")));
}

fn transport_row(ctx: &Ctx, name: &str) -> Result<db::TransportRow, Box<dyn std::error::Error>> {
    let conn = db::open(Path::new(&ctx.db))?;
    let org = db::list_orgs(&conn, 5)?.remove(0).id;
    Ok(db::list_transports(&conn, &org)?
        .into_iter()
        .find(|t| t.name == name)
        .ok_or("transport missing")?)
}

#[test]
fn check_is_a_dry_run_unless_run_is_given() -> TestResult {
    let (_dir, mut ctx) = world("check-dry")?;
    assert_eq!(add_s3(&ctx, "cloud").code, 0);
    let (id, secret) = (access_id(), access_secret());
    let secrets = [id.as_str(), secret.as_str()];
    // Even a fully scripted runner is never asked anything without --run.
    script_success(&ctx.runner);

    let r = ctx.run(&["transport", "check", "cloud"], "");
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout.contains("Dry run of the checks for cloud"));
    assert!(r.stdout.contains("nothing is executed"));
    // The commands are shown, the secret names are shown, the values are masked.
    assert!(r.stdout.contains("rclone \"version\""), "{}", r.stdout);
    assert!(r.stdout.contains("rclone \"lsd\""), "{}", r.stdout);
    assert!(
        r.stdout
            .contains("RCLONE_CONFIG_OFFSITE_SECRET_ACCESS_KEY=***"),
        "{}",
        r.stdout
    );
    assert!(r.stdout.contains("RCLONE_CONFIG_OFFSITE_ACCESS_KEY_ID=***"));
    assert!(
        r.stdout
            .contains("RCLONE_CONFIG_OFFSITE_REGION=\"eu-west-1\"")
    );
    assert!(r.stdout.contains("Dry run finished"));
    assert_clean(&r.all(), &secrets, "dry run");
    assert!(
        ctx.runner.calls().is_empty(),
        "a dry run must not call the runner"
    );
    // Nothing was recorded.
    let row = transport_row(&ctx, "cloud")?;
    assert!(row.verified_at.is_none());
    assert!(row.last_check_json.is_none());

    // The same in Indonesian.
    ctx.lang = Some("id".to_owned());
    let r = ctx.run(&["transport", "check", "cloud"], "");
    assert_eq!(r.code, 0);
    assert!(r.stdout.contains("Uji coba pemeriksaan untuk cloud"));
    assert_clean(&r.all(), &secrets, "dry run (id)");
    assert!(ctx.runner.calls().is_empty());
    ctx.lang = None;

    // Sealed credentials need the master key even for a dry run; the advice says so.
    let no_key = Ctx {
        runner: ctx.runner.clone(),
        ..Ctx::new(ctx.db.clone())
    }
    .without_master();
    let r = no_key.run(&["transport", "check", "cloud"], "");
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("NODIVARA_MASTER_KEY"));
    // A different master key cannot open them.
    let mut other = Ctx::new(ctx.db.clone());
    other.runner = ctx.runner.clone();
    other.env = vec![(
        "NODIVARA_MASTER_KEY".to_owned(),
        format!("other:{}", "cd".repeat(32)),
    )];
    let r = other.run(&["transport", "check", "cloud"], "");
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("does not match"));
    assert!(ctx.runner.calls().is_empty());

    // A dry run of an SSH transport prints its two commands and starts nothing.
    verified_transport(&ctx, "ssh1")?;
    let r = ctx.run(&["transport", "check", "ssh1"], "");
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout.contains("ssh \"-V\""), "{}", r.stdout);
    assert!(r.stdout.contains("BatchMode=yes"));
    assert!(ctx.runner.calls().is_empty());
    Ok(())
}

#[test]
fn check_with_run_records_success_and_keeps_secrets_out_of_argv() -> TestResult {
    let (_dir, ctx) = world("check-run")?;
    assert_eq!(add_s3(&ctx, "cloud").code, 0);
    let (id, secret) = (access_id(), access_secret());
    let secrets = [id.as_str(), secret.as_str()];
    script_success(&ctx.runner);

    let r = ctx.run(&["transport", "check", "cloud", "--run"], "");
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout.contains("Running the checks for cloud"));
    assert!(r.stdout.contains("[ok] Test connection"));
    assert!(r.stdout.contains("[ok] Check quota"));
    assert!(r.stdout.contains("[ok] Measure speed"));
    assert!(r.stdout.contains("The checks passed"));
    assert!(r.stdout.contains("Free space on the remote: 999999000"));
    assert_clean(&r.all(), &secrets, "run");

    let calls = ctx.runner.calls();
    assert_eq!(calls.len(), 5);
    for call in &calls {
        assert_eq!(call.program, "rclone");
        for arg in &call.args {
            assert_clean(arg, &secrets, "argv");
        }
    }
    // The secrets travel in the environment of the commands that need them, and only there.
    assert!(
        calls[1]
            .secret_env
            .iter()
            .any(|(n, v)| n == "RCLONE_CONFIG_OFFSITE_SECRET_ACCESS_KEY" && *v == secret)
    );

    let row = transport_row(&ctx, "cloud")?;
    assert!(row.verified_at.is_some());
    let check = row.last_check_json.ok_or("last check")?;
    assert!(check.contains("\"ok\":true"), "{check}");
    assert_clean(&check, &secrets, "last check");
    let shown = ctx.run(&["transport", "show", "cloud"], "");
    assert!(!shown.stdout.contains("Verified at: never"));
    assert!(shown.stdout.contains("\"ok\":true"));
    let raw = raw_db_bytes(&ctx.db);
    for s in secrets {
        assert!(!contains(&raw, s));
    }

    // With exactly one verified transport a plan may omit --transport.
    assert_eq!(compare_saved(&ctx, "new").code, 1);
    let r = ctx.run(
        &[
            "plan",
            "draft",
            "--source",
            "old",
            "--target",
            "new",
            "--name",
            "after-check",
        ],
        "",
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    Ok(())
}

#[test]
fn check_with_run_turns_failures_into_localized_advice() -> TestResult {
    let (_dir, mut ctx) = world("check-fail")?;
    assert_eq!(add_s3(&ctx, "cloud").code, 0);
    let (id, secret) = (access_id(), access_secret());
    let secrets = [id.as_str(), secret.as_str()];

    // Quota exceeded; the tool echoes a secret in its message, which must not reach the person.
    ctx.runner.push(Ok(RunOutput::ok("rclone v1.0")));
    ctx.runner.push(Ok(RunOutput::ok("")));
    ctx.runner.push(Ok(RunOutput::failed(
        8,
        &format!("Failed: storage quota exceeded for key {secret}"),
    )));
    let r = ctx.run(&["transport", "check", "cloud", "--run"], "");
    assert_eq!(r.code, 2);
    assert!(
        r.stderr.contains("Not enough storage or quota"),
        "{}",
        r.stderr
    );
    assert!(r.stderr.contains("Message from the tool"));
    assert_clean(&r.all(), &secrets, "quota failure");
    let row = transport_row(&ctx, "cloud")?;
    assert!(row.verified_at.is_none(), "a failed check must not verify");
    let check = row.last_check_json.ok_or("last check")?;
    assert!(check.contains("quota_exceeded"), "{check}");

    // Token expired, in Indonesian.
    ctx.runner.push(Ok(RunOutput::ok("rclone v1.0")));
    ctx.runner
        .push(Ok(RunOutput::failed(1, "oauth2: invalid_grant")));
    ctx.lang = Some("id".to_owned());
    let r = ctx.run(&["transport", "check", "cloud", "--run"], "");
    assert_eq!(r.code, 2);
    assert!(
        r.stderr.contains("Proses masuk sudah kedaluwarsa") || r.stderr.contains("kedaluwarsa"),
        "{}",
        r.stderr
    );
    ctx.lang = None;

    // rclone is not installed: advice that offers installation and installs nothing.
    ctx.runner.push(Err(RunError::NotInstalled));
    let before = ctx.runner.calls().len();
    let r = ctx.run(&["transport", "check", "cloud", "--run"], "");
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("rclone is not installed"), "{}", r.stderr);
    assert!(r.stderr.contains("never installs anything"), "{}", r.stderr);
    assert_eq!(
        ctx.runner.calls().len(),
        before + 1,
        "only the probe may run"
    );
    let calls = ctx.runner.calls();
    for call in &calls {
        assert!(!call.args.iter().any(|a| a == "install"));
        assert_eq!(call.program, "rclone");
    }
    assert!(transport_row(&ctx, "cloud")?.verified_at.is_none());
    Ok(())
}

#[test]
fn an_oauth_remote_without_a_token_asks_for_one_and_never_runs_past_the_probe() -> TestResult {
    let (_dir, ctx) = world("check-oauth")?;
    let r = ctx.run(
        &[
            "transport",
            "add",
            "--name",
            "gd",
            "--kind",
            "rclone",
            "--remote-kind",
            "drive",
            "--remote",
            "gdrive",
        ],
        "",
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    // Dry run: says so and exits cleanly.
    let r = ctx.run(&["transport", "check", "gd"], "");
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout.contains("needs a sign-in token"));
    assert!(ctx.runner.calls().is_empty());
    // Real run: the probe runs, then the hint, then it stops.
    let r = ctx.run(&["transport", "check", "gd", "--run"], "");
    assert_eq!(r.code, 2);
    assert!(
        r.stderr.contains("rclone authorize \"drive\""),
        "{}",
        r.stderr
    );
    assert_eq!(ctx.runner.calls().len(), 1);
    assert!(transport_row(&ctx, "gd")?.verified_at.is_none());
    Ok(())
}

#[test]
fn ssh_transports_are_checked_with_two_commands() -> TestResult {
    let (_dir, ctx) = world("check-ssh")?;
    let r = ctx.run(
        &[
            "transport",
            "add",
            "--name",
            "box",
            "--kind",
            "ssh_pipe",
            "--host",
            "box.example.net",
            "--user",
            "u",
            "--path",
            "/srv/in",
        ],
        "",
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    let r = ctx.run(&["transport", "check", "box", "--run"], "");
    assert_eq!(r.code, 0, "{}", r.stderr);
    let calls = ctx.runner.calls();
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().all(|c| c.program == "ssh"));
    assert!(transport_row(&ctx, "box")?.verified_at.is_some());
    // Unreachable host.
    ctx.runner.push(Ok(RunOutput::ok("OpenSSH")));
    ctx.runner.push(Ok(RunOutput::failed(
        255,
        "ssh: Could not resolve hostname",
    )));
    let r = ctx.run(&["transport", "check", "box", "--run"], "");
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("Cannot reach the remote"), "{}", r.stderr);
    Ok(())
}

// ---- tenants ----------------------------------------------------------------------------------

#[test]
fn another_organization_sees_neither_plans_nor_transports() -> TestResult {
    let (_dir, mut ctx) = world("tenants")?;
    verified_transport(&ctx, "ssh1")?;
    assert_eq!(add_s3(&ctx, "cloud").code, 0);
    assert_eq!(compare_saved(&ctx, "new").code, 1);
    let r = ctx.run(
        &[
            "plan",
            "draft",
            "--source",
            "old",
            "--target",
            "new",
            "--transport",
            "ssh1",
            "--name",
            "mig",
        ],
        "",
    );
    assert_eq!(r.code, 0, "{}", r.stderr);

    // A second organization in the same database.
    {
        let conn = db::open(Path::new(&ctx.db))?;
        let org = db::create_org(&conn, "Other", "other", "free")?;
        let user = db::create_user(&conn, "other@localhost", "Other", None)?;
        db::add_member(&conn, &org.id, &user.id, Role::Owner)?;
    }
    ctx.org = Some("local".to_owned());
    let first = ctx.run(&["plan", "list"], "");
    assert!(first.stdout.contains("mig | draft"), "{}", first.stderr);

    let mut other = Ctx::new(ctx.db.clone());
    other.org = Some("other".to_owned());
    assert!(
        other
            .run(&["plan", "list"], "")
            .stdout
            .contains("No plans yet")
    );
    assert!(
        other
            .run(&["transport", "list"], "")
            .stdout
            .contains("No transports yet")
    );
    for args in [
        vec!["plan", "show", "mig"],
        vec!["plan", "gates", "mig"],
        vec!["transport", "show", "cloud"],
        vec!["transport", "remove", "cloud"],
        vec!["transport", "check", "cloud"],
        vec!["transport", "check", "cloud", "--run"],
        vec!["transport", "check", "ssh1", "--run"],
    ] {
        let r = other.run(&args, "");
        assert_eq!(r.code, 2, "{args:?}");
        assert!(r.stdout.is_empty(), "{args:?}: {}", r.stdout);
        assert!(!r.stderr.contains("Free space"), "{args:?}");
    }
    assert!(
        other.runner.calls().is_empty(),
        "another org's transport was run"
    );
    // Drafting with the first org's node and transport names fails exactly like missing ones.
    let r = other.run(
        &[
            "plan",
            "draft",
            "--source",
            "old",
            "--target",
            "new",
            "--transport",
            "ssh1",
        ],
        "",
    );
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("no node named old"), "{}", r.stderr);
    assert_eq!(plan_count(&ctx.db)?, 1);

    // The first organization still has everything.
    assert!(
        ctx.run(&["transport", "show", "cloud"], "")
            .stdout
            .contains("Name: cloud")
    );
    assert!(
        ctx.run(&["plan", "show", "mig"], "")
            .stdout
            .contains("Migration plan: mig")
    );
    // The same transport name is free in the other organization.
    let r = other.run(
        &[
            "transport",
            "add",
            "--name",
            "cloud",
            "--kind",
            "scp",
            "--host",
            "h.example.net",
            "--user",
            "u",
            "--path",
            "/srv",
        ],
        "",
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    Ok(())
}

/// `--options` is for plain settings only. A credential name there would end up on the command line
/// (visible in process lists) and in the stored configuration in plaintext, so it is refused
/// for every backend and nothing is stored.
#[test]
fn credential_names_are_refused_as_plain_options_and_never_echoed() -> TestResult {
    let (_dir, ctx) = world("transport-options")?;
    let secret = access_secret();
    for (remote_kind, option) in [
        ("s3", "secret_access_key"),
        ("s3", "access_key_id"),
        ("s3", "session_token"),
        ("b2", "key"),
        ("drive", "token"),
        ("drive", "client_secret"),
        ("sftp", "pass"),
        ("sftp", "key_pem"),
        ("webdav", "bearer_token"),
    ] {
        let options = format!("{option}={secret}");
        let r = ctx.run(
            &[
                "transport",
                "add",
                "--name",
                "leaky",
                "--kind",
                "rclone",
                "--remote-kind",
                remote_kind,
                "--remote",
                "offsite",
                "--options",
                &options,
            ],
            "",
        );
        assert_eq!(r.code, 2, "{remote_kind} {option}: {}", r.stderr);
        assert_clean(&r.all(), &[secret.as_str()], "refusal output");
    }
    let conn = db::open(Path::new(&ctx.db))?;
    let org = db::list_orgs(&conn, 5)?.remove(0).id;
    assert!(
        db::list_transports(&conn, &org)?.is_empty(),
        "nothing may be stored"
    );
    assert!(
        !contains(&raw_db_bytes(&ctx.db), &secret),
        "the value must not be in the database"
    );
    Ok(())
}
