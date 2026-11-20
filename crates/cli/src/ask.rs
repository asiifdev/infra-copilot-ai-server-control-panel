//! `infra ask [--new | --conversation ID] [--lang L] "question"`.
//!
//! Runs one assistant turn against the org's default provider. The answer streams to stdout line
//! by line (each line redacted and stripped of control characters first), one line per tool call
//! goes to stderr, and every failure is mapped to advice in the reader's language with a
//! non-zero exit code. The turn runs on a worker thread with its own database connection, so the
//! answer can be printed while it arrives without the (non-`Send`) output handle crossing threads.

use crate::Ui;
use crate::args::{Globals, parse};
use agent::{Assistant, DbProviderLoader, Executor, LoadError, ProviderLoader, Turn, TurnError};
use ai::AiError;
use core_domain::redact::redact;
use db::{ConversationId, OrgId, UserId};
use i18n::{Catalog, Locale, MessageKey};
use std::io::Write;
use std::path::Path;
use std::sync::mpsc;
use vault::Vault;

/// Exit code for a failed model call or turn (2 stays for usage and configuration errors).
const EXIT_AI: u8 = 3;

const NO_PROVIDER: MessageKey = MessageKey::new("cli.ask.no_provider");
const TRACE: MessageKey = MessageKey::new("cli.ask.trace");
const CONVERSATION: MessageKey = MessageKey::new("cli.ask.conversation");
const BAD_CONVERSATION: MessageKey = MessageKey::new("cli.ask.bad_conversation");
const VAULT_MISMATCH: MessageKey = MessageKey::new("cli.vault.mismatch");
const KEY_MISSING: MessageKey = MessageKey::new("cli.ask.key_missing");
const PROVIDER_CONFIG: MessageKey = MessageKey::new("cli.provider.bad_config");
const SETUP: MessageKey = MessageKey::new("cli.ask_err.setup");

/// Why the worker could not produce a report.
enum Failure {
    Load(LoadError),
    Turn(TurnError),
    Setup,
}

/// Prints text as it streams in, whole lines at a time.
struct LinePrinter {
    pending: String,
    printed: bool,
}

/// Replaces control characters except tab (the model's text could carry terminal escapes).
fn clean(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_control() && c != '\t' {
                '\u{FFFD}'
            } else {
                c
            }
        })
        .collect()
}

impl LinePrinter {
    fn new() -> Self {
        Self {
            pending: String::new(),
            printed: false,
        }
    }

    fn line(&mut self, line: &str, out: &mut dyn Write) {
        let safe = clean(&redact(line).text);
        let _ = writeln!(out, "{safe}");
        self.printed = true;
    }

    fn push(&mut self, piece: &str, out: &mut dyn Write) {
        self.pending.push_str(piece);
        while let Some(end) = self.pending.find('\n') {
            let line: String = self.pending.drain(..=end).collect();
            self.line(line.trim_end_matches(['\n', '\r']), out);
        }
        let _ = out.flush();
    }

    fn finish(&mut self, out: &mut dyn Write) {
        if !self.pending.is_empty() {
            let rest = std::mem::take(&mut self.pending);
            self.line(&rest, out);
        }
        let _ = out.flush();
    }
}

/// Runs the turn. Everything is built inside the call: it runs on its own thread.
#[allow(clippy::too_many_arguments)]
fn worker(
    path: &Path,
    vault: &Vault,
    org: &OrgId,
    user: &UserId,
    conversation: &ConversationId,
    question: &str,
    locale: Locale,
    tx: mpsc::Sender<String>,
) -> Result<agent::TurnReport, Failure> {
    let conn = db::open(path).map_err(|_| Failure::Setup)?;
    let provider = DbProviderLoader::new(&conn, vault)
        .load(org, None)
        .map_err(Failure::Load)?;
    let executor = Executor::new(agent::assistant_registry().map_err(|_| Failure::Setup)?);
    let catalog = Catalog::builtin().map_err(|_| Failure::Setup)?;
    let assistant = Assistant::new(&conn, &provider, &executor, &catalog);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| Failure::Setup)?;
    let mut sink = move |piece: &str| {
        let _ = tx.send(piece.to_owned());
    };
    runtime
        .block_on(assistant.turn(
            &Turn {
                org,
                user,
                conversation,
                message: question,
                locale,
            },
            &mut sink,
        ))
        .map_err(Failure::Turn)
}

impl Ui<'_> {
    pub(crate) fn ask(&mut self, globals: &Globals, rest: &[String]) -> u8 {
        let parsed = match parse(rest, &["--conversation", "--lang"], &["--new"]) {
            Ok(parsed) => parsed,
            Err(detail) => return self.usage(&detail, Some(crate::USAGE_ASK)),
        };
        if !self.apply_lang(&parsed) {
            return self.usage("--lang", Some(crate::USAGE_ASK));
        }
        if parsed.flag("--new") && parsed.option("--conversation").is_some() {
            return self.usage("--new", Some(crate::USAGE_ASK));
        }
        let question = parsed.positional.join(" ");
        if question.trim().is_empty() {
            return self.usage("-", Some(crate::USAGE_ASK));
        }
        let (conn, path) = match self.open_db(globals, false) {
            Ok(opened) => opened,
            Err(code) => return code,
        };
        let scope = match self.scope(&conn, globals) {
            Ok(scope) => scope,
            Err(code) => return code,
        };
        let provider = match db::default_ai_provider(&conn, &scope.org) {
            Ok(provider) => provider,
            Err(db::Error::NotFound) => return self.fail_code(NO_PROVIDER, &[], EXIT_AI),
            Err(error) => return self.store_error(&error),
        };
        // The master key matters only when the provider has a sealed key to open.
        let vault = if provider.secret_id.is_some() {
            match self.vault_from_env() {
                Ok(vault) => vault,
                Err(code) => return code,
            }
        } else {
            match Vault::new(&format!("unused:{}", "0".repeat(64)), &[]) {
                Ok(vault) => vault,
                Err(_) => return self.fail(SETUP, &[]),
            }
        };
        let conversation =
            match self.pick_conversation(&conn, &scope.org, &scope.user, &parsed, &question) {
                Ok(id) => id,
                Err(code) => return code,
            };
        let seen_calls =
            db::list_tool_calls(&conn, &scope.org, &conversation).map_or(0, |c| c.len());

        let (tx, rx) = mpsc::channel::<String>();
        let locale = self.locale;
        let mut printer = LinePrinter::new();
        let outcome = std::thread::scope(|threads| {
            let handle = threads.spawn(|| {
                worker(
                    &path,
                    &vault,
                    &scope.org,
                    &scope.user,
                    &conversation,
                    &question,
                    locale,
                    tx,
                )
            });
            // The sender lives in the worker's sink; the loop ends when the worker is done.
            for piece in rx {
                printer.push(&piece, self.out);
            }
            printer.finish(self.out);
            handle.join()
        });

        // One line per tool call, from what was stored: nothing the model said is echoed raw.
        if let Ok(calls) = db::list_tool_calls(&conn, &scope.org, &conversation) {
            for call in calls.iter().skip(seen_calls) {
                let tier = call.tier.map_or_else(|| "-".to_owned(), |t| t.to_string());
                let text = self.tr(
                    TRACE,
                    &[
                        ("tool", &call.tool),
                        ("tier", &tier),
                        ("status", &call.status),
                    ],
                );
                self.note_err(&text);
            }
        }
        let text = self.tr(CONVERSATION, &[("id", conversation.as_str())]);
        self.note_err(&text);

        match outcome {
            Ok(Ok(report)) => {
                if !printer.printed {
                    // The provider did not stream: show the stored (redacted) answer.
                    let mut late = LinePrinter::new();
                    late.push(&report.reply, self.out);
                    late.finish(self.out);
                }
                crate::EXIT_OK
            }
            Ok(Err(Failure::Load(error))) => self.load_failure(error),
            Ok(Err(Failure::Turn(error))) => self.turn_failure(error),
            Ok(Err(Failure::Setup)) | Err(_) => self.fail_code(SETUP, &[], EXIT_AI),
        }
    }

    /// `--new`, `--conversation ID`, or the newest open conversation (a new one when none).
    fn pick_conversation(
        &mut self,
        conn: &db::Connection,
        org: &OrgId,
        user: &UserId,
        parsed: &crate::args::Parsed,
        question: &str,
    ) -> Result<ConversationId, u8> {
        let create = |ui: &mut Self| {
            let title: String = question.chars().take(60).collect();
            db::create_conversation(conn, org, user, &title, None).map_err(|e| ui.store_error(&e))
        };
        if let Some(text) = parsed.option("--conversation") {
            let Ok(id) = ConversationId::parse(text) else {
                return Err(self.fail(BAD_CONVERSATION, &[]));
            };
            return match db::get_conversation(conn, org, &id) {
                Ok(row) if row.user_id == *user => Ok(id),
                Ok(_) | Err(db::Error::NotFound) => Err(self.fail(BAD_CONVERSATION, &[])),
                Err(error) => Err(self.store_error(&error)),
            };
        }
        if parsed.flag("--new") {
            return create(self);
        }
        match db::list_conversations(conn, org, user, false, 1) {
            Ok(rows) => match rows.into_iter().next() {
                Some(row) => Ok(row.id),
                None => create(self),
            },
            Err(error) => Err(self.store_error(&error)),
        }
    }

    fn load_failure(&mut self, error: LoadError) -> u8 {
        match error {
            LoadError::NotFound => self.fail_code(NO_PROVIDER, &[], EXIT_AI),
            LoadError::Config(why) => self.fail(PROVIDER_CONFIG, &[("why", why)]),
            LoadError::Key("vault_not_found") => self.fail_code(KEY_MISSING, &[], EXIT_AI),
            LoadError::Key(_) => self.fail_code(VAULT_MISMATCH, &[], EXIT_AI),
            LoadError::Client | LoadError::Store => self.fail_code(SETUP, &[], EXIT_AI),
        }
    }

    fn turn_failure(&mut self, error: TurnError) -> u8 {
        let key = match error {
            TurnError::Provider(ai) => return self.ai_failure(&ai),
            TurnError::NotFound => "cli.ask_err.not_found",
            TurnError::Busy => "cli.ask_err.busy",
            TurnError::Archived => "cli.ask_err.archived",
            TurnError::EmptyMessage => "cli.ask_err.empty_message",
            TurnError::MessageTooLarge => "cli.ask_err.too_large",
            TurnError::BadMessage => "cli.ask_err.bad_message",
            TurnError::ClaimLost => "cli.ask_err.claim_lost",
            TurnError::BudgetExceeded { .. } => "cli.ask_err.budget",
            TurnError::NoAnswer => "cli.ask_err.no_answer",
            TurnError::Store => "cli.ask_err.store",
        };
        self.fail_code(MessageKey::new(key), &[], EXIT_AI)
    }

    fn ai_failure(&mut self, error: &AiError) -> u8 {
        let status;
        let seconds;
        let (key, args): (&str, Vec<(&str, &str)>) = match error {
            AiError::Auth => ("cli.ask_err.auth", vec![]),
            AiError::Quota => ("cli.ask_err.quota", vec![]),
            AiError::RateLimited { retry_after_secs } => match retry_after_secs {
                Some(secs) => {
                    seconds = secs.to_string();
                    ("cli.ask_err.rate", vec![("seconds", seconds.as_str())])
                }
                None => ("cli.ask_err.rate_unknown", vec![]),
            },
            AiError::CircuitOpen => ("cli.ask_err.circuit", vec![]),
            AiError::Unsupported(why) => ("cli.ask_err.unsupported", vec![("why", *why)]),
            AiError::Empty => ("cli.ask_err.empty", vec![]),
            AiError::Config(why) => ("cli.ask_err.config", vec![("why", *why)]),
            AiError::Transport(_) => ("cli.ask_err.transport", vec![]),
            AiError::Server(code) => {
                status = code.to_string();
                ("cli.ask_err.server", vec![("status", status.as_str())])
            }
            AiError::Rejected(code) => {
                status = code.to_string();
                ("cli.ask_err.rejected", vec![("status", status.as_str())])
            }
            AiError::Malformed(why) => ("cli.ask_err.malformed", vec![("why", *why)]),
            other => ("cli.ask_err.other", vec![("code", other.code())]),
        };
        self.fail_code(MessageKey::new(key), &args, EXIT_AI)
    }
}
