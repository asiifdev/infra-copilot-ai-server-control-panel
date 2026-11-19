//! The provider: one request, retried when that is safe, behind a circuit breaker.

use crate::config::ProviderConfig;
use crate::error::AiError;
use crate::sse::SseDecoder;
use crate::transport::{ByteStream, Transport};
use crate::types::{ChatRequest, ChatResponse};
use crate::wire::{StreamAssembler, parse_completion, request_body};
use core_domain::secret::Secret;
use std::future::Future;
use std::sync::Mutex;
use std::time::Duration;
use tokio::time::Instant;

/// Receives visible text while an answer streams.
pub trait DeltaSink: Send {
    /// A piece of visible text (reasoning blocks already removed).
    fn text(&mut self, piece: &str);
}

/// Discards deltas.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoSink;

impl DeltaSink for NoSink {
    fn text(&mut self, _piece: &str) {}
}

impl<F: FnMut(&str) + Send> DeltaSink for F {
    fn text(&mut self, piece: &str) {
        self(piece);
    }
}

/// A chat model. The agent loop depends on this trait, not on any vendor.
pub trait AiProvider: Send + Sync {
    /// Runs one assistant turn. Text is passed to `sink` as it arrives when the provider streams.
    ///
    /// # Errors
    /// [`AiError`], with fixed text and no server data.
    fn chat(
        &self,
        request: &ChatRequest,
        sink: &mut dyn DeltaSink,
    ) -> impl Future<Output = Result<ChatResponse, AiError>> + Send;
}

/// Largest error body read for classification.
const ERROR_BODY_BYTES: usize = 4096;
/// Largest non-streaming answer read.
const MAX_ANSWER_BYTES: usize = 4 * 1024 * 1024;
/// Largest line or event accepted from a stream.
const MAX_EVENT_BYTES: usize = 1024 * 1024;
/// Longest wait between retries.
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// Wraps a sink to count how much text reached the caller: after the first delta a retry would
/// repeat visible text, so it is no longer allowed.
struct Counting<'a> {
    inner: &'a mut dyn DeltaSink,
    delivered: usize,
}

impl DeltaSink for Counting<'_> {
    fn text(&mut self, piece: &str) {
        self.delivered += piece.len();
        self.inner.text(piece);
    }
}

#[derive(Debug, Default)]
struct Breaker {
    failures: u32,
    open_until: Option<Instant>,
}

/// An OpenAI-compatible provider over a [`Transport`].
pub struct OpenAiCompatible<T> {
    config: ProviderConfig,
    key: Option<Secret>,
    transport: T,
    breaker: Mutex<Breaker>,
}

impl<T: Transport> OpenAiCompatible<T> {
    /// A provider. `key` is `None` for a keyless local model.
    #[must_use]
    pub fn new(config: ProviderConfig, key: Option<Secret>, transport: T) -> Self {
        Self {
            config,
            key,
            transport,
            breaker: Mutex::new(Breaker::default()),
        }
    }

    /// The transport, for tests that inspect what was sent.
    #[must_use]
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// The configuration in use.
    #[must_use]
    pub fn config(&self) -> &ProviderConfig {
        &self.config
    }

    fn admit(&self) -> Result<(), AiError> {
        let mut b = self
            .breaker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match b.open_until {
            Some(until) if Instant::now() < until => Err(AiError::CircuitOpen),
            Some(_) => {
                // Half open: let one request through; one more failure reopens at once.
                b.open_until = None;
                b.failures = self.config.breaker_failures.saturating_sub(1);
                Ok(())
            }
            None => Ok(()),
        }
    }

    fn record(&self, result: &Result<ChatResponse, AiError>) {
        let mut b = self
            .breaker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match result {
            Ok(_) => b.failures = 0,
            Err(e) if e.counts_against_breaker() => {
                b.failures = b.failures.saturating_add(1);
                if b.failures >= self.config.breaker_failures {
                    b.open_until = Some(Instant::now() + self.config.breaker_cooldown);
                }
            }
            // A refusal of this request (bad key, bad request, no credit) says nothing about the
            // provider's health.
            Err(_) => {}
        }
    }

    async fn once(
        &self,
        url: &str,
        body: &[u8],
        stream: bool,
        sink: &mut dyn DeltaSink,
    ) -> Result<ChatResponse, AiError> {
        let authorization = self
            .key
            .as_ref()
            .map(|k| format!("Bearer {}", k.expose_secret()));
        let accept = if stream {
            "text/event-stream"
        } else {
            "application/json"
        };
        let mut headers: Vec<(&str, &str)> = vec![("accept", accept)];
        if let Some(value) = &authorization {
            headers.push(("authorization", value));
        }
        let mut response = self
            .transport
            .post(url, body, &headers)
            .await
            .map_err(AiError::Transport)?;
        let status = response.status();
        if !(200..300).contains(&status) {
            let retry_after = response.retry_after_secs();
            let text = read_capped(&mut response, ERROR_BODY_BYTES).await;
            return Err(classify(status, retry_after, &text));
        }
        if stream {
            read_stream(&mut response, sink).await
        } else {
            read_whole(&mut response).await
        }
    }
}

impl<T: Transport> AiProvider for OpenAiCompatible<T> {
    async fn chat(
        &self,
        request: &ChatRequest,
        sink: &mut dyn DeltaSink,
    ) -> Result<ChatResponse, AiError> {
        self.admit()?;
        let stream = self.config.supports_stream;
        let body = request_body(&self.config, request, stream)?;
        let url = self.config.endpoint();
        let mut counting = Counting {
            inner: sink,
            delivered: 0,
        };
        let mut attempt: u8 = 0;
        let result = loop {
            let result = self.once(&url, &body, stream, &mut counting).await;
            match result {
                Err(e)
                    if e.is_retryable()
                        && counting.delivered == 0
                        && attempt < self.config.max_retries =>
                {
                    tokio::time::sleep(backoff(attempt, e.retry_after_secs())).await;
                    attempt += 1;
                }
                other => break other,
            }
        };
        self.record(&result);
        result
    }
}

/// `500 ms * 2^attempt` (at most 8 s) plus a little jitter, or the server's `Retry-After` when
/// that is longer; never more than [`MAX_BACKOFF`].
fn backoff(attempt: u8, retry_after_secs: Option<u32>) -> Duration {
    let base = Duration::from_millis(500) * 2_u32.pow(u32::from(attempt.min(4)));
    let jitter = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::from(d.subsec_nanos()) % 250);
    let own = base.min(Duration::from_secs(8)) + Duration::from_millis(jitter);
    retry_after_secs
        .map_or(own, |s| own.max(Duration::from_secs(u64::from(s))))
        .min(MAX_BACKOFF)
}

fn classify(status: u16, retry_after_secs: Option<u32>, body: &[u8]) -> AiError {
    match status {
        401 | 403 => AiError::Auth,
        402 => AiError::Quota,
        429 => {
            let text = String::from_utf8_lossy(body).to_ascii_lowercase();
            let out_of_credit = [
                "insufficient_quota",
                "credit_balance",
                "exceeded your current quota",
                "billing",
            ]
            .iter()
            .any(|m| text.contains(m));
            if out_of_credit {
                AiError::Quota
            } else {
                AiError::RateLimited { retry_after_secs }
            }
        }
        408 | 500..=599 => AiError::Server(status),
        _ => AiError::Rejected(status),
    }
}

async fn read_capped<S: ByteStream>(response: &mut S, max: usize) -> Vec<u8> {
    let mut out = Vec::new();
    while out.len() < max {
        match response.next_chunk().await {
            Ok(Some(chunk)) => {
                let take = chunk.len().min(max - out.len());
                out.extend_from_slice(chunk.get(..take).unwrap_or_default());
            }
            _ => break,
        }
    }
    out
}

async fn read_whole<S: ByteStream>(response: &mut S) -> Result<ChatResponse, AiError> {
    let mut all = Vec::new();
    while let Some(chunk) = response.next_chunk().await.map_err(AiError::Transport)? {
        if all.len() + chunk.len() > MAX_ANSWER_BYTES {
            return Err(AiError::Malformed("answer is too large"));
        }
        all.extend_from_slice(&chunk);
    }
    parse_completion(&all)
}

async fn read_stream<S: ByteStream>(
    response: &mut S,
    sink: &mut dyn DeltaSink,
) -> Result<ChatResponse, AiError> {
    let mut decoder = SseDecoder::new(MAX_EVENT_BYTES);
    let mut assembler = StreamAssembler::new();
    // Some servers ignore `stream: true` and answer with plain JSON: keep the raw bytes until the
    // first event proves it is a real stream.
    let mut raw: Vec<u8> = Vec::new();
    let mut saw_event = false;
    while let Some(chunk) = response.next_chunk().await.map_err(AiError::Transport)? {
        if !saw_event {
            if raw.len() + chunk.len() > MAX_ANSWER_BYTES {
                return Err(AiError::Malformed("answer is too large"));
            }
            raw.extend_from_slice(&chunk);
        }
        let events = decoder
            .push(&chunk)
            .map_err(|_| AiError::Malformed("stream event is too large"))?;
        for event in events {
            saw_event = true;
            raw = Vec::new();
            assembler.feed(&event.data, sink)?;
        }
        if assembler.is_done() {
            break;
        }
    }
    if let Some(event) = decoder.finish() {
        saw_event = true;
        assembler.feed(&event.data, sink)?;
    }
    if !saw_event && raw.trim_ascii_start().first() == Some(&b'{') {
        return parse_completion(&raw);
    }
    assembler.finish(sink)
}
