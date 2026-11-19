//! An OpenAI-compatible chat provider (ADR 0020, M2-c).
//!
//! One wire format (`POST {base}/chat/completions`, function calling, optional streaming) that
//! Muse, OpenAI and local servers such as Ollama all speak. Nothing here is vendor specific
//! except optional parameters that are only sent when configured.
//!
//! Rules enforced here:
//! * the API key is a [`core_domain::secret::Secret`], injected into the `Authorization` header at
//!   send time; it is never in an error, `Debug`, log line or request body;
//! * errors ([`AiError`]) have fixed text and never carry server text, the URL or the key;
//! * everything a server returns is bounded (bytes, tool calls, argument size) and untrusted;
//! * only fields the provider is configured to support are sent, and none is ever `null`;
//! * a request is retried only for transient failures and only before any text reached the
//!   caller, with capped exponential backoff that honours `Retry-After`; repeated failures open a
//!   circuit breaker so a dead provider is not hammered.

mod config;
mod error;
mod provider;
mod sse;
pub mod testing;
mod think;
mod transport;
mod types;
mod wire;

pub use config::{ProviderConfig, ReasoningEffort};
pub use error::AiError;
pub use provider::{AiProvider, DeltaSink, NoSink, OpenAiCompatible};
pub use sse::{SseDecoder, SseError, SseEvent};
pub use think::ThinkFilter;
pub use transport::{ByteStream, NetTransport, Transport, TransportError};
pub use types::{ChatRequest, ChatResponse, FinishReason, Message, Role, ToolCall, Usage};
pub use wire::StreamAssembler;
