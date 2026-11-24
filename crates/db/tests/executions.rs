//! Proposals with an action and their single execution: the state machine, immutability, audit
//! and tenant isolation (ADR 0037).

mod common;
use common::*;
use db::*;
use rusqlite::params;

type TestResultOf<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const ACTION: &str = r#"{"action":"restart_service","manager":"systemd","service":"nginx"}"#;

fn new_proposal(hash: &str) -> NewProposal<'_> {
    NewProposal {
        conversation: None,
        tool: "propose_restart_service",
        tier: 2,
        title: "Restart nginx",
        rationale: "It stopped answering.",
        suggested_action: "Restart the service.",
        impact: "assistant.proposal.impact",
        rule_id: None,
        subject_key: None,
        args_hash: hash,
    }
}

fn approved(conn: &Connection, t: &Tenant) -> TestResultOf<ProposalId> {
    let hash = sha256_hex(ACTION.as_bytes());
    let id = create_action_proposal(conn, &t.org, &new_proposal(&hash), &t.node, ACTION)?;
    decide_proposal(conn, &t.org, &id, ProposalDecision::Approve, &t.user)?;
    Ok(id)
}

fn count_audit(conn: &Connection, t: &Tenant, action: &str) -> TestResultOf<usize> {
    Ok(list_audit(conn, &t.org, 100)?
        .iter()
        .filter(|e| e.action == action)
        .count())
}

#[test]
fn an_action_is_attached_once_to_a_node_of_the_same_org_and_never_changes() -> TestResult {
    let conn = open_in_memory()?;
    let a = populate(&conn, "alpha")?;
    let b = populate(&conn, "beta")?;
    let hash = sha256_hex(ACTION.as_bytes());
    // Another org's node is refused, and nothing is left behind.
    assert!(matches!(
        create_action_proposal(&conn, &a.org, &new_proposal(&hash), &b.node, ACTION),
        Err(Error::NotFound)
    ));
    assert!(list_proposals(&conn, &a.org, None, 10)?.is_empty());

    let id = create_action_proposal(&conn, &a.org, &new_proposal(&hash), &a.node, ACTION)?;
    let row = get_proposal(&conn, &a.org, &id)?;
    assert_eq!(row.action_json.as_deref(), Some(ACTION));
    assert_eq!(row.node_id.as_ref(), Some(&a.node));
    for sql in [
        "UPDATE proposals SET action_json = '{}' WHERE id = ?1",
        "UPDATE proposals SET args_hash = repeat('b', 64) WHERE id = ?1",
        "UPDATE proposals SET tier = 1 WHERE id = ?1",
    ] {
        // `repeat` does not exist in SQLite: build the hash differently for that statement.
        let sql = sql.replace("repeat('b', 64)", &format!("'{}'", "b".repeat(64)));
        assert!(
            conn.execute(&sql, params![id.as_str()]).is_err(),
            "{sql} must be refused"
        );
    }
    assert_eq!(
        get_proposal(&conn, &a.org, &id)?.action_json.as_deref(),
        Some(ACTION)
    );
    Ok(())
}

#[test]
fn an_approved_proposal_runs_once_and_both_ends_are_audited() -> TestResult {
    let conn = open_in_memory()?;
    let t = populate(&conn, "alpha")?;
    let id = approved(&conn, &t)?;
    let BeginOutcome::Started {
        execution,
        proposal,
    } = begin_execution(&conn, &t.org, &id, &t.user)?
    else {
        return Err("expected Started".into());
    };
    assert_eq!(proposal.status, ProposalStatus::Approved);
    // While it runs, a second request cannot start another one.
    assert_eq!(
        begin_execution(&conn, &t.org, &id, &t.user)?,
        BeginOutcome::AlreadyRun
    );
    let running = get_execution_for_proposal(&conn, &t.org, &id)?.ok_or("missing row")?;
    assert_eq!(
        (running.status, running.finished_at.as_ref()),
        (ExecutionStatus::Running, None)
    );

    finish_execution(&conn, &t.org, &execution, true, true, "ok", "fine")?;
    let done = get_execution_for_proposal(&conn, &t.org, &id)?.ok_or("missing row")?;
    assert_eq!(
        (done.status, done.verified, done.output.as_str()),
        (ExecutionStatus::Succeeded, true, "fine")
    );
    assert!(done.finished_at.is_some());
    assert_eq!(
        get_proposal(&conn, &t.org, &id)?.status,
        ProposalStatus::Executed
    );
    // Finishing twice, or running again, is refused.
    assert!(matches!(
        finish_execution(&conn, &t.org, &execution, true, true, "ok", ""),
        Err(Error::NotFound)
    ));
    assert_eq!(
        begin_execution(&conn, &t.org, &id, &t.user)?,
        BeginOutcome::AlreadyRun
    );
    assert_eq!(count_audit(&conn, &t, "proposal.execute.start")?, 1);
    assert_eq!(count_audit(&conn, &t, "proposal.execute.finish")?, 1);
    assert!(matches!(
        verify_audit_chain(&conn, &t.org)?,
        ChainStatus::Intact { .. }
    ));
    Ok(())
}

#[test]
fn a_failed_execution_marks_the_proposal_failed_and_is_never_repeated() -> TestResult {
    let conn = open_in_memory()?;
    let t = populate(&conn, "alpha")?;
    let id = approved(&conn, &t)?;
    let BeginOutcome::Started { execution, .. } = begin_execution(&conn, &t.org, &id, &t.user)?
    else {
        return Err("expected Started".into());
    };
    let long = "x".repeat(10_000);
    finish_execution(
        &conn,
        &t.org,
        &execution,
        false,
        false,
        "command_failed",
        &long,
    )?;
    assert_eq!(
        get_proposal(&conn, &t.org, &id)?.status,
        ProposalStatus::Failed
    );
    let row = get_execution_for_proposal(&conn, &t.org, &id)?.ok_or("missing row")?;
    assert_eq!(row.output.chars().count(), MAX_OUTPUT_CHARS);
    assert_eq!(
        begin_execution(&conn, &t.org, &id, &t.user)?,
        BeginOutcome::AlreadyRun
    );
    Ok(())
}

#[test]
fn only_an_approved_unexpired_proposal_with_an_action_can_start() -> TestResult {
    let conn = open_in_memory()?;
    let t = populate(&conn, "alpha")?;
    let hash = sha256_hex(ACTION.as_bytes());

    // Pending, and rejected.
    let pending = create_action_proposal(&conn, &t.org, &new_proposal(&hash), &t.node, ACTION)?;
    assert_eq!(
        begin_execution(&conn, &t.org, &pending, &t.user)?,
        BeginOutcome::NotApproved(ProposalStatus::Pending)
    );
    decide_proposal(&conn, &t.org, &pending, ProposalDecision::Reject, &t.user)?;
    assert_eq!(
        begin_execution(&conn, &t.org, &pending, &t.user)?,
        BeginOutcome::NotApproved(ProposalStatus::Rejected)
    );

    // Approved but without an action.
    let plain = create_proposal(&conn, &t.org, &new_proposal(&hash))?;
    decide_proposal(&conn, &t.org, &plain, ProposalDecision::Approve, &t.user)?;
    assert_eq!(
        begin_execution(&conn, &t.org, &plain, &t.user)?,
        BeginOutcome::NoAction
    );

    // Approved, then left until it expired.
    let late = approved(&conn, &t)?;
    conn.execute(
        "UPDATE proposals SET expires_at = '2000-01-01T00:00:00.000Z' WHERE id = ?1",
        params![late.as_str()],
    )?;
    assert_eq!(
        begin_execution(&conn, &t.org, &late, &t.user)?,
        BeginOutcome::Expired
    );
    assert_eq!(
        get_proposal(&conn, &t.org, &late)?.status,
        ProposalStatus::Expired
    );
    assert!(get_execution_for_proposal(&conn, &t.org, &late)?.is_none());
    assert_eq!(count_audit(&conn, &t, "proposal.execute.start")?, 0);
    Ok(())
}

#[test]
fn another_org_cannot_start_or_see_an_execution() -> TestResult {
    let conn = open_in_memory()?;
    let a = populate(&conn, "alpha")?;
    let b = populate(&conn, "beta")?;
    let id = approved(&conn, &a)?;
    assert!(matches!(
        begin_execution(&conn, &b.org, &id, &b.user),
        Err(Error::NotFound)
    ));
    let BeginOutcome::Started { execution, .. } = begin_execution(&conn, &a.org, &id, &a.user)?
    else {
        return Err("expected Started".into());
    };
    assert!(get_execution_for_proposal(&conn, &b.org, &id)?.is_none());
    assert!(matches!(
        finish_execution(&conn, &b.org, &execution, true, true, "ok", ""),
        Err(Error::NotFound)
    ));
    Ok(())
}

#[test]
fn executions_left_running_are_failed_as_interrupted() -> TestResult {
    let conn = open_in_memory()?;
    let t = populate(&conn, "alpha")?;
    let id = approved(&conn, &t)?;
    let BeginOutcome::Started { .. } = begin_execution(&conn, &t.org, &id, &t.user)? else {
        return Err("expected Started".into());
    };
    assert_eq!(fail_interrupted_executions(&conn, &t.org)?, 1);
    assert_eq!(fail_interrupted_executions(&conn, &t.org)?, 0);
    let row = get_execution_for_proposal(&conn, &t.org, &id)?.ok_or("missing row")?;
    assert_eq!(
        (row.status, row.reason.as_deref()),
        (ExecutionStatus::Failed, Some("interrupted"))
    );
    assert_eq!(
        get_proposal(&conn, &t.org, &id)?.status,
        ProposalStatus::Failed
    );
    Ok(())
}
