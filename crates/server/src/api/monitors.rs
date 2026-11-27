//! Monitors, incidents, notification channels, and Telegram chat linking (M1-b).
//!
//! Channel bot tokens are sealed in the vault (`notification_channel_token` purpose) and are
//! never stored plaintext or returned by any API.

use super::{Reply, json_ok, json_status, no_content, with_db};
use crate::app::App;
use crate::errors::{ApiError, INVALID_INPUT, NOT_FOUND, STORE_FAILED, from_db};
use crate::util::{clean_text, json_object, limit_param, opt_str, query_get};
use axum::body::Bytes;
use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use db::{ChannelId, ChannelKind, IncidentId, MonitorId, MonitorKind, NewChannel, NewMonitor};
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::{Value, json};
use std::sync::Arc;

pub(crate) const CHANNEL_TOKEN_PURPOSE: &str = "notification_channel_token";

fn monitor_json(row: &db::MonitorRow) -> Value {
    json!({
        "id": row.id.as_str(),
        "name": clean_text(&row.name),
        "kind": row.kind.as_str(),
        "target": clean_text(&row.target),
        "interval_sec": row.interval_sec,
        "timeout_sec": row.timeout_sec,
        "allow_private": row.allow_private,
        "is_active": row.is_active,
        "current_status": row.current_status.as_str(),
        "last_checked_at": row.last_checked_at,
        "created_at": row.created_at,
    })
}

fn incident_json(row: &db::IncidentRow) -> Value {
    json!({
        "id": row.id.as_str(),
        "monitor_id": row.monitor_id.as_str(),
        "started_at": row.started_at,
        "resolved_at": row.resolved_at,
        "cause": clean_text(&row.cause),
        "severity": row.severity.as_str(),
        "acknowledged_by": row.acknowledged_by.as_ref().map(db::UserId::as_str),
        "acknowledged_at": row.acknowledged_at,
    })
}

fn channel_json(row: &db::ChannelRow) -> Value {
    let config = serde_json::from_str::<Value>(&row.config_json).unwrap_or(Value::Null);
    json!({
        "id": row.id.as_str(),
        "kind": row.kind.as_str(),
        "name": clean_text(&row.name),
        "config": config,
        "has_secret": row.secret_id.is_some(),
        "is_active": row.is_active,
        "created_at": row.created_at,
    })
}

/// `GET /api/monitors`.
pub(crate) async fn monitors_list(State(app): State<Arc<App>>) -> Reply {
    let body = with_db(&app, |conn, app| {
        let rows = db::list_monitors(conn, &app.scope.org).map_err(|e| from_db(&e))?;
        Ok(Value::Array(rows.iter().map(monitor_json).collect()))
    })
    .await?;
    Ok(json_ok(&body))
}

/// `POST /api/monitors {"name", "kind": "http"|"tcp"|"tls", "target", "interval_sec"?, "timeout_sec"?, "allow_private"?}`.
pub(crate) async fn monitor_create(State(app): State<Arc<App>>, body: Bytes) -> Reply {
    let map = json_object(&body, false).ok_or(INVALID_INPUT)?;
    if map.keys().any(|k| {
        !matches!(
            k.as_str(),
            "name" | "kind" | "target" | "interval_sec" | "timeout_sec" | "allow_private"
        )
    }) {
        return Err(INVALID_INPUT);
    }
    let name = opt_str(&map, "name")
        .map_err(|()| INVALID_INPUT)?
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= 120)
        .ok_or(INVALID_INPUT)?
        .to_owned();
    let kind_str = opt_str(&map, "kind")
        .map_err(|()| INVALID_INPUT)?
        .ok_or(INVALID_INPUT)?;
    let kind = MonitorKind::parse(kind_str).map_err(|_| INVALID_INPUT)?;
    let target = opt_str(&map, "target")
        .map_err(|()| INVALID_INPUT)?
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= 500)
        .ok_or(INVALID_INPUT)?
        .to_owned();

    let interval_sec = match map.get("interval_sec") {
        None => 60u32,
        Some(v) => v
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| (10..=86400).contains(n))
            .ok_or(INVALID_INPUT)?,
    };
    let timeout_sec = match map.get("timeout_sec") {
        None => 10u32,
        Some(v) => v
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| (1..=60).contains(n))
            .ok_or(INVALID_INPUT)?,
    };
    let allow_private = match map.get("allow_private") {
        None => false,
        Some(v) => v.as_bool().ok_or(INVALID_INPUT)?,
    };

    let row = with_db(&app, move |conn, app| {
        let id = db::create_monitor(
            conn,
            &app.scope.org,
            &NewMonitor {
                name: &name,
                kind,
                target: &target,
                interval_sec,
                timeout_sec,
                allow_private,
            },
        )
        .map_err(|e| match e {
            db::Error::Sqlite(_) => ApiError::new("already_exists"),
            other => from_db(&other),
        })?;
        db::get_monitor(conn, &app.scope.org, &id).map_err(|e| from_db(&e))
    })
    .await?;
    Ok(json_status(StatusCode::CREATED, &monitor_json(&row)))
}

/// `DELETE /api/monitors/{id}`.
pub(crate) async fn monitor_delete(State(app): State<Arc<App>>, Path(id): Path<String>) -> Reply {
    let id = MonitorId::parse(&id).map_err(|_| NOT_FOUND)?;
    with_db(&app, move |conn, app| {
        db::delete_monitor(conn, &app.scope.org, &id).map_err(|e| from_db(&e))
    })
    .await?;
    Ok(no_content())
}

/// `GET /api/incidents?open=true|false&limit=`.
pub(crate) async fn incidents_list(
    State(app): State<Arc<App>>,
    RawQuery(query): RawQuery,
) -> Reply {
    let only_open = match query_get(query.as_deref(), "open").as_deref() {
        None => None,
        Some("true") => Some(true),
        Some("false") => Some(false),
        Some(_) => return Err(INVALID_INPUT),
    };
    let limit = limit_param(query.as_deref(), "limit", 100, 500).map_err(|()| INVALID_INPUT)?;
    let body = with_db(&app, move |conn, app| {
        let rows =
            db::list_incidents(conn, &app.scope.org, only_open, limit).map_err(|e| from_db(&e))?;
        Ok(Value::Array(rows.iter().map(incident_json).collect()))
    })
    .await?;
    Ok(json_ok(&body))
}

/// `POST /api/incidents/{id}/ack`.
pub(crate) async fn incident_ack(State(app): State<Arc<App>>, Path(id): Path<String>) -> Reply {
    let id = IncidentId::parse(&id).map_err(|_| NOT_FOUND)?;
    let id_for_db = id.clone();
    with_db(&app, move |conn, app| {
        db::acknowledge_incident(conn, &app.scope.org, &id_for_db, &app.scope.user)
            .map_err(|e| from_db(&e))
    })
    .await?;
    Ok(json_ok(&json!({"id": id.as_str(), "acknowledged": true})))
}

/// `GET /api/channels`.
pub(crate) async fn channels_list(State(app): State<Arc<App>>) -> Reply {
    let body = with_db(&app, |conn, app| {
        let rows = db::list_channels(conn, &app.scope.org).map_err(|e| from_db(&e))?;
        Ok(Value::Array(rows.iter().map(channel_json).collect()))
    })
    .await?;
    Ok(json_ok(&body))
}

/// `POST /api/channels {"kind": "telegram"|"webhook", "name", "config"?, "token"?}`.
pub(crate) async fn channel_create(State(app): State<Arc<App>>, body: Bytes) -> Reply {
    let map = json_object(&body, false).ok_or(INVALID_INPUT)?;
    if map
        .keys()
        .any(|k| !matches!(k.as_str(), "kind" | "name" | "config" | "token"))
    {
        return Err(INVALID_INPUT);
    }
    let kind_str = opt_str(&map, "kind")
        .map_err(|()| INVALID_INPUT)?
        .ok_or(INVALID_INPUT)?;
    let kind = ChannelKind::parse(kind_str).map_err(|_| INVALID_INPUT)?;
    let name = opt_str(&map, "name")
        .map_err(|()| INVALID_INPUT)?
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= 120)
        .ok_or(INVALID_INPUT)?
        .to_owned();
    let config_val = match map.get("config") {
        None => json!({}),
        Some(v) if v.is_object() => v.clone(),
        Some(_) => return Err(INVALID_INPUT),
    };
    let config_json = config_val.to_string();
    let token = opt_str(&map, "token")
        .map_err(|()| INVALID_INPUT)?
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);

    if token.is_some() && app.vault.is_none() {
        return Err(ApiError::new("master_key_missing"));
    }

    let row = with_db(&app, move |conn, app| {
        let org = &app.scope.org;
        let secret_id = match (token.as_deref(), app.vault.as_ref()) {
            (Some(tok), Some(vault)) => Some(
                vault
                    .store(conn, org, CHANNEL_TOKEN_PURPOSE, tok.as_bytes())
                    .map_err(|_| STORE_FAILED)?,
            ),
            _ => None,
        };
        let created = db::create_channel(
            conn,
            org,
            &NewChannel {
                kind,
                name: &name,
                config_json: &config_json,
                secret: secret_id.as_ref(),
            },
        );
        let id = match created {
            Ok(id) => id,
            Err(e) => {
                if let Some(sec) = &secret_id {
                    let _ = db::delete_secret(conn, org, sec);
                }
                return Err(match e {
                    db::Error::Sqlite(_) => ApiError::new("already_exists"),
                    other => from_db(&other),
                });
            }
        };
        db::get_channel(conn, org, &id).map_err(|e| from_db(&e))
    })
    .await?;
    Ok(json_status(StatusCode::CREATED, &channel_json(&row)))
}

/// `DELETE /api/channels/{id}`.
pub(crate) async fn channel_delete(State(app): State<Arc<App>>, Path(id): Path<String>) -> Reply {
    let id = ChannelId::parse(&id).map_err(|_| NOT_FOUND)?;
    with_db(&app, move |conn, app| {
        let org = &app.scope.org;
        let sec = db::delete_channel(conn, org, &id).map_err(|e| from_db(&e))?;
        if let Some(sec) = sec {
            let _ = db::delete_secret(conn, org, &sec);
        }
        Ok(())
    })
    .await?;
    Ok(no_content())
}

fn generate_link_code() -> Result<String, ApiError> {
    let rng = SystemRandom::new();
    let mut bytes = [0u8; 12];
    rng.fill(&mut bytes)
        .map_err(|_| ApiError::new("internal"))?;
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let code: String = bytes
        .iter()
        .map(|&b| char::from(ALPHABET[usize::from(b) % ALPHABET.len()]))
        .collect();
    Ok(code)
}

/// `POST /api/channels/telegram/link-code`: creates a one-time `/link <code>` for the signed-in
/// user (stored as SHA-256 hash, expires in 15 minutes).
pub(crate) async fn telegram_link_code(State(app): State<Arc<App>>) -> Reply {
    let code = generate_link_code()?;
    let hash = db::sha256_hex(code.as_bytes());
    let body = with_db(&app, move |conn, app| {
        let id = db::create_telegram_link(conn, &app.scope.org, &app.scope.user, &hash, "en")
            .map_err(|e| from_db(&e))?;
        Ok(json!({
            "id": id.as_str(),
            "code": code,
            "command": format!("/link {code}"),
        }))
    })
    .await?;
    Ok(json_status(StatusCode::CREATED, &body))
}
