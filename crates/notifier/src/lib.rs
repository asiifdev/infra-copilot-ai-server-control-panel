//! Notification channels: the `NotificationChannel` contract, alert rendering, send policy
//! helpers (severity, quiet hours, throttle) and a send-only Telegram channel.
//!
//! M1 scope: one-way sending only. Interactive Telegram commands, buttons and approvals arrive
//! later together with Proposals (the `ChatBotChannel` of docs/design.md 12.4); this crate only
//! contains the pure `/start <code>` parser needed for linking a chat.
//!
//! Security rules enforced here (project rules 2 and 4):
//! - the bot token is an opaque [`Secret`], injected at send time, never in an error, `Debug`
//!   or log string;
//! - alert content is closed enums, numbers and sanitised names, never free text from a probed
//!   server;
//! - network access sits behind [`HttpTransport`]; this crate has no HTTP client.
//!
//! The channel traits return `impl Future` and are not object safe: dispatch with generics or
//! an enum of concrete channels.

mod channel;
mod event;
mod policy;
mod render;
pub mod telegram;
pub mod testing;

pub use channel::{
    ChannelConfig, ConfigError, HttpTransport, Message, NotificationChannel, QuietHours, Secret,
    SendError, SendReceipt, Severity, TransportError,
};
pub use event::{AlertDetail, AlertEvent, AlertKind, ErrorClass, sanitise_target, sanitise_text};
pub use policy::{Throttle, should_send};
pub use render::{format_utc, render, render_test};
