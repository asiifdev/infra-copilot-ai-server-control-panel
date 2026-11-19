//! Behaviour tests against local listeners on 127.0.0.1 only. No real third-party host is used.

use net::{CertInfo, Client, NetError, Policy, TlsFailure};
use rcgen::{CertificateParams, DnType, KeyPair, date_time_ymd};
use rustls::ServerConfig;
use rustls::crypto::ring;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use std::error::Error;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

type R = Result<(), Box<dyn Error>>;

/// What the test server does with a request.
enum Action {
    Raw(Vec<u8>),
    /// Accept and never answer.
    Hang,
    /// Valid head with `Content-Length` far above what is sent, then an endless body.
    Flood,
}

type Handler = Arc<dyn Fn(&str, &[u8]) -> Action + Send + Sync>;

fn reply(status: &str, headers: &str, body: &[u8]) -> Action {
    let mut out = format!(
        "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body);
    Action::Raw(out)
}

async fn serve_conn<S: AsyncRead + AsyncWrite + Unpin>(mut stream: S, handler: Handler) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(chunk.get(..n).unwrap_or_default()),
        }
    };
    let head = String::from_utf8_lossy(buf.get(..head_end).unwrap_or_default()).into_owned();
    let wanted: usize = head
        .lines()
        .find_map(|l| {
            l.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .and_then(|v| v.trim().parse().ok())
        })
        .unwrap_or(0);
    while buf.len() < head_end + wanted {
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(chunk.get(..n).unwrap_or_default()),
        }
    }
    let body = buf.get(head_end..).unwrap_or_default().to_vec();
    match handler(&head, &body) {
        Action::Raw(bytes) => {
            let _ = stream.write_all(&bytes).await;
            let _ = stream.shutdown().await;
        }
        Action::Hang => tokio::time::sleep(Duration::from_secs(30)).await,
        Action::Flood => {
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100000000\r\n\r\n")
                .await;
            let block = [b'a'; 8192];
            while stream.write_all(&block).await.is_ok() {}
        }
    }
}

async fn http_server(
    handler: impl Fn(&str, &[u8]) -> Action + Send + Sync + 'static,
) -> Result<u16, Box<dyn Error>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let handler: Handler = Arc::new(handler);
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(serve_conn(stream, Arc::clone(&handler)));
        }
    });
    Ok(port)
}

struct TestCert {
    der: Vec<u8>,
    key: Vec<u8>,
}

fn make_cert(
    not_before: (i32, u8, u8),
    not_after: (i32, u8, u8),
) -> Result<TestCert, Box<dyn Error>> {
    let mut params = CertificateParams::new(vec!["localhost".to_owned()])?;
    params.not_before = date_time_ymd(not_before.0, not_before.1, not_before.2);
    params.not_after = date_time_ymd(not_after.0, not_after.1, not_after.2);
    // A distinct subject per certificate, so a different "trust anchor" does not match by name.
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    params
        .distinguished_name
        .push(DnType::CommonName, format!("test cert {id}"));
    let key = KeyPair::generate()?;
    let cert = params.self_signed(&key)?;
    Ok(TestCert {
        der: cert.der().to_vec(),
        key: key.serialize_der(),
    })
}

fn valid_cert() -> Result<TestCert, Box<dyn Error>> {
    make_cert((2020, 1, 1), (2100, 1, 1))
}

async fn https_server(
    cert: &TestCert,
    handler: impl Fn(&str, &[u8]) -> Action + Send + Sync + 'static,
) -> Result<u16, Box<dyn Error>> {
    let config = ServerConfig::builder_with_provider(Arc::new(ring::default_provider()))
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(cert.der.clone())],
            PrivateKeyDer::from(PrivatePkcs8KeyDer::from(cert.key.clone())),
        )?;
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let handler: Handler = Arc::new(handler);
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let acceptor = acceptor.clone();
            let handler = Arc::clone(&handler);
            tokio::spawn(async move {
                if let Ok(tls) = acceptor.accept(stream).await {
                    serve_conn(tls, handler).await;
                }
            });
        }
    });
    Ok(port)
}

/// Loopback allowed, plain HTTP allowed: what tests need.
fn local() -> Policy {
    Policy {
        block_private_addresses: false,
        allow_http: true,
        timeout: Duration::from_secs(5),
        ..Policy::default()
    }
}

fn client_trusting(cert: &TestCert) -> Result<Client, NetError> {
    Client::with_root_certificates([cert.der.clone()])
}

fn client_for_http() -> Result<Client, Box<dyn Error>> {
    // Trust store content is irrelevant for plain HTTP; any one valid certificate will do.
    Ok(client_trusting(&valid_cert()?)?)
}

#[tokio::test]
async fn get_returns_status_body_and_final_url() -> R {
    let port = http_server(|head, _| {
        assert!(head.starts_with("GET /hello?x=1 HTTP/1.1\r\n"), "{head}");
        assert!(head.contains("user-agent: monitor/1"), "{head}");
        assert!(!head.to_ascii_lowercase().contains("authorization"));
        reply("200 OK", "", b"hi there")
    })
    .await?;
    let res = client_for_http()?
        .get(&format!("http://127.0.0.1:{port}/hello?x=1#frag"), &local())
        .await?;
    assert_eq!(res.status, 200);
    assert_eq!(res.body, b"hi there");
    assert!(!res.body_truncated);
    assert!(res.tls.is_none());
    assert_eq!(res.final_url, format!("http://127.0.0.1:{port}/hello?x=1"));
    Ok(())
}

#[tokio::test]
async fn non_2xx_status_is_a_response_not_an_error() -> R {
    let port = http_server(|_, _| reply("503 Service Unavailable", "", b"down")).await?;
    let res = client_for_http()?
        .get(&format!("http://127.0.0.1:{port}/"), &local())
        .await?;
    assert_eq!(res.status, 503);
    Ok(())
}

#[tokio::test]
async fn loopback_is_blocked_by_default_and_allowed_when_switched_off() -> R {
    let port = http_server(|_, _| reply("200 OK", "", b"ok")).await?;
    let client = client_for_http()?;
    let url = format!("http://127.0.0.1:{port}/");
    let blocked = Policy {
        block_private_addresses: true,
        allow_http: true,
        ..Policy::default()
    };
    assert_eq!(
        client.get(&url, &blocked).await.err(),
        Some(NetError::BlockedAddress)
    );
    assert_eq!(
        client.tcp_connect("127.0.0.1", port, &blocked).await.err(),
        Some(NetError::BlockedAddress)
    );
    assert_eq!(
        client
            .tcp_connect("::ffff:127.0.0.1", port, &blocked)
            .await
            .err(),
        Some(NetError::BlockedAddress)
    );
    assert_eq!(
        client
            .tls_certificate("127.0.0.1", port, &blocked)
            .await
            .err(),
        Some(NetError::BlockedAddress)
    );
    assert_eq!(client.get(&url, &local()).await?.status, 200);
    Ok(())
}

#[tokio::test]
async fn plain_http_needs_allow_http() -> R {
    let port = http_server(|_, _| reply("200 OK", "", b"ok")).await?;
    let policy = Policy {
        block_private_addresses: false,
        ..Policy::default()
    };
    let res = client_for_http()?
        .get(&format!("http://127.0.0.1:{port}/"), &policy)
        .await;
    assert_eq!(res.err(), Some(NetError::SchemeNotAllowed));
    Ok(())
}

#[tokio::test]
async fn userinfo_and_bad_urls_are_rejected_without_leaking() -> R {
    let client = client_for_http()?;
    let err = client
        .get("https://admin:hunter2@example.invalid/?token=abc", &local())
        .await
        .err();
    assert_eq!(err, Some(NetError::UserInfoNotAllowed));
    let text = err.map(|e| e.to_string()).unwrap_or_default();
    assert!(!text.contains("hunter2") && !text.contains("admin") && !text.contains("token"));
    assert_eq!(
        client.get("ftp://example.invalid/", &local()).await.err(),
        Some(NetError::SchemeNotAllowed)
    );
    assert_eq!(
        client.get("not a url", &local()).await.err(),
        Some(NetError::InvalidUrl)
    );
    assert_eq!(
        client.get("//example.invalid/", &local()).await.err(),
        Some(NetError::InvalidUrl)
    );
    Ok(())
}

#[tokio::test]
async fn redirect_chain_within_cap_is_followed() -> R {
    let port = http_server(|head, _| {
        let path = head.split_whitespace().nth(1).unwrap_or("/");
        match path {
            "/a" => reply("302 Found", "Location: /b\r\n", b""),
            "/b" => reply("301 Moved Permanently", "Location: c?k=v\r\n", b""),
            "/c" => reply("200 OK", "", b"x"),
            _ if path.starts_with("/c?") => reply("200 OK", "", b"end"),
            _ => reply("404 Not Found", "", b""),
        }
    })
    .await?;
    let res = client_for_http()?
        .get(&format!("http://127.0.0.1:{port}/a"), &local())
        .await?;
    assert_eq!((res.status, res.body.as_slice()), (200, b"end".as_slice()));
    assert_eq!(res.final_url, format!("http://127.0.0.1:{port}/c?k=v"));
    Ok(())
}

#[tokio::test]
async fn redirect_chain_over_cap_fails_and_loops_terminate() -> R {
    let port = http_server(|head, _| {
        let n: u32 = head
            .split_whitespace()
            .nth(1)
            .and_then(|p| p.trim_start_matches('/').parse().ok())
            .unwrap_or(0);
        reply("302 Found", &format!("Location: /{}\r\n", n + 1), b"")
    })
    .await?;
    let client = client_for_http()?;
    let url = format!("http://127.0.0.1:{port}/0");
    // Exactly 3 redirects allowed: hop 4 still answers with a redirect.
    assert_eq!(
        client.get(&url, &local()).await.err(),
        Some(NetError::TooManyRedirects)
    );
    let none = Policy {
        max_redirects: 0,
        ..local()
    };
    assert_eq!(
        client.get(&url, &none).await.err(),
        Some(NetError::TooManyRedirects)
    );
    Ok(())
}

#[tokio::test]
async fn redirect_exactly_at_the_cap_succeeds() -> R {
    let port = http_server(|head, _| {
        let n: u32 = head
            .split_whitespace()
            .nth(1)
            .and_then(|p| p.trim_start_matches('/').parse().ok())
            .unwrap_or(0);
        if n < 3 {
            reply("302 Found", &format!("Location: /{}\r\n", n + 1), b"")
        } else {
            reply("200 OK", "", b"ok")
        }
    })
    .await?;
    let res = client_for_http()?
        .get(&format!("http://127.0.0.1:{port}/0"), &local())
        .await?;
    assert_eq!(res.status, 200);
    Ok(())
}

#[tokio::test]
async fn redirect_without_location_is_returned_as_is() -> R {
    let port = http_server(|_, _| reply("302 Found", "", b"")).await?;
    let res = client_for_http()?
        .get(&format!("http://127.0.0.1:{port}/"), &local())
        .await?;
    assert_eq!(res.status, 302);
    Ok(())
}

#[tokio::test]
async fn redirect_to_other_scheme_or_userinfo_is_refused() -> R {
    let port = http_server(
        |head, _| match head.split_whitespace().nth(1).unwrap_or("/") {
            "/ftp" => reply("302 Found", "Location: ftp://example.invalid/\r\n", b""),
            "/creds" => reply("302 Found", "Location: http://u:p@127.0.0.1/\r\n", b""),
            _ => reply("302 Found", "Location: \r\n", b""),
        },
    )
    .await?;
    let client = client_for_http()?;
    let base = format!("http://127.0.0.1:{port}");
    assert_eq!(
        client.get(&format!("{base}/ftp"), &local()).await.err(),
        Some(NetError::SchemeNotAllowed)
    );
    assert_eq!(
        client.get(&format!("{base}/creds"), &local()).await.err(),
        Some(NetError::UserInfoNotAllowed)
    );
    assert_eq!(
        client.get(&format!("{base}/empty"), &local()).await.err(),
        Some(NetError::InvalidRedirect)
    );
    Ok(())
}

#[tokio::test]
async fn oversized_body_is_truncated_and_flagged() -> R {
    let big = vec![b'z'; 1_000_000];
    let port = http_server(move |_, _| reply("200 OK", "", &big)).await?;
    let policy = Policy {
        max_body_bytes: 1000,
        ..local()
    };
    let res = client_for_http()?
        .get(&format!("http://127.0.0.1:{port}/"), &policy)
        .await?;
    assert_eq!(res.body.len(), 1000);
    assert!(res.body_truncated);
    Ok(())
}

#[tokio::test]
async fn body_exactly_at_the_cap_is_not_flagged() -> R {
    let port = http_server(|_, _| reply("200 OK", "", &[b'q'; 100])).await?;
    let policy = Policy {
        max_body_bytes: 100,
        ..local()
    };
    let res = client_for_http()?
        .get(&format!("http://127.0.0.1:{port}/"), &policy)
        .await?;
    assert_eq!((res.body.len(), res.body_truncated), (100, false));
    Ok(())
}

#[tokio::test]
async fn endless_body_stops_at_the_cap() -> R {
    let port = http_server(|_, _| Action::Flood).await?;
    let policy = Policy {
        max_body_bytes: 10_000,
        ..local()
    };
    let res = client_for_http()?
        .get(&format!("http://127.0.0.1:{port}/"), &policy)
        .await?;
    assert_eq!(res.body.len(), 10_000);
    assert!(res.body_truncated);
    Ok(())
}

#[tokio::test]
async fn slow_server_hits_the_total_timeout() -> R {
    let port = http_server(|_, _| Action::Hang).await?;
    let policy = Policy {
        timeout: Duration::from_millis(300),
        ..local()
    };
    let started = std::time::Instant::now();
    let res = client_for_http()?
        .get(&format!("http://127.0.0.1:{port}/"), &policy)
        .await;
    assert_eq!(res.err(), Some(NetError::Timeout));
    assert!(started.elapsed() < Duration::from_secs(3));
    Ok(())
}

#[tokio::test]
async fn huge_response_head_is_refused() -> R {
    let port = http_server(|_, _| {
        let headers = format!("X-Pad: {}\r\n", "p".repeat(200_000));
        reply("200 OK", &headers, b"")
    })
    .await?;
    let res = client_for_http()?
        .get(&format!("http://127.0.0.1:{port}/"), &local())
        .await;
    assert_eq!(res.err(), Some(NetError::Protocol));
    Ok(())
}

#[tokio::test]
async fn garbage_instead_of_http_is_a_protocol_error() -> R {
    let port = http_server(|_, _| Action::Raw(b"not http at all\r\n\r\n".to_vec())).await?;
    let res = client_for_http()?
        .get(&format!("http://127.0.0.1:{port}/"), &local())
        .await;
    assert_eq!(res.err(), Some(NetError::Protocol));
    Ok(())
}

#[tokio::test]
async fn post_json_sends_body_and_content_type_and_does_not_follow_redirects() -> R {
    let port = http_server(|head, body| {
        let lower = head.to_ascii_lowercase();
        assert!(head.starts_with("POST /bot/send HTTP/1.1\r\n"), "{head}");
        assert!(lower.contains("content-type: application/json"), "{head}");
        assert!(lower.contains("content-length: 13"), "{head}");
        if head.contains("/bot/send") && body == br#"{"text":"hi"}"# {
            reply("200 OK", "", b"{\"ok\":true}")
        } else {
            reply("400 Bad Request", "", b"")
        }
    })
    .await?;
    let client = client_for_http()?;
    let res = client
        .post_json(
            &format!("http://127.0.0.1:{port}/bot/send"),
            br#"{"text":"hi"}"#,
            &local(),
        )
        .await?;
    assert_eq!(
        (res.status, res.body.as_slice()),
        (200, b"{\"ok\":true}".as_slice())
    );

    let redirecting =
        http_server(|_, _| reply("307 Temporary Redirect", "Location: /elsewhere\r\n", b""))
            .await?;
    let res = client
        .post_json(
            &format!("http://127.0.0.1:{redirecting}/x"),
            b"{}",
            &local(),
        )
        .await?;
    assert_eq!(res.status, 307);
    Ok(())
}

#[tokio::test]
async fn tcp_connect_measures_and_reports_refusal() -> R {
    let port = http_server(|_, _| reply("200 OK", "", b"")).await?;
    let client = client_for_http()?;
    let took = client.tcp_connect("127.0.0.1", port, &local()).await?;
    assert!(took < Duration::from_secs(2));
    // A port that was just freed: connection refused.
    let closed = {
        let l = TcpListener::bind("127.0.0.1:0").await?;
        l.local_addr()?.port()
    };
    assert_eq!(
        client
            .tcp_connect("127.0.0.1", closed, &local())
            .await
            .err(),
        Some(NetError::Connect)
    );
    Ok(())
}

fn assert_cert(info: &CertInfo, not_before: i64, not_after: i64, trusted: bool) {
    assert_eq!(info.not_before, not_before);
    assert_eq!(info.not_after, not_after);
    assert_eq!(info.chain_trusted, trusted);
    assert!(!info.subject.is_empty());
    assert_eq!(
        info.issuer, info.subject,
        "self-signed: issuer equals subject"
    );
}

// 2020-01-01, 2021-01-01 and 2100-01-01 UTC.
const Y2020: i64 = 1_577_836_800;
const Y2021: i64 = 1_609_459_200;
const Y2100: i64 = 4_102_444_800;

#[tokio::test]
async fn self_signed_certificate_is_read_and_reported_untrusted() -> R {
    let cert = valid_cert()?;
    let port = https_server(&cert, |_, _| reply("200 OK", "", b"")).await?;
    // The client trusts an unrelated certificate, so the server's one is untrusted.
    let client = client_trusting(&valid_cert()?)?;
    let info = client.tls_certificate("localhost", port, &local()).await?;
    assert_cert(&info, Y2020, Y2100, false);
    Ok(())
}

#[tokio::test]
async fn trusted_certificate_reports_chain_trusted() -> R {
    let cert = valid_cert()?;
    let port = https_server(&cert, |_, _| reply("200 OK", "", b"")).await?;
    let info = client_trusting(&cert)?
        .tls_certificate("localhost", port, &local())
        .await?;
    assert_cert(&info, Y2020, Y2100, true);
    Ok(())
}

#[tokio::test]
async fn expired_certificate_is_read_and_flagged() -> R {
    let cert = make_cert((2020, 1, 1), (2021, 1, 1))?;
    let port = https_server(&cert, |_, _| reply("200 OK", "", b"")).await?;
    // Even when the certificate is the configured trust anchor, expiry makes it untrusted.
    let info = client_trusting(&cert)?
        .tls_certificate("localhost", port, &local())
        .await?;
    assert_cert(&info, Y2020, Y2021, false);
    assert!(info.not_after < 1_800_000_000, "expiry in the past");
    Ok(())
}

#[tokio::test]
async fn name_mismatch_is_reported_untrusted_but_readable() -> R {
    let cert = valid_cert()?; // valid for "localhost" only
    let port = https_server(&cert, |_, _| reply("200 OK", "", b"")).await?;
    let info = client_trusting(&cert)?
        .tls_certificate("127.0.0.1", port, &local())
        .await?;
    assert!(!info.chain_trusted);
    Ok(())
}

#[tokio::test]
async fn normal_path_refuses_untrusted_and_expired_certificates() -> R {
    let cert = valid_cert()?;
    let port = https_server(&cert, |_, _| reply("200 OK", "", b"must not be read")).await?;
    let client = client_trusting(&valid_cert()?)?;
    let res = client
        .get(&format!("https://localhost:{port}/"), &local())
        .await;
    assert_eq!(res.err(), Some(NetError::Tls(TlsFailure::UntrustedIssuer)));

    let expired = make_cert((2020, 1, 1), (2021, 1, 1))?;
    let port = https_server(&expired, |_, _| reply("200 OK", "", b"must not be read")).await?;
    let res = client_trusting(&expired)?
        .get(&format!("https://localhost:{port}/"), &local())
        .await;
    assert_eq!(res.err(), Some(NetError::Tls(TlsFailure::Expired)));
    Ok(())
}

#[tokio::test]
async fn trusted_https_get_returns_certificate_info() -> R {
    let cert = valid_cert()?;
    let port = https_server(&cert, |_, _| reply("200 OK", "", b"secure")).await?;
    let res = client_trusting(&cert)?
        .get(&format!("https://localhost:{port}/p"), &local())
        .await?;
    assert_eq!(
        (res.status, res.body.as_slice()),
        (200, b"secure".as_slice())
    );
    let info = res.tls.ok_or("missing tls info")?;
    assert_cert(&info, Y2020, Y2100, true);
    assert_eq!(res.final_url, format!("https://localhost:{port}/p"));
    Ok(())
}

#[tokio::test]
async fn https_to_http_redirect_is_refused() -> R {
    let plain = http_server(|_, _| reply("200 OK", "", b"plain")).await?;
    let cert = valid_cert()?;
    let location = format!("Location: http://localhost:{plain}/\r\n");
    let port = https_server(&cert, move |_, _| reply("302 Found", &location, b"")).await?;
    let res = client_trusting(&cert)?
        .get(&format!("https://localhost:{port}/"), &local())
        .await;
    assert_eq!(res.err(), Some(NetError::SchemeNotAllowed));
    Ok(())
}

#[tokio::test]
async fn http_to_https_upgrade_redirect_is_followed() -> R {
    let cert = valid_cert()?;
    let secure = https_server(&cert, |_, _| reply("200 OK", "", b"upgraded")).await?;
    let location = format!("Location: https://localhost:{secure}/\r\n");
    let plain = http_server(move |_, _| reply("301 Moved Permanently", &location, b"")).await?;
    let res = client_trusting(&cert)?
        .get(&format!("http://localhost:{plain}/"), &local())
        .await?;
    assert_eq!(res.body, b"upgraded");
    assert!(res.tls.is_some());
    Ok(())
}

#[test]
fn empty_root_set_is_a_clear_error() {
    let none: Vec<Vec<u8>> = vec![vec![1, 2, 3]];
    assert_eq!(
        Client::with_root_certificates(none).err(),
        Some(NetError::NoRootCertificates)
    );
    assert_eq!(
        Client::with_root_certificates(Vec::new()).err(),
        Some(NetError::NoRootCertificates)
    );
}

#[test]
fn error_text_is_fixed() {
    for e in [
        NetError::InvalidUrl,
        NetError::BlockedAddress,
        NetError::Tls(TlsFailure::Expired),
        NetError::Timeout,
    ] {
        assert!(!e.to_string().is_empty());
        let _: &dyn Error = &e;
    }
}
