//! The real stack (provider, `NetTransport`, hardened `net` client) against a local server that
//! speaks chunked Server-Sent Events, plus an opt-in live smoke test.

use ai::{
    AiError, AiProvider, ChatRequest, FinishReason, Message, NetTransport, NoSink,
    OpenAiCompatible, ProviderConfig,
};
use core_domain::secret::Secret;
use net::Client;
use rcgen::{CertificateParams, KeyPair};
use serde_json::json;
use std::error::Error;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

type R = Result<(), Box<dyn Error>>;

const KEY: &str = "sk-E2E-0123456789abcdef";

/// Replies to every request with `head` and the chunked `pieces`; returns the port and the
/// requests (head + body) it received.
async fn server(
    head: &'static str,
    pieces: Vec<String>,
) -> Result<(u16, Arc<Mutex<Vec<String>>>), Box<dyn Error>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let log = Arc::clone(&seen);
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let (log, pieces) = (Arc::clone(&log), pieces.clone());
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0_u8; 4096];
                let (head_end, wanted) = loop {
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let text = String::from_utf8_lossy(&buf[..pos]).to_ascii_lowercase();
                        let n = text.lines().find_map(|l| {
                            l.strip_prefix("content-length:")
                                .and_then(|v| v.trim().parse::<usize>().ok())
                        });
                        break (pos + 4, n.unwrap_or(0));
                    }
                    match stream.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                };
                while buf.len() < head_end + wanted {
                    match stream.read(&mut chunk).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                }
                if let Ok(mut l) = log.lock() {
                    l.push(String::from_utf8_lossy(&buf).into_owned());
                }
                let _ = stream.write_all(head.as_bytes()).await;
                for piece in pieces {
                    let framed = format!("{:x}\r\n{piece}\r\n", piece.len());
                    if stream.write_all(framed.as_bytes()).await.is_err() {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
                let _ = stream.write_all(b"0\r\n\r\n").await;
                let _ = stream.shutdown().await;
            });
        }
    });
    Ok((port, seen))
}

const SSE_HEAD: &str = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";

fn provider(port: u16, params: &str) -> Result<OpenAiCompatible<NetTransport>, Box<dyn Error>> {
    let config =
        ProviderConfig::from_params(&format!("http://127.0.0.1:{port}/v1"), "test-model", params)?;
    let key = KeyPair::generate()?;
    let cert = CertificateParams::new(vec!["localhost".to_owned()])?.self_signed(&key)?;
    let client = Client::with_root_certificates([cert.der().to_vec()])?;
    let transport = NetTransport::new(client, config.net_policy(), config.stream_limits());
    Ok(OpenAiCompatible::new(
        config,
        Some(Secret::new(KEY)),
        transport,
    ))
}

fn tool_request() -> ChatRequest {
    ChatRequest {
        messages: vec![
            Message::system("You are an ops assistant."),
            Message::user("Which servers do I have?"),
        ],
        tools: vec![
            json!({"type":"function","function":{"name":"list_nodes","description":"List nodes.","parameters":{"type":"object","properties":{},"additionalProperties":false}}}),
        ],
        ..ChatRequest::default()
    }
}

#[tokio::test]
async fn a_streamed_tool_call_round_trips_over_real_sockets() -> R {
    let pieces = [
        r#"{"model":"m","choices":[{"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"list_nodes","arguments":""}}]}}]}"#,
        r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{}"}}]}}]}"#,
        r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
    ];
    let mut events: Vec<String> = pieces.iter().map(|p| format!("data: {p}\n\n")).collect();
    events.push("data: [DONE]\n\n".to_owned());
    let (port, seen) = server(SSE_HEAD, events).await?;
    let p = provider(port, r#"{"allow_private_network":true}"#)?;
    let r = p.chat(&tool_request(), &mut NoSink).await?;
    assert_eq!(r.finish, FinishReason::ToolCalls);
    assert_eq!(r.tool_calls.len(), 1);
    assert_eq!(
        (
            r.tool_calls[0].id.as_str(),
            r.tool_calls[0].name.as_str(),
            r.tool_calls[0].arguments.as_str()
        ),
        ("call_1", "list_nodes", "{}")
    );
    let request = seen
        .lock()
        .map(|s| s.first().cloned().unwrap_or_default())
        .unwrap_or_default();
    let lower = request.to_ascii_lowercase();
    assert!(lower.starts_with("post /v1/chat/completions "), "{request}");
    assert!(lower.contains(&format!(
        "authorization: bearer {}",
        KEY.to_ascii_lowercase()
    )));
    assert!(lower.contains("accept: text/event-stream"));
    assert!(request.contains("\"tools\"") && request.contains("\"stream\":true"));
    assert!(
        !request
            .split("\r\n\r\n")
            .nth(1)
            .unwrap_or_default()
            .contains(KEY),
        "key in the body"
    );
    Ok(())
}

#[tokio::test]
async fn a_text_answer_streams_to_the_sink() -> R {
    let events: Vec<String> = [
        r#"{"choices":[{"delta":{"content":"You have "}}]}"#,
        r#"{"choices":[{"delta":{"content":"two servers."},"finish_reason":"stop"}]}"#,
    ]
    .iter()
    .map(|p| format!("data: {p}\n\n"))
    .chain(["data: [DONE]\n\n".to_owned()])
    .collect();
    let (port, _) = server(SSE_HEAD, events).await?;
    let p = provider(port, r#"{"allow_private_network":true}"#)?;
    let mut pieces: Vec<String> = Vec::new();
    let mut sink = |t: &str| pieces.push(t.to_owned());
    let r = p.chat(&tool_request(), &mut sink).await?;
    assert_eq!(r.content, "You have two servers.");
    assert_eq!(pieces, ["You have ", "two servers."]);
    Ok(())
}

#[tokio::test]
async fn a_loopback_provider_is_refused_unless_the_config_allows_it() -> R {
    let (port, seen) = server(SSE_HEAD, vec![]).await?;
    let config = ProviderConfig::new("https://127.0.0.1/v1", "m")?;
    let key = KeyPair::generate()?;
    let cert = CertificateParams::new(vec!["localhost".to_owned()])?.self_signed(&key)?;
    let client = Client::with_root_certificates([cert.der().to_vec()])?;
    let strict = OpenAiCompatible::new(
        config.clone(),
        None,
        NetTransport::new(client, config.net_policy(), config.stream_limits()),
    );
    let e = strict.chat(&tool_request(), &mut NoSink).await.err();
    assert!(
        matches!(e, Some(AiError::Transport(ai::TransportError::Blocked))),
        "{e:?}"
    );
    assert!(
        ProviderConfig::new(&format!("http://127.0.0.1:{port}/v1"), "m").is_err(),
        "plain http needs the switch"
    );
    assert!(seen.lock().map(|s| s.is_empty()).unwrap_or(false));
    Ok(())
}

#[tokio::test]
async fn a_rate_limit_from_a_real_server_is_classified() -> R {
    let head = "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 1\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
    let (port, _) = server(
        head,
        vec![r#"{"error":{"message":"slow down"}}"#.to_owned()],
    )
    .await?;
    let p = provider(port, r#"{"allow_private_network":true,"max_retries":0}"#)?;
    let e = p.chat(&tool_request(), &mut NoSink).await.err();
    assert_eq!(
        e,
        Some(AiError::RateLimited {
            retry_after_secs: Some(1)
        })
    );
    Ok(())
}

/// Opt-in: talks to a real provider. Run with
/// `AI_BASE_URL=... AI_API_KEY=... AI_MODEL=... cargo test -p ai --test end_to_end -- --ignored`.
/// The key is read from the environment only and is never printed.
#[tokio::test]
#[ignore = "needs a real provider: AI_BASE_URL, AI_API_KEY, AI_MODEL"]
async fn live_smoke_test() -> R {
    let (Ok(base), Ok(key), Ok(model)) = (
        std::env::var("AI_BASE_URL"),
        std::env::var("AI_API_KEY"),
        std::env::var("AI_MODEL"),
    ) else {
        return Err("set AI_BASE_URL, AI_API_KEY and AI_MODEL".into());
    };
    let params = std::env::var("AI_PARAMS").unwrap_or_else(|_| "{}".to_owned());
    let config = ProviderConfig::from_params(&base, &model, &params)?;
    let transport = NetTransport::new(Client::new()?, config.net_policy(), config.stream_limits());
    let p = OpenAiCompatible::new(config, Some(Secret::new(key)), transport);
    let req = ChatRequest {
        messages: vec![Message::user("Reply with the single word: pong")],
        max_tokens: Some(64),
        ..ChatRequest::default()
    };
    let r = p.chat(&req, &mut NoSink).await?;
    assert!(!r.content.trim().is_empty(), "empty answer");
    let tools = p.chat(&tool_request(), &mut NoSink).await?;
    assert!(!tools.tool_calls.is_empty() || !tools.content.is_empty());
    Ok(())
}
