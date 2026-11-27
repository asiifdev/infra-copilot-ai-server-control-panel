//! The layer every request passes, whatever it asks for (static file, API call, unknown path,
//! wrong method): it checks who may talk to the server, and it sets the security headers on
//! every answer. Order of the checks:
//!
//! 1. `Host` must be exactly the address the server listens on (DNS rebinding);
//! 2. an `Origin`, when sent, must be the server's own; a request that changes something
//!    (anything but GET and HEAD) must carry it, and the header `X-Nodivara: 1` (CSRF);
//! 3. every `/api` call except the token exchange needs a live session cookie;
//! 4. a body is read with a hard limit and must be `application/json`.
//!
//! Errors are rendered here, in the language of the request, so handlers deal in codes only.

use crate::app::App;
use crate::auth::session_cookie;
use crate::errors::{ApiError, ErrorMark, message};
use crate::util::locale_of;
use axum::body::{Body, to_bytes};
use axum::extract::{Request, State};
use axum::http::header::{
    CACHE_CONTROL, CONTENT_TYPE, COOKIE, HOST, HeaderName, HeaderValue, ORIGIN,
};
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use i18n::Locale;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

/// Largest request body.
pub(crate) const MAX_BODY_BYTES: usize = 64 * 1024;
/// The longest a request may take to produce its response (a stream goes on after that).
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// The Content-Security-Policy of every response (decision 0029).
pub(crate) const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; \
img-src 'self' data:; font-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none'; \
frame-ancestors 'none'";

const X_NODIVARA: HeaderName = HeaderName::from_static("x-nodivara");
const X_CONTENT_TYPE_OPTIONS: HeaderName = HeaderName::from_static("x-content-type-options");
const REFERRER_POLICY: HeaderName = HeaderName::from_static("referrer-policy");
const COOP: HeaderName = HeaderName::from_static("cross-origin-opener-policy");
const CSP_HEADER: HeaderName = HeaderName::from_static("content-security-policy");

/// The request language, put in the request extensions for handlers.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Lang(pub(crate) Locale);

fn is_api_path(path: &str) -> bool {
    path == "/api" || path.starts_with("/api/")
}

/// Exactly one header value, as text.
fn single<'a>(request: &'a Request, name: &HeaderName) -> Result<Option<&'a str>, ()> {
    let mut values = request.headers().get_all(name).iter();
    let first = values.next();
    if values.next().is_some() {
        return Err(());
    }
    match first {
        None => Ok(None),
        Some(value) => value.to_str().map(Some).map_err(|_| ()),
    }
}

fn json_content_type(value: &str) -> bool {
    let media = value.split(';').next().unwrap_or_default().trim();
    media.eq_ignore_ascii_case("application/json")
}

/// The checks of the module docs. `Ok` carries the request, with its body replaced by the bytes
/// that were read.
async fn check(app: &App, request: Request) -> Result<Request, ApiError> {
    let host = single(&request, &HOST).map_err(|()| ApiError::new("bad_host"))?;
    if host != Some(app.authority.as_str()) {
        return Err(ApiError::new("bad_host"));
    }
    let method = request.method().clone();
    let changes = !matches!(method, Method::GET | Method::HEAD);
    let path = request.uri().path().to_owned();
    let agent_route =
        method == Method::POST && (path == "/api/agents/poll" || path == "/api/agents/result");
    let origin = single(&request, &ORIGIN).map_err(|()| ApiError::new("bad_origin"))?;
    match origin {
        Some(origin) if origin != app.origin => return Err(ApiError::new("bad_origin")),
        None if changes && !agent_route => return Err(ApiError::new("bad_origin")),
        _ => {}
    }
    if changes && !agent_route {
        let marker = single(&request, &X_NODIVARA).map_err(|()| ApiError::new("csrf_header"))?;
        if marker != Some("1") {
            return Err(ApiError::new("csrf_header"));
        }
    }
    let exchange = method == Method::POST && path == "/api/auth/exchange";
    // The message catalog holds UI text only and no data of the organization. The sign-in page
    // needs it before there is a session; it still passes the Host and Origin checks above.
    let catalog = method == Method::GET && path.starts_with("/api/i18n/");
    if is_api_path(&path) && !exchange && !catalog && !agent_route {
        let cookie = single(&request, &COOKIE).map_err(|()| ApiError::new("unauthenticated"))?;
        let live = cookie
            .and_then(session_cookie)
            .is_some_and(|value| app.auth().check(value, app.clock.now()));
        if !live {
            return Err(ApiError::new("unauthenticated"));
        }
    }
    if !changes {
        return Ok(request);
    }
    let (parts, body) = request.into_parts();
    let bytes = to_bytes(body, MAX_BODY_BYTES)
        .await
        .map_err(|_| ApiError::new("body_too_large"))?;
    if !bytes.is_empty() {
        let kind = parts
            .headers
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(json_content_type);
        if !kind {
            return Err(ApiError::new("bad_content_type"));
        }
    }
    Ok(Request::from_parts(parts, Body::from(bytes)))
}

fn set_security_headers(response: &mut Response, api: bool) {
    let headers = response.headers_mut();
    headers.insert(CSP_HEADER, HeaderValue::from_static(CSP));
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    headers.insert(COOP, HeaderValue::from_static("same-origin"));
    if api {
        headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
}

/// An error with its JSON body, in `locale`.
fn error_response(app: &App, locale: Locale, status: StatusCode, code: &'static str) -> Response {
    let body = json!({"error": {"code": code, "message": message(&app.catalog, locale, code)}});
    let mut response = Response::new(Body::from(body.to_string()));
    *response.status_mut() = status;
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    // An error is never cached, wherever it came from.
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// The guard layer. See the module docs.
pub(crate) async fn guard(
    State(app): State<Arc<App>>,
    mut request: Request,
    next: Next,
) -> Response {
    let api = is_api_path(request.uri().path());
    let accept = request
        .headers()
        .get("accept-language")
        .and_then(|v| v.to_str().ok());
    let locale = locale_of(request.uri().query(), accept);
    request.extensions_mut().insert(Lang(locale));

    let outcome = tokio::time::timeout(REQUEST_TIMEOUT, async {
        match check(&app, request).await {
            Ok(request) => Ok(next.run(request).await),
            Err(error) => Err(error),
        }
    })
    .await;
    let mut response = match outcome {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => error.into_response(),
        Err(_) => ApiError::new("timeout").into_response(),
    };

    // Render the body of an error. Anything with an error status and no JSON of ours (a 405 or
    // a rejection produced by the router itself) becomes a coded error too.
    let status = response.status();
    let marked = response.extensions().get::<ErrorMark>().map(|m| m.0);
    let own_body = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|t| t.starts_with("application/json") || t.starts_with("text/event-stream"));
    let code = match marked {
        Some(code) => Some(code),
        None if (status.is_client_error() || status.is_server_error()) && !own_body => {
            Some(match status {
                StatusCode::NOT_FOUND => "not_found",
                StatusCode::METHOD_NOT_ALLOWED => "method_not_allowed",
                s if s.is_client_error() => "bad_request",
                _ => "internal",
            })
        }
        None => None,
    };
    if let Some(code) = code {
        // Headers that belong to the meaning of the error survive its re-rendering.
        let kept: Vec<_> = ["allow", "retry-after"]
            .iter()
            .filter_map(|name| response.headers().get(*name).cloned().map(|v| (*name, v)))
            .collect();
        response = error_response(&app, locale, status, code);
        for (name, value) in kept {
            response.headers_mut().insert(name, value);
        }
    }
    set_security_headers(&mut response, api);
    response
}
