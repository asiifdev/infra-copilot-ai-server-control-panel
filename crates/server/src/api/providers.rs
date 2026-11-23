//! AI providers: list, add, remove and the connection test.
//!
//! The API key of a provider goes in (`POST`), is sealed by the vault and is never returned: not
//! in a list, not in an error, not in a test result. Without the master key the server refuses to
//! take a key (`409 master_key_missing`) but keyless providers work.

use super::{Reply, json_ok, json_status, no_content, with_db};
use crate::app::App;
use crate::errors::{
    ApiError, INTERNAL, INVALID_INPUT, NOT_FOUND, STORE_FAILED, ai_code, from_db, load_code,
    message,
};
use crate::guard::Lang;
use crate::util::{clean_text, has_control, json_object, opt_str};
use agent::{DbProviderLoader, ProviderLoader};
use ai::{AiProvider, ChatRequest, Message, NoSink};
use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Instant;

/// Longest key accepted (as in the command line).
const MAX_KEY_BYTES: usize = 1024;
/// Tokens a connection test may ask the model for.
const TEST_MAX_TOKENS: u32 = 16;

fn provider_json(row: &db::AiProvider) -> Value {
    let mut params: Value = serde_json::from_str(&row.params_json).unwrap_or_else(|_| json!({}));
    // Settings hold no secrets by design; the second lock costs nothing.
    crate::util::redact_json(&mut params);
    json!({
        "name": clean_text(&row.name),
        "base_url": clean_text(&row.base_url),
        "model": clean_text(&row.default_model),
        "params": params,
        "has_key": row.secret_id.is_some(),
        "is_default": row.is_default,
    })
}

/// `GET /api/providers`.
pub(crate) async fn providers_list(State(app): State<Arc<App>>) -> Reply {
    let body = with_db(&app, |conn, app| {
        let rows = db::list_ai_providers(conn, &app.scope.org).map_err(|e| from_db(&e))?;
        Ok(Value::Array(rows.iter().map(provider_json).collect()))
    })
    .await?;
    Ok(json_ok(&body))
}

/// A provider to add, checked field by field. The key is held only until it is sealed.
struct Input {
    name: String,
    base_url: String,
    model: String,
    params_json: String,
    key: Option<String>,
    is_default: bool,
}

fn parse_input(body: &[u8]) -> Result<Input, ApiError> {
    let map = json_object(body, false).ok_or(INVALID_INPUT)?;
    const FIELDS: [&str; 6] = ["name", "base_url", "model", "params", "key", "is_default"];
    if map.keys().any(|k| !FIELDS.contains(&k.as_str())) {
        return Err(INVALID_INPUT);
    }
    let text = |field: &str, max: usize| -> Result<String, ApiError> {
        match opt_str(&map, field) {
            Ok(Some(t)) if !t.trim().is_empty() && t.len() <= max && !has_control(t) => {
                Ok(t.trim().to_owned())
            }
            _ => Err(INVALID_INPUT),
        }
    };
    let name = text("name", 100)?;
    let base_url = text("base_url", 500)?;
    let model = text("model", 200)?;
    let params_json = match map.get("params") {
        None | Some(Value::Null) => "{}".to_owned(),
        Some(value @ Value::Object(_)) => value.to_string(),
        Some(_) => return Err(INVALID_INPUT),
    };
    if params_json.len() > 4096 {
        return Err(INVALID_INPUT);
    }
    let key = match opt_str(&map, "key").map_err(|()| INVALID_INPUT)? {
        None => None,
        Some(key) => {
            let key = key.trim();
            if key.len() > MAX_KEY_BYTES || has_control(key) {
                return Err(INVALID_INPUT);
            }
            (!key.is_empty()).then(|| key.to_owned())
        }
    };
    let is_default = match map.get("is_default") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(_) => return Err(INVALID_INPUT),
    };
    Ok(Input {
        name,
        base_url,
        model,
        params_json,
        key,
        is_default,
    })
}

/// `POST /api/providers {name, base_url, model, params?, key?, is_default?}`.
pub(crate) async fn provider_add(State(app): State<Arc<App>>, body: Bytes) -> Reply {
    let input = parse_input(&body)?;
    // The master key matters only when there is a key to seal.
    if input.key.is_some() && app.vault.is_none() {
        return Err(ApiError::new("master_key_missing"));
    }
    bootstrap::validate_provider_config(&input.base_url, &input.model, &input.params_json)
        .map_err(|_| ApiError::new("invalid_provider"))?;
    let row = with_db(&app, move |conn, app| {
        let org = &app.scope.org;
        let key = input.key.as_deref().zip(app.vault.as_deref());
        bootstrap::add_provider(
            conn,
            org,
            &bootstrap::NewProvider {
                name: &input.name,
                base_url: &input.base_url,
                model: &input.model,
                params_json: &input.params_json,
                is_default: input.is_default,
            },
            key.map(|(key, vault)| (vault, key)),
        )
        .map_err(|error| match error {
            bootstrap::ProviderError::DefaultExists => ApiError::new("default_exists"),
            bootstrap::ProviderError::Exists => ApiError::new("already_exists"),
            bootstrap::ProviderError::Store { .. } => STORE_FAILED,
        })?;
        let rows = db::list_ai_providers(conn, org).map_err(|e| from_db(&e))?;
        rows.iter()
            .find(|r| r.name == input.name)
            .map(provider_json)
            .ok_or(STORE_FAILED)
    })
    .await?;
    Ok(json_status(StatusCode::CREATED, &row))
}

/// `DELETE /api/providers/{name}`.
pub(crate) async fn provider_remove(
    State(app): State<Arc<App>>,
    Path(name): Path<String>,
) -> Reply {
    with_db(&app, move |conn, app| {
        bootstrap::remove_provider(conn, &app.scope.org, &name).map_err(|error| match error {
            bootstrap::RemoveError::Unknown => NOT_FOUND,
            bootstrap::RemoveError::InUse => ApiError::new("provider_in_use"),
            bootstrap::RemoveError::Store { .. } => STORE_FAILED,
        })
    })
    .await?;
    Ok(no_content())
}

/// How a connection test ended: the model on success, or the code of the failure.
enum Tested {
    Ok { latency_ms: u128, model: String },
    Failed(&'static str),
}

/// Runs the tiny test turn. Everything is built inside the call because it runs on a blocking
/// thread with its own runtime (a connection and a provider are not shared between threads).
fn run_test(conn: &db::Connection, app: &App, name: &str) -> Result<Tested, ApiError> {
    let org = &app.scope.org;
    let rows = db::list_ai_providers(conn, org).map_err(|e| from_db(&e))?;
    let row = rows.iter().find(|r| r.name == name).ok_or(NOT_FOUND)?;
    let vault = if row.secret_id.is_some() {
        match &app.vault {
            Some(vault) => Arc::clone(vault),
            None => return Ok(Tested::Failed("master_key_missing")),
        }
    } else {
        Arc::new(bootstrap::placeholder_vault().map_err(|_| INTERNAL)?)
    };
    let provider = match DbProviderLoader::new(conn, &vault).load(org, Some(&row.id)) {
        Ok(provider) => provider,
        Err(error) => return Ok(Tested::Failed(load_code(&error))),
    };
    // No tools are offered and the answer is capped: this only proves the address, the key and
    // the model work.
    let request = ChatRequest {
        messages: vec![
            Message::system("This is a connection test. Reply with the single word: ok"),
            Message::user("ping"),
        ],
        tools: Vec::new(),
        max_tokens: Some(TEST_MAX_TOKENS),
        temperature: None,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| INTERNAL)?;
    let started = Instant::now();
    let outcome = runtime.block_on(tokio::time::timeout(
        app.provider_test_timeout,
        provider.chat(&request, &mut NoSink),
    ));
    Ok(match outcome {
        Ok(Ok(_)) => Tested::Ok {
            latency_ms: started.elapsed().as_millis(),
            model: row.default_model.clone(),
        },
        Ok(Err(error)) => Tested::Failed(ai_code(&error)),
        Err(_) => Tested::Failed("provider_timeout"),
    })
}

/// `POST /api/providers/{name}/test`.
pub(crate) async fn provider_test(
    State(app): State<Arc<App>>,
    Extension(lang): Extension<Lang>,
    Path(name): Path<String>,
) -> Reply {
    let tested = with_db(&app, move |conn, app| run_test(conn, app, &name)).await?;
    Ok(json_ok(&match tested {
        Tested::Ok { latency_ms, model } => {
            json!({"ok": true, "latency_ms": u64::try_from(latency_ms).unwrap_or(u64::MAX), "model": clean_text(&model)})
        }
        Tested::Failed(code) => {
            json!({"ok": false, "code": code, "message": message(&app.catalog, lang.0, code)})
        }
    }))
}
