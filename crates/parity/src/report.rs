//! The rule list and the comparer.

use inventory::{Finding, Inventory, ParityRule, RuleContext};
use std::cmp::Reverse;

use crate::rules::{Par001, Par002, Par006, Par007, Par008, Par009, Par010};

/// The result of comparing two inventories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// Findings ordered by severity (highest first), then rule id, then subject key, then subject
    /// kind, then arguments, so the same input always gives the same order.
    pub findings: Vec<Finding>,
    /// `(id, version)` of every rule that ran, in id order.
    pub rules: Vec<(&'static str, u32)>,
}

/// Every rule, in id order.
#[must_use]
pub fn all_rules() -> Vec<Box<dyn ParityRule>> {
    vec![
        Box::new(Par001),
        Box::new(Par002),
        Box::new(Par006),
        Box::new(Par007),
        Box::new(Par008),
        Box::new(Par009),
        Box::new(Par010),
    ]
}

/// Runs every rule over `source` and `target` and returns the ordered findings.
#[must_use]
pub fn compare(source: &Inventory, target: &Inventory, ctx: &RuleContext) -> Report {
    let rules = all_rules();
    let mut findings: Vec<Finding> = rules
        .iter()
        .flat_map(|rule| rule.evaluate(source, target, ctx))
        .collect();
    findings.sort_by(|a, b| {
        (
            Reverse(a.severity),
            &a.rule_id,
            &a.subject.key,
            a.subject.kind,
            &a.args,
        )
            .cmp(&(
                Reverse(b.severity),
                &b.rule_id,
                &b.subject.key,
                b.subject.kind,
                &b.args,
            ))
    });
    let mut ran: Vec<_> = rules
        .iter()
        .map(|rule| (rule.id(), rule.version()))
        .collect();
    ran.sort_unstable();
    Report {
        findings,
        rules: ran,
    }
}
