//! The tier 0 tools of M2-b: they read what the org's collectors and parity rules produced and
//! change nothing. All wording a person might read comes from the i18n catalog (through
//! `parity::render`); the rest is machine-oriented text for the model.

use crate::registry::{RegisterError, Registry};
use crate::schema::{ArgSchema, Args, Field, Kind};
use crate::store::StoreError;
use crate::tool::{AiTool, ToolContext, ToolError, ToolOutput, ToolSpec};
use core_domain::tier::Tier;
use inventory::{Finding, Inventory, Kind as ItemKind, Severity, Status};
use std::fmt::Write as _;

const NODE_FIELD: Field = Field {
    name: "node",
    kind: Kind::Text {
        max_chars: 200,
        choices: None,
    },
    required: true,
    description: "Node name, as returned by list_nodes.",
};

const KIND_CHOICES: &[&str] = &["listener", "process", "cron", "env_ref"];
const SEVERITY_CHOICES: &[&str] = &["info", "important", "critical"];

fn map_store(error: StoreError) -> ToolError {
    match error {
        StoreError::NotFound => ToolError::NotFound,
        StoreError::Unavailable => ToolError::Unavailable("store".to_owned()),
    }
}

fn parse_severity(text: &str) -> Option<Severity> {
    match text {
        "info" => Some(Severity::Info),
        "important" => Some(Severity::Important),
        "critical" => Some(Severity::Critical),
        _ => None,
    }
}

const fn severity_word(severity: Severity) -> &'static str {
    match severity {
        Severity::Info => "info",
        Severity::Important => "important",
        Severity::Critical => "critical",
    }
}

fn limit_of(args: &Args, default: u32) -> usize {
    args.int("limit")
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(default as usize)
}

// ---- list_nodes ------------------------------------------------------------------------------

struct ListNodes;

const LIST_NODES: ToolSpec = ToolSpec {
    name: "list_nodes",
    description: "List the servers (nodes) of this organization with kind and status.",
    tier: Tier::Read,
    schema: ArgSchema::NONE,
    impact: None,
};

impl AiTool for ListNodes {
    fn spec(&self) -> &ToolSpec {
        &LIST_NODES
    }

    fn run(&self, ctx: &ToolContext<'_>, _args: &Args) -> Result<ToolOutput, ToolError> {
        let nodes = ctx.store.nodes(ctx.org).map_err(map_store)?;
        if nodes.is_empty() {
            return Ok(ToolOutput::new("no nodes"));
        }
        let mut out = String::from("name | kind | status\n");
        for n in nodes {
            let _ = writeln!(out, "{} | {} | {}", n.name, n.kind, n.status);
        }
        Ok(ToolOutput::new(out))
    }
}

// ---- get_inventory_summary -------------------------------------------------------------------

struct GetInventorySummary;

const GET_INVENTORY_SUMMARY: ToolSpec = ToolSpec {
    name: "get_inventory_summary",
    description: "Summarize the latest inventory of a node: when it was taken, whether it is \
                  complete, and how many items of each kind it holds.",
    tier: Tier::Read,
    schema: ArgSchema {
        fields: &[NODE_FIELD],
    },
    impact: None,
};

impl AiTool for GetInventorySummary {
    fn spec(&self) -> &ToolSpec {
        &GET_INVENTORY_SUMMARY
    }

    fn run(&self, ctx: &ToolContext<'_>, args: &Args) -> Result<ToolOutput, ToolError> {
        let node = args.text("node").ok_or(ToolError::NotFound)?;
        let inventory = ctx
            .store
            .latest_inventory(ctx.org, node)
            .map_err(map_store)?;
        Ok(ToolOutput::new(summarize(&inventory)))
    }
}

fn summarize(inventory: &Inventory) -> String {
    let mut out = format!(
        "taken_at: {}\nstatus: {}\nplatform: {}/{}\ncollector: {}\n",
        inventory.taken_at,
        match inventory.status {
            Status::Complete => "complete",
            Status::Partial => "partial",
        },
        inventory.platform.os,
        inventory.platform.arch,
        inventory.collector_version,
    );
    for kind in [
        ItemKind::Listener,
        ItemKind::Process,
        ItemKind::Cron,
        ItemKind::EnvRef,
    ] {
        let _ = writeln!(
            out,
            "{}: {}",
            kind.as_str(),
            inventory.items_of(kind).count()
        );
    }
    for warning in &inventory.warnings {
        let _ = writeln!(out, "warning: {warning}");
    }
    out
}

// ---- list_inventory_items --------------------------------------------------------------------

struct ListInventoryItems;

const ITEM_FIELDS: &[Field] = &[
    NODE_FIELD,
    Field {
        name: "kind",
        kind: Kind::Text {
            max_chars: 20,
            choices: Some(KIND_CHOICES),
        },
        required: false,
        description: "Only items of this kind.",
    },
    Field {
        name: "limit",
        kind: Kind::Int { min: 1, max: 100 },
        required: false,
        description: "Maximum items, default 50.",
    },
];

const LIST_INVENTORY_ITEMS: ToolSpec = ToolSpec {
    name: "list_inventory_items",
    description: "List items (listening ports, processes, cron jobs, env references) from the \
                  latest inventory of a node.",
    tier: Tier::Read,
    schema: ArgSchema {
        fields: ITEM_FIELDS,
    },
    impact: None,
};

impl AiTool for ListInventoryItems {
    fn spec(&self) -> &ToolSpec {
        &LIST_INVENTORY_ITEMS
    }

    fn run(&self, ctx: &ToolContext<'_>, args: &Args) -> Result<ToolOutput, ToolError> {
        let node = args.text("node").ok_or(ToolError::NotFound)?;
        let inventory = ctx
            .store
            .latest_inventory(ctx.org, node)
            .map_err(map_store)?;
        let wanted = args.text("kind");
        let limit = limit_of(args, 50);
        let mut out = String::new();
        let mut shown = 0;
        let mut total = 0;
        for item in inventory
            .items()
            .iter()
            .filter(|i| wanted.is_none_or(|k| i.kind.as_str() == k))
        {
            total += 1;
            if shown < limit {
                let data = serde_json::Value::Object(item.data.clone()).to_string();
                let data: String = data.chars().take(300).collect();
                let _ = writeln!(out, "{} {} {}", item.kind.as_str(), item.key, data);
                shown += 1;
            }
        }
        if total == 0 {
            return Ok(ToolOutput::new("no items"));
        }
        if total > shown {
            let _ = writeln!(out, "[{} more not shown; use kind or limit]", total - shown);
        }
        Ok(ToolOutput::new(out))
    }
}

// ---- list_findings ---------------------------------------------------------------------------

struct ListFindings;

const FINDING_FIELDS: &[Field] = &[
    Field {
        name: "min_severity",
        kind: Kind::Text {
            max_chars: 20,
            choices: Some(SEVERITY_CHOICES),
        },
        required: false,
        description: "Only findings at or above this severity.",
    },
    Field {
        name: "limit",
        kind: Kind::Int { min: 1, max: 50 },
        required: false,
        description: "Maximum findings, default 20.",
    },
];

const LIST_FINDINGS: ToolSpec = ToolSpec {
    name: "list_findings",
    description: "List the parity findings of the latest migration comparison, most severe first: \
                  what the new server is missing compared with the old one.",
    tier: Tier::Read,
    schema: ArgSchema {
        fields: FINDING_FIELDS,
    },
    impact: None,
};

impl AiTool for ListFindings {
    fn spec(&self) -> &ToolSpec {
        &LIST_FINDINGS
    }

    fn run(&self, ctx: &ToolContext<'_>, args: &Args) -> Result<ToolOutput, ToolError> {
        let min = args.text("min_severity").and_then(parse_severity);
        let limit = u32::try_from(limit_of(args, 20)).unwrap_or(20);
        let view = ctx
            .store
            .latest_findings(ctx.org, min, limit)
            .map_err(map_store)?;
        let mut out = format!(
            "comparison: {} -> {} at {}\n",
            view.source_node, view.target_node, view.created_at
        );
        if view.findings.is_empty() {
            out.push_str("no findings\n");
        }
        for f in &view.findings {
            let title = parity::render(f, ctx.catalog, ctx.locale).title;
            let _ = writeln!(
                out,
                "[{}] {} {}:{} {}",
                severity_word(f.severity),
                f.rule_id,
                f.subject.kind.as_str(),
                f.subject.key,
                title
            );
        }
        Ok(ToolOutput::new(out))
    }
}

// ---- explain_finding -------------------------------------------------------------------------

struct ExplainFinding;

const EXPLAIN_FIELDS: &[Field] = &[
    Field {
        name: "rule_id",
        kind: Kind::Text {
            max_chars: 20,
            choices: None,
        },
        required: true,
        description: "Rule id, e.g. PAR-001.",
    },
    Field {
        name: "subject_key",
        kind: Kind::Text {
            max_chars: 500,
            choices: None,
        },
        required: true,
        description: "Key of the item the finding is about, as shown by list_findings.",
    },
];

const EXPLAIN_FINDING: ToolSpec = ToolSpec {
    name: "explain_finding",
    description: "Explain one finding of the latest comparison: what it means, the evidence and \
                  what to do about it.",
    tier: Tier::Read,
    schema: ArgSchema {
        fields: EXPLAIN_FIELDS,
    },
    impact: None,
};

impl AiTool for ExplainFinding {
    fn spec(&self) -> &ToolSpec {
        &EXPLAIN_FINDING
    }

    fn run(&self, ctx: &ToolContext<'_>, args: &Args) -> Result<ToolOutput, ToolError> {
        let (rule, key) = (
            args.text("rule_id").ok_or(ToolError::NotFound)?,
            args.text("subject_key").ok_or(ToolError::NotFound)?,
        );
        let view = ctx
            .store
            .latest_findings(ctx.org, None, 1000)
            .map_err(map_store)?;
        let finding: &Finding = view
            .findings
            .iter()
            .find(|f| f.rule_id == rule && f.subject.key == key)
            .ok_or(ToolError::NotFound)?;
        let text = parity::render(finding, ctx.catalog, ctx.locale);
        let mut out = format!(
            "severity: {}\ntitle: {}\nexplanation: {}\nadvice: {}\n",
            text.severity_label, text.title, text.explanation, text.advice
        );
        if let Some(note) = text.note {
            let _ = writeln!(out, "note: {note}");
        }
        for e in &finding.evidence {
            let _ = writeln!(out, "evidence: {}:{}", e.kind.as_str(), e.key);
        }
        Ok(ToolOutput::new(out))
    }
}

/// The registry M2 offers a model: every tool is tier 0, and nothing above tier 1 can be added.
///
/// # Errors
/// [`RegisterError`] if a built-in tool breaks a registry rule (a bug caught by a unit test).
pub fn read_only_registry() -> Result<Registry, RegisterError> {
    let mut registry = Registry::new(Tier::Plan);
    registry.register(Box::new(ListNodes))?;
    registry.register(Box::new(GetInventorySummary))?;
    registry.register(Box::new(ListInventoryItems))?;
    registry.register(Box::new(ListFindings))?;
    registry.register(Box::new(ExplainFinding))?;
    Ok(registry)
}
