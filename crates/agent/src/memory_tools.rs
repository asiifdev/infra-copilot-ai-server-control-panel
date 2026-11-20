//! The M2-e tools: search over what the panel knows, and the assistant's memory (ADR 0026).
//!
//! * `search_inventory` and `list_memories` are tier 0 reads.
//! * `remember` is tier 1: it only creates a PENDING memory. A pending memory is never shown to a
//!   model and never searched; only a person can activate it, outside the model's reach.
//!
//! Wording for people comes from the i18n catalog; text returned to the model is machine-oriented.

use crate::memory::{MemoryDraft, MemoryError};
use crate::registry::{RegisterError, Registry};
use crate::schema::{ArgSchema, Args, Field, Kind};
use crate::tool::{AiTool, ToolContext, ToolError, ToolOutput, ToolSpec};
use core_domain::tier::Tier;
use db::{MemoryKind, SearchKind};
use std::fmt::Write as _;

/// Name of the tool that writes a memory.
pub(crate) const REMEMBER_NAME: &str = "remember";

/// Whether the arguments of this tool must not be stored in replayable history. What `remember`
/// was asked to store is a pending note: it must not reach a later prompt through the replayed
/// tool call before a person approves it.
pub(crate) fn hides_arguments(tool: &str) -> bool {
    tool == REMEMBER_NAME
}

/// Replaces control characters (a newline could forge a result line) with spaces.
fn one_line(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

const fn kind_word(kind: SearchKind) -> &'static str {
    match kind {
        SearchKind::Item => "item",
        SearchKind::Finding => "finding",
        SearchKind::Memory => "memory",
    }
}

fn map_memory(error: MemoryError) -> ToolError {
    match error {
        MemoryError::TooManyInConversation => {
            ToolError::Invalid("too_many_memories_in_conversation")
        }
        MemoryError::TooManyPending => ToolError::Invalid("too_many_pending_memories"),
        MemoryError::OrgFull => ToolError::Invalid("memory_limit_reached"),
        MemoryError::Invalid(why) => ToolError::Invalid(why),
        MemoryError::Unavailable => ToolError::Unavailable("memory".to_owned()),
    }
}

// ---- search_inventory ------------------------------------------------------------------------

struct SearchInventory;

const SEARCH_FIELDS: &[Field] = &[
    Field {
        name: "query",
        kind: Kind::Text {
            max_chars: 200,
            choices: None,
        },
        required: true,
        description: "Words to look for, e.g. a service name or a port number.",
    },
    Field {
        name: "limit",
        kind: Kind::Int { min: 1, max: 20 },
        required: false,
        description: "Maximum hits, default 10.",
    },
];

const SEARCH_INVENTORY: ToolSpec = ToolSpec {
    name: "search_inventory",
    description: "Search the latest inventory items, the latest findings and the approved \
                  organization notes by words. Returns ranked hits, each with a stable ref.",
    tier: Tier::Read,
    schema: ArgSchema {
        fields: SEARCH_FIELDS,
    },
    impact: None,
};

impl AiTool for SearchInventory {
    fn spec(&self) -> &ToolSpec {
        &SEARCH_INVENTORY
    }

    fn run(&self, ctx: &ToolContext<'_>, args: &Args) -> Result<ToolOutput, ToolError> {
        let query = args.text("query").ok_or(ToolError::NotFound)?;
        let limit = args
            .int("limit")
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(10);
        let hits = ctx
            .memory
            .search(ctx.org, query, limit)
            .map_err(map_memory)?;
        if hits.is_empty() {
            return Ok(ToolOutput::new("no hits"));
        }
        let mut out = String::from("ref | kind | node | title | text\n");
        for hit in hits {
            let _ = writeln!(
                out,
                "{} | {} | {} | {} | {}",
                one_line(&hit.reference),
                kind_word(hit.kind),
                one_line(hit.node.as_deref().unwrap_or("-")),
                one_line(&hit.title),
                one_line(&hit.snippet)
            );
        }
        Ok(ToolOutput::new(out))
    }
}

// ---- list_memories ---------------------------------------------------------------------------

struct ListMemories;

const LIST_MEMORIES: ToolSpec = ToolSpec {
    name: "list_memories",
    description: "List the approved organization notes, plus how many are still waiting for a \
                  person to approve them.",
    tier: Tier::Read,
    schema: ArgSchema::NONE,
    impact: None,
};

impl AiTool for ListMemories {
    fn spec(&self) -> &ToolSpec {
        &LIST_MEMORIES
    }

    fn run(&self, ctx: &ToolContext<'_>, _args: &Args) -> Result<ToolOutput, ToolError> {
        let notes = ctx.memory.active(ctx.org).map_err(map_memory)?;
        let pending = ctx.memory.pending_count(ctx.org).map_err(map_memory)?;
        let mut out = format!("active: {}\n", notes.len());
        for note in notes {
            let _ = writeln!(
                out,
                "{} | {} | {}",
                note.id,
                note.kind,
                one_line(&note.content)
            );
        }
        // Only the count: pending text is never shown to a model.
        let _ = writeln!(out, "pending_for_approval: {pending}");
        Ok(ToolOutput::new(out))
    }
}

// ---- remember --------------------------------------------------------------------------------

struct Remember;

const REMEMBER_IMPACT: &str = "assistant.memory.impact";
const MEMORY_KINDS: &[&str] = &["org_fact", "task", "preference"];

const REMEMBER_FIELDS: &[Field] = &[
    Field {
        name: "kind",
        kind: Kind::Text {
            max_chars: 20,
            choices: Some(MEMORY_KINDS),
        },
        required: true,
        description: "org_fact: a lasting fact; task: context of ongoing work; preference: how \
                      this person likes answers.",
    },
    Field {
        name: "content",
        kind: Kind::Text {
            max_chars: db::MAX_MEMORY_CHARS,
            choices: None,
        },
        required: true,
        description: "One short factual sentence. Never include secrets.",
    },
];

const REMEMBER: ToolSpec = ToolSpec {
    name: REMEMBER_NAME,
    description: "Propose a note to remember, only when the person asks you to. It is saved as \
                  pending and is not used until a person approves it; nothing else changes.",
    tier: Tier::Plan,
    schema: ArgSchema {
        fields: REMEMBER_FIELDS,
    },
    impact: Some(REMEMBER_IMPACT),
};

impl AiTool for Remember {
    fn spec(&self) -> &ToolSpec {
        &REMEMBER
    }

    fn run(&self, ctx: &ToolContext<'_>, args: &Args) -> Result<ToolOutput, ToolError> {
        let kind = args
            .text("kind")
            .and_then(|k| MemoryKind::parse(k).ok())
            .ok_or(ToolError::Invalid("bad_kind"))?;
        let raw = args
            .text("content")
            .ok_or(ToolError::Invalid("missing_text"))?;
        // Redact, strip control characters and check the length AFTER redaction (redaction can
        // lengthen text beyond what the schema allowed).
        let content = db::clean_memory_text(raw).map_err(|e| match e {
            db::Error::Invalid("memory_too_long") => ToolError::Invalid("text_too_long"),
            _ => ToolError::Invalid("empty_text"),
        })?;
        let id = ctx
            .memory
            .create_pending(
                ctx.org,
                &MemoryDraft {
                    kind,
                    content: &content,
                },
            )
            .map_err(map_memory)?;
        Ok(ToolOutput::new(format!(
            "memory {id} saved as pending. A person must approve it before it is used; nothing \
             else was changed."
        )))
    }
}

/// Registers the M2-e tools. Called from [`crate::assistant_registry`].
///
/// # Errors
/// [`RegisterError`] if a tool breaks a registry rule (a bug caught by a unit test).
pub(crate) fn register(registry: &mut Registry) -> Result<(), RegisterError> {
    registry.register(Box::new(SearchInventory))?;
    registry.register(Box::new(ListMemories))?;
    registry.register(Box::new(Remember))?;
    Ok(())
}
