//! Inventory model and the contracts between collectors and parity rules (docs/design.md 4, 5, 12.4).
//!
//! * [`Inventory`] is a dated snapshot of what runs on one machine: a sorted, duplicate-free list of
//!   [`Item`]s. It is plain data: serializable, comparable, and free of secret values (collectors
//!   store names and host/port, never values; see the `parity-rules` skill).
//! * [`Collector`] gathers items read-only. [`ParityRule`] is a pure function from a source and a
//!   target inventory to [`Finding`]s.
//!
//! This crate defines contracts only. Collectors live in `collectors`, rules in `parity`, so neither
//! depends on the other (decision 0013).

mod finding;
mod item;
mod snapshot;
mod traits;

pub use finding::{Finding, Severity, SubjectRef};
pub use item::{Item, Kind, SeenVia};
pub use snapshot::{Inventory, InventoryError, Platform, SCHEMA_VERSION, Status};
pub use traits::{CollectError, Collection, Collector, ParityRule, RuleContext};
