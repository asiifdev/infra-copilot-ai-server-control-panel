//! The local database of the command line: where it lives, opening it with owner-only
//! permissions, and finding "the" organization and its owner.
//!
//! Location, first match wins: `--db PATH`, the `NODIVARA_DB` variable, then a per-user data
//! directory: `~/Library/Application Support/Nodivara/nodivara.db` on macOS, otherwise
//! `$XDG_DATA_HOME/nodivara/nodivara.db` (or `~/.local/share/nodivara/nodivara.db`).

use crate::Ui;
use crate::args::Globals;
use db::{Connection, OrgId, Role, UserId};
use i18n::MessageKey;
use std::path::{Path, PathBuf};

const ERR_DB_OPEN: MessageKey = MessageKey::new("cli.error.db_open");
const ERR_DB_MISSING: MessageKey = MessageKey::new("cli.error.db_missing");
const ERR_DB_PATH: MessageKey = MessageKey::new("cli.error.db_path");
const ERR_NO_ORG: MessageKey = MessageKey::new("cli.error.no_org");
const ERR_MANY_ORGS: MessageKey = MessageKey::new("cli.error.many_orgs");
const ERR_ORG_UNKNOWN: MessageKey = MessageKey::new("cli.error.org_unknown");
const ERR_NO_OWNER: MessageKey = MessageKey::new("cli.error.no_owner");
const ERR_STORE: MessageKey = MessageKey::new("cli.error.store");

/// Most organizations listed when looking for the one to use.
const ORG_LIMIT: u32 = 50;

/// The organization and person a command acts for.
pub(crate) struct Scope {
    pub(crate) org: OrgId,
    pub(crate) user: UserId,
}

/// Creates `path` (when missing) readable by its owner only, and tightens an existing file that
/// others can read: it will hold sealed provider keys and conversations.
fn prepare_file(path: &Path, create_parent: bool) -> std::io::Result<()> {
    #[cfg(unix)]
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
    if create_parent && let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        builder.mode(0o700);
        builder.create(parent)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    match options.open(path) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            #[cfg(unix)]
            {
                let meta = std::fs::metadata(path)?;
                if meta.is_file() && meta.permissions().mode() & 0o077 != 0 {
                    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
                }
            }
            Ok(())
        }
        Err(error) => Err(error),
    }
}

impl Ui<'_> {
    /// The database path and whether it is the default per-user location.
    fn db_path(&mut self, globals: &Globals) -> Result<(PathBuf, bool), u8> {
        if let Some(path) = &globals.db {
            return Ok((PathBuf::from(path), false));
        }
        if let Some(path) = self.env.get("NODIVARA_DB").filter(|p| !p.is_empty()) {
            return Ok((PathBuf::from(path), false));
        }
        let home = self.env.get("HOME").filter(|h| !h.is_empty());
        let dir = if cfg!(target_os = "macos") {
            home.map(|h| PathBuf::from(h).join("Library/Application Support/Nodivara"))
        } else {
            self.env
                .get("XDG_DATA_HOME")
                .filter(|d| !d.is_empty())
                .map(|d| PathBuf::from(d).join("nodivara"))
                .or_else(|| home.map(|h| PathBuf::from(h).join(".local/share/nodivara")))
        };
        match dir {
            Some(dir) => Ok((dir.join("nodivara.db"), true)),
            None => Err(self.fail(ERR_DB_PATH, &[])),
        }
    }

    /// Opens the database, creating it (owner-only) when `create` is set; otherwise a missing
    /// file is an error that points at `infra init`.
    pub(crate) fn open_db(
        &mut self,
        globals: &Globals,
        create: bool,
    ) -> Result<(Connection, PathBuf), u8> {
        let (path, is_default) = self.db_path(globals)?;
        let shown = path.to_string_lossy().into_owned();
        if create {
            if let Err(error) = prepare_file(&path, is_default) {
                return Err(self.fail(
                    ERR_DB_OPEN,
                    &[("path", &shown), ("reason", &error.to_string())],
                ));
            }
        } else if !path.is_file() {
            return Err(self.fail(ERR_DB_MISSING, &[("path", &shown)]));
        }
        match db::open(&path) {
            Ok(conn) => Ok((conn, path)),
            Err(error) => Err(self.fail(
                ERR_DB_OPEN,
                &[("path", &shown), ("reason", &error.to_string())],
            )),
        }
    }

    /// The organization to use (`--org SLUG`, or the only one) and its owner.
    pub(crate) fn scope(&mut self, conn: &Connection, globals: &Globals) -> Result<Scope, u8> {
        let orgs = match db::list_orgs(conn, ORG_LIMIT) {
            Ok(orgs) => orgs,
            Err(error) => return Err(self.store_error(&error)),
        };
        let org = match (&globals.org, orgs.as_slice()) {
            (_, []) => return Err(self.fail(ERR_NO_ORG, &[])),
            (Some(slug), _) => match orgs.iter().find(|o| &o.slug == slug) {
                Some(org) => org,
                None => return Err(self.fail(ERR_ORG_UNKNOWN, &[("slug", slug)])),
            },
            (None, [only]) => only,
            (None, many) => {
                let slugs: Vec<&str> = many.iter().map(|o| o.slug.as_str()).collect();
                return Err(self.fail(ERR_MANY_ORGS, &[("slugs", &slugs.join(", "))]));
            }
        };
        let members = match db::list_members(conn, &org.id) {
            Ok(members) => members,
            Err(error) => return Err(self.store_error(&error)),
        };
        let owner = members
            .iter()
            .find(|m| m.role == Role::Owner)
            .or_else(|| members.first());
        match owner {
            Some(member) => Ok(Scope {
                org: org.id.clone(),
                user: member.user_id.clone(),
            }),
            None => Err(self.fail(ERR_NO_OWNER, &[])),
        }
    }

    /// A database error as text. The text of a database error never holds secret values.
    pub(crate) fn store_error(&mut self, error: &db::Error) -> u8 {
        self.store_reason(&error.to_string())
    }

    /// A storage failure with a fixed reason.
    pub(crate) fn store_reason(&mut self, reason: &str) -> u8 {
        self.fail(ERR_STORE, &[("reason", reason)])
    }
}
