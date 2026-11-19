//! What a parity rule reports.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::item::Kind;

/// How much a finding matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Informational.
    Info,
    /// Should be handled before cutover.
    Important,
    /// Blocks cutover until resolved or explicitly accepted.
    Critical,
}

/// Points at an item of an inventory (`source` or `target`) without copying it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SubjectRef {
    /// Kind of the item.
    pub kind: Kind,
    /// Key of the item.
    pub key: String,
}

/// One result of a parity rule. Human text is not stored here: the renderer looks it up in the
/// i18n catalog from `rule_id` and fills the placeholders from `args` (docs/design.md 14.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    /// Stable rule id, e.g. `PAR-001`.
    pub rule_id: String,
    /// Weight of the finding.
    pub severity: Severity,
    /// The item the finding is about.
    pub subject: SubjectRef,
    /// Items that support the finding, so a person can trace it.
    pub evidence: Vec<SubjectRef>,
    /// Values for the message placeholders (names and ports only, never secrets).
    pub args: BTreeMap<String, String>,
}
