//! Translation catalog loader (en, id) with stable keys. Display text never lives in code.
//!
//! Depends on `core-domain` only (see docs/decisions/0007 and the rust-conventions skill).
//! Catalog format and rationale: docs/decisions/0008. No external dependencies.
//!
//! ```
//! use i18n::{Catalog, Locale, MessageKey};
//! const UNREACHABLE: MessageKey = MessageKey::new("error.node.unreachable");
//! let catalog = Catalog::builtin().unwrap_or_else(|e| panic!("{e}"));
//! let t = catalog.get(Locale::parse_or_default("id-ID"), UNREACHABLE, &[("name", "web-1")]);
//! assert_eq!(t.text, "Node web-1 tidak dapat dijangkau.");
//! ```

mod catalog;
mod locale;
mod parse;
mod template;

pub use catalog::{Catalog, CatalogError, MessageKey, Translated};
pub use locale::Locale;
pub use parse::{Entry, ParseError, ParseErrorKind, is_valid_key, parse_catalog};
pub use template::{TemplateError, placeholders};
