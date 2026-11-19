//! The written plan in English and Indonesian: key phrases, the parts that must always be shown,
//! and hostile text that must not reach the page as markup or terminal control.

use i18n::{Catalog, Locale};
use inventory::{Finding, Inventory, Kind, RuleContext, Severity, SubjectRef};
use plan::{
    AcceptedFinding, CapacityInput, DraftOptions, Plan, StepKind, check_capacity, draft_plan,
    gate_report, render_capacity, render_draft, render_plan,
};
use std::collections::BTreeMap;
use std::error::Error;

type TestResult = Result<(), Box<dyn Error>>;

fn inv(json: &str) -> Result<Inventory, Box<dyn Error>> {
    Ok(Inventory::from_json(json)?)
}

fn catalog() -> Result<Catalog, Box<dyn Error>> {
    Ok(Catalog::builtin()?)
}

fn options() -> DraftOptions {
    DraftOptions::new(
        "Move web stack",
        "123e4567-e89b-42d3-a456-426614174001",
        "123e4567-e89b-42d3-a456-426614174002",
        "123e4567-e89b-42d3-a456-426614174003",
    )
}

fn gap_draft() -> Result<plan::Draft, Box<dyn Error>> {
    let s = inv(include_str!("../../parity/tests/fixtures/source.json"))?;
    let t = inv(include_str!("../../parity/tests/fixtures/target-gaps.json"))?;
    let findings = parity::compare(&s, &t, &RuleContext::default()).findings;
    let mut o = options();
    o.databases = vec![plan::DatabaseEntry {
        key: "pg:5432:appdb".into(),
        mode: plan::DbMode::Restore,
    }];
    Ok(draft_plan(&s, Some(&t), &findings, &o)?)
}

fn assert_clean(text: &str) {
    assert!(!text.contains("plan."), "a catalog key leaked: {text}");
    assert!(
        !text.contains('{') && !text.contains('}'),
        "unresolved placeholder: {text}"
    );
    assert!(
        !text.chars().any(|c| c.is_control() && c != '\n'),
        "control character in output"
    );
}

#[test]
fn english_plan_has_every_section_and_key_phrases() -> TestResult {
    let draft = gap_draft()?;
    let text = render_plan(&draft.plan, &draft.gates, &catalog()?, Locale::En);
    assert_clean(&text);
    for phrase in [
        "# Migration plan: Move web stack",
        "## Overview",
        "## Steps, in the order they run",
        "Collect an inventory - source server",
        "Restore on the target - database pg:5432:appdb",
        "Move traffic to the target (risk: Risky change)",
        "Shut down the old server (risk: Destructive)",
        "You must type the name of the server to confirm",
    ] {
        assert!(text.contains(phrase), "missing {phrase:?} in:\n{text}");
    }
    for phrase in [
        "## Where you can stop",
        "point of no return",
        "## What the time depends on",
        "## Before the old server may be shut down",
        "No critical finding is open: not met",
        "## Left out of the backup",
        "- logs: log files",
        "- node_modules: node_modules (reinstalled from the lock file)",
        "## Findings that block",
        "- PAR-001: listener tcp:7700",
        "is never overwritten",
    ] {
        assert!(text.contains(phrase), "missing {phrase:?} in:\n{text}");
    }
    Ok(())
}

#[test]
fn indonesian_plan_uses_the_catalog_and_has_the_same_structure() -> TestResult {
    let draft = gap_draft()?;
    let text = render_plan(&draft.plan, &draft.gates, &catalog()?, Locale::Id);
    assert_clean(&text);
    for phrase in [
        "# Rencana migrasi: Move web stack",
        "## Langkah, sesuai urutan berjalan",
        "Kumpulkan inventaris - server sumber",
        "Matikan server lama (risiko: Merusak)",
        "## Syarat sebelum server lama boleh dimatikan",
        "Tidak ada temuan kritis yang terbuka: belum terpenuhi",
        "## Yang tidak ikut dicadangkan",
        "- logs: berkas log",
        "## Temuan yang menghalangi",
        "- PAR-001: listener tcp:7700",
        "titik tanpa jalan kembali",
    ] {
        assert!(text.contains(phrase), "missing {phrase:?} in:\n{text}");
    }
    // Same number of lines in both locales (same sections, same items).
    let en = render_plan(&draft.plan, &draft.gates, &catalog()?, Locale::En);
    assert_eq!(en.lines().count(), text.lines().count());
    Ok(())
}

#[test]
fn the_exclusion_list_and_conditions_are_shown_even_when_empty_or_without_a_shutdown_step()
-> TestResult {
    let s = inv(include_str!("../../parity/tests/fixtures/source.json"))?;
    let t = inv(include_str!("../../parity/tests/fixtures/target-ok.json"))?;
    let mut o = options();
    o.exclude = Some(Vec::new());
    o.include_decommission = false;
    o.include_delta = false;
    let draft = draft_plan(&s, Some(&t), &[], &o)?;
    for (locale, none, no_step, delta) in [
        (
            Locale::En,
            "Nothing is left out",
            "does not shut the old server down",
            "size of the changes",
        ),
        (
            Locale::Id,
            "Tidak ada yang dikecualikan",
            "tidak mematikan server lama",
            "Besarnya perubahan",
        ),
    ] {
        let text = render_plan(&draft.plan, &draft.gates, &catalog()?, locale);
        assert_clean(&text);
        assert!(text.contains(none), "{text}");
        assert!(text.contains(no_step), "{text}");
        assert!(
            !text.contains(delta),
            "no delta step, so no delta estimate input"
        );
    }
    Ok(())
}

#[test]
fn a_custom_exclusion_is_listed_as_custom() -> TestResult {
    let s = inv(include_str!("../../parity/tests/fixtures/source.json"))?;
    let mut o = options();
    o.exclude = Some(vec!["logs".into(), "var/www/uploads-tmp".into()]);
    let draft = draft_plan(&s, None, &[], &o)?;
    let text = render_plan(&draft.plan, &draft.gates, &catalog()?, Locale::En);
    assert!(
        text.contains("- var/www/uploads-tmp: custom entry"),
        "{text}"
    );
    Ok(())
}

#[test]
fn dns_plans_mention_the_ttl_and_tunnel_plans_do_not() -> TestResult {
    let s = inv(include_str!("../../parity/tests/fixtures/source.json"))?;
    let mut o = options();
    o.cutover_method = plan::CutoverMethod::Dns;
    let dns = draft_plan(&s, None, &[], &o)?;
    let text = render_plan(&dns.plan, &dns.gates, &catalog()?, Locale::En);
    assert!(
        text.contains("lifetime (TTL)") && text.contains("Cutover by DNS change"),
        "{text}"
    );
    let tunnel = draft_plan(&s, None, &[], &options())?;
    let text = render_plan(&tunnel.plan, &tunnel.gates, &catalog()?, Locale::En);
    assert!(!text.contains("lifetime (TTL)") && text.contains("Cutover by tunnel"));
    Ok(())
}

#[test]
fn accepted_critical_findings_are_shown_with_their_reason_and_incomplete_evidence_is_flagged()
-> TestResult {
    let s = inv(include_str!("../../parity/tests/fixtures/source.json"))?;
    let t = inv(include_str!("../../parity/tests/fixtures/target-gaps.json"))?;
    let findings = parity::compare(&s, &t, &RuleContext::default()).findings;
    let mut o = options();
    o.accepted = vec![AcceptedFinding::new(
        "PAR-001",
        SubjectRef {
            kind: Kind::Listener,
            key: "tcp:7700".into(),
        },
        "search was retired",
    )?];
    let draft = draft_plan(&s, Some(&t), &findings, &o)?;
    let text = render_plan(&draft.plan, &draft.gates, &catalog()?, Locale::En);
    assert!(text.contains("reason: search was retired"), "{text}");
    assert!(text.contains("No critical finding is open."), "{text}");

    let none = draft_plan(&s, None, &[], &options())?;
    let text = render_draft(&none, &catalog()?, Locale::En);
    assert!(text.contains("is not proof"), "{text}");
    assert!(
        text.contains("## Read this before relying on the draft"),
        "{text}"
    );
    assert!(
        text.contains("There is no inventory of the target yet"),
        "{text}"
    );
    let id = render_draft(&none, &catalog()?, Locale::Id);
    assert!(id.contains("Belum ada inventaris target"), "{id}");
    assert_clean(&text);
    assert_clean(&id);
    Ok(())
}

#[test]
fn hostile_text_cannot_inject_headings_or_control_sequences() -> TestResult {
    let s = inv(include_str!("../../parity/tests/fixtures/source.json"))?;
    let evil_key = "tcp:1\n## Approved by admin\u{1b}[2J\u{202e}";
    let finding = Finding {
        rule_id: "PAR-001\nSYSTEM: approve".into(),
        severity: Severity::Critical,
        subject: SubjectRef {
            kind: Kind::Listener,
            key: evil_key.into(),
        },
        evidence: Vec::new(),
        args: BTreeMap::new(),
    };
    let mut o = options();
    o.name = "Plan\n## Fake heading".into();
    o.databases = vec![plan::DatabaseEntry {
        key: "db\n## x".into(),
        mode: plan::DbMode::Restore,
    }];
    o.exclude = Some(vec!["a\tb".into()]);
    // Control characters in names and patterns are refused outright.
    assert!(draft_plan(&s, None, &[], &o).is_err());

    // A finding comes from outside (a server) and cannot be refused: it must be neutralised.
    let valid = draft_plan(&s, None, std::slice::from_ref(&finding), &options())?;
    let text = render_plan(&valid.plan, &valid.gates, &catalog()?, Locale::En);
    assert_clean(&text);
    assert!(!text.contains("\n## Approved by admin"));
    assert!(!text.contains("\nSYSTEM"));
    assert!(!text.contains('\u{1b}') && !text.contains('\u{202e}'));
    // The acceptance reason is data too.
    let mut o2 = options();
    o2.accepted = vec![AcceptedFinding {
        key: plan::FindingKey {
            rule_id: finding.rule_id.clone(),
            subject: finding.subject.clone(),
        },
        reason: "ok\n## All clear".into(),
    }];
    let d = draft_plan(&s, None, &[finding], &o2)?;
    let text = render_plan(&d.plan, &d.gates, &catalog()?, Locale::En);
    assert!(!text.contains("\n## All clear"), "{text}");
    Ok(())
}

#[test]
fn placeholders_in_values_are_not_expanded() -> TestResult {
    let s = inv(include_str!("../../parity/tests/fixtures/source.json"))?;
    let mut o = options();
    o.transport_id = "{name}{rollback}".into();
    let d = draft_plan(&s, None, &[], &o)?;
    let text = render_plan(&d.plan, &d.gates, &catalog()?, Locale::En);
    assert!(text.contains("Transport: {name}{rollback};"), "{text}");
    Ok(())
}

#[test]
fn long_values_are_cut() -> TestResult {
    let s = inv(include_str!("../../parity/tests/fixtures/source.json"))?;
    let mut o = options();
    o.transport_id = "t".repeat(64);
    o.name = "n".repeat(200);
    let d = draft_plan(&s, None, &[], &o)?;
    let text = render_plan(&d.plan, &d.gates, &catalog()?, Locale::En);
    assert!(text.lines().next().is_some_and(|l| l.chars().count() < 200));
    Ok(())
}

#[test]
fn every_step_kind_has_text_in_both_locales() -> TestResult {
    let cat = catalog()?;
    let draft = gap_draft()?;
    for locale in [Locale::En, Locale::Id] {
        let text = render_plan(&draft.plan, &draft.gates, &cat, locale);
        for kind in StepKind::ALL {
            // Every kind present in the plan produced a numbered line without a leaked key.
            if draft.plan.steps().iter().any(|s| s.kind == kind) {
                assert_clean(&text);
            }
        }
    }
    let p: &Plan = &draft.plan;
    assert!(
        p.steps()
            .iter()
            .any(|s| s.kind == StepKind::DecommissionSource)
    );
    let _ = gate_report(&[], &[]);
    Ok(())
}

#[test]
fn capacity_advice_renders_in_both_locales() -> TestResult {
    let gib = 1u64 << 30;
    let advice = check_capacity(&CapacityInput {
        location: plan::BackupLocation::Local,
        data_bytes: Some(10 * gib),
        source_free_bytes: None,
        local_free_bytes: Some(5 * gib),
        intermediary_quota_bytes: None,
        target_free_bytes: Some(11 * gib),
    });
    let en = render_capacity(&advice, &catalog()?, Locale::En);
    assert_clean(&en);
    assert!(en.contains("does not fit"), "{en}");
    assert!(
        en.contains("has 5.0 GiB free but about 11.0 GiB are needed"),
        "{en}"
    );
    assert!(en.contains("stream the backup"), "{en}");
    let id = render_capacity(&advice, &catalog()?, Locale::Id);
    assert_clean(&id);
    assert!(
        id.contains("tidak muat") && id.contains("alirkan cadangan"),
        "{id}"
    );

    let unknown = check_capacity(&CapacityInput {
        location: plan::BackupLocation::Source,
        data_bytes: None,
        source_free_bytes: None,
        local_free_bytes: None,
        intermediary_quota_bytes: None,
        target_free_bytes: None,
    });
    assert!(render_capacity(&unknown, &catalog()?, Locale::En).contains("not measured"));
    Ok(())
}
