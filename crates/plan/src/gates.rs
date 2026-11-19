//! The conditions before cutover and before the old server may be shut down (skill
//! `migration-planning`): no critical finding is open, and the source stays until the target is
//! verified. Pure functions over findings; nothing here reads or changes a server.

use inventory::{Finding, Inventory, Severity, Status, SubjectRef};
use std::collections::BTreeSet;

use crate::error::PlanError;

/// Longest accepted reason for accepting a finding, in bytes.
const MAX_REASON_LEN: usize = 500;

/// Marker the parity rules put in `Finding::args` when the evidence rests on an incomplete
/// inventory (kept in sync with `parity`, which this crate does not depend on).
const PARTIAL_ARG: &str = "partial";

/// Identifies one finding: the rule and the item it is about.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct FindingKey {
    /// Rule id, e.g. `PAR-001`.
    pub rule_id: String,
    /// The item the finding is about.
    pub subject: SubjectRef,
}

impl FindingKey {
    fn of(finding: &Finding) -> Self {
        Self {
            rule_id: finding.rule_id.clone(),
            subject: finding.subject.clone(),
        }
    }
}

/// A finding a person accepted as it is. A reason is mandatory (docs/design.md 3.3): accepting
/// without saying why leaves the next reader nothing to judge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedFinding {
    /// Which finding.
    pub key: FindingKey,
    /// Why it is accepted.
    pub reason: String,
}

impl AcceptedFinding {
    /// Builds an acceptance.
    ///
    /// # Errors
    /// [`PlanError::Invalid`] when the reason is blank, too long or has control characters.
    pub fn new(rule_id: &str, subject: SubjectRef, reason: &str) -> Result<Self, PlanError> {
        let accepted = Self {
            key: FindingKey {
                rule_id: rule_id.to_owned(),
                subject,
            },
            reason: reason.to_owned(),
        };
        if accepted.has_valid_reason() {
            Ok(accepted)
        } else {
            Err(PlanError::Invalid("accepted.reason"))
        }
    }

    // Re-checked by `gate_report`, because the fields are public and a literal bypasses `new`.
    fn has_valid_reason(&self) -> bool {
        !self.reason.trim().is_empty()
            && self.reason.len() <= MAX_REASON_LEN
            && !self.reason.chars().any(|c| c.is_control() && c != '\n')
    }
}

/// Where a plan stands against its gates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateReport {
    /// Critical findings nobody accepted. Any entry blocks cutover and decommissioning.
    pub open_critical: Vec<FindingKey>,
    /// Critical findings accepted with a reason; shown, never hidden.
    pub accepted_critical: Vec<AcceptedFinding>,
    /// Number of open findings of `important` severity (advice, not a gate).
    pub important: usize,
    /// Number of findings of `info` severity.
    pub info: usize,
    /// The findings (or an inventory behind them) are incomplete, so an empty list is not proof.
    /// This blocks like an open critical finding until the inventory is collected again.
    pub inventory_incomplete: bool,
}

impl GateReport {
    /// No critical finding is open and the evidence is complete: traffic may move.
    #[must_use]
    pub fn cutover_allowed(&self) -> bool {
        self.open_critical.is_empty() && !self.inventory_incomplete
    }

    /// The old server may only be shut down when cutover is allowed **and** the target has been
    /// verified after traffic moved. `post_verified` is decided by the caller from step results.
    #[must_use]
    pub fn decommission_allowed(&self, post_verified: bool) -> bool {
        self.cutover_allowed() && post_verified
    }

    /// Marks the report incomplete when an inventory behind it is partial. A partial inventory
    /// makes absence rules unreliable (they drop to `info`), so it must not pass a gate silently.
    #[must_use]
    pub fn with_inventories(mut self, source: &Inventory, target: Option<&Inventory>) -> Self {
        let partial = source.status == Status::Partial
            || target.is_some_and(|inv| inv.status == Status::Partial);
        self.inventory_incomplete |= partial;
        self
    }
}

/// Evaluates the gates over the findings of a comparison.
///
/// A critical finding is open unless an entry of `accepted` names the same rule and subject **and**
/// carries a valid reason. Acceptances that match nothing are ignored. A finding flagged as resting
/// on a partial inventory marks the report incomplete.
#[must_use]
pub fn gate_report(findings: &[Finding], accepted: &[AcceptedFinding]) -> GateReport {
    let valid: BTreeSet<&FindingKey> = accepted
        .iter()
        .filter(|a| a.has_valid_reason())
        .map(|a| &a.key)
        .collect();
    let mut report = GateReport {
        open_critical: Vec::new(),
        accepted_critical: Vec::new(),
        important: 0,
        info: 0,
        inventory_incomplete: false,
    };
    for finding in findings {
        if finding.args.get(PARTIAL_ARG).map(String::as_str) == Some("true") {
            report.inventory_incomplete = true;
        }
        match finding.severity {
            Severity::Critical => {
                let key = FindingKey::of(finding);
                if valid.contains(&key) {
                    if let Some(entry) = accepted
                        .iter()
                        .find(|a| a.key == key && a.has_valid_reason())
                    {
                        report.accepted_critical.push(entry.clone());
                    }
                } else {
                    report.open_critical.push(key);
                }
            }
            Severity::Important => report.important += 1,
            Severity::Info => report.info += 1,
        }
    }
    report.open_critical.sort();
    report.open_critical.dedup();
    report.accepted_critical.sort_by(|a, b| a.key.cmp(&b.key));
    report.accepted_critical.dedup();
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use inventory::Kind;
    use std::collections::BTreeMap;

    fn finding(rule: &str, severity: Severity, key: &str) -> Finding {
        Finding {
            rule_id: rule.to_owned(),
            severity,
            subject: SubjectRef {
                kind: Kind::Listener,
                key: key.to_owned(),
            },
            evidence: Vec::new(),
            args: BTreeMap::new(),
        }
    }

    fn subject(key: &str) -> SubjectRef {
        SubjectRef {
            kind: Kind::Listener,
            key: key.to_owned(),
        }
    }

    #[test]
    fn open_critical_blocks_and_accepting_with_a_reason_unblocks() -> Result<(), PlanError> {
        let findings = [finding("PAR-001", Severity::Critical, "tcp:7700")];
        let open = gate_report(&findings, &[]);
        assert!(!open.cutover_allowed());
        assert_eq!(open.open_critical.len(), 1);

        let accepted = [AcceptedFinding::new(
            "PAR-001",
            subject("tcp:7700"),
            "unused",
        )?];
        let ok = gate_report(&findings, &accepted);
        assert!(ok.cutover_allowed());
        assert_eq!(ok.accepted_critical.len(), 1);
        assert!(ok.open_critical.is_empty());
        Ok(())
    }

    #[test]
    fn a_blank_reason_literal_does_not_count() {
        let findings = [finding("PAR-001", Severity::Critical, "tcp:7700")];
        let bypass = AcceptedFinding {
            key: FindingKey {
                rule_id: "PAR-001".into(),
                subject: subject("tcp:7700"),
            },
            reason: "   ".into(),
        };
        assert!(!gate_report(&findings, &[bypass]).cutover_allowed());
    }

    #[test]
    fn acceptance_matches_rule_and_subject_exactly() -> Result<(), PlanError> {
        let findings = [finding("PAR-001", Severity::Critical, "tcp:7700")];
        for (rule, key) in [("PAR-002", "tcp:7700"), ("PAR-001", "tcp:7701")] {
            let accepted = [AcceptedFinding::new(rule, subject(key), "why")?];
            assert!(!gate_report(&findings, &accepted).cutover_allowed());
        }
        Ok(())
    }

    #[test]
    fn only_critical_findings_gate() {
        let findings = [
            finding("PAR-001", Severity::Important, "tcp:1"),
            finding("PAR-010", Severity::Info, "tcp:2"),
        ];
        let report = gate_report(&findings, &[]);
        assert!(report.cutover_allowed());
        assert_eq!((report.important, report.info), (1, 1));
    }

    #[test]
    fn decommission_needs_cutover_allowed_and_post_verification() {
        let clean = gate_report(&[], &[]);
        assert!(!clean.decommission_allowed(false));
        assert!(clean.decommission_allowed(true));
        let blocked = gate_report(&[finding("PAR-001", Severity::Critical, "tcp:1")], &[]);
        assert!(!blocked.decommission_allowed(true));
    }

    #[test]
    fn partial_evidence_blocks_like_an_open_critical() {
        let mut flagged = finding("PAR-001", Severity::Info, "tcp:1");
        flagged.args.insert("partial".into(), "true".into());
        let report = gate_report(&[flagged], &[]);
        assert!(report.inventory_incomplete);
        assert!(!report.cutover_allowed());
    }

    #[test]
    fn rejects_blank_or_oversized_reasons_at_construction() {
        for reason in ["", "  ", "a\u{0}b", &"x".repeat(MAX_REASON_LEN + 1)] {
            assert!(AcceptedFinding::new("PAR-001", subject("tcp:1"), reason).is_err());
        }
    }

    #[test]
    fn duplicate_findings_are_listed_once() {
        let f = finding("PAR-001", Severity::Critical, "tcp:1");
        let report = gate_report(&[f.clone(), f], &[]);
        assert_eq!(report.open_critical.len(), 1);
    }
}
