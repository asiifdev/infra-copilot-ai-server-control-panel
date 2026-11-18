//! The in-memory catalog and lookup with fallback.

use crate::locale::Locale;
use crate::parse::{ParseError, parse_catalog};
use crate::template::{Piece, pieces};
use std::collections::BTreeMap;
use std::fmt;

const EN_SOURCE: &str = include_str!("../locales/en.txt");
const ID_SOURCE: &str = include_str!("../locales/id.txt");

/// A stable message key (`area.subject.action`, lowercase).
///
/// Define keys as `const` so call sites never carry display text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MessageKey(&'static str);

impl MessageKey {
    /// Wraps a key string. Format is checked by the catalog tests, not at runtime.
    pub const fn new(key: &'static str) -> Self {
        MessageKey(key)
    }

    /// The key text.
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

/// The result of a lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Translated {
    /// The rendered text (or the key itself when `missing`).
    pub text: String,
    /// The key was absent in the requested locale and English was used.
    pub fell_back: bool,
    /// The key is absent even in English; `text` is the key.
    pub missing: bool,
    /// Placeholders that had no argument; they are left as `{name}` in `text`.
    pub unresolved: Vec<String>,
}

/// A catalog load error: which locale and the parse error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogError {
    /// The locale whose source failed.
    pub locale: Locale,
    /// The parse error with line number.
    pub source: ParseError,
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "catalog '{}': {}", self.locale.tag(), self.source)
    }
}

impl std::error::Error for CatalogError {}

/// Translation catalogs for all locales.
#[derive(Debug, Clone)]
pub struct Catalog {
    en: BTreeMap<String, String>,
    id: BTreeMap<String, String>,
}

fn load(locale: Locale, source: &str) -> Result<BTreeMap<String, String>, CatalogError> {
    let entries = parse_catalog(source).map_err(|source| CatalogError { locale, source })?;
    Ok(entries.into_iter().map(|e| (e.key, e.value)).collect())
}

impl Catalog {
    /// Loads the catalogs embedded in the binary.
    pub fn builtin() -> Result<Catalog, CatalogError> {
        Catalog::from_sources(EN_SOURCE, ID_SOURCE)
    }

    /// Builds a catalog from explicit sources (used by tests).
    pub fn from_sources(en: &str, id: &str) -> Result<Catalog, CatalogError> {
        Ok(Catalog {
            en: load(Locale::En, en)?,
            id: load(Locale::Id, id)?,
        })
    }

    fn messages(&self, locale: Locale) -> &BTreeMap<String, String> {
        match locale {
            Locale::En => &self.en,
            Locale::Id => &self.id,
        }
    }

    /// Raw template for a key in exactly this locale (no fallback).
    pub fn raw(&self, locale: Locale, key: &str) -> Option<&str> {
        self.messages(locale).get(key).map(String::as_str)
    }

    /// All keys of a locale, sorted.
    pub fn keys(&self, locale: Locale) -> impl Iterator<Item = &str> {
        self.messages(locale).keys().map(String::as_str)
    }

    /// Looks up `key` for `locale` and substitutes named `args`.
    ///
    /// Missing in `locale` -> English with `fell_back`. Missing in English ->
    /// the key itself with `missing`. A placeholder without an argument is left
    /// as `{name}` and listed in `unresolved`. Substituted values are never
    /// re-scanned for placeholders.
    pub fn get(&self, locale: Locale, key: MessageKey, args: &[(&str, &str)]) -> Translated {
        let (template, fell_back) = match self.raw(locale, key.as_str()) {
            Some(t) => (t, false),
            None => match self.raw(Locale::En, key.as_str()) {
                Some(t) => (t, true),
                None => {
                    return Translated {
                        text: key.as_str().to_owned(),
                        fell_back: false,
                        missing: true,
                        unresolved: Vec::new(),
                    };
                }
            },
        };
        let (text, unresolved) = render(template, args);
        Translated {
            text,
            fell_back,
            missing: false,
            unresolved,
        }
    }
}

fn render(template: &str, args: &[(&str, &str)]) -> (String, Vec<String>) {
    let mut out = String::with_capacity(template.len());
    let mut unresolved = Vec::new();
    // Templates are validated at load; a failure here can only come from a
    // hand-built value, in which case the raw text is returned unchanged.
    let Ok(parts) = pieces(template) else {
        return (template.to_owned(), unresolved);
    };
    for part in parts {
        match part {
            Piece::Text(t) => out.push_str(t),
            Piece::Brace(c) => out.push(c),
            Piece::Placeholder(name) => match args.iter().find(|(n, _)| *n == name) {
                Some((_, v)) => out.push_str(v),
                None => {
                    out.push('{');
                    out.push_str(name);
                    out.push('}');
                    if !unresolved.iter().any(|u| u == name) {
                        unresolved.push(name.to_owned());
                    }
                }
            },
        }
    }
    (out, unresolved)
}
