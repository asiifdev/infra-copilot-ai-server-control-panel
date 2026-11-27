//! Integration tests for Sprint 6: remote SSH executor, pull-only agent tokens/jobs, bounded
//! service log viewer, and `.test` local site management.

mod common;

use actions::{Action, LocalExecutor, ServiceManagerKind};
use common::{Opts, Req, Scratch, TestResult, TestServer, World};
use db::{
    NewNode, NewProposal, NodeAccess, NodeKind, ProposalDecision, create_action_proposal,
    create_node, decide_proposal,
};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use transport::{CommandPlan, RunError, RunOutput, Runner};

struct FakeRunner {
    replies: Mutex<VecDeque<Result<RunOutput, RunError>>>,
}

impl FakeRunner {
    fn with_replies(replies: Vec<Result<RunOutput, RunError>>) -> Self {
        Self {
            replies: Mutex::new(replies.into()),
        }
    }
}

impl Runner for FakeRunner {
    fn run(&self, _plan: &CommandPlan) -> Result<RunOutput, RunError> {
        self.replies
            .lock()
            .ok()
            .and_then(|mut q| q.pop_front())
            .unwrap_or(Err(RunError::Io))
    }
}

fn ok_out(stdout: &str) -> Result<RunOutput, RunError> {
    Ok(RunOutput {
        exit_code: Some(0),
        stdout: stdout.as_bytes().to_vec(),
        stderr: Vec::new(),
        truncated: false,
        elapsed: Duration::from_millis(2),
    })
}

#[test]
fn agent_token_poll_result_and_ssh_execution_work_end_to_end() -> TestResult {
    let world = World::new()?;
    let conn = world.conn()?;
    let agent_node = create_node(
        &conn,
        &world.org,
        &NewNode {
            name: "remote-agent",
            kind: NodeKind::Vps,
            access: NodeAccess::Agent,
            labels_json: "{}",
        },
    )?;
    let ssh_node = create_node(
        &conn,
        &world.org,
        &NewNode {
            name: "remote-ssh",
            kind: NodeKind::Vps,
            access: NodeAccess::Ssh,
            labels_json: r#"{"ssh_host":"203.0.113.50","ssh_port":22,"ssh_user":"deploy"}"#,
        },
    )?;

    let action = Action::restart_service(ServiceManagerKind::Systemd, "nginx")?;
    let action_json = action.canonical_json();
    let args_hash = action.hash();

    let agent_prop = create_action_proposal(
        &conn,
        &world.org,
        &NewProposal {
            conversation: None,
            tool: action.tool(),
            tier: action.tier().level(),
            title: "Restart nginx on agent",
            rationale: "test",
            suggested_action: "restart nginx",
            impact: "brief downtime",
            rule_id: None,
            subject_key: None,
            args_hash: &args_hash,
        },
        &agent_node,
        &action_json,
    )?;
    decide_proposal(
        &conn,
        &world.org,
        &agent_prop,
        ProposalDecision::Approve,
        &world.user,
    )?;

    let ssh_prop = create_action_proposal(
        &conn,
        &world.org,
        &NewProposal {
            conversation: None,
            tool: action.tool(),
            tier: action.tier().level(),
            title: "Restart nginx on ssh",
            rationale: "test",
            suggested_action: "restart nginx",
            impact: "brief downtime",
            rule_id: None,
            subject_key: None,
            args_hash: &args_hash,
        },
        &ssh_node,
        &action_json,
    )?;
    decide_proposal(
        &conn,
        &world.org,
        &ssh_prop,
        ProposalDecision::Approve,
        &world.user,
    )?;
    drop(conn);

    let ssh_runner = Arc::new(FakeRunner::with_replies(vec![
        ok_out(""),         // ssh execute: systemctl restart
        ok_out("active\n"), // ssh execute: systemctl is-active
    ]));
    let local_exec = Arc::new(LocalExecutor::new(FakeRunner::with_replies(Vec::new())));
    let server = TestServer::start_with(
        &world,
        Opts {
            executor: Some(local_exec),
            ssh_runner: Some(ssh_runner),
            ..Opts::default()
        },
    )?;
    let session = server.sign_in()?;

    // 1. Rotate agent token and verify token length.
    let rot = session.post_empty(&format!("/api/nodes/{}/agent-token", agent_node.as_str()));
    assert_eq!(rot.status, 200);
    let rot_json = rot.json();
    let token = rot_json["token"].as_str().ok_or("missing token")?;
    assert_eq!(token.len(), 64);

    // 2. Unauthenticated poll fails with 401.
    let unauth_poll = Req::post("/api/agents/poll")
        .json(&json!({}))
        .send(server.addr);
    assert_eq!(unauth_poll.status, 401);

    // 3. Execute agent proposal -> queued!
    let exec_agent = session.post_empty(&format!("/api/proposals/{}/execute", agent_prop.as_str()));
    assert_eq!(exec_agent.status, 200);
    let exec_agent_json = exec_agent.json();
    assert_eq!(exec_agent_json["execution"]["status"], "queued");
    assert_eq!(exec_agent_json["agent_job"]["status"], "queued");

    // 4. Agent polls with Bearer token and claims the job atomically.
    let auth_hdr = format!("Bearer {token}");
    let poll = Req::post("/api/agents/poll")
        .header("Authorization", &auth_hdr)
        .json(&json!({}))
        .send(server.addr);
    assert_eq!(poll.status, 200);
    let poll_json = poll.json();
    let job_id = poll_json["job"]["id"].as_str().ok_or("missing job id")?;
    assert_eq!(poll_json["job"]["status"], "running");

    // Second poll sees nothing.
    let poll2 = Req::post("/api/agents/poll")
        .header("Authorization", &auth_hdr)
        .json(&json!({}))
        .send(server.addr);
    assert_eq!(poll2.status, 200);
    assert_eq!(poll2.json()["job"], Value::Null);

    // 5. Agent reports result -> updates job & execution.
    let res = Req::post("/api/agents/result")
        .header("Authorization", &auth_hdr)
        .json(&json!({
            "job_id": job_id,
            "succeeded": true,
            "verified": true,
            "reason": "ok",
            "output": "restarted"
        }))
        .send(server.addr);
    assert_eq!(res.status, 200);
    let res_json = res.json();
    assert_eq!(res_json["status"], "succeeded");
    assert_eq!(res_json["verified"], true);

    // 6. Execute SSH proposal -> runs via SshExecutor + FakeRunner.
    let exec_ssh = session.post_empty(&format!("/api/proposals/{}/execute", ssh_prop.as_str()));
    assert_eq!(exec_ssh.status, 200);
    let exec_ssh_json = exec_ssh.json();
    assert_eq!(exec_ssh_json["execution"]["status"], "succeeded");
    assert_eq!(exec_ssh_json["execution"]["verified"], true);
    Ok(())
}

#[test]
fn node_logs_and_test_sites_endpoints_enforce_bounds_and_validation() -> TestResult {
    let world = World::new()?;
    let conn = world.conn()?;
    let node_id = create_node(
        &conn,
        &world.org,
        &NewNode {
            name: "app-box",
            kind: NodeKind::Local,
            access: NodeAccess::Local,
            labels_json: "{}",
        },
    )?;
    drop(conn);

    let logs_dir = Scratch::new("logs")?;
    let sites_dir = Scratch::new("sites")?;
    std::fs::write(
        logs_dir.0.join("nginx.service.log"),
        b"line1\nPASSWORD=supersecretpassword\nline3\n",
    )?;

    let server = TestServer::start_with(
        &world,
        Opts {
            logs_dir: Some(logs_dir.0.clone()),
            sites_dir: Some(sites_dir.0.clone()),
            ..Opts::default()
        },
    )?;
    let session = server.sign_in()?;

    // Logs: valid service redacts secrets.
    let logs_resp = session.get(&format!(
        "/api/nodes/{}/logs?service=nginx&lines=10",
        node_id.as_str()
    ));
    assert_eq!(logs_resp.status, 200);
    let logs_json = logs_resp.json();
    assert_eq!(logs_json["service"], "nginx.service");
    let content = logs_json["content"].as_str().ok_or("missing content")?;
    assert!(content.contains("line1"));
    assert!(!content.contains("supersecretpassword"));

    // Logs: bad service name refused.
    let bad_logs = session.get(&format!(
        "/api/nodes/{}/logs?service=reboot.target",
        node_id.as_str()
    ));
    assert_eq!(bad_logs.status, 400);

    // Sites: non-.test domain refused.
    let bad_site = session.post(
        "/api/sites",
        &json!({"domain": "example.com", "upstream_port": 3000}),
    );
    assert_eq!(bad_site.status, 400);

    // Sites: valid .test domain creates config inside managed sites_dir and lists it.
    let ok_site = session.post(
        "/api/sites",
        &json!({"domain": "shop.test", "upstream_port": 4000}),
    );
    assert_eq!(ok_site.status, 201);
    let ok_json = ok_site.json();
    assert_eq!(ok_json["site"]["domain"], "shop.test");
    assert_eq!(ok_json["site"]["status"], "active");
    assert!(sites_dir.0.join("shop.test.conf").exists());

    let list = session.get("/api/sites");
    assert_eq!(list.status, 200);
    assert_eq!(list.json()["sites"].as_array().map(Vec::len), Some(1));
    Ok(())
}
