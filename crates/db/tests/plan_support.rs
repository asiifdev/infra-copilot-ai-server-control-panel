//! The guarded draft insert and the lookups of the AI planning tools: limits checked inside the
//! write transaction, names matched without regard to case, tenant isolation.

mod common;
use common::*;
use db::*;

fn node(conn: &Connection, org: &OrgId, name: &str) -> std::result::Result<NodeId, Error> {
    create_node(
        conn,
        org,
        &NewNode {
            name,
            kind: NodeKind::Vps,
            access: NodeAccess::Agent,
            labels_json: "{}",
        },
    )
}

const STEPS: &[(u32, PlanStepKind)] = &[(0, PlanStepKind::Inventory)];

fn new_plan<'a>(name: &'a str, a: &'a NodeId, b: &'a NodeId) -> NewPlan<'a> {
    NewPlan {
        name,
        source_node: a,
        target_node: b,
        spec_json: "{}",
    }
}

fn steps() -> Vec<NewPlanStep<'static>> {
    STEPS
        .iter()
        .map(|(seq, kind)| NewPlanStep {
            seq: *seq,
            kind: *kind,
            spec_json: "{}",
            depends_on: &[],
        })
        .collect()
}

#[test]
fn a_draft_is_stored_as_draft_and_the_name_is_unique_without_regard_to_case() -> TestResult {
    let conn = open_in_memory()?;
    let t = populate(&conn, "a")?;
    let other = node(&conn, &t.org, "db-1")?;
    let id = create_draft_plan(
        &conn,
        &t.org,
        &new_plan("Move", &t.node, &other),
        &steps(),
        5,
    )?;
    assert_eq!(get_plan(&conn, &t.org, &id)?.status, PlanStatus::Draft);
    for name in ["Move", "move", "MOVE"] {
        let again = create_draft_plan(&conn, &t.org, &new_plan(name, &t.node, &other), &steps(), 5);
        assert!(matches!(again, Err(DraftError::NameTaken)), "{name}");
    }
    assert_eq!(find_plan_by_name(&conn, &t.org, "mOvE")?.id, id);
    Ok(())
}

#[test]
fn the_draft_limit_counts_drafts_of_this_org_only() -> TestResult {
    let conn = open_in_memory()?;
    let (a, b) = (populate(&conn, "a")?, populate(&conn, "b")?);
    let (a2, b2) = (node(&conn, &a.org, "x")?, node(&conn, &b.org, "x")?);
    for name in ["one", "two"] {
        create_draft_plan(&conn, &a.org, &new_plan(name, &a.node, &a2), &steps(), 2)?;
    }
    let third = create_draft_plan(&conn, &a.org, &new_plan("three", &a.node, &a2), &steps(), 2);
    assert!(matches!(third, Err(DraftError::TooManyDrafts)));
    // The other org has its own room, and may reuse a name.
    create_draft_plan(&conn, &b.org, &new_plan("one", &b.node, &b2), &steps(), 2)?;
    // A plan that left `draft` frees a slot.
    let id = find_plan_by_name(&conn, &a.org, "one")?.id;
    let mut conn = conn;
    set_plan_status(&mut conn, &a.org, &id, PlanStatus::Aborted)?;
    create_draft_plan(&conn, &a.org, &new_plan("three", &a.node, &a2), &steps(), 2)?;
    Ok(())
}

#[test]
fn a_draft_cannot_point_at_another_orgs_node_or_the_same_node_twice() -> TestResult {
    let conn = open_in_memory()?;
    let (a, b) = (populate(&conn, "a")?, populate(&conn, "b")?);
    let a2 = node(&conn, &a.org, "x")?;
    let foreign = create_draft_plan(&conn, &a.org, &new_plan("f", &a.node, &b.node), &steps(), 5);
    assert!(matches!(foreign, Err(DraftError::Db(Error::NotFound))));
    let same = create_draft_plan(&conn, &a.org, &new_plan("s", &a.node, &a.node), &steps(), 5);
    assert!(matches!(same, Err(DraftError::Db(Error::Invalid(_)))));
    assert!(list_plans(&conn, &a.org, 10)?.is_empty());
    // Nothing of the failed attempts is visible to the other org either.
    assert!(matches!(
        find_plan_by_name(&conn, &b.org, "f"),
        Err(Error::NotFound)
    ));
    let _ = a2;
    Ok(())
}

#[test]
fn the_latest_comparison_between_two_nodes_is_found_per_org() -> TestResult {
    let mut conn = open_in_memory()?;
    let (a, b) = (populate(&conn, "a")?, populate(&conn, "b")?);
    let a2 = node(&conn, &a.org, "x")?;
    let save = |conn: &mut Connection, org: &OrgId, node: &NodeId| {
        save_inventory(
            conn,
            org,
            &NewInventory {
                node,
                taken_at: "2026-11-19T10:00:00Z",
                collector_version: "0.1.0",
                status: InventoryStatus::Complete,
                os: "linux",
                arch: "x86_64",
                warnings_json: "[]",
            },
            &[],
        )
    };
    let (s, t) = (
        save(&mut conn, &a.org, &a.node)?,
        save(&mut conn, &a.org, &a2)?,
    );
    let c = save_comparison(&mut conn, &a.org, &s, &t, &[])?;
    assert_eq!(
        latest_comparison_between(&conn, &a.org, &a.node, &a2)?.id,
        c
    );
    // The other direction, an unrelated node and another org are all "none".
    for (org, x, y) in [(&a.org, &a2, &a.node), (&b.org, &a.node, &a2)] {
        assert!(matches!(
            latest_comparison_between(&conn, org, x, y),
            Err(Error::NotFound)
        ));
    }
    Ok(())
}
