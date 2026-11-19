//! Listening TCP ports, from `/proc/net/tcp` and `/proc/net/tcp6` (kind `listener`, seen via `proc`).
//!
//! Item key `tcp:<port>`, one item per port. Data (docs/design.md 4.1): `proto` (`"tcp"`), `port`,
//! `bind` (address string), `process` (name or null), `pid_hint` (number or null, informational
//! only: PIDs are not comparable across machines), `exposed` (`public` | `loopback` | `lan`).
//!
//! Only sockets in state LISTEN (`0A`) are taken. When IPv4 and IPv6 both listen on a port the
//! two are merged and the widest exposure wins (`public` > `lan` > `loopback`); `bind` is the
//! address of that widest entry (IPv4 first on a tie). Wildcard addresses and any address not known
//! to be loopback or private are `public`.
//!
//! The owning process comes from the socket inode: `/proc/<pid>/fd/*` symlinks of the form
//! `socket:[inode]`, with the name from `/proc/<pid>/comm`; the lowest pid wins when several share
//! an inode. Without permission to read other users' `fd` directories the owner stays null and a
//! warning says so. The hex addresses are the kernel's native-endian words (`0100007F` is
//! `127.0.0.1` on little-endian hosts), decoded with the host's byte order.

use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use inventory::{CollectError, Collection, Collector, Item, Kind, SeenVia};
use serde_json::{Map, Value};

use crate::procfs::{list_pids, read_comm};
use crate::rootfs::{FsError, RootFs};
use crate::util::read_warning;

const TCP4: &str = "/proc/net/tcp";
const TCP6: &str = "/proc/net/tcp6";
const MAX_TABLE_BYTES: usize = 8 * 1024 * 1024;
const MAX_SOCKETS: usize = 65_536;
const MAX_FDS_PER_PID: usize = 4_096;

/// How far a bind address reaches. Ordered from narrowest to widest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Exposure {
    Loopback,
    Lan,
    Public,
}

impl Exposure {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Loopback => "loopback",
            Self::Lan => "lan",
            Self::Public => "public",
        }
    }
}

pub(crate) fn classify(addr: &IpAddr) -> Exposure {
    match addr {
        IpAddr::V4(v4) => classify_v4(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => classify_v4(&v4),
            None => classify_v6(v6),
        },
    }
}

fn classify_v4(addr: &Ipv4Addr) -> Exposure {
    if addr.is_unspecified() {
        Exposure::Public
    } else if addr.is_loopback() {
        Exposure::Loopback
    } else if addr.is_private() || addr.is_link_local() || is_shared_address_space(addr) {
        Exposure::Lan
    } else {
        Exposure::Public
    }
}

/// 100.64.0.0/10 (RFC 6598 carrier-grade NAT, also used by Tailscale): not reachable from the
/// public internet, so it counts as `lan`.
fn is_shared_address_space(addr: &Ipv4Addr) -> bool {
    let [first, second, ..] = addr.octets();
    first == 100 && second & 0xc0 == 64
}

fn classify_v6(addr: &Ipv6Addr) -> Exposure {
    let first = addr.segments().first().copied().unwrap_or(0);
    if addr.is_unspecified() {
        Exposure::Public
    } else if addr.is_loopback() {
        Exposure::Loopback
    } else if first & 0xffc0 == 0xfe80 || first & 0xfe00 == 0xfc00 {
        Exposure::Lan
    } else {
        Exposure::Public
    }
}

/// One LISTEN row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ListeningSocket {
    pub(crate) addr: IpAddr,
    pub(crate) port: u16,
    pub(crate) inode: u64,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ParsedTable {
    pub(crate) sockets: Vec<ListeningSocket>,
    /// Rows that could not be understood (too short, bad hex).
    pub(crate) malformed: usize,
}

fn parse_addr(hex: &str, ipv6: bool) -> Option<IpAddr> {
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    if ipv6 {
        if hex.len() != 32 {
            return None;
        }
        let mut bytes = [0u8; 16];
        for (index, target) in bytes.iter_mut().enumerate() {
            let start = (index / 4) * 8;
            let value = u32::from_str_radix(hex.get(start..start + 8)?, 16).ok()?;
            *target = *value.to_ne_bytes().get(index % 4)?;
        }
        Some(IpAddr::V6(Ipv6Addr::from(bytes)))
    } else {
        if hex.len() != 8 {
            return None;
        }
        let value = u32::from_str_radix(hex, 16).ok()?;
        Some(IpAddr::V4(Ipv4Addr::from(value.to_ne_bytes())))
    }
}

fn parse_port(hex: &str) -> Option<u16> {
    if hex.len() != 4 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u16::from_str_radix(hex, 16).ok()
}

/// Parses the text of `/proc/net/tcp` (`ipv6 = false`) or `/proc/net/tcp6` (`ipv6 = true`),
/// keeping LISTEN rows only. Never panics.
pub(crate) fn parse_net_table(text: &str, ipv6: bool) -> ParsedTable {
    let mut parsed = ParsedTable::default();
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let Some(first) = fields.first() else {
            continue;
        };
        if *first == "sl" {
            continue;
        }
        let (Some(local), Some(state), Some(inode)) = (fields.get(1), fields.get(3), fields.get(9))
        else {
            parsed.malformed += 1;
            continue;
        };
        if !state.eq_ignore_ascii_case("0A") {
            continue;
        }
        let row = local.rsplit_once(':').and_then(|(addr, port)| {
            Some(ListeningSocket {
                addr: parse_addr(addr, ipv6)?,
                port: parse_port(port)?,
                inode: inode.parse().ok()?,
            })
        });
        match row {
            Some(socket) if parsed.sockets.len() < MAX_SOCKETS => parsed.sockets.push(socket),
            Some(_) => {}
            None => parsed.malformed += 1,
        }
    }
    parsed
}

/// `socket:[12345]` to `12345`.
fn parse_socket_link(target: &str) -> Option<u64> {
    target
        .strip_prefix("socket:[")?
        .strip_suffix(']')?
        .parse()
        .ok()
}

#[derive(Debug)]
struct PortEntry {
    bind: IpAddr,
    exposed: Exposure,
    inodes: Vec<u64>,
}

/// Collector of listening TCP ports.
#[derive(Debug, Clone)]
pub struct Listeners {
    root: RootFs,
}

impl Listeners {
    /// A collector reading from `root`.
    #[must_use]
    pub fn new(root: RootFs) -> Self {
        Self { root }
    }

    fn read_tables(&self, warnings: &mut Vec<String>) -> Vec<ListeningSocket> {
        let mut sockets = Vec::new();
        let mut v4_readable = false;
        for (path, ipv6) in [(TCP4, false), (TCP6, true)] {
            match self.root.read_to_string(path, MAX_TABLE_BYTES) {
                Ok(file) => {
                    if !ipv6 {
                        v4_readable = true;
                    }
                    if file.truncated {
                        warnings.push(format!(
                            "{path} is larger than {MAX_TABLE_BYTES} bytes; the rest was not read"
                        ));
                    }
                    let parsed = parse_net_table(&file.text, ipv6);
                    if parsed.malformed > 0 {
                        warnings.push(format!("{} unparsable row(s) in {path}", parsed.malformed));
                    }
                    sockets.extend(parsed.sockets);
                }
                // IPv6 disabled in the kernel: the file legitimately does not exist.
                Err(FsError::NotFound) if ipv6 && v4_readable => {}
                Err(err) => warnings.push(read_warning(path, &err)),
            }
        }
        sockets
    }

    /// Maps wanted socket inodes to the lowest owning pid.
    fn find_owners(
        &self,
        wanted: &BTreeSet<u64>,
        warnings: &mut Vec<String>,
    ) -> BTreeMap<u64, u32> {
        let mut owners: BTreeMap<u64, u32> = BTreeMap::new();
        if wanted.is_empty() {
            return owners;
        }
        let mut denied = 0usize;
        for pid in list_pids(&self.root, warnings) {
            if owners.len() == wanted.len() {
                break;
            }
            let dir = format!("/proc/{pid}/fd");
            let names = match self.root.read_dir(&dir, MAX_FDS_PER_PID) {
                Ok(listing) => listing.names,
                Err(FsError::PermissionDenied) => {
                    denied += 1;
                    continue;
                }
                // Process gone, or something unexpected in its place: nothing to learn.
                Err(_) => continue,
            };
            for name in names {
                let Ok(target) = self.root.read_link(&format!("{dir}/{name}")) else {
                    continue;
                };
                if let Some(inode) = parse_socket_link(&target)
                    && wanted.contains(&inode)
                {
                    owners.entry(inode).or_insert(pid);
                }
            }
        }
        if denied > 0 && owners.len() < wanted.len() {
            warnings.push(format!(
                "could not read open sockets of {denied} process(es) (permission denied); \
                 the owning process of some listeners is unknown"
            ));
        }
        owners
    }

    fn process_name(&self, pid: u32) -> Option<String> {
        let name = read_comm(&self.root, pid).ok()?;
        (!name.is_empty()).then_some(name)
    }
}

impl Collector for Listeners {
    fn kind(&self) -> &'static str {
        "listeners"
    }

    fn run(&self) -> Result<Collection, CollectError> {
        self.root
            .validate()
            .map_err(|err| CollectError(format!("root filesystem: {err}")))?;
        let mut warnings = Vec::new();
        let sockets = self.read_tables(&mut warnings);

        let wanted: BTreeSet<u64> = sockets
            .iter()
            .map(|s| s.inode)
            .filter(|inode| *inode != 0)
            .collect();
        let owners = self.find_owners(&wanted, &mut warnings);

        let mut ports: BTreeMap<u16, PortEntry> = BTreeMap::new();
        for socket in sockets {
            let exposed = classify(&socket.addr);
            let entry = ports.entry(socket.port).or_insert_with(|| PortEntry {
                bind: socket.addr,
                exposed,
                inodes: Vec::new(),
            });
            if exposed > entry.exposed {
                entry.bind = socket.addr;
                entry.exposed = exposed;
            }
            entry.inodes.push(socket.inode);
        }

        let items = ports
            .into_iter()
            .map(|(port, entry)| {
                let pid = entry
                    .inodes
                    .iter()
                    .filter_map(|inode| owners.get(inode).copied())
                    .min();
                let process = pid.and_then(|pid| self.process_name(pid));
                let mut data = Map::new();
                data.insert("proto".into(), Value::from("tcp"));
                data.insert("port".into(), Value::from(port));
                data.insert("bind".into(), Value::from(entry.bind.to_string()));
                data.insert("process".into(), process.map_or(Value::Null, Value::from));
                data.insert("pid_hint".into(), pid.map_or(Value::Null, Value::from));
                data.insert("exposed".into(), Value::from(entry.exposed.as_str()));
                Item {
                    kind: Kind::Listener,
                    key: format!("tcp:{port}"),
                    data,
                    seen_via: SeenVia::Proc,
                }
            })
            .collect();
        Ok(Collection { items, warnings })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n";

    fn row(local: &str, state: &str, inode: u64) -> String {
        format!(
            "   0: {local} 00000000:0000 {state} 00000000:00000000 00:00000000 00000000     0        0 {inode} 1 0000000000000000 100 0 0 10 0\n"
        )
    }

    #[cfg(target_endian = "little")]
    #[test]
    fn decodes_ipv4_hex() {
        assert_eq!(
            parse_addr("0100007F", false),
            Some(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)))
        );
        assert_eq!(
            parse_addr("017100CB", false),
            Some(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 1)))
        );
        assert_eq!(parse_port("1F90"), Some(8080));
    }

    #[cfg(target_endian = "little")]
    #[test]
    fn decodes_ipv6_hex() {
        assert_eq!(
            parse_addr("00000000000000000000000001000000", true),
            Some(IpAddr::V6(Ipv6Addr::LOCALHOST))
        );
        assert_eq!(
            parse_addr("00000000000000000000000000000000", true),
            Some(IpAddr::V6(Ipv6Addr::UNSPECIFIED))
        );
        assert_eq!(
            parse_addr("0000000000000000FFFF00000100007F", true),
            Some(IpAddr::V6(Ipv4Addr::LOCALHOST.to_ipv6_mapped()))
        );
    }

    #[test]
    fn rejects_bad_hex() {
        for bad in [
            "",
            "0100007",
            "0100007FF",
            "0100007G",
            "+100007F",
            "ÿÿÿÿÿÿÿÿ",
        ] {
            assert_eq!(parse_addr(bad, false), None, "{bad:?}");
        }
        assert_eq!(parse_addr("0100007F", true), None);
        assert_eq!(parse_port("1F9"), None);
        assert_eq!(parse_port("+F90"), None);
    }

    #[test]
    fn classifies_exposure() {
        let cases = [
            ("127.0.0.1", Exposure::Loopback),
            ("127.9.9.9", Exposure::Loopback),
            ("::1", Exposure::Loopback),
            ("::ffff:127.0.0.1", Exposure::Loopback),
            ("10.1.2.3", Exposure::Lan),
            ("172.16.0.1", Exposure::Lan),
            ("172.31.255.255", Exposure::Lan),
            ("192.168.1.1", Exposure::Lan),
            ("169.254.1.1", Exposure::Lan),
            ("fe80::1", Exposure::Lan),
            ("fd00::1", Exposure::Lan),
            ("0.0.0.0", Exposure::Public),
            ("::", Exposure::Public),
            ("100.64.0.1", Exposure::Lan),
            ("100.127.255.255", Exposure::Lan),
            ("100.63.255.255", Exposure::Public),
            ("100.128.0.1", Exposure::Public),
            ("172.32.0.1", Exposure::Public),
            ("203.0.113.7", Exposure::Public),
            ("2001:db8::1", Exposure::Public),
        ];
        for (text, expected) in cases {
            let addr: IpAddr = text.parse().unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
            assert_eq!(classify(&addr), expected, "{text}");
        }
    }

    #[test]
    fn keeps_only_listen_rows_and_counts_malformed() {
        let text = format!(
            "{HEADER}{}{}{}garbage\n   1: 0100007F:ZZZZ 00000000:0000 0A x x x x x 5\n",
            row("0100007F:1F90", "0A", 11),
            row("0100007F:1F91", "01", 12),
            row("0100007F:1F92", "0a", 13),
        );
        let parsed = parse_net_table(&text, false);
        let ports: Vec<u16> = parsed.sockets.iter().map(|s| s.port).collect();
        assert_eq!(ports, vec![8080, 8082]);
        assert_eq!(parsed.malformed, 2);
    }

    #[test]
    fn socket_link_parsing() {
        assert_eq!(parse_socket_link("socket:[12345]"), Some(12345));
        for bad in [
            "socket:[]",
            "socket:[x]",
            "pipe:[1]",
            "/dev/null",
            "socket:[1",
        ] {
            assert_eq!(parse_socket_link(bad), None, "{bad}");
        }
    }
}
