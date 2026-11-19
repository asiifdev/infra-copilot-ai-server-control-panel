//! The plan crate and the db crate agree on names and shapes (db stores opaque JSON text and does
//! not depend on `plan`, so this test is the contract between them). In-memory SQLite only.

use db::{
    NewNode, NewPlan, NewPlanStep, NodeAccess, NodeKind, PlanStepKind, create_node, create_org,
    create_plan, get_plan, list_plan_steps, open_in_memory,
};
use inventory::Inventory;
use plan::{DraftOptions, Plan, StepKind, StoredStep, draft_plan};
use std::error::Error;

type TestResult = Result<(), Box<dyn Error>>;

#[test]
fn step_kind_names_match_between_the_crates() -> TestResult {
    for kind in StepKind::ALL {
        assert_eq!(PlanStepKind::parse(kind.as_str())?.as_str(), kind.as_str());
    }
    Ok(())
}

#[test]
fn a_drafted_plan_survives_a_round_trip_through_the_database() -> TestResult {
    let mut conn = open_in_memory()?;
    let org = create_org(&conn, "Org", "org", "free")?.id;
    let mk = |name| {
        create_node(
            &conn,
            &org,
            &NewNode {
                name,
                kind: NodeKind::Vps,
                access: NodeAccess::Agent,
                labels_json: "{}",
            },
        )
    };
    let (src, dst, other) = (mk("old")?, mk("new")?, mk("other")?);

    let inv = Inventory::from_json(include_str!("../../parity/tests/fixtures/source.json"))?;
    let mut options = DraftOptions::new("move", src.as_str(), dst.as_str(), "transport-1");
    options.databases = vec![plan::DatabaseEntry {
        key: "pg:5432:appdb".into(),
        mode: plan::DbMode::Restore,
    }];
    let draft = draft_plan(&inv, None, &[], &options)?;

    let spec_json = draft.plan.spec_json()?;
    let stored = draft.plan.stored_steps()?;
    let rows: Vec<NewPlanStep<'_>> = stored
        .iter()
        .map(|s| {
            Ok(NewPlanStep {
                seq: s.seq,
                kind: PlanStepKind::parse(&s.kind)?,
                spec_json: &s.spec_json,
                depends_on: &s.depends_on,
            })
        })
        .collect::<Result<_, db::Error>>()?;
    let id = create_plan(
        &mut conn,
        &org,
        &NewPlan {
            name: draft.plan.name(),
            source_node: &src,
            target_node: &dst,
            spec_json: &spec_json,
        },
        &rows,
    )?;

    let row = get_plan(&conn, &org, &id)?;
    let back: Vec<StoredStep> = list_plan_steps(&conn, &org, &id)?
        .into_iter()
        .map(|s| StoredStep {
            seq: s.seq,
            kind: s.kind.as_str().to_owned(),
            spec_json: s.spec_json,
            depends_on: s.depends_on,
        })
        .collect();
    let rebuilt = Plan::from_stored(
        &row.name,
        row.source_node_id.as_str(),
        row.target_node_id.as_str(),
        &row.spec_json,
        &back,
    )?;
    assert_eq!(rebuilt, draft.plan);
    assert_eq!(row.source_node_id, src);

    // A spec that names another node than the row columns is refused on load.
    let mismatched = create_plan(
        &mut conn,
        &org,
        &NewPlan {
            name: "bad",
            source_node: &src,
            target_node: &other,
            spec_json: &spec_json,
        },
        &rows,
    )?;
    let bad = get_plan(&conn, &org, &mismatched)?;
    assert_eq!(
        Plan::from_stored(
            &bad.name,
            bad.source_node_id.as_str(),
            bad.target_node_id.as_str(),
            &bad.spec_json,
            &back,
        )
        .err(),
        Some(plan::PlanError::NodeMismatch)
    );
    Ok(())
}
