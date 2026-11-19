//! Pure monitoring logic (docs/plan.md 12.2): no async, no network, no clock.
//!
//! * [`MonitorConfig`] describes one check and validates itself with a closed [`ConfigError`].
//! * A [`Prober`] turns a config into a [`CheckOutcome`]. Outcomes carry only closed enums and numbers,
//!   never text from the probed server (text from servers is data, not for display).
//! * [`MonitorState::observe`] is the state machine: it takes the outcome and the current time as unix
//!   seconds and returns [`Transition`]s (incident opened, still down, recovered, certificate alerts).
//! * [`MonitorState::uptime`] computes [`UptimeStats`] from incident intervals.
//!
//! All time arithmetic is saturating and a clock that goes backwards never panics or yields negative
//! values. Real probers (HTTP/TLS) and notification delivery live in other crates.

mod config;
mod outcome;
mod state;
mod uptime;

pub use config::{ConfigError, MaintenanceWindow, MonitorConfig, MonitorKind};
pub use outcome::{CheckOutcome, ErrorClass, FakeProber, Prober};
pub use state::{Incident, MAX_HISTORY, MonitorState, OpenIncident, Transition};
pub use uptime::{UptimeStats, uptime_from_intervals};
