//! Where a transport may point: the endpoint policy (ADR 0033).
//!
//! On the owner's own machine a transport may point anywhere, including a LAN host or localhost
//! (they own that network). When commands run on infrastructure shared between organizations
//! (the cloud edition) the same spec would let one customer make that infrastructure connect to
//! its own internal network, a loopback service or a cloud metadata address (server-side request
//! forgery). [`EndpointPolicy::SharedInfrastructure`] refuses those.
//!
//! This module judges names and address literals only. A public-looking DNS name can still
//! resolve to a private address, so a runner that executes on shared infrastructure must resolve
//! the name itself and run [`is_internal_ip`] on every answer before connecting (and connect to
//! that address, not the name again). That runner does not exist yet (milestone M4 and later).

use crate::spec::{RcloneSpec, RemoteKind, SshTarget, TransportSpec};
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Where the commands of a transport run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointPolicy {
    /// The owner's own machine or network: no restriction on endpoints.
    OwnNetwork,
    /// Infrastructure shared between organizations: internal endpoints are refused.
    SharedInfrastructure,
}

/// Why an endpoint was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndpointBlocked {
    /// The host is a loopback, private, link-local, shared-address-space, multicast or
    /// unspecified address, or an internal name such as `localhost` or `printer.lan`.
    Internal,
    /// The host is written in a form that can hide an address (`0x7f.1`, `2130706433`, an
    /// octal part) or has no dot, so it cannot be judged by its text.
    Ambiguous,
    /// A URL option is not `https://host[:port][/path]` (a scheme other than https, user
    /// information, or no host).
    BadUrl,
}

impl fmt::Display for EndpointBlocked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Internal => "transport_endpoint_internal",
            Self::Ambiguous => "transport_endpoint_ambiguous",
            Self::BadUrl => "transport_endpoint_bad_url",
        })
    }
}

impl std::error::Error for EndpointBlocked {}

/// Whether an address is one an outside tenant must not reach: loopback, private (RFC 1918),
/// link-local (including the cloud metadata address), shared address space (RFC 6598),
/// unspecified, broadcast, multicast, documentation, benchmarking, unique-local and IPv6
/// link-local ranges, and IPv4 addresses wrapped in IPv6 (mapped, compatible, NAT64).
#[must_use]
pub fn is_internal_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_internal_v4(v4),
        IpAddr::V6(v6) => is_internal_v6(v6),
    }
}

fn is_internal_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || a == 0
        || (a == 100 && (64..=127).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 0 && c == 2)
        || (a == 198 && (b == 18 || b == 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || a >= 240
}

fn is_internal_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_internal_v4(v4);
    }
    let seg = ip.segments();
    // 64:ff9b::/96 (NAT64) and ::/96 (IPv4-compatible) carry an IPv4 address in the last 32 bits.
    let embedded = Ipv4Addr::new(
        (seg[6] >> 8) as u8,
        (seg[6] & 0xff) as u8,
        (seg[7] >> 8) as u8,
        (seg[7] & 0xff) as u8,
    );
    if seg[..6] == [0x64, 0xff9b, 0, 0, 0, 0] || seg[..6] == [0, 0, 0, 0, 0, 0] {
        return is_internal_v4(embedded) || ip.is_unspecified() || ip.is_loopback();
    }
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || (seg[0] & 0xfe00) == 0xfc00 // unique local fc00::/7
        || (seg[0] & 0xffc0) == 0xfe80 // link-local fe80::/10
        || (seg[0] & 0xffc0) == 0xfec0 // deprecated site-local fec0::/10
        || (seg[0] == 0x2001 && seg[1] == 0x0db8) // documentation
        || seg[0] == 0x2002 // 6to4: wraps an arbitrary IPv4 address
}

const INTERNAL_SUFFIXES: [&str; 9] = [
    "localhost",
    "local",
    "internal",
    "lan",
    "home",
    "corp",
    "intranet",
    "home.arpa",
    "localdomain",
];

/// Judges one host (a name or an address literal) under the policy.
///
/// # Errors
/// [`EndpointBlocked::Internal`] or [`EndpointBlocked::Ambiguous`] under
/// [`EndpointPolicy::SharedInfrastructure`]; never under [`EndpointPolicy::OwnNetwork`].
pub fn check_host(host: &str, policy: EndpointPolicy) -> Result<(), EndpointBlocked> {
    if policy == EndpointPolicy::OwnNetwork {
        return Ok(());
    }
    let bare = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.');
    if let Ok(ip) = bare.parse::<IpAddr>() {
        return if is_internal_ip(ip) {
            Err(EndpointBlocked::Internal)
        } else {
            Ok(())
        };
    }
    let lower = bare.to_ascii_lowercase();
    // Only digits, dots and hex letters/x: a number in disguise (decimal, hex, octal or short
    // forms such as `127.1`), which resolvers and some libraries read as an IPv4 address.
    if !lower.is_empty()
        && lower
            .chars()
            .all(|c| c.is_ascii_hexdigit() || c == '.' || c == 'x')
        && (lower.starts_with("0x") || lower.chars().all(|c| c.is_ascii_digit() || c == '.'))
    {
        return Err(EndpointBlocked::Ambiguous);
    }
    if !lower.contains('.') {
        // `localhost`, `db`, `metadata`: a single label resolves through the host's search path.
        return Err(if lower == "localhost" {
            EndpointBlocked::Internal
        } else {
            EndpointBlocked::Ambiguous
        });
    }
    if INTERNAL_SUFFIXES
        .iter()
        .any(|s| lower == *s || lower.ends_with(&format!(".{s}")))
    {
        return Err(EndpointBlocked::Internal);
    }
    Ok(())
}

/// The host of an `https://host[:port][/path]` URL.
fn https_host(url: &str) -> Result<&str, EndpointBlocked> {
    let rest = url
        .strip_prefix("https://")
        .ok_or(EndpointBlocked::BadUrl)?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') || authority.contains('\\') {
        return Err(EndpointBlocked::BadUrl);
    }
    let host = if let Some(inner) = authority.strip_prefix('[') {
        inner.split(']').next().unwrap_or_default()
    } else {
        authority.split(':').next().unwrap_or_default()
    };
    if host.is_empty() {
        Err(EndpointBlocked::BadUrl)
    } else {
        Ok(host)
    }
}

fn check_rclone(spec: &RcloneSpec, policy: EndpointPolicy) -> Result<(), EndpointBlocked> {
    let options = spec.options();
    match spec.kind() {
        RemoteKind::Sftp => options
            .get("host")
            .map_or(Ok(()), |h| check_host(h, policy)),
        RemoteKind::Webdav => options
            .get("url")
            .map_or(Ok(()), |u| check_host(https_host(u)?, policy)),
        RemoteKind::S3 | RemoteKind::B2 => options.get("endpoint").map_or(Ok(()), |e| {
            // rclone accepts a bare host or a URL here; a URL must be https.
            if e.contains("://") {
                check_host(https_host(e)?, policy)
            } else {
                check_host(e.split(['/', ':']).next().unwrap_or_default(), policy)
            }
        }),
        _ => Ok(()),
    }
}

fn check_ssh(target: &SshTarget, policy: EndpointPolicy) -> Result<(), EndpointBlocked> {
    check_host(target.host(), policy)
}

impl TransportSpec {
    /// Checks every endpoint this transport would connect to under `policy`.
    ///
    /// # Errors
    /// [`EndpointBlocked`] for the first endpoint the policy refuses.
    pub fn check_endpoints(&self, policy: EndpointPolicy) -> Result<(), EndpointBlocked> {
        if policy == EndpointPolicy::OwnNetwork {
            return Ok(());
        }
        match self {
            Self::Rclone(spec) => check_rclone(spec, policy),
            Self::Rsync(t) | Self::Scp(t) | Self::SshPipe(t) => check_ssh(t, policy),
            Self::Managed => Ok(()),
        }
    }
}
