//! Domain types shared by every crate. No external dependencies (see docs/decisions/0005).
//!
//! * [`entitlements`] - open-core edition tiers, limits, and quota enforcement traits.
//! * [`tier`] - the five risk tiers and the approval rules attached to them.
//! * [`secret`] - the opaque `Secret` wrapper.
//! * [`redact`] - secret redaction, applied before anything is logged, stored, shown or sent to a model.

pub mod entitlements;
pub mod redact;
pub mod secret;
pub mod tier;
