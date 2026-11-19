//! Running processes, from `/proc/<pid>/{comm,cmdline,cwd}` (kind `process`, seen via `proc`).
//!
//! Item key `proc:<comm>` (no supervisor is recognised yet). Processes with the same `comm` are
//! merged into one item; `cmd` and `cwd` come from the lowest pid. Data (docs/design.md 4.1):
//! `cmd` (command line, secrets redacted, at most 512 characters), `cwd` (path or null), `mode`
//! (`"unknown"`), `supervised` (null), `env_refs` (empty), plus `instances` (how many processes
//! were merged).
//!
//! Kernel threads and zombies (empty `cmdline`) are skipped. `/proc/<pid>/environ` is never read.
//! A pid that disappears mid-read is skipped silently (normal); unreadable `cwd` links are counted
//! in a single warning. Values that cannot be determined are null/`unknown`, never guessed.
//!
//! Redaction: the arguments go through [`crate::util::redact_command_args`] **before** truncation,
//! so a cut can never leave half a secret. It also hides separate-argument secrets
//! (`--password hunter2`, `-p secret`, `-a secret`, `-u user:pw`); see its documentation.

use std::collections::BTreeMap;

use inventory::{CollectError, Collection, Collector, Item, Kind, SeenVia};
use serde_json::{Map, Value};

use crate::procfs::{list_pids, read_comm};
use crate::rootfs::{FsError, RootFs};
use crate::util::{MAX_CMD_CHARS, redact_command_args, truncate_chars};

const MAX_CMDLINE_BYTES: usize = 64 * 1024;

/// Redacted, bounded command line from the NUL-separated `cmdline` content (shared argument
/// redaction: [`redact_command_args`]; truncation happens after it).
pub(crate) fn redacted_cmd(cmdline: &str) -> String {
    let args = cmdline.split('\0').filter(|a| !a.is_empty());
    truncate_chars(&redact_command_args(args), MAX_CMD_CHARS)
}

struct Group {
    cmd: String,
    cwd: Option<String>,
    instances: u64,
}

/// Collector of running processes.
#[derive(Debug, Clone)]
pub struct Processes {
    root: RootFs,
}

impl Processes {
    /// A collector reading from `root`.
    #[must_use]
    pub fn new(root: RootFs) -> Self {
        Self { root }
    }
}

impl Collector for Processes {
    fn kind(&self) -> &'static str {
        "processes"
    }

    fn run(&self) -> Result<Collection, CollectError> {
        self.root
            .validate()
            .map_err(|err| CollectError(format!("root filesystem: {err}")))?;
        let mut warnings = Vec::new();
        let mut groups: BTreeMap<String, Group> = BTreeMap::new();
        let mut denied = 0usize;
        let mut failed = 0usize;
        let mut cwd_denied = 0usize;

        for pid in list_pids(&self.root, &mut warnings) {
            let comm = match read_comm(&self.root, pid) {
                Ok(comm) => comm,
                Err(FsError::NotFound) => continue,
                Err(FsError::PermissionDenied) => {
                    denied += 1;
                    continue;
                }
                Err(_) => {
                    failed += 1;
                    continue;
                }
            };
            let cmdline = match self
                .root
                .read_to_string(&format!("/proc/{pid}/cmdline"), MAX_CMDLINE_BYTES)
            {
                Ok(file) => file.text,
                Err(FsError::NotFound) => continue,
                Err(FsError::PermissionDenied) => {
                    denied += 1;
                    continue;
                }
                Err(_) => {
                    failed += 1;
                    continue;
                }
            };
            if comm.is_empty() || cmdline.trim_matches('\0').is_empty() {
                continue; // kernel thread, zombie, or a process that vanished
            }
            let group = groups.entry(comm).or_insert_with(|| {
                let cwd = match self.root.read_link(&format!("/proc/{pid}/cwd")) {
                    Ok(target) => Some(truncate_chars(&target, MAX_CMD_CHARS)),
                    Err(FsError::PermissionDenied) => {
                        cwd_denied += 1;
                        None
                    }
                    Err(_) => None,
                };
                Group {
                    cmd: redacted_cmd(&cmdline),
                    cwd,
                    instances: 0,
                }
            });
            group.instances += 1;
        }

        if denied > 0 {
            warnings.push(format!(
                "could not read {denied} process(es) (permission denied); they are missing"
            ));
        }
        if failed > 0 {
            warnings.push(format!(
                "could not read {failed} process(es) (I/O error); they are missing"
            ));
        }
        if cwd_denied > 0 {
            warnings.push(format!(
                "working directory unreadable for {cwd_denied} process(es) (permission denied)"
            ));
        }

        let items = groups
            .into_iter()
            .map(|(comm, group)| {
                let mut data = Map::new();
                data.insert("cmd".into(), Value::from(group.cmd));
                data.insert("cwd".into(), group.cwd.map_or(Value::Null, Value::from));
                data.insert("mode".into(), Value::from("unknown"));
                data.insert("supervised".into(), Value::Null);
                data.insert("env_refs".into(), Value::Array(Vec::new()));
                data.insert("instances".into(), Value::from(group.instances));
                Item {
                    kind: Kind::Process,
                    key: format!("proc:{comm}"),
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

    #[test]
    fn redacts_equals_url_and_separate_argument_secrets() {
        let cmd = redacted_cmd(
            "/opt/app\0--password=hunter2\0--db\0postgres://u:pw@h/db\0--token\0abc123\0--port\08080\0",
        );
        for secret in ["hunter2", "pw@", "abc123"] {
            assert!(!cmd.contains(secret), "{secret} leaked in {cmd}");
        }
        assert!(cmd.contains("--port 8080"));
        assert!(cmd.contains("/opt/app"));
    }

    #[test]
    fn truncates_after_redaction() {
        let long = format!("tool\0--note\0{}\0", "x".repeat(2_000));
        assert_eq!(redacted_cmd(&long).chars().count(), MAX_CMD_CHARS);
    }

    #[test]
    fn flag_at_end_and_empty_input_do_not_panic() {
        assert_eq!(redacted_cmd("app\0--password\0"), "app --password");
        assert_eq!(redacted_cmd(""), "");
        assert_eq!(redacted_cmd("\0\0\0"), "");
    }
}
