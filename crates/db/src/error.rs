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
    /// A migration failed.
    Migration(rusqlite_migration::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => f.write_str("not found"),
            Self::Invalid(what) => write!(f, "invalid value: {what}"),
            Self::Config(what) => write!(f, "database configuration error: {what}"),
            Self::Sqlite(err) => write!(f, "database error: {err}"),
            Self::Migration(err) => write!(f, "migration error: {err}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(err) => Some(err),
            Self::Migration(err) => Some(err),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for Error {
    fn from(err: rusqlite::Error) -> Self {
        Self::Sqlite(err)
    }
}

impl From<rusqlite_migration::Error> for Error {
    fn from(err: rusqlite_migration::Error) -> Self {
        Self::Migration(err)
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
