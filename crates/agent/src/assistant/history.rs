//! Rebuilding the model's context from stored messages.
//!
//! Stored history can be cut in odd places: the window may start in the middle of a round, and a
//! crash may have left a group unfinished. Providers reject a tool message without its call and a
//! call without its answers, so only whole, well-formed groups are replayed.

use ai::{Message, ToolCall};
use core_domain::redact::redact;
use db::{MessageRole, MessageRow};
use serde_json::{Value, json};

/// Longest argument text stored or replayed for a call.
const MAX_STORED_ARGS: usize = 4000;

/// Arguments as they are stored and replayed: redacted, and only if still a JSON object within
/// the size cap. Anything else (broken JSON from the model, a cut-off string) becomes `{}`, so the
/// history never carries text the provider could choke on or a secret fragment.
pub(crate) fn storable_args(raw: &str) -> String {
    let redacted = redact(raw).text;
    let is_object = redacted.len() <= MAX_STORED_ARGS
        && matches!(
            serde_json::from_str::<Value>(&redacted),
            Ok(Value::Object(_))
        );
    if is_object { redacted } else { "{}".to_owned() }
}

/// The JSON stored in `messages.tool_calls`.
pub(crate) fn calls_to_json(calls: &[ToolCall]) -> String {
    Value::Array(
        calls
            .iter()
            .map(|c| json!({"id": c.id, "name": c.name, "arguments": c.arguments}))
            .collect(),
    )
    .to_string()
}

fn calls_from_json(text: &str) -> Option<Vec<ToolCall>> {
    let Value::Array(items) = serde_json::from_str::<Value>(text).ok()? else {
        return None;
    };
    let mut calls = Vec::with_capacity(items.len());
    for item in &items {
        calls.push(ToolCall {
            id: item.get("id")?.as_str()?.to_owned(),
            name: item.get("name")?.as_str()?.to_owned(),
            arguments: item.get("arguments")?.as_str()?.to_owned(),
        });
    }
    (!calls.is_empty()).then_some(calls)
}

/// Messages to replay, oldest first. Starts at the first user message, keeps plain messages, and
/// keeps an assistant tool-call message only with the full set of its answers.
pub(crate) fn build_history(rows: &[MessageRow]) -> Vec<Message> {
    let mut out = Vec::new();
    let mut i = rows
        .iter()
        .position(|r| r.role == MessageRole::User)
        .unwrap_or(rows.len());
    while i < rows.len() {
        let row = &rows[i];
        i += 1;
        match (row.role, row.tool_calls_json.as_deref()) {
            (MessageRole::User, _) => out.push(Message::user(row.content.clone())),
            (MessageRole::Assistant, None) => out.push(Message::assistant(row.content.clone())),
            (MessageRole::Assistant, Some(json)) => {
                let answers: Vec<&MessageRow> = rows[i..]
                    .iter()
                    .take_while(|r| r.role == MessageRole::Tool)
                    .collect();
                i += answers.len();
                let Some(calls) = calls_from_json(json) else {
                    continue;
                };
                let answered = |id: &str| {
                    answers
                        .iter()
                        .filter(|a| a.tool_call_id.as_deref() == Some(id))
                        .count()
                        == 1
                };
                let complete =
                    answers.len() == calls.len() && calls.iter().all(|c| answered(&c.id));
                if complete {
                    out.push(Message::assistant_calls(row.content.clone(), calls));
                    for a in answers {
                        if let Some(id) = &a.tool_call_id {
                            out.push(Message::tool_result(id.clone(), a.content.clone()));
                        }
                    }
                }
            }
            // A tool message here has no call before it; a system message is never stored.
            (MessageRole::Tool | MessageRole::System, _) => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broken_or_oversized_arguments_become_an_empty_object() {
        assert_eq!(storable_args("{not json"), "{}");
        assert_eq!(storable_args("[1]"), "{}");
        assert_eq!(
            storable_args(&format!("{{\"a\":\"{}\"}}", "x".repeat(5000))),
            "{}"
        );
        assert_eq!(storable_args(r#"{"node":"web-1"}"#), r#"{"node":"web-1"}"#);
    }

    #[test]
    fn a_secret_in_arguments_is_redacted_but_the_json_survives() {
        let out = storable_args(r#"{"note":"DB_PASSWORD=hunter2hunter2"}"#);
        assert!(!out.contains("hunter2"), "{out}");
    }
}
