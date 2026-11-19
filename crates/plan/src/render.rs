//! The written plan a person reads before anything runs (skill `migration-planning`): steps and
//! their impact, order, where to stop, what the time depends on, the conditions before the old
//! server may be shut down, what is left out of the backup, and the findings that block. All
//! wording comes from the i18n catalog; values taken from the plan, a server or a person are
//! cleaned before they are shown, because they are data and never markup or commands.

use core_domain::tier::Tier;
use i18n::{Catalog, Locale, MessageKey};

use crate::capacity::{CapacityAdvice, CapacityIssueKind, CapacityVerdict};
use crate::draft::{Draft, DraftWarning};
use crate::exclude::describe_key;
use crate::gates::GateReport;
use crate::plan::Plan;
use crate::spec::{BackupLocation, CutoverMethod, DbMode, Encryption};
use crate::steps::{Side, Step, StepKind};

/// Longest value shown from the plan or a finding, in characters.
const MAX_SHOWN: usize = 120;

/// Makes a value safe to print on one line: control characters (newlines, escape sequences) and
/// characters that reorder or hide text are replaced, and long values are cut.
fn clean(value: &str) -> String {
    let mut out: String = value
        .chars()
        .take(MAX_SHOWN)
        .map(|c| {
            let hidden = matches!(
                c,
                '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}'
            );
            if c.is_control() || hidden { '?' } else { c }
        })
        .collect();
    if value.chars().count() > MAX_SHOWN {
        out.push('\u{2026}');
    }
    out
}

struct Text<'a> {
    catalog: &'a Catalog,
    locale: Locale,
}

impl Text<'_> {
    fn t(&self, key: MessageKey, args: &[(&str, &str)]) -> String {
        self.catalog.get(self.locale, key, args).text
    }
}

fn tier_key(tier: Tier) -> MessageKey {
    match tier {
        Tier::Read => MessageKey::new("tier.read.label"),
        Tier::Plan => MessageKey::new("tier.plan.label"),
        Tier::LightChange => MessageKey::new("tier.change_light.label"),
        Tier::RiskyChange => MessageKey::new("tier.change_risky.label"),
        Tier::Destructive => MessageKey::new("tier.destructive.label"),
    }
}

fn step_keys(kind: StepKind) -> (MessageKey, MessageKey) {
    match kind {
        StepKind::Inventory => (
            MessageKey::new("plan.step_inventory.title"),
            MessageKey::new("plan.step_inventory.impact"),
        ),
        StepKind::Backup => (
            MessageKey::new("plan.step_backup.title"),
            MessageKey::new("plan.step_backup.impact"),
        ),
        StepKind::Transfer => (
            MessageKey::new("plan.step_transfer.title"),
            MessageKey::new("plan.step_transfer.impact"),
        ),
        StepKind::Restore => (
            MessageKey::new("plan.step_restore.title"),
            MessageKey::new("plan.step_restore.impact"),
        ),
        StepKind::Verify => (
            MessageKey::new("plan.step_verify.title"),
            MessageKey::new("plan.step_verify.impact"),
        ),
        StepKind::Delta => (
            MessageKey::new("plan.step_delta.title"),
            MessageKey::new("plan.step_delta.impact"),
        ),
        StepKind::Cutover => (
            MessageKey::new("plan.step_cutover.title"),
            MessageKey::new("plan.step_cutover.impact"),
        ),
        StepKind::PostVerify => (
            MessageKey::new("plan.step_post_verify.title"),
            MessageKey::new("plan.step_post_verify.impact"),
        ),
        StepKind::DecommissionSource => (
            MessageKey::new("plan.step_decommission.title"),
            MessageKey::new("plan.step_decommission.impact"),
        ),
    }
}

fn step_lines(x: &Text<'_>, step: &Step) -> Vec<String> {
    let (title_key, impact_key) = step_keys(step.kind);
    let mut title = x.t(title_key, &[]);
    if let Some(side) = step.side {
        let side_key = match side {
            Side::Source => MessageKey::new("plan.side_source.text"),
            Side::Target => MessageKey::new("plan.side_target.text"),
        };
        title.push_str(" - ");
        title.push_str(&x.t(side_key, &[]));
    }
    if let Some(db) = &step.database {
        title.push_str(" - ");
        title.push_str(&x.t(
            MessageKey::new("plan.steps.database"),
            &[("database", &clean(db))],
        ));
    }
    let tier = step.kind.tier();
    let number = (step.seq + 1).to_string();
    let mut lines = vec![x.t(
        MessageKey::new("plan.steps.line"),
        &[
            ("number", &number),
            ("title", &title),
            ("tier", &x.t(tier_key(tier), &[])),
        ],
    )];
    lines.push(x.t(
        MessageKey::new("plan.steps.impact"),
        &[("impact", &x.t(impact_key, &[]))],
    ));
    if step.depends_on.is_empty() {
        lines.push(x.t(MessageKey::new("plan.steps.first"), &[]));
    } else {
        let mut after: Vec<u32> = step.depends_on.iter().map(|d| d + 1).collect();
        after.sort_unstable();
        let list = after
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(x.t(MessageKey::new("plan.steps.after"), &[("steps", &list)]));
    }
    if tier.requires_typed_confirmation() {
        lines.push(x.t(MessageKey::new("plan.steps.typed"), &[]));
    } else if !tier.approvable_from_chat_bot() {
        lines.push(x.t(MessageKey::new("plan.steps.panel_only"), &[]));
    }
    lines
}

/// Renders the written plan.
///
/// The exclusion list and the conditions before the old server may be shut down are always
/// printed, even when the list is empty or no shutdown step is planned.
#[must_use]
pub fn render_plan(plan: &Plan, gates: &GateReport, catalog: &Catalog, locale: Locale) -> String {
    let x = Text { catalog, locale };
    let spec = plan.spec();
    let mut out: Vec<String> = Vec::new();

    out.push(format!(
        "# {}",
        x.t(
            MessageKey::new("plan.heading.title"),
            &[("name", &clean(plan.name()))]
        )
    ));
    out.push(String::new());

    // Overview.
    out.push(format!(
        "## {}",
        x.t(MessageKey::new("plan.heading.overview"), &[])
    ));
    out.push(x.t(
        MessageKey::new("plan.overview.route"),
        &[
            ("source", &clean(&spec.source.node_id)),
            ("target", &clean(&spec.target.node_id)),
        ],
    ));
    let location_key = match spec.backup.location {
        BackupLocation::Source => MessageKey::new("plan.location_source.text"),
        BackupLocation::Local => MessageKey::new("plan.location_local.text"),
        BackupLocation::Intermediary => MessageKey::new("plan.location_intermediary.text"),
    };
    out.push(x.t(
        MessageKey::new("plan.overview.backup"),
        &[("location", &x.t(location_key, &[]))],
    ));
    let encryption_key = match spec.transport.encryption {
        Encryption::ClientSide => MessageKey::new("plan.encryption_client_side.text"),
        Encryption::None => MessageKey::new("plan.encryption_none.text"),
    };
    out.push(x.t(
        MessageKey::new("plan.overview.transport"),
        &[
            ("transport", &clean(&spec.transport.transport_id)),
            ("encryption", &x.t(encryption_key, &[])),
        ],
    ));
    let yes_no = |flag: bool| {
        x.t(
            if flag {
                MessageKey::new("plan.word_yes.text")
            } else {
                MessageKey::new("plan.word_no.text")
            },
            &[],
        )
    };
    let method_key = match spec.cutover.method {
        CutoverMethod::Tunnel => MessageKey::new("plan.method_tunnel.text"),
        CutoverMethod::Dns => MessageKey::new("plan.method_dns.text"),
    };
    out.push(x.t(
        MessageKey::new("plan.overview.cutover"),
        &[
            ("method", &x.t(method_key, &[])),
            ("snapshot", &yes_no(spec.cutover.dns_snapshot)),
            ("rollback", &yes_no(spec.cutover.rollback)),
        ],
    ));
    let count = |mode: DbMode| {
        spec.data
            .databases
            .iter()
            .filter(|db| db.mode == mode)
            .count()
            .to_string()
    };
    out.push(x.t(
        MessageKey::new("plan.overview.databases"),
        &[
            ("total", &spec.data.databases.len().to_string()),
            ("restore", &count(DbMode::Restore)),
            ("copy", &count(DbMode::CopyAsSuffix)),
            ("skip", &count(DbMode::Skip)),
        ],
    ));
    out.push(x.t(MessageKey::new("plan.overview.policy"), &[]));
    out.push(String::new());

    // Steps, in the order they run.
    out.push(format!(
        "## {}",
        x.t(MessageKey::new("plan.heading.steps"), &[])
    ));
    for step in plan.execution_order() {
        out.extend(step_lines(&x, step));
    }
    out.push(String::new());

    // Where to stop.
    out.push(format!(
        "## {}",
        x.t(MessageKey::new("plan.heading.abort"), &[])
    ));
    out.push(x.t(MessageKey::new("plan.abort.before_cutover"), &[]));
    out.push(x.t(
        MessageKey::new("plan.abort.at_cutover"),
        &[("rollback", &yes_no(spec.cutover.rollback))],
    ));
    if plan
        .steps()
        .iter()
        .any(|s| s.kind == StepKind::DecommissionSource)
    {
        out.push(x.t(MessageKey::new("plan.abort.no_return"), &[]));
    }
    out.push(String::new());

    // What the time depends on.
    out.push(format!(
        "## {}",
        x.t(MessageKey::new("plan.heading.estimate"), &[])
    ));
    out.push(x.t(MessageKey::new("plan.estimate.size"), &[]));
    out.push(x.t(MessageKey::new("plan.estimate.speed"), &[]));
    out.push(x.t(
        MessageKey::new("plan.estimate.databases"),
        &[("count", &spec.data.databases.len().to_string())],
    ));
    if plan.steps().iter().any(|s| s.kind == StepKind::Delta) {
        out.push(x.t(MessageKey::new("plan.estimate.delta"), &[]));
    }
    if spec.cutover.method == CutoverMethod::Dns {
        out.push(x.t(MessageKey::new("plan.estimate.ttl"), &[]));
    }
    out.push(String::new());

    // Conditions before the old server may be shut down: always shown.
    out.push(format!(
        "## {}",
        x.t(MessageKey::new("plan.heading.conditions"), &[])
    ));
    out.push(x.t(MessageKey::new("plan.conditions.intro"), &[]));
    let state = |met: bool| {
        x.t(
            if met {
                MessageKey::new("plan.conditions.met")
            } else {
                MessageKey::new("plan.conditions.not_met")
            },
            &[],
        )
    };
    out.push(x.t(
        MessageKey::new("plan.conditions.line"),
        &[
            (
                "condition",
                &x.t(MessageKey::new("plan.conditions.no_critical"), &[]),
            ),
            ("state", &state(gates.open_critical.is_empty())),
        ],
    ));
    out.push(x.t(
        MessageKey::new("plan.conditions.line"),
        &[
            (
                "condition",
                &x.t(MessageKey::new("plan.conditions.complete"), &[]),
            ),
            ("state", &state(!gates.inventory_incomplete)),
        ],
    ));
    out.push(x.t(
        MessageKey::new("plan.conditions.line"),
        &[
            (
                "condition",
                &x.t(MessageKey::new("plan.conditions.verified"), &[]),
            ),
            (
                "state",
                &x.t(MessageKey::new("plan.conditions.at_run"), &[]),
            ),
        ],
    ));
    if !plan
        .steps()
        .iter()
        .any(|s| s.kind == StepKind::DecommissionSource)
    {
        out.push(x.t(MessageKey::new("plan.conditions.no_step"), &[]));
    }
    out.push(String::new());

    // What is left out: always shown.
    out.push(format!(
        "## {}",
        x.t(MessageKey::new("plan.heading.exclusions"), &[])
    ));
    if spec.backup.exclude.is_empty() {
        out.push(x.t(MessageKey::new("plan.exclusions.none"), &[]));
    } else {
        out.push(x.t(MessageKey::new("plan.exclusions.intro"), &[]));
        for item in &spec.backup.exclude {
            let description = describe_key(item).map_or_else(
                || x.t(MessageKey::new("plan.exclusions.custom"), &[]),
                |key| x.t(key, &[]),
            );
            out.push(x.t(
                MessageKey::new("plan.exclusions.line"),
                &[("item", &clean(item)), ("description", &description)],
            ));
        }
    }
    out.push(String::new());

    // Findings.
    out.push(format!(
        "## {}",
        x.t(MessageKey::new("plan.heading.blocking"), &[])
    ));
    if gates.open_critical.is_empty() {
        out.push(x.t(MessageKey::new("plan.blocking.none"), &[]));
    } else {
        out.push(x.t(MessageKey::new("plan.blocking.intro"), &[]));
        for key in &gates.open_critical {
            out.push(x.t(
                MessageKey::new("plan.blocking.line"),
                &[
                    ("rule", &clean(&key.rule_id)),
                    ("kind", key.subject.kind.as_str()),
                    ("key", &clean(&key.subject.key)),
                ],
            ));
        }
    }
    if !gates.accepted_critical.is_empty() {
        out.push(x.t(MessageKey::new("plan.blocking.accepted_intro"), &[]));
        for accepted in &gates.accepted_critical {
            out.push(x.t(
                MessageKey::new("plan.blocking.accepted_line"),
                &[
                    ("rule", &clean(&accepted.key.rule_id)),
                    ("kind", accepted.key.subject.kind.as_str()),
                    ("key", &clean(&accepted.key.subject.key)),
                    ("reason", &clean(&accepted.reason)),
                ],
            ));
        }
    }
    if gates.inventory_incomplete {
        out.push(x.t(MessageKey::new("plan.blocking.incomplete"), &[]));
    }
    out.join("\n")
}

fn warning_key(warning: DraftWarning) -> MessageKey {
    match warning {
        DraftWarning::SourceInventoryPartial => MessageKey::new("plan.warning_source_partial.text"),
        DraftWarning::TargetInventoryPartial => MessageKey::new("plan.warning_target_partial.text"),
        DraftWarning::NoTargetInventory => MessageKey::new("plan.warning_no_target.text"),
        DraftWarning::PlatformDiffers => MessageKey::new("plan.warning_platform.text"),
        DraftWarning::NoDatabases => MessageKey::new("plan.warning_no_databases.text"),
    }
}

/// Renders a draft: the written plan followed by the warnings that go with it.
#[must_use]
pub fn render_draft(draft: &Draft, catalog: &Catalog, locale: Locale) -> String {
    let mut text = render_plan(&draft.plan, &draft.gates, catalog, locale);
    if !draft.warnings.is_empty() {
        let x = Text { catalog, locale };
        text.push_str("\n\n");
        text.push_str(&format!(
            "## {}",
            x.t(MessageKey::new("plan.heading.warnings"), &[])
        ));
        for warning in &draft.warnings {
            text.push('\n');
            text.push_str(&x.t(warning_key(*warning), &[]));
        }
    }
    text
}

/// Binary size with one decimal, e.g. `1.5 GiB`. Units are symbols, not words, so they are the
/// same in every language.
fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut whole = bytes;
    let mut rest = 0u64;
    let mut unit = 0;
    while whole >= 1024 && unit < UNITS.len() - 1 {
        rest = whole % 1024;
        whole /= 1024;
        unit += 1;
    }
    let tenth = rest * 10 / 1024;
    let name = UNITS.get(unit).copied().unwrap_or("B");
    if unit == 0 {
        format!("{whole} {name}")
    } else {
        format!("{whole}.{tenth} {name}")
    }
}

/// Renders the capacity advice as lines a person can act on, in the verdict-first order.
#[must_use]
pub fn render_capacity(advice: &CapacityAdvice, catalog: &Catalog, locale: Locale) -> String {
    let x = Text { catalog, locale };
    let verdict_key = match advice.verdict {
        CapacityVerdict::Fits => MessageKey::new("plan.capacity_fits.text"),
        CapacityVerdict::Unknown => MessageKey::new("plan.capacity_unknown.text"),
        CapacityVerdict::Blocked => MessageKey::new("plan.capacity_blocked.text"),
    };
    let mut lines = vec![x.t(verdict_key, &[])];
    for issue in &advice.issues {
        let key = match issue.kind {
            CapacityIssueKind::DataSizeUnknown => {
                MessageKey::new("plan.capacity_data_unknown.text")
            }
            CapacityIssueKind::TargetTooSmall => MessageKey::new("plan.capacity_target_small.text"),
            CapacityIssueKind::TargetFreeUnknown => {
                MessageKey::new("plan.capacity_target_unknown.text")
            }
            CapacityIssueKind::StagingTooSmall => {
                MessageKey::new("plan.capacity_staging_small.text")
            }
            CapacityIssueKind::StagingFreeUnknown => {
                MessageKey::new("plan.capacity_staging_unknown.text")
            }
            CapacityIssueKind::QuotaTooSmall => MessageKey::new("plan.capacity_quota_small.text"),
            CapacityIssueKind::QuotaUnknown => MessageKey::new("plan.capacity_quota_unknown.text"),
        };
        let needed = issue.needed.map(format_bytes).unwrap_or_default();
        let available = issue.available.map(format_bytes).unwrap_or_default();
        lines.push(x.t(key, &[("needed", &needed), ("available", &available)]));
    }
    if advice.suggest_streaming {
        lines.push(x.t(MessageKey::new("plan.capacity_stream.text"), &[]));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_replaces_control_and_bidi_characters_and_cuts_long_values() {
        assert_eq!(clean("a\nb\u{1b}[31mc"), "a?b?[31mc");
        assert_eq!(clean("x\u{202e}y"), "x?y");
        let long = "a".repeat(MAX_SHOWN + 10);
        let shown = clean(&long);
        assert_eq!(shown.chars().count(), MAX_SHOWN + 1);
        assert!(shown.ends_with('\u{2026}'));
    }

    #[test]
    fn formats_sizes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1024), "1.0 KiB");
        assert_eq!(format_bytes(1536 * 1024 * 1024), "1.5 GiB");
        assert!(format_bytes(u64::MAX).ends_with("TiB"));
    }
}
