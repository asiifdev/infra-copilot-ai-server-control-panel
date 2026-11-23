//! Setup shared by the command line and the local server (ADR 0030): the same database location,
//! the same owner-only file permissions, the same single-organization resolution and the same
//! vault-from-environment rules, so `infra serve` and every other command agree on all of them.
//!
//! Nothing here prints: failures are a [`SetupError`] and each caller turns it into text from the
//! i18n catalog in the reader's language.
//!
//! Database location, first match wins: an explicit path (`--db PATH`), the `NODIVARA_DB`
//! variable, then a per-user data directory: `~/Library/Application Support/Nodivara/nodivara.db`
//! on macOS, otherwise `$XDG_DATA_HOME/nodivara/nodivara.db` (or
//! `~/.local/share/nodivara/nodivara.db`).

use db::{Connection, OrgId, Role, UserId};
use std::path::{Path, PathBuf};
use vault::Vault;

/// Where setup reads environment variables from (the process environment, or a fixed map in
/// tests, which must not touch the shared process environment).
pub trait Environment {
    /// The value of `name`, if set.
    fn get(&self, name: &str) -> Option<String>;
}

/// The real process environment.
#[derive(Clone, Copy, Debug, Default)]
pub struct ProcessEnv;

impl Environment for ProcessEnv {
    fn get(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }
}

/// Why setup failed. Carries values the caller needs for its message (a path, slugs) and fixed
/// reasons; never a key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetupError {
    /// No explicit path, no `NODIVARA_DB` and no home directory to derive one from.
    NoDbPath,
    /// The database file does not exist (and it was not to be created).
    DbMissing {
        /// The path looked at.
        path: String,
    },
    /// The file could not be created, tightened or opened.
    DbOpen {
        /// The path.
        path: String,
        /// The operating-system or database reason (no secret values).
        reason: String,
    },
    /// The database holds no organization yet.
    NoOrg,
    /// `--org SLUG` names an organization that does not exist.
    UnknownOrg {
        /// The slug asked for.
        slug: String,
    },
    /// Several organizations exist and none was named.
    ManyOrgs {
        /// Their slugs, comma separated.
        slugs: String,
    },
    /// The organization has no member to act as.
    NoOwner,
    /// A storage failure while resolving the scope.
    Store {
        /// The database error text (it holds no secret values).
        reason: String,
    },
    /// `NODIVARA_MASTER_KEY` is not set.
    MasterKeyMissing,
    /// `NODIVARA_MASTER_KEY` (or a retired key) is not valid.
    MasterKeyInvalid,
}

impl std::fmt::Display for SetupError {
    /// Fixed text for logs and tests; users see the catalog text the caller picks instead.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NoDbPath => "no_db_path",
            Self::DbMissing { .. } => "db_missing",
            Self::DbOpen { .. } => "db_open",
            Self::NoOrg => "no_org",
            Self::UnknownOrg { .. } => "unknown_org",
            Self::ManyOrgs { .. } => "many_orgs",
            Self::NoOwner => "no_owner",
            Self::Store { .. } => "store_failed",
            Self::MasterKeyMissing => "master_key_missing",
            Self::MasterKeyInvalid => "master_key_invalid",
        })
    }
}

impl std::error::Error for SetupError {}

/// The organization and person a command acts for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scope {
    /// The organization every query is scoped to.
    pub org: OrgId,
    /// The person (the owner, or the first member) acting.
    pub user: UserId,
}

/// Most organizations listed when looking for the one to use.
const ORG_LIMIT: u32 = 50;

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

/// The database path and whether it is the default per-user location.
///
/// # Errors
/// [`SetupError::NoDbPath`] when nothing says where the database is.
pub fn db_path(
    env: &dyn Environment,
    explicit: Option<&str>,
) -> Result<(PathBuf, bool), SetupError> {
    if let Some(path) = explicit {
        return Ok((PathBuf::from(path), false));
    }
    if let Some(path) = env.get("NODIVARA_DB").filter(|p| !p.is_empty()) {
        return Ok((PathBuf::from(path), false));
    }
    let home = env.get("HOME").filter(|h| !h.is_empty());
    let dir = if cfg!(target_os = "macos") {
        home.map(|h| PathBuf::from(h).join("Library/Application Support/Nodivara"))
    } else {
        env.get("XDG_DATA_HOME")
            .filter(|d| !d.is_empty())
            .map(|d| PathBuf::from(d).join("nodivara"))
            .or_else(|| home.map(|h| PathBuf::from(h).join(".local/share/nodivara")))
    };
    match dir {
        Some(dir) => Ok((dir.join("nodivara.db"), true)),
        None => Err(SetupError::NoDbPath),
    }
}

/// Opens the database, creating it (owner-only) when `create` is set; otherwise a missing file
/// is [`SetupError::DbMissing`], which callers answer by pointing at `infra init`.
///
/// # Errors
/// [`SetupError`] for a missing, uncreatable or unopenable database.
pub fn open_database(
    env: &dyn Environment,
    explicit: Option<&str>,
    create: bool,
) -> Result<(Connection, PathBuf), SetupError> {
    let (path, is_default) = db_path(env, explicit)?;
    let shown = path.to_string_lossy().into_owned();
    if create {
        prepare_file(&path, is_default).map_err(|error| SetupError::DbOpen {
            path: shown.clone(),
            reason: error.to_string(),
        })?;
    } else if !path.is_file() {
        return Err(SetupError::DbMissing { path: shown });
    }
    match db::open(&path) {
        Ok(conn) => Ok((conn, path)),
        Err(error) => Err(SetupError::DbOpen {
            path: shown,
            reason: error.to_string(),
        }),
    }
}

/// The organization to use (`slug`, or the only one) and its owner.
///
/// # Errors
/// [`SetupError`] when there is no organization, the slug is unknown, several exist and none
/// was named, or the organization has no member.
pub fn resolve_scope(conn: &Connection, slug: Option<&str>) -> Result<Scope, SetupError> {
    let store = |error: db::Error| SetupError::Store {
        reason: error.to_string(),
    };
    let orgs = db::list_orgs(conn, ORG_LIMIT).map_err(store)?;
    let org = match (slug, orgs.as_slice()) {
        (_, []) => return Err(SetupError::NoOrg),
        (Some(slug), _) => match orgs.iter().find(|o| o.slug == slug) {
            Some(org) => org,
            None => {
                return Err(SetupError::UnknownOrg {
                    slug: slug.to_owned(),
                });
            }
        },
        (None, [only]) => only,
        (None, many) => {
            let slugs: Vec<&str> = many.iter().map(|o| o.slug.as_str()).collect();
            return Err(SetupError::ManyOrgs {
                slugs: slugs.join(", "),
            });
        }
    };
    let members = db::list_members(conn, &org.id).map_err(store)?;
    let owner = members
        .iter()
        .find(|m| m.role == Role::Owner)
        .or_else(|| members.first());
    match owner {
        Some(member) => Ok(Scope {
            org: org.id.clone(),
            user: member.user_id.clone(),
        }),
        None => Err(SetupError::NoOwner),
    }
}

/// The vault from `NODIVARA_MASTER_KEY` (and retired keys).
///
/// # Errors
/// [`SetupError::MasterKeyMissing`] or [`SetupError::MasterKeyInvalid`]; the error never carries
/// key material.
pub fn vault_from_env(env: &dyn Environment) -> Result<Vault, SetupError> {
    let Some(current) = env.get(vault::ENV_KEY).filter(|k| !k.trim().is_empty()) else {
        return Err(SetupError::MasterKeyMissing);
    };
    let retired = env.get(vault::ENV_RETIRED_KEYS).unwrap_or_default();
    let specs: Vec<&str> = retired
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    Vault::new(current.trim(), &specs).map_err(|_| SetupError::MasterKeyInvalid)
}

/// A vault with a throwaway key, for code paths that need a `Vault` value but will never open a
/// sealed secret (a keyless provider). It cannot open anything stored under the real key.
///
/// # Errors
/// [`SetupError::MasterKeyInvalid`] only if the built-in placeholder were rejected (a defect).
pub fn placeholder_vault() -> Result<Vault, SetupError> {
    Vault::new(&format!("unused:{}", "0".repeat(64)), &[]).map_err(|_| SetupError::MasterKeyInvalid)
}

/// Purpose label a provider key is sealed under (bound into the ciphertext).
pub const PROVIDER_KEY_PURPOSE: &str = "ai_provider_key";

/// An AI provider to add.
#[derive(Debug)]
pub struct NewProvider<'a> {
    /// Unique name inside the org.
    pub name: &'a str,
    /// Base URL of the OpenAI-compatible endpoint.
    pub base_url: &'a str,
    /// Default model.
    pub model: &'a str,
    /// JSON object text of provider settings.
    pub params_json: &'a str,
    /// Whether it becomes the org default (it always does when the org has none yet).
    pub is_default: bool,
}

/// Why a provider could not be added.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderError {
    /// `is_default` was asked for but the org already has a default.
    DefaultExists,
    /// A provider with this name exists.
    Exists,
    /// A storage or sealing failure (a fixed code or a database error text, never a key).
    Store {
        /// The reason.
        reason: String,
    },
}

/// Checks provider settings the same way the assistant will read them later, so an unusable
/// provider is refused now rather than at the first question.
///
/// # Errors
/// The settings error text (it never contains a key).
pub fn validate_provider_config(
    base_url: &str,
    model: &str,
    params_json: &str,
) -> Result<(), String> {
    ai::ProviderConfig::from_params(base_url, model, params_json)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// Adds a provider, sealing `key` (vault and plaintext) first when there is one. A sealed key is
/// removed again when the provider row cannot be created.
///
/// # Errors
/// [`ProviderError`].
pub fn add_provider(
    conn: &Connection,
    org: &OrgId,
    new: &NewProvider<'_>,
    key: Option<(&Vault, &str)>,
) -> Result<db::AiProviderId, ProviderError> {
    let has_default = match db::default_ai_provider(conn, org) {
        Ok(_) => true,
        Err(db::Error::NotFound) => false,
        Err(error) => {
            return Err(ProviderError::Store {
                reason: error.to_string(),
            });
        }
    };
    if new.is_default && has_default {
        return Err(ProviderError::DefaultExists);
    }
    // The first provider of an org becomes its default whatever the flag says, so the first
    // question never fails with "no default provider".
    let is_default = new.is_default || !has_default;
    let secret = match key {
        Some((vault, key)) => Some(
            vault
                .store(conn, org, PROVIDER_KEY_PURPOSE, key.as_bytes())
                .map_err(|error| ProviderError::Store {
                    reason: error.code().to_owned(),
                })?,
        ),
        None => None,
    };
    let created = db::create_ai_provider(
        conn,
        org,
        &db::NewAiProvider {
            name: new.name,
            base_url: new.base_url,
            default_model: new.model,
            secret: secret.as_ref(),
            params_json: new.params_json,
            is_default,
        },
    );
    match created {
        Ok(id) => Ok(id),
        Err(error) => {
            // Do not leave a sealed key behind for a provider that was not created.
            if let Some(id) = &secret {
                let _ = db::delete_secret(conn, org, id);
            }
            if error.is_constraint_violation() {
                Err(ProviderError::Exists)
            } else {
                Err(ProviderError::Store {
                    reason: error.to_string(),
                })
            }
        }
    }
}

/// Why a provider could not be removed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoveError {
    /// No provider has this name in the org.
    Unknown,
    /// A conversation still uses it.
    InUse,
    /// A storage failure.
    Store {
        /// The database error text.
        reason: String,
    },
}

/// Removes the provider called `name` and its sealed key.
///
/// # Errors
/// [`RemoveError`].
pub fn remove_provider(conn: &Connection, org: &OrgId, name: &str) -> Result<(), RemoveError> {
    let store = |error: db::Error| RemoveError::Store {
        reason: error.to_string(),
    };
    let rows = db::list_ai_providers(conn, org).map_err(store)?;
    let Some(row) = rows.iter().find(|r| r.name == name) else {
        return Err(RemoveError::Unknown);
    };
    match db::delete_ai_provider(conn, org, &row.id) {
        Ok(()) => {}
        Err(error) if error.is_constraint_violation() => return Err(RemoveError::InUse),
        Err(error) => return Err(store(error)),
    }
    if let Some(secret) = &row.secret_id {
        // The provider is gone, so its sealed key goes too.
        let _ = db::delete_secret(conn, org, secret);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    struct Fixed(BTreeMap<String, String>);

    impl Fixed {
        fn new(pairs: &[(&str, &str)]) -> Self {
            Self(
                pairs
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                    .collect(),
            )
        }
    }

    impl Environment for Fixed {
        fn get(&self, name: &str) -> Option<String> {
            self.0.get(name).cloned()
        }
    }

    #[test]
    fn explicit_path_beats_the_variable_and_the_home_directory() {
        let env = Fixed::new(&[("NODIVARA_DB", "/from/env.db"), ("HOME", "/home/x")]);
        assert_eq!(
            db_path(&env, Some("/explicit.db")),
            Ok((PathBuf::from("/explicit.db"), false))
        );
        assert_eq!(
            db_path(&env, None),
            Ok((PathBuf::from("/from/env.db"), false))
        );
        let home_only = Fixed::new(&[("HOME", "/home/x")]);
        let Ok((path, is_default)) = db_path(&home_only, None) else {
            unreachable!("a home directory is set")
        };
        assert!(is_default);
        assert!(path.starts_with("/home/x") && path.ends_with("nodivara.db"));
        assert_eq!(db_path(&Fixed::new(&[]), None), Err(SetupError::NoDbPath));
    }

    #[test]
    fn a_missing_database_is_reported_and_not_created() {
        let env = Fixed::new(&[]);
        let path =
            std::env::temp_dir().join(format!("bootstrap-missing-{}.db", std::process::id()));
        let shown = path.to_string_lossy().into_owned();
        let result = open_database(&env, Some(&shown), false);
        assert!(matches!(result, Err(SetupError::DbMissing { .. })));
        assert!(!path.exists());
    }

    #[test]
    fn master_key_errors_are_distinguished_and_carry_no_key() {
        assert_eq!(
            vault_from_env(&Fixed::new(&[])).err(),
            Some(SetupError::MasterKeyMissing)
        );
        let bad = Fixed::new(&[("NODIVARA_MASTER_KEY", "not-a-key-at-all")]);
        let error = vault_from_env(&bad).err();
        assert_eq!(error, Some(SetupError::MasterKeyInvalid));
        assert!(!format!("{error:?}").contains("not-a-key"));
        let good = Fixed::new(&[("NODIVARA_MASTER_KEY", &format!("main:{}", "ab".repeat(32)))]);
        assert!(vault_from_env(&good).is_ok());
    }

    #[test]
    fn scope_needs_one_organization_or_a_slug() -> Result<(), Box<dyn std::error::Error>> {
        let conn = db::open_in_memory()?;
        assert_eq!(resolve_scope(&conn, None), Err(SetupError::NoOrg));
        for slug in ["acme", "beta"] {
            let org = db::create_org(&conn, slug, slug, "free")?;
            let user = db::create_user(&conn, &format!("{slug}@example.test"), "Owner", None)?;
            db::add_member(&conn, &org.id, &user.id, Role::Owner)?;
        }
        assert!(matches!(
            resolve_scope(&conn, None),
            Err(SetupError::ManyOrgs { .. })
        ));
        assert!(matches!(
            resolve_scope(&conn, Some("nope")),
            Err(SetupError::UnknownOrg { .. })
        ));
        let scope = resolve_scope(&conn, Some("beta"))?;
        assert_eq!(db::get_org(&conn, &scope.org)?.slug, "beta");
        Ok(())
    }

    fn provider(name: &str, is_default: bool) -> NewProvider<'_> {
        NewProvider {
            name,
            base_url: "http://127.0.0.1:1/v1",
            model: "m",
            params_json: "{}",
            is_default,
        }
    }

    #[test]
    fn the_first_provider_becomes_the_default_whatever_the_flag()
    -> Result<(), Box<dyn std::error::Error>> {
        let conn = db::open_in_memory()?;
        let org = db::create_org(&conn, "acme", "acme", "free")?;
        let other = db::create_org(&conn, "beta", "beta", "free")?;
        add_provider(&conn, &org.id, &provider("first", false), None)
            .map_err(|e| format!("{e:?}"))?;
        assert_eq!(db::default_ai_provider(&conn, &org.id)?.name, "first");
        // A second one without the flag does not take it over.
        add_provider(&conn, &org.id, &provider("second", false), None)
            .map_err(|e| format!("{e:?}"))?;
        assert_eq!(db::default_ai_provider(&conn, &org.id)?.name, "first");
        // Asking for the default while one exists is still refused.
        assert_eq!(
            add_provider(&conn, &org.id, &provider("third", true), None),
            Err(ProviderError::DefaultExists)
        );
        // Another org is independent: it still has none.
        assert!(db::default_ai_provider(&conn, &other.id).is_err());
        add_provider(&conn, &other.id, &provider("solo", false), None)
            .map_err(|e| format!("{e:?}"))?;
        assert_eq!(db::default_ai_provider(&conn, &other.id)?.name, "solo");
        // Removing the default and adding again makes the new one the default.
        remove_provider(&conn, &org.id, "first").map_err(|e| format!("{e:?}"))?;
        add_provider(&conn, &org.id, &provider("fourth", false), None)
            .map_err(|e| format!("{e:?}"))?;
        assert_eq!(db::default_ai_provider(&conn, &org.id)?.name, "fourth");
        Ok(())
    }
}
