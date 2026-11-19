//! The async prober: maps `net` results onto the monitor's closed outcomes.

use crate::config::MonitorSpec;
use monitor::{CheckOutcome, ErrorClass, MonitorConfig, MonitorKind};
use net::{CertInfo, Client, NetError, Policy};
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Body bytes inspected for a keyword (decision 0016 suggests 64 KiB).
const KEYWORD_BODY_CAP: usize = 64 * 1024;
const SECS_PER_DAY: i64 = 86_400;

/// Performs one check. The engine holds it in an `Arc` and runs many probes concurrently.
pub trait Probe: Send + Sync + 'static {
    /// Probes `spec` at unix time `now`. Must never panic and must honour the monitor timeout.
    fn probe(&self, spec: &MonitorSpec, now: i64) -> impl Future<Output = CheckOutcome> + Send;
}

/// Probe backed by [`net::Client`].
#[derive(Debug, Clone)]
pub struct NetProber {
    client: Arc<Client>,
}

impl NetProber {
    /// Wraps a client (system trust roots via `Client::new`, or test roots).
    pub fn new(client: Arc<Client>) -> Self {
        NetProber { client }
    }

    /// Strict probe: private addresses are blocked, certificate days use the system clock.
    pub async fn probe(&self, cfg: &MonitorConfig) -> CheckOutcome {
        let spec = MonitorSpec {
            config: cfg.clone(),
            allow_private: false,
        };
        self.probe_spec(&spec, system_now()).await
    }

    /// Probe honouring `spec.allow_private`; `now` is unix seconds (for certificate days).
    pub async fn probe_spec(&self, spec: &MonitorSpec, now: i64) -> CheckOutcome {
        let cfg = &spec.config;
        let policy = Policy {
            timeout: Duration::from_secs(cfg.timeout_secs),
            max_body_bytes: KEYWORD_BODY_CAP,
            block_private_addresses: !spec.allow_private,
            // Monitoring a plain-http site is normal; https -> http redirects stay refused by net.
            allow_http: true,
            ..Policy::default()
        };
        match &cfg.kind {
            MonitorKind::Http { url, keyword, .. } => match self.client.get(url, &policy).await {
                Ok(resp) => {
                    if !cfg.kind.status_accepts(resp.status) {
                        return down(ErrorClass::UnexpectedStatus(resp.status));
                    }
                    if let Some(word) = keyword
                        && !contains(&resp.body, word.as_bytes())
                    {
                        return down(ErrorClass::KeywordMissing);
                    }
                    CheckOutcome::Up {
                        latency_ms: millis(resp.elapsed),
                    }
                }
                Err(e) => down(map_net_error(e)),
            },
            MonitorKind::Tcp { host, port } => {
                match self.client.tcp_connect(host, *port, &policy).await {
                    Ok(elapsed) => CheckOutcome::Up {
                        latency_ms: millis(elapsed),
                    },
                    Err(e) => down(map_net_error(e)),
                }
            }
            MonitorKind::TlsExpiry {
                host,
                port,
                warn_days,
            } => {
                let started = std::time::Instant::now();
                match self.client.tls_certificate(host, *port, &policy).await {
                    Ok(info) => {
                        judge_certificate(&info, now, *warn_days, millis(started.elapsed()))
                    }
                    Err(e) => down(map_net_error(e)),
                }
            }
        }
    }
}

impl Probe for NetProber {
    fn probe(&self, spec: &MonitorSpec, now: i64) -> impl Future<Output = CheckOutcome> + Send {
        self.probe_spec(spec, now)
    }
}

fn down(class: ErrorClass) -> CheckOutcome {
    CheckOutcome::Down { class }
}

fn millis(d: Duration) -> u32 {
    u32::try_from(d.as_millis()).unwrap_or(u32::MAX)
}

fn system_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or(0)
}

/// Case-sensitive byte search; an empty needle matches (validation rejects empty keywords).
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    needle.is_empty() || haystack.windows(needle.len()).any(|w| w == needle)
}

/// Certificate verdict. Expired, not yet valid or untrusted (chain or name) is `Down(TlsFailure)`;
/// a valid certificate inside the warning window is `CertExpiring`; otherwise `Up`.
fn judge_certificate(info: &CertInfo, now: i64, warn_days: u32, latency_ms: u32) -> CheckOutcome {
    let remaining = info.not_after.saturating_sub(now);
    if remaining <= 0 || info.not_before > now || !info.chain_trusted {
        return down(ErrorClass::TlsFailure);
    }
    let days_left = remaining / SECS_PER_DAY;
    if days_left <= i64::from(warn_days) {
        CheckOutcome::CertExpiring { days_left }
    } else {
        CheckOutcome::Up { latency_ms }
    }
}

/// Maps a transport error onto the closed monitor classes. `BlockedAddress` stays itself.
///
/// `NetError` is `#[non_exhaustive]`, so the last arm covers variants added later; the test
/// `every_net_error_has_a_class` lists all current ones.
pub fn map_net_error(err: NetError) -> ErrorClass {
    match err {
        NetError::Timeout => ErrorClass::Timeout,
        NetError::Dns => ErrorClass::DnsFailure,
        NetError::Connect => ErrorClass::ConnectFailed,
        NetError::Tls(_) | NetError::NoRootCertificates => ErrorClass::TlsFailure,
        NetError::BlockedAddress => ErrorClass::BlockedAddress,
        NetError::TooManyRedirects | NetError::InvalidRedirect | NetError::Protocol => {
            ErrorClass::Protocol
        }
        NetError::InvalidUrl | NetError::UserInfoNotAllowed | NetError::SchemeNotAllowed => {
            ErrorClass::Other
        }
        _ => ErrorClass::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use net::TlsFailure;

    #[test]
    fn every_net_error_has_a_class() {
        let table = [
            (NetError::InvalidUrl, ErrorClass::Other),
            (NetError::UserInfoNotAllowed, ErrorClass::Other),
            (NetError::SchemeNotAllowed, ErrorClass::Other),
            (NetError::BlockedAddress, ErrorClass::BlockedAddress),
            (NetError::Timeout, ErrorClass::Timeout),
            (NetError::TooManyRedirects, ErrorClass::Protocol),
            (NetError::InvalidRedirect, ErrorClass::Protocol),
            (NetError::Dns, ErrorClass::DnsFailure),
            (NetError::Connect, ErrorClass::ConnectFailed),
            (NetError::Protocol, ErrorClass::Protocol),
            (NetError::NoRootCertificates, ErrorClass::TlsFailure),
            (NetError::Tls(TlsFailure::Handshake), ErrorClass::TlsFailure),
            (
                NetError::Tls(TlsFailure::UntrustedIssuer),
                ErrorClass::TlsFailure,
            ),
            (NetError::Tls(TlsFailure::Expired), ErrorClass::TlsFailure),
            (
                NetError::Tls(TlsFailure::NotYetValid),
                ErrorClass::TlsFailure,
            ),
            (
                NetError::Tls(TlsFailure::NameMismatch),
                ErrorClass::TlsFailure,
            ),
            (
                NetError::Tls(TlsFailure::InvalidCertificate),
                ErrorClass::TlsFailure,
            ),
            (
                NetError::Tls(TlsFailure::MalformedCertificate),
                ErrorClass::TlsFailure,
            ),
        ];
        for (err, want) in table {
            assert_eq!(map_net_error(err), want, "{err:?}");
        }
    }

    #[test]
    fn keyword_search_handles_edges() {
        assert!(contains(b"hello world", b"world"));
        assert!(!contains(b"hello", b"world"));
        assert!(!contains(b"", b"x"));
        assert!(contains(b"abc", b""));
        assert!(!contains(b"ab", b"abc"));
    }
}
