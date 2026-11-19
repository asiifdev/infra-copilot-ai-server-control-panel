//! Heuristics for "is this process a development server" and "is something supervising it".
//!
//! Both are read-only and conservative: a result is only positive or negative when `/proc` shows
//! clear evidence, otherwise it is unknown. Nothing here runs a command.
//!
//! * Dev mode: the command line is a well-known development server (`vite`, `next dev`,
//!   `nodemon`, `php artisan serve`, `manage.py runserver`, `--reload`, ...). Never a guess from
//!   the environment (`environ` is not read), so a production-looking process stays `unknown`.
//! * Supervision: the cgroup says systemd service or container runtime (supervised), or a parent
//!   process is a known supervisor such as PM2 (supervised), or the process sits in a login
//!   session scope with no supervisor above it (not supervised).

use std::collections::BTreeMap;

use crate::rootfs::RootFs;

/// Parent process name and id, for walking up to a supervisor.
pub(crate) type Parents = BTreeMap<u32, (u32, String)>;

/// Where `/proc/<pid>/cgroup` places a process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cgroup {
    /// A systemd service.
    Service,
    /// A container runtime scope.
    Container,
    /// A login session scope: started by hand from a shell.
    Session,
    /// Anything else, or unreadable.
    Other,
}

const MAX_SMALL_FILE: usize = 16 * 1024;
const MAX_HOPS: usize = 32;

/// Reads and classifies `/proc/<pid>/cgroup`; unreadable is [`Cgroup::Other`].
pub(crate) fn read_cgroup(root: &RootFs, pid: u32) -> Cgroup {
    root.read_to_string(&format!("/proc/{pid}/cgroup"), MAX_SMALL_FILE)
        .map_or(Cgroup::Other, |file| parse_cgroup(&file.text))
}

/// Parent id from `/proc/<pid>/stat` (the name may contain spaces and parentheses, so the fields
/// are read after the last `)`).
pub(crate) fn read_ppid(root: &RootFs, pid: u32) -> Option<u32> {
    let file = root
        .read_to_string(&format!("/proc/{pid}/stat"), MAX_SMALL_FILE)
        .ok()?;
    parse_ppid(&file.text)
}

pub(crate) fn parse_ppid(stat: &str) -> Option<u32> {
    let after = stat.get(stat.rfind(')')? + 1..)?;
    after.split_whitespace().nth(1)?.parse().ok()
}

/// Classifies the unified (`0::`) or systemd-named cgroup path by its last component.
pub(crate) fn parse_cgroup(text: &str) -> Cgroup {
    let path = text
        .lines()
        .find_map(|line| {
            let mut parts = line.splitn(3, ':');
            let (_, name, path) = (parts.next()?, parts.next()?, parts.next()?);
            (name.is_empty() || name == "name=systemd").then_some(path)
        })
        .unwrap_or("");
    let last = path.rsplit('/').next().unwrap_or("");
    if last.ends_with(".service") {
        Cgroup::Service
    } else if path
        .split('/')
        .any(|part| part == "docker" || part == "kubepods")
        || ["docker-", "libpod-", "crio-", "cri-containerd-"]
            .iter()
            .any(|prefix| last.starts_with(prefix) && last.ends_with(".scope"))
    {
        Cgroup::Container
    } else if last.starts_with("session-") && last.ends_with(".scope") {
        Cgroup::Session
    } else {
        Cgroup::Other
    }
}

/// Whether a process name is a supervisor that restarts what it starts.
fn is_supervisor(comm: &str) -> bool {
    let lower = comm.to_ascii_lowercase();
    lower.starts_with("pm2 ")
        || [
            "pm2",
            "supervisord",
            "supervise",
            "s6-supervise",
            "runsv",
            "forever",
            "monit",
        ]
        .contains(&lower.as_str())
}

/// Whether `pid` is supervised: `Some(true)` or `Some(false)` on clear evidence, else `None`.
pub(crate) fn supervised(pid: u32, cgroup: Cgroup, parents: &Parents) -> Option<bool> {
    let mut current = pid;
    for _ in 0..MAX_HOPS {
        let Some((ppid, _)) = parents.get(&current) else {
            break;
        };
        if let Some((_, comm)) = parents.get(ppid)
            && is_supervisor(comm)
        {
            return Some(true);
        }
        if *ppid == 0 || *ppid == current {
            break;
        }
        current = *ppid;
    }
    match cgroup {
        Cgroup::Service | Cgroup::Container => Some(true),
        Cgroup::Session => Some(false),
        Cgroup::Other => None,
    }
}

fn base(arg: &str) -> &str {
    arg.rsplit('/').next().unwrap_or(arg)
}

/// Whether the arguments (program first) look like a development server.
pub(crate) fn is_dev_command(args: &[&str]) -> bool {
    let names: Vec<String> = args.iter().map(|a| base(a).to_ascii_lowercase()).collect();
    let after = |tool: &str| -> Option<&str> {
        let at = names.iter().position(|n| n == tool)?;
        args.get(at + 1).copied()
    };
    if args.contains(&"--reload") {
        return true;
    }
    if names
        .iter()
        .any(|n| ["nodemon", "ts-node-dev", "webpack-dev-server", "next-dev"].contains(&n.as_str()))
    {
        return true;
    }
    if !matches!(after("vite"), Some("build" | "preview" | "optimize"))
        && names.iter().any(|n| n == "vite")
    {
        return true;
    }
    let sub = |tool: &str, subs: &[&str]| after(tool).is_some_and(|s| subs.contains(&s));
    sub("next", &["dev"])
        || sub("nuxt", &["dev"])
        || sub("astro", &["dev"])
        || sub("ng", &["serve"])
        || sub("webpack", &["serve", "watch"])
        || sub("flask", &["run"])
        || sub("cargo", &["watch"])
        || sub("artisan", &["serve"])
        || sub("manage.py", &["runserver"])
        || (names
            .first()
            .is_some_and(|n| ["npm", "yarn", "pnpm", "bun"].contains(&n.as_str()))
            && args
                .iter()
                .skip(1)
                .any(|a| ["dev", "start:dev", "serve"].contains(a)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(line: &str) -> bool {
        is_dev_command(&line.split(' ').collect::<Vec<_>>())
    }

    #[test]
    fn recognises_common_dev_servers() {
        for line in [
            "node /app/node_modules/.bin/vite",
            "node /app/node_modules/.bin/vite dev --port 3000",
            "node /app/node_modules/.bin/vite --port 3000",
            "node /app/node_modules/.bin/next dev",
            "nodemon server.js",
            "npm run dev",
            "php artisan serve --port=8000",
            "python manage.py runserver 0.0.0.0:8000",
            "uvicorn main:app --reload",
            "flask run",
        ] {
            assert!(dev(line), "{line}");
        }
    }

    #[test]
    fn does_not_flag_production_commands() {
        for line in [
            "node /app/node_modules/.bin/vite build",
            "node /app/node_modules/.bin/vite preview",
            "node /app/node_modules/.bin/next start",
            "node server.js",
            "nginx -g daemon off;",
            "php-fpm: master process",
            "gunicorn main:app",
            "",
        ] {
            assert!(!dev(line), "{line}");
        }
    }

    #[test]
    fn classifies_cgroups() {
        assert_eq!(
            parse_cgroup("0::/system.slice/web.service\n"),
            Cgroup::Service
        );
        assert_eq!(
            parse_cgroup("12:cpu:/x\n1:name=systemd:/system.slice/docker-abc.scope\n"),
            Cgroup::Container
        );
        assert_eq!(
            parse_cgroup("0::/user.slice/user-1000.slice/session-3.scope\n"),
            Cgroup::Session
        );
        assert_eq!(parse_cgroup("0::/\n"), Cgroup::Other);
        assert_eq!(parse_cgroup(""), Cgroup::Other);
    }

    #[test]
    fn reads_ppid_even_when_the_name_has_parentheses() {
        assert_eq!(parse_ppid("12 (a) b) S 7 12 12 0"), Some(7));
        assert_eq!(parse_ppid("garbage"), None);
    }

    #[test]
    fn supervisor_ancestor_wins_over_session_scope_and_loops_end() {
        let parents: Parents = BTreeMap::from([
            (10, (5, "node".into())),
            (5, (1, "PM2 v5.4.2: God".into())),
            (1, (0, "init".into())),
        ]);
        assert_eq!(supervised(10, Cgroup::Session, &parents), Some(true));
        assert_eq!(supervised(5, Cgroup::Session, &parents), Some(false));
        let looped: Parents = BTreeMap::from([(1, (2, "a".into())), (2, (1, "b".into()))]);
        assert_eq!(supervised(1, Cgroup::Other, &looped), None);
    }
}
