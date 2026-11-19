//! Migration plans and steps: storage rules, status rules and tenant isolation.

mod common;
use common::*;
use db::*;

const SPEC: &str = r#"{"source":{"node_id":"a"},"transport":{"transport_id":"t"}}"#;

fn second_node(conn: &Connection, org: &OrgId, name: &str) -> std::result::Result<NodeId, Error> {
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

fn step<'a>(seq: u32, kind: PlanStepKind, depends_on: &'a [u32]) -> NewPlanStep<'a> {
    NewPlanStep {
        seq,
        kind,
        spec_json: "{}",
        depends_on,
    }
}

/// inventory(0) -> backup(1) -> delta(2) -> decommission(3)
fn steps() -> Vec<NewPlanStep<'static>> {
    vec![
        step(0, PlanStepKind::Inventory, &[]),
        step(1, PlanStepKind::Backup, &[0]),
        step(2, PlanStepKind::Delta, &[1]),
        step(3, PlanStepKind::DecommissionSource, &[1, 2]),
    ]
}

struct Fixture {
    conn: Connection,
    a: Tenant,
    b: Tenant,
    a_target: NodeId,
    plan: PlanId,
}

fn fixture() -> std::result::Result<Fixture, Box<dyn std::error::Error>> {
    let mut conn = open_in_memory()?;
    let a = populate(&conn, "alpha")?;
    let b = populate(&conn, "beta")?;
    let a_target = second_node(&conn, &a.org, "web-2")?;
    let plan = create_plan(
        &mut conn,
        &a.org,
        &NewPlan {
            name: "move",
            source_node: &a.node,
            target_node: &a_target,
            spec_json: SPEC,
        },
        &steps(),
    )?;
    Ok(Fixture {
        conn,
        a,
        b,
        a_target,
        plan,
    })
}

#[test]
fn stores_and_reads_back_a_plan_with_its_steps() -> TestResult {
    let f = fixture()?;
    let row = get_plan(&f.conn, &f.a.org, &f.plan)?;
    assert_eq!(row.name, "move");
    assert_eq!(row.status, PlanStatus::Draft);
    assert_eq!(row.source_node_id, f.a.node);
    assert_eq!(row.target_node_id, f.a_target);
    assert_eq!(row.spec_json, SPEC);
    let listed = list_plans(&f.conn, &f.a.org, 10)?;
    assert_eq!(listed, vec![row]);
    let stored = list_plan_steps(&f.conn, &f.a.org, &f.plan)?;
    assert_eq!(stored.len(), 4);
    assert_eq!(stored[3].kind, PlanStepKind::DecommissionSource);
    assert_eq!(stored[3].depends_on, vec![1, 2]);
    assert_eq!(stored[0].depends_on, Vec::<u32>::new());
    assert!(stored.iter().all(|s| s.status == PlanStepStatus::Pending));
    Ok(())
}

#[test]
fn source_and_target_must_differ_and_json_must_be_json() -> TestResult {
    let mut f = fixture()?;
    let same = NewPlan {
        name: "x",
        source_node: &f.a.node,
        target_node: &f.a.node,
        spec_json: SPEC,
    };
    assert!(matches!(
        create_plan(&mut f.conn, &f.a.org, &same, &[]),
        Err(Error::Invalid(_))
    ));
    let bad_json = NewPlan {
        name: "x",
        source_node: &f.a.node,
        target_node: &f.a_target,
        spec_json: "{not json",
    };
    assert!(is_constraint(&create_plan(
        &mut f.conn,
        &f.a.org,
        &bad_json,
        &[]
    )));
    let huge = format!("{{\"p\":\"{}\"}}", "x".repeat(262_144));
    let too_big = NewPlan {
        spec_json: &huge,
        ..bad_json
    };
    assert!(is_constraint(&create_plan(
        &mut f.conn,
        &f.a.org,
        &too_big,
        &[]
    )));
    // Raw SQL cannot bypass the schema rules either.
    let id = "123e4567-e89b-42d3-a456-426614174099";
    let direct = f.conn.execute(
        "INSERT INTO migration_plans (id, org_id, name, source_node_id, target_node_id, spec) \
         VALUES (?1, ?2, 'x', ?3, ?3, '{}')",
        rusqlite::params![id, f.a.org.as_str(), f.a.node.as_str()],
    );
    assert!(direct.is_err());
    let bad_status = f.conn.execute(
        "INSERT INTO migration_plans (id, org_id, name, source_node_id, target_node_id, status, spec) \
         VALUES (?1, ?2, 'x', ?3, ?4, 'approved-ish', '{}')",
        rusqlite::params![id, f.a.org.as_str(), f.a.node.as_str(), f.a_target.as_str()],
    );
    assert!(bad_status.is_err());
    Ok(())
}

#[test]
fn a_failing_step_rolls_the_whole_plan_back() -> TestResult {
    let mut f = fixture()?;
    let before = list_plans(&f.conn, &f.a.org, 10)?.len();
    let dup = [
        step(0, PlanStepKind::Inventory, &[]),
        step(0, PlanStepKind::Backup, &[]),
    ];
    let new = NewPlan {
        name: "dup",
        source_node: &f.a.node,
        target_node: &f.a_target,
        spec_json: SPEC,
    };
    assert!(is_constraint(&create_plan(
        &mut f.conn,
        &f.a.org,
        &new,
        &dup
    )));
    assert_eq!(list_plans(&f.conn, &f.a.org, 10)?.len(), before);
    // A step document that is not JSON is refused.
    let bad = [NewPlanStep {
        spec_json: "nope",
        ..step(0, PlanStepKind::Inventory, &[])
    }];
    assert!(is_constraint(&create_plan(
        &mut f.conn,
        &f.a.org,
        &new,
        &bad
    )));
    assert_eq!(list_plans(&f.conn, &f.a.org, 10)?.len(), before);
    // depends_on must be a JSON array (raw SQL).
    let raw = f.conn.execute(
        "INSERT INTO migration_steps (org_id, plan_id, seq, kind, depends_on) \
         VALUES (?1, ?2, 50, 'verify', '{\"a\":1}')",
        rusqlite::params![f.a.org.as_str(), f.plan.as_str()],
    );
    assert!(raw.is_err());
    Ok(())
}

#[test]
fn org_b_cannot_read_list_update_or_link_org_a_plans() -> TestResult {
    let mut f = fixture()?;
    let b_target = second_node(&f.conn, &f.b.org, "web-2")?;
    let b_plan = create_plan(
        &mut f.conn,
        &f.b.org,
        &NewPlan {
            name: "beta move",
            source_node: &f.b.node,
            target_node: &b_target,
            spec_json: SPEC,
        },
        &steps(),
    )?;
    let a_before = (
        get_plan(&f.conn, &f.a.org, &f.plan)?,
        list_plan_steps(&f.conn, &f.a.org, &f.plan)?,
    );

    // Reads, as B, with A's id.
    assert!(is_not_found(&get_plan(&f.conn, &f.b.org, &f.plan)));
    assert!(is_not_found(&list_plan_steps(&f.conn, &f.b.org, &f.plan)));
    assert!(
        list_plans(&f.conn, &f.b.org, 100)?
            .iter()
            .all(|p| p.id != f.plan)
    );
    assert_eq!(list_plans(&f.conn, &f.b.org, 100)?.len(), 1);
    // And the other way round.
    assert!(is_not_found(&get_plan(&f.conn, &f.a.org, &b_plan)));

    // Writes, as B, with A's id.
    let evil = NewPlan {
        name: "hijacked",
        source_node: &f.b.node,
        target_node: &b_target,
        spec_json: "{}",
    };
    assert!(is_not_found(&replace_plan_content(
        &mut f.conn,
        &f.b.org,
        &f.plan,
        &evil,
        &[]
    )));
    assert!(is_not_found(&set_plan_status(
        &mut f.conn,
        &f.b.org,
        &f.plan,
        PlanStatus::Aborted
    )));
    assert!(is_not_found(&set_plan_step_status(
        &f.conn,
        &f.b.org,
        &f.plan,
        0,
        PlanStepStatus::Succeeded
    )));

    // Pointing a plan at another org's node, as either end, through the API.
    for (src, dst) in [
        (&f.a.node, &f.b.node),
        (&f.b.node, &f.a_target),
        (&f.b.node, &b_target),
    ] {
        let new = NewPlan {
            name: "cross",
            source_node: src,
            target_node: dst,
            spec_json: SPEC,
        };
        let created = create_plan(&mut f.conn, &f.a.org, &new, &steps());
        assert!(is_not_found(&created), "{created:?}");
    }
    // Updating one of A's own plans to point at B's node is refused too.
    let cross = NewPlan {
        name: "move",
        source_node: &f.a.node,
        target_node: &f.b.node,
        spec_json: SPEC,
    };
    assert!(is_not_found(&replace_plan_content(
        &mut f.conn,
        &f.a.org,
        &f.plan,
        &cross,
        &steps()
    )));

    // The schema refuses the same links when raw SQL is used (composite foreign keys).
    let id = "123e4567-e89b-42d3-a456-426614174098";
    let link = f.conn.execute(
        "INSERT INTO migration_plans (id, org_id, name, source_node_id, target_node_id, spec) \
         VALUES (?1, ?2, 'x', ?3, ?4, '{}')",
        rusqlite::params![id, f.a.org.as_str(), f.a.node.as_str(), f.b.node.as_str()],
    );
    assert!(link.is_err());
    let step_link = f.conn.execute(
        "INSERT INTO migration_steps (org_id, plan_id, seq, kind) VALUES (?1, ?2, 77, 'verify')",
        rusqlite::params![f.a.org.as_str(), b_plan.as_str()],
    );
    assert!(
        step_link.is_err(),
        "org A must not add a step to org B's plan"
    );

    // Nothing of A changed, nothing of B changed.
    assert_eq!(get_plan(&f.conn, &f.a.org, &f.plan)?, a_before.0);
    assert_eq!(list_plan_steps(&f.conn, &f.a.org, &f.plan)?, a_before.1);
    assert_eq!(get_plan(&f.conn, &f.b.org, &b_plan)?.name, "beta move");
    assert_eq!(list_plan_steps(&f.conn, &f.b.org, &b_plan)?.len(), 4);
    Ok(())
}

#[test]
fn a_node_that_a_plan_uses_cannot_be_deleted() -> TestResult {
    let f = fixture()?;
    assert!(is_constraint(&delete_node(&f.conn, &f.a.org, &f.a_target)));
    assert!(get_node(&f.conn, &f.a.org, &f.a_target).is_ok());
    Ok(())
}

#[test]
fn status_follows_the_allowed_transitions_only() -> TestResult {
    let mut f = fixture()?;
    use PlanStatus::*;
    // Forbidden jumps.
    for bad in [Approved, Running, Done, Paused] {
        assert!(
            matches!(
                set_plan_status(&mut f.conn, &f.a.org, &f.plan, bad),
                Err(Error::Invalid(_))
            ),
            "draft -> {bad:?}"
        );
    }
    for next in [Reviewed, Approved, Running, Paused, Running, Done] {
        set_plan_status(&mut f.conn, &f.a.org, &f.plan, next)?;
    }
    assert_eq!(get_plan(&f.conn, &f.a.org, &f.plan)?.status, Done);
    // Final states stay final.
    for next in [Draft, Reviewed, Running, Aborted] {
        assert!(matches!(
            set_plan_status(&mut f.conn, &f.a.org, &f.plan, next),
            Err(Error::Invalid(_))
        ));
    }
    // The table itself.
    for from in [Draft, Reviewed, Approved, Running, Paused, Done, Aborted] {
        assert!(!from.can_become(from), "{from:?} -> itself");
    }
    assert!(!Done.can_become(Aborted) && !Aborted.can_become(Draft));
    Ok(())
}

#[test]
fn only_a_draft_can_change_content() -> TestResult {
    let mut f = fixture()?;
    let new = NewPlan {
        name: "renamed",
        source_node: &f.a.node,
        target_node: &f.a_target,
        spec_json: r#"{"v":2}"#,
    };
    let fewer = [step(0, PlanStepKind::Inventory, &[])];
    replace_plan_content(&mut f.conn, &f.a.org, &f.plan, &new, &fewer)?;
    assert_eq!(get_plan(&f.conn, &f.a.org, &f.plan)?.name, "renamed");
    assert_eq!(list_plan_steps(&f.conn, &f.a.org, &f.plan)?.len(), 1);

    set_plan_status(&mut f.conn, &f.a.org, &f.plan, PlanStatus::Reviewed)?;
    let refused = replace_plan_content(&mut f.conn, &f.a.org, &f.plan, &new, &fewer);
    assert!(matches!(refused, Err(Error::Invalid(_))));
    assert_eq!(list_plan_steps(&f.conn, &f.a.org, &f.plan)?.len(), 1);
    Ok(())
}

#[test]
fn decommission_cannot_move_before_every_other_step_succeeded() -> TestResult {
    let f = fixture()?;
    let org = &f.a.org;
    let set = |seq, status| set_plan_step_status(&f.conn, org, &f.plan, seq, status);

    // A new decommission step cannot be created already approved or running (raw SQL).
    let sneaky = f.conn.execute(
        "INSERT INTO migration_steps (org_id, plan_id, seq, kind, status) \
         VALUES (?1, ?2, 40, 'decommission_source', 'approved')",
        rusqlite::params![org.as_str(), f.plan.as_str()],
    );
    assert!(sneaky.is_err());

    // Nothing done yet.
    for status in [
        PlanStepStatus::Proposed,
        PlanStepStatus::Approved,
        PlanStepStatus::Running,
        PlanStepStatus::Succeeded,
    ] {
        assert!(is_constraint(&set(3, status)), "{status:?} must be refused");
    }
    // Harmless states are fine (for example to mark it failed or skipped by a person).
    set(3, PlanStepStatus::Skipped)?;
    set(3, PlanStepStatus::Pending)?;

    set(0, PlanStepStatus::Succeeded)?;
    set(1, PlanStepStatus::Succeeded)?;
    // The delta is still pending.
    assert!(is_constraint(&set(3, PlanStepStatus::Proposed)));
    // A failed or rolled back step blocks it as well.
    set(2, PlanStepStatus::Failed)?;
    assert!(is_constraint(&set(3, PlanStepStatus::Proposed)));
    set(2, PlanStepStatus::RolledBack)?;
    assert!(is_constraint(&set(3, PlanStepStatus::Proposed)));
    // The optional delta may be skipped; then the way is open.
    set(2, PlanStepStatus::Skipped)?;
    set(3, PlanStepStatus::Proposed)?;
    set(3, PlanStepStatus::Approved)?;
    Ok(())
}

#[test]
fn a_skipped_required_step_does_not_unlock_decommission() -> TestResult {
    let f = fixture()?;
    let org = &f.a.org;
    set_plan_step_status(&f.conn, org, &f.plan, 0, PlanStepStatus::Succeeded)?;
    set_plan_step_status(&f.conn, org, &f.plan, 1, PlanStepStatus::Skipped)?;
    set_plan_step_status(&f.conn, org, &f.plan, 2, PlanStepStatus::Skipped)?;
    assert!(is_constraint(&set_plan_step_status(
        &f.conn,
        org,
        &f.plan,
        3,
        PlanStepStatus::Proposed
    )));
    Ok(())
}

#[test]
fn unknown_step_numbers_are_not_found() -> TestResult {
    let f = fixture()?;
    assert!(is_not_found(&set_plan_step_status(
        &f.conn,
        &f.a.org,
        &f.plan,
        99,
        PlanStepStatus::Failed
    )));
    Ok(())
}
