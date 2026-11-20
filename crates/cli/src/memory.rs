//! `infra memory list|pending|approve ID|delete ID`.
//!
//! This is where a person decides about notes the assistant proposed: nothing a model can call
//! reaches `approve`. Memory text is untrusted (a model wrote it), so it is printed with control
//! characters replaced.

use crate::Ui;
use crate::args::{Globals, parse};
use db::{MemoryId, MemoryRow, MemoryStatus};
use i18n::MessageKey;

const EMPTY: MessageKey = MessageKey::new("cli.memory.empty");
const LINE: MessageKey = MessageKey::new("cli.memory.line");
const APPROVED: MessageKey = MessageKey::new("cli.memory.approved");
const DELETED: MessageKey = MessageKey::new("cli.memory.deleted");
const UNKNOWN: MessageKey = MessageKey::new("cli.memory.unknown");

impl Ui<'_> {
    pub(crate) fn memory_command(&mut self, globals: &Globals, rest: &[String]) -> u8 {
        let Some((sub, args)) = rest.split_first() else {
            return self.usage("-", Some(crate::USAGE_MEMORY));
        };
        let parsed = match parse(args, &["--lang"], &[]) {
            Ok(parsed) => parsed,
            Err(detail) => return self.usage(&detail, Some(crate::USAGE_MEMORY)),
        };
        if !self.apply_lang(&parsed) {
            return self.usage("--lang", Some(crate::USAGE_MEMORY));
        }
        let wants_id = matches!(sub.as_str(), "approve" | "delete");
        let known = wants_id || matches!(sub.as_str(), "list" | "pending");
        if !known || parsed.positional.len() != usize::from(wants_id) {
            let detail = parsed
                .positional
                .first()
                .map_or(sub.as_str(), String::as_str);
            return self.usage(detail, Some(crate::USAGE_MEMORY));
        }
        let id = if wants_id {
            match parsed.positional.first().map(|t| MemoryId::parse(t)) {
                Some(Ok(id)) => Some(id),
                _ => return self.usage("ID", Some(crate::USAGE_MEMORY)),
            }
        } else {
            None
        };
        let (conn, _) = match self.open_db(globals, false) {
            Ok(opened) => opened,
            Err(code) => return code,
        };
        let scope = match self.scope(&conn, globals) {
            Ok(scope) => scope,
            Err(code) => return code,
        };
        let org = &scope.org;
        match (sub.as_str(), id) {
            ("list", _) => {
                self.print_memories(db::list_memories(&conn, org, Some(MemoryStatus::Active)))
            }
            ("pending", _) => {
                self.print_memories(db::list_memories(&conn, org, Some(MemoryStatus::Pending)))
            }
            ("approve", Some(id)) => match db::approve_memory(&conn, org, &id) {
                Ok(()) => self.say_out(APPROVED, &[("id", id.as_str())]),
                Err(db::Error::NotFound) => self.fail(UNKNOWN, &[("id", id.as_str())]),
                Err(error) => self.store_error(&error),
            },
            (_, Some(id)) => match db::delete_memory(&conn, org, &id) {
                Ok(()) => self.say_out(DELETED, &[("id", id.as_str())]),
                Err(db::Error::NotFound) => self.fail(UNKNOWN, &[("id", id.as_str())]),
                Err(error) => self.store_error(&error),
            },
            _ => self.usage("-", Some(crate::USAGE_MEMORY)),
        }
    }

    fn print_memories(&mut self, rows: db::Result<Vec<MemoryRow>>) -> u8 {
        let rows = match rows {
            Ok(rows) => rows,
            Err(error) => return self.store_error(&error),
        };
        if rows.is_empty() {
            return self.say_out(EMPTY, &[]);
        }
        let lines: Vec<String> = rows
            .iter()
            .map(|row| {
                self.tr(
                    LINE,
                    &[
                        ("id", row.id.as_str()),
                        ("kind", row.kind.as_str()),
                        ("source", row.source.as_str()),
                        ("content", &row.content),
                    ],
                )
            })
            .collect();
        self.write_out(&lines.join("\n"))
    }
}
