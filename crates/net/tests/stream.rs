//! Extra headers and streamed bodies, against local listeners on 127.0.0.1 only.

use net::{Client, NetError, Policy, StreamLimits};
use rcgen::{CertificateParams, KeyPair};
use std::error::Error;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

type R = Result<(), Box<dyn Error>>;

/// One scripted reply: head text, then body pieces sent after the given delays (chunked).
struct Script {
    head: &'static str,
    pieces: Vec<(Duration, Vec<u8>)>,
    /// Close the chunked body properly at the end.
    finish: bool,
}

fn sse(pieces: Vec<(Duration, &str)>) -> Script {
    Script {
        head: "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
        pieces: pieces
            .into_iter()
            .map(|(d, s)| (d, s.as_bytes().to_vec()))
            .collect(),
        finish: true,
    }
}

type Heads = Arc<Mutex<Vec<String>>>;

async fn server(
    script: impl Fn() -> Script + Send + Sync + 'static,
) -> Result<(u16, Heads), Box<dyn Error>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let heads: Heads = Arc::default();
    let seen = Arc::clone(&heads);
    let script = Arc::new(script);
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let seen = Arc::clone(&seen);
            let script = Arc::clone(&script);
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0_u8; 4096];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(chunk.get(..n).unwrap_or_default()),
                    }
                }
                if let Ok(mut heads) = seen.lock() {
                    heads.push(String::from_utf8_lossy(&buf).into_owned());
                }
                let script = script();
                let _ = stream.write_all(script.head.as_bytes()).await;
                for (delay, data) in script.pieces {
                    tokio::time::sleep(delay).await;
                    let framed = [
                        format!("{:x}\r\n", data.len()).into_bytes(),
                        data,
                        b"\r\n".to_vec(),
                    ]
                    .concat();
                    if stream.write_all(&framed).await.is_err() {
                        return;
                    }
                }
                if script.finish {
                    let _ = stream.write_all(b"0\r\n\r\n").await;
                    let _ = stream.shutdown().await;
                } else {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                }
            });
        }
    });
    Ok((port, heads))
}

fn client() -> Result<Client, Box<dyn Error>> {
    let key = KeyPair::generate()?;
    let cert = CertificateParams::new(vec!["localhost".to_owned()])?.self_signed(&key)?;
    Ok(Client::with_root_certificates([cert.der().to_vec()])?)
}

fn local() -> Policy {
    Policy {
        block_private_addresses: false,
        allow_http: true,
        timeout: Duration::from_secs(5),
        ..Policy::default()
    }
}

fn url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/v1/chat/completions")
}

fn head_of(heads: &Heads) -> String {
    heads
        .lock()
        .map(|h| h.first().cloned().unwrap_or_default())
        .unwrap_or_default()
}

const MS: Duration = Duration::from_millis(1);

#[tokio::test]
async fn extra_headers_are_sent_and_defaults_stay() -> R {
    let (port, heads) = server(|| sse(vec![(MS, "ok")])).await?;
    let c = client()?;
    let headers = [("Authorization", "Bearer test-key-123"), ("X-Trace", "abc")];
    let r = c
        .post_json_with(&url(port), b"{}", &headers, &local())
        .await?;
    assert_eq!(r.status, 200);
    let head = head_of(&heads).to_ascii_lowercase();
    assert!(
        head.contains("authorization: bearer test-key-123"),
        "{head}"
    );
    assert!(head.contains("x-trace: abc"));
    assert!(head.contains("content-type: application/json"));
    assert!(head.contains("accept: */*"));
    assert!(head.contains("user-agent: monitor/1"));
    Ok(())
}

#[tokio::test]
async fn a_custom_accept_replaces_the_default_instead_of_adding_to_it() -> R {
    let (port, heads) = server(|| sse(vec![])).await?;
    let c = client()?;
    let _ = c
        .post_json_stream(
            &url(port),
            b"{}",
            &[("Accept", "text/event-stream")],
            &local(),
            StreamLimits::default(),
        )
        .await?;
    let head = head_of(&heads).to_ascii_lowercase();
    assert!(head.contains("accept: text/event-stream"), "{head}");
    assert!(!head.contains("*/*"), "{head}");
    Ok(())
}

#[tokio::test]
async fn framing_and_injection_headers_are_refused_before_anything_is_sent() -> R {
    let (port, heads) = server(|| sse(vec![])).await?;
    let c = client()?;
    for bad in [
        ("Host", "evil.test"),
        ("Content-Length", "0"),
        ("Transfer-Encoding", "chunked"),
        ("Connection", "keep-alive"),
        ("Upgrade", "websocket"),
        ("X-Ok", "a\r\nX-Injected: 1"),
        ("X-Ok", "a\nb"),
        ("bad name", "v"),
        ("", "v"),
    ] {
        let r = c.post_json_with(&url(port), b"{}", &[bad], &local()).await;
        assert!(matches!(r, Err(NetError::InvalidHeader)), "{bad:?}");
    }
    assert!(
        heads.lock().map(|h| h.is_empty()).unwrap_or(false),
        "a request was sent"
    );
    Ok(())
}

#[tokio::test]
async fn chunks_arrive_in_order_and_the_stream_ends_cleanly() -> R {
    let (port, _) = server(|| {
        sse(vec![
            (MS, "data: one\n\n"),
            (5 * MS, "data: two\n\n"),
            (MS, "data: [DONE]\n\n"),
        ])
    })
    .await?;
    let c = client()?;
    let mut s = c
        .post_json_stream(&url(port), b"{}", &[], &local(), StreamLimits::default())
        .await?;
    assert_eq!(s.status(), 200);
    let mut all = Vec::new();
    while let Some(chunk) = s.next_chunk().await? {
        all.extend_from_slice(&chunk);
    }
    assert_eq!(
        String::from_utf8(all)?,
        "data: one\n\ndata: two\n\ndata: [DONE]\n\n"
    );
    assert!(s.next_chunk().await?.is_none(), "stays ended");
    Ok(())
}

#[tokio::test]
async fn a_stalled_stream_hits_the_idle_limit() -> R {
    let (port, _) = server(|| Script {
        finish: false,
        ..sse(vec![(MS, "data: one\n\n")])
    })
    .await?;
    let c = client()?;
    let limits = StreamLimits {
        idle: Duration::from_millis(200),
        ..StreamLimits::default()
    };
    let mut s = c
        .post_json_stream(&url(port), b"{}", &[], &local(), limits)
        .await?;
    assert!(s.next_chunk().await?.is_some());
    assert_eq!(s.next_chunk().await.err(), Some(NetError::Timeout));
    Ok(())
}

#[tokio::test]
async fn a_slow_drip_cannot_outlast_the_total_limit() -> R {
    let (port, _) = server(|| Script {
        finish: false,
        ..sse((0..50)
            .map(|_| (Duration::from_millis(60), "data: x\n\n"))
            .collect())
    })
    .await?;
    let c = client()?;
    let limits = StreamLimits {
        idle: Duration::from_secs(5),
        total: Duration::from_millis(300),
        ..StreamLimits::default()
    };
    let mut s = c
        .post_json_stream(&url(port), b"{}", &[], &local(), limits)
        .await?;
    let mut got = 0;
    let end = loop {
        match s.next_chunk().await {
            Ok(Some(_)) => got += 1,
            other => break other,
        }
    };
    assert_eq!(end.err(), Some(NetError::Timeout));
    assert!((1..20).contains(&got), "{got}");
    Ok(())
}

#[tokio::test]
async fn too_many_bytes_stop_the_stream() -> R {
    let big = "x".repeat(4000);
    let (port, _) = server(move || {
        let mut s = sse(vec![]);
        s.pieces = (0..10).map(|_| (MS, big.as_bytes().to_vec())).collect();
        s
    })
    .await?;
    let c = client()?;
    let limits = StreamLimits {
        max_bytes: 10_000,
        ..StreamLimits::default()
    };
    let mut s = c
        .post_json_stream(&url(port), b"{}", &[], &local(), limits)
        .await?;
    let end = loop {
        match s.next_chunk().await {
            Ok(Some(_)) => {}
            other => break other,
        }
    };
    assert_eq!(end.err(), Some(NetError::ResponseTooLarge));
    Ok(())
}

#[tokio::test]
async fn an_error_status_is_a_stream_whose_body_can_be_read_capped() -> R {
    let (port, _) = server(|| Script {
        head: "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 7\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
        pieces: vec![(MS, b"{\"error\":\"slow down\"}".to_vec())],
        finish: true,
    })
    .await?;
    let c = client()?;
    let s = c
        .post_json_stream(&url(port), b"{}", &[], &local(), StreamLimits::default())
        .await?;
    assert_eq!(s.status(), 429);
    let body = s.read_capped(8).await;
    assert_eq!(body, b"{\"error\"");
    Ok(())
}

#[tokio::test]
async fn a_post_never_follows_a_redirect_so_credentials_stay_put() -> R {
    let (port, heads) = server(|| Script {
        head: "HTTP/1.1 307 Temporary Redirect\r\nLocation: http://127.0.0.1:1/steal\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        pieces: vec![],
        finish: false,
    })
    .await?;
    let c = client()?;
    let r = c
        .post_json_with(
            &url(port),
            b"{}",
            &[("Authorization", "Bearer k")],
            &local(),
        )
        .await?;
    assert_eq!(r.status, 307);
    assert_eq!(heads.lock().map(|h| h.len()).unwrap_or(0), 1);
    Ok(())
}

#[tokio::test]
async fn loopback_and_plain_http_are_refused_by_the_default_policy() -> R {
    let (port, heads) = server(|| sse(vec![])).await?;
    let c = client()?;
    let strict = Policy {
        allow_http: true,
        ..Policy::default()
    };
    let r = c
        .post_json_stream(&url(port), b"{}", &[], &strict, StreamLimits::default())
        .await;
    assert_eq!(r.err(), Some(NetError::BlockedAddress));
    let https_only = Policy {
        block_private_addresses: false,
        ..Policy::default()
    };
    let r = c
        .post_json_stream(&url(port), b"{}", &[], &https_only, StreamLimits::default())
        .await;
    assert_eq!(r.err(), Some(NetError::SchemeNotAllowed));
    assert!(heads.lock().map(|h| h.is_empty()).unwrap_or(false));
    Ok(())
}
