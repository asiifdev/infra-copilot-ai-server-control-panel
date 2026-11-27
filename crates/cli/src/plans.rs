//! `infra plan draft|list|show|gates` (ADR 0028).
//!
//! A plan made here is only ever a **draft**: the drafting code is the one the AI tool
//! `draft_migration_plan` uses ([`agent::draft_migration_plan`]), and there is no approve, schedule
//! or run command (approval and execution are milestone M4, for a person in the panel). Names are
//! resolved inside the organization, so another organization's plans, nodes and transports behave
//! like missing ones. Text from the database is untrusted: it goes through the catalog, which
//! replaces control characters.

use crate::Ui;
use crate::args::{Globals, Parsed, parse};
use agent::{
    DbPlanSink, DbPlanStore, DbStore, DefaultPlanStepRunner, DraftContext, DraftError,
    DraftRequest, Drafted, PlanRunError, PlanStore, StoreError, approve_plan, gates_for_stored,
    run_plan,
};
use i18n::MessageKey;
use plan::{BackupLocation, CutoverMethod, GateReport, render_draft, render_plan};

const DRAFTED: MessageKey = MessageKey::new("cli.plan.drafted");
const DRAFT_NOTE: MessageKey = MessageKey::new("cli.plan.draft_note");
const COMPARISON_CURRENT: MessageKey = MessageKey::new("cli.plan.comparison_current");
const COMPARISON_STALE: MessageKey = MessageKey::new("cli.plan.comparison_stale");
const UNKNOWN: MessageKey = MessageKey::new("cli.plan.unknown");
const LIST_EMPTY: MessageKey = MessageKey::new("cli.plan.list_empty");
const LIST_HEAD: MessageKey = MessageKey::new("cli.plan.list_head");
const LIST_LINE: MessageKey = MessageKey::new("cli.plan.list_line");
const SHOW_HEAD: MessageKey = MessageKey::new("cli.plan.show_head");
const GATES_HEAD: MessageKey = MessageKey::new("cli.plan.gates_head");
const GATES_CUTOVER: MessageKey = MessageKey::new("cli.plan.gates_cutover");
const GATES_COMPLETE: MessageKey = MessageKey::new("cli.plan.gates_complete");
const GATES_SHUTDOWN: MessageKey = MessageKey::new("cli.plan.gates_shutdown");
const GATES_OPEN: MessageKey = MessageKey::new("cli.plan.gates_open");
const GATES_ACCEPTED: MessageKey = MessageKey::new("cli.plan.gates_accepted");
const GATES_FINDING: MessageKey = MessageKey::new("cli.plan.gates_finding");
const GATES_COUNTS: MessageKey = MessageKey::new("cli.plan.gates_counts");
const ERR_SAME_NODE: MessageKey = MessageKey::new("cli.plan.err_same_node");
const ERR_TRANSPORT_REQUIRED: MessageKey = MessageKey::new("cli.plan.err_transport_required");
const ERR_TRANSPORT_MISSING: MessageKey = MessageKey::new("cli.plan.err_transport_missing");
const ERR_TRANSPORT_UNVERIFIED: MessageKey = MessageKey::new("cli.plan.err_transport_unverified");
const ERR_NAME: MessageKey = MessageKey::new("cli.plan.err_name");
const ERR_REJECTED: MessageKey = MessageKey::new("cli.plan.err_rejected");
const ERR_NAME_TAKEN: MessageKey = MessageKey::new("cli.plan.err_name_taken");
const ERR_TOO_MANY: MessageKey = MessageKey::new("cli.plan.err_too_many");
const NODE_UNKNOWN: MessageKey = MessageKey::new("cli.collect.node_unknown");
const NO_INVENTORY: MessageKey = MessageKey::new("cli.compare.no_inventory");
const WORD_YES: MessageKey = MessageKey::new("cli.word.yes");
const WORD_NO: MessageKey = MessageKey::new("cli.word.no");
const APPROVED: MessageKey = MessageKey::new("cli.plan.approved");
const ERR_NOT_APPROVABLE: MessageKey = MessageKey::new("cli.plan.err_not_approvable");
const ERR_NOT_APPROVED: MessageKey = MessageKey::new("cli.plan.err_not_approved");
const ERR_GATE_BLOCKED: MessageKey = MessageKey::new("cli.plan.err_gate_blocked");
const RUN_DRY: MessageKey = MessageKey::new("cli.plan.run_dry");
const RUN_DONE: MessageKey = MessageKey::new("cli.plan.run_done");
const RUN_STEP: MessageKey = MessageKey::new("cli.plan.run_step");

/// Cuts `--name=value` down to `--name`: a usage message must never echo a value that might be a
/// secret typed in the wrong form.
pub(crate) fn safe_detail(detail: &str) -> String {
    if detail.starts_with("--") {
        detail.split('=').next().unwrap_or_default().to_owned()
    } else {
        detail.to_owned()
    }
}

impl Ui<'_> {
    pub(crate) fn plan_command(&mut self, globals: &Globals, rest: &[String]) -> u8 {
        let usage = Some(crate::USAGE_PLAN);
        let Some((sub, args)) = rest.split_first() else {
            return self.usage("-", usage);
        };
        let parsed = match sub.as_str() {
            "draft" => parse(
                args,
                &[
                    "--source",
                    "--target",
                    "--transport",
                    "--backup-location",
                    "--cutover",
                    "--name",
                    "--lang",
                ],
                &[],
            ),
            "list" | "show" | "gates" | "approve" => parse(args, &["--lang"], &[]),
            "run" => parse(args, &["--lang"], &["--enable-execution"]),
            other => return self.usage(&safe_detail(other), usage),
        };
        let parsed = match parsed {
            Ok(parsed) => parsed,
            Err(detail) => return self.usage(&safe_detail(&detail), usage),
        };
        if !self.apply_lang(&parsed) {
            return self.usage("--lang", usage);
        }
        match sub.as_str() {
            "draft" => self.plan_draft(globals, &parsed),
            "list" => {
                if parsed.positional.is_empty() {
                    self.plan_list(globals)
                } else {
                    self.usage("-", usage)
                }
            }
            "show" | "gates" => match parsed.positional.as_slice() {
                [name] if sub == "show" => self.plan_show(globals, name),
                [name] => self.plan_gates(globals, name),
                _ => self.usage("-", usage),
            },
            "approve" => match parsed.positional.as_slice() {
                [target] => self.plan_approve(globals, target),
                _ => self.usage("-", usage),
            },
            "run" => match parsed.positional.as_slice() {
                [target] => self.plan_run(globals, target, !parsed.flag("--enable-execution")),
                _ => self.usage("-", usage),
            },
            _ => self.usage("-", usage),
        }
    }

    fn plan_draft(&mut self, globals: &Globals, parsed: &Parsed) -> u8 {
        let usage = Some(crate::USAGE_PLAN);
        let (Some(source), Some(target)) = (parsed.option("--source"), parsed.option("--target"))
        else {
            return self.usage("-", usage);
        };
        if !parsed.positional.is_empty() {
            return self.usage("-", usage);
        }
        let backup_location = match parsed.option("--backup-location") {
            None => None,
            Some("source") => Some(BackupLocation::Source),
            Some("local") => Some(BackupLocation::Local),
            Some("intermediary") => Some(BackupLocation::Intermediary),
            Some(_) => return self.usage("--backup-location", usage),
        };
        let cutover_method = match parsed.option("--cutover") {
            None => None,
            Some("tunnel") => Some(CutoverMethod::Tunnel),
            Some("dns") => Some(CutoverMethod::Dns),
            Some(_) => return self.usage("--cutover", usage),
        };
        let (conn, _) = match self.open_db(globals, false) {
            Ok(opened) => opened,
            Err(code) => return code,
        };
        let scope = match self.scope(&conn, globals) {
            Ok(scope) => scope,
            Err(code) => return code,
        };
        let (store, plans, sink) = (
            DbStore::new(&conn),
            DbPlanStore::new(&conn),
            DbPlanSink::new(&conn),
        );
        let ctx = DraftContext {
            org: &scope.org,
            store: &store,
            plans: &plans,
            sink: &sink,
            catalog: &self.catalog,
            locale: self.locale,
        };
        let request = DraftRequest {
            source,
            target,
            transport: parsed.option("--transport"),
            backup_location,
            cutover_method,
            name: parsed.option("--name"),
        };
        match agent::draft_migration_plan(&ctx, &request) {
            Ok(drafted) => self.print_drafted(&drafted, source, target),
            Err(error) => self.draft_failed(error, &request),
        }
    }

    fn print_drafted(&mut self, drafted: &Drafted, source: &str, target: &str) -> u8 {
        let mut text = self.tr(DRAFTED, &[("name", &drafted.name), ("id", &drafted.id)]);
        text.push('\n');
        text.push_str(&self.tr(DRAFT_NOTE, &[("name", &drafted.name)]));
        text.push('\n');
        text.push_str(&self.tr(
            if drafted.evidence.is_current() {
                COMPARISON_CURRENT
            } else {
                COMPARISON_STALE
            },
            &[("source", source), ("target", target)],
        ));
        text.push_str("\n\n");
        let nodes = plan::PlanNames::of_plan(&drafted.draft.plan, source, target).with(
            &drafted.draft.plan.spec().transport.transport_id,
            &drafted.transport_name,
        );
        text.push_str(&render_draft(
            &drafted.draft,
            &nodes,
            &self.catalog,
            self.locale,
        ));
        self.write_out(&text)
    }

    fn draft_failed(&mut self, error: DraftError, request: &DraftRequest<'_>) -> u8 {
        let name = request.name.unwrap_or("-");
        let transport = request.transport.unwrap_or("-");
        match error {
            DraftError::SameNode => self.fail(ERR_SAME_NODE, &[]),
            DraftError::SourceMissing | DraftError::NodeMissing => {
                self.fail(NODE_UNKNOWN, &[("node", request.source)])
            }
            DraftError::TargetMissing => self.fail(NODE_UNKNOWN, &[("node", request.target)]),
            DraftError::SourceNoInventory => self.fail(NO_INVENTORY, &[("node", request.source)]),
            DraftError::TargetNoInventory => self.fail(NO_INVENTORY, &[("node", request.target)]),
            DraftError::TransportRequired => self.fail(ERR_TRANSPORT_REQUIRED, &[]),
            DraftError::TransportMissing => {
                self.fail(ERR_TRANSPORT_MISSING, &[("name", transport)])
            }
            DraftError::TransportUnverified => {
                self.fail(ERR_TRANSPORT_UNVERIFIED, &[("name", transport)])
            }
            DraftError::Name(_) => self.fail(ERR_NAME, &[]),
            DraftError::Rejected => self.fail(ERR_REJECTED, &[]),
            DraftError::NameTaken => self.fail(ERR_NAME_TAKEN, &[("name", name)]),
            DraftError::TooManyDrafts => self.fail(ERR_TOO_MANY, &[]),
            DraftError::Unavailable(seam) => self.store_reason(seam),
        }
    }

    fn plan_list(&mut self, globals: &Globals) -> u8 {
        let (conn, _) = match self.open_db(globals, false) {
            Ok(opened) => opened,
            Err(code) => return code,
        };
        let scope = match self.scope(&conn, globals) {
            Ok(scope) => scope,
            Err(code) => return code,
        };
        let plans = match DbPlanStore::new(&conn).plans(&scope.org) {
            Ok(plans) => plans,
            Err(_) => return self.store_reason("plans_unavailable"),
        };
        if plans.is_empty() {
            return self.say_out(LIST_EMPTY, &[]);
        }
        let mut text = self.tr(LIST_HEAD, &[]);
        for p in &plans {
            text.push('\n');
            text.push_str(&self.tr(
                LIST_LINE,
                &[
                    ("name", &p.name),
                    ("status", p.status),
                    ("source", &p.source_node),
                    ("target", &p.target_node),
                    ("created", &p.created_at),
                ],
            ));
        }
        self.write_out(&text)
    }

    /// The stored plan `name` and its gates, or the exit code of the failure already reported.
    fn stored_with_gates(
        &mut self,
        globals: &Globals,
        name: &str,
    ) -> Result<(agent::StoredPlan, GateReport, agent::Evidence), u8> {
        let (conn, _) = self.open_db(globals, false)?;
        let scope = self.scope(&conn, globals)?;
        let plans = DbPlanStore::new(&conn);
        let stored = match plans.plan(&scope.org, name) {
            Ok(stored) => stored,
            Err(StoreError::NotFound) => return Err(self.fail(UNKNOWN, &[("name", name)])),
            Err(StoreError::Unavailable) => return Err(self.store_reason("plans_unavailable")),
        };
        match gates_for_stored(&plans, &self.catalog, self.locale, &scope.org, &stored) {
            Ok((gates, evidence)) => Ok((stored, gates, evidence)),
            Err(_) => Err(self.store_reason("plans_unavailable")),
        }
    }

    fn comparison_line(&self, evidence: &agent::Evidence, plan: &agent::StoredPlan) -> String {
        self.tr(
            if evidence.is_current() {
                COMPARISON_CURRENT
            } else {
                COMPARISON_STALE
            },
            &[
                ("source", &plan.summary.source_node),
                ("target", &plan.summary.target_node),
            ],
        )
    }

    fn plan_show(&mut self, globals: &Globals, name: &str) -> u8 {
        let (stored, gates, evidence) = match self.stored_with_gates(globals, name) {
            Ok(found) => found,
            Err(code) => return code,
        };
        let s = &stored.summary;
        let mut text = self.tr(
            SHOW_HEAD,
            &[
                ("name", &s.name),
                ("status", s.status),
                ("source", &s.source_node),
                ("target", &s.target_node),
                ("created", &s.created_at),
            ],
        );
        text.push('\n');
        text.push_str(&self.comparison_line(&evidence, &stored));
        text.push_str("\n\n");
        text.push_str(&render_plan(
            &stored.plan,
            &gates,
            &stored.plan_names(),
            &self.catalog,
            self.locale,
        ));
        self.write_out(&text)
    }

    fn yes_no(&self, flag: bool) -> String {
        self.tr(if flag { WORD_YES } else { WORD_NO }, &[])
    }

    /// Prints where the plan stands against its gates. Exit code 1 while cutover is blocked: an
    /// open critical finding, or evidence that is missing, stale or incomplete (an empty list of
    /// findings proves nothing then).
    fn plan_gates(&mut self, globals: &Globals, name: &str) -> u8 {
        let (stored, gates, evidence) = match self.stored_with_gates(globals, name) {
            Ok(found) => found,
            Err(code) => return code,
        };
        let s = &stored.summary;
        let mut text = self.tr(GATES_HEAD, &[("name", &s.name), ("status", s.status)]);
        text.push('\n');
        text.push_str(&self.comparison_line(&evidence, &stored));
        let cutover = self.yes_no(gates.cutover_allowed());
        let complete = self.yes_no(!gates.inventory_incomplete);
        // Nothing is verified after cutover in this milestone, so the shutdown gate is closed.
        let shutdown = self.yes_no(gates.decommission_allowed(false));
        text.push('\n');
        text.push_str(&self.tr(GATES_CUTOVER, &[("state", &cutover)]));
        text.push('\n');
        text.push_str(&self.tr(GATES_COMPLETE, &[("state", &complete)]));
        text.push('\n');
        text.push_str(&self.tr(GATES_SHUTDOWN, &[("state", &shutdown)]));
        text.push('\n');
        text.push_str(&self.tr(
            GATES_OPEN,
            &[("count", &gates.open_critical.len().to_string())],
        ));
        for key in &gates.open_critical {
            text.push('\n');
            text.push_str(&self.finding_line(&key.rule_id, key.subject.kind, &key.subject.key));
        }
        text.push('\n');
        text.push_str(&self.tr(
            GATES_ACCEPTED,
            &[("count", &gates.accepted_critical.len().to_string())],
        ));
        for accepted in &gates.accepted_critical {
            let key = &accepted.key;
            text.push('\n');
            text.push_str(&self.finding_line(&key.rule_id, key.subject.kind, &key.subject.key));
        }
        text.push('\n');
        text.push_str(&self.tr(
            GATES_COUNTS,
            &[
                ("important", &gates.important.to_string()),
                ("info", &gates.info.to_string()),
            ],
        ));
        let printed = self.write_out(&text);
        if printed != crate::EXIT_OK {
            return printed;
        }
        if gates.cutover_allowed() {
            crate::EXIT_OK
        } else {
            crate::EXIT_CRITICAL
        }
    }

    fn finding_line(&self, rule: &str, kind: inventory::Kind, key: &str) -> String {
        let kind = self.tr(crate::kind_key(kind), &[]);
        self.tr(
            GATES_FINDING,
            &[("rule", rule), ("kind", &kind), ("key", key)],
        )
    }

    fn plan_approve(&mut self, globals: &Globals, target: &str) -> u8 {
        let (conn, _) = match self.open_db(globals, false) {
            Ok(ok) => ok,
            Err(code) => return code,
        };
        let scope = match self.scope(&conn, globals) {
            Ok(scope) => scope,
            Err(code) => return code,
        };
        match approve_plan(
            &conn,
            &scope.org,
            target,
            db::ActorKind::User,
            Some(scope.user.as_str()),
        ) {
            Ok(stored) => {
                let msg = self.tr(APPROVED, &[("name", &stored.summary.name)]);
                self.write_out(&msg)
            }
            Err(PlanRunError::NotFound) => self.fail(UNKNOWN, &[("name", target)]),
            Err(PlanRunError::NotApprovable) => self.fail(ERR_NOT_APPROVABLE, &[("name", target)]),
            Err(_) => self.store_reason("plan_approve"),
        }
    }

    fn plan_run(&mut self, globals: &Globals, target: &str, dry_run: bool) -> u8 {
        let (conn, _) = match self.open_db(globals, false) {
            Ok(ok) => ok,
            Err(code) => return code,
        };
        let scope = match self.scope(&conn, globals) {
            Ok(scope) => scope,
            Err(code) => return code,
        };
        let runner = DefaultPlanStepRunner;
        match run_plan(
            &conn,
            &self.catalog,
            self.locale,
            &scope.org,
            target,
            dry_run,
            &runner,
            db::ActorKind::User,
            Some(scope.user.as_str()),
        ) {
            Ok(report) => {
                let mut text = if report.dry_run {
                    self.tr(RUN_DRY, &[("name", &report.name)])
                } else {
                    self.tr(
                        RUN_DONE,
                        &[("name", &report.name), ("status", report.status)],
                    )
                };
                for step in &report.steps {
                    text.push('\n');
                    text.push_str(&self.tr(
                        RUN_STEP,
                        &[
                            ("seq", &step.seq.to_string()),
                            ("kind", step.kind),
                            ("status", step.status),
                        ],
                    ));
                }
                self.write_out(&text)
            }
            Err(PlanRunError::NotFound) => self.fail(UNKNOWN, &[("name", target)]),
            Err(PlanRunError::NotApproved) => self.fail(ERR_NOT_APPROVED, &[("name", target)]),
            Err(PlanRunError::GateBlocked(refusal)) => self.fail(
                ERR_GATE_BLOCKED,
                &[("name", target), ("reason", &format!("{refusal:?}"))],
            ),
            Err(_) => self.store_reason("plan_run"),
        }
    }
}
