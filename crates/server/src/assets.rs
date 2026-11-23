//! The web UI, compiled into the binary from `web/dist` (it may be absent: the build then has a
//! placeholder page).
//!
//! Lookup is by exact name in the embedded table, so there is no file system path to escape from;
//! names with `..`, a backslash or a NUL are refused before the lookup anyway. A path that is
//! not a file falls back to `index.html` (the app routes client-side) only for a GET that asks
//! for `text/html` and is not under `/api`.

use crate::app::App;
use crate::errors::{ApiError, NOT_FOUND};
use crate::guard::Lang;
use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, ETAG, IF_NONE_MATCH};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use i18n::MessageKey;
use rust_embed::RustEmbed;
use std::borrow::Cow;
use std::sync::Arc;

const NOT_BUILT: MessageKey = MessageKey::new("api.static.not_built");
const NOT_BUILT_TITLE: MessageKey = MessageKey::new("api.static.title");

#[derive(RustEmbed)]
#[folder = "../../web/dist"]
#[allow_missing = true]
struct Dist;

/// The content type of a file by its extension.
fn content_type(path: &str) -> &'static str {
    let extension = path.rsplit_once('.').map_or("", |(_, e)| e);
    match extension.to_ascii_lowercase().as_str() {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "webmanifest" => "application/manifest+json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Whether a request path may be looked up at all.
fn safe_name(name: &str) -> bool {
    !name.contains(['\0', '\\']) && name.split('/').all(|part| part != ".." && part != ".")
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn respond(status: StatusCode, content_type: &'static str, body: Body) -> Response {
    let mut response = Response::new(body);
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
}

/// A file of the table as a response, with the right type and cache headers.
fn serve_file<E: RustEmbed>(name: &str, request: &Request) -> Option<Response> {
    let file = E::get(name)?;
    let etag = format!("\"{}\"", hex(&file.metadata.sha256_hash()));
    let unchanged = request
        .headers()
        .get(IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|candidate| candidate.trim() == etag));
    // Hashed build output never changes under its name; everything else is revalidated.
    let cache = if name.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    let mut response = if unchanged {
        let mut r = Response::new(Body::empty());
        *r.status_mut() = StatusCode::NOT_MODIFIED;
        r
    } else {
        let body = match file.data {
            Cow::Borrowed(data) => Bytes::from_static(data),
            Cow::Owned(data) => Bytes::from(data),
        };
        respond(StatusCode::OK, content_type(name), Body::from(body))
    };
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static(cache));
    if let Ok(value) = HeaderValue::from_str(&etag) {
        response.headers_mut().insert(ETAG, value);
    }
    Some(response)
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// What `/` shows when the binary was built without the UI.
fn placeholder(app: &App, lang: Lang) -> Response {
    let title = escape(&app.catalog.get(lang.0, NOT_BUILT_TITLE, &[]).text);
    let text = escape(&app.catalog.get(lang.0, NOT_BUILT, &[]).text);
    let page = format!(
        "<!doctype html><html lang=\"{}\"><head><meta charset=\"utf-8\"><title>{title}</title></head>\
         <body><h1>{title}</h1><p>{text}</p></body></html>",
        lang.0.tag()
    );
    let mut response = respond(StatusCode::OK, "text/html; charset=utf-8", Body::from(page));
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
}

fn wants_html(request: &Request) -> bool {
    request
        .headers()
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|accept| accept.to_ascii_lowercase().contains("text/html"))
}

/// What a path resolves to.
enum Resolved {
    /// A file, or the app shell for a client-side route.
    Served(Response),
    /// A client-side route, but the build has no `index.html`.
    NoIndex,
    /// Nothing.
    NotFound,
}

/// Resolves `path` (of a GET or HEAD) in the embedded table `E`.
fn resolve<E: RustEmbed>(path: &str, request: &Request) -> Resolved {
    let name = path.trim_start_matches('/');
    let name = if name.is_empty() { "index.html" } else { name };
    if !safe_name(name) {
        return Resolved::NotFound;
    }
    if let Some(response) = serve_file::<E>(name, request) {
        return Resolved::Served(response);
    }
    if wants_html(request) {
        return serve_file::<E>("index.html", request).map_or(Resolved::NoIndex, Resolved::Served);
    }
    Resolved::NotFound
}

/// Everything no API route matched: the UI's files, the client-side routes, or a 404.
pub(crate) async fn fallback(
    State(app): State<Arc<App>>,
    axum::Extension(lang): axum::Extension<Lang>,
    request: Request,
) -> Response {
    let path = request.uri().path().to_owned();
    if path == "/api" || path.starts_with("/api/") {
        return NOT_FOUND.into_response();
    }
    if !matches!(*request.method(), Method::GET | Method::HEAD) {
        return ApiError::new("method_not_allowed").into_response();
    }
    match resolve::<Dist>(&path, &request) {
        Resolved::Served(response) => response,
        Resolved::NoIndex => placeholder(&app, lang),
        Resolved::NotFound => NOT_FOUND.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request as HttpRequest;

    /// A small build output: an app shell, a hashed asset and an icon.
    #[derive(RustEmbed)]
    #[folder = "tests/fixtures/shell"]
    struct Fixture;

    /// A folder without an `index.html`.
    #[derive(RustEmbed)]
    #[folder = "tests/fixtures/no-index"]
    struct NoIndex;

    fn get(path: &str, headers: &[(&str, &str)]) -> Request {
        let mut builder = HttpRequest::builder().uri(path);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        builder.body(Body::empty()).unwrap_or_default()
    }

    fn served(outcome: Resolved) -> Option<Response> {
        match outcome {
            Resolved::Served(response) => Some(response),
            Resolved::NoIndex | Resolved::NotFound => None,
        }
    }

    fn header<'a>(response: &'a Response, name: &str) -> &'a str {
        response
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
    }

    #[test]
    fn files_get_their_type_and_hashed_assets_are_cached_for_a_year() {
        let path = "/assets/app-abc123.js";
        let Some(r) = served(resolve::<Fixture>(path, &get(path, &[]))) else {
            unreachable!("asset is embedded")
        };
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(header(&r, "content-type"), "text/javascript; charset=utf-8");
        assert_eq!(
            header(&r, "cache-control"),
            "public, max-age=31536000, immutable"
        );
        assert!(header(&r, "etag").starts_with('"'));
        let Some(page) = served(resolve::<Fixture>("/", &get("/", &[]))) else {
            unreachable!("index is embedded")
        };
        assert_eq!(header(&page, "content-type"), "text/html; charset=utf-8");
        assert_eq!(header(&page, "cache-control"), "no-cache");
        let icon = "/favicon.svg";
        let Some(icon) = served(resolve::<Fixture>(icon, &get(icon, &[]))) else {
            unreachable!("icon is embedded")
        };
        assert_eq!(header(&icon, "content-type"), "image/svg+xml");
        assert_eq!(header(&icon, "cache-control"), "no-cache");
    }

    #[test]
    fn an_unchanged_file_is_answered_with_304() {
        let path = "/index.html";
        let Some(first) = served(resolve::<Fixture>(path, &get(path, &[]))) else {
            unreachable!("index is embedded")
        };
        let etag = header(&first, "etag").to_owned();
        let request = get(path, &[("if-none-match", &etag)]);
        let Some(again) = served(resolve::<Fixture>(path, &request)) else {
            unreachable!("index is embedded")
        };
        assert_eq!(again.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(header(&again, "etag"), etag);
        let request = get(path, &[("if-none-match", "\"other\"")]);
        let stale = served(resolve::<Fixture>(path, &request));
        assert_eq!(stale.map(|r| r.status()), Some(StatusCode::OK));
    }

    #[test]
    fn only_html_navigations_fall_back_to_the_app_shell() {
        let nav = [("accept", "text/html,application/xhtml+xml")];
        let shell = served(resolve::<Fixture>("/nodes/old", &get("/nodes/old", &nav)));
        let Some(shell) = shell else {
            unreachable!("falls back")
        };
        assert_eq!(header(&shell, "content-type"), "text/html; charset=utf-8");
        assert!(matches!(
            resolve::<Fixture>("/nodes/old", &get("/nodes/old", &[("accept", "*/*")])),
            Resolved::NotFound
        ));
        assert!(matches!(
            resolve::<Fixture>("/assets/missing.js", &get("/assets/missing.js", &[])),
            Resolved::NotFound
        ));
        assert!(matches!(
            resolve::<NoIndex>("/anything", &get("/anything", &nav)),
            Resolved::NoIndex
        ));
    }

    #[test]
    fn traversal_never_reaches_the_table() {
        for path in [
            "/../Cargo.toml",
            "/assets/../index.html",
            "/a\\b",
            "/a\0b",
            "/./index.html",
        ] {
            let nav = [("accept", "text/html")];
            assert!(
                !matches!(
                    resolve::<Fixture>(path, &get("/", &nav)),
                    Resolved::Served(_)
                ),
                "{path}"
            );
        }
    }

    #[test]
    fn names_with_traversal_are_refused() {
        assert!(safe_name("assets/app.js"));
        assert!(!safe_name("../secret"));
        assert!(!safe_name("a/../b"));
        assert!(!safe_name("a\\b"));
        assert!(!safe_name("a\0b"));
        assert!(!safe_name("./a"));
    }

    #[test]
    fn content_types_follow_the_extension() {
        assert_eq!(content_type("a.js"), "text/javascript; charset=utf-8");
        assert_eq!(content_type("a.CSS"), "text/css; charset=utf-8");
        assert_eq!(content_type("a.svg"), "image/svg+xml");
        assert_eq!(content_type("noext"), "application/octet-stream");
    }
}
