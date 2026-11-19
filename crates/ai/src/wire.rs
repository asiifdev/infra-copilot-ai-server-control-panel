//! The OpenAI-compatible wire format: building requests, parsing answers, assembling streams.

use crate::config::ProviderConfig;
use crate::error::AiError;
use crate::provider::DeltaSink;
use crate::think::{ThinkFilter, strip_think};
use crate::types::{ChatRequest, ChatResponse, FinishReason, Message, Role, ToolCall, Usage};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

/// Most tool calls accepted from one answer.
const MAX_TOOL_CALLS: usize = 32;
/// Longest tool name accepted.
const MAX_NAME_BYTES: usize = 64;
/// Longest raw argument string accepted per call.
const MAX_ARGS_BYTES: usize = 64 * 1024;
/// Longest visible text accepted.
const MAX_CONTENT_BYTES: usize = 1024 * 1024;
/// Largest request body sent.
const MAX_REQUEST_BYTES: usize = 2 * 1024 * 1024;

/// Builds the JSON body. Only fields the provider is configured for are included, and none is
/// ever `null`.
pub(crate) fn request_body(
    config: &ProviderConfig,
    req: &ChatRequest,
    stream: bool,
) -> Result<Vec<u8>, AiError> {
    if req.messages.is_empty() {
        return Err(AiError::Config("no messages"));
    }
    if !req.tools.is_empty() && !config.supports_tools {
        return Err(AiError::Unsupported("tools"));
    }
    let mut body = Map::new();
    body.insert("model".into(), json!(config.model));
    body.insert(
        "messages".into(),
        Value::Array(req.messages.iter().map(message_json).collect()),
    );
    body.insert(
        "max_tokens".into(),
        json!(req.max_tokens.unwrap_or(config.max_tokens)),
    );
    if let Some(t) = req.temperature {
        body.insert("temperature".into(), json!(t));
    }
    if !req.tools.is_empty() {
        body.insert("tools".into(), Value::Array(req.tools.clone()));
    }
    if let Some(effort) = config.reasoning_effort {
        body.insert("reasoning_effort".into(), json!(effort.as_str()));
    }
    if stream {
        body.insert("stream".into(), json!(true));
        if config.include_usage_in_stream {
            body.insert("stream_options".into(), json!({"include_usage": true}));
        }
    }
    let bytes = serde_json::to_vec(&Value::Object(body))
        .map_err(|_| AiError::Config("request is not serialisable"))?;
    if bytes.len() > MAX_REQUEST_BYTES {
        return Err(AiError::Unsupported("request too large"));
    }
    Ok(bytes)
}

fn message_json(m: &Message) -> Value {
    let mut out = Map::new();
    out.insert("role".into(), json!(m.role.as_str()));
    let calls_only = m.role == Role::Assistant && !m.tool_calls.is_empty() && m.content.is_empty();
    if !calls_only {
        out.insert("content".into(), json!(m.content));
    }
    if !m.tool_calls.is_empty() {
        let calls = m
            .tool_calls
            .iter()
            .map(|c| json!({"id": c.id, "type": "function", "function": {"name": c.name, "arguments": c.arguments}}))
            .collect();
        out.insert("tool_calls".into(), Value::Array(calls));
    }
    if let Some(id) = &m.tool_call_id {
        out.insert("tool_call_id".into(), json!(id));
    }
    Value::Object(out)
}

/// Parses a non-streaming answer.
pub(crate) fn parse_completion(bytes: &[u8]) -> Result<ChatResponse, AiError> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| AiError::Malformed("answer is not JSON"))?;
    if value.get("error").is_some_and(|e| !e.is_null()) && value.get("choices").is_none() {
        return Err(AiError::Server(502));
    }
    let choice = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|c| c.first())
        .ok_or(AiError::Malformed("answer has no choices"))?;
    let message = choice
        .get("message")
        .ok_or(AiError::Malformed("choice has no message"))?;
    let content = strip_think(&text_of(message.get("content")));
    let mut calls = Vec::new();
    if let Some(list) = message.get("tool_calls").and_then(Value::as_array) {
        if list.len() > MAX_TOOL_CALLS {
            return Err(AiError::Malformed("too many tool calls"));
        }
        for (i, call) in list.iter().enumerate() {
            let function = call
                .get("function")
                .ok_or(AiError::Malformed("tool call has no function"))?;
            calls.push(checked_call(
                call.get("id").and_then(Value::as_str).unwrap_or_default(),
                function
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                &arguments_text(function.get("arguments")),
                i,
            )?);
        }
    }
    finalise(
        content,
        calls,
        choice.get("finish_reason").and_then(Value::as_str),
        parse_usage(value.get("usage")),
        value.get("model").and_then(Value::as_str),
    )
}

/// Content can be a string, `null`, or (some servers) a list of `{type:"text", text}` parts.
fn text_of(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

/// Arguments are normally a string; a few servers send an object. Keep it as text either way.
fn arguments_text(arguments: Option<&Value>) -> String {
    match arguments {
        Some(Value::String(s)) => s.clone(),
        Some(v @ Value::Object(_)) => v.to_string(),
        _ => String::new(),
    }
}

fn checked_call(id: &str, name: &str, arguments: &str, index: usize) -> Result<ToolCall, AiError> {
    if name.is_empty() || name.len() > MAX_NAME_BYTES {
        return Err(AiError::Malformed("tool call name is missing or too long"));
    }
    if arguments.len() > MAX_ARGS_BYTES {
        return Err(AiError::Malformed("tool call arguments are too large"));
    }
    // Some local servers omit ids; the tool result must still answer something.
    let id = if id.is_empty() {
        format!("call_{index}")
    } else {
        id.chars().take(100).collect()
    };
    Ok(ToolCall {
        id,
        name: name.to_owned(),
        arguments: arguments.to_owned(),
    })
}

fn parse_usage(usage: Option<&Value>) -> Option<Usage> {
    let usage = usage?.as_object()?;
    let get = |k: &str| usage.get(k).and_then(Value::as_u64);
    let prompt = get("prompt_tokens")?;
    let completion = get("completion_tokens").unwrap_or(0);
    Some(Usage {
        prompt,
        completion,
        total: get("total_tokens").unwrap_or(prompt.saturating_add(completion)),
    })
}

fn finish_reason(reason: Option<&str>, has_calls: bool) -> FinishReason {
    match reason {
        Some("stop") => FinishReason::Stop,
        Some("tool_calls" | "function_call") => FinishReason::ToolCalls,
        Some("length") => FinishReason::Length,
        Some("content_filter") => FinishReason::ContentFilter,
        Some(other) => FinishReason::Other(
            other
                .chars()
                .filter(|c| c.is_ascii_graphic())
                .take(40)
                .collect(),
        ),
        None if has_calls => FinishReason::ToolCalls,
        None => FinishReason::Stop,
    }
}

fn finalise(
    content: String,
    calls: Vec<ToolCall>,
    reason: Option<&str>,
    usage: Option<Usage>,
    model: Option<&str>,
) -> Result<ChatResponse, AiError> {
    if content.len() > MAX_CONTENT_BYTES {
        return Err(AiError::Malformed("answer is too long"));
    }
    let finish = finish_reason(reason, !calls.is_empty());
    // An empty answer cut by the token limit is reported as such, so the caller can say "raise
    // the limit"; any other empty answer is a failure, never a made-up reply.
    if content.trim().is_empty() && calls.is_empty() && finish != FinishReason::Length {
        return Err(AiError::Empty);
    }
    Ok(ChatResponse {
        content,
        tool_calls: calls,
        finish,
        usage,
        model: model.map(|m| m.chars().filter(|c| !c.is_control()).take(100).collect()),
    })
}

#[derive(Default)]
struct PartialCall {
    id: String,
    name: String,
    arguments: String,
}

/// Builds a [`ChatResponse`] from the `data:` payloads of a streamed answer. Tool-call fragments
/// are merged by `index`; text is passed to the sink as it arrives (reasoning blocks removed).
#[derive(Default)]
pub struct StreamAssembler {
    content: String,
    filter: ThinkFilter,
    calls: BTreeMap<usize, PartialCall>,
    finish: Option<String>,
    usage: Option<Usage>,
    model: Option<String>,
    done: bool,
}

impl StreamAssembler {
    /// A fresh assembler.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the `[DONE]` marker was seen.
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// Takes the payload of one event.
    ///
    /// # Errors
    /// [`AiError::Malformed`] for bad JSON or a breach of a size cap; [`AiError::Server`] when the
    /// provider reports an error inside the stream.
    pub fn feed(&mut self, data: &str, sink: &mut dyn DeltaSink) -> Result<(), AiError> {
        let data = data.trim();
        if data == "[DONE]" {
            self.done = true;
            return Ok(());
        }
        if data.is_empty() {
            return Ok(());
        }
        let chunk: Value = serde_json::from_str(data)
            .map_err(|_| AiError::Malformed("stream chunk is not JSON"))?;
        if chunk.get("error").is_some_and(|e| !e.is_null()) {
            return Err(AiError::Server(502));
        }
        if let Some(model) = chunk.get("model").and_then(Value::as_str) {
            self.model.get_or_insert_with(|| {
                model
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(100)
                    .collect()
            });
        }
        if let Some(usage) = parse_usage(chunk.get("usage")) {
            self.usage = Some(usage);
        }
        let Some(choice) = chunk
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|c| c.first())
        else {
            return Ok(()); // a usage-only chunk has no choices
        };
        if let Some(delta) = choice.get("delta") {
            if let Some(piece) = delta.get("content").and_then(Value::as_str) {
                let visible = self.filter.push(piece);
                self.add_text(&visible, sink)?;
            }
            if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
                for (position, call) in calls.iter().enumerate() {
                    self.merge_call(position, call)?;
                }
            }
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish = Some(reason.to_owned());
        }
        Ok(())
    }

    fn add_text(&mut self, text: &str, sink: &mut dyn DeltaSink) -> Result<(), AiError> {
        if text.is_empty() {
            return Ok(());
        }
        if self.content.len() + text.len() > MAX_CONTENT_BYTES {
            return Err(AiError::Malformed("answer is too long"));
        }
        self.content.push_str(text);
        sink.text(text);
        Ok(())
    }

    fn merge_call(&mut self, position: usize, call: &Value) -> Result<(), AiError> {
        let index = call
            .get("index")
            .and_then(Value::as_u64)
            .and_then(|i| usize::try_from(i).ok())
            .unwrap_or(position);
        if index >= MAX_TOOL_CALLS {
            return Err(AiError::Malformed("too many tool calls"));
        }
        let entry = self.calls.entry(index).or_default();
        if let Some(id) = call.get("id").and_then(Value::as_str)
            && entry.id.is_empty()
        {
            entry.id = id.chars().take(100).collect();
        }
        if let Some(function) = call.get("function") {
            if let Some(name) = function.get("name").and_then(Value::as_str) {
                if entry.name.is_empty() {
                    entry.name = name.to_owned();
                } else if entry.name != name {
                    entry.name.push_str(name);
                }
            }
            if let Some(args) = function.get("arguments").and_then(Value::as_str) {
                if entry.arguments.len() + args.len() > MAX_ARGS_BYTES {
                    return Err(AiError::Malformed("tool call arguments are too large"));
                }
                entry.arguments.push_str(args);
            }
            if entry.name.len() > MAX_NAME_BYTES {
                return Err(AiError::Malformed("tool call name is missing or too long"));
            }
        }
        Ok(())
    }

    /// Ends the stream and returns the answer.
    ///
    /// # Errors
    /// [`AiError::Malformed`] when the stream was cut before any finish reason or `[DONE]`,
    /// [`AiError::Empty`] for an answer with nothing in it.
    pub fn finish(mut self, sink: &mut dyn DeltaSink) -> Result<ChatResponse, AiError> {
        let tail = self.filter.finish();
        self.add_text(&tail, sink)?;
        if !self.done && self.finish.is_none() {
            return Err(AiError::Malformed("stream ended early"));
        }
        let mut calls = Vec::with_capacity(self.calls.len());
        for (index, partial) in self.calls {
            calls.push(checked_call(
                &partial.id,
                &partial.name,
                &partial.arguments,
                index,
            )?);
        }
        finalise(
            self.content,
            calls,
            self.finish.as_deref(),
            self.usage,
            self.model.as_deref(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::NoSink;

    fn config() -> ProviderConfig {
        match ProviderConfig::new("https://h.example.test/v1", "test-model") {
            Ok(c) => c,
            Err(_) => unreachable!("valid test config"),
        }
    }

    fn body(req: &ChatRequest, stream: bool) -> Value {
        let bytes = request_body(&config(), req, stream).unwrap_or_default();
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    }

    #[test]
    fn requests_carry_only_what_is_configured_and_never_null() {
        let req = ChatRequest {
            messages: vec![Message::system("s"), Message::user("hi")],
            ..ChatRequest::default()
        };
        let v = body(&req, false);
        assert_eq!(v["model"], "test-model");
        assert_eq!(v["max_tokens"], 1024);
        for absent in [
            "tools",
            "temperature",
            "stream",
            "stream_options",
            "reasoning_effort",
        ] {
            assert!(v.get(absent).is_none(), "{absent}");
        }
        fn no_nulls(v: &Value) -> bool {
            match v {
                Value::Null => false,
                Value::Array(a) => a.iter().all(no_nulls),
                Value::Object(o) => o.values().all(no_nulls),
                _ => true,
            }
        }
        assert!(no_nulls(&v));
        let v = body(
            &ChatRequest {
                temperature: Some(0.5),
                max_tokens: Some(50),
                ..req
            },
            true,
        );
        assert_eq!(v["stream"], true);
        assert_eq!(v["max_tokens"], 50);
        assert!(v.get("stream_options").is_none(), "off unless configured");
    }

    #[test]
    fn tool_exchanges_round_trip_in_the_expected_shape() {
        let call = ToolCall {
            id: "c1".into(),
            name: "list_nodes".into(),
            arguments: "{}".into(),
        };
        let req = ChatRequest {
            messages: vec![
                Message::user("what servers?"),
                Message::assistant_calls("", vec![call]),
                Message::tool_result("c1", "<untrusted_output>x</untrusted_output>"),
            ],
            tools: vec![json!({"type":"function","function":{"name":"list_nodes"}})],
            ..ChatRequest::default()
        };
        let v = body(&req, false);
        let assistant = &v["messages"][1];
        assert!(
            assistant.get("content").is_none(),
            "calls-only message omits content"
        );
        assert_eq!(assistant["tool_calls"][0]["function"]["name"], "list_nodes");
        assert_eq!(assistant["tool_calls"][0]["type"], "function");
        assert_eq!(v["messages"][2]["role"], "tool");
        assert_eq!(v["messages"][2]["tool_call_id"], "c1");
        assert_eq!(v["tools"][0]["function"]["name"], "list_nodes");
    }

    #[test]
    fn capabilities_gate_what_is_sent() {
        let mut c = config();
        c.supports_tools = false;
        let req = ChatRequest {
            messages: vec![Message::user("x")],
            tools: vec![json!({})],
            ..ChatRequest::default()
        };
        assert_eq!(
            request_body(&c, &req, false),
            Err(AiError::Unsupported("tools"))
        );
        assert_eq!(
            request_body(&c, &ChatRequest::default(), false),
            Err(AiError::Config("no messages"))
        );
        c.include_usage_in_stream = true;
        c.reasoning_effort = Some(crate::ReasoningEffort::Minimal);
        let v: Value = serde_json::from_slice(
            &request_body(
                &c,
                &ChatRequest {
                    messages: vec![Message::user("x")],
                    ..ChatRequest::default()
                },
                true,
            )
            .unwrap_or_default(),
        )
        .unwrap_or(Value::Null);
        assert_eq!(v["stream_options"]["include_usage"], true);
        assert_eq!(v["reasoning_effort"], "minimal");
    }

    #[test]
    fn parses_a_plain_answer_and_a_tool_call_answer() {
        let a = parse_completion(br#"{"model":"m1","choices":[{"message":{"content":"Hello"},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":3,"total_tokens":13}}"#);
        let a = a.unwrap_or_else(|_| unreachable!());
        assert_eq!(
            (a.content.as_str(), a.finish.clone(), a.model.as_deref()),
            ("Hello", FinishReason::Stop, Some("m1"))
        );
        assert_eq!(
            a.usage,
            Some(Usage {
                prompt: 10,
                completion: 3,
                total: 13
            })
        );
        let b = parse_completion(br#"{"choices":[{"message":{"content":null,"tool_calls":[{"id":"x1","type":"function","function":{"name":"list_nodes","arguments":"{}"}},{"function":{"name":"list_findings","arguments":{"limit":3}}}]},"finish_reason":"tool_calls"}]}"#);
        let b = b.unwrap_or_else(|_| unreachable!());
        assert_eq!(b.finish, FinishReason::ToolCalls);
        assert_eq!(b.tool_calls.len(), 2);
        assert_eq!(b.tool_calls[1].id, "call_1", "missing id is generated");
        assert_eq!(
            b.tool_calls[1].arguments, r#"{"limit":3}"#,
            "object arguments become text"
        );
        assert!(b.usage.is_none());
    }

    #[test]
    fn content_parts_and_reasoning_blocks_are_handled() {
        let r = parse_completion(br#"{"choices":[{"message":{"content":[{"type":"text","text":"<think>x</think>A"},{"type":"text","text":"B"}]},"finish_reason":"stop"}]}"#);
        assert_eq!(r.map(|r| r.content), Ok("AB".to_owned()));
    }

    #[test]
    fn bad_answers_are_refused_with_the_right_error() {
        for (raw, expected) in [
            ("not json", AiError::Malformed("answer is not JSON")),
            (r#"{}"#, AiError::Malformed("answer has no choices")),
            (
                r#"{"choices":[]}"#,
                AiError::Malformed("answer has no choices"),
            ),
            (
                r#"{"choices":[{}]}"#,
                AiError::Malformed("choice has no message"),
            ),
            (
                r#"{"choices":[{"message":{"content":"  "},"finish_reason":"stop"}]}"#,
                AiError::Empty,
            ),
            (r#"{"error":{"message":"x"}}"#, AiError::Server(502)),
            (
                r#"{"choices":[{"message":{"tool_calls":[{"function":{"arguments":"{}"}}]}}]}"#,
                AiError::Malformed("tool call name is missing or too long"),
            ),
        ] {
            assert_eq!(
                parse_completion(raw.as_bytes()).err(),
                Some(expected),
                "{raw}"
            );
        }
        let long = format!(
            r#"{{"choices":[{{"message":{{"tool_calls":[{{"function":{{"name":"{}"}}}}]}}}}]}}"#,
            "n".repeat(65)
        );
        assert!(parse_completion(long.as_bytes()).is_err());
        let big = format!(
            r#"{{"choices":[{{"message":{{"tool_calls":[{{"function":{{"name":"t","arguments":"{}"}}}}]}}}}]}}"#,
            "a".repeat(70_000)
        );
        assert!(parse_completion(big.as_bytes()).is_err());
        let many = (0..33)
            .map(|_| r#"{"function":{"name":"t"}}"#)
            .collect::<Vec<_>>()
            .join(",");
        let many = format!(r#"{{"choices":[{{"message":{{"tool_calls":[{many}]}}}}]}}"#);
        assert!(parse_completion(many.as_bytes()).is_err());
    }

    #[test]
    fn an_empty_answer_cut_by_the_token_limit_is_reported_as_length() {
        let r = parse_completion(
            br#"{"choices":[{"message":{"content":""},"finish_reason":"length"}]}"#,
        );
        assert_eq!(r.map(|r| r.finish), Ok(FinishReason::Length));
    }

    fn stream(events: &[&str]) -> Result<ChatResponse, AiError> {
        let mut a = StreamAssembler::new();
        for e in events {
            a.feed(e, &mut NoSink)?;
        }
        a.finish(&mut NoSink)
    }

    #[test]
    fn assembles_text_deltas_usage_and_the_finish_reason() {
        let r = stream(&[
            r#"{"model":"m1","choices":[{"delta":{"role":"assistant","content":""}}]}"#,
            r#"{"choices":[{"delta":{"content":"Hel"}}]}"#,
            r#"{"choices":[{"delta":{"content":"lo"},"finish_reason":"stop"}]}"#,
            r#"{"choices":[],"usage":{"prompt_tokens":5,"completion_tokens":2}}"#,
            "[DONE]",
        ]);
        let r = r.unwrap_or_else(|_| unreachable!());
        assert_eq!(r.content, "Hello");
        assert_eq!(r.finish, FinishReason::Stop);
        assert_eq!(
            r.usage,
            Some(Usage {
                prompt: 5,
                completion: 2,
                total: 7
            })
        );
        assert_eq!(r.model.as_deref(), Some("m1"));
    }

    #[test]
    fn merges_fragmented_tool_calls_by_index_in_order() {
        let r = stream(&[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a","function":{"name":"list_nodes","arguments":""}},{"index":1,"id":"b","function":{"name":"list_findings","arguments":"{\"li"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"function":{"arguments":"mit\":2}"}},{"index":0,"function":{"arguments":"{}"}}]}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
            "[DONE]",
        ]);
        let r = r.unwrap_or_else(|_| unreachable!());
        assert_eq!(r.finish, FinishReason::ToolCalls);
        let got: Vec<_> = r
            .tool_calls
            .iter()
            .map(|c| (c.id.as_str(), c.name.as_str(), c.arguments.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("a", "list_nodes", "{}"),
                ("b", "list_findings", r#"{"limit":2}"#)
            ]
        );
    }

    #[test]
    fn text_reaches_the_sink_without_reasoning_blocks() {
        let mut seen = String::new();
        let mut sink = |p: &str| seen.push_str(p);
        let mut a = StreamAssembler::new();
        for e in [
            r#"{"choices":[{"delta":{"content":"<thi"}}]}"#,
            r#"{"choices":[{"delta":{"content":"nk>plan</think>Answ"}}]}"#,
            r#"{"choices":[{"delta":{"content":"er"},"finish_reason":"stop"}]}"#,
        ] {
            assert!(a.feed(e, &mut sink).is_ok());
        }
        let r = a.finish(&mut sink);
        assert_eq!(r.map(|r| r.content), Ok("Answer".to_owned()));
        assert_eq!(seen, "Answer");
    }

    #[test]
    fn a_cut_or_broken_stream_is_an_error_not_a_made_up_answer() {
        assert_eq!(
            stream(&[r#"{"choices":[{"delta":{"content":"par"}}]}"#]).err(),
            Some(AiError::Malformed("stream ended early"))
        );
        assert_eq!(
            stream(&["{not json"]).err(),
            Some(AiError::Malformed("stream chunk is not JSON"))
        );
        assert_eq!(
            stream(&[r#"{"error":{"message":"boom"}}"#]).err(),
            Some(AiError::Server(502))
        );
        assert_eq!(
            stream(&[r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#]).err(),
            Some(AiError::Empty)
        );
        assert_eq!(
            stream(&[
                r#"{"choices":[{"delta":{"tool_calls":[{"index":40,"function":{"name":"t"}}]}}]}"#
            ])
            .err(),
            Some(AiError::Malformed("too many tool calls"))
        );
    }
}
