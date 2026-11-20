//! The turn itself. See the module docs of [`super`] for the guarantees.

use super::history::{build_history, calls_to_json, storable_args};
use super::notes::system_addendum;
use super::prompt::{FINAL_NOTE, system_prompt};
use super::{Assistant, Turn, TurnError, TurnReport};
use crate::exec::{CallStatus, Outcome};
use crate::memory::DbMemorySink;
use crate::memory_tools::hides_arguments;
use crate::proposals::DbProposalSink;
use crate::store::DbStore;
use crate::tool::ToolContext;
use ai::{AiProvider, ChatRequest, ChatResponse, DeltaSink, FinishReason, Message, ToolCall};
use core_domain::redact::redact;
use db::{
    ActorKind, AuditOutcome, ClaimOutcome, Error as DbError, MessageRole, NewAuditEntry,
    NewMessage, NewToolCall, TurnClaim,
};
use serde_json::{Value, json};

fn store_err(error: DbError) -> TurnError {
    match error {
        DbError::NotFound => TurnError::NotFound,
        _ => TurnError::Store,
    }
}

/// Appending fails with `NotFound` only when the claim is gone.
fn append_err(error: DbError) -> TurnError {
    match error {
        DbError::NotFound => TurnError::ClaimLost,
        _ => TurnError::Store,
    }
}

/// Tokens spent so far in this turn.
#[derive(Default)]
struct Tally {
    used: u64,
    estimated: bool,
}

/// What one model call cost, for the stored usage JSON.
struct CallUsage {
    json: Value,
}

impl Tally {
    fn add(&mut self, request_bytes: usize, response: &ChatResponse) -> CallUsage {
        let finish = finish_word(&response.finish);
        if let Some(u) = response.usage {
            let total = if u.total > 0 {
                u.total
            } else {
                u.prompt.saturating_add(u.completion)
            };
            if total > 0 {
                self.used = self.used.saturating_add(total);
                return CallUsage {
                    json: json!({
                        "prompt": u.prompt, "completion": u.completion, "total": total,
                        "estimated": false, "finish": finish,
                    }),
                };
            }
        }
        // The provider reported nothing usable: count bytes/4 of what went in and came out, and
        // say that it is an estimate.
        let out_bytes = response.content.len()
            + response
                .tool_calls
                .iter()
                .map(|c| c.name.len() + c.arguments.len())
                .sum::<usize>();
        let total =
            u64::try_from(request_bytes.saturating_add(out_bytes) / 4 + 1).unwrap_or(u64::MAX);
        self.used = self.used.saturating_add(total);
        self.estimated = true;
        CallUsage {
            json: json!({"total": total, "estimated": true, "finish": finish}),
        }
    }
}

const fn finish_word(finish: &FinishReason) -> &'static str {
    match finish {
        FinishReason::Stop => "stop",
        FinishReason::ToolCalls => "tool_calls",
        FinishReason::Length => "length",
        FinishReason::ContentFilter => "content_filter",
        // Provider-chosen text is not stored.
        FinishReason::Other(_) => "other",
    }
}

fn request_bytes(messages: &[Message], tools: &[Value]) -> usize {
    let messages_bytes: usize = messages
        .iter()
        .map(|m| {
            m.content.len()
                + m.tool_calls
                    .iter()
                    .map(|c| c.name.len() + c.arguments.len())
                    .sum::<usize>()
        })
        .sum();
    messages_bytes + tools.iter().map(|t| t.to_string().len()).sum::<usize>()
}

fn validate_message(text: &str, max_bytes: usize) -> Result<&str, TurnError> {
    if text.len() > max_bytes {
        return Err(TurnError::MessageTooLarge);
    }
    if text.contains('\0') {
        return Err(TurnError::BadMessage);
    }
    if text.trim().is_empty() {
        return Err(TurnError::EmptyMessage);
    }
    Ok(text)
}

/// Call ids that are safe to replay: non-empty, at most 200 bytes and unique inside the response.
/// A model (or a broken proxy) that repeats or omits ids would otherwise make the next request
/// invalid, because every id must be answered exactly once.
fn unique_ids(calls: &[ToolCall], round: usize) -> Vec<String> {
    let mut ids: Vec<String> = Vec::with_capacity(calls.len());
    for (i, call) in calls.iter().enumerate() {
        let usable = !call.id.is_empty()
            && call.id.len() <= 200
            && !call.id.chars().any(char::is_control)
            && !ids.contains(&call.id);
        ids.push(if usable {
            call.id.clone()
        } else {
            format!("call_{round}_{i}")
        });
    }
    ids
}

const fn audit_outcome(status: &CallStatus) -> AuditOutcome {
    match status {
        CallStatus::Ok => AuditOutcome::Success,
        CallStatus::ToolFailed(_) => AuditOutcome::Failure,
        CallStatus::UnknownTool
        | CallStatus::TierDenied
        | CallStatus::BadArguments(_)
        | CallStatus::Refused(_) => AuditOutcome::Denied,
    }
}

impl<P: AiProvider> Assistant<'_, P> {
    /// Answers one person message.
    ///
    /// # Errors
    /// [`TurnError`]. On a provider error nothing is stored in place of an answer; messages and
    /// tool results of rounds that completed stay stored.
    pub async fn turn(
        &self,
        turn: &Turn<'_>,
        sink: &mut dyn DeltaSink,
    ) -> Result<TurnReport, TurnError> {
        let message = validate_message(turn.message, self.config.max_message_bytes)?;
        let conversation =
            db::get_conversation(self.conn, turn.org, turn.conversation).map_err(store_err)?;
        // Another person's conversation is indistinguishable from a missing one, and so is the
        // conversation of someone who is no longer a member: a stale session cannot keep using it.
        if conversation.user_id != *turn.user
            || !db::is_member(self.conn, turn.org, turn.user).map_err(store_err)?
        {
            return Err(TurnError::NotFound);
        }
        let claim = match db::claim_turn(
            self.conn,
            turn.org,
            turn.conversation,
            self.config.claim_stale_secs,
        ) {
            Ok(ClaimOutcome::Claimed(claim)) => claim,
            Ok(ClaimOutcome::Busy) => return Err(TurnError::Busy),
            Err(DbError::Invalid(_)) => return Err(TurnError::Archived),
            Err(error) => return Err(store_err(error)),
        };
        let result = self.run(turn, &claim, message, sink).await;
        // A failed release leaves a claim that expires on its own; it must not hide the result.
        let _ = db::release_turn(self.conn, turn.org, turn.conversation, &claim);
        result
    }

    async fn run(
        &self,
        turn: &Turn<'_>,
        claim: &TurnClaim,
        message: &str,
        sink: &mut dyn DeltaSink,
    ) -> Result<TurnReport, TurnError> {
        let rows = db::list_messages(
            self.conn,
            turn.org,
            turn.conversation,
            self.config.history_messages,
        )
        .map_err(store_err)?;
        // Organization notes: ACTIVE memories only, fenced as data (pending ones never appear).
        let mut system = system_prompt(turn.locale).to_owned();
        system.push_str("\n\n");
        system.push_str(&system_addendum(
            self.conn,
            turn.org,
            turn.user,
            turn.locale,
        ));
        let mut context = vec![Message::system(system)];
        context.extend(build_history(&rows));

        // Redact BEFORE anything is stored or sent: the raw text goes nowhere else.
        let redaction = redact(message);
        let redactions = u32::try_from(redaction.count).unwrap_or(u32::MAX);
        self.append(
            turn,
            claim,
            &[NewMessage {
                role: MessageRole::User,
                content: &redaction.text,
                tool_calls_json: None,
                tool_call_id: None,
                token_usage_json: None,
                redactions,
            }],
            &[],
        )?;
        context.push(Message::user(redaction.text));

        let tools: Vec<Value> = self
            .executor
            .registry()
            .definitions()
            .iter()
            .map(crate::ToolDefinition::to_openai)
            .collect();
        let store = DbStore::new(self.conn);
        let proposals = DbProposalSink::new(self.conn, Some(turn.conversation.clone()));
        let memory = DbMemorySink::new(
            self.conn,
            Some(turn.conversation.clone()),
            Some(turn.user.clone()),
        );
        let tool_ctx = ToolContext::new(turn.org, &store, self.catalog, turn.locale)
            .with_proposals(&proposals)
            .with_memory(&memory);

        let mut tally = Tally::default();
        let (mut model_calls, mut tool_calls) = (0u32, 0u32);
        for round in 0..usize::from(self.config.max_rounds) {
            let (response, usage) = self
                .call_model(turn, claim, &context, tools.clone(), &mut tally, sink)
                .await?;
            model_calls += 1;
            if response.tool_calls.is_empty() {
                return self.finish(
                    turn,
                    claim,
                    &response,
                    &usage,
                    &tally,
                    model_calls,
                    tool_calls,
                    false,
                    redactions,
                );
            }
            tool_calls = tool_calls
                .saturating_add(u32::try_from(response.tool_calls.len()).unwrap_or(u32::MAX));
            self.run_calls(
                turn,
                claim,
                &tool_ctx,
                (&response, &usage),
                round,
                &mut context,
            )?;
        }

        // The model kept asking for tools: one last call without any, asking for an answer.
        context.push(Message::system(FINAL_NOTE));
        let (response, usage) = self
            .call_model(turn, claim, &context, Vec::new(), &mut tally, sink)
            .await?;
        model_calls += 1;
        // No tools were offered, so any call in this response is ignored, never executed.
        self.finish(
            turn,
            claim,
            &response,
            &usage,
            &tally,
            model_calls,
            tool_calls,
            true,
            redactions,
        )
    }

    async fn call_model(
        &self,
        turn: &Turn<'_>,
        claim: &TurnClaim,
        context: &[Message],
        tools: Vec<Value>,
        tally: &mut Tally,
        sink: &mut dyn DeltaSink,
    ) -> Result<(ChatResponse, CallUsage), TurnError> {
        // Heartbeat: keeps a long turn's claim fresh, and fails if it was taken over.
        db::touch_turn(self.conn, turn.org, turn.conversation, claim).map_err(append_err)?;
        if tally.used >= self.config.token_budget {
            return Err(TurnError::BudgetExceeded {
                used: tally.used,
                budget: self.config.token_budget,
                estimated: tally.estimated,
            });
        }
        let request = ChatRequest {
            messages: context.to_vec(),
            tools,
            max_tokens: self.config.max_tokens,
            temperature: None,
        };
        let bytes = request_bytes(&request.messages, &request.tools);
        let response = self
            .provider
            .chat(&request, sink)
            .await
            .map_err(TurnError::Provider)?;
        let usage = tally.add(bytes, &response);
        Ok((response, usage))
    }

    /// Runs every tool call of one response, stores the group atomically, audits each call, and
    /// extends the context with the sanitised group.
    fn run_calls(
        &self,
        turn: &Turn<'_>,
        claim: &TurnClaim,
        tool_ctx: &ToolContext<'_>,
        (response, usage): (&ChatResponse, &CallUsage),
        round: usize,
        context: &mut Vec<Message>,
    ) -> Result<(), TurnError> {
        let ids = unique_ids(&response.tool_calls, round);
        let outcomes: Vec<Outcome> = response
            .tool_calls
            .iter()
            .enumerate()
            .map(|(i, call)| {
                if i < self.config.max_calls_per_round {
                    self.executor.call(tool_ctx, &call.name, &call.arguments)
                } else {
                    self.executor
                        .refuse(&call.name, &call.arguments, "too_many_calls")
                }
            })
            .collect();

        // What is stored and replayed: sanitised names (what the registry saw) and redacted,
        // valid-JSON arguments. The raw text the model produced is not kept.
        let replay_calls: Vec<ToolCall> = response
            .tool_calls
            .iter()
            .zip(&ids)
            .zip(&outcomes)
            .map(|((call, id), outcome)| ToolCall {
                id: id.clone(),
                name: outcome.record.tool.clone(),
                // What `remember` was asked to store stays out of replayed history: it is a
                // pending note and must not reach a prompt before a person approves it.
                arguments: if hides_arguments(&outcome.record.tool) {
                    "{}".to_owned()
                } else {
                    storable_args(&call.arguments)
                },
            })
            .collect();
        let calls_json = calls_to_json(&replay_calls);
        let assistant_text = redact(&response.content);
        let usage_json = usage.json.to_string();

        let mut messages = vec![NewMessage {
            role: MessageRole::Assistant,
            content: &assistant_text.text,
            tool_calls_json: Some(&calls_json),
            tool_call_id: None,
            token_usage_json: Some(&usage_json),
            redactions: u32::try_from(assistant_text.count).unwrap_or(u32::MAX),
        }];
        let mut records = Vec::with_capacity(outcomes.len());
        for (i, (id, outcome)) in ids.iter().zip(&outcomes).enumerate() {
            messages.push(NewMessage {
                role: MessageRole::Tool,
                content: &outcome.content,
                tool_calls_json: None,
                tool_call_id: Some(id),
                token_usage_json: None,
                redactions: u32::try_from(outcome.record.redactions).unwrap_or(u32::MAX),
            });
            let r = &outcome.record;
            records.push(NewToolCall {
                message_index: 0,
                idx: u32::try_from(i).unwrap_or(u32::MAX),
                call_id: id,
                tool: &r.tool,
                tier: r.tier.map(core_domain::tier::Tier::level),
                args: &r.args,
                status: r.status.code(),
                duration_ms: r.duration_ms,
                redactions: u32::try_from(r.redactions).unwrap_or(u32::MAX),
                truncated: r.truncated,
            });
        }
        self.append(turn, claim, &messages, &records)?;

        for (id, outcome) in ids.iter().zip(&outcomes) {
            self.audit_call(turn, id, outcome)?;
        }

        context.push(Message::assistant_calls(assistant_text.text, replay_calls));
        for (id, outcome) in ids.into_iter().zip(outcomes) {
            context.push(Message::tool_result(id, outcome.content));
        }
        Ok(())
    }

    fn audit_call(
        &self,
        turn: &Turn<'_>,
        call_id: &str,
        outcome: &Outcome,
    ) -> Result<(), TurnError> {
        let r = &outcome.record;
        // Redacted arguments and fixed codes only: never the raw output.
        let subject = json!({
            "conversation": turn.conversation.as_str(),
            "user": turn.user.as_str(),
            "call_id": call_id,
            "tool": r.tool,
            "tier": r.tier.map(core_domain::tier::Tier::level),
            "status": r.status.code(),
            "args": r.args,
        })
        .to_string();
        db::append_audit(
            self.conn,
            turn.org,
            &NewAuditEntry {
                actor_kind: ActorKind::System,
                actor_id: Some("assistant"),
                action: "assistant.tool_call",
                node_id: None,
                subject_json: &subject,
                result: audit_outcome(&r.status),
            },
        )
        .map_err(|_| TurnError::Store)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn finish(
        &self,
        turn: &Turn<'_>,
        claim: &TurnClaim,
        response: &ChatResponse,
        usage: &CallUsage,
        tally: &Tally,
        model_calls: u32,
        tool_calls: u32,
        forced_final: bool,
        redactions: u32,
    ) -> Result<TurnReport, TurnError> {
        let reply = redact(&response.content);
        // Nothing to show means no answer: never store an empty or made-up reply.
        if reply.text.trim().is_empty() {
            return Err(TurnError::NoAnswer);
        }
        let usage_json = usage.json.to_string();
        self.append(
            turn,
            claim,
            &[NewMessage {
                role: MessageRole::Assistant,
                content: &reply.text,
                tool_calls_json: None,
                tool_call_id: None,
                token_usage_json: Some(&usage_json),
                redactions: u32::try_from(reply.count).unwrap_or(u32::MAX),
            }],
            &[],
        )?;
        Ok(TurnReport {
            reply: reply.text,
            model_calls,
            tool_calls,
            tokens_used: tally.used,
            tokens_estimated: tally.estimated,
            cut_off: response.finish == FinishReason::Length,
            forced_final,
            redactions,
        })
    }

    fn append(
        &self,
        turn: &Turn<'_>,
        claim: &TurnClaim,
        messages: &[NewMessage<'_>],
        calls: &[NewToolCall<'_>],
    ) -> Result<(), TurnError> {
        db::append_messages(
            self.conn,
            turn.org,
            turn.conversation,
            claim,
            messages,
            calls,
        )
        .map(|_| ())
        .map_err(append_err)
    }
}
