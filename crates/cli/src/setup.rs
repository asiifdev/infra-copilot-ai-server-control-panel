//! `infra init` and `infra node add`.

use crate::Ui;
use crate::args::{Globals, Parsed};
use db::{NewNode, NodeAccess, NodeKind, Role};
use i18n::MessageKey;

const INIT_EXISTS: MessageKey = MessageKey::new("cli.init.exists");
const INIT_DONE: MessageKey = MessageKey::new("cli.init.done");
const NODE_ADDED: MessageKey = MessageKey::new("cli.node.added");
const NODE_EXISTS: MessageKey = MessageKey::new("cli.node.exists");

impl Ui<'_> {
    pub(crate) fn init(&mut self, globals: &Globals, parsed: &Parsed) -> u8 {
        if let Some(extra) = parsed.positional.first() {
            return self.usage(extra, Some(crate::USAGE_INIT));
        }
        let slug = parsed.option("--slug").unwrap_or("local");
        let name = parsed.option("--name").unwrap_or("Local");
        let email = parsed
            .option("--email")
            .unwrap_or("owner@localhost")
            .to_ascii_lowercase();
        let (conn, path) = match self.open_db(globals, true) {
            Ok(opened) => opened,
            Err(code) => return code,
        };
        match db::list_orgs(&conn, 1) {
            Ok(orgs) if !orgs.is_empty() => {
                let existing = orgs.first().map_or("", |o| o.slug.as_str());
                return self.fail(INIT_EXISTS, &[("slug", existing)]);
            }
            Ok(_) => {}
            Err(error) => return self.store_error(&error),
        }
        // Org, user and membership are created together or not at all.
        let created = (|| -> db::Result<()> {
            conn.execute_batch("BEGIN IMMEDIATE")?;
            let result = (|| {
                let org = db::create_org(&conn, name, slug, "free")?;
                let user = db::create_user(&conn, &email, "Owner", None)?;
                db::add_member(&conn, &org.id, &user.id, Role::Owner)?;
                Ok(())
            })();
            match &result {
                Ok(()) => conn.execute_batch("COMMIT")?,
                Err(_) => {
                    let _ = conn.execute_batch("ROLLBACK");
                }
            }
            result
        })();
        if let Err(error) = created {
            return self.store_error(&error);
        }
        let shown = path.to_string_lossy().into_owned();
        self.say_out(
            INIT_DONE,
            &[("slug", slug), ("email", &email), ("path", &shown)],
        )
    }

    pub(crate) fn node_command(&mut self, globals: &Globals, rest: &[String]) -> u8 {
        let Some((sub, args)) = rest.split_first() else {
            return self.usage("-", Some(crate::USAGE_NODE));
        };
        if sub != "add" {
            return self.usage(sub, Some(crate::USAGE_NODE));
        }
        let parsed = match crate::args::parse(args, &["--name", "--kind", "--lang"], &[]) {
            Ok(parsed) => parsed,
            Err(detail) => return self.usage(&detail, Some(crate::USAGE_NODE)),
        };
        if !self.apply_lang(&parsed) {
            return self.usage("--lang", Some(crate::USAGE_NODE));
        }
        let (Some(name), Some(kind)) = (parsed.option("--name"), parsed.option("--kind")) else {
            return self.usage("-", Some(crate::USAGE_NODE));
        };
        let Ok(kind) = NodeKind::parse(kind) else {
            return self.usage("--kind", Some(crate::USAGE_NODE));
        };
        if let Some(extra) = parsed.positional.first() {
            return self.usage(extra, Some(crate::USAGE_NODE));
        }
        let (conn, _) = match self.open_db(globals, false) {
            Ok(opened) => opened,
            Err(code) => return code,
        };
        let scope = match self.scope(&conn, globals) {
            Ok(scope) => scope,
            Err(code) => return code,
        };
        let access = match kind {
            NodeKind::Vps => NodeAccess::Agent,
            NodeKind::Local => NodeAccess::Local,
        };
        let created = db::create_node(
            &conn,
            &scope.org,
            &NewNode {
                name,
                kind,
                access,
                labels_json: "{}",
            },
        );
        match created {
            Ok(_) => self.say_out(NODE_ADDED, &[("name", name), ("kind", kind.as_str())]),
            Err(error) if error.is_constraint_violation() => {
                self.fail(NODE_EXISTS, &[("name", name)])
            }
            Err(error) => self.store_error(&error),
        }
    }
}
