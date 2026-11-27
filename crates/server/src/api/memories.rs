//! Memories (notes the assistant proposed or a person wrote) and Usulan (proposals).
//!
//! A memory becomes active only when a person approves it, and only here (and in the command
//! line): nothing a model can call reaches approval. A person may approve or reject a pending
//! proposal here; approving only records the decision (and an audit entry): nothing is executed,
//! running approved proposals is milestone M4.

use super::{Reply, json_ok, no_content, with_db};
use crate::app::App;
use crate::errors::{INVALID_INPUT, NOT_FOUND, from_db};
use crate::guard::Lang;
use crate::util::{clean_text, limit_param, query_get};
use crate::util::{json_object, opt_str};
use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, RawQuery, State};
use db::{
    DecisionOutcome, MemoryId, MemoryRow, MemoryStatus, ProposalDecision, ProposalId,
    ProposalStatus,
};
use serde_json::{Value, json};
use std::sync::Arc;

/// Whether the person the server acts for may see the memory: org-wide notes, and their own
/// personal preferences (another member's preference is as good as missing).
fn visible(row: &MemoryRow, app: &App) -> bool {
    row.user_id
        .as_ref()
        .is_none_or(|owner| *owner == app.scope.user)
}

fn memory_json(row: &MemoryRow) -> Value {
    json!({
        "id": row.id.as_str(),
        "kind": row.kind.as_str(),
        // A model wrote it: shown as text, with control characters replaced.
        "content": clean_text(&row.content),
        "source": row.source.as_str(),
        "status": row.status.as_str(),
        "conversation_id": row.conversation_id.as_ref().map(db::ConversationId::as_str),
        "created_at": row.created_at,
        "updated_at": row.updated_at,
    })
}

/// `GET /api/memories?status=pending|active`.
pub(crate) async fn memories_list(State(app): State<Arc<App>>, RawQuery(query): RawQuery) -> Reply {
    let status = match query_get(query.as_deref(), "status").as_deref() {
        None => None,
        Some("pending") => Some(MemoryStatus::Pending),
        Some("active") => Some(MemoryStatus::Active),
        Some(_) => return Err(INVALID_INPUT),
    };
    let body = with_db(&app, move |conn, app| {
        let rows = db::list_memories(conn, &app.scope.org, status).map_err(|e| from_db(&e))?;
        Ok(Value::Array(
            rows.iter()
                .filter(|row| visible(row, app))
                .map(memory_json)
                .collect(),
        ))
    })
    .await?;
    Ok(json_ok(&body))
}

/// The memory `id` if it exists and may be seen; a malformed id is a missing one.
fn find_memory(
    conn: &db::Connection,
    app: &App,
    id: &str,
) -> Result<MemoryId, crate::errors::ApiError> {
    let id = MemoryId::parse(id).map_err(|_| NOT_FOUND)?;
    let row = db::get_memory(conn, &app.scope.org, &id).map_err(|e| from_db(&e))?;
    if visible(&row, app) {
        Ok(id)
    } else {
        Err(NOT_FOUND)
    }
}

/// `POST /api/memories/{id}/approve`.
pub(crate) async fn memory_approve(State(app): State<Arc<App>>, Path(id): Path<String>) -> Reply {
    with_db(&app, move |conn, app| {
        let id = find_memory(conn, app, &id)?;
        db::approve_memory(conn, &app.scope.org, &id).map_err(|e| from_db(&e))
    })
    .await?;
    Ok(no_content())
}

/// `DELETE /api/memories/{id}`.
pub(crate) async fn memory_delete(State(app): State<Arc<App>>, Path(id): Path<String>) -> Reply {
    with_db(&app, move |conn, app| {
        let id = find_memory(conn, app, &id)?;
        db::delete_memory(conn, &app.scope.org, &id).map_err(|e| from_db(&e))
    })
    .await?;
    Ok(no_content())
}

/// The impact sentence of a proposal. A tool stores the catalog key of its own fixed sentence
/// (`assistant.proposal.*`); it is shown in the reader's language. Anything else is text, shown as
/// cleaned text.
fn impact_text(app: &App, locale: i18n::Locale, impact: &str) -> String {
    let sentence = impact
        .starts_with("assistant.proposal.")
        .then(|| app.catalog.raw(locale, impact))
        .flatten();
    clean_text(sentence.unwrap_or(impact))
}

/// `GET /api/proposals?status=`.
pub(crate) async fn proposals_list(
    State(app): State<Arc<App>>,
    Extension(lang): Extension<Lang>,
    RawQuery(query): RawQuery,
) -> Reply {
    let status = match query_get(query.as_deref(), "status") {
        None => None,
        Some(text) => Some(ProposalStatus::parse(&text).map_err(|_| INVALID_INPUT)?),
    };
    let limit = limit_param(query.as_deref(), "limit", 200, 500).map_err(|()| INVALID_INPUT)?;
    let body = with_db(&app, move |conn, app| {
        let locale = lang.0;
        let two_approvers =
            db::org_requires_two_approvers(conn, &app.scope.org).map_err(|e| from_db(&e))?;
        let rows =
            db::list_proposals(conn, &app.scope.org, status, limit).map_err(|e| from_db(&e))?;
        let mut items = Vec::with_capacity(rows.len());
        for p in &rows {
            let required_approvals: u32 = if two_approvers && p.tier == 4 { 2 } else { 1 };
            let (approvals_count, user_has_approved) =
                db::count_approvals_for_proposal(conn, &app.scope.org, &p.id, &app.scope.user)
                    .map_err(|e| from_db(&e))?;
            items.push(json!({
                "id": p.id.as_str(),
                "conversation_id": p.conversation_id.as_ref().map(db::ConversationId::as_str),
                "tool": clean_text(&p.tool),
                "tier": p.tier,
                "title": clean_text(&p.title),
                "rationale": clean_text(&p.rationale),
                "suggested_action": clean_text(&p.suggested_action),
                "impact": impact_text(app, locale, &p.impact),
                "rule_id": p.rule_id,
                "subject_key": p.subject_key.as_deref().map(clean_text),
                "status": p.status.as_str(),
                "created_at": p.created_at,
                "expires_at": p.expires_at,
                "action": super::execute::action_view(p),
                "dry_run": super::execute::dry_run_view(p),
                "approval": super::execute::approval_view(conn, app, p),
                "approvals_count": approvals_count,
                "required_approvals": required_approvals,
                "user_has_approved": user_has_approved,
                "has_backup": super::execute::has_backup_view(conn, app, p),
                "node": p.node_id.as_ref().and_then(|n| {
                    db::get_node(conn, &app.scope.org, n).ok().map(|n| clean_text(&n.name))
                }),
                "execution": super::execute::execution_view(conn, app, p),
                "rollback": super::execute::rollback_view(conn, app, p),
            }));
        }
        Ok(Value::Array(items))
    })
    .await?;
    Ok(json_ok(&body))
}

/// `POST /api/proposals/{id}/decision {"decision": "approve" | "reject", "typed_confirmation"?: string, "comment"?: string}`:
/// a person decides a pending proposal. Tier 4 approvals require `typed_confirmation` matching
/// `action.target_label()`. A proposal someone already decided answers `409 proposal_decided`; an
/// expired one `409 proposal_expired`.
pub(crate) async fn proposal_decide(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    body: Bytes,
) -> Reply {
    let map = json_object(&body, false).ok_or(INVALID_INPUT)?;
    if map
        .keys()
        .any(|k| !matches!(k.as_str(), "decision" | "typed_confirmation" | "comment"))
    {
        return Err(INVALID_INPUT);
    }
    let decision = match opt_str(&map, "decision") {
        Ok(Some("approve")) => ProposalDecision::Approve,
        Ok(Some("reject")) => ProposalDecision::Reject,
        _ => return Err(INVALID_INPUT),
    };
    let typed_confirmation = opt_str(&map, "typed_confirmation")
        .map_err(|()| INVALID_INPUT)?
        .map(str::to_owned);
    let comment = opt_str(&map, "comment")
        .map_err(|()| INVALID_INPUT)?
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    if comment.as_ref().is_some_and(|c| c.chars().count() > 500) {
        return Err(INVALID_INPUT);
    }
    let id = ProposalId::parse(&id).map_err(|_| NOT_FOUND)?;
    let (outcome, approvals_count, required_approvals, user_has_approved) =
        with_db(&app, move |conn, app| {
            let row = db::get_proposal(conn, &app.scope.org, &id).map_err(|e| from_db(&e))?;
            let mut confirmed = false;
            if decision == ProposalDecision::Approve && row.tier == 4 {
                let action_json = row
                    .action_json
                    .as_deref()
                    .ok_or(crate::errors::ApiError::new("action_invalid"))?;
                let action = actions::Action::parse(action_json)
                    .map_err(|_| crate::errors::ApiError::new("action_invalid"))?;
                if typed_confirmation.as_deref() != Some(action.target_label()) {
                    return Err(crate::errors::ApiError::new("typed_confirmation_mismatch"));
                }
                confirmed = true;
            }
            let two_approvers =
                db::org_requires_two_approvers(conn, &app.scope.org).map_err(|e| from_db(&e))?;
            let required_approvals: u32 = if two_approvers && row.tier == 4 { 2 } else { 1 };
            let outcome = db::decide_proposal_with_details(
                conn,
                &app.scope.org,
                &id,
                decision,
                &app.scope.user,
                confirmed,
                comment.as_deref(),
            )
            .map_err(|e| from_db(&e))?;
            let (approvals_count, user_has_approved) =
                db::count_approvals_for_proposal(conn, &app.scope.org, &id, &app.scope.user)
                    .map_err(|e| from_db(&e))?;
            Ok((
                outcome,
                approvals_count,
                required_approvals,
                user_has_approved,
            ))
        })
        .await?;
    match outcome {
        DecisionOutcome::Decided(row) => Ok(json_ok(&json!({
            "id": row.id.as_str(),
            "status": row.status.as_str(),
            "approvals_count": approvals_count,
            "required_approvals": required_approvals,
            "user_has_approved": user_has_approved,
        }))),
        DecisionOutcome::AlreadyDecided(_) => Err(crate::errors::ApiError::new("proposal_decided")),
        DecisionOutcome::Expired => Err(crate::errors::ApiError::new("proposal_expired")),
    }
}
