//! Memories (notes the assistant proposed or a person wrote) and Usulan (proposals).
//!
//! A memory becomes active only when a person approves it, and only here (and in the command
//! line): nothing a model can call reaches approval. Proposals are read-only: approving and
//! running them is milestone M4.

use super::{Reply, json_ok, no_content, with_db};
use crate::app::App;
use crate::errors::{INVALID_INPUT, NOT_FOUND, from_db};
use crate::util::{clean_text, limit_param, query_get};
use axum::extract::{Path, RawQuery, State};
use db::{MemoryId, MemoryRow, MemoryStatus, ProposalStatus};
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

/// `GET /api/proposals?status=`: read-only.
pub(crate) async fn proposals_list(
    State(app): State<Arc<App>>,
    RawQuery(query): RawQuery,
) -> Reply {
    let status = match query_get(query.as_deref(), "status") {
        None => None,
        Some(text) => Some(ProposalStatus::parse(&text).map_err(|_| INVALID_INPUT)?),
    };
    let limit = limit_param(query.as_deref(), "limit", 200, 500).map_err(|()| INVALID_INPUT)?;
    let body = with_db(&app, move |conn, app| {
        let rows = db::list_proposals(conn, &app.scope.org, status, limit)
            .map_err(|e| from_db(&e))?;
        Ok(Value::Array(
            rows.iter()
                .map(|p| {
                    json!({
                        "id": p.id.as_str(),
                        "conversation_id": p.conversation_id.as_ref().map(db::ConversationId::as_str),
                        "tool": clean_text(&p.tool),
                        "tier": p.tier,
                        "title": clean_text(&p.title),
                        "rationale": clean_text(&p.rationale),
                        "suggested_action": clean_text(&p.suggested_action),
                        "impact": clean_text(&p.impact),
                        "rule_id": p.rule_id,
                        "subject_key": p.subject_key.as_deref().map(clean_text),
                        "status": p.status.as_str(),
                        "created_at": p.created_at,
                        "expires_at": p.expires_at,
                    })
                })
                .collect(),
        ))
    })
    .await?;
    Ok(json_ok(&body))
}
