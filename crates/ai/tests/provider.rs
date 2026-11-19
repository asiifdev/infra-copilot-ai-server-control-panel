//! The provider against a scripted transport: retries, the breaker, secrets and protocol quirks.

use ai::testing::{FakeTransport, Reply};
use ai::{
    AiError, AiProvider, ChatRequest, FinishReason, Message, NoSink, OpenAiCompatible,
    ProviderConfig, TransportError,
};
use core_domain::secret::Secret;
use std::time::Duration;
use tokio::time::Instant;

const KEY: &str = "sk-TESTKEY-0123456789abcdef";

fn config(params: &str) -> ProviderConfig {
    match ProviderConfig::from_params("https://api.example.test/v1", "test-model", params) {
        Ok(c) => c,
        Err(e) => unreachable!("test config: {e}"),
    }
}

fn provider(params: &str, replies: Vec<Reply>) -> OpenAiCompatible<FakeTransport> {
    OpenAiCompatible::new(
        config(params),
        Some(Secret::new(KEY)),
        FakeTransport::new(replies),
    )
}

fn request() -> ChatRequest {
    ChatRequest {
        messages: vec![Message::user("hello")],
        ..ChatRequest::default()
    }
}

const ANSWER: &str = r#"{"model":"m","choices":[{"message":{"content":"Hi there"},"finish_reason":"stop"}],"usage":{"prompt_tokens":4,"completion_tokens":2}}"#;
const SSE_TEXT: [&str; 4] = [
    "data: {\"choices\":[{\"delta\":{\"content\":\"Hi \"}}]}\n\n",
    "data: {\"choices\":[{\"delta\":{\"content\":\"there\"}}]}\n\n",
    "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
    "data: [DONE]\n\n",
];

#[tokio::test]
async fn a_plain_answer_is_parsed_and_the_key_goes_only_in_the_header() {
    let p = provider(
        r#"{"supports_stream":false}"#,
        vec![Reply::ok(vec![ANSWER])],
    );
    let r = p.chat(&request(), &mut NoSink).await;
    let r = r.unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(
        (r.content.as_str(), r.finish),
        ("Hi there", FinishReason::Stop)
    );
    let seen = p.transport().seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].url, "https://api.example.test/v1/chat/completions");
    assert!(!seen[0].body.contains(KEY), "key leaked into the body");
    assert!(!seen[0].body.contains("\"stream\""));
    let auth = seen[0].headers.iter().find(|(n, _)| n == "authorization");
    assert_eq!(
        auth.map(|(_, v)| v.as_str()),
        Some("Bearer sk-TESTKEY-0123456789abcdef")
    );
    assert!(
        seen[0]
            .headers
            .contains(&("accept".to_owned(), "application/json".to_owned()))
    );
}

#[tokio::test]
async fn a_keyless_provider_sends_no_authorization_header() {
    let p = OpenAiCompatible::new(
        config(r#"{"supports_stream":false}"#),
        None,
        FakeTransport::new(vec![Reply::ok(vec![ANSWER])]),
    );
    assert!(p.chat(&request(), &mut NoSink).await.is_ok());
    assert!(
        p.transport().seen()[0]
            .headers
            .iter()
            .all(|(n, _)| n != "authorization")
    );
}

#[tokio::test]
async fn a_streamed_answer_reaches_the_sink_piece_by_piece() {
    let p = provider("{}", vec![Reply::ok(SSE_TEXT.to_vec())]);
    let mut pieces: Vec<String> = Vec::new();
    let mut sink = |t: &str| pieces.push(t.to_owned());
    let r = p
        .chat(&request(), &mut sink)
        .await
        .unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(r.content, "Hi there");
    assert_eq!(pieces, ["Hi ", "there"]);
    let seen = p.transport().seen();
    assert!(seen[0].body.contains("\"stream\":true"));
    assert!(
        seen[0]
            .headers
            .contains(&("accept".to_owned(), "text/event-stream".to_owned()))
    );
}

#[tokio::test]
async fn a_server_that_ignores_stream_true_and_answers_json_still_works() {
    let p = provider("{}", vec![Reply::ok(vec![ANSWER])]);
    let r = p.chat(&request(), &mut NoSink).await;
    assert_eq!(r.map(|r| r.content), Ok("Hi there".to_owned()));
}

#[tokio::test]
async fn tools_are_refused_up_front_when_the_provider_cannot_call_them() {
    let p = provider(r#"{"supports_tools":false}"#, vec![Reply::ok(vec![ANSWER])]);
    let req = ChatRequest {
        tools: vec![serde_json::json!({"type":"function"})],
        ..request()
    };
    assert_eq!(
        p.chat(&req, &mut NoSink).await.err(),
        Some(AiError::Unsupported("tools"))
    );
    assert!(p.transport().seen().is_empty(), "nothing was sent");
}

#[tokio::test]
async fn errors_never_contain_the_key_or_server_text() {
    let body = format!("Incorrect API key provided: {KEY}. See https://secret.example.test/keys");
    for status in [401, 403, 400, 404, 500, 429] {
        let p = provider(
            r#"{"max_retries":0,"supports_stream":false}"#,
            vec![Reply::status(status, &body)],
        );
        let e = p
            .chat(&request(), &mut NoSink)
            .await
            .err()
            .unwrap_or(AiError::Empty);
        for text in [format!("{e}"), format!("{e:?}")] {
            assert!(
                !text.contains(KEY)
                    && !text.contains("secret.example")
                    && !text.contains("Incorrect"),
                "{status}: {text}"
            );
        }
    }
    let debug = format!("{:?}", provider("{}", vec![]).config());
    assert!(!debug.contains(KEY));
}

#[tokio::test(start_paused = true)]
async fn statuses_map_to_the_right_errors_and_only_transient_ones_retry() {
    for (status, body, expected, retried) in [
        (401, "{}", AiError::Auth, false),
        (403, "{}", AiError::Auth, false),
        (402, "{}", AiError::Quota, false),
        (400, "{}", AiError::Rejected(400), false),
        (404, "{}", AiError::Rejected(404), false),
        (
            429,
            r#"{"error":{"code":"insufficient_quota"}}"#,
            AiError::Quota,
            false,
        ),
        (
            429,
            r#"{"error":{"message":"You exceeded your current quota"}}"#,
            AiError::Quota,
            false,
        ),
        (
            429,
            "{}",
            AiError::RateLimited {
                retry_after_secs: None,
            },
            true,
        ),
        (500, "{}", AiError::Server(500), true),
        (503, "{}", AiError::Server(503), true),
    ] {
        let replies = vec![Reply::status(status, body); 3];
        let p = provider(r#"{"supports_stream":false}"#, replies);
        let e = p.chat(&request(), &mut NoSink).await.err();
        assert_eq!(e, Some(expected), "{status} {body}");
        assert_eq!(
            p.transport().seen().len(),
            if retried { 3 } else { 1 },
            "{status} {body}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn a_transient_failure_is_retried_and_the_retry_after_hint_is_honoured() {
    let mut limited = Reply::status(429, "{}");
    if let Reply::Http { retry_after, .. } = &mut limited {
        *retry_after = Some(7);
    }
    let p = provider(
        r#"{"supports_stream":false}"#,
        vec![limited, Reply::status(503, "{}"), Reply::ok(vec![ANSWER])],
    );
    let started = Instant::now();
    let r = p.chat(&request(), &mut NoSink).await;
    assert_eq!(r.map(|r| r.content), Ok("Hi there".to_owned()));
    assert_eq!(p.transport().seen().len(), 3);
    assert!(
        started.elapsed() >= Duration::from_secs(7),
        "{:?}",
        started.elapsed()
    );
    assert!(started.elapsed() < Duration::from_secs(30));
}

#[tokio::test(start_paused = true)]
async fn network_failures_retry_but_a_blocked_destination_does_not() {
    let p = provider(
        r#"{"supports_stream":false}"#,
        vec![
            Reply::Fail(TransportError::Connect),
            Reply::Fail(TransportError::Timeout),
            Reply::ok(vec![ANSWER]),
        ],
    );
    assert!(p.chat(&request(), &mut NoSink).await.is_ok());
    assert_eq!(p.transport().seen().len(), 3);
    let p = provider(
        r#"{"supports_stream":false}"#,
        vec![
            Reply::Fail(TransportError::Blocked),
            Reply::ok(vec![ANSWER]),
        ],
    );
    assert_eq!(
        p.chat(&request(), &mut NoSink).await.err(),
        Some(AiError::Transport(TransportError::Blocked))
    );
    assert_eq!(p.transport().seen().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn once_text_reached_the_caller_a_failure_is_not_retried() {
    // Half an answer streamed, then the connection dies: retrying would repeat visible text.
    let cut = Reply::Http {
        status: 200,
        retry_after: None,
        chunks: vec![SSE_TEXT[0].as_bytes().to_vec()],
        then_fail: Some(TransportError::Timeout),
    };
    let p = provider("{}", vec![cut, Reply::ok(SSE_TEXT.to_vec())]);
    let mut pieces: Vec<String> = Vec::new();
    let mut sink = |t: &str| pieces.push(t.to_owned());
    let e = p.chat(&request(), &mut sink).await.err();
    assert_eq!(e, Some(AiError::Transport(TransportError::Timeout)));
    assert_eq!(p.transport().seen().len(), 1);
    assert_eq!(pieces, ["Hi "]);
}

#[tokio::test(start_paused = true)]
async fn a_failure_before_any_text_is_retried_even_mid_stream() {
    let cut = Reply::Http {
        status: 200,
        retry_after: None,
        chunks: vec![b": hello\n\n".to_vec()],
        then_fail: Some(TransportError::Protocol),
    };
    let p = provider("{}", vec![cut, Reply::ok(SSE_TEXT.to_vec())]);
    assert_eq!(
        p.chat(&request(), &mut NoSink).await.map(|r| r.content),
        Ok("Hi there".to_owned())
    );
    assert_eq!(p.transport().seen().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn the_breaker_opens_after_repeated_failures_and_half_opens_after_the_cooldown() {
    let params = r#"{"supports_stream":false,"max_retries":0}"#;
    let mut cfg = config(params);
    cfg.breaker_failures = 2;
    cfg.breaker_cooldown = Duration::from_secs(60);
    let transport = FakeTransport::new(vec![
        Reply::status(503, "{}"),
        Reply::status(503, "{}"),
        Reply::status(503, "{}"),
        Reply::ok(vec![ANSWER]),
    ]);
    let p = OpenAiCompatible::new(cfg, Some(Secret::new(KEY)), transport);
    assert_eq!(
        p.chat(&request(), &mut NoSink).await.err(),
        Some(AiError::Server(503))
    );
    assert_eq!(
        p.chat(&request(), &mut NoSink).await.err(),
        Some(AiError::Server(503))
    );
    // Open: refused without touching the network.
    assert_eq!(
        p.chat(&request(), &mut NoSink).await.err(),
        Some(AiError::CircuitOpen)
    );
    assert_eq!(p.transport().seen().len(), 2);
    tokio::time::advance(Duration::from_secs(61)).await;
    // Half open: one probe goes out; it fails, so the breaker reopens at once.
    assert_eq!(
        p.chat(&request(), &mut NoSink).await.err(),
        Some(AiError::Server(503))
    );
    assert_eq!(
        p.chat(&request(), &mut NoSink).await.err(),
        Some(AiError::CircuitOpen)
    );
    tokio::time::advance(Duration::from_secs(61)).await;
    // The next probe succeeds and closes it.
    assert!(p.chat(&request(), &mut NoSink).await.is_ok());
    assert_eq!(p.transport().seen().len(), 4);
}

#[tokio::test(start_paused = true)]
async fn refusals_of_this_request_do_not_trip_the_breaker_but_successes_reset_the_count() {
    let mut cfg = config(r#"{"supports_stream":false,"max_retries":0}"#);
    cfg.breaker_failures = 2;
    let replies = vec![
        Reply::status(401, "{}"),
        Reply::status(401, "{}"),
        Reply::status(401, "{}"),
        Reply::status(503, "{}"),
        Reply::ok(vec![ANSWER]),
        Reply::status(503, "{}"),
        Reply::ok(vec![ANSWER]),
    ];
    let p = OpenAiCompatible::new(cfg, None, FakeTransport::new(replies));
    for _ in 0..3 {
        assert_eq!(
            p.chat(&request(), &mut NoSink).await.err(),
            Some(AiError::Auth)
        );
    }
    assert_eq!(
        p.chat(&request(), &mut NoSink).await.err(),
        Some(AiError::Server(503))
    );
    assert!(
        p.chat(&request(), &mut NoSink).await.is_ok(),
        "a success resets the count"
    );
    assert_eq!(
        p.chat(&request(), &mut NoSink).await.err(),
        Some(AiError::Server(503))
    );
    assert!(
        p.chat(&request(), &mut NoSink).await.is_ok(),
        "one failure after a reset does not open it"
    );
}

#[tokio::test]
async fn malformed_streams_fail_cleanly() {
    let huge = format!("data: {}\n\n", "x".repeat(1_100_000));
    for (chunks, expected) in [
        (
            vec!["data: {not json\n\n"],
            AiError::Malformed("stream chunk is not JSON"),
        ),
        (
            vec!["data: {\"choices\":[{\"delta\":{\"content\":\"cut\"}}]}\n\n"],
            AiError::Malformed("stream ended early"),
        ),
        (
            vec![
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n",
            ],
            AiError::Empty,
        ),
        (
            vec![huge.as_str()],
            AiError::Malformed("stream event is too large"),
        ),
    ] {
        let p = provider(r#"{"max_retries":0}"#, vec![Reply::ok(chunks)]);
        assert_eq!(p.chat(&request(), &mut NoSink).await.err(), Some(expected));
    }
}
