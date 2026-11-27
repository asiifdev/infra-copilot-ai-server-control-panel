//! The stored commands end to end: `init`, `node add`, `provider`, `ask`, `memory`, and the
//! `--save` variants of `collect` and `compare`, against a real SQLite file and a fake
//! OpenAI-compatible server on 127.0.0.1 (plain HTTP with Server-Sent Events).

use cli::Env;
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

type TestResult = Result<(), Box<dyn std::error::Error>>;

// ---- keys, built at run time so no token-looking literal sits in the repository ---------------

fn api_key() -> String {
    format!("{}{}", "sk-", "q7Zr3Lm9".repeat(4))
}

fn master_key(fill: &str) -> String {
    format!("main:{}", fill.repeat(32))
}

fn good_master() -> String {
    master_key("ab")
}

// ---- a scratch directory ----------------------------------------------------------------------

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> std::io::Result<Self> {
        static N: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "nodivara-cli-{}-{}-{tag}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&path)?;
        Ok(Self(path))
    }

    fn db(&self) -> String {
        self.0.join("test.db").to_string_lossy().into_owned()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ---- driving the command line -----------------------------------------------------------------

struct Out {
    code: u8,
    stdout: String,
    stderr: String,
}

fn run(args: &[&str], env: &[(&str, &str)], stdin: &str) -> Out {
    let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = cli::run_with(
        &args,
        None,
        &Env::fixed(env),
        &mut stdin.as_bytes(),
        &mut out,
        &mut err,
    );
    Out {
        code,
        stdout: String::from_utf8_lossy(&out).into_owned(),
        stderr: String::from_utf8_lossy(&err).into_owned(),
    }
}

fn manifest(rel: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(rel)
        .to_string_lossy()
        .into_owned()
}

// ---- a fake OpenAI-compatible server ----------------------------------------------------------

#[derive(Clone)]
enum Reply {
    /// 200 with these Server-Sent Events (each already a JSON string).
    Sse(Vec<Value>),
    /// An error status with a JSON body and optional extra headers.
    Status(u16, &'static str, Vec<(&'static str, &'static str)>),
}

struct Server {
    port: u16,
    seen: Arc<Mutex<Vec<String>>>,
}

impl Server {
    fn start(script: Vec<Reply>) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let seen: Arc<Mutex<Vec<String>>> = Arc::default();
        let log = Arc::clone(&seen);
        std::thread::spawn(move || {
            let mut replies = script.into_iter();
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let Some(request) = read_request(&mut stream) else {
                    continue;
                };
                if let Ok(mut l) = log.lock() {
                    l.push(request);
                }
                // Past the end of the script the last behaviour is an empty server error.
                let reply = replies
                    .next()
                    .unwrap_or(Reply::Status(500, "{}", Vec::new()));
                respond(&mut stream, &reply);
            }
        });
        Ok(Self { port, seen })
    }

    fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}/v1", self.port)
    }

    fn requests(&self) -> Vec<String> {
        self.seen.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

fn read_request(stream: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..pos]).to_ascii_lowercase();
            let wanted = head
                .lines()
                .find_map(|l| {
                    l.strip_prefix("content-length:")?
                        .trim()
                        .parse::<usize>()
                        .ok()
                })
                .unwrap_or(0);
            while buf.len() < pos + 4 + wanted {
                let n = stream.read(&mut chunk).ok()?;
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            return Some(String::from_utf8_lossy(&buf).into_owned());
        }
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

fn respond(stream: &mut TcpStream, reply: &Reply) {
    match reply {
        Reply::Sse(events) => {
            let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                        Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(head.as_bytes());
            let mut pieces: Vec<String> = events.iter().map(|e| format!("data: {e}\n\n")).collect();
            pieces.push("data: [DONE]\n\n".to_owned());
            for piece in pieces {
                let framed = format!("{:x}\r\n{piece}\r\n", piece.len());
                if stream.write_all(framed.as_bytes()).is_err() {
                    return;
                }
            }
            let _ = stream.write_all(b"0\r\n\r\n");
        }
        Reply::Status(status, body, headers) => {
            let mut head = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
                body.len()
            );
            for (k, v) in headers {
                head.push_str(&format!("{k}: {v}\r\n"));
            }
            head.push_str("\r\n");
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(body.as_bytes());
        }
    }
    let _ = stream.flush();
}

fn text_reply(pieces: &[&str]) -> Reply {
    let mut events: Vec<Value> = pieces
        .iter()
        .map(|p| json!({"choices":[{"delta":{"content":p}}]}))
        .collect();
    events.push(json!({"choices":[{"delta":{},"finish_reason":"stop"}]}));
    Reply::Sse(events)
}

fn tool_reply(id: &str, name: &str, args: &str) -> Reply {
    Reply::Sse(vec![
        json!({"choices":[{"delta":{"role":"assistant","tool_calls":[
            {"index":0,"id":id,"type":"function","function":{"name":name,"arguments":args}}]}}]}),
        json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
    ])
}

// ---- a ready-made installation ----------------------------------------------------------------

struct Setup {
    dir: Scratch,
    server: Server,
}

const PARAMS: &str = r#"{"allow_private_network":true,"max_retries":0}"#;

impl Setup {
    /// `init`, one node `old`, and a default provider pointing at the fake server (key sealed
    /// under the good master key when `with_key`).
    fn new(script: Vec<Reply>, with_key: bool) -> Result<Self, Box<dyn std::error::Error>> {
        let dir = Scratch::new("setup")?;
        let server = Server::start(script)?;
        let db = dir.db();
        let master = good_master();
        let r = run(&["init", "--db", &db], &[], "");
        assert_eq!(r.code, 0, "{}", r.stderr);
        let r = run(
            &["node", "add", "--db", &db, "--name", "old", "--kind", "vps"],
            &[],
            "",
        );
        assert_eq!(r.code, 0, "{}", r.stderr);
        let key = api_key();
        let url = server.base_url();
        let args = [
            "provider",
            "add",
            "--db",
            &db,
            "--name",
            "main",
            "--base-url",
            &url,
            "--model",
            "test-model",
            "--params",
            PARAMS,
            "--default",
            "--key-stdin",
        ];
        let env = [("NODIVARA_MASTER_KEY", master.as_str())];
        let r = if with_key {
            run(&args, &env, &format!("{key}\n"))
        } else {
            run(&args, &env, "")
        };
        assert_eq!(r.code, 0, "{}", r.stderr);
        Ok(Self { dir, server })
    }

    fn db(&self) -> String {
        self.dir.db()
    }

    fn ask(&self, question: &str) -> Out {
        let master = good_master();
        run(
            &["ask", "--db", &self.db(), "--new", question],
            &[("NODIVARA_MASTER_KEY", master.as_str())],
            "",
        )
    }
}

/// Everything in the database files (main, WAL and shared memory): the key must be in none.
fn raw_db_bytes(db: &str) -> Vec<u8> {
    let mut all = Vec::new();
    for suffix in ["", "-wal", "-shm"] {
        if let Ok(bytes) = std::fs::read(format!("{db}{suffix}")) {
            all.extend(bytes);
        }
    }
    all
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|w| w == needle.as_bytes())
}

// ---- tests ------------------------------------------------------------------------------------

#[test]
fn init_node_provider_ask_with_a_tool_call_and_no_key_leaks() -> TestResult {
    let setup = Setup::new(
        vec![
            tool_reply("call_1", "list_nodes", "{}"),
            text_reply(&["The server old ", "is pending.\n", "Nothing else."]),
        ],
        true,
    )?;
    let db = setup.db();
    let key = api_key();
    let master = good_master();

    let listed = run(&["provider", "list", "--db", &db], &[], "");
    assert_eq!(listed.code, 0);
    assert!(listed.stdout.contains("main"));
    assert!(listed.stdout.contains("stored"));
    assert!(!listed.stdout.contains(&key));

    let r = setup.ask("Which servers do I have?");
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    assert_eq!(r.stdout, "The server old is pending.\nNothing else.\n");
    assert!(
        r.stderr.contains("tool list_nodes (tier 0): ok"),
        "{}",
        r.stderr
    );
    assert!(r.stderr.contains("Conversation "));

    // What was stored: the question, the tool round and the answer.
    let conn = db::open(Path::new(&db))?;
    let org = db::list_orgs(&conn, 5)?.remove(0).id;
    let user = db::list_members(&conn, &org)?.remove(0).user_id;
    let conversation = db::list_conversations(&conn, &org, &user, false, 5)?
        .remove(0)
        .id;
    let messages = db::list_messages(&conn, &org, &conversation, 50)?;
    let roles: Vec<&str> = messages
        .iter()
        .map(|m| match m.role {
            db::MessageRole::User => "user",
            db::MessageRole::Assistant => "assistant",
            db::MessageRole::Tool => "tool",
            db::MessageRole::System => "system",
        })
        .collect();
    assert_eq!(roles, ["user", "assistant", "tool", "assistant"]);
    assert_eq!(messages[0].content, "Which servers do I have?");
    assert!(messages[2].content.contains("old"));
    assert_eq!(
        messages[3].content,
        "The server old is pending.\nNothing else."
    );
    let calls = db::list_tool_calls(&conn, &org, &conversation)?;
    assert_eq!(calls.len(), 1);
    assert_eq!(
        (calls[0].tool.as_str(), calls[0].status.as_str()),
        ("list_nodes", "ok")
    );

    // The key reached the provider (so it was opened correctly) and is nowhere else.
    let requests = setup.server.requests();
    assert_eq!(requests.len(), 2);
    let lower = requests[0].to_ascii_lowercase();
    assert!(lower.contains(&format!(
        "authorization: bearer {}",
        key.to_ascii_lowercase()
    )));
    for text in [&r.stdout, &r.stderr, &listed.stdout, &listed.stderr] {
        assert!(!text.contains(&key) && !text.contains(&master));
    }
    let bytes = raw_db_bytes(&db);
    assert!(
        !contains(&bytes, &key),
        "API key found in the database files"
    );
    assert!(
        !contains(&bytes, &master),
        "master key found in the database files"
    );
    assert!(!contains(&bytes, &master["main:".len()..]));
    for request in &requests {
        assert!(!request.contains(&master));
    }
    // The audit trail has the call, without the key.
    assert!(matches!(
        db::verify_audit_chain(&conn, &org)?,
        db::ChainStatus::Intact { entries: 1 }
    ));
    Ok(())
}

#[test]
fn the_key_is_never_taken_from_the_command_line() -> TestResult {
    let dir = Scratch::new("argv")?;
    let db = dir.db();
    assert_eq!(run(&["init", "--db", &db], &[], "").code, 0);
    let key = api_key();
    let master = good_master();
    for flag in ["--key", "--api-key", "--secret"] {
        let r = run(
            &[
                "provider",
                "add",
                "--db",
                &db,
                "--name",
                "p",
                "--base-url",
                "https://example.test/v1",
                "--model",
                "m",
                flag,
                &key,
            ],
            &[("NODIVARA_MASTER_KEY", master.as_str())],
            "",
        );
        assert_eq!(r.code, 2, "{flag}");
        assert!(r.stderr.contains("Usage"));
        assert!(!r.stderr.contains(&key) && !r.stdout.contains(&key));
    }
    let listed = run(&["provider", "list", "--db", &db], &[], "");
    assert!(listed.stdout.contains("No providers"));
    Ok(())
}

#[test]
fn a_key_from_the_environment_works_and_odd_keys_are_refused() -> TestResult {
    let dir = Scratch::new("envkey")?;
    let db = dir.db();
    assert_eq!(run(&["init", "--db", &db], &[], "").code, 0);
    let master = good_master();
    let key = api_key();
    let base = [
        "provider",
        "add",
        "--db",
        &db,
        "--name",
        "p",
        "--base-url",
        "https://example.test/v1",
        "--model",
        "m",
    ];
    let env = [
        ("NODIVARA_MASTER_KEY", master.as_str()),
        ("NODIVARA_PROVIDER_KEY", key.as_str()),
    ];
    let r = run(&base, &env, "");
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(!contains(&raw_db_bytes(&db), &key));

    let bad = format!("{key}\u{1b}[2J");
    let other = [
        "provider",
        "add",
        "--db",
        &db,
        "--name",
        "q",
        "--base-url",
        "https://example.test/v1",
        "--model",
        "m",
        "--key-stdin",
    ];
    let r = run(&other, &[("NODIVARA_MASTER_KEY", master.as_str())], &bad);
    assert_eq!(r.code, 2);
    assert!(!r.stderr.contains(&key));
    let r = run(
        &other,
        &[("NODIVARA_MASTER_KEY", master.as_str())],
        &"k".repeat(5000),
    );
    assert_eq!(r.code, 2);
    // Nothing was left behind by the refused attempts.
    let listed = run(&["provider", "list", "--db", &db], &[], "");
    assert_eq!(listed.stdout.matches('\n').count(), 2, "{}", listed.stdout);
    Ok(())
}

#[test]
fn a_missing_or_malformed_master_key_is_refused_with_advice() -> TestResult {
    let dir = Scratch::new("master")?;
    let db = dir.db();
    assert_eq!(run(&["init", "--db", &db], &[], "").code, 0);
    let key = api_key();
    let args = [
        "provider",
        "add",
        "--db",
        &db,
        "--name",
        "p",
        "--base-url",
        "https://example.test/v1",
        "--model",
        "m",
        "--key-stdin",
    ];
    let stdin = format!("{key}\n");

    let r = run(&args, &[], &stdin);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("master key is not set"), "{}", r.stderr);
    assert!(r.stderr.contains("openssl rand -hex 32"));

    for bad in ["short", "main:zz", &master_key("ab")[..40], "nocolon"] {
        let r = run(&args, &[("NODIVARA_MASTER_KEY", bad)], &stdin);
        assert_eq!(r.code, 2, "{bad}");
        assert!(r.stderr.contains("not valid"), "{}", r.stderr);
        assert!(
            !r.stderr.contains(bad) || bad.len() < 8,
            "key echoed: {}",
            r.stderr
        );
    }
    let listed = run(&["provider", "list", "--db", &db], &[], "");
    assert!(listed.stdout.contains("No providers"));
    Ok(())
}

#[test]
fn ask_without_a_provider_says_how_to_add_one() -> TestResult {
    let dir = Scratch::new("noprov")?;
    let db = dir.db();
    assert_eq!(run(&["init", "--db", &db], &[], "").code, 0);
    let r = run(&["ask", "--db", &db, "hello"], &[], "");
    assert_eq!(r.code, 3);
    assert!(r.stderr.contains("infra provider add"), "{}", r.stderr);
    assert!(r.stdout.is_empty());
    Ok(())
}

#[test]
fn ask_with_the_wrong_or_missing_master_key_never_reaches_the_provider() -> TestResult {
    let setup = Setup::new(vec![text_reply(&["never"])], true)?;
    let db = setup.db();
    // Same id, other key material: the ciphertext does not open.
    let wrong = master_key("cd");
    let r = run(
        &["ask", "--db", &db, "--new", "hi"],
        &[("NODIVARA_MASTER_KEY", wrong.as_str())],
        "",
    );
    assert_eq!(r.code, 3);
    assert!(r.stderr.contains("does not match"), "{}", r.stderr);
    // Another key id: also a mismatch.
    let other_id = format!("other:{}", "ab".repeat(32));
    let r = run(
        &["ask", "--db", &db, "--new", "hi"],
        &[("NODIVARA_MASTER_KEY", other_id.as_str())],
        "",
    );
    assert_eq!(r.code, 3);
    assert!(r.stderr.contains("does not match"));
    // No key at all.
    let r = run(&["ask", "--db", &db, "--new", "hi"], &[], "");
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("master key is not set"));
    // Malformed.
    let r = run(
        &["ask", "--db", &db, "--new", "hi"],
        &[("NODIVARA_MASTER_KEY", "garbage")],
        "",
    );
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("not valid"));
    assert!(
        setup.server.requests().is_empty(),
        "the provider was contacted"
    );
    Ok(())
}

#[test]
fn provider_failures_become_localized_advice_and_nonzero_exits() -> TestResult {
    let cases: Vec<(Reply, &str)> = vec![
        (
            Reply::Status(401, r#"{"error":{"message":"bad key"}}"#, vec![]),
            "rejected the API key",
        ),
        (Reply::Status(402, "{}", vec![]), "out of credit or quota"),
        (
            Reply::Status(
                429,
                r#"{"error":{"message":"slow"}}"#,
                vec![("Retry-After", "7")],
            ),
            "Wait 7 seconds",
        ),
        (Reply::Status(429, "{}", vec![]), "limiting requests"),
        (
            Reply::Status(500, "{}", vec![]),
            "internal error (HTTP 500)",
        ),
        (
            Reply::Status(404, "{}", vec![]),
            "refused the request (HTTP 404)",
        ),
        (
            Reply::Sse(vec![
                json!({"choices":[{"delta":{},"finish_reason":"stop"}]}),
            ]),
            "no answer",
        ),
    ];
    for (reply, expected) in cases {
        let setup = Setup::new(vec![reply], true)?;
        let r = setup.ask("hello");
        assert_eq!(r.code, 3, "{expected}: {}", r.stderr);
        assert!(
            r.stderr.contains(expected),
            "want {expected:?} in {:?}",
            r.stderr
        );
        assert!(r.stdout.is_empty(), "{}", r.stdout);
        assert!(!r.stderr.contains(&api_key()));
    }
    // Indonesian advice follows --lang.
    let setup = Setup::new(vec![Reply::Status(401, "{}", vec![])], true)?;
    let master = good_master();
    let r = run(
        &["ask", "--db", &setup.db(), "--new", "--lang", "id", "halo"],
        &[("NODIVARA_MASTER_KEY", master.as_str())],
        "",
    );
    assert_eq!(r.code, 3);
    assert!(r.stderr.contains("menolak kunci API"), "{}", r.stderr);
    Ok(())
}

#[test]
fn a_provider_without_a_stored_key_gets_a_401_advice_and_needs_no_master_key() -> TestResult {
    let setup = Setup::new(vec![Reply::Status(401, "{}", vec![])], false)?;
    // No master key in the environment: a keyless provider does not need one.
    let r = run(&["ask", "--db", &setup.db(), "--new", "hello"], &[], "");
    assert_eq!(r.code, 3, "{}", r.stderr);
    assert!(r.stderr.contains("rejected the API key"));
    let requests = setup.server.requests();
    assert_eq!(requests.len(), 1);
    assert!(!requests[0].to_ascii_lowercase().contains("authorization:"));
    Ok(())
}

#[test]
fn model_text_is_redacted_and_stripped_of_terminal_escapes_before_it_is_printed() -> TestResult {
    let leaked = format!("{}={}", ["pass", "word"].concat(), "plainvalue123");
    let line = format!("config {leaked} end\n");
    let setup = Setup::new(
        vec![text_reply(&[
            "fine\u{1b}[31m red\u{1b}[0m\n",
            &line,
            "\u{7}bell",
        ])],
        true,
    )?;
    let r = setup.ask("show me");
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(
        !r.stdout.contains('\u{1b}') && !r.stdout.contains('\u{7}'),
        "{:?}",
        r.stdout
    );
    assert!(!r.stdout.contains("plainvalue123"), "{}", r.stdout);
    assert!(r.stdout.contains("fine"));
    assert!(r.stdout.ends_with('\n'));
    Ok(())
}

#[test]
fn conversations_continue_with_an_id_and_foreign_ids_are_refused() -> TestResult {
    let setup = Setup::new(vec![text_reply(&["one"]), text_reply(&["two"])], true)?;
    let first = setup.ask("first question");
    assert_eq!(first.code, 0);
    let id = first
        .stderr
        .lines()
        .find_map(|l| l.strip_prefix("Conversation "))
        .and_then(|l| l.split_whitespace().next())
        .ok_or("no conversation id on stderr")?
        .to_owned();
    let master = good_master();
    let env = [("NODIVARA_MASTER_KEY", master.as_str())];
    let second = run(
        &[
            "ask",
            "--db",
            &setup.db(),
            "--conversation",
            &id,
            "second question",
        ],
        &env,
        "",
    );
    assert_eq!(second.code, 0, "{}", second.stderr);
    // The second request replays the first exchange.
    let requests = setup.server.requests();
    assert!(requests[1].contains("first question") && requests[1].contains("one"));
    // Bad ids and the --new/--conversation clash are refused.
    let bad = run(
        &["ask", "--db", &setup.db(), "--conversation", "nope", "x"],
        &env,
        "",
    );
    assert_eq!(bad.code, 2);
    let missing = run(
        &[
            "ask",
            "--db",
            &setup.db(),
            "--conversation",
            "00000000-0000-4000-8000-000000000000",
            "x",
        ],
        &env,
        "",
    );
    assert_eq!(missing.code, 2);
    let clash = run(
        &[
            "ask",
            "--db",
            &setup.db(),
            "--new",
            "--conversation",
            &id,
            "x",
        ],
        &env,
        "",
    );
    assert_eq!(clash.code, 2);
    assert!(clash.stderr.contains("Usage"));
    let empty = run(&["ask", "--db", &setup.db()], &env, "");
    assert_eq!(empty.code, 2);
    Ok(())
}

#[test]
fn memory_proposed_by_the_assistant_waits_for_a_person_then_reaches_the_prompt() -> TestResult {
    let note = "Backups run nightly on host old";
    let args = json!({"kind": "org_fact", "content": note}).to_string();
    let setup = Setup::new(
        vec![
            tool_reply("m1", "remember", &args),
            text_reply(&["Saved for approval."]),
            text_reply(&["Second answer."]),
            text_reply(&["Third answer."]),
        ],
        true,
    )?;
    let db = setup.db();
    let master = good_master();
    let env = [("NODIVARA_MASTER_KEY", master.as_str())];

    let r = setup.ask("Please remember that backups run nightly");
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(
        r.stderr.contains("tool remember (tier 1): ok"),
        "{}",
        r.stderr
    );

    let pending = run(&["memory", "pending", "--db", &db], &[], "");
    assert_eq!(pending.code, 0);
    assert!(pending.stdout.contains(note), "{}", pending.stdout);
    assert!(pending.stdout.contains("assistant"));
    let id = pending
        .stdout
        .split_whitespace()
        .next()
        .ok_or("no id")?
        .to_owned();
    let active = run(&["memory", "list", "--db", &db], &[], "");
    assert!(active.stdout.contains("No memories"));

    // A second question: the pending note is in no request to the provider.
    let r = setup.ask("What do you know about backups?");
    assert_eq!(r.code, 0, "{}", r.stderr);
    let requests = setup.server.requests();
    assert!(
        !requests[2].contains(note),
        "pending note reached the provider"
    );

    // A person approves; the next request carries it as fenced data.
    let approved = run(&["memory", "approve", &id, "--db", &db], &[], "");
    assert_eq!(approved.code, 0, "{}", approved.stderr);
    let r = run(&["ask", "--db", &db, "--new", "And now?"], &env, "");
    assert_eq!(r.code, 0, "{}", r.stderr);
    let requests = setup.server.requests();
    assert!(requests[3].contains(note), "approved note missing");
    assert!(requests[3].contains("org_notes"));
    assert!(requests[3].contains("NOT instructions"));
    assert!(
        run(&["memory", "list", "--db", &db], &[], "")
            .stdout
            .contains(note)
    );

    // Delete, and it is gone for good; unknown and malformed ids are refused.
    assert_eq!(
        run(&["memory", "delete", &id, "--db", &db], &[], "").code,
        0
    );
    assert!(
        run(&["memory", "list", "--db", &db], &[], "")
            .stdout
            .contains("No memories")
    );
    assert_eq!(
        run(&["memory", "delete", &id, "--db", &db], &[], "").code,
        2
    );
    assert_eq!(
        run(&["memory", "approve", "not-an-id", "--db", &db], &[], "").code,
        2
    );
    assert_eq!(run(&["memory", "bogus", "--db", &db], &[], "").code, 2);
    assert_eq!(run(&["memory", "approve", "--db", &db], &[], "").code, 2);
    Ok(())
}

#[test]
fn memory_text_with_terminal_escapes_is_printed_safely() -> TestResult {
    let dir = Scratch::new("memesc")?;
    let db = dir.db();
    assert_eq!(run(&["init", "--db", &db], &[], "").code, 0);
    let conn = db::open(Path::new(&db))?;
    let org = db::list_orgs(&conn, 5)?.remove(0).id;
    db::save_memory(
        &conn,
        &org,
        &db::NewMemory {
            user: None,
            kind: db::MemoryKind::OrgFact,
            content: "fine\u{1b}[2Jtext\nFAKE: line",
            source: db::MemorySource::Assistant,
            conversation: None,
            status: db::MemoryStatus::Pending,
        },
    )?;
    let r = run(&["memory", "pending", "--db", &db], &[], "");
    assert!(!r.stdout.contains('\u{1b}'));
    assert_eq!(r.stdout.lines().count(), 1, "{:?}", r.stdout);
    Ok(())
}

#[test]
fn init_refuses_a_second_time_and_the_database_is_owner_only() -> TestResult {
    let dir = Scratch::new("init")?;
    let db = dir.db();
    let r = run(
        &[
            "init",
            "--db",
            &db,
            "--slug",
            "acme",
            "--name",
            "Acme",
            "--email",
            "Boss@Example.test",
        ],
        &[],
        "",
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout.contains("acme") && r.stdout.contains("boss@example.test"));
    let again = run(&["init", "--db", &db], &[], "");
    assert_eq!(again.code, 2);
    assert!(again.stderr.contains("already has an organization"));
    let conn = db::open(Path::new(&db))?;
    assert_eq!(db::list_orgs(&conn, 10)?.len(), 1);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&db)?.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
    // A bad slug is refused and leaves no half-created organization.
    let dir2 = Scratch::new("init2")?;
    let r = run(
        &["init", "--db", &dir2.db(), "--slug", "Not Valid!"],
        &[],
        "",
    );
    assert_eq!(r.code, 2);
    let conn = db::open(Path::new(&dir2.db()))?;
    assert!(db::list_orgs(&conn, 10)?.is_empty());
    Ok(())
}

#[cfg(unix)]
#[test]
fn an_existing_world_readable_database_is_tightened() -> TestResult {
    use std::os::unix::fs::PermissionsExt;
    let dir = Scratch::new("perm")?;
    let db = dir.db();
    std::fs::write(&db, b"")?;
    std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o644))?;
    assert_eq!(run(&["init", "--db", &db], &[], "").code, 0);
    assert_eq!(std::fs::metadata(&db)?.permissions().mode() & 0o777, 0o600);
    Ok(())
}

#[test]
fn the_database_location_follows_flag_then_environment_then_the_user_data_directory() -> TestResult
{
    let dir = Scratch::new("where")?;
    // NODIVARA_DB wins over the default directory.
    let via_env = dir.0.join("env.db").to_string_lossy().into_owned();
    let r = run(&["init"], &[("NODIVARA_DB", via_env.as_str())], "");
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(Path::new(&via_env).is_file());
    // --db wins over NODIVARA_DB.
    let via_flag = dir.0.join("flag.db").to_string_lossy().into_owned();
    let r = run(
        &["init", "--db", &via_flag],
        &[("NODIVARA_DB", via_env.as_str())],
        "",
    );
    assert_eq!(r.code, 0);
    assert!(Path::new(&via_flag).is_file());
    // Otherwise a per-user data directory under HOME is created, owner-only.
    let home = dir.0.join("home").to_string_lossy().into_owned();
    let r = run(
        &["init"],
        &[
            ("HOME", home.as_str()),
            ("XDG_DATA_HOME", ""),
            ("LOCALAPPDATA", ""),
            ("USERPROFILE", ""),
        ],
        "",
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    let default = if cfg!(target_os = "macos") {
        dir.0
            .join("home/Library/Application Support/Nodivara/nodivara.db")
    } else if cfg!(target_os = "windows") {
        dir.0.join("home/.nodivara/nodivara.db")
    } else {
        dir.0.join("home/.local/share/nodivara/nodivara.db")
    };
    assert!(default.is_file(), "{default:?}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let parent = default.parent().ok_or("no parent")?;
        assert_eq!(
            std::fs::metadata(parent)?.permissions().mode() & 0o777,
            0o700
        );
    }
    // Nothing to go on: an error, not a guess.
    let r = run(&["init"], &[], "");
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("--db PATH"));
    // A missing file is not silently created by a non-init command.
    let missing = dir.0.join("none.db").to_string_lossy().into_owned();
    let r = run(&["provider", "list", "--db", &missing], &[], "");
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("infra init"));
    assert!(!Path::new(&missing).exists());
    Ok(())
}

#[test]
fn several_organizations_require_org() -> TestResult {
    let dir = Scratch::new("orgs")?;
    let db = dir.db();
    assert_eq!(
        run(&["init", "--db", &db, "--slug", "alpha"], &[], "").code,
        0
    );
    let conn = db::open(Path::new(&db))?;
    let second = db::create_org(&conn, "Beta", "beta", "free")?;
    let user = db::create_user(&conn, "b@example.test", "B", None)?;
    db::add_member(&conn, &second.id, &user.id, db::Role::Owner)?;

    let r = run(
        &["node", "add", "--db", &db, "--name", "n", "--kind", "local"],
        &[],
        "",
    );
    assert_eq!(r.code, 2);
    assert!(
        r.stderr.contains("--org SLUG") && r.stderr.contains("alpha") && r.stderr.contains("beta")
    );
    let r = run(
        &[
            "node", "add", "--db", &db, "--org", "beta", "--name", "n", "--kind", "local",
        ],
        &[],
        "",
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    // The node exists in beta only.
    let alpha = db::list_orgs(&conn, 5)?.remove(0);
    assert!(db::list_nodes(&conn, &alpha.id)?.is_empty());
    assert_eq!(db::list_nodes(&conn, &second.id)?.len(), 1);
    let r = run(
        &[
            "node", "add", "--db", &db, "--org", "nope", "--name", "n", "--kind", "local",
        ],
        &[],
        "",
    );
    assert_eq!(r.code, 2);
    // A duplicate name and a bad kind are refused.
    let r = run(
        &[
            "node", "add", "--db", &db, "--org", "beta", "--name", "n", "--kind", "local",
        ],
        &[],
        "",
    );
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("already exists"));
    let r = run(
        &[
            "node", "add", "--db", &db, "--org", "beta", "--name", "m", "--kind", "cloud",
        ],
        &[],
        "",
    );
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("Usage"));
    Ok(())
}

#[test]
fn collect_save_stores_the_inventory_for_a_node() -> TestResult {
    let dir = Scratch::new("collect")?;
    let db = dir.db();
    assert_eq!(run(&["init", "--db", &db], &[], "").code, 0);
    assert_eq!(
        run(
            &["node", "add", "--db", &db, "--name", "old", "--kind", "vps"],
            &[],
            ""
        )
        .code,
        0
    );
    let root = manifest("../collectors/tests/fixtures/full");
    let base = [
        "collect", "--root", &root, "--os", "linux", "--arch", "x86_64",
    ];
    let plain = run(&base, &[], "");
    assert_eq!(plain.code, 0);

    let mut args = base.to_vec();
    args.extend(["--save", "--node", "old", "--db", &db]);
    let saved = run(&args, &[], "");
    assert_eq!(saved.code, 0, "{}", saved.stderr);
    // The JSON on stdout is the same as without --save (apart from the timestamp).
    let strip = |text: &str| -> Result<Value, serde_json::Error> {
        let mut value: Value = serde_json::from_str(text)?;
        value["taken_at"] = Value::Null;
        Ok(value)
    };
    assert_eq!(strip(&saved.stdout)?, strip(&plain.stdout)?);
    assert!(saved.stderr.contains("Saved inventory"), "{}", saved.stderr);

    let conn = db::open(Path::new(&db))?;
    let org = db::list_orgs(&conn, 5)?.remove(0).id;
    let node = db::list_nodes(&conn, &org)?.remove(0);
    let row = db::latest_inventory(&conn, &org, &node.id)?;
    let counts = db::count_inventory_items(&conn, &org, &row.id)?;
    assert!(counts.iter().map(|(_, n)| n).sum::<u64>() > 5);
    // The stored items are searchable by the assistant's search index.
    let hits = db::search(&conn, &org, "5432", 5)?;
    assert!(!hits.is_empty());

    // Unknown node and half-given options.
    let mut bad = base.to_vec();
    bad.extend(["--save", "--node", "nope", "--db", &db]);
    let r = run(&bad, &[], "");
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("no node named nope"));
    for extra in [vec!["--save"], vec!["--node", "old"]] {
        let mut args = base.to_vec();
        args.extend(extra);
        let r = run(&args, &[], "");
        assert_eq!(r.code, 2);
        assert!(r.stderr.contains("Usage"));
    }
    Ok(())
}

#[test]
fn compare_save_compares_the_stored_inventories_and_stores_the_result() -> TestResult {
    let dir = Scratch::new("compare")?;
    let db = dir.db();
    assert_eq!(run(&["init", "--db", &db], &[], "").code, 0);
    for name in ["old", "new", "empty"] {
        assert_eq!(
            run(
                &["node", "add", "--db", &db, "--name", name, "--kind", "vps"],
                &[],
                ""
            )
            .code,
            0
        );
    }
    // Store the parity fixtures as the nodes' inventories.
    let mut conn = db::open(Path::new(&db))?;
    let org = db::list_orgs(&conn, 5)?.remove(0).id;
    for (node, file) in [("old", "source.json"), ("new", "target-gaps.json")] {
        let text = std::fs::read_to_string(manifest(&format!("../parity/tests/fixtures/{file}")))?;
        let inventory = inventory::Inventory::from_json(&text)?;
        agent::persist_inventory(&mut conn, &org, node, &inventory)?;
    }
    let from_files = run(
        &[
            "compare",
            &manifest("../parity/tests/fixtures/source.json"),
            &manifest("../parity/tests/fixtures/target-gaps.json"),
        ],
        &[],
        "",
    );
    assert_eq!(from_files.code, 1);

    let saved = run(
        &[
            "compare",
            "--save",
            "--source-node",
            "old",
            "--target-node",
            "new",
            "--db",
            &db,
        ],
        &[],
        "",
    );
    // Same report and same exit code as the file-based comparison.
    assert_eq!(saved.code, 1, "{}", saved.stderr);
    assert_eq!(saved.stdout, from_files.stdout);
    assert!(saved.stderr.contains("Saved comparison"));

    let comparison = db::latest_comparison(&conn, &org)?;
    let findings = db::list_findings(&conn, &org, &comparison.id, None, 100)?;
    assert!(!findings.is_empty());
    assert!(findings.iter().any(|f| f.rule_id == "PAR-001"));
    // The assistant can now search the stored findings.
    assert!(!db::search(&conn, &org, "PAR-001", 5)?.is_empty());

    // A node without a stored inventory, and misuse of the options.
    let r = run(
        &[
            "compare",
            "--save",
            "--source-node",
            "old",
            "--target-node",
            "empty",
            "--db",
            &db,
        ],
        &[],
        "",
    );
    assert_eq!(r.code, 2);
    assert!(
        r.stderr.contains("Node empty has no stored inventory"),
        "{}",
        r.stderr
    );
    for args in [
        vec!["compare", "--save", "--source-node", "old"],
        vec![
            "compare",
            "--source-node",
            "old",
            "--target-node",
            "new",
            "a.json",
            "b.json",
        ],
        vec![
            "compare",
            "--save",
            "--source-node",
            "old",
            "--target-node",
            "new",
            "x.json",
        ],
    ] {
        let r = run(&args, &[], "");
        assert_eq!(r.code, 2, "{args:?}");
        assert!(r.stderr.contains("Usage"));
    }
    Ok(())
}

#[test]
fn every_new_command_prints_usage_on_misuse() {
    for args in [
        vec!["init", "extra"],
        vec!["node"],
        vec!["node", "remove"],
        vec!["provider"],
        vec!["provider", "bogus"],
        vec!["provider", "add", "--name", "x"],
        vec!["provider", "remove"],
        vec!["ask", "--bogus"],
        vec!["memory"],
        vec!["--db"],
    ] {
        let r = run(&args, &[], "");
        assert_eq!(r.code, 2, "{args:?}");
        assert!(r.stderr.contains("Usage"), "{args:?}: {}", r.stderr);
        assert!(r.stdout.is_empty());
    }
}

#[test]
fn provider_remove_deletes_the_sealed_key_with_the_provider() -> TestResult {
    let setup = Setup::new(vec![text_reply(&["hi"])], true)?;
    let db = setup.db();
    let r = setup.ask("hello");
    assert_eq!(r.code, 0);
    // A conversation refers to the default provider only through "the default", so removal works;
    // the sealed key goes with it.
    let removed = run(&["provider", "remove", "main", "--db", &db], &[], "");
    assert_eq!(removed.code, 0, "{}", removed.stderr);
    let conn = db::open(Path::new(&db))?;
    let secrets: i64 = conn.query_row("SELECT count(*) FROM secrets", [], |row| row.get(0))?;
    assert_eq!(secrets, 0);
    assert_eq!(
        run(&["provider", "remove", "main", "--db", &db], &[], "").code,
        2
    );
    Ok(())
}

#[test]
fn a_second_default_provider_is_refused_without_leaving_a_sealed_key() -> TestResult {
    let setup = Setup::new(vec![], true)?;
    let db = setup.db();
    let master = good_master();
    let key = api_key();
    let r = run(
        &[
            "provider",
            "add",
            "--db",
            &db,
            "--name",
            "second",
            "--base-url",
            "https://example.test/v1",
            "--model",
            "m",
            "--default",
            "--key-stdin",
        ],
        &[("NODIVARA_MASTER_KEY", master.as_str())],
        &key,
    );
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("default provider already exists"));
    // Same name twice: refused, and its key is cleaned up.
    let r = run(
        &[
            "provider",
            "add",
            "--db",
            &db,
            "--name",
            "main",
            "--base-url",
            "https://example.test/v1",
            "--model",
            "m",
            "--key-stdin",
        ],
        &[("NODIVARA_MASTER_KEY", master.as_str())],
        &key,
    );
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("already exists"));
    let conn = db::open(Path::new(&db))?;
    let secrets: i64 = conn.query_row("SELECT count(*) FROM secrets", [], |row| row.get(0))?;
    assert_eq!(secrets, 1);
    // Unusable settings are refused before anything is sealed.
    let r = run(
        &[
            "provider",
            "add",
            "--db",
            &db,
            "--name",
            "third",
            "--base-url",
            "http://plain.example.test/v1",
            "--model",
            "m",
            "--key-stdin",
        ],
        &[("NODIVARA_MASTER_KEY", master.as_str())],
        &key,
    );
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("not valid"));
    Ok(())
}
