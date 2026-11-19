//! Hardened outbound client for the monitor and notifiers (ADR 0016).
//!
//! * HTTP/1.1 over `hyper` + `tokio-rustls` (rustls with `ring`, TLS 1.2+, system trust roots).
//! * SSRF/DoS guards: own DNS resolution with the address check on the exact connect call,
//!   re-checked on every redirect hop; redirect, header and body caps; one total deadline;
//!   no credentials in URLs.
//! * `post_json_with` adds validated extra headers (credentials are marked sensitive, framing
//!   headers refused); `post_json_stream` hands back the body chunk by chunk with idle, total and
//!   size limits, for Server-Sent Events. A POST never follows redirects, so credentials only go
//!   to the URL given.
//! * `tls_certificate` reads the certificate of expired or self-signed servers and reports the
//!   real verification verdict (`chain_trusted`); that permissive path never carries data.
//!
//! Errors ([`NetError`]) have fixed text: no URL, host or response content. Anything returned from
//! a server (body, certificate names) is untrusted data, never instructions.

mod addr;
mod client;
mod error;
mod target;
mod tls;

pub use client::{BodyStream, Client, Headers, Policy, Response, StreamLimits};
pub use error::{NetError, TlsFailure};
pub use tls::CertInfo;
