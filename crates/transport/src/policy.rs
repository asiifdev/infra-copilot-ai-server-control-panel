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

use crate::plan::CommandPlan;
use crate::runner::{RunError, RunOutput, Runner};
use crate::spec::{RcloneSpec, RemoteKind, SshTarget, TransportSpec};
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};

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

    /// Checks every endpoint under `policy` and, under [`EndpointPolicy::SharedInfrastructure`],
    /// resolves hostnames via `resolver` and rejects any answer where [`is_internal_ip`] is true.
    ///
    /// # Errors
    /// [`EndpointBlocked`] for the first endpoint or resolved IP that the policy refuses.
    pub fn check_endpoints_with_resolver(
        &self,
        policy: EndpointPolicy,
        resolver: &dyn HostResolver,
    ) -> Result<(), EndpointBlocked> {
        if policy == EndpointPolicy::OwnNetwork {
            return Ok(());
        }
        self.check_endpoints(policy)?;
        match self {
            Self::Rclone(spec) => check_rclone_resolved(spec, policy, resolver),
            Self::Rsync(t) | Self::Scp(t) | Self::SshPipe(t) => {
                check_host_resolved(t.host(), policy, resolver).map(|_| ())
            }
            Self::Managed => Ok(()),
        }
    }

    /// Convenience wrapper over [`TransportSpec::check_endpoints_with_resolver`] using
    /// [`SystemResolver`].
    ///
    /// # Errors
    /// [`EndpointBlocked`].
    pub fn check_endpoints_resolved(&self, policy: EndpointPolicy) -> Result<(), EndpointBlocked> {
        self.check_endpoints_with_resolver(policy, &SystemResolver)
    }
}

/// Resolves a hostname to its IP addresses for [`EndpointPolicy::SharedInfrastructure`] checks.
pub trait HostResolver {
    /// Resolves `host` to one or more IP addresses.
    ///
    /// # Errors
    /// [`EndpointBlocked::Ambiguous`] when resolution fails or yields no addresses.
    fn resolve(&self, host: &str) -> Result<Vec<IpAddr>, EndpointBlocked>;
}

/// System DNS resolver using `std::net::ToSocketAddrs`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemResolver;

impl HostResolver for SystemResolver {
    fn resolve(&self, host: &str) -> Result<Vec<IpAddr>, EndpointBlocked> {
        let addrs: Vec<IpAddr> = (host, 0)
            .to_socket_addrs()
            .map_err(|_| EndpointBlocked::Ambiguous)?
            .map(|sa| sa.ip())
            .collect();
        if addrs.is_empty() {
            Err(EndpointBlocked::Ambiguous)
        } else {
            Ok(addrs)
        }
    }
}

/// Checks `host` textually and, for non-literal hostnames under
/// [`EndpointPolicy::SharedInfrastructure`], resolves it via `resolver` and checks every IP with
/// [`is_internal_ip`].
///
/// # Errors
/// [`EndpointBlocked`].
pub fn check_host_resolved(
    host: &str,
    policy: EndpointPolicy,
    resolver: &dyn HostResolver,
) -> Result<Vec<IpAddr>, EndpointBlocked> {
    if policy == EndpointPolicy::OwnNetwork {
        return Ok(Vec::new());
    }
    check_host(host, policy)?;
    let bare = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.');
    if let Ok(ip) = bare.parse::<IpAddr>() {
        return Ok(vec![ip]);
    }
    let addrs = resolver.resolve(bare)?;
    if addrs.is_empty() {
        return Err(EndpointBlocked::Ambiguous);
    }
    if addrs.iter().any(|ip| is_internal_ip(*ip)) {
        return Err(EndpointBlocked::Internal);
    }
    Ok(addrs)
}

fn check_rclone_resolved(
    spec: &RcloneSpec,
    policy: EndpointPolicy,
    resolver: &dyn HostResolver,
) -> Result<(), EndpointBlocked> {
    let options = spec.options();
    match spec.kind() {
        RemoteKind::Sftp => options.get("host").map_or(Ok(()), |h| {
            check_host_resolved(h, policy, resolver).map(|_| ())
        }),
        RemoteKind::Webdav => options.get("url").map_or(Ok(()), |u| {
            check_host_resolved(https_host(u)?, policy, resolver).map(|_| ())
        }),
        RemoteKind::S3 | RemoteKind::B2 => options.get("endpoint").map_or(Ok(()), |e| {
            let host = if e.contains("://") {
                https_host(e)?
            } else {
                e.split(['/', ':']).next().unwrap_or_default()
            };
            check_host_resolved(host, policy, resolver).map(|_| ())
        }),
        _ => Ok(()),
    }
}

/// Why [`run_with_policy`] did not complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyRunError {
    /// Refused by [`EndpointPolicy`] (including DNS resolution to an internal IP).
    Blocked(EndpointBlocked),
    /// The underlying [`Runner`] failed.
    Run(RunError),
}

impl fmt::Display for PolicyRunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Blocked(e) => e.fmt(f),
            Self::Run(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for PolicyRunError {}

/// Checks `spec` against `policy` (resolving hostnames via `resolver` when `SharedInfrastructure`)
/// before invoking `runner.run(plan)`.
///
/// # Errors
/// [`PolicyRunError::Blocked`] if the endpoint is refused (runner is never called);
/// [`PolicyRunError::Run`] if the runner fails.
pub fn run_with_policy(
    spec: &TransportSpec,
    command: &CommandPlan,
    policy: EndpointPolicy,
    resolver: &dyn HostResolver,
    runner: &dyn Runner,
) -> Result<RunOutput, PolicyRunError> {
    spec.check_endpoints_with_resolver(policy, resolver)
        .map_err(PolicyRunError::Blocked)?;
    runner.run(command).map_err(PolicyRunError::Run)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::FakeRunner;
    use std::collections::BTreeMap;

    struct MockResolver(BTreeMap<String, Vec<IpAddr>>);

    impl HostResolver for MockResolver {
        fn resolve(&self, host: &str) -> Result<Vec<IpAddr>, EndpointBlocked> {
            self.0.get(host).cloned().ok_or(EndpointBlocked::Ambiguous)
        }
    }

    fn ssh_spec(host: &str) -> TransportSpec {
        TransportSpec::Rsync(
            SshTarget::new(host, "backup", Some(22), "/srv/bak")
                .unwrap_or_else(|e| panic!("valid target: {e}")),
        )
    }

    #[test]
    fn shared_policy_blocks_dns_resolving_to_internal_or_loopback() {
        let mut map = BTreeMap::new();
        map.insert(
            "rebind.example.com".to_owned(),
            vec![IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))],
        );
        map.insert(
            "dual.example.com".to_owned(),
            vec![
                IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34)),
                IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254)),
            ],
        );
        map.insert(
            "public.example.com".to_owned(),
            vec![IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))],
        );
        let resolver = MockResolver(map);

        assert_eq!(
            ssh_spec("rebind.example.com")
                .check_endpoints_with_resolver(EndpointPolicy::SharedInfrastructure, &resolver),
            Err(EndpointBlocked::Internal)
        );
        assert_eq!(
            ssh_spec("dual.example.com")
                .check_endpoints_with_resolver(EndpointPolicy::SharedInfrastructure, &resolver),
            Err(EndpointBlocked::Internal)
        );
        assert_eq!(
            ssh_spec("missing.example.com")
                .check_endpoints_with_resolver(EndpointPolicy::SharedInfrastructure, &resolver),
            Err(EndpointBlocked::Ambiguous)
        );
        assert_eq!(
            ssh_spec("public.example.com")
                .check_endpoints_with_resolver(EndpointPolicy::SharedInfrastructure, &resolver),
            Ok(())
        );
        // OwnNetwork allows even unresolvable/internal endpoints without calling resolver.
        assert_eq!(
            ssh_spec("rebind.example.com")
                .check_endpoints_with_resolver(EndpointPolicy::OwnNetwork, &resolver),
            Ok(())
        );
    }

    #[test]
    fn run_with_policy_blocks_before_calling_runner() {
        let mut map = BTreeMap::new();
        map.insert(
            "evil.example.com".to_owned(),
            vec![IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5))],
        );
        let resolver = MockResolver(map);
        let runner = FakeRunner::new();
        runner.push(Ok(RunOutput::ok("should not run")));
        let cmd = CommandPlan {
            program: "rsync",
            args: vec!["--version".to_owned()],
            env: Vec::new(),
            stdin: None,
        };
        let res = run_with_policy(
            &ssh_spec("evil.example.com"),
            &cmd,
            EndpointPolicy::SharedInfrastructure,
            &resolver,
            &runner,
        );
        assert_eq!(res, Err(PolicyRunError::Blocked(EndpointBlocked::Internal)));
        assert!(runner.calls().is_empty());
    }
}
