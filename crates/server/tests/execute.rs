//! Running an approved proposal over the real HTTP surface, with a fake executor: nothing here
//! can touch a real service (ADR 0037).

mod common;

use actions::{Action, BackupSnapshot, DryRun, Executor, Outcome, OutcomeStatus};
use common::*;
use db::{NewNode, NewProposal, NodeAccess, NodeKind};
use serde_json::json;
use std::path::Path;
use std::sync::{Arc, Mutex};

const ACTION: &str =
    r#"{"action":"restart_service","manager":"systemd","service":"nginx.service"}"#;

/// Records what it was asked to run and answers as scripted.
struct Recording {
    ran: Mutex<Vec<String>>,
    rolled_back: Mutex<Vec<String>>,
    succeed: bool,
    backup_succeed: bool,
}

impl Executor for Recording {
    fn dry_run(&self, action: &Action) -> DryRun {
        match action {
            Action::UpdateEnvVar { key, path, .. } => DryRun {
                steps: vec![format!("update {} in {}", key.as_str(), path.as_str())],
                current_state: "present".to_owned(),
                diff: Some(format!(
                    "- {}=[REDACTED]\n+ {}=[REDACTED]",
                    key.as_str(),
                    key.as_str()
                )),
            },
            Action::DeleteManagedFile { path } => DryRun {
                steps: vec![format!("delete {}", path.as_str())],
                current_state: "present".to_owned(),
                diff: Some(format!("--- {}\n+++ /dev/null", path.as_str())),
            },
            Action::RestartService { name, .. } => DryRun {
                steps: vec![format!("systemctl restart {}", name.as_str())],
                current_state: "active".to_owned(),
                diff: None,
            },
        }
    }

    fn backup(&self, action: &Action, backup_dir: &Path) -> Result<BackupSnapshot, &'static str> {
        if !self.backup_succeed {
            return Err("io");
        }
        std::fs::create_dir_all(backup_dir).map_err(|_| "io")?;
        let file = backup_dir.join("rec.bak");
        let payload = action.canonical_json();
        std::fs::write(&file, payload.as_bytes()).map_err(|_| "io")?;
        Ok(BackupSnapshot {
            storage_ref: file.to_string_lossy().into_owned(),
            sha256: db::sha256_hex(payload.as_bytes()),
        })
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

    fn rollback(&self, action: &Action, _backup: &BackupSnapshot) -> Outcome {
        if let Ok(mut rb) = self.rolled_back.lock() {
            rb.push(action.canonical_json());
        }
        Outcome {
            status: if self.succeed {
                OutcomeStatus::Succeeded
            } else {
                OutcomeStatus::Failed
            },
            verified: self.succeed,
            reason: if self.succeed { "ok" } else { "command_failed" },
            detail: String::new(),
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
        rolled_back: Mutex::new(Vec::new()),
        succeed,
        backup_succeed: true,
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
    assert_eq!(list[0]["action"]["service"], "nginx.service");
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
    // A critical unit stored as tier 2 (a downgrade of its real tier) is refused, and so is one
    // stored with its real tier 3: critical units are not runnable at all yet.
    let crit = r#"{"action":"restart_service","manager":"systemd","service":"sshd.service"}"#;
    let downgraded = proposal(
        &s,
        &s.local_node,
        crit,
        &db::sha256_hex(crit.as_bytes()),
        true,
    )?;
    assert_eq!(run(&downgraded), 409);
    // Only a unit that is not a service can never even be stored by the tool, but a hand-made row
    // must not run either.
    let target = r#"{"action":"restart_service","manager":"systemd","service":"poweroff.target"}"#;
    let powered = proposal(
        &s,
        &s.local_node,
        target,
        &db::sha256_hex(target.as_bytes()),
        true,
    )?;
    assert_eq!(run(&powered), 409);
    // An unsupported kind/access combination.
    let unsupported_node = {
        let conn = s.world.conn()?;
        db::create_node(
            &conn,
            &s.world.org,
            &db::NewNode {
                name: "unsupported-target",
                kind: db::NodeKind::Local,
                access: db::NodeAccess::Ssh,
                labels_json: "{}",
            },
        )?
    };
    let far = proposal(&s, &unsupported_node, ACTION, &good_hash(), true)?;
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

#[test]
fn dry_run_tier3_backup_and_rollback_flow() -> TestResult {
    let s = setup(true, true)?;
    let session = s.server.sign_in()?;
    let env_action = Action::update_env_var("/etc/nodivara/.env", "API_KEY", "secret-xyz-999")?;
    let conn = s.world.conn()?;
    let id = db::create_action_proposal(
        &conn,
        &s.world.org,
        &NewProposal {
            conversation: None,
            tool: env_action.tool(),
            tier: env_action.tier().level(),
            title: "Update API_KEY",
            rationale: "Rotate key",
            suggested_action: "Update .env",
            impact: "assistant.proposal.impact",
            rule_id: None,
            subject_key: None,
            args_hash: &env_action.hash(),
        },
        &s.local_node,
        &env_action.canonical_json(),
    )?;
    drop(conn);

    // 1. Dry-run works while pending and redacts secret values.
    let dry = session.post_empty(&format!("/api/proposals/{}/dry-run", id.as_str()));
    assert_eq!(dry.status, 200, "{}", dry.text());
    let dry_body = dry.json();
    assert!(!dry.text().contains("secret-xyz-999"));
    assert_eq!(dry_body["current_state"], "present");
    assert!(
        dry_body["diff"]
            .as_str()
            .unwrap_or("")
            .contains("[REDACTED]")
    );

    // 2. Rollback before execution is refused.
    let early_rb = session.post_empty(&format!("/api/proposals/{}/rollback", id.as_str()));
    assert_eq!(early_rb.status, 409);
    assert_eq!(early_rb.json()["error"]["code"], "proposal_not_executed");

    // 3. Approve, execute (takes backup first), then list shows dry_run, approval, has_backup.
    let dec = session.post(
        &format!("/api/proposals/{}/decision", id.as_str()),
        &json!({"decision": "approve"}),
    );
    assert_eq!(dec.status, 200);

    let ex = session.post_empty(&format!("/api/proposals/{}/execute", id.as_str()));
    assert_eq!(ex.status, 200, "{}", ex.text());
    let ex_body = ex.json();
    assert_eq!(ex_body["status"], "executed");
    assert_eq!(ex_body["has_backup"], true);
    assert_eq!(ran(&s), 1);

    // 4. Rollback succeeds once, then second rollback is refused.
    let rb = session.post_empty(&format!("/api/proposals/{}/rollback", id.as_str()));
    assert_eq!(rb.status, 200, "{}", rb.text());
    assert_eq!(rb.json()["rollback"]["status"], "succeeded");
    assert_eq!(rb.json()["rollback"]["verified"], true);

    let rb2 = session.post_empty(&format!("/api/proposals/{}/rollback", id.as_str()));
    assert_eq!(rb2.status, 409);
    assert_eq!(rb2.json()["error"]["code"], "already_rolled_back");

    // 5. GET /api/proposals exposes dry_run, approval, has_backup, rollback, and redacted action.
    let list = session.get("/api/proposals");
    assert_eq!(list.status, 200);
    assert!(!list.text().contains("secret-xyz-999"));
    let item = &list.json()[0];
    assert_eq!(item["action"]["value"], "[REDACTED]");
    assert!(item["dry_run"].is_object());
    assert_eq!(item["approval"]["decision"], "approved");
    assert_eq!(item["has_backup"], true);
    assert_eq!(item["rollback"]["status"], "succeeded");
    Ok(())
}

#[test]
fn tier4_requires_typed_confirmation_and_backup_failure_aborts_execute() -> TestResult {
    let world = World::new()?;
    let conn = world.conn()?;
    let local_node = db::create_node(
        &conn,
        &world.org,
        &NewNode {
            name: "this-mac",
            kind: NodeKind::Local,
            access: NodeAccess::Local,
            labels_json: "{}",
        },
    )?;
    let del_action = Action::delete_managed_file("/etc/nodivara/old.tar")?;
    let id = db::create_action_proposal(
        &conn,
        &world.org,
        &NewProposal {
            conversation: None,
            tool: del_action.tool(),
            tier: 4,
            title: "Delete old.tar",
            rationale: "Reclaim space",
            suggested_action: "Delete file",
            impact: "assistant.proposal.impact",
            rule_id: None,
            subject_key: None,
            args_hash: &del_action.hash(),
        },
        &local_node,
        &del_action.canonical_json(),
    )?;
    drop(conn);

    // Executor whose backup fails.
    let executor = Arc::new(Recording {
        ran: Mutex::new(Vec::new()),
        rolled_back: Mutex::new(Vec::new()),
        succeed: true,
        backup_succeed: false,
    });
    let server = TestServer::start_with(
        &world,
        Opts {
            executor: Some(Arc::clone(&executor) as Arc<dyn Executor>),
            ..Opts::default()
        },
    )?;
    let session = server.sign_in()?;

    // Missing or wrong typed_confirmation is 400 typed_confirmation_mismatch.
    let bad1 = session.post(
        &format!("/api/proposals/{}/decision", id.as_str()),
        &json!({"decision": "approve"}),
    );
    assert_eq!(bad1.status, 400);
    assert_eq!(bad1.json()["error"]["code"], "typed_confirmation_mismatch");

    let bad2 = session.post(
        &format!("/api/proposals/{}/decision", id.as_str()),
        &json!({"decision": "approve", "typed_confirmation": "wrong"}),
    );
    assert_eq!(bad2.status, 400);

    // Exact target_label succeeds.
    let ok = session.post(
        &format!("/api/proposals/{}/decision", id.as_str()),
        &json!({"decision": "approve", "typed_confirmation": "/etc/nodivara/old.tar"}),
    );
    assert_eq!(ok.status, 200);

    // Execute: backup fails -> execute() is NEVER called, proposal becomes failed.
    let ex = session.post_empty(&format!("/api/proposals/{}/execute", id.as_str()));
    assert_eq!(ex.status, 200);
    assert_eq!(ex.json()["status"], "failed");
    assert_eq!(ex.json()["has_backup"], false);
    assert_eq!(executor.ran.lock().map(|g| g.len()).unwrap_or(99), 0);
    Ok(())
}

#[test]
fn plan_approve_and_run_respects_gates_and_updates_step_statuses() -> TestResult {
    let s = setup(true, true)?;
    let session = s.server.sign_in()?;
    let mut conn = s.world.conn()?;
    // Create a minimal valid draft plan using agent::draft_migration_plan.
    let tid = db::create_transport(
        &conn,
        &s.world.org,
        &db::NewTransport {
            name: "t-plan",
            kind: "rsync",
            config_json: r#"{"host":"example.test","user":"u","port":22,"path":"/srv"}"#,
            secret: None,
        },
    )?;
    db::record_transport_check(&conn, &s.world.org, &tid, r#"{"listing_ok":true}"#, true)?;
    db::save_inventory(
        &mut conn,
        &s.world.org,
        &db::NewInventory {
            node: &s.local_node,
            status: db::InventoryStatus::Complete,
            taken_at: "2026-11-27T00:00:00Z",
            collector_version: "0.1.0",
            os: "linux",
            arch: "x86_64",
            warnings_json: "[]",
        },
        &[],
    )?;
    db::save_inventory(
        &mut conn,
        &s.world.org,
        &db::NewInventory {
            node: &s.remote_node,
            status: db::InventoryStatus::Complete,
            taken_at: "2026-11-27T00:00:00Z",
            collector_version: "0.1.0",
            os: "linux",
            arch: "x86_64",
            warnings_json: "[]",
        },
        &[],
    )?;
    let catalog = i18n::Catalog::builtin()?;
    let (store, plans, sink) = (
        agent::DbStore::new(&conn),
        agent::DbPlanStore::new(&conn),
        agent::DbPlanSink::new(&conn),
    );
    let ctx = agent::DraftContext {
        org: &s.world.org,
        store: &store,
        plans: &plans,
        sink: &sink,
        catalog: &catalog,
        locale: i18n::Locale::En,
    };
    let drafted = agent::draft_migration_plan(
        &ctx,
        &agent::DraftRequest {
            source: "this-mac",
            target: "far-away",
            transport: Some("t-plan"),
            name: Some("sprint2-plan"),
            ..agent::DraftRequest::default()
        },
    )
    .map_err(|e| format!("{e:?}"))?;
    let plan_id = db::find_plan_by_name(&conn, &s.world.org, &drafted.name)?.id;
    drop(conn);

    // Running before approval is 409 plan_not_approved.
    let pre = session.post_empty(&format!("/api/plans/{}/run", plan_id.as_str()));
    assert_eq!(pre.status, 409);
    assert_eq!(pre.json()["error"]["code"], "plan_not_approved");

    // Approve: draft -> approved.
    let app = session.post_empty(&format!("/api/plans/{}/approve", plan_id.as_str()));
    assert_eq!(app.status, 200, "{}", app.text());
    assert_eq!(app.json()["status"], "approved");

    // Second approve is 409 plan_not_approvable.
    let app2 = session.post_empty(&format!("/api/plans/{}/approve", plan_id.as_str()));
    assert_eq!(app2.status, 409);

    // Run: hits cutover gate because no comparison exists -> 409 plan_gate_blocked, plan paused,
    // steps 1..=6 succeeded!
    let run = session.post_empty(&format!("/api/plans/{}/run", plan_id.as_str()));
    assert_eq!(run.status, 409);
    assert_eq!(run.json()["error"]["code"], "plan_gate_blocked");

    let detail = session.get(&format!("/api/plans/{}", plan_id.as_str()));
    assert_eq!(detail.status, 200);
    let body = detail.json();
    assert_eq!(body["status"], "paused");
    assert_eq!(body["steps"][0]["status"], "succeeded");
    assert_eq!(body["steps"][6]["status"], "succeeded");
    assert_eq!(body["steps"][7]["status"], "pending");
    assert_eq!(body["steps"][9]["status"], "pending");
    Ok(())
}

#[test]
fn transport_credentials_update_seals_in_vault_resets_verified_and_audits() -> TestResult {
    let world = World::new()?;
    let conn = world.conn()?;
    let tid = db::create_transport(
        &conn,
        &world.org,
        &db::NewTransport {
            name: "s3-rot",
            kind: "rclone",
            config_json: r#"{"remote_kind":"s3","remote":"r","path":"b","options":{"endpoint":"s3.example.test"}}"#,
            secret: None,
        },
    )?;
    db::record_transport_check(&conn, &world.org, &tid, r#"{"listing_ok":true}"#, true)?;
    drop(conn);

    let server = TestServer::start_with(
        &world,
        Opts {
            vault: Some(Arc::new(vault::Vault::new(
                "k1:00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff",
                &[],
            )?)),
            ..Opts::default()
        },
    )?;
    let session = server.sign_in()?;
    let r = session.post(
        &format!("/api/transports/{}/credentials", tid.as_str()),
        &json!({"credentials": {"access_key_id": "AKIA-NEW", "secret_access_key": "SUPER-SECRET-999"}}),
    );
    assert_eq!(r.status, 200, "{}", r.text());
    assert!(!r.text().contains("SUPER-SECRET-999"));
    assert!(r.json()["verified_at"].is_null());

    let conn = world.conn()?;
    let row = db::get_transport(&conn, &world.org, &tid)?;
    assert!(row.verified_at.is_none());
    assert!(row.secret_id.is_some());
    assert!(
        db::list_audit(&conn, &world.org, 50)?
            .iter()
            .any(|e| e.action == "transport.credentials.update")
    );
    Ok(())
}

#[test]
fn monitors_incidents_channels_and_telegram_link_code_api() -> TestResult {
    let world = World::new()?;
    let server = TestServer::start_with(
        &world,
        Opts {
            vault: Some(Arc::new(vault::Vault::new(
                "k1:00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff",
                &[],
            )?)),
            ..Opts::default()
        },
    )?;
    let session = server.sign_in()?;

    // 1. Monitors CRUD
    let cm = session.post(
        "/api/monitors",
        &json!({"name": "api", "kind": "http", "target": "https://example.test/health"}),
    );
    assert_eq!(cm.status, 201, "{}", cm.text());
    let mid = cm.json()["id"]
        .as_str()
        .ok_or("missing monitor id")?
        .to_owned();
    assert_eq!(
        session.get("/api/monitors").json().as_array().map(Vec::len),
        Some(1)
    );

    // 2. Incidents list & ack
    let conn = world.conn()?;
    let inc_id = db::open_incident(
        &conn,
        &world.org,
        &db::MonitorId::parse(&mid)?,
        "timeout",
        db::AlertSeverity::Critical,
    )?;
    drop(conn);
    assert_eq!(
        session
            .get("/api/incidents?open=true")
            .json()
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    let ack = session.post_empty(&format!("/api/incidents/{}/ack", inc_id.as_str()));
    assert_eq!(ack.status, 200);
    assert_eq!(ack.json()["acknowledged"], true);

    // 3. Channels CRUD (token sealed in vault, never returned)
    let bot_tok = "123456789:SECRET-BOT-TOKEN-XYZ";
    let cc = session.post(
        "/api/channels",
        &json!({
            "kind": "telegram",
            "name": "ops-tg",
            "config": {"chat_id": "12345"},
            "token": bot_tok,
        }),
    );
    assert_eq!(cc.status, 201, "{}", cc.text());
    assert!(!cc.text().contains(bot_tok));
    assert_eq!(cc.json()["has_secret"], true);
    let ch_id = cc.json()["id"]
        .as_str()
        .ok_or("missing channel id")?
        .to_owned();
    let ch_list = session.get("/api/channels");
    assert!(!ch_list.text().contains(bot_tok));

    // 4. Telegram link code
    let lc = session.post_empty("/api/channels/telegram/link-code");
    assert_eq!(lc.status, 201, "{}", lc.text());
    assert!(lc.json()["code"].as_str().is_some());
    assert!(
        lc.json()["command"]
            .as_str()
            .unwrap_or("")
            .starts_with("/link ")
    );

    // 5. Delete monitor and channel
    assert_eq!(session.delete(&format!("/api/monitors/{mid}")).status, 204);
    assert_eq!(
        session.delete(&format!("/api/channels/{ch_id}")).status,
        204
    );
    Ok(())
}
