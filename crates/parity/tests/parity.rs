//! Rule, comparer and renderer tests on anonymous fixtures (parity-rules skill).
//!
//! Fixtures (`tests/fixtures/`, fictional, shared with the CLI):
//! * `source.json`: public nginx on 80; postgres 5432; redis 6379; `searchd` on 7700; cron
//!   `backup-db` and `queue-run`.
//! * `target-gaps.json`: against the source it triggers six findings: PAR-001 Critical (7700, no
//!   `searchd` at all), PAR-001 Important (6379, redis moved to 6380), PAR-008 Important
//!   (`backup-db`), PAR-010 Important on the target (80 and 8081) and PAR-010 Info on the source (80).
//! * `target-ok.json`: full equivalent ("no findings" fixture). Port 80 is `lan` there, so the only
//!   finding is the PAR-010 Info for port 80 on the source side; PAR-001 and PAR-008 are silent.
//! * `target-partial.json`: the items of `target-gaps.json` with `status: partial`. Absence rules
//!   (PAR-001, PAR-008) drop to Info there; PAR-010 on a visible public listener stays Important.

use i18n::{Catalog, Locale};
use inventory::{Finding, Inventory, ParityRule, RuleContext, Severity};
use parity::{
    Par001, Par002, Par006, Par007, Par008, Par009, Par010, UNKNOWN_ARG, all_rules, compare, render,
};
use std::error::Error;

type TestResult = Result<(), Box<dyn Error>>;

fn load(json: &str) -> Result<Inventory, Box<dyn Error>> {
    Ok(Inventory::from_json(json)?)
}
fn source() -> Result<Inventory, Box<dyn Error>> {
    load(include_str!("fixtures/source.json"))
}
fn gaps() -> Result<Inventory, Box<dyn Error>> {
    load(include_str!("fixtures/target-gaps.json"))
}
fn ok() -> Result<Inventory, Box<dyn Error>> {
    load(include_str!("fixtures/target-ok.json"))
}
fn partial() -> Result<Inventory, Box<dyn Error>> {
    load(include_str!("fixtures/target-partial.json"))
}

fn run(rule: &dyn ParityRule, s: &Inventory, t: &Inventory) -> Vec<Finding> {
    rule.evaluate(s, t, &RuleContext::default())
}
fn keys(findings: &[Finding]) -> Vec<(&str, Severity, &str)> {
    findings
        .iter()
        .map(|f| (f.rule_id.as_str(), f.severity, f.subject.key.as_str()))
        .collect()
}
fn arg<'a>(f: &'a Finding, name: &str) -> Option<&'a str> {
    f.args.get(name).map(String::as_str)
}

#[test]
fn par001_triggers_critical_and_important() -> TestResult {
    let found = run(&Par001, &source()?, &gaps()?);
    assert_eq!(
        keys(&found),
        [
            ("PAR-001", Severity::Important, "tcp:6379"),
            ("PAR-001", Severity::Critical, "tcp:7700"),
        ]
    );
    let moved = &found[0];
    assert_eq!(arg(moved, "process"), Some("redis-server"));
    assert_eq!(arg(moved, "port"), Some("6379"));
    assert_eq!(arg(moved, "bind"), Some("127.0.0.1"));
    // Evidence of a moved port also points at the target listener.
    let evidence: Vec<_> = moved.evidence.iter().map(|e| e.key.as_str()).collect();
    assert_eq!(evidence, ["tcp:6379", "tcp:6380"]);
    assert_eq!(found[1].evidence.len(), 1);
    Ok(())
}

#[test]
fn par001_is_silent_when_equivalent() -> TestResult {
    assert!(run(&Par001, &source()?, &ok()?).is_empty());
    let s = source()?;
    assert!(run(&Par001, &s, &s).is_empty());
    Ok(())
}

fn listener_inventory(items: &str) -> Result<Inventory, Box<dyn Error>> {
    load(&format!(
        r#"{{"schema_version":1,"taken_at":"t","collector_version":"c",
        "platform":{{"os":"linux","arch":"x86_64"}},"status":"complete","warnings":[],
        "items":[{items}]}}"#
    ))
}

#[test]
fn par001_unknown_process_is_critical_and_named_unknown() -> TestResult {
    let s = listener_inventory(
        r#"{"kind":"listener","key":"tcp:9","data":{"port":9,"bind":"127.0.0.1","process":null,"exposed":"loopback"},"seen_via":"proc"}"#,
    )?;
    let t = listener_inventory(
        r#"{"kind":"listener","key":"tcp:10","data":{"port":10,"bind":"127.0.0.1","process":null,"exposed":"loopback"},"seen_via":"proc"}"#,
    )?;
    let found = run(&Par001, &s, &t);
    assert_eq!(keys(&found), [("PAR-001", Severity::Critical, "tcp:9")]);
    // A sentinel, not display text: the renderer maps it to a catalog word.
    assert_eq!(arg(&found[0], "process"), Some(UNKNOWN_ARG));
    assert_eq!(arg(&found[0], "bind"), Some("127.0.0.1"));
    let catalog = Catalog::builtin()?;
    let id = render(&found[0], &catalog, Locale::Id);
    assert!(
        id.explanation.contains("(tidak diketahui)"),
        "{}",
        id.explanation
    );
    assert!(!id.explanation.contains("unknown"), "{}", id.explanation);
    let en = render(&found[0], &catalog, Locale::En);
    assert!(en.explanation.contains("(unknown)"), "{}", en.explanation);
    Ok(())
}

fn node_listener(port: u16, name: &str) -> String {
    format!(
        r#"{{"kind":"listener","key":"tcp:{port}","data":{{"port":{port},"bind":"127.0.0.1","process":"{name}","exposed":"loopback"}},"seen_via":"proc"}}"#
    )
}

#[test]
fn par001_port_that_already_has_a_counterpart_is_not_a_move_target() -> TestResult {
    // Source node on 3000 and 3001; target node only on 3001 (the counterpart of 3001).
    // 3000 has no counterpart at all, so it is Critical, not "moved".
    let s = listener_inventory(&format!(
        "{},{}",
        node_listener(3000, "node"),
        node_listener(3001, "node")
    ))?;
    let t = listener_inventory(&node_listener(3001, "node"))?;
    let found = run(&Par001, &s, &t);
    assert_eq!(keys(&found), [("PAR-001", Severity::Critical, "tcp:3000")]);
    assert_eq!(found[0].evidence.len(), 1);
    Ok(())
}

#[test]
fn par001_each_target_listener_is_used_once_in_key_order() -> TestResult {
    // Two source ports are missing, one spare target listener (3002): the first key gets it.
    let s = listener_inventory(&format!(
        "{},{}",
        node_listener(3000, "node"),
        node_listener(3001, "node")
    ))?;
    let t = listener_inventory(&node_listener(3002, "node"))?;
    let found = run(&Par001, &s, &t);
    assert_eq!(
        keys(&found),
        [
            ("PAR-001", Severity::Important, "tcp:3000"),
            ("PAR-001", Severity::Critical, "tcp:3001"),
        ]
    );
    let evidence: Vec<_> = found[0].evidence.iter().map(|e| e.key.as_str()).collect();
    assert_eq!(evidence, ["tcp:3000", "tcp:3002"]);
    Ok(())
}

#[test]
fn par008_triggers_and_is_silent() -> TestResult {
    let found = run(&Par008, &source()?, &gaps()?);
    assert_eq!(
        keys(&found),
        [(
            "PAR-008",
            Severity::Important,
            "user:root:0 3 * * *:backup-db"
        )]
    );
    assert_eq!(arg(&found[0], "schedule"), Some("0 3 * * *"));
    assert_eq!(arg(&found[0], "command"), Some("backup-db"));
    assert_eq!(arg(&found[0], "owner"), Some("root"));
    assert!(run(&Par008, &source()?, &ok()?).is_empty());
    Ok(())
}

#[test]
fn par010_target_important_source_info() -> TestResult {
    let found = run(&Par010, &source()?, &gaps()?);
    let seen: Vec<_> = found
        .iter()
        .map(|f| (f.severity, f.subject.key.as_str(), arg(f, "side")))
        .collect();
    assert_eq!(
        seen,
        [
            (Severity::Info, "tcp:80", Some("source")),
            (Severity::Important, "tcp:80", Some("target")),
            (Severity::Important, "tcp:8081", Some("target")),
        ]
    );
    Ok(())
}

#[test]
fn par010_does_not_trigger_without_public_target_listeners() -> TestResult {
    // Target side is silent for the equivalent target; only the source's public port 80 shows (Info).
    let found = run(&Par010, &source()?, &ok()?);
    assert_eq!(keys(&found), [("PAR-010", Severity::Info, "tcp:80")]);
    assert_eq!(arg(&found[0], "side"), Some("source"));
    Ok(())
}

#[test]
fn partial_target_caps_absence_rules_but_not_visible_exposure() -> TestResult {
    let report = compare(&source()?, &partial()?, &RuleContext::default());
    assert!(!report.findings.is_empty());
    for f in &report.findings {
        if f.rule_id == "PAR-010" {
            // A public listener that was SEEN is evidence of existence, partial or not.
            assert_eq!(arg(f, "partial"), None, "{f:?}");
            let expected = if arg(f, "side") == Some("source") {
                Severity::Info
            } else {
                Severity::Important
            };
            assert_eq!(f.severity, expected, "{f:?}");
        } else {
            // "No counterpart" rules cannot claim absence from an incomplete target.
            assert_eq!(f.severity, Severity::Info, "{f:?}");
            assert_eq!(arg(f, "partial"), Some("true"), "{f:?}");
        }
    }
    // Same finding set as with the complete target: partial only lowers absence severities.
    let full = compare(&source()?, &gaps()?, &RuleContext::default());
    assert_eq!(report.findings.len(), full.findings.len());
    Ok(())
}

#[test]
fn partial_source_does_not_flag_par010() -> TestResult {
    let s = load(&include_str!("fixtures/source.json").replace("\"complete\"", "\"partial\""))?;
    let found = run(&Par010, &s, &ok()?);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].severity, Severity::Info);
    assert_eq!(arg(&found[0], "partial"), None);
    Ok(())
}

#[test]
fn report_order_and_rules_are_deterministic() -> TestResult {
    let ctx = RuleContext::default();
    let report = compare(&source()?, &gaps()?, &ctx);
    assert_eq!(report, compare(&source()?, &gaps()?, &ctx));
    assert_eq!(
        keys(&report.findings),
        [
            ("PAR-001", Severity::Critical, "tcp:7700"),
            ("PAR-001", Severity::Important, "tcp:6379"),
            (
                "PAR-008",
                Severity::Important,
                "user:root:0 3 * * *:backup-db"
            ),
            ("PAR-010", Severity::Important, "tcp:80"),
            ("PAR-010", Severity::Important, "tcp:8081"),
            ("PAR-010", Severity::Info, "tcp:80"),
        ]
    );
    assert_eq!(
        report.rules,
        [
            ("PAR-001", 1),
            ("PAR-002", 1),
            ("PAR-006", 1),
            ("PAR-007", 1),
            ("PAR-008", 1),
            ("PAR-009", 1),
            ("PAR-010", 1)
        ]
    );
    Ok(())
}

#[test]
fn equivalent_target_has_only_the_source_side_info() -> TestResult {
    let report = compare(&source()?, &ok()?, &RuleContext::default());
    assert_eq!(
        keys(&report.findings),
        [("PAR-010", Severity::Info, "tcp:80")]
    );
    Ok(())
}

#[test]
fn comparing_an_inventory_with_itself_only_reports_public_ports() -> TestResult {
    // Nothing is missing, so PAR-001/PAR-008 are silent. PAR-010 still reports every public
    // listener, once as source (Info) and once as target (Important): port 80 here.
    let s = source()?;
    let report = compare(&s, &s, &RuleContext::default());
    assert!(report.findings.iter().all(|f| f.rule_id == "PAR-010"));
    let sides: Vec<_> = report.findings.iter().map(|f| arg(f, "side")).collect();
    assert_eq!(sides, [Some("target"), Some("source")]);
    // An inventory with no public listener is fully silent against itself.
    let o = ok()?;
    let quiet = compare(&o, &o, &RuleContext::default());
    assert!(quiet.findings.is_empty());
    Ok(())
}

#[test]
fn rule_metadata_matches_the_documents() {
    let rules = all_rules();
    let meta: Vec<_> = rules
        .iter()
        .map(|r| {
            (
                r.id(),
                r.version(),
                r.severity(),
                r.category(),
                r.fix_tier(),
            )
        })
        .collect();
    assert_eq!(
        meta,
        [
            ("PAR-001", 1, Severity::Critical, "coverage", None),
            ("PAR-002", 1, Severity::Important, "configuration", None),
            ("PAR-006", 1, Severity::Important, "runtime", None),
            ("PAR-007", 1, Severity::Important, "configuration", None),
            ("PAR-008", 1, Severity::Important, "scheduling", None),
            ("PAR-009", 1, Severity::Important, "versions", None),
            ("PAR-010", 1, Severity::Important, "security", None),
        ]
    );
}

#[test]
fn render_en_and_id_complete_for_every_finding() -> TestResult {
    let catalog = Catalog::builtin()?;
    let reports = [
        compare(&source()?, &gaps()?, &RuleContext::default()),
        compare(&source()?, &partial()?, &RuleContext::default()),
    ];
    for report in &reports {
        for f in &report.findings {
            for locale in Locale::ALL {
                let r = render(f, &catalog, locale);
                for text in [&r.severity_label, &r.title, &r.explanation, &r.advice] {
                    assert!(!text.is_empty());
                    assert!(!text.contains('{') && !text.contains('}'), "{text}");
                    assert!(!text.starts_with("parity."), "missing key: {text}");
                }
                assert_eq!(r.note.is_some(), arg(f, "partial") == Some("true"));
                if let Some(note) = &r.note {
                    assert!(!note.starts_with("parity."));
                }
            }
        }
    }
    Ok(())
}

#[test]
fn render_content_is_localized_and_filled() -> TestResult {
    let catalog = Catalog::builtin()?;
    let report = compare(&source()?, &gaps()?, &RuleContext::default());
    let first = &report.findings[0];
    let en = render(first, &catalog, Locale::En);
    let id = render(first, &catalog, Locale::Id);
    assert_eq!(en.severity_label, "Critical");
    assert_eq!(id.severity_label, "Kritis");
    assert_eq!(en.title, "Port 7700 has no counterpart on the target");
    assert!(en.explanation.contains("searchd"));
    assert_ne!(en.advice, id.advice);
    // The wire value `source` is shown as a localized word.
    let public = report
        .findings
        .iter()
        .find(|f| arg(f, "side") == Some("source"))
        .ok_or("no source-side finding")?;
    assert!(
        render(public, &catalog, Locale::En)
            .title
            .ends_with("on the source server")
    );
    assert!(
        render(public, &catalog, Locale::Id)
            .title
            .ends_with("di server sumber")
    );
    Ok(())
}

#[test]
fn unknown_rule_renders_generically() -> TestResult {
    let catalog = Catalog::builtin()?;
    let mut finding = compare(&source()?, &gaps()?, &RuleContext::default())
        .findings
        .remove(0);
    finding.rule_id = "PAR-999".into();
    let r = render(&finding, &catalog, Locale::En);
    assert!(r.title.contains("PAR-999"));
    assert!(!r.title.contains('{'));
    Ok(())
}

#[test]
fn catalog_keys_follow_rule_ids() -> TestResult {
    let catalog = Catalog::builtin()?;
    for rule in all_rules() {
        let stem = rule.id().to_lowercase().replace('-', "");
        for part in ["title", "explanation", "advice"] {
            let key = format!("parity.{stem}.{part}");
            for locale in Locale::ALL {
                assert!(
                    catalog.raw(locale, &key).is_some(),
                    "{key} missing in {}",
                    locale.tag()
                );
            }
        }
    }
    Ok(())
}

fn app(name: &str, port: u16, mode: &str, supervised: &str) -> String {
    format!(
        r#"{},{{"kind":"process","key":"proc:{name}","data":{{"cmd":"{name}","cwd":null,"mode":"{mode}","supervised":{supervised},"env_refs":[]}},"seen_via":"proc"}}"#,
        node_listener(port, name)
    )
}

#[test]
fn par006_flags_dev_and_unsupervised_apps_on_the_target_only() -> TestResult {
    let target = listener_inventory(&format!(
        "{},{},{},{}",
        app("web", 3000, "dev", "true"),
        app("api", 4000, "unknown", "false"),
        app("ok", 5000, "unknown", "true"),
        app("who", 6000, "unknown", "null"),
    ))?;
    let found = run(&Par006, &listener_inventory("")?, &target);
    assert_eq!(
        keys(&found),
        [
            ("PAR-006", Severity::Important, "proc:api"),
            ("PAR-006", Severity::Important, "proc:web"),
        ]
    );
    assert_eq!(arg(&found[0], "reason"), Some("unsupervised"));
    assert_eq!(arg(&found[1], "reason"), Some("dev"));
    assert_eq!(arg(&found[1], "ports"), Some("3000"));
    // The same inventory as the source never triggers: only the target is judged.
    assert!(run(&Par006, &target, &listener_inventory("")?).is_empty());
    Ok(())
}

#[test]
fn par006_dev_wins_and_processes_without_listeners_are_ignored() -> TestResult {
    let target = listener_inventory(&format!(
        r#"{},{{"kind":"process","key":"proc:bash","data":{{"mode":"unknown","supervised":false}},"seen_via":"proc"}}"#,
        app("web", 3000, "dev", "false")
    ))?;
    let found = run(&Par006, &listener_inventory("")?, &target);
    assert_eq!(keys(&found), [("PAR-006", Severity::Important, "proc:web")]);
    assert_eq!(arg(&found[0], "reason"), Some("dev"));
    Ok(())
}

#[test]
fn par006_renders_in_both_locales() -> TestResult {
    let target = listener_inventory(&app("web", 3000, "dev", "true"))?;
    let found = run(&Par006, &listener_inventory("")?, &target);
    let catalog = Catalog::builtin()?;
    let en = render(&found[0], &catalog, Locale::En);
    assert!(
        en.explanation.contains("a development server"),
        "{}",
        en.explanation
    );
    let id = render(&found[0], &catalog, Locale::Id);
    assert!(
        id.explanation.contains("server pengembangan"),
        "{}",
        id.explanation
    );
    assert!(!id.title.contains("parity."), "{}", id.title);
    Ok(())
}

fn inventory_with(status: &str, items: &str) -> Result<Inventory, Box<dyn Error>> {
    load(&format!(
        r#"{{"schema_version":1,"taken_at":"t","collector_version":"c",
        "platform":{{"os":"linux","arch":"x86_64"}},"status":"{status}","warnings":[],
        "items":[{items}]}}"#
    ))
}

fn env_ref(app: &str, name: &str, host: &str, port: Option<u16>, in_container: bool) -> String {
    let port = port.map_or_else(|| "null".to_owned(), |p| p.to_string());
    format!(
        r#"{{"kind":"env_ref","key":"{app}:{name}","data":{{"app":"{app}","name":"{name}","target_host":"{host}","target_port":{port},"in_container":{in_container},"resolves_to":null}},"seen_via":"config"}}"#
    )
}

fn docker(name: &str, version: Option<&str>) -> String {
    let v = version.map_or_else(|| "null".to_owned(), |v| format!("\"{v}\""));
    format!(
        r#"{{"kind":"process","key":"docker:{name}","data":{{"version":{v}}},"seen_via":"docker"}}"#
    )
}

fn empty() -> Result<Inventory, Box<dyn Error>> {
    inventory_with("complete", "")
}

#[test]
fn par007_flags_local_references_without_a_listener_on_the_target_only() -> TestResult {
    let items = [
        env_ref("pm2:web", "CACHE_URL", "127.0.0.1", Some(6379), false),
        env_ref("pm2:web", "SEARCH_URL", "0.0.0.0", Some(7700), false),
        env_ref("pm2:web", "DB_HOST", "localhost", Some(5432), false),
        env_ref("pm2:web", "OTHER", "203.0.113.5", Some(9999), false),
        env_ref("pm2:web", "NOPORT", "localhost", None, false),
        env_ref("docker:app", "IN_BOX", "localhost", Some(8888), true),
        node_listener(5432, "postgres"),
    ]
    .join(",");
    let target = inventory_with("complete", &items)?;
    let found = run(&Par007, &empty()?, &target);
    assert_eq!(
        keys(&found),
        [
            ("PAR-007", Severity::Important, "pm2:web:CACHE_URL"),
            ("PAR-007", Severity::Important, "pm2:web:SEARCH_URL"),
        ]
    );
    assert_eq!(arg(&found[0], "app"), Some("pm2:web"));
    assert_eq!(arg(&found[0], "name"), Some("CACHE_URL"));
    assert_eq!(arg(&found[0], "host"), Some("127.0.0.1"));
    assert_eq!(arg(&found[0], "port"), Some("6379"));
    assert_eq!(arg(&found[0], "partial"), None);
    // The source side is never judged.
    assert!(run(&Par007, &target, &empty()?).is_empty());
    Ok(())
}

#[test]
fn par007_is_silent_when_the_port_has_a_listener() -> TestResult {
    let items = format!(
        "{},{}",
        env_ref("pm2:web", "CACHE_URL", "::1", Some(6379), false),
        node_listener(6379, "redis-server")
    );
    assert!(run(&Par007, &empty()?, &inventory_with("complete", &items)?).is_empty());
    Ok(())
}

#[test]
fn par007_is_capped_on_a_partial_target() -> TestResult {
    let items = env_ref("pm2:web", "CACHE_URL", "127.0.0.1", Some(6379), false);
    let found = run(&Par007, &empty()?, &inventory_with("partial", &items)?);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].severity, Severity::Info);
    assert_eq!(arg(&found[0], "partial"), Some("true"));
    // A partial SOURCE does not matter: only the target is read.
    let complete = inventory_with("complete", &items)?;
    let found = run(&Par007, &inventory_with("partial", "")?, &complete);
    assert_eq!(found[0].severity, Severity::Important);
    Ok(())
}

#[test]
fn par002_flags_container_loopback_references_to_host_listeners() -> TestResult {
    let items = [
        env_ref("docker:app", "DB_HOST", "localhost", Some(5432), true),
        env_ref("docker:app", "CACHE", "::1", Some(6379), true),
        env_ref("docker:app", "ANY", "0.0.0.0", Some(5432), true),
        env_ref("docker:app", "NOLISTEN", "127.0.0.1", Some(9999), true),
        env_ref("docker:app", "NOPORT", "localhost", None, true),
        env_ref("docker:app", "REMOTE", "203.0.113.7", Some(5432), true),
        env_ref("pm2:web", "HOST_APP", "localhost", Some(5432), false),
        node_listener(5432, "postgres"),
        node_listener(6379, "redis-server"),
    ]
    .join(",");
    let target = inventory_with("complete", &items)?;
    let found = run(&Par002, &empty()?, &target);
    assert_eq!(
        keys(&found),
        [
            ("PAR-002", Severity::Important, "docker:app:CACHE"),
            ("PAR-002", Severity::Important, "docker:app:DB_HOST"),
        ]
    );
    assert_eq!(arg(&found[1], "port"), Some("5432"));
    assert_eq!(arg(&found[1], "host"), Some("localhost"));
    assert!(run(&Par002, &target, &empty()?).is_empty());
    Ok(())
}

#[test]
fn par002_is_not_capped_on_a_partial_target() -> TestResult {
    let items = format!(
        "{},{}",
        env_ref("docker:app", "DB_HOST", "localhost", Some(5432), true),
        node_listener(5432, "postgres")
    );
    let found = run(&Par002, &empty()?, &inventory_with("partial", &items)?);
    assert_eq!(
        keys(&found),
        [("PAR-002", Severity::Important, "docker:app:DB_HOST")]
    );
    assert_eq!(arg(&found[0], "partial"), None);
    Ok(())
}

#[test]
fn par009_flags_major_version_changes_only() -> TestResult {
    let source = inventory_with(
        "complete",
        &[
            docker("db", Some("14.2")),
            docker("cache", Some("7.0")),
            docker("proxy", None),
            docker("tool", Some("v1")),
            docker("gone", Some("1.0")),
            docker("pad", Some("08.1")),
            docker("alpine", Some("1.25-alpine")),
        ]
        .join(","),
    )?;
    let target = inventory_with(
        "partial",
        &[
            docker("db", Some("16.1")),
            docker("cache", Some("7.2")),
            docker("proxy", Some("1.0")),
            docker("tool", Some("V2")),
            docker("pad", Some("8.9")),
            docker("alpine", Some("1.26-alpine")),
            docker("new", Some("3.0")),
        ]
        .join(","),
    )?;
    let found = run(&Par009, &source, &target);
    assert_eq!(
        keys(&found),
        [
            ("PAR-009", Severity::Important, "docker:db"),
            ("PAR-009", Severity::Important, "docker:tool"),
        ]
    );
    assert_eq!(arg(&found[0], "name"), Some("docker:db"));
    assert_eq!(arg(&found[0], "source_version"), Some("14.2"));
    assert_eq!(arg(&found[0], "target_version"), Some("16.1"));
    // Both versions were seen, so a partial target does not lower the severity.
    assert_eq!(arg(&found[0], "partial"), None);
    Ok(())
}

#[test]
fn par009_non_numeric_majors_compare_whole_strings_ignoring_case() -> TestResult {
    let s = inventory_with("complete", &docker("x", Some("Stable-1")))?;
    let same = inventory_with("complete", &docker("x", Some("stable-1")))?;
    let other = inventory_with("complete", &docker("x", Some("stable-2")))?;
    assert!(run(&Par009, &s, &same).is_empty());
    assert_eq!(run(&Par009, &s, &other).len(), 1);
    Ok(())
}

fn configuration_pair() -> Result<(Inventory, Inventory), Box<dyn Error>> {
    let source = inventory_with("complete", &docker("db", Some("14.2")))?;
    let items = [
        env_ref("pm2:web", "CACHE_URL", "127.0.0.1", Some(6379), false),
        env_ref("docker:app", "DB_HOST", "localhost", Some(5432), true),
        node_listener(5432, "postgres"),
        docker("db", Some("16.1")),
    ]
    .join(",");
    Ok((source, inventory_with("partial", &items)?))
}

#[test]
fn new_rules_render_in_both_locales_without_leftovers() -> TestResult {
    let (s, t) = configuration_pair()?;
    let report = compare(&s, &t, &RuleContext::default());
    let ids: Vec<_> = report.findings.iter().map(|f| f.rule_id.as_str()).collect();
    for id in ["PAR-002", "PAR-007", "PAR-009"] {
        assert!(ids.contains(&id), "{id} missing from {ids:?}");
    }
    let catalog = Catalog::builtin()?;
    for f in &report.findings {
        for locale in Locale::ALL {
            let r = render(f, &catalog, locale);
            for text in [&r.title, &r.explanation, &r.advice] {
                assert!(!text.contains('{') && !text.contains('}'), "{text}");
                assert!(!text.contains("parity."), "{text}");
                assert!(!text.contains('\u{0}'), "{text}");
            }
        }
    }
    let by = |id: &str| report.findings.iter().find(|f| f.rule_id == id);
    let p9 = by("PAR-009").ok_or("no PAR-009")?;
    let en = render(p9, &catalog, Locale::En);
    assert!(en.explanation.contains("14.2") && en.explanation.contains("16.1"));
    let id = render(p9, &catalog, Locale::Id);
    assert!(id.explanation.contains("versi mayor") || id.title.contains("versi mayor"));
    let p7 = by("PAR-007").ok_or("no PAR-007")?;
    assert!(render(p7, &catalog, Locale::En).note.is_some());
    assert!(render(p7, &catalog, Locale::Id).note.is_some());
    Ok(())
}

#[test]
fn new_rules_are_deterministic_and_never_carry_values() -> TestResult {
    let (s, t) = configuration_pair()?;
    let ctx = RuleContext::default();
    let report = compare(&s, &t, &ctx);
    assert_eq!(report, compare(&s, &t, &ctx));
    // Only the documented argument names appear; there is nowhere for a value to go.
    for f in &report.findings {
        let allowed: &[&str] = match f.rule_id.as_str() {
            "PAR-002" | "PAR-007" => &["app", "name", "host", "port", "partial"],
            "PAR-009" => &["name", "source_version", "target_version"],
            _ => continue,
        };
        assert!(
            f.args.keys().all(|k| allowed.contains(&k.as_str())),
            "{:?}",
            f.args
        );
    }
    Ok(())
}

/// `cleaned` passes server-supplied values through the cleaner but leaves the rules' own
/// "unknown" marker alone, so the renderer can still turn it into the localized word.
#[test]
fn cleaning_keeps_the_unknown_marker_and_cleans_everything_else() {
    use inventory::{Finding, Kind, Severity, SubjectRef};
    use std::collections::BTreeMap;
    let mut args = BTreeMap::new();
    args.insert("process".to_owned(), parity::UNKNOWN_ARG.to_owned());
    args.insert("bind".to_owned(), "10.0.0.1\u{1b}[2J".to_owned());
    let finding = Finding {
        rule_id: "PAR-001".to_owned(),
        severity: Severity::Info,
        subject: SubjectRef {
            kind: Kind::Listener,
            key: "tcp:22\u{7}".to_owned(),
        },
        evidence: Vec::new(),
        args,
    };
    let clean = |t: &str| {
        t.chars()
            .map(|c| if c.is_control() { '\u{FFFD}' } else { c })
            .collect()
    };
    let out = parity::cleaned(&finding, &clean);
    assert_eq!(
        out.args["process"],
        parity::UNKNOWN_ARG,
        "the marker survives"
    );
    assert_eq!(out.args["bind"], "10.0.0.1\u{FFFD}[2J");
    assert_eq!(out.subject.key, "tcp:22\u{FFFD}");
    assert_eq!(
        finding.args["bind"], "10.0.0.1\u{1b}[2J",
        "the original is untouched"
    );
}
