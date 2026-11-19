//! Validation, scheduling, strict parsing and storage round trips of the plan model.
//! A misbehaving caller (or a model drafting a plan) must fail each of these.

use inventory::{Inventory, RuleContext};
use plan::{
    DraftOptions, MAX_DATABASES, MAX_JSON_BYTES, MAX_STEPS, Plan, PlanError, PlanSpec,
    ScheduleRefusal, Side, Step, StepKind, StoredStep, draft_plan, gate_report,
};
use std::collections::BTreeSet;
use std::error::Error;

type TestResult = Result<(), Box<dyn Error>>;

fn inv(json: &str) -> Result<Inventory, Box<dyn Error>> {
    Ok(Inventory::from_json(json)?)
}
fn source() -> Result<Inventory, Box<dyn Error>> {
    inv(include_str!("../../parity/tests/fixtures/source.json"))
}
fn gaps() -> Result<Inventory, Box<dyn Error>> {
    inv(include_str!("../../parity/tests/fixtures/target-gaps.json"))
}

const SRC: &str = "123e4567-e89b-42d3-a456-426614174001";
const DST: &str = "123e4567-e89b-42d3-a456-426614174002";

fn options() -> DraftOptions {
    DraftOptions::new(
        "Move web stack",
        SRC,
        DST,
        "123e4567-e89b-42d3-a456-426614174003",
    )
}

/// A valid plan with one restored database: steps 0..=9 (see `draft_plan`).
fn valid() -> Result<Plan, Box<dyn Error>> {
    let mut o = options();
    o.databases = vec![plan::DatabaseEntry {
        key: "pg:5432:appdb".into(),
        mode: plan::DbMode::Restore,
    }];
    Ok(draft_plan(&source()?, Some(&gaps()?), &[], &o)?.plan)
}

fn kind_seq(p: &Plan, kind: StepKind) -> u32 {
    p.steps()
        .iter()
        .find(|s| s.kind == kind)
        .map_or(u32::MAX, |s| s.seq)
}

fn rebuild(p: &Plan, steps: Vec<Step>) -> Result<Plan, PlanError> {
    Plan::new(p.name().to_owned(), p.spec().clone(), steps)
}

#[test]
fn a_drafted_plan_is_valid_and_ordered() -> TestResult {
    let p = valid()?;
    let kinds: Vec<StepKind> = p.execution_order().iter().map(|s| s.kind).collect();
    use StepKind::*;
    assert_eq!(
        kinds,
        [
            Inventory,
            Inventory,
            Backup,
            Transfer,
            Restore,
            Restore,
            Verify,
            Delta,
            Cutover,
            PostVerify,
            DecommissionSource
        ]
    );
    Ok(())
}

#[test]
fn removing_any_required_kind_is_refused() -> TestResult {
    let p = valid()?;
    for kind in StepKind::ALL.into_iter().filter(|k| k.required()) {
        let dropped: Vec<u32> = p
            .steps()
            .iter()
            .filter(|s| s.kind == kind)
            .map(|s| s.seq)
            .collect();
        let steps: Vec<Step> = p
            .steps()
            .iter()
            .filter(|s| s.kind != kind)
            .cloned()
            .map(|mut s| {
                s.depends_on.retain(|d| !dropped.contains(d));
                s
            })
            .collect();
        let err = rebuild(&p, steps).err();
        assert!(
            matches!(err, Some(PlanError::MissingKind(k)) if k == kind),
            "{kind:?}: {err:?}"
        );
    }
    Ok(())
}

#[test]
fn optional_kinds_may_be_absent() -> TestResult {
    let mut o = options();
    o.include_delta = false;
    o.include_decommission = false;
    let draft = draft_plan(&source()?, Some(&gaps()?), &[], &o)?;
    assert!(
        draft
            .plan
            .steps()
            .iter()
            .all(|s| !matches!(s.kind, StepKind::Delta | StepKind::DecommissionSource))
    );
    Ok(())
}

fn set_deps(steps: &mut [Step], seq: u32, deps: Vec<u32>) {
    for s in steps.iter_mut().filter(|s| s.seq == seq) {
        s.depends_on = deps.clone();
    }
}

#[test]
fn every_ordering_violation_is_refused() -> TestResult {
    let p = valid()?;
    let (backup, transfer, restore, verify, delta, cutover, post, decom) = (
        kind_seq(&p, StepKind::Backup),
        kind_seq(&p, StepKind::Transfer),
        kind_seq(&p, StepKind::Restore),
        kind_seq(&p, StepKind::Verify),
        kind_seq(&p, StepKind::Delta),
        kind_seq(&p, StepKind::Cutover),
        kind_seq(&p, StepKind::PostVerify),
        kind_seq(&p, StepKind::DecommissionSource),
    );

    // Reversed edges (no cycle): a later kind runs before an earlier one.
    let reversed: [(&str, u32, Vec<u32>); 5] = [
        // cutover does not wait for verify, verify waits for cutover.
        ("verify after cutover", verify, vec![cutover]),
        // delta after cutover.
        ("delta after cutover", delta, vec![cutover]),
        // decommission before post_verify.
        ("decommission first", post, vec![decom]),
        // restore waits for verify.
        ("restore after verify", restore, vec![verify]),
        // backup waits for transfer.
        ("backup after transfer", backup, vec![transfer]),
    ];
    for (label, seq, deps) in reversed {
        let mut steps = p.steps().to_vec();
        set_deps(&mut steps, seq, deps);
        // Detach the original edge in the other direction so the graph stays acyclic.
        let victim = match label {
            "verify after cutover" => cutover,
            "delta after cutover" => cutover,
            "decommission first" => decom,
            "restore after verify" => verify,
            _ => transfer,
        };
        let victim_deps: Vec<u32> = match label {
            "verify after cutover" => vec![restore],
            "delta after cutover" => vec![verify],
            "decommission first" => vec![cutover],
            "restore after verify" => vec![transfer],
            _ => vec![],
        };
        set_deps(&mut steps, victim, victim_deps);
        let err = rebuild(&p, steps).err();
        assert!(
            matches!(&err, Some(PlanError::OrderReversed { .. })),
            "{label}: {err:?}"
        );
    }

    // Missing edges: a later kind does not depend on an earlier one.
    let missing: [(&str, u32, Vec<u32>); 5] = [
        ("cutover skips verify", cutover, vec![restore]),
        ("post_verify skips cutover", post, vec![verify]),
        ("decommission skips post_verify", decom, vec![cutover]),
        ("transfer skips backup", transfer, vec![0, 1]),
        ("delta skips verify", delta, vec![restore]),
    ];
    for (label, seq, deps) in missing {
        let mut steps = p.steps().to_vec();
        set_deps(&mut steps, seq, deps);
        let err = rebuild(&p, steps).err();
        assert!(
            matches!(err, Some(PlanError::OrderMissing { .. })),
            "{label}: {err:?}"
        );
    }
    Ok(())
}

#[test]
fn delta_may_only_sit_between_verify_and_cutover() -> TestResult {
    let p = valid()?;
    let delta = kind_seq(&p, StepKind::Delta);
    let verify = kind_seq(&p, StepKind::Verify);
    let post = kind_seq(&p, StepKind::PostVerify);
    // Delta that only follows post_verify (after cutover) while cutover still depends on it: cycle.
    let mut steps = p.steps().to_vec();
    set_deps(&mut steps, delta, vec![post]);
    assert_eq!(rebuild(&p, steps).err(), Some(PlanError::Cycle));
    // Delta that is not behind verify.
    let mut steps = p.steps().to_vec();
    set_deps(&mut steps, delta, vec![0, 1]);
    assert!(matches!(
        rebuild(&p, steps).err(),
        Some(PlanError::OrderMissing { .. })
    ));
    let _ = verify;
    Ok(())
}

#[test]
fn cycles_missing_self_and_duplicate_dependencies_are_refused() -> TestResult {
    let p = valid()?;
    let verify = kind_seq(&p, StepKind::Verify);
    let cutover = kind_seq(&p, StepKind::Cutover);

    let mut steps = p.steps().to_vec();
    set_deps(&mut steps, verify, vec![cutover]);
    assert_eq!(rebuild(&p, steps).err(), Some(PlanError::Cycle));

    // A cycle between steps of the same kind.
    let mut steps = p.steps().to_vec();
    let restores: Vec<u32> = steps
        .iter()
        .filter(|s| s.kind == StepKind::Restore)
        .map(|s| s.seq)
        .collect();
    set_deps(&mut steps, restores[0], vec![3, restores[1]]);
    set_deps(&mut steps, restores[1], vec![3, restores[0]]);
    assert_eq!(rebuild(&p, steps).err(), Some(PlanError::Cycle));

    let mut steps = p.steps().to_vec();
    set_deps(&mut steps, cutover, vec![verify, 999]);
    assert_eq!(
        rebuild(&p, steps).err(),
        Some(PlanError::UnknownDependency {
            step: cutover,
            missing: 999
        })
    );

    let mut steps = p.steps().to_vec();
    set_deps(&mut steps, cutover, vec![cutover]);
    assert_eq!(
        rebuild(&p, steps).err(),
        Some(PlanError::SelfDependency(cutover))
    );

    let mut steps = p.steps().to_vec();
    set_deps(&mut steps, cutover, vec![verify, verify]);
    assert_eq!(
        rebuild(&p, steps).err(),
        Some(PlanError::DuplicateDependency(cutover))
    );

    let mut steps = p.steps().to_vec();
    steps.push(steps[0].clone());
    assert_eq!(rebuild(&p, steps).err(), Some(PlanError::DuplicateSeq(0)));

    assert_eq!(rebuild(&p, Vec::new()).err(), Some(PlanError::NoSteps));
    Ok(())
}

#[test]
fn single_kinds_and_step_count_are_bounded() -> TestResult {
    let p = valid()?;
    for kind in [StepKind::Cutover, StepKind::DecommissionSource] {
        let mut steps = p.steps().to_vec();
        let mut extra = steps
            .iter()
            .find(|s| s.kind == kind)
            .cloned()
            .ok_or("no step")?;
        extra.seq = 500;
        steps.push(extra);
        assert_eq!(rebuild(&p, steps).err(), Some(PlanError::TooMany(kind)));
    }
    let mut steps = p.steps().to_vec();
    for i in 0..MAX_STEPS as u32 {
        steps.push(Step::new(1000 + i, StepKind::Restore, vec![3]));
    }
    assert!(matches!(
        rebuild(&p, steps).err(),
        Some(PlanError::TooLarge { .. })
    ));
    Ok(())
}

#[test]
fn sides_and_databases_are_checked() -> TestResult {
    let p = valid()?;
    let backup = kind_seq(&p, StepKind::Backup);
    let verify = kind_seq(&p, StepKind::Verify);

    let mut steps = p.steps().to_vec();
    steps[0].side = None;
    steps[1].side = None;
    assert_eq!(rebuild(&p, steps).err(), Some(PlanError::SideRequired(0)));

    let mut steps = p.steps().to_vec();
    steps[1].side = Some(Side::Source);
    steps[0].side = Some(Side::Target);
    steps[1].side = Some(Side::Target);
    assert_eq!(rebuild(&p, steps).err(), Some(PlanError::NoSourceInventory));

    let mut steps = p.steps().to_vec();
    for s in steps.iter_mut().filter(|s| s.seq == backup) {
        s.side = Some(Side::Source);
    }
    assert_eq!(
        rebuild(&p, steps).err(),
        Some(PlanError::SideNotAllowed(backup))
    );

    let mut steps = p.steps().to_vec();
    for s in steps.iter_mut().filter(|s| s.seq == backup) {
        s.database = Some("pg:5432:appdb".into());
    }
    assert_eq!(
        rebuild(&p, steps).err(),
        Some(PlanError::DatabaseNotAllowed(backup))
    );

    let mut steps = p.steps().to_vec();
    for s in steps.iter_mut().filter(|s| s.seq == verify) {
        s.database = Some("pg:5432:other".into());
    }
    assert_eq!(
        rebuild(&p, steps).err(),
        Some(PlanError::UnknownDatabase(verify))
    );

    // A database marked skip must never be restored.
    let mut spec = p.spec().clone();
    spec.data.databases[0].mode = plan::DbMode::Skip;
    let err = Plan::new("x".into(), spec, p.steps().to_vec()).err();
    assert!(
        matches!(err, Some(PlanError::SkippedDatabaseRestored(_))),
        "{err:?}"
    );
    Ok(())
}

#[test]
fn source_and_target_must_differ_and_the_name_is_checked() -> TestResult {
    let p = valid()?;
    let mut spec = p.spec().clone();
    spec.target.node_id = spec.source.node_id.clone();
    assert_eq!(
        Plan::new("x".into(), spec, p.steps().to_vec()).err(),
        Some(PlanError::SameNode)
    );
    for bad in ["", "   ", "a\nb", &"x".repeat(201)] {
        assert!(Plan::new(bad.into(), p.spec().clone(), p.steps().to_vec()).is_err());
    }
    // The same through the drafting entry point.
    let o = DraftOptions::new("x", SRC, SRC, "t");
    assert_eq!(
        draft_plan(&source()?, None, &[], &o).err(),
        Some(PlanError::SameNode)
    );
    Ok(())
}

// ---- spec rules ----

fn spec_json() -> Result<String, Box<dyn Error>> {
    Ok(valid()?.spec_json()?)
}

fn with(json: &str, from: &str, to: &str) -> String {
    assert!(json.contains(from), "{from} not in {json}");
    json.replacen(from, to, 1)
}

#[test]
fn the_spec_round_trips_and_matches_the_design_shape() -> TestResult {
    let json = spec_json()?;
    let spec = PlanSpec::from_json(&json)?;
    assert_eq!(spec.to_json()?, json);
    assert!(json.contains("\"strategy\":\"snapshot+delta\""));
    assert!(json.contains("\"conflict_policy\":\"never_overwrite\""));
    assert!(json.contains("\"encryption\":\"client_side\""));
    Ok(())
}

#[test]
fn strict_parsing_rejects_unknown_fields_and_wrong_values() -> TestResult {
    let json = spec_json()?;
    let cases = [
        // Unknown fields at every level.
        with(&json, "{\"source\"", "{\"extra\":1,\"source\""),
        with(&json, "\"cutover\":{", "\"cutover\":{\"force\":true,"),
        with(&json, "\"gates\":{", "\"gates\":{\"skip\":true,"),
        with(
            &json,
            "\"transport\":{",
            "\"transport\":{\"password\":\"x\",",
        ),
        with(&json, "\"backup\":{", "\"backup\":{\"compress\":1,"),
        with(&json, "\"data\":{", "\"data\":{\"truncate\":true,"),
        // Wrong enum values.
        with(&json, "\"never_overwrite\"", "\"overwrite\""),
        with(&json, "\"never_overwrite\"", "\"always_overwrite\""),
        with(&json, "\"snapshot+delta\"", "\"full\""),
        with(&json, "\"client_side\"", "\"server_side\""),
        with(&json, "\"tunnel\"", "\"magic\""),
        with(&json, "\"restore\"", "\"drop\""),
        with(&json, "\"source\",\"strategy\"", "\"nowhere\",\"strategy\""),
        // Wrong types.
        with(&json, "\"rollback\":true", "\"rollback\":\"yes\""),
        with(&json, "\"exclude\":[", "\"exclude\":{\"a\":["),
    ];
    for (i, case) in cases.iter().enumerate() {
        let err = PlanSpec::from_json(case).err();
        assert!(
            matches!(err, Some(PlanError::Parse(_))),
            "case {i}: {err:?}"
        );
    }
    // Missing fields are refused too.
    assert!(matches!(
        PlanSpec::from_json("{}").err(),
        Some(PlanError::Parse(_))
    ));
    assert!(matches!(
        PlanSpec::from_json("").err(),
        Some(PlanError::Parse(_))
    ));
    assert!(matches!(
        PlanSpec::from_json("null").err(),
        Some(PlanError::Parse(_))
    ));
    Ok(())
}

#[test]
fn spec_safety_rules_are_enforced_on_parse() -> TestResult {
    let json = spec_json()?;
    // DNS needs snapshot and rollback.
    let dns = with(&json, "\"tunnel\"", "\"dns\"");
    assert!(PlanSpec::from_json(&dns).is_ok());
    for off in ["\"dns_snapshot\":true", "\"rollback\":true"] {
        let bad = with(&dns, off, &off.replace("true", "false"));
        assert_eq!(
            PlanSpec::from_json(&bad).err(),
            Some(PlanError::UnsafeDnsCutover)
        );
    }
    // Gates cannot be switched off.
    let bad = with(
        &json,
        "\"no_open_critical_findings\":true",
        "\"no_open_critical_findings\":false",
    );
    assert_eq!(
        PlanSpec::from_json(&bad).err(),
        Some(PlanError::GateDisabled("no_open_critical_findings"))
    );
    let bad = with(
        &json,
        "\"keep_source_until_verified\":true",
        "\"keep_source_until_verified\":false",
    );
    assert_eq!(
        PlanSpec::from_json(&bad).err(),
        Some(PlanError::GateDisabled("keep_source_until_verified"))
    );
    // An intermediary needs client-side encryption.
    let inter = with(
        &json,
        "\"location\":\"source\"",
        "\"location\":\"intermediary\"",
    );
    assert!(PlanSpec::from_json(&inter).is_ok());
    let bad = with(&inter, "\"client_side\"", "\"none\"");
    assert_eq!(
        PlanSpec::from_json(&bad).err(),
        Some(PlanError::IntermediaryNeedsEncryption)
    );
    Ok(())
}

#[test]
fn spec_lists_values_and_documents_are_bounded() -> TestResult {
    let p = valid()?;
    let mut spec = p.spec().clone();
    spec.data.databases = (0..=MAX_DATABASES)
        .map(|i| plan::DatabaseEntry {
            key: format!("pg:5432:db{i}"),
            mode: plan::DbMode::Skip,
        })
        .collect();
    assert!(matches!(
        spec.validate().err(),
        Some(PlanError::TooLarge { .. })
    ));

    let mut spec = p.spec().clone();
    spec.data.databases.push(spec.data.databases[0].clone());
    assert_eq!(spec.validate().err(), Some(PlanError::DuplicateDatabase));

    let mut spec = p.spec().clone();
    spec.backup.exclude = (0..=plan::MAX_EXCLUDES).map(|i| format!("p{i}")).collect();
    assert!(matches!(
        spec.validate().err(),
        Some(PlanError::TooLarge { .. })
    ));

    for bad in ["*", "/", "", "a\nb", "../x"] {
        let mut spec = p.spec().clone();
        spec.backup.exclude = vec![bad.to_owned()];
        assert_eq!(
            spec.validate().err(),
            Some(PlanError::Invalid("backup.exclude")),
            "{bad:?}"
        );
    }
    let mut spec = p.spec().clone();
    spec.backup.exclude = vec!["logs".into(), "logs".into()];
    assert!(spec.validate().is_err());

    let mut spec = p.spec().clone();
    spec.transport.transport_id = String::new();
    assert!(spec.validate().is_err());
    let mut spec = p.spec().clone();
    spec.source.node_id = "x".repeat(65);
    assert!(spec.validate().is_err());
    let mut spec = p.spec().clone();
    spec.data.databases[0].key = "a\u{0}b".into();
    assert!(spec.validate().is_err());

    let huge = format!("{{\"pad\":\"{}\"}}", "x".repeat(MAX_JSON_BYTES));
    assert!(matches!(
        PlanSpec::from_json(&huge).err(),
        Some(PlanError::TooLarge { .. })
    ));
    assert!(matches!(
        Plan::from_json(&huge).err(),
        Some(PlanError::TooLarge { .. })
    ));
    Ok(())
}

// ---- plan documents and storage ----

#[test]
fn a_plan_round_trips_through_json_and_storage() -> TestResult {
    let p = valid()?;
    assert_eq!(Plan::from_json(&p.to_json()?)?, p);
    let stored = p.stored_steps()?;
    assert_eq!(stored.len(), p.steps().len());
    let back = Plan::from_stored(
        p.name(),
        &p.spec().source.node_id,
        &p.spec().target.node_id,
        &p.spec_json()?,
        &stored,
    )?;
    assert_eq!(back, p);
    assert!(stored.iter().any(|s| s.spec_json.contains("pg:5432:appdb")));
    Ok(())
}

#[test]
fn deserializing_a_plan_validates_it() -> TestResult {
    let p = valid()?;
    let json = p.to_json()?;
    // Direct serde deserialization goes through the same validation as `Plan::new`.
    let bad = with(&json, "\"kind\":\"verify\"", "\"kind\":\"cutover\"");
    assert!(serde_json::from_str::<Plan>(&bad).is_err());
    assert!(matches!(
        Plan::from_json(&bad).err(),
        Some(PlanError::Parse(_))
    ));
    let extra = with(&json, "{\"name\"", "{\"approved\":true,\"name\"");
    assert!(Plan::from_json(&extra).is_err());
    let step_extra = with(&json, "{\"seq\":0", "{\"tier\":0,\"seq\":0");
    assert!(Plan::from_json(&step_extra).is_err());
    let bad_kind = with(&json, "\"kind\":\"backup\"", "\"kind\":\"format_disk\"");
    assert!(Plan::from_json(&bad_kind).is_err());
    Ok(())
}

#[test]
fn stored_data_is_not_trusted() -> TestResult {
    let p = valid()?;
    let spec = p.spec_json()?;
    let good = p.stored_steps()?;
    let (src, dst) = (
        p.spec().source.node_id.clone(),
        p.spec().target.node_id.clone(),
    );

    // The row columns and the spec must name the same nodes (e.g. another org's node in the spec).
    assert_eq!(
        Plan::from_stored("x", &src, "other-orgs-node", &spec, &good).err(),
        Some(PlanError::NodeMismatch)
    );
    assert_eq!(
        Plan::from_stored("x", &dst, &src, &spec, &good).err(),
        Some(PlanError::NodeMismatch)
    );

    let mut stored = good.clone();
    stored[2].kind = "wipe".into();
    assert_eq!(
        Plan::from_stored("x", &src, &dst, &spec, &stored).err(),
        Some(PlanError::UnknownKind)
    );

    let mut stored = good.clone();
    stored[0].spec_json = "{\"side\":\"source\",\"tier\":0}".into();
    assert!(matches!(
        Plan::from_stored("x", &src, &dst, &spec, &stored).err(),
        Some(PlanError::Parse(_))
    ));

    let mut stored = good.clone();
    stored[0].spec_json = "not json".into();
    assert!(matches!(
        Plan::from_stored("x", &src, &dst, &spec, &stored).err(),
        Some(PlanError::Parse(_))
    ));

    let mut stored = good.clone();
    stored[3].depends_on = vec![];
    assert!(matches!(
        Plan::from_stored("x", &src, &dst, &spec, &stored).err(),
        Some(PlanError::OrderMissing { .. })
    ));

    let many: Vec<StoredStep> = (0..=MAX_STEPS as u32)
        .map(|i| StoredStep {
            seq: i,
            kind: "inventory".into(),
            spec_json: "{}".into(),
            depends_on: vec![],
        })
        .collect();
    assert!(matches!(
        Plan::from_stored("x", &src, &dst, &spec, &many).err(),
        Some(PlanError::TooLarge { .. })
    ));

    let tampered = with(&spec, "\"never_overwrite\"", "\"overwrite\"");
    assert!(Plan::from_stored("x", &src, &dst, &tampered, &good).is_err());
    Ok(())
}

// ---- scheduling and gates ----

fn done(p: &Plan, kinds_before: StepKind) -> BTreeSet<u32> {
    p.steps()
        .iter()
        .filter(|s| s.kind.rank() < kinds_before.rank())
        .map(|s| s.seq)
        .collect()
}

#[test]
fn cutover_is_refused_without_gates_or_with_an_open_critical() -> TestResult {
    let p = valid()?;
    let cutover = kind_seq(&p, StepKind::Cutover);
    let completed = done(&p, StepKind::Cutover);

    assert_eq!(
        p.check_schedulable(cutover, &completed, None),
        Err(ScheduleRefusal::GatesUnknown)
    );

    let s = source()?;
    let t = gaps()?;
    let findings = parity::compare(&s, &t, &RuleContext::default()).findings;
    let blocked = gate_report(&findings, &[]);
    assert!(!blocked.open_critical.is_empty());
    assert_eq!(
        p.check_schedulable(cutover, &completed, Some(&blocked)),
        Err(ScheduleRefusal::GatesNotSatisfied)
    );

    let clean = gate_report(&[], &[]);
    assert_eq!(
        p.check_schedulable(cutover, &completed, Some(&clean)),
        Ok(())
    );
    Ok(())
}

#[test]
fn decommission_is_never_schedulable_before_everything_else_and_the_gates() -> TestResult {
    let p = valid()?;
    let decom = kind_seq(&p, StepKind::DecommissionSource);
    let clean = gate_report(&[], &[]);
    let all_but_last: BTreeSet<u32> = p
        .steps()
        .iter()
        .filter(|s| s.seq != decom)
        .map(|s| s.seq)
        .collect();

    // Everything done and clean gates: allowed.
    assert_eq!(
        p.check_schedulable(decom, &all_but_last, Some(&clean)),
        Ok(())
    );
    // No gate report.
    assert_eq!(
        p.check_schedulable(decom, &all_but_last, None),
        Err(ScheduleRefusal::GatesUnknown)
    );
    // An open critical finding.
    let s = source()?;
    let findings = parity::compare(&s, &gaps()?, &RuleContext::default()).findings;
    let blocked = gate_report(&findings, &[]);
    assert_eq!(
        p.check_schedulable(decom, &all_but_last, Some(&blocked)),
        Err(ScheduleRefusal::GatesNotSatisfied)
    );
    // Incomplete evidence is the same as an open critical.
    let mut incomplete = gate_report(&[], &[]);
    incomplete.inventory_incomplete = true;
    assert_eq!(
        p.check_schedulable(decom, &all_but_last, Some(&incomplete)),
        Err(ScheduleRefusal::GatesNotSatisfied)
    );
    // post_verify (a dependency) not finished.
    let post = kind_seq(&p, StepKind::PostVerify);
    let mut missing_post = all_but_last.clone();
    missing_post.remove(&post);
    assert_eq!(
        p.check_schedulable(decom, &missing_post, Some(&clean)),
        Err(ScheduleRefusal::DependenciesPending(vec![post]))
    );
    // Dependencies done but some unrelated step (a verify of another branch) still open.
    let mut spec_steps = p.steps().to_vec();
    spec_steps.push(Step::new(900, StepKind::Restore, vec![3]));
    // That plan is invalid by order (later steps must follow every restore): proof the shape cannot exist.
    assert!(rebuild(&p, spec_steps).is_err());
    // Nothing completed at all.
    assert!(matches!(
        p.check_schedulable(decom, &BTreeSet::new(), Some(&clean)),
        Err(ScheduleRefusal::DependenciesPending(_))
    ));
    Ok(())
}

#[test]
fn scheduling_rejects_unknown_done_and_blocked_steps() -> TestResult {
    let p = valid()?;
    let none = BTreeSet::new();
    assert_eq!(
        p.check_schedulable(999, &none, None),
        Err(ScheduleRefusal::UnknownStep)
    );
    assert_eq!(p.check_schedulable(0, &none, None), Ok(()));
    let first: BTreeSet<u32> = [0].into();
    assert_eq!(
        p.check_schedulable(0, &first, None),
        Err(ScheduleRefusal::AlreadyCompleted)
    );
    assert!(matches!(
        p.check_schedulable(3, &first, None),
        Err(ScheduleRefusal::DependenciesPending(_))
    ));
    Ok(())
}

// ---- draft_plan on the parity fixtures ----

#[test]
fn drafting_against_the_gap_fixture_blocks_on_the_critical_finding() -> TestResult {
    let s = source()?;
    let t = gaps()?;
    let findings = parity::compare(&s, &t, &RuleContext::default()).findings;
    let draft = draft_plan(&s, Some(&t), &findings, &options())?;
    let open: Vec<&str> = draft
        .gates
        .open_critical
        .iter()
        .map(|k| k.subject.key.as_str())
        .collect();
    assert_eq!(open, ["tcp:7700"]);
    assert!(!draft.gates.cutover_allowed());
    assert!(draft.gates.important >= 2);
    Ok(())
}

#[test]
fn accepting_the_critical_with_a_reason_clears_the_gate_but_a_blank_reason_does_not() -> TestResult
{
    let s = source()?;
    let t = gaps()?;
    let findings = parity::compare(&s, &t, &RuleContext::default()).findings;
    let subject = findings
        .iter()
        .find(|f| f.severity == inventory::Severity::Critical)
        .map(|f| f.subject.clone())
        .ok_or("no critical")?;
    let mut o = options();
    o.accepted = vec![plan::AcceptedFinding::new(
        "PAR-001",
        subject.clone(),
        "search is retired",
    )?];
    let draft = draft_plan(&s, Some(&t), &findings, &o)?;
    assert!(draft.gates.cutover_allowed());
    assert_eq!(draft.gates.accepted_critical.len(), 1);

    // A literal with a blank reason bypasses `new` but not the gate.
    let mut o = options();
    o.accepted = vec![plan::AcceptedFinding {
        key: plan::FindingKey {
            rule_id: "PAR-001".into(),
            subject,
        },
        reason: " ".into(),
    }];
    assert!(
        !draft_plan(&s, Some(&t), &findings, &o)?
            .gates
            .cutover_allowed()
    );
    Ok(())
}

#[test]
fn the_equivalent_target_has_no_blocking_finding() -> TestResult {
    let s = source()?;
    let t = inv(include_str!("../../parity/tests/fixtures/target-ok.json"))?;
    let findings = parity::compare(&s, &t, &RuleContext::default()).findings;
    let draft = draft_plan(&s, Some(&t), &findings, &options())?;
    assert!(draft.gates.cutover_allowed());
    assert_eq!(draft.warnings, [plan::DraftWarning::NoDatabases]);
    Ok(())
}

#[test]
fn a_partial_or_missing_target_never_passes_the_gates_by_silence() -> TestResult {
    let s = source()?;
    let partial = inv(include_str!(
        "../../parity/tests/fixtures/target-partial.json"
    ))?;
    let findings = parity::compare(&s, &partial, &RuleContext::default()).findings;
    let draft = draft_plan(&s, Some(&partial), &findings, &options())?;
    assert!(
        draft.gates.open_critical.is_empty(),
        "absence findings are capped at info"
    );
    assert!(draft.gates.inventory_incomplete);
    assert!(!draft.gates.cutover_allowed());
    assert!(
        draft
            .warnings
            .contains(&plan::DraftWarning::TargetInventoryPartial)
    );

    // No target inventory and therefore no findings: still blocked.
    let none = draft_plan(&s, None, &[], &options())?;
    assert!(!none.gates.cutover_allowed());
    assert!(
        none.warnings
            .contains(&plan::DraftWarning::NoTargetInventory)
    );
    Ok(())
}

#[test]
fn databases_are_planned_by_mode_and_skipped_ones_get_no_restore() -> TestResult {
    let mut o = options();
    o.databases = vec![
        plan::DatabaseEntry {
            key: "pg:5432:a".into(),
            mode: plan::DbMode::Restore,
        },
        plan::DatabaseEntry {
            key: "pg:5432:b".into(),
            mode: plan::DbMode::Skip,
        },
        plan::DatabaseEntry {
            key: "mysql:3306:c".into(),
            mode: plan::DbMode::CopyAsSuffix,
        },
    ];
    let draft = draft_plan(&source()?, Some(&gaps()?), &[], &o)?;
    let restored: Vec<&str> = draft
        .plan
        .steps()
        .iter()
        .filter(|s| s.kind == StepKind::Restore)
        .filter_map(|s| s.database.as_deref())
        .collect();
    assert_eq!(restored, ["pg:5432:a", "mysql:3306:c"]);
    assert_eq!(
        draft.plan.spec().data.conflict_policy,
        plan::ConflictPolicy::NeverOverwrite
    );
    Ok(())
}

#[test]
fn drafting_is_deterministic_and_dns_always_gets_snapshot_and_rollback() -> TestResult {
    let s = source()?;
    let t = gaps()?;
    let mut o = options();
    o.cutover_method = plan::CutoverMethod::Dns;
    let a = draft_plan(&s, Some(&t), &[], &o)?;
    let b = draft_plan(&s, Some(&t), &[], &o)?;
    assert_eq!(a, b);
    assert!(a.plan.spec().cutover.dns_snapshot && a.plan.spec().cutover.rollback);
    assert!(
        a.plan.spec().gates.no_open_critical_findings
            && a.plan.spec().gates.keep_source_until_verified
    );
    assert_eq!(a.plan.spec().backup.exclude, plan::default_excludes());
    Ok(())
}

#[test]
fn drafting_refuses_bad_options() -> TestResult {
    let s = source()?;
    let mut o = options();
    o.backup_location = plan::BackupLocation::Intermediary;
    o.encryption = plan::Encryption::None;
    assert_eq!(
        draft_plan(&s, None, &[], &o).err(),
        Some(PlanError::IntermediaryNeedsEncryption)
    );
    let mut o = options();
    o.exclude = Some(vec!["*".into()]);
    assert!(draft_plan(&s, None, &[], &o).is_err());
    let mut o = options();
    o.name = String::new();
    assert!(draft_plan(&s, None, &[], &o).is_err());
    Ok(())
}
