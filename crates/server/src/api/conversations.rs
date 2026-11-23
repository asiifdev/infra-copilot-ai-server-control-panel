//! Conversations of the person the server acts for. A conversation of another person, or of
//! another organization, is indistinguishable from a missing one.

use super::{Reply, json_ok, json_status, with_db};
use crate::app::App;
use crate::errors::{INVALID_INPUT, NOT_FOUND, from_db};
use crate::guard::Lang;
use crate::util::{clean_text, json_object, limit_param, opt_str};
use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use db::{ConversationId, ConversationRow, MessageRole};
use i18n::MessageKey;
use serde_json::{Value, json};
use std::sync::Arc;

const DEFAULT_TITLE: MessageKey = MessageKey::new("api.conversation.new_title");
/// Longest title accepted, in characters (the store caps it at 200 as well).
const MAX_TITLE_CHARS: usize = 200;
/// Conversations returned by the list.
const LIST_LIMIT: u32 = 100;

fn conversation_json(row: &ConversationRow) -> Value {
    json!({
        "id": row.id.as_str(),
        "title": clean_text(&row.title),
        "created_at": row.created_at,
        "archived": row.archived_at.is_some(),
    })
}

/// The conversation `id` if it belongs to the person the server acts for.
pub(crate) fn own_conversation(
    conn: &db::Connection,
    app: &App,
    id: &str,
) -> Result<ConversationRow, crate::errors::ApiError> {
    let id = ConversationId::parse(id).map_err(|_| NOT_FOUND)?;
    let row = db::get_conversation(conn, &app.scope.org, &id).map_err(|e| from_db(&e))?;
    if row.user_id == app.scope.user {
        Ok(row)
    } else {
        Err(NOT_FOUND)
    }
}

/// `GET /api/conversations`: newest first.
pub(crate) async fn conversations_list(State(app): State<Arc<App>>) -> Reply {
    let body = with_db(&app, |conn, app| {
        let rows = db::list_conversations(conn, &app.scope.org, &app.scope.user, false, LIST_LIMIT)
            .map_err(|e| from_db(&e))?;
        Ok(Value::Array(rows.iter().map(conversation_json).collect()))
    })
    .await?;
    Ok(json_ok(&body))
}

/// `POST /api/conversations {title?}`.
pub(crate) async fn conversations_create(
    State(app): State<Arc<App>>,
    Extension(lang): Extension<Lang>,
    body: Bytes,
) -> Reply {
    let map = json_object(&body, true).ok_or(INVALID_INPUT)?;
    if map.keys().any(|k| k != "title") {
        return Err(INVALID_INPUT);
    }
    let title = match opt_str(&map, "title").map_err(|()| INVALID_INPUT)? {
        Some(t) if t.chars().count() <= MAX_TITLE_CHARS && !t.trim().is_empty() => {
            clean_text(t.trim())
        }
        Some(t) if !t.trim().is_empty() => return Err(INVALID_INPUT),
        _ => app.catalog.get(lang.0, DEFAULT_TITLE, &[]).text,
    };
    let row = with_db(&app, move |conn, app| {
        let id = db::create_conversation(conn, &app.scope.org, &app.scope.user, &title, None)
            .map_err(|e| from_db(&e))?;
        db::get_conversation(conn, &app.scope.org, &id).map_err(|e| from_db(&e))
    })
    .await?;
    Ok(json_status(StatusCode::CREATED, &conversation_json(&row)))
}

/// Names of the tools an assistant message called (from the stored call list; arguments are not
/// shown).
fn tool_names(calls_json: Option<&str>) -> Vec<String> {
    let Some(Value::Array(calls)) = calls_json.and_then(|t| serde_json::from_str(t).ok()) else {
        return Vec::new();
    };
    calls
        .iter()
        .filter_map(|c| c.get("name")?.as_str())
        .map(|name| clean_text(name).chars().take(80).collect())
        .collect()
}

/// `GET /api/conversations/{id}/messages`: the person's and the assistant's messages (system
/// text and raw tool results stay on the server).
pub(crate) async fn conversation_messages(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
) -> Reply {
    let limit = limit_param(query.as_deref(), "limit", 200, 500).map_err(|()| INVALID_INPUT)?;
    let body = with_db(&app, move |conn, app| {
        let row = own_conversation(conn, app, &id)?;
        let rows =
            db::list_messages(conn, &app.scope.org, &row.id, limit).map_err(|e| from_db(&e))?;
        Ok(Value::Array(
            rows.iter()
                .filter(|m| matches!(m.role, MessageRole::User | MessageRole::Assistant))
                .map(|m| {
                    json!({
                        "seq": m.seq,
                        "role": m.role.as_str(),
                        "content": clean_text(&m.content),
                        "tools": tool_names(m.tool_calls_json.as_deref()),
                        "created_at": m.created_at,
                    })
                })
                .collect(),
        ))
    })
    .await?;
    Ok(json_ok(&body))
}
