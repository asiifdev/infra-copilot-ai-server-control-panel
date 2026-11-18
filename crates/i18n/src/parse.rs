//! Parser for the catalog text format.
//!
//! One entry per line: `key = text with {placeholder}`. Lines starting with `#`
//! and blank lines are ignored. Only the first `=` separates key and value.

use crate::template::{TemplateError, pieces};
use std::fmt;

/// One parsed catalog line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Stable message key.
    pub key: String,
    /// Raw template text.
    pub value: String,
    /// 1-based source line.
    pub line: usize,
}

/// What went wrong on a catalog line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseErrorKind {
    /// No `=` on the line.
    MissingSeparator,
    /// Nothing before the `=`.
    EmptyKey,
    /// Key is not `area.subject.action` in lowercase.
    InvalidKey(String),
    /// Nothing after the `=`.
    EmptyValue,
    /// The key already appeared on an earlier line.
    DuplicateKey {
        /// Line of the first occurrence.
        first_line: usize,
    },
    /// Bad `{placeholder}` syntax in the value.
    Template(TemplateError),
}

/// A catalog parse error with its 1-based line number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// 1-based line number.
    pub line: usize,
    /// The problem.
    pub kind: ParseErrorKind,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: ", self.line)?;
        match &self.kind {
            ParseErrorKind::MissingSeparator => f.write_str("missing '=' between key and text"),
            ParseErrorKind::EmptyKey => f.write_str("empty key"),
            ParseErrorKind::InvalidKey(k) => {
                write!(
                    f,
                    "invalid key '{k}' (expected area.subject.action, lowercase)"
                )
            }
            ParseErrorKind::EmptyValue => f.write_str("empty text"),
            ParseErrorKind::DuplicateKey { first_line } => {
                write!(f, "duplicate key (first defined on line {first_line})")
            }
            ParseErrorKind::Template(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ParseError {}

/// True if `key` is exactly three dot-separated segments, each `[a-z][a-z0-9_]*`.
pub fn is_valid_key(key: &str) -> bool {
    let mut count = 0;
    for seg in key.split('.') {
        let mut chars = seg.chars();
        let ok = matches!(chars.next(), Some('a'..='z'))
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !ok {
            return false;
        }
        count += 1;
    }
    count == 3
}

/// Parses a whole catalog source.
pub fn parse_catalog(source: &str) -> Result<Vec<Entry>, ParseError> {
    let mut entries: Vec<Entry> = Vec::new();
    for (idx, raw) in source.lines().enumerate() {
        let line = idx + 1;
        let text = raw.trim();
        if text.is_empty() || text.starts_with('#') {
            continue;
        }
        let err = |kind| ParseError { line, kind };
        let (key, value) = text
            .split_once('=')
            .ok_or_else(|| err(ParseErrorKind::MissingSeparator))?;
        let (key, value) = (key.trim(), value.trim());
        if key.is_empty() {
            return Err(err(ParseErrorKind::EmptyKey));
        }
        if !is_valid_key(key) {
            return Err(err(ParseErrorKind::InvalidKey(key.to_owned())));
        }
        if value.is_empty() {
            return Err(err(ParseErrorKind::EmptyValue));
        }
        pieces(value).map_err(|e| err(ParseErrorKind::Template(e)))?;
        if let Some(first) = entries.iter().find(|e| e.key == key) {
            return Err(err(ParseErrorKind::DuplicateKey {
                first_line: first.line,
            }));
        }
        entries.push(Entry {
            key: key.to_owned(),
            value: value.to_owned(),
            line,
        });
    }
    Ok(entries)
}
