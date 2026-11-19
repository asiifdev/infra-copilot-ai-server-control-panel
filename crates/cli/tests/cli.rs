//! End to end: collectors fixture tree -> inventory JSON -> parity report, through `cli::run`.

use inventory::{Inventory, Kind};
use std::path::PathBuf;

type TestResult = Result<(), Box<dyn std::error::Error>>;

struct Outcome {
    code: u8,
    stdout: String,
    stderr: String,
}

fn run(args: &[&str], lang: Option<&str>) -> Outcome {
    let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = cli::run(&args, lang, &mut out, &mut err);
    Outcome {
        code,
        stdout: String::from_utf8_lossy(&out).into_owned(),
        stderr: String::from_utf8_lossy(&err).into_owned(),
    }
}

fn manifest(rel: &str) -> String {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push(rel);
    path.to_string_lossy().into_owned()
}

fn parity_fixture(name: &str) -> String {
    manifest(&format!("../parity/tests/fixtures/{name}"))
}

#[test]
fn compare_with_gaps_reports_findings_and_exits_1() {
    let r = run(
        &[
            "compare",
            &parity_fixture("source.json"),
            &parity_fixture("target-gaps.json"),
        ],
        None,
    );
    assert_eq!(r.code, 1, "stderr: {}", r.stderr);
    for expected in [
        "PAR-001",
        "PAR-008",
        "PAR-010",
        "[Critical]",
        "Advice",
        "tcp:7700",
    ] {
        assert!(
            r.stdout.contains(expected),
            "missing {expected} in:\n{}",
            r.stdout
        );
    }
    // The report must say what it does NOT prove.
    assert!(r.stdout.contains("does not prove"));
    assert!(
        !r.stdout.contains('{'),
        "unresolved placeholder in:\n{}",
        r.stdout
    );
}

#[test]
fn compare_without_critical_findings_exits_0() {
    let r = run(
        &[
            "compare",
            &parity_fixture("source.json"),
            &parity_fixture("target-ok.json"),
        ],
        None,
    );
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    assert!(!r.stdout.contains("[Critical]"));
}

#[test]
fn partial_inventories_are_called_out() {
    let r = run(
        &[
            "compare",
            &parity_fixture("source.json"),
            &parity_fixture("target-partial.json"),
        ],
        None,
    );
    assert_eq!(
        r.code, 0,
        "a partial target must not produce critical findings"
    );
    assert!(r.stdout.contains("partial"));
}

#[test]
fn indonesian_output_uses_the_catalog() {
    let en = run(
        &[
            "compare",
            &parity_fixture("source.json"),
            &parity_fixture("target-gaps.json"),
        ],
        None,
    );
    let id = run(
        &[
            "compare",
            &parity_fixture("source.json"),
            &parity_fixture("target-gaps.json"),
            "--lang",
            "id",
        ],
        None,
    );
    assert_eq!(id.code, 1);
    assert!(id.stdout.contains("Saran"));
    assert!(id.stdout.contains("tidak membuktikan"));
    assert_ne!(en.stdout, id.stdout);
    // LANG from the environment selects the locale when --lang is absent.
    let env = run(
        &[
            "compare",
            &parity_fixture("source.json"),
            &parity_fixture("target-gaps.json"),
        ],
        Some("id_ID.UTF-8"),
    );
    assert_eq!(env.stdout, id.stdout);
}

#[test]
fn collect_reads_a_fixture_root_and_round_trips() -> TestResult {
    let root = manifest("../collectors/tests/fixtures/full");
    let r = run(
        &[
            "collect", "--root", &root, "--os", "linux", "--arch", "x86_64",
        ],
        None,
    );
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    let inventory = Inventory::from_json(&r.stdout)?;
    assert_eq!(inventory.platform.os, "linux");
    assert!(inventory.get(Kind::Listener, "tcp:5432").is_some());
    assert!(inventory.items_of(Kind::Cron).count() > 0);
    // Nothing that looks like a secret survives into the output.
    for secret in ["hunter2", "postgres://u:pw@"] {
        assert!(!r.stdout.contains(secret), "{secret} leaked");
    }
    Ok(())
}

#[test]
fn collect_then_compare_against_itself_end_to_end() -> TestResult {
    let root = manifest("../collectors/tests/fixtures/full");
    let r = run(
        &[
            "collect", "--root", &root, "--os", "linux", "--arch", "x86_64",
        ],
        None,
    );
    assert_eq!(r.code, 0);
    let mut file = std::env::temp_dir();
    file.push(format!("infra-cli-e2e-{}.json", std::process::id()));
    std::fs::write(&file, &r.stdout)?;
    let path = file.to_string_lossy().into_owned();
    let report = run(&["compare", &path, &path], None);
    std::fs::remove_file(&file)?;
    // Same machine on both sides: no missing listeners or cron jobs, only exposure notices.
    assert_eq!(report.code, 0, "stderr: {}", report.stderr);
    assert!(!report.stdout.contains("] PAR-001:"));
    assert!(!report.stdout.contains("] PAR-008:"));
    Ok(())
}

#[test]
fn usage_and_io_errors_exit_2_with_localized_messages() {
    for args in [
        vec![],
        vec!["bogus"],
        vec!["compare", "only-one.json"],
        vec!["collect", "--nope", "x"],
        vec!["collect", "--root"],
    ] {
        let r = run(&args, None);
        assert_eq!(r.code, 2, "args {args:?}");
        assert!(r.stderr.contains("Usage"), "{}", r.stderr);
        assert!(r.stdout.is_empty());
    }
    let missing = run(
        &["compare", "/nonexistent/a.json", "/nonexistent/b.json"],
        None,
    );
    assert_eq!(missing.code, 2);
    assert!(missing.stderr.contains("Cannot read"));
    let id = run(
        &[
            "compare",
            "/nonexistent/a.json",
            "/nonexistent/b.json",
            "--lang",
            "id",
        ],
        None,
    );
    assert!(id.stderr.contains("Tidak bisa membaca"));
    let bad = run(
        &["compare", &manifest("Cargo.toml"), &manifest("Cargo.toml")],
        None,
    );
    assert_eq!(bad.code, 2);
    assert!(bad.stderr.contains("not a valid inventory"));
    let wrong_lang = run(&["compare", "a", "b", "--lang", "xx"], None);
    assert_eq!(wrong_lang.code, 2);
}

#[test]
fn limits_note_lists_the_rules_that_ran() {
    let args = [
        "compare",
        &parity_fixture("source.json"),
        &parity_fixture("target-ok.json"),
    ];
    let en = run(&args, None);
    assert!(
        en.stdout.contains("PAR-001, PAR-006, PAR-008, PAR-010"),
        "{}",
        en.stdout
    );
    assert!(!en.stdout.contains("listed above"), "{}", en.stdout);
    let id = run(&args, Some("id"));
    assert!(
        id.stdout.contains("PAR-001, PAR-006, PAR-008, PAR-010"),
        "{}",
        id.stdout
    );
    assert!(!id.stdout.contains("di atas"), "{}", id.stdout);
    assert!(!id.stdout.contains('{'), "{}", id.stdout);
}

#[test]
fn collect_usage_mentions_lang() {
    let r = run(&["collect", "--nope", "x"], None);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("--lang"), "{}", r.stderr);
}

#[test]
fn repeated_options_are_a_usage_error() {
    for args in [
        vec!["collect", "--root", "/a", "--root", "/b"],
        vec!["collect", "--os", "linux", "--os", "linux"],
        vec!["compare", "a", "b", "--lang", "en", "--lang", "id"],
    ] {
        let r = run(&args, None);
        assert_eq!(r.code, 2, "args {args:?}");
        assert!(r.stderr.contains("Usage"), "{}", r.stderr);
        assert!(r.stdout.is_empty());
    }
}

#[test]
fn non_regular_files_are_rejected_without_reading() {
    // /dev/zero reports length 0 but never ends; a directory is not an inventory either.
    for path in ["/dev/zero", "/dev/null", env!("CARGO_MANIFEST_DIR")] {
        let r = run(&["compare", path, path], None);
        assert_eq!(r.code, 2, "{path}: {}", r.stderr);
        assert!(r.stderr.contains("not a regular file"), "{}", r.stderr);
    }
    let id = run(&["compare", "/dev/zero", "/dev/zero", "--lang", "id"], None);
    assert!(id.stderr.contains("bukan berkas biasa"), "{}", id.stderr);
}

#[test]
fn report_layout_comes_from_the_catalog() {
    let args = [
        "compare",
        &parity_fixture("source.json"),
        &parity_fixture("target-gaps.json"),
    ];
    let en = run(&args, None);
    assert!(en.stdout.contains("[Critical] PAR-001: "), "{}", en.stdout);
    assert!(en.stdout.contains("(listener tcp:7700)"), "{}", en.stdout);
    assert!(en.stdout.contains("(cron "), "{}", en.stdout);
    let id = run(&args, Some("id"));
    assert!(id.stdout.contains("[Kritis] PAR-001: "), "{}", id.stdout);
    assert!(id.stdout.contains("(pendengar tcp:7700)"), "{}", id.stdout);
    assert!(id.stdout.contains("Saran: "), "{}", id.stdout);
    assert!(!id.stdout.contains("(listener"), "{}", id.stdout);
}

#[cfg(unix)]
#[test]
fn odd_arguments_never_panic() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    let raw = vec![
        OsString::from("compare"),
        OsString::from_vec(vec![b'a', 0xff, b'b']),
        OsString::from("--lang"),
    ];
    let args = cli::args_from_os(raw);
    assert_eq!(args[1], "a\u{fffd}b");
    let r = run(&args.iter().map(String::as_str).collect::<Vec<_>>(), None);
    assert_eq!(r.code, 2);
}

#[test]
fn hostile_inventories_cannot_inject_terminal_escapes_or_fake_lines() -> TestResult {
    // A source inventory is untrusted input: names and warnings may carry escape sequences and
    // newlines that would repaint the terminal or forge report lines.
    let evil_name = "sea\\u001b[31mrchd\\n[Critical] PAR-999: forged";
    let source =
        std::fs::read_to_string(parity_fixture("source.json"))?.replace("searchd", evil_name);
    let target = std::fs::read_to_string(parity_fixture("target-partial.json"))?.replace(
        "\"warnings\": []",
        "\"warnings\": [\"bad\\u001b]0;pwned\\u0007\"]",
    );
    let dir = std::env::temp_dir();
    let (s_path, t_path) = (
        dir.join(format!("infra-evil-s-{}.json", std::process::id())),
        dir.join(format!("infra-evil-t-{}.json", std::process::id())),
    );
    std::fs::write(&s_path, source)?;
    std::fs::write(&t_path, target)?;
    let r = run(
        &[
            "compare",
            &s_path.to_string_lossy(),
            &t_path.to_string_lossy(),
        ],
        None,
    );
    std::fs::remove_file(&s_path)?;
    std::fs::remove_file(&t_path)?;
    assert!(r.code == 0 || r.code == 1, "stderr: {}", r.stderr);
    assert!(
        !r.stdout.contains('\u{1b}'),
        "escape sequence reached the terminal:\n{:?}",
        r.stdout
    );
    assert!(!r.stdout.contains('\u{7}'));
    assert!(
        !r.stdout.contains("\n[Critical] PAR-999"),
        "forged line:\n{}",
        r.stdout
    );
    Ok(())
}
