//! Domain types shared by every crate. No external dependencies (see docs/decisions/0005).
//!
//! * [`tier`] - the five risk tiers and the approval rules attached to them.
//! * [`secret`] - the opaque `Secret` wrapper.
//! * [`redact`] - secret redaction, applied before anything is logged, stored, shown or sent to a model.

pub mod redact;
pub mod secret;
pub mod tier;
