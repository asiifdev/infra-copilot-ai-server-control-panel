//! Provider-neutral chat types.

use serde_json::Value;

/// Who wrote a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Standing instructions.
    System,
    /// The person.
    User,
    /// The model.
    Assistant,
    /// The result of a tool call.
    Tool,
}

impl Role {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        }
    }
}

/// A tool call chosen by the model. `arguments` stays the raw string the model produced until the
/// executor validates it: models send broken JSON.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCall {
    /// Provider-assigned id; the tool result must answer it.
    pub id: String,
    /// Tool name as the model wrote it (untrusted until the registry resolves it).
    pub name: String,
    /// Raw argument text.
    pub arguments: String,
}

/// One message of a conversation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    /// Author.
    pub role: Role,
    /// Text. Empty for an assistant message that only calls tools.
    pub content: String,
    /// Tool calls made by an assistant message.
    pub tool_calls: Vec<ToolCall>,
    /// For a tool message: the call it answers.
    pub tool_call_id: Option<String>,
}

impl Message {
    /// A system message.
    #[must_use]
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    /// A user message.
    #[must_use]
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    /// An assistant text reply.
    #[must_use]
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    /// An assistant message that calls tools (as returned by the model, replayed verbatim).
    #[must_use]
    pub fn assistant_calls(content: impl Into<String>, calls: Vec<ToolCall>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_calls: calls,
            tool_call_id: None,
        }
    }

    /// The result of a tool call.
    #[must_use]
    pub fn tool_result(call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: Some(call_id.into()),
        }
    }
}

/// A request for one assistant turn.
#[derive(Clone, Debug, Default)]
pub struct ChatRequest {
    /// The conversation so far.
    pub messages: Vec<Message>,
    /// Tools offered, each already in the `{"type":"function","function":{...}}` shape.
    pub tools: Vec<Value>,
    /// Cap on generated tokens; the provider default applies when `None`.
    pub max_tokens: Option<u32>,
    /// Sampling temperature; omitted when `None`.
    pub temperature: Option<f32>,
}

/// Why the model stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FinishReason {
    /// A natural end.
    Stop,
    /// It wants tools run.
    ToolCalls,
    /// It ran out of tokens; the answer may be cut.
    Length,
    /// The provider filtered the content.
    ContentFilter,
    /// Anything else (capped, sanitised).
    Other(String),
}

/// Token counts, when the provider reports them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    /// Tokens in the prompt.
    pub prompt: u64,
    /// Tokens generated.
    pub completion: u64,
    /// Total as reported.
    pub total: u64,
}

/// One assistant turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatResponse {
    /// Visible text (reasoning blocks removed).
    pub content: String,
    /// Tools the model wants run, in order.
    pub tool_calls: Vec<ToolCall>,
    /// Why it stopped.
    pub finish: FinishReason,
    /// Token usage, `None` when the provider did not report it.
    pub usage: Option<Usage>,
    /// The model that answered, as the provider reports it (capped).
    pub model: Option<String>,
}
