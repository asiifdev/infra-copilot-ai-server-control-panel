//! Health, sign-in, sign-out, the session description and the UI texts.

use super::{Reply, json_ok, json_status, no_content, with_db};
use crate::app::App;
use crate::auth::{COOKIE, ExchangeError, session_cookie};
use crate::errors::{ApiError, INVALID_INPUT, NOT_FOUND, from_db};
use crate::guard::Lang;
use crate::util::json_object;
use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::header::{COOKIE as COOKIE_HEADER, RETRY_AFTER, SET_COOKIE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use i18n::Locale;
use serde_json::{Map, Value, json};
use std::sync::Arc;

/// Longest sign-in token accepted (the real one is 64 characters).
const MAX_TOKEN_CHARS: usize = 256;

/// `GET /healthz`: no sign-in needed (the `Host` check still applies).
pub(crate) async fn healthz() -> Reply {
    Ok(json_ok(&json!({"status": "ok"})))
}

fn cookie_header(value: &str, clear: bool) -> Option<HeaderValue> {
    let attributes = if clear { "; Max-Age=0" } else { "" };
    HeaderValue::from_str(&format!(
        "{COOKIE}={value}{attributes}; HttpOnly; SameSite=Strict; Path=/"
    ))
    .ok()
}

/// `POST /api/auth/exchange {token}`: trades the one-time token for a session cookie.
pub(crate) async fn exchange(State(app): State<Arc<App>>, body: Bytes) -> Reply {
    let map = json_object(&body, false).ok_or(INVALID_INPUT)?;
    let token = match map.get("token") {
        Some(Value::String(token)) if token.len() <= MAX_TOKEN_CHARS && map.len() == 1 => token,
        _ => return Err(INVALID_INPUT),
    };
    let outcome = app.auth().exchange(token, app.clock.now());
    match outcome {
        Ok(session) => {
            let mut response = no_content();
            let cookie = cookie_header(&session, false).ok_or(crate::errors::INTERNAL)?;
            response.headers_mut().insert(SET_COOKIE, cookie);
            Ok(response)
        }
        Err(ExchangeError::Invalid) => Err(ApiError::new("invalid_token")),
        Err(ExchangeError::RateLimited) => {
            let mut response = ApiError::new("rate_limited").into_response();
            response
                .headers_mut()
                .insert(RETRY_AFTER, HeaderValue::from_static("60"));
            Ok(response)
        }
        Err(ExchangeError::Random) => Err(crate::errors::INTERNAL),
    }
}

/// `POST /api/auth/logout`: ends the session and clears the cookie.
pub(crate) async fn logout(State(app): State<Arc<App>>, headers: HeaderMap) -> Reply {
    if let Some(value) = headers
        .get(COOKIE_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(session_cookie)
    {
        app.auth().logout(value);
    }
    let mut response = no_content();
    if let Some(cookie) = cookie_header("", true) {
        response.headers_mut().insert(SET_COOKIE, cookie);
    }
    Ok(response)
}

/// `GET /api/session`.
pub(crate) async fn session(
    State(app): State<Arc<App>>,
    Extension(lang): Extension<Lang>,
) -> Reply {
    let configured = app.vault.is_some();
    let value = with_db(&app, |conn, app| {
        let org = db::get_org(conn, &app.scope.org).map_err(|e| from_db(&e))?;
        let user =
            db::get_member_user(conn, &app.scope.org, &app.scope.user).map_err(|e| from_db(&e))?;
        Ok(json!({
            "user": {"email": user.email, "name": user.name},
            "org": {"name": org.name, "slug": org.slug},
        }))
    })
    .await?;
    let Value::Object(mut map) = value else {
        return Err(crate::errors::INTERNAL);
    };
    map.insert("locale".to_owned(), json!(lang.0.tag()));
    map.insert("master_key_configured".to_owned(), json!(configured));
    map.insert(
        "execution_enabled".to_owned(),
        json!(app.executor.is_some()),
    );
    map.insert("version".to_owned(), json!(env!("CARGO_PKG_VERSION")));
    Ok(json_ok(&Value::Object(map)))
}

/// `GET /api/i18n/{locale}`: the `ui.*` texts as a flat map of raw templates (placeholders are
/// `{name}` and `{{` `}}` are literal braces). A key missing in the locale falls back to English,
/// as everywhere else.
pub(crate) async fn i18n_catalog(State(app): State<Arc<App>>, Path(tag): Path<String>) -> Reply {
    if tag.len() > 16 {
        return Err(NOT_FOUND);
    }
    let Some(locale) = Locale::parse(&tag) else {
        return Err(NOT_FOUND);
    };
    let mut map = Map::new();
    for key in app
        .catalog
        .keys(Locale::En)
        .filter(|k| k.starts_with("ui."))
    {
        let text = app
            .catalog
            .raw(locale, key)
            .or_else(|| app.catalog.raw(Locale::En, key));
        if let Some(text) = text {
            map.insert(key.to_owned(), Value::String(text.to_owned()));
        }
    }
    Ok(json_status(StatusCode::OK, &Value::Object(map)))
}
