//! Running an approved proposal over the real HTTP surface, with a fake executor: nothing here
//! can touch a real service (ADR 0037).

mod common;

use actions::{Action, DryRun, Executor, Outcome, OutcomeStatus};
use common::*;
use db::{NewNode, NewProposal, NodeAccess, NodeKind};
use serde_json::json;
use std::sync::{Arc, Mutex};

const ACTION: &str = r#"{"action":"restart_service","manager":"systemd","service":"nginx"}"#;

/// Records what it was asked to run and answers as scripted.
struct Recording {
    ran: Mutex<Vec<String>>,
    succeed: bool,
}

impl Executor for Recording {
    fn dry_run(&self, _action: &Action) -> DryRun {
        DryRun {
            steps: Vec::new(),
            current_state: String::new(),
        }
    }

    fn execute(&self, action: &Action) -> Outcome {
        if let Ok(mut ran) = self.ran.lock() {
            ran.push(action.canonical_json());
        }
        Outcome {
            status: if self.succeed {
                OutcomeStatus::Succeeded
            } else {
                OutcomeStatus::Failed
            },
            verified: self.succeed,
            reason: if self.succeed { "ok" } else { "command_failed" },
            detail: if self.succeed {
                String::new()
            } else {
                "boom\u{7}".to_owned()
            },
        }
    }
}

struct Setup {
    world: World,
    server: TestServer,
    executor: Arc<Recording>,
    local_node: db::NodeId,
    remote_node: db::NodeId,
}

fn setup(succeed: bool, enabled: bool) -> Result<Setup, Box<dyn std::error::Error>> {
    let world = World::new()?;
    let conn = world.conn()?;
    let make = |name: &str, kind, access| {
        db::create_node(
            &conn,
            &world.org,
            &NewNode {
                name,
                kind,
                access,
                labels_json: "{}",
            },
        )
    };
    let local_node = make("this-mac", NodeKind::Local, NodeAccess::Local)?;
    let remote_node = make("far-away", NodeKind::Vps, NodeAccess::Ssh)?;
    drop(conn);
    let executor = Arc::new(Recording {
        ran: Mutex::new(Vec::new()),
        succeed,
    });
    let server = TestServer::start_with(
        &world,
        Opts {
            executor: enabled.then(|| Arc::clone(&executor) as Arc<dyn Executor>),
            ..Opts::default()
        },
    )?;
    Ok(Setup {
        world,
        server,
        executor,
        local_node,
        remote_node,
    })
}

fn proposal(
    s: &Setup,
    node: &db::NodeId,
    action: &str,
    hash: &str,
    approve: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    let conn = s.world.conn()?;
    let id = db::create_action_proposal(
        &conn,
        &s.world.org,
        &NewProposal {
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
        },
        node,
        action,
    )?;
    if approve {
        db::decide_proposal(
            &conn,
            &s.world.org,
            &id,
            db::ProposalDecision::Approve,
            &s.world.user,
        )?;
    }
    Ok(id.as_str().to_owned())
}

fn good_hash() -> String {
    db::sha256_hex(ACTION.as_bytes())
}

fn ran(s: &Setup) -> usize {
    s.executor.ran.lock().map(|r| r.len()).unwrap_or(usize::MAX)
}

#[test]
fn an_approved_action_runs_once_and_the_result_is_listed() -> TestResult {
    let s = setup(true, true)?;
    let session = s.server.sign_in()?;
    assert_eq!(
        session.get("/api/session").json()["execution_enabled"],
        true
    );
    let id = proposal(&s, &s.local_node, ACTION, &good_hash(), true)?;
    let r = session.post_empty(&format!("/api/proposals/{id}/execute"));
    assert_eq!(r.status, 200, "{}", r.text());
    let body = r.json();
    assert_eq!(body["status"], "executed");
    assert_eq!(body["execution"]["status"], "succeeded");
    assert_eq!(body["execution"]["verified"], true);
    assert_eq!(ran(&s), 1);
    // The list shows the action, the node and the execution.
    let list = session.get("/api/proposals?status=executed").json();
    assert_eq!(list[0]["action"]["service"], "nginx");
    assert_eq!(list[0]["node"], "this-mac");
    assert_eq!(list[0]["execution"]["status"], "succeeded");
    // The impact is a catalog key in the database and a sentence in the reader's language here.
    assert!(
        list[0]["impact"]
            .as_str()
            .unwrap_or_default()
            .starts_with("Saves a pending")
    );
    let id_list = session.get("/api/proposals?status=executed&lang=id").json();
    assert!(
        id_list[0]["impact"]
            .as_str()
            .unwrap_or_default()
            .starts_with("Menyimpan usulan")
    );
    // A second run is refused and runs nothing.
    let again = session.post_empty(&format!("/api/proposals/{id}/execute"));
    assert_eq!(again.status, 409);
    assert_eq!(ran(&s), 1);
    Ok(())
}

#[test]
fn a_failed_run_is_recorded_with_cleaned_output_and_never_repeated() -> TestResult {
    let s = setup(false, true)?;
    let session = s.server.sign_in()?;
    let id = proposal(&s, &s.local_node, ACTION, &good_hash(), true)?;
    let body = session
        .post_empty(&format!("/api/proposals/{id}/execute"))
        .json();
    assert_eq!(body["status"], "failed");
    assert_eq!(body["execution"]["reason"], "command_failed");
    assert!(
        !body["execution"]["output"]
            .as_str()
            .unwrap_or_default()
            .contains('\u{7}')
    );
    assert_eq!(
        session
            .post_empty(&format!("/api/proposals/{id}/execute"))
            .status,
        409
    );
    assert_eq!(ran(&s), 1);
    Ok(())
}

#[test]
fn nothing_runs_unless_everything_is_in_order() -> TestResult {
    let s = setup(true, true)?;
    let session = s.server.sign_in()?;
    let run = |id: &str| {
        session
            .post_empty(&format!("/api/proposals/{id}/execute"))
            .status
    };
    // Pending (not approved).
    let pending = proposal(&s, &s.local_node, ACTION, &good_hash(), false)?;
    assert_eq!(run(&pending), 409);
    // The hash does not match the action: the approval no longer binds to it.
    let swapped = proposal(&s, &s.local_node, ACTION, &"a".repeat(64), true)?;
    assert_eq!(run(&swapped), 409);
    // A stored action that is not on the list.
    let evil = r#"{"action":"drop_database","name":"prod"}"#;
    let unknown = proposal(
        &s,
        &s.local_node,
        evil,
        &db::sha256_hex(evil.as_bytes()),
        true,
    )?;
    assert_eq!(run(&unknown), 409);
    // A target that is not this machine.
    let far = proposal(&s, &s.remote_node, ACTION, &good_hash(), true)?;
    assert_eq!(run(&far), 409);
    // Malformed and unknown ids.
    assert_eq!(run("not-a-uuid"), 404);
    assert_eq!(run("00000000-0000-4000-8000-000000000000"), 404);
    assert_eq!(ran(&s), 0);
    // None of the refused ones claimed an execution.
    let claimed: i64 = s
        .world
        .conn()?
        .query_row("SELECT COUNT(*) FROM executions", [], |r| r.get(0))?;
    assert_eq!(claimed, 0);
    Ok(())
}

#[test]
fn execution_is_off_unless_the_server_was_started_with_it() -> TestResult {
    let s = setup(true, false)?;
    let session = s.server.sign_in()?;
    assert_eq!(
        session.get("/api/session").json()["execution_enabled"],
        false
    );
    let id = proposal(&s, &s.local_node, ACTION, &good_hash(), true)?;
    let r = session.post_empty(&format!("/api/proposals/{id}/execute"));
    assert_eq!(r.status, 409);
    assert_eq!(r.json()["error"]["code"], json!("execution_disabled"));
    assert_eq!(ran(&s), 0);
    Ok(())
}

#[test]
fn another_organization_cannot_run_a_proposal() -> TestResult {
    let s = setup(true, true)?;
    let session = s.server.sign_in()?;
    // A proposal that belongs to the rival org, approved there.
    let conn = s.world.conn()?;
    let rival_node = db::create_node(
        &conn,
        &s.world.rival,
        &NewNode {
            name: "rival-box",
            kind: NodeKind::Local,
            access: NodeAccess::Local,
            labels_json: "{}",
        },
    )?;
    let id = db::create_action_proposal(
        &conn,
        &s.world.rival,
        &NewProposal {
            conversation: None,
            tool: "propose_restart_service",
            tier: 2,
            title: "Restart",
            rationale: "r",
            suggested_action: "a",
            impact: "assistant.proposal.impact",
            rule_id: None,
            subject_key: None,
            args_hash: &good_hash(),
        },
        &rival_node,
        ACTION,
    )?;
    db::decide_proposal(
        &conn,
        &s.world.rival,
        &id,
        db::ProposalDecision::Approve,
        &s.world.rival_user,
    )?;
    let r = session.post_empty(&format!("/api/proposals/{}/execute", id.as_str()));
    assert_eq!(r.status, 404);
    assert_eq!(ran(&s), 0);
    Ok(())
}
