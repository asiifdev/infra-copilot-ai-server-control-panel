//! Running, dry-running, and rolling back an approved proposal (ADR 0037).
//!
//! Everything is checked before anything starts: execution is switched on, the proposal is
//! approved and unexpired, its stored action is valid, its hash and tier are those the approval
//! bound to, and the target is this machine. For tier 3–4, an approval record is verified (with
//! `typed_confirmation == true` for tier 4) and a pre-execution backup is taken and recorded
//! before any mutation runs. Nothing a model can call reaches these handlers.

use super::{Reply, json_ok, with_db};
use crate::app::App;
use crate::errors::{ApiError, NOT_FOUND, from_db};
use crate::util::clean_text;
use actions::{Action, BackupSnapshot, Executor, OutcomeStatus, SshExecutor, SshTarget};
use axum::extract::{Path, State};
use db::{
    AgentJobStatus, BeginOutcome, BeginRollbackOutcome, Connection, Node, NodeAccess, NodeKind,
    ProposalDecision, ProposalId, ProposalRow,
};
use serde_json::{Value, json};
use std::sync::Arc;

const MAX_DRY_RUN_JSON_BYTES: usize = 8000;

/// The structured action of a proposal as the UI shows it, or `Null` when it has none or the
/// stored text is not a valid action. Secret values (such as `.env` variable values) are always
/// redacted.
pub(crate) fn action_view(row: &ProposalRow) -> Value {
    let Some(json) = row.action_json.as_deref() else {
        return Value::Null;
    };
    match Action::parse(json) {
        Ok(Action::RestartService { manager, name }) => json!({
            "kind": "restart_service",
            "manager": manager.as_str(),
            "service": clean_text(name.as_str()),
            "target_label": clean_text(name.as_str()),
        }),
        Ok(Action::UpdateEnvVar { path, key, .. }) => json!({
            "kind": "update_env_var",
            "path": clean_text(path.as_str()),
            "key": clean_text(key.as_str()),
            "value": "[REDACTED]",
            "target_label": clean_text(path.as_str()),
        }),
        Ok(Action::DeleteManagedFile { path }) => json!({
            "kind": "delete_managed_file",
            "path": clean_text(path.as_str()),
            "target_label": clean_text(path.as_str()),
        }),
        Err(_) => Value::Null,
    }
}

/// The cached dry-run of a proposal as the UI shows it, or `Null`.
pub(crate) fn dry_run_view(row: &ProposalRow) -> Value {
    let Some(raw) = row.dry_run_json.as_deref() else {
        return Value::Null;
    };
    let Ok(mut parsed) = serde_json::from_str::<Value>(raw) else {
        return Value::Null;
    };
    if let Some(obj) = parsed.as_object_mut() {
        obj.insert(
            "dry_run_at".to_owned(),
            row.dry_run_at
                .as_deref()
                .map_or(Value::Null, |t| Value::String(t.to_owned())),
        );
    }
    parsed
}

/// The stored approval decision of a proposal as the UI shows it, or `Null`.
pub(crate) fn approval_view(conn: &Connection, app: &App, row: &ProposalRow) -> Value {
    match db::get_approval_for_proposal(conn, &app.scope.org, &row.id) {
        Ok(Some(a)) => json!({
            "decision": match a.decision {
                ProposalDecision::Approve => "approved",
                ProposalDecision::Reject => "rejected",
            },
            "typed_confirmation": a.typed_confirmation,
            "created_at": a.created_at,
        }),
        _ => Value::Null,
    }
}

/// The execution of a proposal as the UI shows it, or `Null`. The output is text from a server:
/// redacted by the executor, and control characters are replaced again here.
pub(crate) fn execution_view(conn: &Connection, app: &App, row: &ProposalRow) -> Value {
    match db::get_execution_for_proposal(conn, &app.scope.org, &row.id) {
        Ok(Some(e)) => {
            let status = if e.status == db::ExecutionStatus::Running {
                match db::get_agent_job_for_execution(conn, &app.scope.org, &e.id) {
                    Ok(Some(job)) if job.status == AgentJobStatus::Queued => "queued",
                    _ => e.status.as_str(),
                }
            } else {
                e.status.as_str()
            };
            json!({
                "status": status,
                "verified": e.verified,
                "reason": e.reason,
                "output": clean_text(&e.output),
                "started_at": e.started_at,
                "finished_at": e.finished_at,
            })
        }
        _ => Value::Null,
    }
}

/// Whether the proposal's execution has a recorded backup snapshot.
pub(crate) fn has_backup_view(conn: &Connection, app: &App, row: &ProposalRow) -> bool {
    let Ok(Some(exec)) = db::get_execution_for_proposal(conn, &app.scope.org, &row.id) else {
        return false;
    };
    matches!(
        db::get_execution_backup(conn, &app.scope.org, &exec.id),
        Ok(Some(_))
    )
}

/// The rollback of a proposal's execution as the UI shows it, or `Null`.
pub(crate) fn rollback_view(conn: &Connection, app: &App, row: &ProposalRow) -> Value {
    let Ok(Some(exec)) = db::get_execution_for_proposal(conn, &app.scope.org, &row.id) else {
        return Value::Null;
    };
    match db::get_rollback_for_execution(conn, &app.scope.org, &exec.id) {
        Ok(Some(r)) => json!({
            "status": r.status.as_str(),
            "verified": r.verified,
            "reason": r.reason,
            "output": clean_text(&r.output),
            "started_at": r.started_at,
            "finished_at": r.finished_at,
        }),
        _ => Value::Null,
    }
}

/// Checks that the proposal may run on its target node and returns its action and node.
fn checked_action(
    conn: &Connection,
    app: &App,
    row: &ProposalRow,
) -> Result<(Action, Node), ApiError> {
    let json = row
        .action_json
        .as_deref()
        .ok_or(ApiError::new("proposal_no_action"))?;
    let action = Action::parse(json).map_err(|_| ApiError::new("action_invalid"))?;
    // The approval bound to this hash, this tool and this tier: anything else is refused.
    if action.hash() != row.args_hash
        || action.tool() != row.tool
        || action.tier().level() != row.tier
        || !action.executable_now()
    {
        return Err(ApiError::new("action_invalid"));
    }
    let node_id = row
        .node_id
        .as_ref()
        .ok_or(ApiError::new("action_invalid"))?;
    let node = db::get_node(conn, &app.scope.org, node_id).map_err(|e| from_db(&e))?;
    let supported = matches!(
        (node.kind, node.access),
        (NodeKind::Local, NodeAccess::Local) | (NodeKind::Vps, NodeAccess::Ssh | NodeAccess::Agent)
    );
    if !supported {
        return Err(ApiError::new("unsupported_target"));
    }
    Ok((action, node))
}

struct SharedRunner(Arc<dyn transport::Runner + Send + Sync>);

impl transport::Runner for SharedRunner {
    fn run(
        &self,
        plan: &transport::CommandPlan,
    ) -> Result<transport::RunOutput, transport::RunError> {
        self.0.run(plan)
    }
}

fn ssh_executor_for_node(app: &App, node: &Node) -> Result<Arc<dyn Executor>, ApiError> {
    let target =
        SshTarget::from_node_labels(&node.name, &node.labels_json).map_err(ApiError::new)?;
    if let Some(ref runner) = app.ssh_runner {
        Ok(Arc::new(SshExecutor::new(
            SharedRunner(Arc::clone(runner)),
            target,
        )))
    } else {
        Ok(Arc::new(SshExecutor::new(
            transport::ProcessRunner::new(std::time::Duration::from_secs(30), 64 * 1024),
            target,
        )))
    }
}

/// Verifies that tier 3–4 proposals have a matching stored approval (and typed confirmation for
/// tier 4).
fn verify_tier_approval(
    conn: &Connection,
    app: &App,
    row: &ProposalRow,
    action: &Action,
) -> Result<(), ApiError> {
    if !action.requires_backup() {
        return Ok(());
    }
    let approval = db::get_approval_for_proposal(conn, &app.scope.org, &row.id)
        .map_err(|e| from_db(&e))?
        .ok_or(ApiError::new("approval_required"))?;
    if approval.decision != ProposalDecision::Approve {
        return Err(ApiError::new("approval_required"));
    }
    if action.requires_typed_confirmation() && !approval.typed_confirmation {
        return Err(ApiError::new("typed_confirmation_required"));
    }
    Ok(())
}

/// `POST /api/proposals/{id}/dry-run`.
pub(crate) async fn proposal_dry_run(State(app): State<Arc<App>>, Path(id): Path<String>) -> Reply {
    let id = ProposalId::parse(&id).map_err(|_| NOT_FOUND)?;
    let body = with_db(&app, move |conn, app| {
        let org = &app.scope.org;
        let row = db::get_proposal(conn, org, &id).map_err(|e| from_db(&e))?;
        let (action, node) = checked_action(conn, app, &row)?;
        let dry = match node.access {
            NodeAccess::Ssh => ssh_executor_for_node(app, &node)?.dry_run(&action),
            NodeAccess::Local | NodeAccess::Agent => match app.executor.as_ref() {
                Some(exec) if node.access == NodeAccess::Local => exec.dry_run(&action),
                _ => actions::dry_run_readonly(&action),
            },
        };
        let steps: Vec<String> = dry.steps.iter().map(|s| clean_text(s)).collect();
        let current_state = clean_text(&dry.current_state);
        let mut diff = dry.diff.as_deref().map(clean_text);
        let mut payload = json!({
            "steps": steps,
            "current_state": current_state,
            "diff": diff,
        })
        .to_string();
        if payload.len() > MAX_DRY_RUN_JSON_BYTES {
            diff = diff.map(|d| d.chars().take(1000).collect());
            payload = json!({
                "steps": steps,
                "current_state": current_state,
                "diff": diff,
            })
            .to_string();
        }
        let dry_run_at =
            db::set_proposal_dry_run(conn, org, &id, &payload).map_err(|e| from_db(&e))?;
        Ok(json!({
            "id": row.id.as_str(),
            "steps": steps,
            "current_state": current_state,
            "diff": diff,
            "dry_run_at": dry_run_at,
        }))
    })
    .await?;
    Ok(json_ok(&body))
}

/// `POST /api/proposals/{id}/execute`.
pub(crate) async fn proposal_execute(State(app): State<Arc<App>>, Path(id): Path<String>) -> Reply {
    let Some(local_executor) = app.executor.clone() else {
        return Err(ApiError::new("execution_disabled"));
    };
    let id = ProposalId::parse(&id).map_err(|_| NOT_FOUND)?;
    let body = with_db(&app, move |conn, app| {
        let org = &app.scope.org;
        let row = db::get_proposal(conn, org, &id).map_err(|e| from_db(&e))?;
        // Look first, so a proposal that cannot run is refused without claiming anything.
        if row.status == db::ProposalStatus::Approved {
            let (action, _) = checked_action(conn, app, &row)?;
            verify_tier_approval(conn, app, &row, &action)?;
        }
        let (execution, row) =
            match db::begin_execution(conn, org, &id, &app.scope.user).map_err(|e| from_db(&e))? {
                BeginOutcome::Started {
                    execution,
                    proposal,
                } => (execution, *proposal),
                BeginOutcome::NotApproved(_) => return Err(ApiError::new("proposal_not_approved")),
                BeginOutcome::Expired => return Err(ApiError::new("proposal_expired")),
                BeginOutcome::NoAction => return Err(ApiError::new("proposal_no_action")),
                BeginOutcome::AlreadyRun => return Err(ApiError::new("proposal_already_run")),
            };
        let checked = checked_action(conn, app, &row);
        if let Ok((_, ref node)) = checked
            && node.access == NodeAccess::Agent
        {
            let action_json = row.action_json.as_deref().unwrap_or_default();
            let job_id = db::enqueue_agent_job(
                conn,
                org,
                &node.id,
                &row.id,
                &execution,
                action_json,
                &row.args_hash,
            )
            .map_err(|e| from_db(&e))?;
            let row = db::get_proposal(conn, org, &id).map_err(|e| from_db(&e))?;
            return Ok(json!({
                "id": row.id.as_str(),
                "status": row.status.as_str(),
                "has_backup": false,
                "execution": execution_view(conn, app, &row),
                "agent_job": {
                    "id": job_id.as_str(),
                    "status": "queued",
                },
            }));
        }
        let outcome = match checked {
            Ok((action, node)) => {
                let executor: Arc<dyn Executor> = match node.access {
                    NodeAccess::Local => local_executor,
                    NodeAccess::Ssh => ssh_executor_for_node(app, &node)?,
                    NodeAccess::Agent => unreachable!(),
                };
                if action.requires_backup() {
                    match executor.backup(&action, &app.backup_dir()) {
                        Ok(snap) => {
                            if let Err(err) = db::record_execution_backup(
                                conn,
                                org,
                                &execution,
                                &snap.storage_ref,
                                &snap.sha256,
                            ) {
                                let _ = db::finish_execution(
                                    conn,
                                    org,
                                    &execution,
                                    false,
                                    false,
                                    "backup_failed",
                                    "",
                                );
                                return Err(from_db(&err));
                            }
                            executor.execute(&action)
                        }
                        Err(reason) => actions::Outcome {
                            status: OutcomeStatus::Failed,
                            verified: false,
                            reason,
                            detail: String::new(),
                        },
                    }
                } else {
                    executor.execute(&action)
                }
            }
            Err(_) => actions::Outcome {
                status: OutcomeStatus::Failed,
                verified: false,
                reason: "action_invalid",
                detail: String::new(),
            },
        };
        db::finish_execution(
            conn,
            org,
            &execution,
            outcome.status == OutcomeStatus::Succeeded,
            outcome.verified,
            outcome.reason,
            &outcome.detail,
        )
        .map_err(|e| from_db(&e))?;
        let row = db::get_proposal(conn, org, &id).map_err(|e| from_db(&e))?;
        Ok(json!({
            "id": row.id.as_str(),
            "status": row.status.as_str(),
            "has_backup": has_backup_view(conn, app, &row),
            "execution": execution_view(conn, app, &row),
        }))
    })
    .await?;
    Ok(json_ok(&body))
}

/// `POST /api/proposals/{id}/rollback`.
pub(crate) async fn proposal_rollback(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> Reply {
    let Some(local_executor) = app.executor.clone() else {
        return Err(ApiError::new("execution_disabled"));
    };
    let id = ProposalId::parse(&id).map_err(|_| NOT_FOUND)?;
    let body = with_db(&app, move |conn, app| {
        let org = &app.scope.org;
        let row = db::get_proposal(conn, org, &id).map_err(|e| from_db(&e))?;
        let (action, node) = checked_action(conn, app, &row)?;
        let executor: Arc<dyn Executor> = match node.access {
            NodeAccess::Local => local_executor,
            NodeAccess::Ssh => ssh_executor_for_node(app, &node)?,
            NodeAccess::Agent => return Err(ApiError::new("unsupported_target")),
        };
        let (rollback, backup) =
            match db::begin_rollback(conn, org, &id, &app.scope.user).map_err(|e| from_db(&e))? {
                BeginRollbackOutcome::Started {
                    rollback, backup, ..
                } => (rollback, backup),
                BeginRollbackOutcome::NoExecution => {
                    return Err(ApiError::new("proposal_not_executed"));
                }
                BeginRollbackOutcome::ExecutionStillRunning => {
                    return Err(ApiError::new("execution_running"));
                }
                BeginRollbackOutcome::NoBackup => return Err(ApiError::new("backup_missing")),
                BeginRollbackOutcome::AlreadyRolledBack => {
                    return Err(ApiError::new("already_rolled_back"));
                }
            };
        let outcome = executor.rollback(
            &action,
            &BackupSnapshot {
                storage_ref: backup.storage_ref,
                sha256: backup.sha256,
            },
        );
        db::finish_rollback(
            conn,
            org,
            &rollback,
            outcome.status == OutcomeStatus::Succeeded,
            outcome.verified,
            outcome.reason,
            &outcome.detail,
        )
        .map_err(|e| from_db(&e))?;
        let row = db::get_proposal(conn, org, &id).map_err(|e| from_db(&e))?;
        Ok(json!({
            "id": row.id.as_str(),
            "rollback": rollback_view(conn, app, &row),
        }))
    })
    .await?;
    Ok(json_ok(&body))
}
