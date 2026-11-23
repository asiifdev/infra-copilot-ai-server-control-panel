//! Shared harness of the server tests: a database with two organizations, a server on an
//! ephemeral loopback port (own thread, own runtime, stopped on drop), a raw HTTP/1.1 client (so
//! tests control every header, including a wrong `Host`) and a fake OpenAI-compatible provider.
//!
//! Secrets are built at run time so no token-looking literal sits in the repository.

#![allow(dead_code)]

use bootstrap::Scope;
use db::{Connection, NewNode, NodeAccess, NodeKind, OrgId, Role, UserId};
use serde_json::{Value, json};
use server::{Clock, Config, LoginHandle, ServeError, Server};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;
use vault::Vault;

pub type TestResult = Result<(), Box<dyn std::error::Error>>;

// ---- secrets -----------------------------------------------------------------------------------

pub fn api_key() -> String {
    format!("{}{}", "sk-", "q7Zr3Lm9".repeat(4))
}

pub fn master_key() -> String {
    format!("main:{}", "ab".repeat(32))
}

pub fn vault() -> Result<Arc<Vault>, Box<dyn std::error::Error>> {
    Ok(Arc::new(Vault::new(&master_key(), &[])?))
}

// ---- a scratch directory -----------------------------------------------------------------------

pub struct Scratch(pub PathBuf);

impl Scratch {
    pub fn new(tag: &str) -> std::io::Result<Self> {
        static N: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "nodivara-server-{}-{}-{tag}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&path)?;
        Ok(Self(path))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ---- the world: a database with two organizations ----------------------------------------------

pub struct World {
    pub dir: Scratch,
    pub db: PathBuf,
    pub org: OrgId,
    /// The owner: the person the server acts for.
    pub user: UserId,
    /// Another member of the same organization.
    pub second: UserId,
    /// A second organization.
    pub rival: OrgId,
    pub rival_user: UserId,
}

pub fn node(conn: &Connection, org: &OrgId, name: &str) -> Result<(), Box<dyn std::error::Error>> {
    db::create_node(
        conn,
        org,
        &NewNode {
            name,
            kind: NodeKind::Vps,
            access: NodeAccess::Agent,
            labels_json: "{}",
        },
    )?;
    Ok(())
}

impl World {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let dir = Scratch::new("world")?;
        let path = dir.0.join("test.db");
        let conn = db::open(&path)?;
        let org = db::create_org(&conn, "Acme", "acme", "free")?;
        let user = db::create_user(&conn, "owner@example.test", "Olive Owner", None)?;
        db::add_member(&conn, &org.id, &user.id, Role::Owner)?;
        let second = db::create_user(&conn, "second@example.test", "Sam Second", None)?;
        db::add_member(&conn, &org.id, &second.id, Role::Admin)?;
        let rival = db::create_org(&conn, "Rival", "rival", "free")?;
        let rival_user = db::create_user(&conn, "boss@rival.test", "Rita Rival", None)?;
        db::add_member(&conn, &rival.id, &rival_user.id, Role::Owner)?;
        Ok(Self {
            db: path,
            org: org.id,
            user: user.id,
            second: second.id,
            rival: rival.id,
            rival_user: rival_user.id,
            dir,
        })
    }

    pub fn conn(&self) -> Result<Connection, db::Error> {
        db::open(&self.db)
    }

    /// The database files as bytes (main, write-ahead log and shared memory).
    pub fn raw_bytes(&self) -> Vec<u8> {
        let mut all = Vec::new();
        for suffix in ["", "-wal", "-shm"] {
            if let Ok(bytes) = std::fs::read(format!("{}{suffix}", self.db.to_string_lossy())) {
                all.extend(bytes);
            }
        }
        all
    }
}

pub fn contains(haystack: &[u8], needle: &str) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|w| w == needle.as_bytes())
}

// ---- the server --------------------------------------------------------------------------------

pub struct Opts {
    pub vault: Option<Arc<Vault>>,
    pub tokens: usize,
    pub catalog: Option<i18n::Catalog>,
    pub provider_test_timeout: Option<Duration>,
}

impl Default for Opts {
    fn default() -> Self {
        Self {
            vault: None,
            tokens: 1,
            catalog: None,
            provider_test_timeout: None,
        }
    }
}

pub struct TestServer {
    pub addr: SocketAddr,
    pub clock: Clock,
    pub tokens: Vec<String>,
    pub login: LoginHandle,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    handle: Option<JoinHandle<Result<(), ServeError>>>,
}

pub fn config(
    world: &World,
    opts: &Opts,
    clock: &Clock,
) -> Result<Config, Box<dyn std::error::Error>> {
    Ok(Config {
        bind: "127.0.0.1:0".parse()?,
        db_path: world.db.clone(),
        scope: Scope {
            org: world.org.clone(),
            user: world.user.clone(),
        },
        vault: opts.vault.clone(),
        clock: clock.clone(),
        catalog: match &opts.catalog {
            Some(catalog) => catalog.clone(),
            None => i18n::Catalog::builtin()?,
        },
        provider_test_timeout: opts
            .provider_test_timeout
            .unwrap_or(server::DEFAULT_PROVIDER_TEST_TIMEOUT),
    })
}

impl TestServer {
    pub fn start(world: &World) -> Result<Self, Box<dyn std::error::Error>> {
        Self::start_with(world, Opts::default())
    }

    pub fn start_with(world: &World, opts: Opts) -> Result<Self, Box<dyn std::error::Error>> {
        let clock = Clock::system();
        let config = config(world, &opts, &clock)?;
        let wanted = opts.tokens;
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
        let handle = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| ServeError::Io(e.to_string()))?;
            runtime.block_on(async move {
                let server = Server::bind(config).await?;
                let mut tokens = Vec::new();
                for _ in 0..wanted {
                    tokens.push(server.issue_token()?);
                }
                let _ = ready_tx.send((server.local_addr(), tokens, server.login_handle()));
                server
                    .run(async move {
                        let _ = stop_rx.await;
                    })
                    .await
            })
        });
        let (addr, tokens, login) = ready_rx.recv_timeout(Duration::from_secs(10))?;
        Ok(Self {
            addr,
            clock,
            tokens,
            login,
            stop: Some(stop_tx),
            handle: Some(handle),
        })
    }

    /// Signs in with the first token and returns a session.
    pub fn sign_in(&self) -> Result<Session, Box<dyn std::error::Error>> {
        self.sign_in_with(&self.tokens[0])
    }

    pub fn sign_in_with(&self, token: &str) -> Result<Session, Box<dyn std::error::Error>> {
        let r = exchange(self.addr, token);
        assert_eq!(r.status, 204, "{}", r.text());
        let cookie = r
            .header("set-cookie")
            .and_then(|c| c.split(';').next())
            .ok_or("no cookie")?
            .to_owned();
        Ok(Session {
            addr: self.addr,
            cookie,
        })
    }

    /// Stops the server and returns how `run` ended.
    pub fn stop(mut self) -> Result<(), ServeError> {
        self.shutdown()
    }

    fn shutdown(&mut self) -> Result<(), ServeError> {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        match self.handle.take() {
            Some(handle) => handle
                .join()
                .unwrap_or(Err(ServeError::Io("panic".to_owned()))),
            None => Ok(()),
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

// ---- a raw HTTP/1.1 client ---------------------------------------------------------------------

pub struct Http {
    pub status: u16,
    /// Names are lower-cased.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Http {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }

    /// Every header, as one text (to look for something that must not be there).
    pub fn header_text(&self) -> String {
        self.headers
            .iter()
            .map(|(n, v)| format!("{n}: {v}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn code(&self) -> String {
        self.json()["error"]["code"]
            .as_str()
            .unwrap_or("")
            .to_owned()
    }
}

fn decode_chunked(mut data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let Some(pos) = data.windows(2).position(|w| w == b"\r\n") else {
            return out;
        };
        let size =
            usize::from_str_radix(String::from_utf8_lossy(&data[..pos]).trim(), 16).unwrap_or(0);
        data = &data[pos + 2..];
        if size == 0 || data.len() < size {
            if size != 0 {
                out.extend_from_slice(data);
            }
            return out;
        }
        out.extend_from_slice(&data[..size]);
        data = data.get(size + 2..).unwrap_or_default();
    }
}

pub fn parse_response(bytes: &[u8]) -> Http {
    let split = bytes
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .unwrap_or(bytes.len());
    let head = String::from_utf8_lossy(&bytes[..split]).into_owned();
    let rest = bytes.get(split + 4..).unwrap_or_default();
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| {
            let (n, v) = l.split_once(':')?;
            Some((n.trim().to_ascii_lowercase(), v.trim().to_owned()))
        })
        .collect();
    let chunked = headers
        .iter()
        .any(|(n, v)| n == "transfer-encoding" && v.contains("chunked"));
    let body = if chunked {
        decode_chunked(rest)
    } else {
        rest.to_vec()
    };
    Http {
        status,
        headers,
        body,
    }
}

/// A request to build; `Host` defaults to the address of the server.
pub struct Req {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    host: Option<Option<String>>,
    body: Option<Vec<u8>>,
}

impl Req {
    pub fn new(method: &str, path: &str) -> Self {
        Self {
            method: method.to_owned(),
            path: path.to_owned(),
            headers: Vec::new(),
            host: None,
            body: None,
        }
    }

    pub fn get(path: &str) -> Self {
        Self::new("GET", path)
    }

    pub fn post(path: &str) -> Self {
        Self::new("POST", path)
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    pub fn host(mut self, host: &str) -> Self {
        self.host = Some(Some(host.to_owned()));
        self
    }

    pub fn no_host(mut self) -> Self {
        self.host = Some(None);
        self
    }

    pub fn body(mut self, body: &[u8]) -> Self {
        self.body = Some(body.to_vec());
        self
    }

    pub fn json(self, value: &Value) -> Self {
        self.header("Content-Type", "application/json")
            .body(value.to_string().as_bytes())
    }

    /// The headers a browser page of this server sends with a changing request.
    pub fn same_origin(self, addr: SocketAddr) -> Self {
        self.header("Origin", &format!("http://{addr}"))
            .header("X-Nodivara", "1")
    }

    pub fn cookie(self, cookie: &str) -> Self {
        self.header("Cookie", cookie)
    }

    pub fn bytes(&self, addr: SocketAddr) -> Vec<u8> {
        let mut head = format!("{} {} HTTP/1.1\r\n", self.method, self.path);
        match &self.host {
            None => head.push_str(&format!("Host: {addr}\r\n")),
            Some(Some(host)) => head.push_str(&format!("Host: {host}\r\n")),
            Some(None) => {}
        }
        for (name, value) in &self.headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        let body = self.body.clone().unwrap_or_default();
        if self.body.is_some() {
            head.push_str(&format!("Content-Length: {}\r\n", body.len()));
        }
        head.push_str("Connection: close\r\n\r\n");
        let mut out = head.into_bytes();
        out.extend(body);
        out
    }

    pub fn send(&self, addr: SocketAddr) -> Http {
        raw(addr, &self.bytes(addr))
    }
}

/// Sends bytes and reads the answer until the server closes the connection.
pub fn raw(addr: SocketAddr, request: &[u8]) -> Http {
    let Ok(mut stream) = TcpStream::connect(addr) else {
        return Http {
            status: 0,
            headers: Vec::new(),
            body: Vec::new(),
        };
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(20)));
    // The server may answer and close before a large body is written: that is not an error here.
    let _ = stream.write_all(request);
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => bytes.extend_from_slice(&chunk[..n]),
        }
    }
    parse_response(&bytes)
}

pub fn exchange(addr: SocketAddr, token: &str) -> Http {
    Req::post("/api/auth/exchange")
        .same_origin(addr)
        .json(&json!({"token": token}))
        .send(addr)
}

/// A signed-in browser: every call carries the cookie and the headers of a same-origin page.
#[derive(Clone)]
pub struct Session {
    pub addr: SocketAddr,
    pub cookie: String,
}

impl Session {
    pub fn get(&self, path: &str) -> Http {
        Req::get(path).cookie(&self.cookie).send(self.addr)
    }

    pub fn get_lang(&self, path: &str, accept_language: &str) -> Http {
        Req::get(path)
            .cookie(&self.cookie)
            .header("Accept-Language", accept_language)
            .send(self.addr)
    }

    pub fn post(&self, path: &str, body: &Value) -> Http {
        Req::post(path)
            .same_origin(self.addr)
            .cookie(&self.cookie)
            .json(body)
            .send(self.addr)
    }

    pub fn post_empty(&self, path: &str) -> Http {
        Req::post(path)
            .same_origin(self.addr)
            .cookie(&self.cookie)
            .send(self.addr)
    }

    pub fn delete(&self, path: &str) -> Http {
        Req::new("DELETE", path)
            .same_origin(self.addr)
            .cookie(&self.cookie)
            .send(self.addr)
    }

    pub fn req(&self, request: Req) -> Http {
        request.cookie(&self.cookie).send(self.addr)
    }
}

/// The events of a stream: `(event, data)` pairs.
pub fn sse_events(body: &str) -> Vec<(String, Value)> {
    body.split("\n\n")
        .filter(|block| !block.trim().is_empty())
        .map(|block| {
            let mut event = String::new();
            let mut data = String::new();
            for line in block.lines() {
                if let Some(rest) = line.strip_prefix("event: ") {
                    event = rest.to_owned();
                } else if let Some(rest) = line.strip_prefix("data: ") {
                    data.push_str(rest);
                }
            }
            (event, serde_json::from_str(&data).unwrap_or(Value::Null))
        })
        .collect()
}

// ---- a fake OpenAI-compatible server ------------------------------------------------------------

#[derive(Clone)]
pub enum Reply {
    /// 200 with these Server-Sent Events (each already a JSON value).
    Sse(Vec<Value>),
    /// An error status with a JSON body and extra headers.
    Status(u16, &'static str, Vec<(&'static str, &'static str)>),
    /// 200 with a body that is not a stream of events.
    Garbage,
    /// Answers after a pause.
    After(u64, Box<Reply>),
}

pub struct Fake {
    pub port: u16,
    seen: Arc<Mutex<Vec<String>>>,
}

impl Fake {
    pub fn start(script: Vec<Reply>) -> std::io::Result<Self> {
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
                let reply = replies
                    .next()
                    .unwrap_or(Reply::Status(500, "{}", Vec::new()));
                respond(&mut stream, &reply);
            }
        });
        Ok(Self { port, seen })
    }

    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}/v1", self.port)
    }

    pub fn requests(&self) -> Vec<String> {
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
        Reply::After(ms, inner) => {
            std::thread::sleep(Duration::from_millis(*ms));
            respond(stream, inner);
            return;
        }
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
        Reply::Garbage => {
            let body = "<html>this is not an event stream</html>";
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(body.as_bytes());
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

pub fn text_reply(pieces: &[&str]) -> Reply {
    let mut events: Vec<Value> = pieces
        .iter()
        .map(|p| json!({"choices":[{"delta":{"content":p}}]}))
        .collect();
    events.push(json!({"choices":[{"delta":{},"finish_reason":"stop"}]}));
    Reply::Sse(events)
}

pub fn tool_reply(id: &str, name: &str, args: &str) -> Reply {
    Reply::Sse(vec![
        json!({"choices":[{"delta":{"role":"assistant","tool_calls":[
            {"index":0,"id":id,"type":"function","function":{"name":name,"arguments":args}}]}}]}),
        json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
    ])
}

/// Provider settings that allow the loopback fake and never retry (a test must see one call).
pub const PARAMS: &str = r#"{"allow_private_network":true,"max_retries":0}"#;

/// Stores a provider pointing at the fake, sealing `key` with `vault` when given.
pub fn add_provider(
    world: &World,
    fake: &Fake,
    key: Option<(&Vault, &str)>,
    is_default: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let conn = world.conn()?;
    bootstrap::add_provider(
        &conn,
        &world.org,
        &bootstrap::NewProvider {
            name: "main",
            base_url: &fake.base_url(),
            model: "test-model",
            params_json: PARAMS,
            is_default,
        },
        key,
    )
    .map_err(|e| format!("{e:?}"))?;
    Ok(())
}

/// The headers every response of the server must carry.
pub fn assert_security_headers(r: &Http, api: bool) {
    assert_eq!(
        r.header("content-security-policy"),
        Some(server_csp()),
        "CSP on {} {}",
        r.status,
        r.text()
    );
    assert_eq!(r.header("x-content-type-options"), Some("nosniff"));
    assert_eq!(r.header("referrer-policy"), Some("no-referrer"));
    assert_eq!(r.header("cross-origin-opener-policy"), Some("same-origin"));
    if api {
        assert_eq!(r.header("cache-control"), Some("no-store"));
    }
}

pub fn server_csp() -> &'static str {
    "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"
}
