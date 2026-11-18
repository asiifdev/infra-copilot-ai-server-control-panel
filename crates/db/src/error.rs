//! Error type of the persistence layer.

use std::fmt;

/// Result alias used across the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Everything that can go wrong. Messages never contain secret values.
#[derive(Debug)]
pub enum Error {
    /// The row does not exist **or belongs to another org** (deliberately indistinguishable).
    NotFound,
    /// An identifier or enum value had the wrong shape.
    Invalid(&'static str),
    /// A connection setting could not be applied.
    Config(&'static str),
    /// The database refused the operation (constraint, lock, I/O).
    Sqlite(rusqlite::Error),
    /// A migration failed and was rolled back.
    Migration {
        /// The 1-based migration that failed.
        version: usize,
        /// The underlying database error.
        source: rusqlite::Error,
    },
    /// The database was created by a newer build; refusing to touch it.
    DatabaseNewer {
        /// Schema version found in the database.
        found: usize,
        /// Newest schema version this build knows.
        supported: usize,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => f.write_str("not found"),
            Self::Invalid(what) => write!(f, "invalid value: {what}"),
            Self::Config(what) => write!(f, "database configuration error: {what}"),
            Self::Sqlite(err) => write!(f, "database error: {err}"),
            Self::Migration { version, source } => {
                write!(f, "migration {version} failed: {source}")
            }
            Self::DatabaseNewer { found, supported } => write!(
                f,
                "database schema version {found} is newer than the supported {supported}"
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(err) => Some(err),
            Self::Migration { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for Error {
    fn from(err: rusqlite::Error) -> Self {
        Self::Sqlite(err)
    }
}

impl Error {
    /// True when the database rejected the write because of a constraint (FK, CHECK, UNIQUE, trigger).
    #[must_use]
    pub fn is_constraint_violation(&self) -> bool {
        matches!(
            self,
            Self::Sqlite(rusqlite::Error::SqliteFailure(e, _))
                if e.code == rusqlite::ErrorCode::ConstraintViolation
        )
    }
}
