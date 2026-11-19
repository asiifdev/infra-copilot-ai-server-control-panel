//! Parity rules, the comparer and the finding renderer (docs/design.md 5, 14.1; decision 0013).
//!
//! * [`all_rules`] lists the rules; [`compare`] runs them and returns a deterministic [`Report`].
//! * [`render`] turns a [`inventory::Finding`] into localized text from the i18n catalog. Rule
//!   code never contains display text.
//!
//! Rules depend only on `inventory`; they know nothing about collectors.

mod honesty;
mod render;
mod report;
mod rules;

pub use render::{RenderedFinding, render};
pub use report::{Report, all_rules, compare};
pub use rules::{Par001, Par002, Par006, Par007, Par008, Par009, Par010, UNKNOWN_ARG};
