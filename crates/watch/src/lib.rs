//! Glue between the pure monitor engine, the hardened `net` client and the `notifier` channels.
//!
//! * [`NetProber`] runs one check with `net` and returns a closed [`monitor::CheckOutcome`].
//!   Private addresses are blocked unless the monitor sets `allow_private` (owner's own machines).
//! * [`NetTransport`] carries Telegram requests over `net`; it never keeps or returns the URL
//!   (the bot token is part of it).
//! * [`Engine`] probes due monitors with bounded concurrency, feeds the state machines, renders
//!   alerts in the channel locale and delivers them (severity and quiet hours, throttle, one
//!   retry on a short rate limit). Reports and errors are closed enums.
//! * [`WatchConfig`] is the standalone config. The bot token is never in it: `token_ref` names a
//!   secret that a [`SecretSource`] resolves at send time. [`EnvSecrets`] (environment variable)
//!   is a stop-gap; the product keeps secrets in the encrypted database.
//!
//! No database persistence here: [`Engine::snapshot`] and [`Engine::restore`] expose serde state.

mod clock;
mod config;
mod engine;
mod error;
mod probe;
mod secret;
mod transport;

pub use clock::{Clock, FakeClock, SystemClock};
pub use config::{ChannelSpec, MAX_MONITORS, MinSeverity, MonitorSpec, QuietSpec, WatchConfig};
pub use engine::{
    AlertReport, Delivery, Engine, EngineSnapshot, ProbeReport, RunReport, map_error_class,
    to_event,
};
pub use error::WatchError;
pub use probe::{NetProber, Probe, map_net_error};
pub use secret::{EnvSecrets, SecretError, SecretSource};
pub use transport::{NetTransport, map_transport_error};
