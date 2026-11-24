//! The endpoint policy (ADR 0033): internal addresses and names are refused on shared
//! infrastructure and allowed on the owner's own network.

use std::collections::BTreeMap;

type TestResult = Result<(), Box<dyn std::error::Error>>;
use std::net::IpAddr;
use transport::{
    EndpointBlocked, EndpointPolicy, RcloneSpec, RemoteKind, SshTarget, TransportSpec, check_host,
    is_internal_ip,
};

const SHARED: EndpointPolicy = EndpointPolicy::SharedInfrastructure;
const OWN: EndpointPolicy = EndpointPolicy::OwnNetwork;

#[test]
fn internal_addresses_are_refused_in_every_spelling_that_can_be_judged() {
    for host in [
        "127.0.0.1",
        "10.1.2.3",
        "172.16.0.9",
        "172.31.255.1",
        "192.168.1.10",
        "169.254.169.254",
        "100.64.0.1",
        "0.0.0.0",
        "224.0.0.1",
        "255.255.255.255",
        "198.18.0.5",
        "::1",
        "[::1]",
        "::",
        "fe80::1",
        "fd00::1234",
        "fc00::1",
        "::ffff:127.0.0.1",
        "::ffff:10.0.0.1",
        "64:ff9b::7f00:1",
        "2002:7f00:1::1",
        "localhost",
        "LOCALHOST.",
        "app.localhost",
        "printer.lan",
        "db.internal",
        "nas.local",
        "host.home.arpa",
    ] {
        assert_eq!(
            check_host(host, SHARED),
            Err(EndpointBlocked::Internal),
            "{host}"
        );
    }
}

#[test]
fn disguised_or_single_label_hosts_are_refused_as_ambiguous() {
    for host in [
        "2130706433",
        "0x7f000001",
        "0x7f.0.0.1",
        "017700000001",
        "127.1",
        "metadata",
        "db",
    ] {
        assert_eq!(
            check_host(host, SHARED),
            Err(EndpointBlocked::Ambiguous),
            "{host}"
        );
    }
}

#[test]
fn public_hosts_pass_and_the_own_network_policy_allows_everything() {
    for host in [
        "203.0.114.7",
        "8.8.8.8",
        "2606:4700:4700::1111",
        "s3.example.org",
        "backup.example.com",
    ] {
        // 203.0.114.7 is outside the documentation block 203.0.113.0/24.
        assert_eq!(check_host(host, SHARED), Ok(()), "{host}");
    }
    for host in [
        "127.0.0.1",
        "localhost",
        "10.0.0.1",
        "nas.lan",
        "169.254.169.254",
    ] {
        assert_eq!(check_host(host, OWN), Ok(()), "{host}");
    }
}

#[test]
fn the_ip_helper_agrees_with_the_host_check() {
    for (text, internal) in [
        ("10.0.0.1", true),
        ("8.8.8.8", false),
        ("::1", true),
        ("2606:4700::1111", false),
    ] {
        let ip: IpAddr = text.parse().unwrap_or(IpAddr::from([8, 8, 4, 4]));
        assert_eq!(is_internal_ip(ip), internal, "{text}");
    }
}

fn rclone(kind: RemoteKind, key: &str, value: &str) -> Result<TransportSpec, transport::SpecError> {
    let mut options = BTreeMap::new();
    options.insert(key.to_owned(), value.to_owned());
    RcloneSpec::new(kind, "offsite", "bucket/path", options).map(TransportSpec::Rclone)
}

#[test]
fn every_endpoint_option_of_a_transport_is_checked() -> TestResult {
    let cases = [
        (
            RemoteKind::Sftp,
            "host",
            "192.168.0.5",
            Err(EndpointBlocked::Internal),
        ),
        (RemoteKind::Sftp, "host", "files.example.org", Ok(())),
        (
            RemoteKind::S3,
            "endpoint",
            "https://127.0.0.1:9000",
            Err(EndpointBlocked::Internal),
        ),
        (
            RemoteKind::S3,
            "endpoint",
            "http://s3.example.org",
            Err(EndpointBlocked::BadUrl),
        ),
        (
            RemoteKind::S3,
            "endpoint",
            "minio.lan:9000",
            Err(EndpointBlocked::Internal),
        ),
        (RemoteKind::S3, "endpoint", "https://s3.example.org", Ok(())),
        (
            RemoteKind::B2,
            "endpoint",
            "https://user@s3.example.org",
            Err(EndpointBlocked::BadUrl),
        ),
        (
            RemoteKind::Webdav,
            "url",
            "https://169.254.169.254/latest",
            Err(EndpointBlocked::Internal),
        ),
        (
            RemoteKind::Webdav,
            "url",
            "https://[::1]:8443/",
            Err(EndpointBlocked::Internal),
        ),
        (
            RemoteKind::Webdav,
            "url",
            "https://dav.example.org/remote.php",
            Ok(()),
        ),
    ];
    for (kind, key, value, expected) in cases {
        let spec = rclone(kind, key, value)?;
        assert_eq!(
            spec.check_endpoints(SHARED),
            expected,
            "{kind:?} {key}={value}"
        );
        assert_eq!(
            spec.check_endpoints(OWN),
            Ok(()),
            "{value} on the own network"
        );
    }
    Ok(())
}

#[test]
fn ssh_kinds_and_managed_follow_the_same_rule() -> TestResult {
    let internal = SshTarget::new("10.0.0.7", "backup", None, "/srv/in")?;
    let public = SshTarget::new("vault.example.org", "backup", None, "/srv/in")?;
    for make in [
        TransportSpec::Rsync,
        TransportSpec::Scp,
        TransportSpec::SshPipe,
    ] {
        assert_eq!(
            make(internal.clone()).check_endpoints(SHARED),
            Err(EndpointBlocked::Internal)
        );
        assert_eq!(make(public.clone()).check_endpoints(SHARED), Ok(()));
    }
    assert_eq!(TransportSpec::Managed.check_endpoints(SHARED), Ok(()));
    Ok(())
}
