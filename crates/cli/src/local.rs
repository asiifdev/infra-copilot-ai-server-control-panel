//! The local database of the command line. The logic (where it lives, owner-only permissions,
//! finding "the" organization and its owner, the vault from the environment) is in the
//! `bootstrap` crate, shared with the local server; this file only turns a `SetupError` into
//! localized advice and an exit code.

use crate::Ui;
use crate::args::Globals;
use bootstrap::SetupError;
use db::Connection;
use i18n::MessageKey;
use std::path::PathBuf;

pub(crate) use bootstrap::Scope;

const ERR_DB_OPEN: MessageKey = MessageKey::new("cli.error.db_open");
const ERR_DB_MISSING: MessageKey = MessageKey::new("cli.error.db_missing");
const ERR_DB_PATH: MessageKey = MessageKey::new("cli.error.db_path");
const ERR_NO_ORG: MessageKey = MessageKey::new("cli.error.no_org");
const ERR_MANY_ORGS: MessageKey = MessageKey::new("cli.error.many_orgs");
const ERR_ORG_UNKNOWN: MessageKey = MessageKey::new("cli.error.org_unknown");
const ERR_NO_OWNER: MessageKey = MessageKey::new("cli.error.no_owner");
const ERR_STORE: MessageKey = MessageKey::new("cli.error.store");
const VAULT_NO_KEY: MessageKey = MessageKey::new("cli.vault.no_key");
const VAULT_BAD_KEY: MessageKey = MessageKey::new("cli.vault.bad_key");

impl bootstrap::Environment for crate::Env {
    fn get(&self, name: &str) -> Option<String> {
        crate::Env::get(self, name)
    }
}

impl Ui<'_> {
    /// Reports a setup failure as localized advice; the exit code is the usage/configuration one.
    pub(crate) fn setup_failure(&mut self, error: &SetupError) -> u8 {
        match error {
            SetupError::NoDbPath => self.fail(ERR_DB_PATH, &[]),
            SetupError::DbMissing { path } => self.fail(ERR_DB_MISSING, &[("path", path)]),
            SetupError::DbOpen { path, reason } => {
                self.fail(ERR_DB_OPEN, &[("path", path), ("reason", reason)])
            }
            SetupError::NoOrg => self.fail(ERR_NO_ORG, &[]),
            SetupError::UnknownOrg { slug } => self.fail(ERR_ORG_UNKNOWN, &[("slug", slug)]),
            SetupError::ManyOrgs { slugs } => self.fail(ERR_MANY_ORGS, &[("slugs", slugs)]),
            SetupError::NoOwner => self.fail(ERR_NO_OWNER, &[]),
            SetupError::Store { reason } => self.store_reason(reason),
            SetupError::MasterKeyMissing => self.fail(VAULT_NO_KEY, &[]),
            SetupError::MasterKeyInvalid => self.fail(VAULT_BAD_KEY, &[]),
        }
    }

    /// Opens the database, creating it (owner-only) when `create` is set; otherwise a missing
    /// file is an error that points at `infra init`.
    pub(crate) fn open_db(
        &mut self,
        globals: &Globals,
        create: bool,
    ) -> Result<(Connection, PathBuf), u8> {
        bootstrap::open_database(self.env, globals.db.as_deref(), create)
            .map_err(|error| self.setup_failure(&error))
    }

    /// The organization to use (`--org SLUG`, or the only one) and its owner.
    pub(crate) fn scope(&mut self, conn: &Connection, globals: &Globals) -> Result<Scope, u8> {
        bootstrap::resolve_scope(conn, globals.org.as_deref())
            .map_err(|error| self.setup_failure(&error))
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
