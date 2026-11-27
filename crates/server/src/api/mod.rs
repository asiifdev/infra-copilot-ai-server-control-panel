//! The API handlers (decision 0029). They are thin: they check input, call the crates that hold
//! the logic (`db`, `agent`, `plan`, `parity`, `vault`, `bootstrap`) and shape JSON. Database work
//! runs on the blocking pool with a connection of its own, scoped to the one organization the
//! server acts for.

mod access;
mod agents;
pub(crate) mod conversations;
mod data;
mod execute;
mod memories;
pub(crate) mod monitors;
mod providers;

pub(crate) use access::{entitlements, exchange, healthz, i18n_catalog, logout, session};
pub(crate) use agents::{
    agent_poll, agent_result, node_agent_token_rotate, node_logs, sites_create, sites_list,
};
pub(crate) use conversations::{
    conversation_messages, conversations_create, conversations_list, own_conversation,
};
pub(crate) use data::{
    findings, inventory, nodes, plan_approve, plan_detail, plan_run, plans, transport_credentials,
    transports,
};
pub(crate) use execute::{proposal_dry_run, proposal_execute, proposal_rollback};
pub(crate) use memories::{
    memories_list, memory_approve, memory_delete, proposal_decide, proposals_list,
};
pub(crate) use monitors::{
    channel_create, channel_delete, channels_list, incident_ack, incidents_list, monitor_create,
    monitor_delete, monitors_list, telegram_link_code,
};
pub(crate) use providers::{provider_add, provider_remove, provider_test, providers_list};

use crate::app::App;
use crate::errors::{ApiError, INTERNAL};
use axum::body::Body;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use db::Connection;
use serde_json::Value;
use std::sync::Arc;

/// What a handler returns.
pub(crate) type Reply = Result<Response, ApiError>;

/// A JSON response with a status.
pub(crate) fn json_status(status: StatusCode, value: &Value) -> Response {
    let mut response = Response::new(Body::from(value.to_string()));
    *response.status_mut() = status;
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    response
}

/// A `200` JSON response.
pub(crate) fn json_ok(value: &Value) -> Response {
    json_status(StatusCode::OK, value)
}

/// A `204` response.
pub(crate) fn no_content() -> Response {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::NO_CONTENT;
    response
}

/// Runs `work` on the blocking pool with a fresh connection to the database. The connection is
/// private to the call, so nothing is shared between requests.
pub(crate) async fn with_db<T, F>(app: &Arc<App>, work: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce(&Connection, &App) -> Result<T, ApiError> + Send + 'static,
{
    let app = Arc::clone(app);
    tokio::task::spawn_blocking(move || {
        let conn = db::open(&app.db_path).map_err(|_| crate::errors::STORE_FAILED)?;
        work(&conn, &app)
    })
    .await
    .map_err(|_| INTERNAL)?
}
