//! Pull-only remote agent endpoints, bounded read-only service log viewer, and `.test` local
//! site management (M5).

use super::{Reply, json_ok, json_status, with_db};
use crate::app::App;
use crate::errors::{ApiError, INVALID_INPUT, NOT_FOUND, from_db};
use crate::util::{clean_text, json_object, query_get};
use actions::{
    MAX_LOG_LINES, ServiceName, read_managed_log, read_service_logs_via_runner,
    render_test_vhost_config, validate_test_domain, write_managed_test_site,
};
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, StatusCode, Uri};
use core_domain::redact::redact;
use db::{AgentJobId, AgentJobResult, LocalSiteStatus, NodeId};
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::{Value, json};
use std::sync::Arc;

const DEFAULT_LOG_LINES: usize = 100;

fn generate_agent_token() -> Result<String, ApiError> {
    let mut raw = [0u8; 32];
    SystemRandom::new()
        .fill(&mut raw)
        .map_err(|_| ApiError::new("store_failed"))?;
    let mut hex = String::with_capacity(64);
    for b in raw {
        use std::fmt::Write as _;
        let _ = write!(hex, "{b:02x}");
    }
    Ok(hex)
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let raw = headers.get(AUTHORIZATION)?.to_str().ok()?;
    let token = raw.strip_prefix("Bearer ")?.trim();
    if token.is_empty() { None } else { Some(token) }
}

/// `POST /api/nodes/{id}/agent-token`: creates/rotates the pull-only agent token for a node.
/// Returns the raw token once; only its SHA-256 hash is stored.
pub(crate) async fn node_agent_token_rotate(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> Reply {
    let node_id = NodeId::parse(&id).map_err(|_| NOT_FOUND)?;
    let raw_token = generate_agent_token()?;
    let token_hash = db::sha256_hex(raw_token.as_bytes());
    let body = with_db(&app, move |conn, app| {
        let token_id =
            db::rotate_agent_token(conn, &app.scope.org, &node_id, &app.scope.user, &token_hash)
                .map_err(|e| from_db(&e))?;
        Ok(json!({
            "node_id": node_id.as_str(),
            "token_id": token_id.as_str(),
            "token": raw_token,
        }))
    })
    .await?;
    Ok(json_ok(&body))
}

/// `POST /api/agents/poll`: authenticated by `Authorization: Bearer <token>`.
/// Atomically claims at most one `queued` job (`queued` -> `running`) for the agent's node.
pub(crate) async fn agent_poll(State(app): State<Arc<App>>, headers: HeaderMap) -> Reply {
    let token = bearer_token(&headers).ok_or(ApiError::new("unauthenticated"))?;
    let token_hash = db::sha256_hex(token.as_bytes());
    let body = with_db(&app, move |conn, _app| {
        let auth = db::authenticate_agent_token(conn, &token_hash)
            .map_err(|e| from_db(&e))?
            .ok_or(ApiError::new("unauthenticated"))?;
        let job =
            db::claim_next_agent_job(conn, &auth.org_id, &auth.node_id).map_err(|e| from_db(&e))?;
        let job_json = match job {
            Some(j) => json!({
                "id": j.id.as_str(),
                "proposal_id": j.proposal_id.as_str(),
                "execution_id": j.execution_id.as_str(),
                "action_json": j.action_json,
                "args_hash": j.args_hash,
                "status": j.status.as_str(),
                "claimed_at": j.claimed_at,
            }),
            None => Value::Null,
        };
        Ok(json!({ "job": job_json }))
    })
    .await?;
    Ok(json_ok(&body))
}

/// `POST /api/agents/result`: authenticated by `Authorization: Bearer <token>`.
/// Records the outcome of a claimed job, updates `executions`, and appends an audit entry.
pub(crate) async fn agent_result(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    raw: Bytes,
) -> Reply {
    let token = bearer_token(&headers).ok_or(ApiError::new("unauthenticated"))?;
    let token_hash = db::sha256_hex(token.as_bytes());
    let map = json_object(&raw, false).ok_or(INVALID_INPUT)?;
    let job_id_raw = map
        .get("job_id")
        .and_then(Value::as_str)
        .ok_or(INVALID_INPUT)?;
    let job_id = AgentJobId::parse(job_id_raw).map_err(|_| NOT_FOUND)?;
    let succeeded = map
        .get("succeeded")
        .and_then(Value::as_bool)
        .ok_or(INVALID_INPUT)?;
    let verified = map
        .get("verified")
        .and_then(Value::as_bool)
        .ok_or(INVALID_INPUT)?;
    let reason_raw = map
        .get("reason")
        .and_then(Value::as_str)
        .ok_or(INVALID_INPUT)?;
    let output_raw = match map.get("output") {
        None => "",
        Some(v) => v.as_str().ok_or(INVALID_INPUT)?,
    };

    let redacted_output = clean_text(&redact(output_raw).text);
    let reason = clean_text(reason_raw);
    let body = with_db(&app, move |conn, _app| {
        let auth = db::authenticate_agent_token(conn, &token_hash)
            .map_err(|e| from_db(&e))?
            .ok_or(ApiError::new("unauthenticated"))?;
        let row = db::complete_agent_job(
            conn,
            &auth.org_id,
            &auth.node_id,
            &job_id,
            AgentJobResult {
                succeeded,
                verified,
                reason: &reason,
                output: &redacted_output,
            },
        )
        .map_err(|e| from_db(&e))?;
        Ok(json!({
            "job_id": row.id.as_str(),
            "execution_id": row.execution_id.as_str(),
            "status": row.status.as_str(),
            "verified": row.verified,
            "reason": row.reason,
        }))
    })
    .await?;
    Ok(json_ok(&body))
}

/// `GET /api/nodes/{id}/logs?service=...&lines=...`: read-only, redacted, bounded service logs.
pub(crate) async fn node_logs(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    uri: Uri,
) -> Reply {
    let node_id = NodeId::parse(&id).map_err(|_| NOT_FOUND)?;
    let raw_service = query_get(uri.query(), "service").ok_or(ApiError::new("invalid_service"))?;
    let service = ServiceName::new(&raw_service).map_err(|_| ApiError::new("invalid_service"))?;
    let lines = match query_get(uri.query(), "lines") {
        Some(v) => v
            .parse::<usize>()
            .map_err(|_| ApiError::new("bad_parameter"))?
            .clamp(1, MAX_LOG_LINES),
        None => DEFAULT_LOG_LINES,
    };

    let body = with_db(&app, move |conn, app| {
        let _node = db::get_node(conn, &app.scope.org, &node_id).map_err(|e| from_db(&e))?;
        let excerpt = if let Some(ref dir) = app.logs_dir {
            read_managed_log(dir, &service, lines).map_err(ApiError::new)?
        } else if let Some(ref runner) = app.ssh_runner {
            read_service_logs_via_runner(runner.as_ref(), &service, lines).map_err(ApiError::new)?
        } else {
            return Err(ApiError::new("logs_unavailable"));
        };
        Ok(json!({
            "node_id": node_id.as_str(),
            "service": excerpt.service,
            "line_count": excerpt.line_count,
            "truncated": excerpt.truncated,
            "content": clean_text(&excerpt.content),
        }))
    })
    .await?;
    Ok(json_ok(&body))
}

/// `GET /api/sites`: lists managed `.test` local sites for the org.
pub(crate) async fn sites_list(State(app): State<Arc<App>>) -> Reply {
    let body = with_db(&app, move |conn, app| {
        let rows = db::list_local_sites(conn, &app.scope.org).map_err(|e| from_db(&e))?;
        let sites: Vec<Value> = rows
            .into_iter()
            .map(|s| {
                json!({
                    "id": s.id.as_str(),
                    "domain": s.domain,
                    "upstream_port": s.upstream_port,
                    "config_path": s.config_path,
                    "status": s.status.as_str(),
                    "created_at": s.created_at,
                })
            })
            .collect();
        Ok(json!({ "sites": sites }))
    })
    .await?;
    Ok(json_ok(&body))
}

/// `POST /api/sites`: validates a `*.test` domain and records/writes a safe managed vhost config.
pub(crate) async fn sites_create(State(app): State<Arc<App>>, raw: Bytes) -> Reply {
    let map = json_object(&raw, false).ok_or(INVALID_INPUT)?;
    let domain_raw = map
        .get("domain")
        .and_then(Value::as_str)
        .ok_or(INVALID_INPUT)?;
    let port_u64 = map
        .get("upstream_port")
        .and_then(Value::as_u64)
        .ok_or(INVALID_INPUT)?;
    let upstream_port =
        u16::try_from(port_u64).map_err(|_| ApiError::new("invalid_upstream_port"))?;
    if upstream_port == 0 {
        return Err(ApiError::new("invalid_upstream_port"));
    }
    let domain = validate_test_domain(domain_raw).map_err(ApiError::new)?;
    let config_preview = render_test_vhost_config(&domain, upstream_port);

    let body = with_db(&app, move |conn, app| {
        let (config_path, status) = if let Some(ref dir) = app.sites_dir {
            let written =
                write_managed_test_site(dir, &domain, upstream_port).map_err(ApiError::new)?;
            (
                written.to_string_lossy().into_owned(),
                LocalSiteStatus::Active,
            )
        } else {
            (
                format!("/etc/nodivara/sites/{domain}.conf"),
                LocalSiteStatus::Proposed,
            )
        };
        let row = db::create_local_site(
            conn,
            &app.scope.org,
            &app.scope.user,
            &domain,
            upstream_port,
            &config_path,
            status,
        )
        .map_err(|e| from_db(&e))?;
        Ok(json!({
            "site": {
                "id": row.id.as_str(),
                "domain": row.domain,
                "upstream_port": row.upstream_port,
                "config_path": row.config_path,
                "status": row.status.as_str(),
                "created_at": row.created_at,
            },
            "config_preview": config_preview,
        }))
    })
    .await?;
    Ok(json_status(StatusCode::CREATED, &body))
}
