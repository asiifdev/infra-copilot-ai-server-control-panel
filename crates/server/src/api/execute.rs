//! Running an approved proposal (ADR 0037).
//!
//! Everything is checked before anything starts: execution is switched on, the proposal is
//! approved and unexpired, its stored action is valid, its hash and tier are those the approval
//! bound to, and the target is this machine. Only then is the one execution claimed
//! (`db::begin_execution`), run through the executor and recorded (`db::finish_execution`).
//! Nothing a model can call reaches this handler.

use super::{Reply, json_ok, with_db};
use crate::app::App;
use crate::errors::{ApiError, NOT_FOUND, from_db};
use crate::util::clean_text;
use actions::{Action, OutcomeStatus};
use axum::extract::{Path, State};
use db::{BeginOutcome, Connection, NodeAccess, NodeKind, ProposalId, ProposalRow};
use serde_json::{Value, json};
use std::sync::Arc;

/// The structured action of a proposal as the UI shows it, or `Null` when it has none or the
/// stored text is not a valid action.
pub(crate) fn action_view(row: &ProposalRow) -> Value {
    let Some(json) = row.action_json.as_deref() else {
        return Value::Null;
    };
    match Action::parse(json) {
        Ok(Action::RestartService { manager, name }) => json!({
            "kind": "restart_service",
            "manager": manager.as_str(),
            "service": clean_text(name.as_str()),
        }),
        Err(_) => Value::Null,
    }
}

/// The execution of a proposal as the UI shows it, or `Null`. The output is text from a server:
/// redacted by the executor, and control characters are replaced again here.
pub(crate) fn execution_view(conn: &Connection, app: &App, row: &ProposalRow) -> Value {
    match db::get_execution_for_proposal(conn, &app.scope.org, &row.id) {
        Ok(Some(e)) => json!({
            "status": e.status.as_str(),
            "verified": e.verified,
            "reason": e.reason,
            "output": clean_text(&e.output),
            "started_at": e.started_at,
            "finished_at": e.finished_at,
        }),
        _ => Value::Null,
    }
}

/// Checks that the proposal may run here and returns its action.
fn checked_action(conn: &Connection, app: &App, row: &ProposalRow) -> Result<Action, ApiError> {
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
    let node = row
        .node_id
        .as_ref()
        .ok_or(ApiError::new("action_invalid"))?;
    let node = db::get_node(conn, &app.scope.org, node).map_err(|e| from_db(&e))?;
    // Only this machine until the agent and SSH executors exist.
    if node.kind != NodeKind::Local || node.access != NodeAccess::Local {
        return Err(ApiError::new("unsupported_target"));
    }
    Ok(action)
}

/// `POST /api/proposals/{id}/execute`.
pub(crate) async fn proposal_execute(State(app): State<Arc<App>>, Path(id): Path<String>) -> Reply {
    let Some(executor) = app.executor.clone() else {
        return Err(ApiError::new("execution_disabled"));
    };
    let id = ProposalId::parse(&id).map_err(|_| NOT_FOUND)?;
    let body = with_db(&app, move |conn, app| {
        let org = &app.scope.org;
        let row = db::get_proposal(conn, org, &id).map_err(|e| from_db(&e))?;
        // Look first, so a proposal that cannot run is refused without claiming anything.
        if row.status == db::ProposalStatus::Approved {
            checked_action(conn, app, &row)?;
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
        let outcome = match checked_action(conn, app, &row) {
            Ok(action) => executor.execute(&action),
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
            "execution": execution_view(conn, app, &row),
        }))
    })
    .await?;
    Ok(json_ok(&body))
}
