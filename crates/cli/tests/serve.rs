//! `infra serve`: argument handling and failures in process, and the real binary end to end (the
//! printed address carries a token that works once; Ctrl-C and SIGTERM stop it cleanly).

use cli::Env;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

type TestResult = Result<(), Box<dyn std::error::Error>>;

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> std::io::Result<Self> {
        static N: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "nodivara-cli-serve-{}-{}-{tag}",
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

struct Out {
    code: u8,
    stdout: String,
    stderr: String,
}

fn run(args: &[&str], env: &[(&str, &str)]) -> Out {
    let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = cli::run_with(
        &args,
        None,
        &Env::fixed(env),
        &mut std::io::empty(),
        &mut out,
        &mut err,
    );
    Out {
        code,
        stdout: String::from_utf8_lossy(&out).into_owned(),
        stderr: String::from_utf8_lossy(&err).into_owned(),
    }
}

fn init(scratch: &Scratch) {
    let r = run(&["init", "--db", &scratch.db()], &[]);
    assert_eq!(r.code, 0, "{}", r.stderr);
}

// ---- arguments and failures, in process --------------------------------------------------------

#[test]
fn bad_arguments_are_usage_errors() -> TestResult {
    let scratch = Scratch::new("args")?;
    init(&scratch);
    let db = scratch.db();
    for args in [
        vec!["serve", "extra", "--db", &db],
        vec!["serve", "--bogus", "--db", &db],
        vec!["serve", "--port", "--db", &db],
        vec!["serve", "--port", "1", "--port", "2", "--db", &db],
        vec!["serve", "--lang", "xx", "--db", &db],
    ] {
        let r = run(&args, &[]);
        assert_eq!(r.code, 2, "{args:?}");
        assert!(r.stderr.contains("infra serve"), "{args:?}: {}", r.stderr);
        assert!(r.stdout.is_empty());
    }
    Ok(())
}

#[test]
fn a_bad_port_is_explained_in_the_readers_language() -> TestResult {
    let scratch = Scratch::new("port")?;
    init(&scratch);
    let db = scratch.db();
    for port in ["abc", "70000", "-1", "", "80.5"] {
        let r = run(&["serve", "--port", port, "--db", &db], &[]);
        assert_eq!(r.code, 2, "{port}");
        assert!(r.stderr.contains("port must be a number"), "{}", r.stderr);
    }
    let r = run(
        &["serve", "--port", "abc", "--db", &db, "--lang", "id"],
        &[],
    );
    assert!(r.stderr.contains("Port harus berupa angka"), "{}", r.stderr);
    Ok(())
}

#[test]
fn a_missing_database_or_organization_points_at_init() -> TestResult {
    let scratch = Scratch::new("setup")?;
    let missing = scratch.db();
    let r = run(&["serve", "--db", &missing, "--port", "0"], &[]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("infra init"), "{}", r.stderr);
    assert!(
        !std::path::Path::new(&missing).exists(),
        "serve must not create the database"
    );
    // A database that exists but has no organization.
    drop(db::open(std::path::Path::new(&missing))?);
    let r = run(&["serve", "--db", &missing, "--port", "0"], &[]);
    assert_eq!(r.code, 2);
    assert!(
        r.stderr.contains("no organization") && r.stderr.contains("infra init"),
        "{}",
        r.stderr
    );
    let r = run(
        &["serve", "--db", &missing, "--port", "0", "--lang", "id"],
        &[],
    );
    assert!(r.stderr.contains("infra init"), "{}", r.stderr);
    Ok(())
}

#[test]
fn several_organizations_need_a_choice() -> TestResult {
    let scratch = Scratch::new("orgs")?;
    init(&scratch);
    let conn = db::open(std::path::Path::new(&scratch.db()))?;
    let second = db::create_org(&conn, "Second", "second", "free")?;
    let user = db::create_user(&conn, "two@example.test", "Two", None)?;
    db::add_member(&conn, &second.id, &user.id, db::Role::Owner)?;
    drop(conn);
    let db = scratch.db();
    let r = run(&["serve", "--db", &db, "--port", "0"], &[]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("--org"), "{}", r.stderr);
    let r = run(&["serve", "--db", &db, "--port", "0", "--org", "nope"], &[]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("nope"), "{}", r.stderr);
    Ok(())
}

#[test]
fn an_invalid_master_key_is_refused_but_a_missing_one_is_not_an_error_here() -> TestResult {
    let scratch = Scratch::new("key")?;
    init(&scratch);
    let r = run(
        &["serve", "--db", &scratch.db(), "--port", "0"],
        &[("NODIVARA_MASTER_KEY", "garbage")],
    );
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("not valid"), "{}", r.stderr);
    assert!(!r.stderr.contains("garbage"));
    Ok(())
}

#[test]
fn a_busy_port_is_explained() -> TestResult {
    let scratch = Scratch::new("busy")?;
    init(&scratch);
    let taken = TcpListener::bind("127.0.0.1:0")?;
    let port = taken.local_addr()?.port().to_string();
    let r = run(&["serve", "--db", &scratch.db(), "--port", &port], &[]);
    assert_eq!(r.code, 2);
    assert!(
        r.stderr.contains(&format!("Port {port} is already in use")),
        "{}",
        r.stderr
    );
    assert!(
        r.stdout.is_empty(),
        "no address may be printed for a server that did not start"
    );
    let r = run(
        &[
            "serve",
            "--db",
            &scratch.db(),
            "--port",
            &port,
            "--lang",
            "id",
        ],
        &[],
    );
    assert!(r.stderr.contains("sudah dipakai"), "{}", r.stderr);
    Ok(())
}

#[test]
fn serve_is_listed_in_the_general_usage() {
    let r = run(&[], &[]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("infra serve"), "{}", r.stderr);
}

// ---- the real binary -----------------------------------------------------------------------------

struct Running {
    child: Child,
    port: u16,
    token: String,
    #[cfg(unix)]
    reader: BufReader<std::process::ChildStdout>,
}

impl Drop for Running {
    fn drop(&mut self) {
        // Only the process this test started.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start(scratch: &Scratch) -> Result<Running, Box<dyn std::error::Error>> {
    init(scratch);
    let mut child = Command::new(env!("CARGO_BIN_EXE_infra"))
        .args(["serve", "--port", "0", "--db", &scratch.db()])
        .env_remove("NODIVARA_MASTER_KEY")
        .env_remove("NODIVARA_DB")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .spawn()?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let mut reader = BufReader::new(stdout);
    let mut url = None;
    let mut line = String::new();
    while url.is_none() {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Err("the server exited before printing its address".into());
        }
        if line.starts_with("http://") {
            url = Some(line.trim().to_owned());
        }
    }
    let url = url.ok_or("no url")?;
    let rest = url
        .strip_prefix("http://127.0.0.1:")
        .ok_or("not loopback")?;
    let (port, token) = rest.split_once("/#t=").ok_or("no token fragment")?;
    Ok(Running {
        child,
        port: port.parse()?,
        token: token.to_owned(),
        #[cfg(unix)]
        reader,
    })
}

fn http(port: u16, request: &str) -> Result<String, Box<dyn std::error::Error>> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(10)))?;
    stream.write_all(request.as_bytes())?;
    let mut out = Vec::new();
    stream.read_to_end(&mut out)?;
    Ok(String::from_utf8_lossy(&out).into_owned())
}

fn exchange(port: u16, token: &str) -> Result<String, Box<dyn std::error::Error>> {
    let body = format!(r#"{{"token":"{token}"}}"#);
    http(
        port,
        &format!(
            "POST /api/auth/exchange HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://127.0.0.1:{port}\r\n\
             X-Nodivara: 1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ),
    )
}

#[cfg(unix)]
fn signal(child: &Child, name: &str) -> TestResult {
    // The pid of the child this test started, and nothing else.
    let status = Command::new("kill")
        .args([name, &child.id().to_string()])
        .status()?;
    assert!(status.success());
    Ok(())
}

#[test]
fn the_printed_address_works_once() -> TestResult {
    let scratch = Scratch::new("once")?;
    let mut running = start(&scratch)?;
    assert_eq!(running.token.len(), 64);
    assert!(running.token.bytes().all(|b| b.is_ascii_hexdigit()));
    let health = http(
        running.port,
        &format!(
            "GET /healthz HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
            running.port
        ),
    )?;
    assert!(health.starts_with("HTTP/1.1 200"), "{health}");
    let first = exchange(running.port, &running.token)?;
    assert!(first.starts_with("HTTP/1.1 204"), "{first}");
    assert!(
        first
            .to_ascii_lowercase()
            .contains("set-cookie: nv_session=")
    );
    let second = exchange(running.port, &running.token)?;
    assert!(
        second.starts_with("HTTP/1.1 401"),
        "the token must work once: {second}"
    );
    let _ = running.child.kill();
    let _ = running.child.wait();
    Ok(())
}

#[cfg(unix)]
#[test]
fn the_server_stops_on_sigterm() -> TestResult {
    let scratch = Scratch::new("term")?;
    let mut running = start(&scratch)?;
    signal(&running.child, "-TERM")?;
    let status = running.child.wait()?;
    assert!(status.success(), "a clean stop exits 0: {status:?}");
    let mut rest = String::new();
    running.reader.read_to_string(&mut rest)?;
    assert!(rest.contains("Stopped."), "{rest}");
    Ok(())
}

#[cfg(unix)]
#[test]
fn the_server_stops_cleanly_on_ctrl_c() -> TestResult {
    let scratch = Scratch::new("int")?;
    let mut running = start(&scratch)?;
    signal(&running.child, "-INT")?;
    let status = running.child.wait()?;
    assert!(status.success(), "{status:?}");
    Ok(())
}

#[test]
fn serving_leaves_the_database_owner_only() -> TestResult {
    // `init` makes the file owner-only and serve must leave it so (it never creates one).
    let scratch = Scratch::new("perm")?;
    let running = start(&scratch)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(scratch.db())?.permissions().mode() & 0o777;
        assert_eq!(mode & 0o077, 0, "{mode:o}");
    }
    drop(running);
    Ok(())
}
