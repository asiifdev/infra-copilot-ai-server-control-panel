//! The two contracts: gathering items and judging two inventories.

use core_domain::tier::Tier;
use std::fmt;

use crate::finding::{Finding, Severity};
use crate::item::Item;
use crate::snapshot::Inventory;

/// What a collector returns: the items it found and anything it could not read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Collection {
    /// Items found.
    pub items: Vec<Item>,
    /// Things that could not be read (permissions, missing files). A non-empty list means the
    /// inventory is partial; it must never be silently treated as "nothing there".
    pub warnings: Vec<String>,
}

/// A collector failed as a whole (not just a missing file, which becomes a warning).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectError(pub String);

impl fmt::Display for CollectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "collection failed: {}", self.0)
    }
}

impl std::error::Error for CollectError {}

/// Gathers one family of items, **read-only**: it must not change anything on the machine.
pub trait Collector {
    /// Short stable kind, e.g. `listeners` (docs/design.md 12.4).
    fn kind(&self) -> &'static str;

    /// Reads the machine and returns what it found.
    ///
    /// # Errors
    /// Only when the collector cannot work at all; partial reads are reported as warnings.
    fn run(&self) -> Result<Collection, CollectError>;
}

/// Extra input for rules beyond the two inventories (docs/design.md 5: `(source, target, context)`).
/// Empty for now; DNS ownership and functional-test results are added here when they exist, so
/// the rule signature does not change.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct RuleContext {}

/// A parity rule: a pure function from a source and a target inventory to findings.
pub trait ParityRule {
    /// Stable id, e.g. `PAR-001`.
    fn id(&self) -> &'static str;

    /// Version of the rule logic; bumped when its behaviour changes (docs/design.md 5).
    fn version(&self) -> u32;

    /// Default weight of this rule's findings.
    fn severity(&self) -> Severity;

    /// Short free-form category tag, e.g. `coverage`, `security`, `scheduling` (parity-rules skill).
    fn category(&self) -> &'static str;

    /// The risk tier of the fix an AI may propose for this finding, or `None` when the fix is
    /// manual only (`ai-tool-tiers` skill).
    fn fix_tier(&self) -> Option<Tier>;

    /// Compares the two inventories. Must be deterministic and free of side effects.
    fn evaluate(&self, source: &Inventory, target: &Inventory, ctx: &RuleContext) -> Vec<Finding>;
}
