//! Providers, the connection test and the assistant turn stream, against a fake OpenAI-compatible
//! provider on 127.0.0.1: a provider key goes in and never comes out, every provider failure
//! becomes a localized advice code, and one turn runs per conversation.

mod common;

use common::*;
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};
use vault::Vault;

struct Setup {
    world: World,
    fake: Fake,
    server: TestServer,
    session: Session,
}

/// A provider `main` (default) pointing at a fake with `script`, sealed with the master key when
/// `with_key`; the server has the vault when `vault_on`.
fn setup(
    script: Vec<Reply>,
    with_key: bool,
    vault_on: bool,
) -> Result<Setup, Box<dyn std::error::Error>> {
    let world = World::new()?;
    node(&world.conn()?, &world.org, "old")?;
    let fake = Fake::start(script)?;
    let v = vault()?;
    let key = api_key();
    add_provider(&world, &fake, with_key.then_some((&*v, key.as_str())), true)?;
    let server = TestServer::start_with(
        &world,
        Opts {
            vault: vault_on.then_some(v),
            ..Opts::default()
        },
    )?;
    let session = server.sign_in()?;
    Ok(Setup {
        world,
        fake,
        server,
        session,
    })
}

fn new_conversation(s: &Setup) -> Result<String, Box<dyn std::error::Error>> {
    let r = s.session.post("/api/conversations", &json!({"title": "t"}));
    assert_eq!(r.status, 201, "{}", r.text());
    Ok(r.json()["id"].as_str().ok_or("id")?.to_owned())
}

fn turn(s: &Setup, id: &str, message: &str) -> Http {
    s.session.post(
        &format!("/api/conversations/{id}/turn"),
        &json!({"message": message}),
    )
}

fn kinds(events: &[(String, Value)]) -> Vec<&str> {
    events.iter().map(|(e, _)| e.as_str()).collect()
}

// ---- providers -----------------------------------------------------------------------------------

#[test]
fn a_provider_key_cannot_be_saved_without_the_master_key() -> TestResult {
    let world = World::new()?;
    let server = TestServer::start(&world)?;
    let session = server.sign_in()?;
    let body = json!({"name": "main", "base_url": "https://api.example.test/v1", "model": "m", "key": api_key()});
    let r = session.post("/api/providers", &body);
    assert_eq!((r.status, r.code().as_str()), (409, "master_key_missing"));
    assert!(
        r.json()["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("NODIVARA_MASTER_KEY"))
    );
    let id = session.post("/api/providers?lang=id", &body);
    assert!(
        id.json()["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("Kunci utama"))
    );
    assert!(!r.text().contains(&api_key()));
    // Nothing was stored, not even a half-made provider.
    assert_eq!(session.get("/api/providers").json(), json!([]));
    assert!(!contains(&world.raw_bytes(), &api_key()));
    // A provider without a key (a local model) needs no master key.
    let keyless = json!({"name": "local", "base_url": "http://127.0.0.1:11434/v1", "model": "m",
        "params": {"allow_private_network": true}, "is_default": true});
    let r = session.post("/api/providers", &keyless);
    assert_eq!(r.status, 201, "{}", r.text());
    let list = session.get("/api/providers").json();
    assert_eq!(list[0]["name"], "local");
    assert_eq!(list[0]["has_key"], false);
    assert_eq!(list[0]["is_default"], true);
    assert_eq!(list[0]["params"], json!({"allow_private_network": true}));
    assert_eq!(list[0]["model"], "m");
    Ok(())
}

#[test]
fn the_provider_key_goes_in_and_never_comes_out() -> TestResult {
    let world = World::new()?;
    node(&world.conn()?, &world.org, "old")?;
    let fake = Fake::start(vec![
        text_reply(&["ok"]),
        tool_reply("c1", "list_nodes", "{}"),
        text_reply(&["The server old is pending.\n"]),
    ])?;
    let server = TestServer::start_with(
        &world,
        Opts {
            vault: Some(vault()?),
            ..Opts::default()
        },
    )?;
    let session = server.sign_in()?;
    let key = api_key();
    let r = session.post(
        "/api/providers",
        &json!({"name": "main", "base_url": fake.base_url(), "model": "test-model",
                "params": {"allow_private_network": true, "max_retries": 0}, "key": key, "is_default": true}),
    );
    assert_eq!(r.status, 201, "{}", r.text());
    assert_eq!(r.json()["has_key"], true);
    let mut seen = vec![r.text(), r.header_text()];
    // Use it: the connection test and a turn that calls a tool.
    let tested = session.post_empty("/api/providers/main/test");
    assert_eq!(tested.json()["ok"], true, "{}", tested.text());
    seen.push(tested.text());
    seen.push(tested.header_text());
    let conversation = session
        .post("/api/conversations", &json!({"title": "t"}))
        .json()["id"]
        .as_str()
        .ok_or("id")?
        .to_owned();
    let answered = session.post(
        &format!("/api/conversations/{conversation}/turn"),
        &json!({"message": "which servers?"}),
    );
    assert_eq!(answered.status, 200);
    seen.push(answered.text());
    seen.push(answered.header_text());
    // The key reached the provider (it is used)...
    let requests = fake.requests();
    assert_eq!(requests.len(), 3);
    assert!(
        requests
            .iter()
            .all(|r| r.contains(&format!("Bearer {key}"))),
        "key not sent"
    );
    // ...and appears nowhere on our side: not in any response or header, not in any listing, not
    // in the database files, in clear.
    for path in [
        "/api/providers",
        "/api/session",
        "/api/nodes",
        "/api/memories",
        "/api/conversations",
        "/api/proposals",
        "/api/transports",
        "/api/plans",
        "/api/findings",
        &format!("/api/conversations/{conversation}/messages"),
    ] {
        let r = session.get(path);
        seen.push(r.text());
        seen.push(r.header_text());
    }
    for text in &seen {
        assert!(!text.contains(&key), "key leaked: {text}");
        assert!(!text.contains(&key[3..]), "key body leaked");
    }
    assert!(!contains(&world.raw_bytes(), &key));
    assert!(!contains(&world.raw_bytes(), &key[3..]));
    // The sealed form is there.
    let sealed: i64 = world.conn()?.query_row(
        "SELECT COUNT(*) FROM secrets WHERE purpose = 'ai_provider_key'",
        [],
        |r| r.get(0),
    )?;
    assert_eq!(sealed, 1);
    Ok(())
}

#[test]
fn bad_provider_input_is_refused_without_echoing_it() -> TestResult {
    let world = World::new()?;
    let server = TestServer::start_with(
        &world,
        Opts {
            vault: Some(vault()?),
            ..Opts::default()
        },
    )?;
    let session = server.sign_in()?;
    let ok = json!({"name": "a", "base_url": "https://api.example.test/v1", "model": "m"});
    let mut variants: Vec<Value> = Vec::new();
    for (field, value) in [
        ("name", json!("")),
        ("name", json!(5)),
        ("name", json!("x".repeat(101))),
        ("name", json!("a\nb")),
        ("base_url", json!("")),
        ("model", json!(null)),
        ("params", json!("text")),
        ("params", json!([1])),
        ("key", json!(5)),
        ("key", json!("k".repeat(1025))),
        ("key", json!("a\u{1}b")),
        ("is_default", json!("yes")),
        ("extra", json!(1)),
        ("org_id", json!("x")),
    ] {
        let mut body = ok.clone();
        body[field] = value;
        variants.push(body);
    }
    for body in &variants {
        let r = session.post("/api/providers", body);
        assert_eq!(r.status, 400, "{body}");
        assert!(r.code() == "invalid_input", "{} {body}", r.text());
    }
    for url in [
        "http://example.test/v1",
        "ftp://example.test",
        "https://user:pass@example.test/v1",
        "not a url",
    ] {
        let mut body = ok.clone();
        body["base_url"] = json!(url);
        let r = session.post("/api/providers", &body);
        assert_eq!(
            (r.status, r.code().as_str()),
            (400, "invalid_provider"),
            "{url}"
        );
        assert!(!r.text().contains("pass"));
    }
    let r = Req::post("/api/providers")
        .same_origin(server.addr)
        .cookie(&session.cookie)
        .header("Content-Type", "application/json")
        .body(b"{oops")
        .send(server.addr);
    assert_eq!(r.status, 400);
    assert_eq!(session.get("/api/providers").json(), json!([]));
    Ok(())
}

#[test]
fn duplicates_and_a_second_default_are_conflicts_and_leave_no_key_behind() -> TestResult {
    let world = World::new()?;
    let server = TestServer::start_with(
        &world,
        Opts {
            vault: Some(vault()?),
            ..Opts::default()
        },
    )?;
    let session = server.sign_in()?;
    let body = |name: &str, default: bool| {
        json!({"name": name, "base_url": "https://api.example.test/v1",
        "model": "m", "key": api_key(), "is_default": default})
    };
    assert_eq!(session.post("/api/providers", &body("a", true)).status, 201);
    let dup = session.post("/api/providers", &body("a", false));
    assert_eq!((dup.status, dup.code().as_str()), (409, "already_exists"));
    let second = session.post("/api/providers", &body("b", true));
    assert_eq!(
        (second.status, second.code().as_str()),
        (409, "default_exists")
    );
    let secrets: i64 = world
        .conn()?
        .query_row("SELECT COUNT(*) FROM secrets", [], |r| r.get(0))?;
    assert_eq!(
        secrets, 1,
        "a refused provider must not leave its sealed key"
    );
    assert_eq!(
        session.post("/api/providers", &body("c", false)).status,
        201
    );
    Ok(())
}

#[test]
fn a_provider_can_be_removed_with_its_key_unless_a_conversation_uses_it() -> TestResult {
    let world = World::new()?;
    let server = TestServer::start_with(
        &world,
        Opts {
            vault: Some(vault()?),
            ..Opts::default()
        },
    )?;
    let session = server.sign_in()?;
    for name in ["a", "b"] {
        let r = session.post("/api/providers", &json!({"name": name, "base_url": "https://api.example.test/v1", "model": "m", "key": api_key()}));
        assert_eq!(r.status, 201);
    }
    let conn = world.conn()?;
    let b = db::list_ai_providers(&conn, &world.org)?
        .into_iter()
        .find(|p| p.name == "b")
        .ok_or("b")?;
    db::create_conversation(&conn, &world.org, &world.user, "uses b", Some(&b.id))?;
    let r = session.delete("/api/providers/b");
    assert_eq!((r.status, r.code().as_str()), (409, "provider_in_use"));
    assert_eq!(session.delete("/api/providers/a").status, 204);
    assert_eq!(session.delete("/api/providers/a").status, 404);
    let secrets: i64 = conn.query_row("SELECT COUNT(*) FROM secrets", [], |r| r.get(0))?;
    assert_eq!(secrets, 1, "removing a provider removes its sealed key");
    Ok(())
}

#[test]
fn another_orgs_provider_is_a_missing_one() -> TestResult {
    let world = World::new()?;
    let fake = Fake::start(vec![])?;
    let conn = world.conn()?;
    bootstrap::add_provider(
        &conn,
        &world.rival,
        &bootstrap::NewProvider {
            name: "rivalprov",
            base_url: &fake.base_url(),
            model: "m",
            params_json: PARAMS,
            is_default: true,
        },
        None,
    )
    .map_err(|e| format!("{e:?}"))?;
    let server = TestServer::start(&world)?;
    let session = server.sign_in()?;
    assert_eq!(session.get("/api/providers").json(), json!([]));
    assert_eq!(
        session.post_empty("/api/providers/rivalprov/test").status,
        404
    );
    assert_eq!(session.delete("/api/providers/rivalprov").status, 404);
    assert!(
        fake.requests().is_empty(),
        "nothing may be sent to another org's provider"
    );
    Ok(())
}

// ---- the connection test --------------------------------------------------------------------------

#[test]
fn the_connection_test_asks_for_a_tiny_answer_without_tools() -> TestResult {
    let s = setup(vec![text_reply(&["ok"])], false, false)?;
    let r = s.session.post_empty("/api/providers/main/test");
    assert_eq!(r.status, 200);
    let body = r.json();
    assert_eq!(body["ok"], true);
    assert_eq!(body["model"], "test-model");
    assert!(body["latency_ms"].is_u64());
    assert_eq!(body.as_object().map(serde_json::Map::len), Some(3));
    let sent = s.fake.requests();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].contains("\"max_tokens\":16"), "{}", sent[0]);
    assert!(!sent[0].contains("\"tools\""), "{}", sent[0]);
    assert_eq!(s.session.post_empty("/api/providers/nope/test").status, 404);
    Ok(())
}

#[test]
fn every_provider_failure_becomes_a_localized_advice_code() -> TestResult {
    let cases: Vec<(&str, Reply)> = vec![
        (
            "ai_auth",
            Reply::Status(
                401,
                r#"{"error":{"message":"bad key https://secret.example/x"}}"#,
                vec![],
            ),
        ),
        ("ai_auth", Reply::Status(403, "{}", vec![])),
        (
            "ai_rate_limited",
            Reply::Status(429, "{}", vec![("Retry-After", "5")]),
        ),
        ("ai_server", Reply::Status(500, "{}", vec![])),
        ("ai_server", Reply::Status(503, "{}", vec![])),
        ("ai_rejected", Reply::Status(400, "{}", vec![])),
        (
            "ai_empty",
            Reply::Sse(vec![
                json!({"choices":[{"delta":{},"finish_reason":"stop"}]}),
            ]),
        ),
        ("ai_malformed", Reply::Garbage),
    ];
    for (code, reply) in cases {
        let s = setup(vec![reply], false, false)?;
        let r = s.session.post_empty("/api/providers/main/test");
        let body = r.json();
        assert_eq!(r.status, 200, "{code}");
        assert_eq!(body["ok"], false, "{code}: {body}");
        assert_eq!(body["code"], code, "{body}");
        let message = body["message"].as_str().ok_or("message")?;
        assert!(
            !message.is_empty() && !message.starts_with("api.error"),
            "{code}: {message}"
        );
        // Nothing the provider said, no URL and no address of it, comes back.
        let text = r.text();
        assert!(
            !text.contains("secret.example") && !text.contains("127.0.0.1"),
            "{text}"
        );
        let id = s.session.post_empty("/api/providers/main/test?lang=id");
        assert_ne!(
            id.json()["message"],
            body["message"],
            "{code} is not localized"
        );
    }
    Ok(())
}

#[test]
fn an_unreachable_provider_is_a_transport_failure() -> TestResult {
    let world = World::new()?;
    let dead = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        listener.local_addr()?.port()
    };
    let conn = world.conn()?;
    bootstrap::add_provider(
        &conn,
        &world.org,
        &bootstrap::NewProvider {
            name: "main",
            base_url: &format!("http://127.0.0.1:{dead}/v1"),
            model: "m",
            params_json: PARAMS,
            is_default: true,
        },
        None,
    )
    .map_err(|e| format!("{e:?}"))?;
    let server = TestServer::start(&world)?;
    let session = server.sign_in()?;
    let body = session.post_empty("/api/providers/main/test").json();
    assert_eq!(
        (body["ok"].as_bool(), body["code"].as_str()),
        (Some(false), Some("ai_transport"))
    );
    Ok(())
}

#[test]
fn the_test_names_the_master_key_when_it_is_missing_or_wrong() -> TestResult {
    // The key was sealed with the master key; the server has none.
    let s = setup(vec![text_reply(&["ok"])], true, false)?;
    let body = s.session.post_empty("/api/providers/main/test").json();
    assert_eq!(
        (body["ok"].as_bool(), body["code"].as_str()),
        (Some(false), Some("master_key_missing"))
    );
    assert!(s.fake.requests().is_empty());
    // The server has a different master key.
    let world = World::new()?;
    let fake = Fake::start(vec![text_reply(&["ok"])])?;
    let key = api_key();
    add_provider(&world, &fake, Some((&*vault()?, key.as_str())), true)?;
    let other = vault_with(&format!("other:{}", "cd".repeat(32)))?;
    let server = TestServer::start_with(
        &world,
        Opts {
            vault: Some(other),
            ..Opts::default()
        },
    )?;
    let session = server.sign_in()?;
    let body = session.post_empty("/api/providers/main/test").json();
    assert_eq!(body["code"], "provider_key_mismatch");
    assert!(fake.requests().is_empty());
    Ok(())
}

fn vault_with(key: &str) -> Result<std::sync::Arc<Vault>, Box<dyn std::error::Error>> {
    Ok(std::sync::Arc::new(Vault::new(key, &[])?))
}

// ---- the turn stream ------------------------------------------------------------------------------

#[test]
fn a_turn_streams_tool_calls_then_text_then_exactly_one_done() -> TestResult {
    let s = setup(
        vec![
            tool_reply("call_1", "list_nodes", "{}"),
            text_reply(&["The server old ", "is pending.\n", "Nothing else."]),
        ],
        true,
        true,
    )?;
    let id = new_conversation(&s)?;
    let r = turn(&s, &id, "Which servers do I have?");
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(
        r.header("content-type"),
        Some("text/event-stream; charset=utf-8")
    );
    assert_eq!(r.header("cache-control"), Some("no-store"));
    assert_security_headers(&r, true);
    let events = sse_events(&r.text());
    let order = kinds(&events);
    assert_eq!(
        order.iter().filter(|k| **k == "done").count(),
        1,
        "{order:?}"
    );
    assert_eq!(order.last(), Some(&"done"));
    assert!(!order.contains(&"error"), "{order:?}");
    let first_delta = order.iter().position(|k| *k == "delta").ok_or("no delta")?;
    let first_tool = order.iter().position(|k| *k == "tool").ok_or("no tool")?;
    assert!(
        first_tool < first_delta,
        "the tool call comes before the text that follows it: {order:?}"
    );
    let (_, tool) = &events[first_tool];
    assert_eq!(tool["name"], "list_nodes");
    assert_eq!(tool["tier"], 0);
    assert_eq!(tool["status"], "ok");
    assert!(tool["duration_ms"].is_number());
    let text: String = events
        .iter()
        .filter(|(e, _)| e == "delta")
        .filter_map(|(_, d)| d["text"].as_str())
        .collect();
    assert_eq!(text, "The server old is pending.\nNothing else.");
    let (_, done) = events.last().ok_or("empty")?;
    assert_eq!(done["rounds"], 2);
    assert_eq!(done["tool_calls"], 1);
    assert!(done["usage"]["tokens"].as_u64().is_some_and(|t| t > 0));
    // The conversation now holds the question and the answer.
    let messages = s
        .session
        .get(&format!("/api/conversations/{id}/messages"))
        .json();
    let roles: Vec<&str> = messages
        .as_array()
        .ok_or("messages")?
        .iter()
        .filter_map(|m| m["role"].as_str())
        .collect();
    assert_eq!(roles, ["user", "assistant", "assistant"]);
    assert_eq!(messages[0]["content"], "Which servers do I have?");
    assert_eq!(messages[1]["tools"], json!(["list_nodes"]));
    assert_eq!(
        messages[2]["content"],
        "The server old is pending.\nNothing else."
    );
    // The turn released the conversation: another one can run (the fake then fails it).
    assert_ne!(turn(&s, &id, "again").status, 409);
    Ok(())
}

#[test]
fn a_secret_in_the_models_text_never_reaches_the_stream() -> TestResult {
    let leaked = format!("{}{}", "sk-", "Qw9Er5Ty".repeat(4));
    let line = format!("My key is {leaked} ok\n");
    let s = setup(vec![text_reply(&[line.as_str(), "fine"])], false, false)?;
    let id = new_conversation(&s)?;
    let r = turn(&s, &id, "hello");
    let body = r.text();
    assert!(
        !body.contains(&leaked) && !body.contains(&leaked[3..]),
        "{body}"
    );
    assert_eq!(kinds(&sse_events(&body)).last(), Some(&"done"));
    let stored = s
        .session
        .get(&format!("/api/conversations/{id}/messages"))
        .text();
    assert!(!stored.contains(&leaked[3..]));
    Ok(())
}

#[test]
fn a_provider_failure_is_an_error_event_and_no_invented_reply() -> TestResult {
    let s = setup(vec![Reply::Status(401, "{}", vec![])], false, false)?;
    let id = new_conversation(&s)?;
    let r = s.session.post(
        &format!("/api/conversations/{id}/turn?lang=id"),
        &json!({"message": "hello"}),
    );
    assert_eq!(r.status, 200);
    let events = sse_events(&r.text());
    assert_eq!(kinds(&events), ["error"], "{}", r.text());
    assert_eq!(events[0].1["code"], "ai_auth");
    assert!(
        events[0].1["message"]
            .as_str()
            .is_some_and(|m| m.contains("Penyedia menolak"))
    );
    assert_eq!(events[0].1.as_object().map(serde_json::Map::len), Some(2));
    // Only the person's message is stored: nothing was made up in place of an answer.
    let messages = s
        .session
        .get(&format!("/api/conversations/{id}/messages"))
        .json();
    let roles: Vec<&str> = messages
        .as_array()
        .ok_or("m")?
        .iter()
        .filter_map(|m| m["role"].as_str())
        .collect();
    assert_eq!(roles, ["user"]);
    // The conversation is free again.
    assert_ne!(turn(&s, &id, "retry").status, 409);
    Ok(())
}

#[test]
fn a_provider_that_breaks_mid_stream_ends_with_an_error_after_the_text_so_far() -> TestResult {
    // One line, then garbage: the text already sent stays, and the turn ends with an error.
    let s = setup(vec![Reply::Garbage], false, false)?;
    let id = new_conversation(&s)?;
    let events = sse_events(&turn(&s, &id, "hello").text());
    assert_eq!(events.last().map(|(e, _)| e.as_str()), Some("error"));
    assert!(!kinds(&events).contains(&"done"));
    Ok(())
}

#[test]
fn only_one_turn_runs_per_conversation() -> TestResult {
    let s = setup(
        vec![
            Reply::After(900, Box::new(text_reply(&["first\n"]))),
            text_reply(&["second\n"]),
        ],
        false,
        false,
    )?;
    let id = new_conversation(&s)?;
    let other = new_conversation(&s)?;
    let (addr, cookie) = (s.session.addr, s.session.cookie.clone());
    let path = format!("/api/conversations/{id}/turn");
    let first = std::thread::spawn({
        let (path, cookie) = (path.clone(), cookie);
        move || {
            Req::post(&path)
                .same_origin(addr)
                .cookie(&cookie)
                .json(&json!({"message": "slow"}))
                .send(addr)
        }
    });
    std::thread::sleep(Duration::from_millis(300));
    let busy = turn(&s, &id, "impatient");
    assert_eq!((busy.status, busy.code().as_str()), (409, "busy"));
    assert_security_headers(&busy, true);
    // Another conversation is not blocked by it.
    let parallel = turn(&s, &other, "meanwhile");
    assert_eq!(parallel.status, 200);
    let done = first.join().map_err(|_| "thread")?;
    assert_eq!(kinds(&sse_events(&done.text())).last(), Some(&"done"));
    Ok(())
}

#[test]
fn a_browser_that_goes_away_does_not_leave_the_conversation_claimed() -> TestResult {
    let s = setup(
        vec![
            Reply::After(500, Box::new(text_reply(&["abandoned\n"]))),
            text_reply(&["next\n"]),
        ],
        false,
        false,
    )?;
    let id = new_conversation(&s)?;
    {
        let request = Req::post(&format!("/api/conversations/{id}/turn"))
            .same_origin(s.session.addr)
            .cookie(&s.session.cookie)
            .json(&json!({"message": "and gone"}))
            .bytes(s.session.addr);
        let mut stream = TcpStream::connect(s.session.addr)?;
        stream.write_all(&request)?;
        // Wait for the head of the stream, so the turn is surely running, then drop the socket:
        // the browser closed the tab.
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let mut head = Vec::new();
        let mut chunk = [0_u8; 512];
        while !head.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream.read(&mut chunk)?;
            assert!(n > 0, "the stream never started");
            head.extend_from_slice(&chunk[..n]);
        }
        assert!(String::from_utf8_lossy(&head).starts_with("HTTP/1.1 200"));
    }
    std::thread::sleep(Duration::from_millis(50));
    // While the abandoned turn still runs, the conversation is busy...
    let during = turn(&s, &id, "still there?");
    assert_eq!(during.status, 409);
    // ...and once it ends, the conversation is free again, in this process and in the database.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut freed = false;
    while Instant::now() < deadline {
        let r = turn(&s, &id, "now?");
        if r.status == 200 {
            freed = true;
            break;
        }
        assert_eq!(r.status, 409, "{}", r.text());
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        freed,
        "the conversation stayed claimed after the client left"
    );
    let conn = s.world.conn()?;
    let conversation = db::ConversationId::parse(&id)?;
    assert!(matches!(
        db::claim_turn(&conn, &s.world.org, &conversation, 600)?,
        db::ClaimOutcome::Claimed(_)
    ));
    Ok(())
}

#[test]
fn a_turn_is_refused_with_a_real_status_before_any_stream_starts() -> TestResult {
    let s = setup(vec![text_reply(&["x"])], false, false)?;
    let id = new_conversation(&s)?;
    let path = format!("/api/conversations/{id}/turn");
    for (body, status, code) in [
        (json!({"message": ""}), 400, "message_empty"),
        (json!({"message": "   \n"}), 400, "message_empty"),
        (
            json!({"message": "x".repeat(16 * 1024 + 1)}),
            400,
            "message_too_large",
        ),
        (json!({"message": "a\u{0}b"}), 400, "message_invalid"),
        (json!({}), 400, "invalid_input"),
        (json!({"message": 5}), 400, "invalid_input"),
        (
            json!({"message": "hi", "conversation": "x"}),
            400,
            "invalid_input",
        ),
        (
            json!({"message": "hi", "org_id": "x"}),
            400,
            "invalid_input",
        ),
    ] {
        let r = s.session.post(&path, &body);
        assert_eq!(
            (r.status, r.code().as_str()),
            (status, code),
            "{}",
            body.to_string().chars().take(60).collect::<String>()
        );
    }
    let none = s.session.req(Req::post(&path).same_origin(s.session.addr));
    assert_eq!(none.status, 400);
    assert!(
        s.fake.requests().is_empty(),
        "no refused turn may reach the provider"
    );
    // Archived, and unknown.
    db::archive_conversation(
        &s.world.conn()?,
        &s.world.org,
        &db::ConversationId::parse(&id)?,
    )?;
    let r = turn(&s, &id, "hello");
    assert_eq!(
        (r.status, r.code().as_str()),
        (409, "conversation_archived")
    );
    for other in ["00000000-0000-4000-8000-000000000000", "nope"] {
        assert_eq!(turn(&s, other, "hello").status, 404, "{other}");
    }
    Ok(())
}

#[test]
fn a_turn_needs_a_default_provider_and_the_master_key_for_a_sealed_key() -> TestResult {
    // No provider at all.
    let world = World::new()?;
    let server = TestServer::start(&world)?;
    let session = server.sign_in()?;
    let id = session.post("/api/conversations", &json!({})).json()["id"]
        .as_str()
        .ok_or("id")?
        .to_owned();
    let r = session.post(
        &format!("/api/conversations/{id}/turn"),
        &json!({"message": "hi"}),
    );
    assert_eq!((r.status, r.code().as_str()), (409, "no_provider"));
    // A sealed key and a server without the master key.
    let s = setup(vec![text_reply(&["x"])], true, false)?;
    let id = new_conversation(&s)?;
    let r = turn(&s, &id, "hi");
    assert_eq!((r.status, r.code().as_str()), (409, "master_key_missing"));
    assert!(s.fake.requests().is_empty());
    Ok(())
}

#[test]
fn another_persons_conversation_cannot_be_continued() -> TestResult {
    let s = setup(vec![text_reply(&["x"])], false, false)?;
    let conn = s.world.conn()?;
    let theirs = db::create_conversation(&conn, &s.world.org, &s.world.second, "theirs", None)?;
    let rival = db::create_conversation(&conn, &s.world.rival, &s.world.rival_user, "rival", None)?;
    for id in [theirs.as_str(), rival.as_str()] {
        let r = turn(&s, id, "let me in");
        assert_eq!((r.status, r.code().as_str()), (404, "not_found"), "{id}");
    }
    assert!(s.fake.requests().is_empty());
    let _ = &s.server;
    Ok(())
}
