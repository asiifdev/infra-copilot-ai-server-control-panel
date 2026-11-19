//! Address classifier for the SSRF guard. Applied to the exact `SocketAddr` passed to `connect`.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// True when `ip` must not be contacted while `block_private_addresses` is set.
///
/// Covers loopback, private (RFC 1918, CGNAT, ULA), link-local (incl. 169.254.169.254),
/// unspecified, multicast, broadcast, reserved, documentation and benchmarking ranges, and IPv6
/// forms that embed an IPv4 address (IPv4-mapped, IPv4-compatible, NAT64, 6to4), judged by the
/// embedded address.
pub fn is_blocked(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => blocked_v4(v4),
        IpAddr::V6(v6) => blocked_v6(v6),
    }
}

fn blocked_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || a == 0 // 0.0.0.0/8 "this network"
        || (a == 100 && (b & 0xc0) == 64) // 100.64.0.0/10 CGNAT
        || (a == 192 && b == 0 && c == 0) // 192.0.0.0/24 IETF protocol assignments
        || (a == 198 && (b & 0xfe) == 18) // 198.18.0.0/15 benchmarking
        || a >= 240 // 240.0.0.0/4 reserved (incl. broadcast)
}

fn blocked_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = embedded_v4(ip) {
        return blocked_v4(v4);
    }
    let seg = ip.segments();
    ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast() // ff00::/8
        || (seg[0] & 0xfe00) == 0xfc00 // fc00::/7 ULA
        || (seg[0] & 0xffc0) == 0xfe80 // fe80::/10 link-local
        || (seg[0] & 0xffc0) == 0xfec0 // fec0::/10 deprecated site-local
        || (seg[0] == 0x2001 && seg[1] == 0x0db8) // 2001:db8::/32 documentation
        || (seg[0] == 0x2001 && seg[1] == 0) // 2001::/32 Teredo (embeds an obfuscated v4)
        || (seg[0] == 0x0100 && seg[1] == 0 && seg[2] == 0 && seg[3] == 0) // 100::/64 discard
}

/// The IPv4 address hidden inside an IPv6 address, if it uses a form that embeds one.
fn embedded_v4(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    let s = ip.segments();
    let v4 = |hi: u16, lo: u16| {
        let [a, b] = hi.to_be_bytes();
        let [c, d] = lo.to_be_bytes();
        Ipv4Addr::new(a, b, c, d)
    };
    let zero_prefix = s[..5].iter().all(|&x| x == 0);
    if zero_prefix && s[5] == 0xffff {
        return Some(v4(s[6], s[7])); // ::ffff:a.b.c.d (IPv4-mapped)
    }
    if zero_prefix && s[5] == 0 && (s[6] != 0 || s[7] > 1) {
        return Some(v4(s[6], s[7])); // ::a.b.c.d (IPv4-compatible); :: and ::1 are handled apart
    }
    if s[0] == 0x0064 && s[1] == 0xff9b && s[2..6].iter().all(|&x| x == 0) {
        return Some(v4(s[6], s[7])); // 64:ff9b::/96 NAT64
    }
    if s[0] == 0x2002 {
        return Some(v4(s[1], s[2])); // 2002::/16 6to4
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifier_table() {
        let blocked = [
            // IPv4
            "0.0.0.0",
            "0.1.2.3",
            "127.0.0.1",
            "127.255.255.254",
            "10.0.0.1",
            "10.255.255.255",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.0.1",
            "100.64.0.1",
            "100.127.255.255",
            "169.254.0.1",
            "169.254.169.254",
            "192.0.0.8",
            "192.0.2.1",
            "198.51.100.1",
            "203.0.113.1",
            "198.18.0.1",
            "198.19.255.255",
            "224.0.0.1",
            "239.255.255.255",
            "240.0.0.1",
            "255.255.255.255",
            // IPv6
            "::",
            "::1",
            "fc00::1",
            "fd12:3456:789a::1",
            "fe80::1",
            "febf::1",
            "fec0::1",
            "ff02::1",
            "2001:db8::1",
            "2001:0:1234::1",
            "100::1",
            // embedded IPv4
            "::ffff:127.0.0.1",
            "::ffff:10.1.2.3",
            "::ffff:169.254.169.254",
            "::ffff:192.168.1.1",
            "::ffff:100.64.0.1",
            "::ffff:0.0.0.0",
            "::7f00:1",
            "64:ff9b::a00:1",
            "64:ff9b::7f00:1",
            "2002:7f00:1::1",
            "2002:a9fe:a9fe::1",
        ];
        for text in blocked {
            let ip: IpAddr = text.parse().unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
            assert!(is_blocked(ip), "{text} must be blocked");
        }
        let allowed = [
            "1.1.1.1",
            "8.8.8.8",
            "93.184.216.34",
            "100.63.255.255",
            "100.128.0.1",
            "172.15.255.255",
            "172.32.0.1",
            "169.253.1.1",
            "198.17.255.255",
            "198.20.0.1",
            "223.255.255.255",
            "2606:4700:4700::1111",
            "2001:4860:4860::8888",
            "2001:1::1",
            "fbff::1",
            "::ffff:8.8.8.8",
            "64:ff9b::808:808",
            "2002:808:808::1",
        ];
        for text in allowed {
            let ip: IpAddr = text.parse().unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
            assert!(!is_blocked(ip), "{text} must be allowed");
        }
    }
}
