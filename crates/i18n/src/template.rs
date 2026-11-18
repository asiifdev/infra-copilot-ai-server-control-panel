//! Message template syntax: literal text with `{name}` placeholders.
//!
//! A placeholder name matches `[a-z][a-z0-9_]*`. `{{` and `}}` produce literal braces.

use std::collections::BTreeSet;
use std::fmt;

/// A template syntax problem (the caller adds the line number).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateError {
    /// A `{` without a matching `}`.
    UnclosedBrace,
    /// A `}` that is not part of `}}` or a placeholder.
    StrayClosingBrace,
    /// Placeholder name is empty or not `[a-z][a-z0-9_]*`.
    InvalidPlaceholder(String),
}

impl fmt::Display for TemplateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TemplateError::UnclosedBrace => {
                f.write_str("unclosed '{' (use '{{' for a literal brace)")
            }
            TemplateError::StrayClosingBrace => {
                f.write_str("stray '}' (use '}}' for a literal brace)")
            }
            TemplateError::InvalidPlaceholder(n) => {
                write!(
                    f,
                    "invalid placeholder name '{n}' (expected [a-z][a-z0-9_]*)"
                )
            }
        }
    }
}

impl std::error::Error for TemplateError {}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Piece<'a> {
    Text(&'a str),
    Brace(char),
    Placeholder(&'a str),
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some('a'..='z'))
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

pub(crate) fn pieces(template: &str) -> Result<Vec<Piece<'_>>, TemplateError> {
    let bytes = template.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b != b'{' && b != b'}' {
            i += 1;
            continue;
        }
        if start < i {
            out.push(Piece::Text(&template[start..i]));
        }
        let doubled = bytes.get(i + 1) == Some(&b);
        if doubled {
            out.push(Piece::Brace(char::from(b)));
            i += 2;
        } else if b == b'}' {
            return Err(TemplateError::StrayClosingBrace);
        } else {
            let rest = &template[i + 1..];
            let close = rest.find('}').ok_or(TemplateError::UnclosedBrace)?;
            let name = &rest[..close];
            if !valid_name(name) {
                return Err(TemplateError::InvalidPlaceholder(name.to_owned()));
            }
            out.push(Piece::Placeholder(name));
            i += close + 2;
        }
        start = i;
    }
    if start < bytes.len() {
        out.push(Piece::Text(&template[start..]));
    }
    Ok(out)
}

/// The set of placeholder names used by a template.
pub fn placeholders(template: &str) -> Result<BTreeSet<&str>, TemplateError> {
    Ok(pieces(template)?
        .into_iter()
        .filter_map(|p| match p {
            Piece::Placeholder(n) => Some(n),
            _ => None,
        })
        .collect())
}
