//! `POST /api/conversations/{id}/turn`: one assistant turn, streamed as Server-Sent Events.
//!
//! Events, in order: any number of `delta {text}` (redacted; see `stream::DeltaRedactor`) and `tool {name, tier,
//! status, duration_ms}` (each before the text of the next model call), then exactly one of
//! `done {rounds, tool_calls, usage}` or `error {code, message}`. After `error` nothing follows
//! and no reply is invented: the assistant stores no answer for a failed turn.
//!
//! The turn itself is `agent::Assistant::turn`, with its limits, run on a thread of its own with
//! its own database connection (the assistant holds a non-`Send` connection). One turn runs per
//! conversation: a second request gets `409 busy`. When the browser goes away the turn still runs
//! to its end (it is bounded by the round limit, the token budget and the provider timeouts) and
//! then releases the conversation, so a disconnect never leaves it claimed.

use crate::api::{Reply, own_conversation, with_db};
use crate::app::App;
use crate::errors::{ApiError, INTERNAL, INVALID_INPUT, ai_code, load_code, message, turn_code};
use crate::guard::Lang;
use crate::stream::DeltaRedactor;
use crate::util::{clean_text, json_object, opt_str};
use agent::{Assistant, DbProviderLoader, Executor, ProviderLoader, Turn, TurnConfig};
use axum::Extension;
use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use core_domain::redact::redact;
use db::{ConversationId, OrgId, UserId};
use futures_core::Stream;
use i18n::Locale;
use serde_json::{Value, json};
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// Longest tool name shown (the name is whatever the model wrote).
const MAX_TOOL_NAME_CHARS: usize = 80;

/// The body of the response: whatever the turn thread sends.
struct Events(UnboundedReceiver<Bytes>);

impl Stream for Events {
    type Item = Result<Bytes, Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.0.poll_recv(cx).map(|item| item.map(Ok))
    }
}

/// Holds the conversation as "running" in this process until dropped.
struct Slot {
    app: Arc<App>,
    id: String,
}

impl Slot {
    fn take(app: &Arc<App>, id: &ConversationId) -> Option<Self> {
        app.turns().insert(id.as_str().to_owned()).then(|| Self {
            app: Arc::clone(app),
            id: id.as_str().to_owned(),
        })
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.app.turns().remove(&self.id);
    }
}

fn frame(event: &str, data: &Value) -> Bytes {
    // One JSON document on one line: newlines inside strings are escaped by the serializer.
    Bytes::from(format!("event: {event}\ndata: {data}\n\n"))
}

/// What the turn thread needs to run and to talk.
struct Job {
    app: Arc<App>,
    org: OrgId,
    user: UserId,
    conversation: ConversationId,
    message: String,
    locale: Locale,
    tx: UnboundedSender<Bytes>,
    _slot: Slot,
}

/// Sends the text of the model after redaction (held back until a cut is safe, so the pieces add
/// up to the redaction of the whole answer) and the tool calls that were stored since the last
/// look, before the text that follows them.
struct Relay {
    tx: UnboundedSender<Bytes>,
    /// A second connection, only to read the tool calls the turn stores.
    observer: db::Connection,
    org: OrgId,
    conversation: ConversationId,
    seen_calls: usize,
    redactor: DeltaRedactor,
    sent_text: bool,
}

impl Relay {
    fn tools(&mut self) {
        let Ok(calls) = db::list_tool_calls(&self.observer, &self.org, &self.conversation) else {
            return;
        };
        for call in calls.iter().skip(self.seen_calls) {
            let name: String = clean_text(&call.tool)
                .chars()
                .take(MAX_TOOL_NAME_CHARS)
                .collect();
            let _ = self.tx.send(frame(
                "tool",
                &json!({
                    "name": name,
                    "tier": call.tier,
                    "status": call.status,
                    "duration_ms": call.duration_ms,
                }),
            ));
        }
        self.seen_calls = self.seen_calls.max(calls.len());
    }

    fn text(&mut self, piece: &str) {
        self.tools();
        if let Some(safe) = self.redactor.push(piece) {
            self.emit(&safe);
        }
    }

    /// Sends text that is already redacted.
    fn emit(&mut self, safe: &str) {
        self.sent_text = true;
        let _ = self.tx.send(frame("delta", &json!({"text": safe})));
    }

    fn send_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        // The model's words are redacted like everything else that reaches a person's screen.
        let safe = redact(text).text;
        self.emit(&safe);
    }

    fn finish_text(&mut self) {
        if let Some(rest) = self.redactor.finish() {
            self.emit(&rest);
        }
    }
}

fn error_event(app: &App, locale: Locale, code: &str) -> Bytes {
    frame(
        "error",
        &json!({"code": code, "message": message(&app.catalog, locale, code)}),
    )
}

/// The body of the turn thread: build, run, report.
fn run(job: Job) {
    let Job {
        app,
        org,
        user,
        conversation,
        message: text,
        locale,
        tx,
        _slot,
    } = job;
    let fail = |code: &str| {
        let _ = tx.send(error_event(&app, locale, code));
    };
    let (Ok(conn), Ok(observer)) = (db::open(&app.db_path), db::open(&app.db_path)) else {
        return fail("store_failed");
    };
    let provider_id = match db::get_conversation(&conn, &org, &conversation) {
        Ok(row) => row.provider_id,
        Err(_) => return fail("conversation_not_found"),
    };
    let vault = match &app.vault {
        Some(vault) => Arc::clone(vault),
        None => match bootstrap::placeholder_vault() {
            Ok(vault) => Arc::new(vault),
            Err(_) => return fail("internal"),
        },
    };
    let provider = match DbProviderLoader::new(&conn, &vault).load(&org, provider_id.as_ref()) {
        Ok(provider) => provider,
        Err(error) => return fail(load_code(&error)),
    };
    let Ok(registry) = agent::assistant_registry() else {
        return fail("internal");
    };
    let executor = Executor::new(registry);
    let assistant = Assistant::new(&conn, &provider, &executor, &app.catalog)
        .with_config(TurnConfig::default());
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return fail("internal");
    };
    let seen_calls = db::list_tool_calls(&observer, &org, &conversation).map_or(0, |c| c.len());
    let mut relay = Relay {
        tx: tx.clone(),
        observer,
        org: org.clone(),
        conversation: conversation.clone(),
        seen_calls,
        redactor: DeltaRedactor::default(),
        sent_text: false,
    };
    let result = {
        let mut sink = |piece: &str| relay.text(piece);
        runtime.block_on(assistant.turn(
            &Turn {
                org: &org,
                user: &user,
                conversation: &conversation,
                message: &text,
                locale,
            },
            &mut sink,
        ))
    };
    relay.finish_text();
    relay.tools();
    match result {
        Ok(report) => {
            if !relay.sent_text {
                // The provider did not stream: show the stored (redacted) answer.
                relay.send_text(&report.reply);
            }
            let _ = tx.send(frame(
                "done",
                &json!({
                    "rounds": report.model_calls,
                    "tool_calls": report.tool_calls,
                    "usage": {"tokens": report.tokens_used, "estimated": report.tokens_estimated},
                }),
            ));
        }
        Err(error) => {
            // A provider error carries its own advice; anything else has a fixed code.
            let code = match &error {
                agent::TurnError::Provider(ai) => ai_code(ai),
                other => turn_code(other),
            };
            fail(code);
        }
    }
}

/// `POST /api/conversations/{id}/turn {message}`.
pub(crate) async fn turn(
    State(app): State<Arc<App>>,
    Extension(lang): Extension<Lang>,
    Path(id): Path<String>,
    body: Bytes,
) -> Reply {
    let map = json_object(&body, false).ok_or(INVALID_INPUT)?;
    if map.keys().any(|k| k != "message") {
        return Err(INVALID_INPUT);
    }
    let Some(text) = opt_str(&map, "message").map_err(|()| INVALID_INPUT)? else {
        return Err(INVALID_INPUT);
    };
    if text.trim().is_empty() {
        return Err(ApiError::new("message_empty"));
    }
    if text.len() > TurnConfig::default().max_message_bytes {
        return Err(ApiError::new("message_too_large"));
    }
    if text.contains('\0') {
        return Err(ApiError::new("message_invalid"));
    }
    let text = text.to_owned();
    // Everything that can be refused is refused before the stream starts, with a real status.
    let conversation = with_db(&app, move |conn, app| {
        let row = own_conversation(conn, app, &id)?;
        if row.archived_at.is_some() {
            return Err(ApiError::new("conversation_archived"));
        }
        let provider = match &row.provider_id {
            Some(provider) => db::get_ai_provider(conn, &app.scope.org, provider),
            None => db::default_ai_provider(conn, &app.scope.org),
        };
        match provider {
            Ok(provider) if provider.secret_id.is_some() && app.vault.is_none() => {
                Err(ApiError::new("master_key_missing"))
            }
            Ok(_) => Ok(row.id),
            Err(db::Error::NotFound) => Err(ApiError::new("no_provider")),
            Err(_) => Err(crate::errors::STORE_FAILED),
        }
    })
    .await?;
    let slot = Slot::take(&app, &conversation).ok_or(ApiError::new("busy"))?;
    let (tx, rx) = unbounded_channel();
    let job = Job {
        org: app.scope.org.clone(),
        user: app.scope.user.clone(),
        conversation,
        message: text,
        locale: lang.0,
        tx,
        _slot: slot,
        app,
    };
    std::thread::Builder::new()
        .name("nodivara-turn".to_owned())
        .spawn(move || run(job))
        .map_err(|_| INTERNAL)?;
    let mut response = Response::new(Body::from_stream(Events(rx)));
    *response.status_mut() = StatusCode::OK;
    let headers = response.headers_mut();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream; charset=utf-8"),
    );
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}
