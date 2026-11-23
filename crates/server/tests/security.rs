//! A hostile browser, page or process against the local server: sign-in, sessions, the Host,
//! Origin, header, content-type and size checks, the security headers on every kind of response,
//! static paths and the loopback-only rule. Real sockets, a real database file.

mod common;

use common::*;
use serde_json::json;
use server::{Config, ServeError, Server};

/// Every API route of the contract, as (method, path with a sample id).
const ROUTES: &[(&str, &str)] = &[
    ("GET", "/api/session"),
    ("POST", "/api/auth/logout"),
    ("GET", "/api/i18n/en"),
    ("GET", "/api/nodes"),
    ("GET", "/api/nodes/old/inventory"),
    ("GET", "/api/findings"),
    ("GET", "/api/plans"),
    ("GET", "/api/plans/x"),
    ("GET", "/api/transports"),
    ("GET", "/api/providers"),
    ("POST", "/api/providers"),
    ("DELETE", "/api/providers/main"),
    ("POST", "/api/providers/main/test"),
    ("GET", "/api/memories"),
    (
        "POST",
        "/api/memories/00000000-0000-4000-8000-000000000000/approve",
    ),
    (
        "DELETE",
        "/api/memories/00000000-0000-4000-8000-000000000000",
    ),
    ("GET", "/api/proposals"),
    ("GET", "/api/conversations"),
    ("POST", "/api/conversations"),
    (
        "GET",
        "/api/conversations/00000000-0000-4000-8000-000000000000/messages",
    ),
    (
        "POST",
        "/api/conversations/00000000-0000-4000-8000-000000000000/turn",
    ),
];

fn request_for(method: &str, path: &str) -> Req {
    Req::new(method, path)
}

// ---- sign-in -----------------------------------------------------------------------------------

#[test]
fn a_token_works_exactly_once() -> TestResult {
    let world = World::new()?;
    let server = TestServer::start(&world)?;
    let token = server.tokens[0].clone();
    assert_eq!(exchange(server.addr, &token).status, 204);
    let again = exchange(server.addr, &token);
    assert_eq!(again.status, 401);
    assert_eq!(again.code(), "invalid_token");
    Ok(())
}

#[test]
fn the_token_is_never_stored_in_clear_and_only_the_fragment_carries_it() -> TestResult {
    let world = World::new()?;
    let server = TestServer::start(&world)?;
    // The token appears in no response and no database file; a wrong one is refused.
    let wrong = exchange(server.addr, &"0".repeat(64));
    assert_eq!(wrong.status, 401);
    assert!(!contains(&world.raw_bytes(), &server.tokens[0]));
    assert!(!wrong.text().contains(&server.tokens[0]));
    Ok(())
}

#[test]
fn a_token_expires_after_ten_minutes() -> TestResult {
    let world = World::new()?;
    let server = TestServer::start(&world)?;
    server.clock.advance(601);
    let r = exchange(server.addr, &server.tokens[0]);
    assert_eq!((r.status, r.code().as_str()), (401, "invalid_token"));
    Ok(())
}

#[test]
fn a_token_just_inside_its_lifetime_still_works() -> TestResult {
    let world = World::new()?;
    let server = TestServer::start(&world)?;
    server.clock.advance(590);
    assert_eq!(exchange(server.addr, &server.tokens[0]).status, 204);
    Ok(())
}

#[test]
fn repeated_wrong_tokens_destroy_the_real_one() -> TestResult {
    let world = World::new()?;
    let server = TestServer::start(&world)?;
    for _ in 0..5 {
        let wrong = format!("{:064x}", 7);
        assert_eq!(exchange(server.addr, &wrong).status, 401);
    }
    // Even the right token no longer works: the server has to be restarted.
    assert_eq!(exchange(server.addr, &server.tokens[0]).status, 401);
    Ok(())
}

#[test]
fn guessing_is_rate_limited() -> TestResult {
    let world = World::new()?;
    let server = TestServer::start(&world)?;
    let mut statuses = Vec::new();
    for i in 0..14 {
        statuses.push(exchange(server.addr, &format!("{i:064x}")).status);
    }
    assert!(statuses[..10].iter().all(|s| *s == 401), "{statuses:?}");
    assert!(statuses[10..].iter().all(|s| *s == 429), "{statuses:?}");
    let limited = exchange(server.addr, &server.tokens[0]);
    assert_eq!(limited.status, 429);
    assert_eq!(limited.header("retry-after"), Some("60"));
    assert_eq!(limited.code(), "rate_limited");
    // After the window the counter is free again (the destroyed token stays destroyed).
    server.clock.advance(61);
    assert_eq!(exchange(server.addr, &server.tokens[0]).status, 401);
    Ok(())
}

#[test]
fn malformed_exchange_bodies_are_refused() -> TestResult {
    let world = World::new()?;
    let server = TestServer::start(&world)?;
    for body in [
        json!({}),
        json!({"token": 5}),
        json!({"token": "a", "extra": 1}),
        json!([1]),
        json!({"token": "x".repeat(300)}),
    ] {
        let r = Req::post("/api/auth/exchange")
            .same_origin(server.addr)
            .json(&body)
            .send(server.addr);
        assert_eq!(r.status, 400, "{body}");
    }
    let r = Req::post("/api/auth/exchange")
        .same_origin(server.addr)
        .header("Content-Type", "application/json")
        .body(b"{not json")
        .send(server.addr);
    assert_eq!(r.status, 400);
    Ok(())
}

// ---- sessions ----------------------------------------------------------------------------------

#[test]
fn the_session_cookie_has_exactly_the_documented_attributes() -> TestResult {
    let world = World::new()?;
    let server = TestServer::start(&world)?;
    let r = exchange(server.addr, &server.tokens[0]);
    let cookie = r.header("set-cookie").ok_or("no cookie")?;
    let (pair, attributes) = cookie.split_once("; ").ok_or("no attributes")?;
    let value = pair.strip_prefix("nv_session=").ok_or("wrong name")?;
    assert_eq!(value.len(), 64);
    assert!(value.bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(attributes, "HttpOnly; SameSite=Strict; Path=/");
    assert!(!cookie.to_ascii_lowercase().contains("secure"));
    assert!(!cookie.to_ascii_lowercase().contains("domain"));
    assert_eq!(r.header("cache-control"), Some("no-store"));
    Ok(())
}

#[test]
fn forged_cookies_never_work() -> TestResult {
    let world = World::new()?;
    let server = TestServer::start(&world)?;
    let real = server.sign_in()?;
    let value = real.cookie.trim_start_matches("nv_session=").to_owned();
    for cookie in [
        "nv_session=".to_owned(),
        format!("nv_session={}", "0".repeat(64)),
        format!("nv_session={}", value.to_uppercase()),
        format!("nv_session={}", &value[..63]),
        format!("nv_session={value}0"),
        "nv_session=' OR 1=1 --".to_owned(),
        format!("other={value}"),
        format!("nv_session_x={value}"),
    ] {
        let r = Req::get("/api/session").cookie(&cookie).send(server.addr);
        assert_eq!(r.status, 401, "{cookie}");
        assert_eq!(r.code(), "unauthenticated");
    }
    // The real one still works, also among other cookies.
    let r = Req::get("/api/session")
        .cookie(&format!("a=1; {}; b=2", real.cookie))
        .send(server.addr);
    assert_eq!(r.status, 200);
    Ok(())
}

#[test]
fn a_session_of_another_server_instance_is_worthless() -> TestResult {
    let world = World::new()?;
    let first = TestServer::start(&world)?;
    let second = TestServer::start(&world)?;
    let stolen = first.sign_in()?;
    let r = Req::get("/api/session")
        .cookie(&stolen.cookie)
        .send(second.addr);
    assert_eq!(r.status, 401);
    Ok(())
}

#[test]
fn a_session_ends_after_eight_idle_hours_and_activity_keeps_it() -> TestResult {
    let world = World::new()?;
    let server = TestServer::start(&world)?;
    let session = server.sign_in()?;
    let hour = 3600;
    server.clock.advance(7 * hour);
    assert_eq!(session.get("/api/session").status, 200);
    server.clock.advance(7 * hour);
    assert_eq!(session.get("/api/session").status, 200);
    server.clock.advance(8 * hour + 1);
    let r = session.get("/api/session");
    assert_eq!((r.status, r.code().as_str()), (401, "unauthenticated"));
    Ok(())
}

#[test]
fn a_sixth_session_drops_the_least_recently_used() -> TestResult {
    let world = World::new()?;
    let server = TestServer::start_with(
        &world,
        Opts {
            tokens: 5,
            ..Opts::default()
        },
    )?;
    let mut sessions = Vec::new();
    for token in &server.tokens {
        server.clock.advance(1);
        sessions.push(server.sign_in_with(token)?);
    }
    assert!(sessions.iter().all(|s| s.get("/api/session").status == 200));
    // Use all but the first, so the first is the least recently used one, then sign in a sixth.
    server.clock.advance(1);
    for session in &sessions[1..] {
        assert_eq!(session.get("/api/session").status, 200);
    }
    let sixth = server.sign_in_with(&server.login.issue_token()?)?;
    assert_eq!(sessions[0].get("/api/session").status, 401);
    for session in sessions[1..].iter().chain([&sixth]) {
        assert_eq!(session.get("/api/session").status, 200);
    }
    Ok(())
}

#[test]
fn logout_ends_the_session_and_clears_the_cookie() -> TestResult {
    let world = World::new()?;
    let server = TestServer::start(&world)?;
    let session = server.sign_in()?;
    let r = session.post_empty("/api/auth/logout");
    assert_eq!(r.status, 204);
    let cookie = r.header("set-cookie").ok_or("no cookie")?;
    assert!(cookie.contains("Max-Age=0"), "{cookie}");
    assert!(cookie.starts_with("nv_session=;"), "{cookie}");
    assert_eq!(session.get("/api/session").status, 401);
    Ok(())
}

#[test]
fn every_api_route_needs_a_session() -> TestResult {
    let world = World::new()?;
    let server = TestServer::start(&world)?;
    for (method, path) in ROUTES {
        if path.starts_with("/api/i18n/") {
            continue; // the one public API route, see `the_catalog_is_public_but_not_unguarded`
        }
        let r = request_for(method, path)
            .same_origin(server.addr)
            .json(&json!({}))
            .send(server.addr);
        assert_eq!(r.status, 401, "{method} {path}");
        assert_eq!(r.code(), "unauthenticated", "{method} {path}");
    }
    // Also an unknown API path: nothing is revealed to a stranger.
    assert_eq!(Req::get("/api/nope").send(server.addr).status, 401);
    // The health check and the page need none.
    assert_eq!(Req::get("/healthz").send(server.addr).status, 200);
    assert_eq!(
        Req::get("/")
            .header("Accept", "text/html")
            .send(server.addr)
            .status,
        200
    );
    Ok(())
}

// ---- Host, Origin and the custom header ----------------------------------------------------------

#[test]
fn a_wrong_host_is_refused_on_every_route() -> TestResult {
    let world = World::new()?;
    let server = TestServer::start(&world)?;
    let session = server.sign_in()?;
    let port = server.addr.port();
    let hosts = [
        "evil.example".to_owned(),
        format!("localhost:{port}"),
        format!("127.0.0.1:{}", port.wrapping_add(1)),
        "127.0.0.1".to_owned(),
        format!("127.0.0.1:{port}.evil.example"),
        format!("evil.example:{port}"),
        String::new(),
    ];
    let mut routes: Vec<(&str, &str)> = ROUTES.to_vec();
    routes.extend([
        ("GET", "/healthz"),
        ("GET", "/"),
        ("GET", "/assets/app.js"),
        ("GET", "/nodes/old"),
        ("GET", "/api/unknown"),
        ("GET", "/../etc/passwd"),
        ("PUT", "/api/nodes"),
    ]);
    for host in &hosts {
        for (method, path) in &routes {
            let r = session.req(
                Req::new(method, path)
                    .host(host)
                    .same_origin(server.addr)
                    .json(&json!({})),
            );
            assert_eq!(r.status, 403, "Host {host:?} {method} {path}");
            assert_eq!(r.code(), "bad_host");
            assert_security_headers(&r, true);
        }
    }
    // No Host at all, and two of them.
    let none = session.req(Req::get("/healthz").no_host());
    assert_eq!(none.status, 403);
    let two = raw(
        server.addr,
        format!(
            "GET /healthz HTTP/1.1\r\nHost: {}\r\nHost: evil.example\r\nConnection: close\r\n\r\n",
            server.addr
        )
        .as_bytes(),
    );
    assert_eq!(two.status, 403);
    Ok(())
}

#[test]
fn a_foreign_origin_cannot_change_anything() -> TestResult {
    let world = World::new()?;
    let server = TestServer::start(&world)?;
    let session = server.sign_in()?;
    for origin in [
        "http://evil.example",
        "https://127.0.0.1",
        &format!("http://localhost:{}", server.addr.port()),
        &format!("http://127.0.0.1:{}", server.addr.port().wrapping_add(1)),
        &format!("https://{}", server.addr),
        "null",
    ] {
        let r = session.req(
            Req::post("/api/conversations")
                .header("Origin", origin)
                .header("X-Nodivara", "1")
                .json(&json!({})),
        );
        assert_eq!(r.status, 403, "{origin}");
        assert_eq!(r.code(), "bad_origin");
    }
    // No Origin on a changing request is refused too.
    let r = session.req(
        Req::post("/api/conversations")
            .header("X-Nodivara", "1")
            .json(&json!({})),
    );
    assert_eq!((r.status, r.code().as_str()), (403, "bad_origin"));
    // Nothing was created by any of them.
    assert_eq!(session.get("/api/conversations").json(), json!([]));
    // A cross-origin read is refused as well, even with a valid cookie.
    let r = session.req(Req::get("/api/nodes").header("Origin", "http://evil.example"));
    assert_eq!(r.status, 403);
    Ok(())
}

#[test]
fn the_custom_header_is_required_for_every_changing_request() -> TestResult {
    let world = World::new()?;
    let server = TestServer::start(&world)?;
    let session = server.sign_in()?;
    let origin = format!("http://{}", server.addr);
    for marker in [None, Some("0"), Some(""), Some("true"), Some("11")] {
        let mut request = Req::post("/api/conversations")
            .header("Origin", &origin)
            .json(&json!({}));
        if let Some(value) = marker {
            request = request.header("X-Nodivara", value);
        }
        let r = session.req(request);
        assert_eq!(
            (r.status, r.code().as_str()),
            (403, "csrf_header"),
            "{marker:?}"
        );
    }
    let r = session.req(Req::new("DELETE", "/api/providers/main").header("Origin", &origin));
    assert_eq!(r.status, 403);
    Ok(())
}

#[test]
fn a_body_must_be_json_and_not_too_large() -> TestResult {
    let world = World::new()?;
    let server = TestServer::start(&world)?;
    let session = server.sign_in()?;
    for content_type in [
        "text/plain",
        "application/x-www-form-urlencoded",
        "multipart/form-data; boundary=x",
        "application/jsonp",
        "text/json",
    ] {
        let r = session.req(
            Req::post("/api/conversations")
                .same_origin(server.addr)
                .header("Content-Type", content_type)
                .body(b"{}"),
        );
        assert_eq!(r.status, 415, "{content_type}");
        assert_eq!(r.code(), "bad_content_type");
    }
    // A body with no content type at all.
    let r = session.req(
        Req::post("/api/conversations")
            .same_origin(server.addr)
            .body(b"{}"),
    );
    assert_eq!(r.status, 415);
    // Charset parameters and case are fine.
    let r = session.req(
        Req::post("/api/conversations")
            .same_origin(server.addr)
            .header("Content-Type", "Application/JSON; charset=utf-8")
            .body(b"{}"),
    );
    assert_eq!(r.status, 201);
    // Over 64 KiB is refused without being parsed.
    let big = format!(r#"{{"title":"{}"}}"#, "a".repeat(70 * 1024));
    let r = session.req(
        Req::post("/api/conversations")
            .same_origin(server.addr)
            .header("Content-Type", "application/json")
            .body(big.as_bytes()),
    );
    assert_eq!((r.status, r.code().as_str()), (413, "body_too_large"));
    assert_security_headers(&r, true);
    Ok(())
}

// ---- headers on every kind of response ------------------------------------------------------------

#[test]
fn the_security_headers_are_on_every_kind_of_response() -> TestResult {
    let world = World::new()?;
    let server = TestServer::start(&world)?;
    let session = server.sign_in()?;
    let a = server.addr;
    let html = [("Accept", "text/html")];
    // Static: the page, a client-side route, a 404, a 405.
    let page = Req::get("/").header(html[0].0, html[0].1).send(a);
    assert_eq!(page.status, 200);
    assert_security_headers(&page, false);
    assert!(
        page.header("content-type")
            .is_some_and(|t| t.starts_with("text/html"))
    );
    let route = Req::get("/nodes/old").header("Accept", "text/html").send(a);
    assert_security_headers(&route, false);
    let missing = Req::get("/assets/x.js").send(a);
    assert_eq!(missing.status, 404);
    assert_security_headers(&missing, false);
    let method = Req::post("/").same_origin(a).json(&json!({})).send(a);
    assert_eq!(method.status, 405);
    assert_security_headers(&method, false);
    // Health.
    assert_security_headers(&Req::get("/healthz").send(a), false);
    // API: success, no content, errors of every layer.
    assert_security_headers(&session.get("/api/nodes"), true);
    assert_security_headers(&session.post_empty("/api/auth/logout"), true);
    assert_security_headers(&Req::get("/api/nodes").send(a), true);
    assert_security_headers(&exchange(a, &"1".repeat(64)), true);
    assert_security_headers(&Req::get("/api/nodes").host("evil.example").send(a), true);
    Ok(())
}

#[test]
fn errors_are_localized_json_with_a_fixed_shape() -> TestResult {
    let world = World::new()?;
    let server = TestServer::start(&world)?;
    let r = Req::get("/api/nodes?lang=id").send(server.addr);
    let body = r.json();
    assert_eq!(body["error"]["code"], "unauthenticated");
    assert!(
        body["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("Anda belum masuk")),
        "{body}"
    );
    assert_eq!(body.as_object().map(|o| o.len()), Some(1));
    assert_eq!(
        r.header("content-type"),
        Some("application/json; charset=utf-8")
    );
    let en = Req::get("/api/nodes").send(server.addr).json();
    assert!(
        en["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("not signed in"))
    );
    Ok(())
}

#[test]
fn unknown_paths_and_methods_are_json_errors() -> TestResult {
    let world = World::new()?;
    let server = TestServer::start(&world)?;
    let session = server.sign_in()?;
    let r = session.get("/api/does/not/exist");
    assert_eq!((r.status, r.code().as_str()), (404, "not_found"));
    assert_security_headers(&r, true);
    // Even a browser navigation to an unknown API path gets JSON, never the page.
    let r = session.req(Req::get("/api/zzz").header("Accept", "text/html"));
    assert_eq!(r.status, 404);
    assert_eq!(
        r.header("content-type"),
        Some("application/json; charset=utf-8")
    );
    // Trailing slash and case are different paths.
    assert_eq!(session.get("/api/nodes/").status, 404);
    assert_eq!(session.get("/API/nodes").status, 404);
    for (method, path) in [
        ("PUT", "/api/nodes"),
        ("POST", "/api/nodes"),
        ("PATCH", "/api/session"),
        ("DELETE", "/api/session"),
        ("PUT", "/api/providers"),
        ("POST", "/api/proposals"),
        ("DELETE", "/api/plans/x"),
        ("GET", "/api/auth/logout"),
        ("GET", "/api/auth/exchange"),
    ] {
        let r = session.req(
            Req::new(method, path)
                .same_origin(server.addr)
                .json(&json!({})),
        );
        assert_eq!(r.status, 405, "{method} {path}");
        assert_eq!(r.code(), "method_not_allowed");
        assert_security_headers(&r, true);
    }
    // There is no way to approve or run a proposal or a plan.
    for path in [
        "/api/proposals/x/approve",
        "/api/proposals/x/run",
        "/api/plans/x/approve",
        "/api/plans/x/run",
        "/api/plans/x/schedule",
    ] {
        let r = session.post(path, &json!({}));
        assert_eq!(r.status, 404, "{path}");
    }
    Ok(())
}

// ---- static files ---------------------------------------------------------------------------------

#[test]
fn static_paths_cannot_escape_or_reach_the_source_tree() -> TestResult {
    let world = World::new()?;
    let server = TestServer::start(&world)?;
    let paths = [
        "/../Cargo.toml",
        "/../../Cargo.toml",
        "/%2e%2e/Cargo.toml",
        "/%2e%2e/%2e%2e/Cargo.toml",
        "/..%2fCargo.toml",
        "/..%2f..%2f..%2fetc/passwd",
        "/assets/../../Cargo.toml",
        "/assets/..\\..\\Cargo.toml",
        "/..\\Cargo.toml",
        "/%5c..%5cCargo.toml",
        "/%00",
        "/index.html%00.txt",
        "/./index.html",
        "//etc/passwd",
        "/....//Cargo.toml",
        "/web/dist/../../Cargo.toml",
        "/crates/server/Cargo.toml",
        "/Cargo.toml",
    ];
    for path in paths {
        for accept in ["*/*", "text/html"] {
            let r = Req::get(path).header("Accept", accept).send(server.addr);
            let body = r.text();
            assert!(
                !body.contains("[package]") && !body.contains("root:"),
                "{path} leaked: {body}"
            );
            assert!(matches!(r.status, 200 | 400 | 404), "{path}: {}", r.status);
            assert_security_headers(&r, false);
        }
    }
    // A raw request line that hyper would not build from a URL: backslashes and a NUL byte.
    let r = raw(
        server.addr,
        format!(
            "GET /a\\..\\Cargo.toml HTTP/1.1\r\nHost: {}\r\nAccept: text/html\r\nConnection: close\r\n\r\n",
            server.addr
        )
        .as_bytes(),
    );
    assert!(!r.text().contains("[package]"));
    Ok(())
}

#[test]
fn the_single_page_app_fallback_is_only_for_html_navigations() -> TestResult {
    let world = World::new()?;
    let server = TestServer::start(&world)?;
    let a = server.addr;
    let nav = Req::get("/nodes/old")
        .header("Accept", "text/html,application/xhtml+xml")
        .send(a);
    assert_eq!(nav.status, 200);
    assert!(
        nav.header("content-type")
            .is_some_and(|t| t.starts_with("text/html"))
    );
    assert_eq!(nav.header("cache-control"), Some("no-cache"));
    let script = Req::get("/nodes/old").header("Accept", "*/*").send(a);
    assert_eq!((script.status, script.code().as_str()), (404, "not_found"));
    let plain = Req::get("/nodes/old").send(a);
    assert_eq!(plain.status, 404);
    // HEAD works like GET without a body.
    let head = Req::new("HEAD", "/").header("Accept", "text/html").send(a);
    assert_eq!(head.status, 200);
    assert!(head.body.is_empty());
    Ok(())
}

#[test]
fn the_placeholder_page_is_localized_and_has_no_inline_script_or_style() -> TestResult {
    // The page served at `/` depends on whether `web/dist` existed when the server was compiled:
    // without it a localized placeholder, with it the real UI (whose own index must also carry
    // no inline script or style, the CSP forbids them).
    let ui_built = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../web/dist/index.html")
        .exists();
    let world = World::new()?;
    let server = TestServer::start(&world)?;
    let en = Req::get("/")
        .header("Accept", "text/html")
        .send(server.addr)
        .text();
    if ui_built {
        assert!(en.contains("<div id=\"app\""), "{en}");
        assert!(!en.contains("scripts/build-web.sh"));
        assert!(
            !en.contains("style="),
            "inline style attribute in the UI index"
        );
        // Every script in the index is an external file of this origin.
        for part in en.split("<script").skip(1) {
            let tag = part.split('>').next().unwrap_or_default();
            assert!(tag.contains("src=\"/"), "inline script: <script{tag}>");
        }
        return Ok(());
    }
    assert!(en.contains("scripts/build-web.sh") && en.contains("lang=\"en\""));
    let id = Req::get("/?lang=id")
        .header("Accept", "text/html")
        .send(server.addr)
        .text();
    assert!(id.contains("Antarmuka web") && id.contains("lang=\"id\""));
    for page in [en, id] {
        assert!(!page.contains("<script") && !page.contains("<style") && !page.contains("style="));
    }
    Ok(())
}

// ---- loopback only ---------------------------------------------------------------------------------

fn bind_result(
    world: &World,
    addr: &str,
) -> Result<Result<Server, ServeError>, Box<dyn std::error::Error>> {
    let mut cfg: Config = config(world, &Opts::default(), &server::Clock::system())?;
    cfg.bind = addr.parse()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    Ok(runtime.block_on(Server::bind(cfg)))
}

#[test]
fn non_loopback_addresses_are_refused() -> TestResult {
    let world = World::new()?;
    for addr in [
        "0.0.0.0:0",
        "[::]:0",
        "192.168.1.10:0",
        "10.0.0.1:0",
        "8.8.8.8:0",
    ] {
        let result = bind_result(&world, addr)?;
        assert!(
            matches!(result, Err(ServeError::NotLoopback(_))),
            "{addr} was accepted"
        );
    }
    let ok = bind_result(&world, "127.0.0.1:0")?;
    assert!(ok.is_ok());
    Ok(())
}

#[test]
fn a_busy_port_is_reported_as_such() -> TestResult {
    let world = World::new()?;
    let first = TestServer::start(&world)?;
    let busy = format!("127.0.0.1:{}", first.addr.port());
    let result = bind_result(&world, &busy)?;
    assert!(matches!(result, Err(ServeError::PortBusy(p)) if p == first.addr.port()));
    Ok(())
}

#[test]
fn the_server_stops_when_told_to() -> TestResult {
    let world = World::new()?;
    let server = TestServer::start(&world)?;
    let addr = server.addr;
    assert_eq!(Req::get("/healthz").send(addr).status, 200);
    server.stop()?;
    assert_eq!(Req::get("/healthz").send(addr).status, 0);
    Ok(())
}

/// The sign-in page needs its text before any session exists, so the message catalog is the one
/// public API route. It must not become a way around the other checks, and it must hold no data
/// of the organization.
#[test]
fn the_catalog_is_public_but_not_unguarded() -> TestResult {
    let world = World::new()?;
    let server = TestServer::start(&world)?;
    let ok = Req::get("/api/i18n/en").send(server.addr);
    assert_eq!(ok.status, 200);
    assert!(ok.header("content-security-policy").is_some());
    assert_eq!(ok.header("cache-control"), Some("no-store"));
    // Still refused for a foreign Host (DNS rebinding) and a foreign Origin.
    assert_eq!(
        Req::get("/api/i18n/en")
            .host("evil.example.test")
            .send(server.addr)
            .status,
        403
    );
    assert_eq!(
        Req::get("/api/i18n/en")
            .header("Origin", "http://evil.example.test")
            .send(server.addr)
            .status,
        403
    );
    // Only UI text: no key from outside the `ui.` area, and nothing from the database.
    let map = ok.json();
    let object = map.as_object().ok_or("an object")?;
    assert!(
        object.keys().all(|k| k.starts_with("ui.")),
        "{:?}",
        object.keys().next()
    );
    let text = ok.text();
    assert!(!text.contains(&api_key()) && !text.contains(&master_key()));
    // Other API routes and unknown API paths still need a session.
    assert_eq!(Req::get("/api/nodes").send(server.addr).status, 401);
    assert_eq!(Req::get("/api/i18n").send(server.addr).status, 401);
    Ok(())
}
