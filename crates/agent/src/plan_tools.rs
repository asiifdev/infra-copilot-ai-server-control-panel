//! The planning tools (M3-c, ADR 0027): a model may *draft* a migration plan and read plans,
//! transports and gates. It cannot approve, schedule or run anything: the only write is one new
//! plan with status `draft` (tier 1), and no tool above tier 1 exists.
//!
//! Names, not ids: the model names nodes and transports; the tool resolves them inside the
//! caller's org (another org's names behave exactly like missing ones). Plans are built by
//! [`plan::draft_plan`] from the latest inventories and the findings of the latest comparison, so
//! the sequence and the safety rules are the `plan` crate's, never the model's. Everything a
//! person reads comes from the plan renderer and the i18n catalog; the model-facing lines are
//! machine text. Strings that came from a server (process names, keys) appear only inside the
//! executor's untrusted-data fence, like every tool result.

use crate::plan_flow::{
    DraftContext, DraftRequest, Drafted, Evidence, NAME_MAX, draft_migration_plan, gates_for_stored,
};
use crate::plans::StoredPlan;
use crate::registry::{RegisterError, Registry};
use crate::schema::{ArgSchema, Args, Field, Kind};
use crate::store::StoreError;
use crate::tool::{AiTool, ToolContext, ToolError, ToolOutput, ToolSpec};
use core_domain::tier::Tier;
use i18n::MessageKey;
use plan::{BackupLocation, CutoverMethod, GateReport, render_draft, render_plan};
use std::fmt::Write as _;

const LOOKUP_MAX: usize = 200;

const SOURCE_FIELD: Field = Field {
    name: "source_node",
    kind: Kind::Text {
        max_chars: LOOKUP_MAX,
        choices: None,
    },
    required: true,
    description: "Name of the node the data comes from, as returned by list_nodes.",
};

const TARGET_FIELD: Field = Field {
    name: "target_node",
    kind: Kind::Text {
        max_chars: LOOKUP_MAX,
        choices: None,
    },
    required: true,
    description: "Name of the node the data goes to, as returned by list_nodes.",
};

const PLAN_NAME_FIELD: Field = Field {
    name: "name",
    kind: Kind::Text {
        max_chars: LOOKUP_MAX,
        choices: None,
    },
    required: true,
    description: "Plan name, as returned by list_plans.",
};

fn map_store(error: StoreError) -> ToolError {
    match error {
        StoreError::NotFound => ToolError::NotFound,
        StoreError::Unavailable => ToolError::Unavailable("store".to_owned()),
    }
}

fn gates_of(
    ctx: &ToolContext<'_>,
    stored: &StoredPlan,
) -> Result<(GateReport, Evidence), ToolError> {
    gates_for_stored(ctx.plans, ctx.catalog, ctx.locale, ctx.org, stored)
        .map_err(|_| ToolError::Unavailable("plans".to_owned()))
}

fn yes(flag: bool) -> &'static str {
    if flag { "yes" } else { "no" }
}

// ---- draft_migration_plan --------------------------------------------------------------------

struct DraftMigrationPlan;

const DRAFT_NAME: &str = "draft_migration_plan";
const DRAFT_IMPACT: &str = "assistant.plan.impact";
const LOCATION_CHOICES: &[&str] = &["source", "local", "intermediary"];
const METHOD_CHOICES: &[&str] = &["tunnel", "dns"];

const DRAFT_FIELDS: &[Field] = &[
    SOURCE_FIELD,
    TARGET_FIELD,
    Field {
        name: "transport",
        kind: Kind::Text {
            max_chars: LOOKUP_MAX,
            choices: None,
        },
        required: false,
        description: "Name of a verified transport, as returned by list_transports. Optional \
                      when exactly one verified transport exists.",
    },
    Field {
        name: "backup_location",
        kind: Kind::Text {
            max_chars: 20,
            choices: Some(LOCATION_CHOICES),
        },
        required: false,
        description: "Where the backup is kept; default source.",
    },
    Field {
        name: "cutover_method",
        kind: Kind::Text {
            max_chars: 10,
            choices: Some(METHOD_CHOICES),
        },
        required: false,
        description: "How traffic moves; default tunnel.",
    },
    Field {
        name: "name",
        kind: Kind::Text {
            max_chars: NAME_MAX,
            choices: None,
        },
        required: false,
        description: "Name of the plan; default is the two node names.",
    },
];

const DRAFT_SPEC: ToolSpec = ToolSpec {
    name: DRAFT_NAME,
    description: "Draft a migration plan between two nodes from their latest inventories and the \
                  latest comparison. It only saves a plan with status draft for a person to \
                  review; nothing is approved, scheduled or run on any server.",
    tier: Tier::Plan,
    schema: ArgSchema {
        fields: DRAFT_FIELDS,
    },
    impact: Some(DRAFT_IMPACT),
};

fn parse_location(text: &str) -> Option<BackupLocation> {
    match text {
        "source" => Some(BackupLocation::Source),
        "local" => Some(BackupLocation::Local),
        "intermediary" => Some(BackupLocation::Intermediary),
        _ => None,
    }
}

fn parse_method(text: &str) -> Option<CutoverMethod> {
    match text {
        "tunnel" => Some(CutoverMethod::Tunnel),
        "dns" => Some(CutoverMethod::Dns),
        _ => None,
    }
}

impl AiTool for DraftMigrationPlan {
    fn spec(&self) -> &ToolSpec {
        &DRAFT_SPEC
    }

    fn run(&self, ctx: &ToolContext<'_>, args: &Args) -> Result<ToolOutput, ToolError> {
        let (Some(source), Some(target)) = (args.text("source_node"), args.text("target_node"))
        else {
            return Err(ToolError::Invalid("missing_node"));
        };
        let draft_ctx = DraftContext {
            org: ctx.org,
            store: ctx.store,
            plans: ctx.plans,
            sink: ctx.plan_sink,
            catalog: ctx.catalog,
            locale: ctx.locale,
        };
        let request = DraftRequest {
            source,
            target,
            transport: args.text("transport"),
            backup_location: args.text("backup_location").and_then(parse_location),
            cutover_method: args.text("cutover_method").and_then(parse_method),
            name: args.text("name"),
        };
        let Drafted {
            id,
            name,
            transport_name,
            evidence,
            draft,
        } = draft_migration_plan(&draft_ctx, &request)
            .map_err(|e| e.into_tool_error(ctx.catalog, ctx.locale))?;

        let saved = ctx
            .catalog
            .get(ctx.locale, MessageKey::new("assistant.plan.saved"), &[])
            .text;
        let mut out = format!(
            "plan: {name}\nid: {id}\nstatus: draft\nsource: {source}\ntarget: {target}\n\
             transport: {transport_name}\ncomparison: {}\ncutover_allowed: {}\n{saved}\n\n",
            evidence.word(),
            yes(draft.gates.cutover_allowed()),
        );
        out.push_str(&render_draft(&draft, ctx.catalog, ctx.locale));
        Ok(ToolOutput::new(out))
    }
}

// ---- show_plan -------------------------------------------------------------------------------

struct ShowPlan;

const SHOW_SPEC: ToolSpec = ToolSpec {
    name: "show_plan",
    description: "Show one migration plan written out for a person: steps, what is left out of \
                  the backup, and the gates before the old server may be shut down.",
    tier: Tier::Read,
    schema: ArgSchema {
        fields: &[PLAN_NAME_FIELD],
    },
    impact: None,
};

impl AiTool for ShowPlan {
    fn spec(&self) -> &ToolSpec {
        &SHOW_SPEC
    }

    fn run(&self, ctx: &ToolContext<'_>, args: &Args) -> Result<ToolOutput, ToolError> {
        let name = args.text("name").ok_or(ToolError::NotFound)?;
        let stored = ctx.plans.plan(ctx.org, name).map_err(map_store)?;
        let (gates, evidence) = gates_of(ctx, &stored)?;
        let mut out = format!(
            "plan: {}\nstatus: {}\nsource: {}\ntarget: {}\ncreated_at: {}\ncomparison: {}\n\n",
            stored.summary.name,
            stored.summary.status,
            stored.summary.source_node,
            stored.summary.target_node,
            stored.summary.created_at,
            evidence.word(),
        );
        out.push_str(&render_plan(&stored.plan, &gates, ctx.catalog, ctx.locale));
        Ok(ToolOutput::new(out))
    }
}

// ---- list_plans ------------------------------------------------------------------------------

struct ListPlans;

const LIST_PLANS_SPEC: ToolSpec = ToolSpec {
    name: "list_plans",
    description: "List the migration plans of this organization with status and nodes.",
    tier: Tier::Read,
    schema: ArgSchema::NONE,
    impact: None,
};

impl AiTool for ListPlans {
    fn spec(&self) -> &ToolSpec {
        &LIST_PLANS_SPEC
    }

    fn run(&self, ctx: &ToolContext<'_>, _args: &Args) -> Result<ToolOutput, ToolError> {
        let plans = ctx.plans.plans(ctx.org).map_err(map_store)?;
        if plans.is_empty() {
            return Ok(ToolOutput::new("no plans"));
        }
        let mut out = String::from("name | status | source -> target | created_at\n");
        for p in plans {
            let _ = writeln!(
                out,
                "{} | {} | {} -> {} | {}",
                p.name, p.status, p.source_node, p.target_node, p.created_at
            );
        }
        Ok(ToolOutput::new(out))
    }
}

// ---- check_plan_gates ------------------------------------------------------------------------

struct CheckPlanGates;

const CHECK_SPEC: ToolSpec = ToolSpec {
    name: "check_plan_gates",
    description: "Check where a plan stands against its gates: open critical findings, accepted \
                  ones, and whether the evidence is complete. Cutover and the shutdown of the \
                  old server stay blocked until the gates hold.",
    tier: Tier::Read,
    schema: ArgSchema {
        fields: &[PLAN_NAME_FIELD],
    },
    impact: None,
};

impl AiTool for CheckPlanGates {
    fn spec(&self) -> &ToolSpec {
        &CHECK_SPEC
    }

    fn run(&self, ctx: &ToolContext<'_>, args: &Args) -> Result<ToolOutput, ToolError> {
        let name = args.text("name").ok_or(ToolError::NotFound)?;
        let stored = ctx.plans.plan(ctx.org, name).map_err(map_store)?;
        let (gates, evidence) = gates_of(ctx, &stored)?;
        let mut out = format!(
            "plan: {}\nstatus: {}\ncomparison: {}\ncutover_allowed: {}\n\
             shutdown_gate_open: {}\ninventory_incomplete: {}\nopen_critical: {}\n",
            stored.summary.name,
            stored.summary.status,
            evidence.word(),
            yes(gates.cutover_allowed()),
            // Nothing is verified after cutover in M3, so the shutdown gate can never be open here.
            yes(gates.decommission_allowed(false)),
            yes(gates.inventory_incomplete),
            gates.open_critical.len(),
        );
        for key in &gates.open_critical {
            let _ = writeln!(
                out,
                "- {} {}:{}",
                key.rule_id,
                key.subject.kind.as_str(),
                key.subject.key
            );
        }
        let _ = writeln!(out, "accepted_critical: {}", gates.accepted_critical.len());
        for accepted in &gates.accepted_critical {
            let _ = writeln!(
                out,
                "- {} {}:{}",
                accepted.key.rule_id,
                accepted.key.subject.kind.as_str(),
                accepted.key.subject.key
            );
        }
        let _ = writeln!(out, "important: {}\ninfo: {}", gates.important, gates.info);
        Ok(ToolOutput::new(out))
    }
}

// ---- list_transports -------------------------------------------------------------------------

struct ListTransports;

const LIST_TRANSPORTS_SPEC: ToolSpec = ToolSpec {
    name: "list_transports",
    description: "List the transports (ways to move a backup) of this organization with kind and \
                  last verification. Settings and credentials are never shown.",
    tier: Tier::Read,
    schema: ArgSchema::NONE,
    impact: None,
};

impl AiTool for ListTransports {
    fn spec(&self) -> &ToolSpec {
        &LIST_TRANSPORTS_SPEC
    }

    fn run(&self, ctx: &ToolContext<'_>, _args: &Args) -> Result<ToolOutput, ToolError> {
        let transports = ctx.plans.transports(ctx.org).map_err(map_store)?;
        if transports.is_empty() {
            return Ok(ToolOutput::new("no transports"));
        }
        let mut out = String::from("name | kind | verified_at\n");
        for t in transports {
            let _ = writeln!(
                out,
                "{} | {} | {}",
                t.name,
                t.kind,
                t.verified_at.as_deref().unwrap_or("never")
            );
        }
        Ok(ToolOutput::new(out))
    }
}

/// Registers the planning tools: one tier 1 (draft) and four tier 0.
///
/// # Errors
/// [`RegisterError`] if a tool breaks a registry rule (a bug caught by a unit test).
pub(crate) fn register(registry: &mut Registry) -> Result<(), RegisterError> {
    registry.register(Box::new(DraftMigrationPlan))?;
    registry.register(Box::new(ShowPlan))?;
    registry.register(Box::new(ListPlans))?;
    registry.register(Box::new(CheckPlanGates))?;
    registry.register(Box::new(ListTransports))?;
    Ok(())
}
