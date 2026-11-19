//! Collectors against anonymous fixture trees and hostile or broken inputs.

use std::fs;
use std::path::{Path, PathBuf};

use collectors::{Cron, FsError, Listeners, Processes, RootFs, collect_inventory};
use inventory::{Collector, Inventory, Item, Kind, Platform, SeenVia, Status};
use serde_json::Value;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn fixture(name: &str) -> RootFs {
    RootFs::new(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
}

fn platform() -> Platform {
    Platform {
        os: "linux".into(),
        arch: "x86_64".into(),
    }
}

fn inventory_of(root: &RootFs) -> Result<Inventory, Box<dyn std::error::Error>> {
    Ok(collect_inventory(
        root,
        "2026-01-01T00:00:00Z".into(),
        platform(),
    )?)
}

fn item<'a>(inv: &'a Inventory, kind: Kind, key: &str) -> Result<&'a Item, String> {
    inv.get(kind, key).ok_or_else(|| format!("missing {key}"))
}

/// A scratch tree under the cargo target directory, removed on drop.
struct Tree {
    root: PathBuf,
}

impl Tree {
    fn new(name: &str) -> Result<Self, std::io::Error> {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("collectors-{name}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    fn write(&self, rel: &str, bytes: &[u8]) -> Result<(), std::io::Error> {
        let path = self.root.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, bytes)
    }

    fn fs(&self) -> RootFs {
        RootFs::new(&self.root)
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[cfg(target_endian = "little")]
mod listeners {
    use super::*;

    #[test]
    fn merges_v4_and_v6_and_classifies_exposure() -> TestResult {
        let inv = inventory_of(&fixture("full"))?;
        let ports: Vec<&str> = inv
            .items_of(Kind::Listener)
            .map(|i| i.key.as_str())
            .collect();
        assert_eq!(
            ports,
            [
                "tcp:22", "tcp:5353", "tcp:5432", "tcp:80", "tcp:8080", "tcp:9090"
            ]
        );

        // 127.0.0.1 (v4) and :: (v6) on 5432: the widest wins.
        let db = item(&inv, Kind::Listener, "tcp:5432")?;
        assert_eq!(db.str_field("exposed"), Some("public"));
        assert_eq!(db.str_field("bind"), Some("::"));
        assert_eq!(db.str_field("proto"), Some("tcp"));
        assert_eq!(db.u64_field("port"), Some(5432));
        assert_eq!(db.seen_via, SeenVia::Proc);

        assert_eq!(
            item(&inv, Kind::Listener, "tcp:22")?.str_field("exposed"),
            Some("lan")
        );
        assert_eq!(
            item(&inv, Kind::Listener, "tcp:80")?.str_field("exposed"),
            Some("public")
        );
        assert_eq!(
            item(&inv, Kind::Listener, "tcp:80")?.str_field("bind"),
            Some("203.0.113.1")
        );
        let ll = item(&inv, Kind::Listener, "tcp:5353")?;
        assert_eq!(ll.str_field("exposed"), Some("lan"));
        assert_eq!(ll.str_field("bind"), Some("fe80::1"));
        let w = item(&inv, Kind::Listener, "tcp:9090")?;
        assert_eq!(w.str_field("exposed"), Some("loopback"));
        assert_eq!(w.str_field("bind"), Some("::1"));
        Ok(())
    }

    #[test]
    fn maps_inode_to_process_and_leaves_unknown_null() -> TestResult {
        let inv = inventory_of(&fixture("full"))?;
        let web = item(&inv, Kind::Listener, "tcp:8080")?;
        assert_eq!(web.str_field("process"), Some("webapp"));
        assert_eq!(web.u64_field("pid_hint"), Some(100)); // pid 101 shares the inode; lowest wins
        assert_eq!(web.str_field("bind"), Some("0.0.0.0"));
        assert_eq!(
            item(&inv, Kind::Listener, "tcp:5432")?.u64_field("pid_hint"),
            Some(200)
        );
        assert_eq!(
            item(&inv, Kind::Listener, "tcp:9090")?.str_field("process"),
            Some("worker")
        );
        let ssh = item(&inv, Kind::Listener, "tcp:22")?;
        assert_eq!(ssh.data.get("process"), Some(&Value::Null));
        assert_eq!(ssh.data.get("pid_hint"), Some(&Value::Null));
        Ok(())
    }

    #[test]
    fn established_rows_are_ignored() -> TestResult {
        let inv = inventory_of(&fixture("full"))?;
        assert!(inv.get(Kind::Listener, "tcp:50000").is_none());
        Ok(())
    }
}

mod processes {
    use super::*;

    #[test]
    fn merges_same_name_and_skips_kernel_threads() -> TestResult {
        let inv = inventory_of(&fixture("full"))?;
        let keys: Vec<&str> = inv
            .items_of(Kind::Process)
            .map(|i| i.key.as_str())
            .collect();
        assert_eq!(keys, ["proc:dbserver", "proc:webapp", "proc:worker"]);

        let web = item(&inv, Kind::Process, "proc:webapp")?;
        assert_eq!(web.u64_field("instances"), Some(2));
        assert_eq!(web.str_field("cwd"), Some("/srv/app")); // lowest pid (100), not 101
        assert!(
            web.str_field("cmd")
                .is_some_and(|c| c.starts_with("/opt/app/webapp --port 8080"))
        );
        assert_eq!(web.str_field("mode"), Some("unknown"));
        assert_eq!(web.data.get("supervised"), Some(&Value::Null));
        assert_eq!(web.data.get("env_refs"), Some(&Value::Array(Vec::new())));
        assert_eq!(web.seen_via, SeenVia::Proc);

        // No cwd link in the fixture: null, not guessed.
        assert_eq!(
            item(&inv, Kind::Process, "proc:worker")?.data.get("cwd"),
            Some(&Value::Null)
        );
        Ok(())
    }

    #[test]
    fn secrets_never_reach_the_output() -> TestResult {
        let inv = inventory_of(&fixture("full"))?;
        let json = inv.to_json()?;
        for secret in ["hunter2", "u:pw@", "pw@h", "abc123secret", "x:pw@"] {
            assert!(!json.contains(secret), "{secret} leaked");
        }
        assert!(json.contains("[REDACTED]"));
        Ok(())
    }

    #[test]
    fn environ_is_never_read() -> TestResult {
        let tree = Tree::new("environ")?;
        tree.write("proc/7/comm", b"app\n")?;
        tree.write("proc/7/cmdline", b"app\0")?;
        // A directory named environ would make any read attempt fail loudly as a warning.
        fs::create_dir_all(tree.root.join("proc/7/environ"))?;
        let collection = Processes::new(tree.fs()).run()?;
        assert!(collection.warnings.is_empty(), "{:?}", collection.warnings);
        assert_eq!(collection.items.len(), 1);
        Ok(())
    }
}

mod cron {
    use super::*;

    #[test]
    fn system_and_user_tables() -> TestResult {
        let inv = inventory_of(&fixture("full"))?;
        let keys: Vec<&str> = inv.items_of(Kind::Cron).map(|i| i.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "system:root:0 3 * * *:backup",
                "system:root:17 * * * *:run-parts",
                "system:root:25 6 * * *:test",
                "system:root:@reboot:startup.sh",
                "system:www-data:*/10 * * * *:queue-run",
                "user:alice:*/5 * * * *:curl",
                "user:alice:@daily:cleanup.sh",
                "user:root:0 3 * * *:backup",
            ]
        );
        let user_root = item(&inv, Kind::Cron, "user:root:0 3 * * *:backup")?;
        assert_eq!(user_root.str_field("owner"), Some("root"));
        assert_eq!(user_root.str_field("source"), Some("crontab"));
        assert_eq!(user_root.str_field("schedule"), Some("0 3 * * *"));
        assert_eq!(
            user_root.str_field("command_hint"),
            Some("/usr/local/bin/backup")
        );
        assert_eq!(user_root.seen_via, SeenVia::Config);
        Ok(())
    }

    #[test]
    fn duplicates_merge_and_assignments_and_hidden_files_are_ignored() -> TestResult {
        let inv = inventory_of(&fixture("full"))?;
        let merged = item(&inv, Kind::Cron, "system:root:0 3 * * *:backup")?;
        assert_eq!(merged.u64_field("occurrences"), Some(2));
        assert_eq!(
            item(&inv, Kind::Cron, "user:root:0 3 * * *:backup")?.u64_field("occurrences"),
            Some(1)
        );
        assert!(
            inv.items_of(Kind::Cron)
                .all(|i| !i.key.contains("nope") && !i.key.contains("SHELL"))
        );
        Ok(())
    }

    #[test]
    fn cron_commands_are_redacted() -> TestResult {
        let inv = inventory_of(&fixture("full"))?;
        let cleanup = item(&inv, Kind::Cron, "user:alice:@daily:cleanup.sh")?;
        assert!(
            cleanup
                .str_field("command_hint")
                .is_some_and(|c| !c.contains("hunter2"))
        );
        let curl = item(&inv, Kind::Cron, "user:alice:*/5 * * * *:curl")?;
        assert!(
            curl.str_field("command_hint")
                .is_some_and(|c| !c.contains("pw@"))
        );
        Ok(())
    }
}

mod warnings {
    use super::*;

    #[test]
    fn unparsable_cron_line_warns_without_copying_it() -> TestResult {
        let inv = inventory_of(&fixture("full"))?;
        assert_eq!(inv.status, Status::Partial);
        assert!(
            inv.warnings.iter().any(|w| w.starts_with("cron:")
                && w.contains("1 unparsable line(s) in /var/spool/cron/crontabs/alice")),
            "{:?}",
            inv.warnings
        );
        assert!(inv.warnings.iter().all(|w| !w.contains("not a cron line")));
        Ok(())
    }

    #[test]
    fn missing_sources_are_warnings_not_silence() -> TestResult {
        let inv = inventory_of(&fixture("empty"))?;
        assert_eq!(inv.status, Status::Partial);
        assert!(inv.items().is_empty());
        for needle in [
            "listeners: cannot read /proc/net/tcp: not found",
            "listeners: cannot read /proc/net/tcp6: not found",
            "no cron configuration found",
        ] {
            assert!(
                inv.warnings.iter().any(|w| w.contains(needle)),
                "{needle}: {:?}",
                inv.warnings
            );
        }
        assert!(
            inv.warnings
                .iter()
                .any(|w| w.contains("cannot read /proc: not found"))
        );
        Ok(())
    }

    #[test]
    fn unreadable_file_types_warn() -> TestResult {
        let tree = Tree::new("wrongtype")?;
        fs::create_dir_all(tree.root.join("proc/net/tcp"))?; // a directory where a file belongs
        fs::create_dir_all(tree.root.join("etc/crontab"))?;
        let listeners = Listeners::new(tree.fs()).run()?;
        assert!(
            listeners
                .warnings
                .iter()
                .any(|w| w.contains("/proc/net/tcp: unexpected file type"))
        );
        let cron = Cron::new(tree.fs()).run()?;
        assert!(
            cron.warnings
                .iter()
                .any(|w| w.contains("/etc/crontab: unexpected file type"))
        );
        Ok(())
    }

    #[test]
    fn missing_ipv6_table_is_fine_when_ipv4_is_readable() -> TestResult {
        let tree = Tree::new("no-v6")?;
        tree.write("proc/net/tcp", b"  sl  local_address\n")?;
        fs::create_dir_all(tree.root.join("proc"))?;
        let listeners = Listeners::new(tree.fs()).run()?;
        assert!(listeners.warnings.is_empty(), "{:?}", listeners.warnings);
        Ok(())
    }

    #[test]
    fn bad_root_is_a_collect_error() {
        let root = RootFs::new(Path::new(env!("CARGO_TARGET_TMPDIR")).join("does-not-exist"));
        assert!(collect_inventory(&root, "t".into(), platform()).is_err());
        assert!(Listeners::new(root.clone()).run().is_err());
        assert!(Processes::new(root.clone()).run().is_err());
        assert!(Cron::new(root).run().is_err());
    }
}

mod rootfs {
    use super::*;

    #[test]
    fn rejects_traversal_and_relative_paths() {
        let root = fixture("full");
        for bad in [
            "/etc/../etc/crontab",
            "/../..",
            "etc/crontab",
            "/proc/net/../net/tcp",
        ] {
            assert!(
                matches!(root.read_to_string(bad, 100), Err(FsError::InvalidPath(_))),
                "{bad}"
            );
            assert!(matches!(
                root.read_dir(bad, 100),
                Err(FsError::InvalidPath(_))
            ));
            assert!(matches!(root.read_link(bad), Err(FsError::InvalidPath(_))));
        }
    }

    #[test]
    fn reads_are_bounded_and_links_not_followed() -> TestResult {
        let root = fixture("full");
        let file = root.read_to_string("/etc/crontab", 20)?;
        assert!(file.truncated);
        assert!(file.text.len() <= 20);
        assert_eq!(root.read_link("/proc/100/cwd")?, "/srv/app");
        assert_eq!(root.read_dir("/proc/net", 1)?.names.len(), 1);
        assert!(root.read_dir("/proc/net", 1)?.truncated);
        Ok(())
    }
}

mod robustness {
    use super::*;

    #[test]
    fn broken_input_never_panics() -> TestResult {
        let tree = Tree::new("broken")?;
        tree.write("proc/net/tcp", b"  sl\n   0: 0100007F:1F90 x 0A\n0: \xff\xfe:\x00 00 0A a b c d e 7\n   1: 0100007F:1F90 00000000:0000 0A 0:0 0:0 0 0 0 5\n")?;
        tree.write(
            "proc/net/tcp6",
            b"   0: 0000000000000000000000000100:1F90 0:0 0A 0:0 0:0 0 0 0 6\n",
        )?;
        tree.write("proc/5/comm", b"\xff\xfe\x01app\n")?;
        tree.write("proc/5/cmdline", b"\xff\0\xfe\0--token\0")?;
        tree.write("proc/6/comm", b"\n")?;
        tree.write("proc/6/cmdline", b"x")?;
        tree.write("proc/99999999999999999999/comm", b"big\n")?;
        tree.write(
            "etc/crontab",
            b"\xff\xfe\n* * * * *\n@\n= =\n\0\0\n61 25 * * * root \xff\n",
        )?;
        tree.write("etc/cron.d/x", b"* * * * * \n")?;
        let inv = inventory_of(&tree.fs())?;
        assert_eq!(inv.status, Status::Partial);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn symlink_loops_and_fifos_do_not_hang_or_panic() -> TestResult {
        use std::os::unix::fs::symlink;
        let tree = Tree::new("loops")?;
        fs::create_dir_all(tree.root.join("proc/net"))?;
        symlink(
            tree.root.join("proc/net/tcp"),
            tree.root.join("proc/net/tcp"),
        )?;
        fs::create_dir_all(tree.root.join("etc/cron.d"))?;
        symlink("a", tree.root.join("etc/cron.d/b"))?;
        symlink("b", tree.root.join("etc/cron.d/a"))?;
        fs::create_dir_all(tree.root.join("proc/8/fd"))?;
        symlink("/proc/8/fd/3", tree.root.join("proc/8/fd/3"))?;
        tree.write("proc/8/comm", b"loop\n")?;
        tree.write("proc/8/cmdline", b"loop\0")?;
        symlink("/proc/8/cwd", tree.root.join("proc/8/cwd"))?;
        let inv = inventory_of(&tree.fs())?;
        assert_eq!(inv.status, Status::Partial);
        Ok(())
    }

    #[test]
    fn huge_files_are_bounded() -> TestResult {
        let tree = Tree::new("huge")?;
        let line = b"   0: 0100007F:1F90 00000000:0000 0A 0:0 0:0 0 0 0 5 1\n";
        let mut big = Vec::new();
        while big.len() < 9 * 1024 * 1024 {
            big.extend_from_slice(line);
        }
        tree.write("proc/net/tcp", &big)?;
        tree.write("etc/crontab", &vec![b'#'; 3 * 1024 * 1024])?;
        let inv = inventory_of(&tree.fs())?;
        assert!(inv.warnings.iter().any(|w| w.contains("larger than")));
        Ok(())
    }

    #[test]
    fn random_garbage_never_panics() -> TestResult {
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let atoms: [&[u8]; 14] = [
            b"0A",
            b" ",
            b"\n",
            b"\t",
            b":",
            b"0100007F",
            b"00000000000000000000000001000000",
            b"* * * * *",
            b"@daily",
            b"root",
            b"=",
            b"\0",
            b"--password",
            b"socket:[1]",
        ];
        for round in 0..40 {
            let tree = Tree::new(&format!("fuzz-{round}"))?;
            for path in [
                "proc/net/tcp",
                "proc/net/tcp6",
                "etc/crontab",
                "proc/3/cmdline",
                "proc/3/comm",
                "var/spool/cron/u",
            ] {
                let mut bytes = Vec::new();
                for _ in 0..(next() % 200) {
                    if next() % 4 == 0 {
                        bytes.push((next() % 256) as u8);
                    } else {
                        bytes.extend_from_slice(atoms[(next() % atoms.len() as u64) as usize]);
                    }
                }
                tree.write(path, &bytes)?;
            }
            inventory_of(&tree.fs())?;
        }
        Ok(())
    }
}

/// Review findings 1-6: each test failed before the fix (see the report).
mod review_fixes {
    use super::*;

    fn cmdline(tree: &Tree, pid: u32, args: &[&str]) -> TestResult {
        tree.write(
            &format!("proc/{pid}/comm"),
            format!("app{pid}\n").as_bytes(),
        )?;
        let mut bytes = Vec::new();
        for arg in args {
            bytes.extend_from_slice(arg.as_bytes());
            bytes.push(0);
        }
        tree.write(&format!("proc/{pid}/cmdline"), &bytes)?;
        Ok(())
    }

    fn cmd_of(items: &[Item], pid: u32) -> String {
        items
            .iter()
            .find(|i| i.key == format!("proc:app{pid}"))
            .and_then(|i| i.str_field("cmd"))
            .unwrap_or("MISSING")
            .to_owned()
    }

    #[test]
    fn nested_shell_commands_are_redacted() -> TestResult {
        let tree = Tree::new("cron-nested")?;
        tree.write(
            "var/spool/cron/crontabs/eve",
            b"0 1 * * * bash -c 'mysql -u root -p nested1 db'\n\
              0 2 * * * sh -c \"cd /srv && curl -u admin:nested2 https://h\"\n\
              0 3 * * * sh -c 'bash -c \"mysqldump --password nested3 db\"'\n\
              0 4 * * * node --password hintleak4 app.js\n\
              0 5 * * * curl --user admin:userflag5 https://h\n\
              0 6 * * * curl --proxy-user=px:userflag6 https://h\n",
        )?;
        let json = inventory_of(&tree.fs())?.to_json()?;
        for secret in [
            "nested1",
            "nested2",
            "nested3",
            "hintleak4",
            "userflag5",
            "userflag6",
        ] {
            assert!(!json.contains(secret), "{secret} leaked:\n{json}");
        }
        Ok(())
    }

    #[test]
    fn cron_commands_hide_separate_argument_secrets() -> TestResult {
        let tree = Tree::new("cron-secrets")?;
        tree.write(
            "var/spool/cron/crontabs/bob",
            b"0 1 * * * mysqldump --password hunter2 db\n\
              0 2 * * * mysql -u root -p secret2\n\
              0 3 * * * mysql -phunter3\n\
              0 4 * * * curl -u user:pw4 https://h\n\
              0 5 * * * TOKEN=abc5 run --pass=zz6 --db-pass=zz7 --auth=zz8\n\
              0 6 * * * tool --password \"two words9\"\n",
        )?;
        let json = inventory_of(&tree.fs())?.to_json()?;
        for secret in [
            "hunter2", "secret2", "hunter3", "pw4", "abc5", "zz6", "zz7", "zz8", "words9",
        ] {
            assert!(!json.contains(secret), "{secret} leaked: {json}");
        }
        Ok(())
    }

    #[test]
    fn process_cmdlines_hide_all_secret_shapes() -> TestResult {
        let tree = Tree::new("proc-secrets")?;
        let cases: [(u32, &[&str], &[&str]); 8] = [
            (1, &["mysql", "-p", "secret1"], &["secret1"]),
            (2, &["mysql", "-psecret2"], &["secret2"]),
            (3, &["redis-cli", "-a", "secret3"], &["secret3"]),
            (4, &["curl", "-u", "user:pw4", "https://h"], &["pw4"]),
            (5, &["app", "--pass=x5"], &["x5"]),
            (6, &["app", "--db-pass=x6", "--auth=x7"], &["x6", "x7"]),
            (7, &["app", "--password", "x8", "--token=x9"], &["x8", "x9"]),
            (
                8,
                &["app", "https://u:pw10@h", "TOKEN=abc11"],
                &["pw10", "abc11"],
            ),
        ];
        for (pid, args, _) in cases {
            cmdline(&tree, pid, args)?;
        }
        let items = Processes::new(tree.fs()).run()?.items;
        for (pid, _, secrets) in cases {
            let cmd = cmd_of(&items, pid);
            for secret in secrets {
                assert!(!cmd.contains(secret), "{secret} leaked in {cmd}");
            }
        }
        // Non-secret arguments survive.
        cmdline(&tree, 9, &["app", "--port", "8080", "--host=example.test"])?;
        let items = Processes::new(tree.fs()).run()?.items;
        assert_eq!(cmd_of(&items, 9), "app --port 8080 --host=example.test");
        Ok(())
    }

    #[test]
    fn long_multibyte_keys_do_not_fail_the_whole_collection() -> TestResult {
        let tree = Tree::new("multibyte")?;
        let owner = "\u{1F600}".repeat(60);
        let hint = "\u{1F680}".repeat(100);
        tree.write(
            &format!("var/spool/cron/crontabs/{owner}"),
            format!("0 1 * * * /bin/{hint}\n0 2 * * * /bin/ok\n").as_bytes(),
        )?;
        let comm = "\u{1F600}".repeat(128);
        tree.write("proc/5/comm", format!("{comm}\n").as_bytes())?;
        tree.write("proc/5/cmdline", b"x\0")?;
        let inv = inventory_of(&tree.fs())?;
        assert!(inv.items_of(Kind::Process).count() == 1);
        assert!(inv.items_of(Kind::Cron).count() == 2);
        assert!(inv.items().iter().all(|i| i.key.len() <= 512));
        assert!(
            inv.warnings.iter().any(|w| w.contains("shortened")),
            "{:?}",
            inv.warnings
        );
        Ok(())
    }

    #[test]
    fn truncated_duplicate_keys_merge_and_count() -> TestResult {
        let tree = Tree::new("multibyte-dup")?;
        let owner = "\u{1F600}".repeat(60);
        // Two hints that differ only after the cut point collapse into one key.
        let a = format!("/bin/{}a", "\u{1F680}".repeat(99));
        let b = format!("/bin/{}b", "\u{1F680}".repeat(99));
        tree.write(
            &format!("var/spool/cron/crontabs/{owner}"),
            format!("0 1 * * * {a}\n0 1 * * * {b}\n").as_bytes(),
        )?;
        let inv = inventory_of(&tree.fs())?;
        let jobs: Vec<&Item> = inv.items_of(Kind::Cron).collect();
        assert_eq!(jobs.len(), 1);
        assert_eq!(
            jobs.first().and_then(|j| j.u64_field("occurrences")),
            Some(2)
        );
        Ok(())
    }

    #[cfg(unix)]
    mod symlinks {
        use super::*;
        use std::os::unix::fs::symlink;

        #[test]
        fn absolute_symlink_never_reaches_the_host() -> TestResult {
            let tree = Tree::new("sym-abs")?;
            tree.write("outside/secret", b"HOST SECRET")?;
            tree.write("root/etc/real", b"inside")?;
            fs::create_dir_all(tree.root.join("root/etc"))?;
            let outside = tree.root.join("outside/secret");
            symlink(&outside, tree.root.join("root/etc/crontab"))?;
            symlink("/etc/real", tree.root.join("root/etc/abs-in"))?;
            symlink("real", tree.root.join("root/etc/rel-in"))?;
            symlink(
                "../../../../outside/secret",
                tree.root.join("root/etc/dots"),
            )?;
            symlink(tree.root.join("outside"), tree.root.join("root/etc/dir"))?;
            let root = RootFs::new(tree.root.join("root"));
            // Absolute target is re-rooted: the host file is not reachable.
            assert!(root.read_to_string("/etc/crontab", 100).is_err());
            // Absolute and relative links inside the root still work.
            assert_eq!(root.read_to_string("/etc/abs-in", 100)?.text, "inside");
            assert_eq!(root.read_to_string("/etc/rel-in", 100)?.text, "inside");
            // `..` can never climb out of the root.
            assert!(root.read_to_string("/etc/dots", 100).is_err());
            assert!(root.read_to_string("/etc/dir/secret", 100).is_err());
            assert!(root.read_dir("/etc/dir", 10).is_err());
            Ok(())
        }

        #[test]
        fn clamped_dotdot_reads_inside_the_root() -> TestResult {
            let tree = Tree::new("sym-clamp")?;
            tree.write("root/target", b"inside-root")?;
            tree.write("outside/target", b"HOST")?;
            fs::create_dir_all(tree.root.join("root/etc"))?;
            symlink("../../../target", tree.root.join("root/etc/up"))?;
            let root = RootFs::new(tree.root.join("root"));
            assert_eq!(root.read_to_string("/etc/up", 100)?.text, "inside-root");
            Ok(())
        }

        #[test]
        fn read_link_does_not_follow_the_last_component() -> TestResult {
            let tree = Tree::new("sym-readlink")?;
            fs::create_dir_all(tree.root.join("real/dir"))?;
            symlink("/srv/app", tree.root.join("real/dir/cwd"))?;
            symlink("real", tree.root.join("alias"))?;
            let root = tree.fs();
            assert_eq!(root.read_link("/alias/dir/cwd")?, "/srv/app");
            Ok(())
        }

        #[test]
        fn symlink_loop_is_an_error_not_a_hang() -> TestResult {
            let tree = Tree::new("sym-loop")?;
            fs::create_dir_all(tree.root.join("etc"))?;
            symlink("b", tree.root.join("etc/a"))?;
            symlink("a", tree.root.join("etc/b"))?;
            let root = tree.fs();
            assert!(root.read_to_string("/etc/a", 10).is_err());
            assert!(root.read_dir("/etc/a", 10).is_err());
            Ok(())
        }

        #[test]
        fn fifo_is_refused_without_blocking() -> TestResult {
            let tree = Tree::new("fifo")?;
            fs::create_dir_all(tree.root.join("etc"))?;
            let status = std::process::Command::new("mkfifo")
                .arg(tree.root.join("etc/pipe"))
                .status()?;
            assert!(status.success());
            assert!(matches!(
                tree.fs().read_to_string("/etc/pipe", 10),
                Err(FsError::WrongType)
            ));
            Ok(())
        }
    }

    // macOS refuses to create non-UTF-8 file names, so this runs on Linux only.
    #[cfg(target_os = "linux")]
    #[test]
    fn non_utf8_cron_file_names_warn() -> TestResult {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let tree = Tree::new("non-utf8")?;
        fs::create_dir_all(tree.root.join("etc/cron.d"))?;
        fs::write(
            tree.root
                .join("etc/cron.d")
                .join(OsStr::from_bytes(b"bad\xff")),
            b"* * * * * root /bin/x\n",
        )?;
        let cron = Cron::new(tree.fs()).run()?;
        assert!(
            cron.warnings.iter().any(|w| w.contains("non-UTF-8")),
            "{:?}",
            cron.warnings
        );
        Ok(())
    }

    fn hint_keys(lines: &[&str]) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let tree = Tree::new(&format!("hints-{}", lines.len()))?;
        let mut text = String::new();
        for line in lines {
            text.push_str(&format!("0 1 * * * {line}\n"));
        }
        tree.write("var/spool/cron/crontabs/web", text.as_bytes())?;
        let cron = Cron::new(tree.fs()).run()?;
        Ok(cron
            .items
            .into_iter()
            .map(|i| i.key.rsplit(':').next().unwrap_or("").to_owned())
            .collect())
    }

    #[test]
    fn hints_distinguish_scripts_behind_wrappers_and_interpreters() -> TestResult {
        let hints = hint_keys(&[
            "cd /var/www/site1 && php artisan schedule:run",
            "php /srv/app/artisan queue:work",
            "bash /opt/backup.sh",
            "curl -s https://h/ping",
            "FOO=1 /usr/bin/env nice -n 5 flock /tmp/l timeout 30 python3 /opt/sync.py --x",
            "sudo -u www php -d memory_limit=1 /srv/x/report.php",
            "bash -c 'cd /x && php /srv/y/job.php'",
            "cd /x; /opt/tool/run.sh",
        ])?;
        for expected in [
            "artisan",
            "backup.sh",
            "curl",
            "sync.py",
            "report.php",
            "job.php",
            "run.sh",
        ] {
            assert!(
                hints.iter().any(|h| h == expected),
                "{expected} in {hints:?}"
            );
        }
        for banned in [
            "cd", "php", "bash", "env", "sudo", "nice", "flock", "timeout",
        ] {
            assert!(hints.iter().all(|h| h != banned), "{banned} in {hints:?}");
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
#[test]
fn real_host_collection_succeeds() -> TestResult {
    let inv = inventory_of(&RootFs::host())?;
    assert!(inv.to_json().is_ok());
    Ok(())
}

mod supervision {
    use super::*;

    fn proc_entry(
        tree: &Tree,
        pid: u32,
        comm: &str,
        args: &[&str],
        stat_cgroup: Option<(u32, &str)>,
    ) -> TestResult {
        tree.write(&format!("proc/{pid}/comm"), format!("{comm}\n").as_bytes())?;
        let mut bytes = Vec::new();
        for arg in args {
            bytes.extend_from_slice(arg.as_bytes());
            bytes.push(0);
        }
        tree.write(&format!("proc/{pid}/cmdline"), &bytes)?;
        if let Some((ppid, cgroup)) = stat_cgroup {
            tree.write(
                &format!("proc/{pid}/stat"),
                format!("{pid} ({comm}) S {ppid} {pid} {pid} 0\n").as_bytes(),
            )?;
            tree.write(
                &format!("proc/{pid}/cgroup"),
                format!("0::{cgroup}\n").as_bytes(),
            )?;
        }
        Ok(())
    }

    const SERVICE: &str = "/system.slice/web.service";
    const SESSION: &str = "/user.slice/user-1000.slice/session-3.scope";

    fn state(tree: &Tree, comm: &str) -> Result<(Option<bool>, String), String> {
        let items = Processes::new(tree.fs())
            .run()
            .map_err(|e| e.to_string())?
            .items;
        let found = items
            .iter()
            .find(|i| i.key == format!("proc:{comm}"))
            .ok_or_else(|| format!("missing {comm}"))?;
        let supervised = match found.data.get("supervised") {
            Some(Value::Bool(b)) => Some(*b),
            Some(Value::Null) => None,
            other => return Err(format!("bad supervised {other:?}")),
        };
        Ok((supervised, found.str_field("mode").unwrap_or("").to_owned()))
    }

    #[test]
    fn systemd_service_is_supervised() -> TestResult {
        let tree = Tree::new("sup-svc")?;
        proc_entry(
            &tree,
            10,
            "node",
            &["node", "server.js"],
            Some((1, SERVICE)),
        )?;
        assert_eq!(state(&tree, "node")?, (Some(true), "unknown".to_owned()));
        Ok(())
    }

    #[test]
    fn session_scope_without_supervisor_is_unsupervised() -> TestResult {
        let tree = Tree::new("sup-sess")?;
        proc_entry(
            &tree,
            10,
            "node",
            &["node", "server.js"],
            Some((5, SESSION)),
        )?;
        proc_entry(&tree, 5, "bash", &["bash"], Some((1, SESSION)))?;
        assert_eq!(state(&tree, "node")?.0, Some(false));
        Ok(())
    }

    #[test]
    fn pm2_ancestor_counts_as_supervisor() -> TestResult {
        let tree = Tree::new("sup-pm2")?;
        proc_entry(&tree, 10, "node", &["node", "app.js"], Some((5, SESSION)))?;
        proc_entry(
            &tree,
            5,
            "PM2 v5.4.2: God",
            &["PM2 v5.4.2: God Daemon"],
            Some((1, SESSION)),
        )?;
        assert_eq!(state(&tree, "node")?.0, Some(true));
        Ok(())
    }

    #[test]
    fn dev_servers_are_flagged_and_builds_are_not() -> TestResult {
        let tree = Tree::new("sup-dev")?;
        proc_entry(
            &tree,
            1,
            "vite",
            &["node", "/app/node_modules/.bin/vite"],
            None,
        )?;
        proc_entry(
            &tree,
            2,
            "next",
            &["node", "/app/node_modules/.bin/next", "dev"],
            None,
        )?;
        proc_entry(
            &tree,
            3,
            "vitebuild",
            &["node", "/app/node_modules/.bin/vite", "build"],
            None,
        )?;
        assert_eq!(state(&tree, "vite")?.1, "dev");
        assert_eq!(state(&tree, "next")?.1, "dev");
        assert_eq!(state(&tree, "vitebuild")?.1, "unknown");
        Ok(())
    }

    #[test]
    fn missing_cgroup_and_stat_gives_null() -> TestResult {
        let tree = Tree::new("sup-none")?;
        proc_entry(&tree, 10, "node", &["node", "server.js"], None)?;
        assert_eq!(state(&tree, "node")?, (None, "unknown".to_owned()));
        Ok(())
    }

    #[test]
    fn one_unsupervised_instance_makes_the_group_unsupervised() -> TestResult {
        let tree = Tree::new("sup-mixed")?;
        proc_entry(&tree, 10, "node", &["node", "a.js"], Some((1, SERVICE)))?;
        proc_entry(&tree, 11, "node", &["node", "b.js"], Some((5, SESSION)))?;
        let (supervised, _) = state(&tree, "node")?;
        assert_eq!(supervised, Some(false));
        Ok(())
    }
}

mod all_collectors {
    use super::*;
    use collectors::{Cmd, FakeRunner, collect_inventory_with};
    use std::sync::Arc;

    #[test]
    fn docker_systemd_and_pm2_items_join_the_inventory() -> TestResult {
        let tree = Tree::new("allc")?;
        tree.write("proc/1/comm", b"init\n")?;
        tree.write("proc/1/cmdline", b"/sbin/init\0")?;
        tree.write(
            "root/.pm2/dump.pm2",
            br#"[{"name":"site","pm_exec_path":"/srv/site/app.js","env":{"NODE_ENV":"production","DB_PASSWORD":"hunter2"}}]"#,
        )?;
        let runner = FakeRunner::new()
            .with(
                Cmd::SystemctlServices,
                Ok("nginx.service loaded active running A web server\n".into()),
            )
            .with(
                Cmd::DockerPs,
                Ok(r#"{"ID":"0123456789abcdef","Names":"cache","Image":"redis:7","State":"running"}"#.into()),
            );
        let inv = collect_inventory_with(
            &tree.fs(),
            Arc::new(runner),
            "2026-01-01T00:00:00Z".into(),
            platform(),
        )?;
        for key in ["systemd:nginx", "docker:cache", "pm2:site"] {
            assert_eq!(
                item(&inv, Kind::Process, key)?.data["supervised"],
                Value::from(true)
            );
        }
        assert_eq!(
            item(&inv, Kind::Process, "pm2:site")?.data["mode"],
            Value::from("prod")
        );
        assert!(!inv.to_json()?.contains("hunter2"));
        Ok(())
    }

    #[cfg(target_endian = "little")]
    #[test]
    fn env_references_resolve_only_to_local_listeners() -> TestResult {
        let tree = Tree::new("envref")?;
        // 127.0.0.1:5432 and 0.0.0.0:8080 listen.
        tree.write(
            "proc/net/tcp",
            b"  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n   0: 0100007F:1538 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 20001 1 0000000000000000 100 0 0 10 0\n   1: 00000000:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 20002 1 0000000000000000 100 0 0 10 0\n",
        )?;
        tree.write(
            "root/.pm2/dump.pm2",
            br#"[{"name":"web","pm_exec_path":"/srv/web/app.js","env":{
              "REDIS_URL":"redis://:topsecret@127.0.0.1:6379/0",
              "DB_HOST":"127.0.0.1","DB_PORT":"5432",
              "WEB":"localhost:8080","OTHER":"10.0.0.5:5432","GARBAGE_URL":"%%%% pw=hunter2"}}]"#,
        )?;
        let runner = FakeRunner::new()
            .with(
                Cmd::DockerPs,
                Ok(r#"{"ID":"0123456789abcdef","Names":"cache","Image":"redis:7.2","State":"running"}"#.into()),
            )
            .with(
                Cmd::DockerInspect("0123456789ab".into()),
                Ok(r#"[{"Config":{"Env":["DATABASE_URL=postgres://admin:dbpass@localhost/shop"]}}]"#.into()),
            );
        let inv = collect_inventory_with(
            &tree.fs(),
            Arc::new(runner),
            "2026-01-01T00:00:00Z".into(),
            platform(),
        )?;
        let resolves = |key: &str| -> Result<Option<String>, String> {
            Ok(item(&inv, Kind::EnvRef, key)?
                .str_field("resolves_to")
                .map(str::to_owned))
        };
        assert_eq!(resolves("pm2:web:REDIS_URL")?, None);
        assert_eq!(
            resolves("pm2:web:DB_HOST")?.as_deref(),
            Some("listener:tcp:5432")
        );
        assert_eq!(
            resolves("pm2:web:WEB")?.as_deref(),
            Some("listener:tcp:8080")
        );
        assert_eq!(resolves("pm2:web:OTHER")?, None);
        let db = item(&inv, Kind::EnvRef, "docker:cache:DATABASE_URL")?;
        assert_eq!(db.data.get("in_container"), Some(&Value::Bool(true)));
        assert_eq!(db.str_field("resolves_to"), Some("listener:tcp:5432"));
        assert_eq!(
            item(&inv, Kind::Process, "docker:cache")?.str_field("version"),
            Some("7.2")
        );
        let json = inv.to_json()?;
        for secret in ["topsecret", "hunter2", "admin", "dbpass", "shop"] {
            assert!(!json.contains(secret), "{secret}");
        }
        Ok(())
    }
}

#[cfg(unix)]
#[test]
fn nolink_read_refuses_a_final_symlink_but_reads_plain_files() -> TestResult {
    let tree = Tree::new("nolink")?;
    tree.write("real.txt", b"hello")?;
    std::os::unix::fs::symlink("/real.txt", tree.root.join("link.txt"))?;
    let fs = tree.fs();
    assert_eq!(fs.read_to_string_nolink("/real.txt", 100)?.text, "hello");
    assert!(matches!(
        fs.read_to_string_nolink("/link.txt", 100),
        Err(FsError::WrongType)
    ));
    // The ordinary read still follows the link (inside the root).
    assert_eq!(fs.read_to_string("/link.txt", 100)?.text, "hello");
    Ok(())
}
