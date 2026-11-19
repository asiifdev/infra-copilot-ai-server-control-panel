//! The M1 rules. Each is a pure, deterministic function of the two inventories.

use core_domain::tier::Tier;
use inventory::{Finding, Inventory, Item, Kind, ParityRule, RuleContext, Severity, SubjectRef};
use std::collections::BTreeMap;

use crate::honesty::cap_if_partial;

/// Sentinel placed in `Finding::args` when a value could not be determined. It is not display
/// text: the renderer maps it to the catalog word `parity.arg.unknown`. It starts with NUL, which
/// cannot occur in a process name, a bind address or a cron field.
pub const UNKNOWN_ARG: &str = "\u{0}unknown";

fn subject(item: &Item) -> SubjectRef {
    SubjectRef {
        kind: item.kind,
        key: item.key.clone(),
    }
}

/// The port of a listener item: the `port` field, or the key as a fallback.
fn port_of(item: &Item) -> String {
    item.u64_field("port")
        .map_or_else(|| item.key.clone(), |port| port.to_string())
}

/// The process name of a listener, or [`UNKNOWN_ARG`] when the collector could not tell.
fn process_of(item: &Item) -> &str {
    item.str_field("process").unwrap_or(UNKNOWN_ARG)
}

/// PAR-001: a process listens on the source but nothing matches it on the target.
///
/// Origin of the case: a search service that listened on the old server was never installed on the
/// new one, and the application that needed it kept running but returned empty search results.
///
/// For each source listener `tcp:P` without a target listener `tcp:P`:
/// * `Critical` when no target listener runs a process of the same name (no counterpart at all);
/// * `Important` when such a process exists on the target on another port (the port moved, so
///   configuration that points to the old port will break).
///
/// A target listener counts as the new home of a moved port only if it (i) runs a process of the
/// same name, (ii) listens on a port that no source listener uses (otherwise it is already the
/// counterpart of another source port) and (iii) has not been claimed by another source listener:
/// each target listener is used once, assigned in source key order, so the result is deterministic.
///
/// No port is ignored on purpose; if that proves noisy it is a separate decision.
#[derive(Debug, Clone, Copy, Default)]
pub struct Par001;

impl ParityRule for Par001 {
    fn id(&self) -> &'static str {
        "PAR-001"
    }

    fn version(&self) -> u32 {
        1
    }

    /// The highest severity this rule produces.
    fn severity(&self) -> Severity {
        Severity::Critical
    }

    fn category(&self) -> &'static str {
        "coverage"
    }

    /// Manual fix in M1-a.
    fn fix_tier(&self) -> Option<Tier> {
        None
    }

    fn evaluate(&self, source: &Inventory, target: &Inventory, _ctx: &RuleContext) -> Vec<Finding> {
        let mut findings = Vec::new();
        // Target listeners that may still be claimed as the new home of a moved port.
        let mut spare: Vec<&Item> = target
            .items_of(Kind::Listener)
            .filter(|candidate| source.get(Kind::Listener, &candidate.key).is_none())
            .collect();
        for item in source.items_of(Kind::Listener) {
            if target.get(Kind::Listener, &item.key).is_some() {
                continue;
            }
            let twin = item.str_field("process").and_then(|name| {
                let at = spare
                    .iter()
                    .position(|candidate| candidate.str_field("process") == Some(name))?;
                Some(spare.remove(at))
            });
            let mut evidence = vec![subject(item)];
            let severity = match twin {
                Some(moved) => {
                    evidence.push(subject(moved));
                    Severity::Important
                }
                None => Severity::Critical,
            };
            let args = BTreeMap::from([
                ("port".to_owned(), port_of(item)),
                ("process".to_owned(), process_of(item).to_owned()),
                (
                    "bind".to_owned(),
                    item.str_field("bind").unwrap_or(UNKNOWN_ARG).to_owned(),
                ),
            ]);
            findings.push(cap_if_partial(
                Finding {
                    rule_id: self.id().to_owned(),
                    severity,
                    subject: subject(item),
                    evidence,
                    args,
                },
                target,
            ));
        }
        findings
    }
}

/// PAR-008: a scheduled job on the source has no counterpart on the target (same item key).
///
/// Origin of the case: a scheduled backup owned by the hosting panel did not move with the sites,
/// so the new server ran for weeks without any backup and nobody noticed. Panel-owned jobs are
/// included; the `source` field of the item is not a filter.
#[derive(Debug, Clone, Copy, Default)]
pub struct Par008;

impl ParityRule for Par008 {
    fn id(&self) -> &'static str {
        "PAR-008"
    }

    fn version(&self) -> u32 {
        1
    }

    fn severity(&self) -> Severity {
        Severity::Important
    }

    fn category(&self) -> &'static str {
        "scheduling"
    }

    fn fix_tier(&self) -> Option<Tier> {
        None
    }

    fn evaluate(&self, source: &Inventory, target: &Inventory, _ctx: &RuleContext) -> Vec<Finding> {
        source
            .items_of(Kind::Cron)
            .filter(|item| target.get(Kind::Cron, &item.key).is_none())
            .map(|item| {
                let field = |name| item.str_field(name).unwrap_or(UNKNOWN_ARG).to_owned();
                let args = BTreeMap::from([
                    ("schedule".to_owned(), field("schedule")),
                    ("command".to_owned(), field("command_hint")),
                    ("owner".to_owned(), field("owner")),
                ]);
                cap_if_partial(
                    Finding {
                        rule_id: self.id().to_owned(),
                        severity: self.severity(),
                        subject: subject(item),
                        evidence: vec![subject(item)],
                        args,
                    },
                    target,
                )
            })
            .collect()
    }
}

/// PAR-010: a port is open to the internet.
///
/// Origin of the case: a database admin interface and a cache were found listening on a public
/// address of the new server, which had no firewall; the old server had hidden them behind one.
///
/// On the target it is `Important` (something new may now be exposed); on the source it is `Info`
/// (context for what the target should look like). `args["side"]` is `source` or `target`.
///
/// A public listener that was seen is evidence that it exists, so a partial inventory does NOT
/// lower the severity (only "no counterpart" rules, which claim absence, are capped).
#[derive(Debug, Clone, Copy, Default)]
pub struct Par010;

impl ParityRule for Par010 {
    fn id(&self) -> &'static str {
        "PAR-010"
    }

    fn version(&self) -> u32 {
        1
    }

    fn severity(&self) -> Severity {
        Severity::Important
    }

    fn category(&self) -> &'static str {
        "security"
    }

    fn fix_tier(&self) -> Option<Tier> {
        None
    }

    fn evaluate(&self, source: &Inventory, target: &Inventory, _ctx: &RuleContext) -> Vec<Finding> {
        let side = |inventory: &Inventory, side: &str, severity: Severity| {
            inventory
                .items_of(Kind::Listener)
                .filter(|item| item.str_field("exposed") == Some("public"))
                .map(|item| {
                    let args = BTreeMap::from([
                        ("port".to_owned(), port_of(item)),
                        ("process".to_owned(), process_of(item).to_owned()),
                        ("side".to_owned(), side.to_owned()),
                    ]);
                    Finding {
                        rule_id: self.id().to_owned(),
                        severity,
                        subject: subject(item),
                        evidence: vec![subject(item)],
                        args,
                    }
                })
                .collect::<Vec<_>>()
        };
        let mut findings = side(source, "source", Severity::Info);
        findings.extend(side(target, "target", self.severity()));
        findings
    }
}

/// PAR-006: an application that serves a port runs a development server, or runs outside any
/// supervisor.
///
/// Origin of the case: after a move, applications were found started by hand from a shell or in
/// development mode; they served traffic but would not come back after a reboot or a crash.
///
/// Evaluated on the target only. For each process that owns a listener (matched by name), the
/// finding is `Important` with `args["reason"]` = `dev` (a known development server) or
/// `unsupervised` (collector saw it in a login session with no supervisor above it). `dev` wins
/// when both apply. An unknown `supervised` (null) never triggers: the rule claims only what was
/// seen. One finding per process; `args["ports"]` lists its ports. A partial inventory does not
/// lower the severity, because the process was seen.
#[derive(Debug, Clone, Copy, Default)]
pub struct Par006;

impl ParityRule for Par006 {
    fn id(&self) -> &'static str {
        "PAR-006"
    }

    fn version(&self) -> u32 {
        1
    }

    fn severity(&self) -> Severity {
        Severity::Important
    }

    fn category(&self) -> &'static str {
        "runtime"
    }

    fn fix_tier(&self) -> Option<Tier> {
        None
    }

    fn evaluate(
        &self,
        _source: &Inventory,
        target: &Inventory,
        _ctx: &RuleContext,
    ) -> Vec<Finding> {
        let mut owned: BTreeMap<&str, Vec<&Item>> = BTreeMap::new();
        for listener in target.items_of(Kind::Listener) {
            if let Some(name) = listener.str_field("process") {
                owned.entry(name).or_default().push(listener);
            }
        }
        let mut findings = Vec::new();
        for (name, listeners) in owned {
            let Some(process) = target.get(Kind::Process, &format!("proc:{name}")) else {
                continue;
            };
            let reason = if process.str_field("mode") == Some("dev") {
                "dev"
            } else if process.data.get("supervised").and_then(|v| v.as_bool()) == Some(false) {
                "unsupervised"
            } else {
                continue;
            };
            let ports: Vec<String> = listeners.iter().map(|l| port_of(l)).collect();
            let mut evidence = vec![subject(process)];
            evidence.extend(listeners.iter().map(|l| subject(l)));
            findings.push(Finding {
                rule_id: self.id().to_owned(),
                severity: self.severity(),
                subject: subject(process),
                evidence,
                args: BTreeMap::from([
                    ("process".to_owned(), name.to_owned()),
                    ("ports".to_owned(), ports.join(", ")),
                    ("reason".to_owned(), reason.to_owned()),
                ]),
            });
        }
        findings
    }
}
