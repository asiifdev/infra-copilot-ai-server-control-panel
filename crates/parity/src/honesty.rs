//! The shared "honesty" rule: an incomplete inventory is not proof of absence (decision 0013,
//! design 3.3).
//!
//! It applies only to rules that claim something is MISSING (PAR-001, PAR-008). A rule that reports
//! something that was actually seen (PAR-010: a visible public listener) must not use it: what was
//! observed exists no matter how incomplete the rest of the inventory is.

use inventory::{Finding, Inventory, Severity, Status};

/// Marker key in `Finding::args` that tells the renderer to add the "partial" note.
pub(crate) const PARTIAL_ARG: &str = "partial";

/// For an absence claim: when `inventory` is partial, caps the finding at [`Severity::Info`] and flags it with
/// `args["partial"] = "true"`. Complete inventories leave the finding untouched.
pub(crate) fn cap_if_partial(mut finding: Finding, inventory: &Inventory) -> Finding {
    if inventory.status == Status::Partial {
        finding.severity = finding.severity.min(Severity::Info);
        finding.args.insert(PARTIAL_ARG.into(), "true".into());
    }
    finding
}
