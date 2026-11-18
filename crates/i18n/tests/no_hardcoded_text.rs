//! Hard-coded display text rejector for the UI.
//!
//! Scans `ui/src/**/*.svelte` (when the directory exists) and fails on:
//! - a literal text node containing letters, outside `{...}` expressions;
//! - a `title`, `aria-label` or `placeholder` attribute whose quoted value has
//!   letters and no `{...}` expression.
//!
//! HONEST LIMITS: this is a pattern-based scanner, not proof. It does not parse
//! Svelte; it only tracks tags, quotes and brace depth. It will miss text built
//! in `<script>` (string literals), other attributes (`alt`, `label`, ...),
//! text hidden in unquoted attributes, and spread props. It can also
//! false-positive on literal non-display text (e.g. a unit like "MB") in markup;
//! put such values in an expression, e.g. `{'MB'}`, or in the catalog. Passing
//! means "no obvious violations", not "fully localized". If `ui/` does not
//! exist yet, the scan test passes with a note.

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

const CHECKED_ATTRS: [&str; 3] = ["title", "aria-label", "placeholder"];

fn has_letters(s: &str) -> bool {
    let mut out = String::new();
    let mut in_entity = false;
    for c in s.chars() {
        match c {
            '&' => in_entity = true,
            ';' if in_entity => in_entity = false,
            _ if !in_entity => out.push(c),
            _ => {}
        }
    }
    out.chars().any(char::is_alphabetic)
}

/// Skips a `{...}` expression starting at `i` (which points at `{`); returns the index after it.
fn skip_braces(b: &[u8], mut i: usize) -> usize {
    let mut depth = 0usize;
    while i < b.len() {
        match b[i] {
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    i
}

fn find_from(s: &str, from: usize, pat: &str) -> usize {
    s[from..].find(pat).map_or(s.len(), |p| from + p)
}

/// Returns human-readable violations for one Svelte source.
pub fn scan(src: &str) -> Vec<String> {
    let b = src.as_bytes();
    let mut found = Vec::new();
    let mut i = 0;
    let mut text_start = 0;
    let flush = |found: &mut Vec<String>, from: usize, to: usize| {
        if from < to && has_letters(&src[from..to]) {
            found.push(format!("literal text: {:?}", src[from..to].trim()));
        }
    };
    while i < b.len() {
        match b[i] {
            b'{' => {
                flush(&mut found, text_start, i);
                i = skip_braces(b, i);
                text_start = i;
            }
            b'<' if src[i..].starts_with("<!--") => {
                flush(&mut found, text_start, i);
                i = find_from(src, i, "-->").saturating_add(3).min(src.len());
                text_start = i;
            }
            b'<' if b
                .get(i + 1)
                .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'/') =>
            {
                flush(&mut found, text_start, i);
                i = scan_tag(src, i, &mut found);
                text_start = i;
            }
            _ => i += 1,
        }
    }
    flush(&mut found, text_start, b.len());
    found
}

/// Scans a tag starting at `<`; returns the index after the closing `>`.
fn scan_tag(src: &str, start: usize, found: &mut Vec<String>) -> usize {
    let b = src.as_bytes();
    let mut i = start + 1;
    if b[i] == b'/' {
        return find_from(src, i, ">").saturating_add(1).min(src.len());
    }
    let name_start = i;
    while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'-' || b[i] == b':') {
        i += 1;
    }
    let tag = src[name_start..i].to_ascii_lowercase();
    let mut self_closed = false;
    while i < b.len() && b[i] != b'>' {
        if b[i] == b'{' {
            i = skip_braces(b, i);
        } else if b[i].is_ascii_alphabetic() {
            let a0 = i;
            while i < b.len() && !b[i].is_ascii_whitespace() && !matches!(b[i], b'=' | b'>' | b'/')
            {
                i += 1;
            }
            let attr = src[a0..i].to_ascii_lowercase();
            if b.get(i) == Some(&b'=') && matches!(b.get(i + 1), Some(b'"' | b'\'')) {
                let q = b[i + 1];
                let v0 = i + 2;
                let mut j = v0;
                let mut depth = 0usize;
                while j < b.len() && !(b[j] == q && depth == 0) {
                    match b[j] {
                        b'{' => depth += 1,
                        b'}' => depth = depth.saturating_sub(1),
                        _ => {}
                    }
                    j += 1;
                }
                let value = &src[v0..j.min(src.len())];
                if CHECKED_ATTRS.contains(&attr.as_str())
                    && !value.contains('{')
                    && has_letters(value)
                {
                    found.push(format!("literal {attr} attribute: {value:?}"));
                }
                i = j + 1;
            }
        } else {
            self_closed = b[i] == b'/';
            i += 1;
        }
    }
    let after = (i + 1).min(src.len());
    if (tag == "script" || tag == "style") && !self_closed {
        let close = format!("</{tag}");
        let end = find_from(&src.to_ascii_lowercase(), after, &close);
        return find_from(src, end, ">").saturating_add(1).min(src.len());
    }
    after
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), Box<dyn Error>> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n != "node_modules") {
                collect(&path, out)?;
            }
        } else if path.extension().is_some_and(|e| e == "svelte") {
            out.push(path);
        }
    }
    Ok(())
}

#[test]
fn ui_has_no_hardcoded_display_text() -> Result<(), Box<dyn Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/src");
    if !root.is_dir() {
        // Not a failure: the UI does not exist yet (nothing is reported; see module docs). The scanner is covered by the self-tests below.
        return Ok(());
    }
    let mut files = Vec::new();
    collect(&root, &mut files)?;
    files.sort();
    let mut report = Vec::new();
    for f in &files {
        for v in scan(&fs::read_to_string(f)?) {
            report.push(format!("{}: {v}", f.display()));
        }
    }
    assert!(
        report.is_empty(),
        "hard-coded display text found:\n{}",
        report.join("\n")
    );
    Ok(())
}

#[test]
fn scanner_flags_literal_text_and_attributes() {
    assert_eq!(scan("<h1>Nodes</h1>").len(), 1);
    assert_eq!(scan("<p>hello {name}</p>").len(), 1);
    assert_eq!(scan("<input placeholder=\"Search\">").len(), 1);
    assert_eq!(
        scan("<button title='Close' aria-label=\"Close\">{t('x.y.z')}</button>").len(),
        2
    );
    assert_eq!(scan("{#if a}<span>Yes</span>{/if}").len(), 1);
}

#[test]
fn scanner_accepts_expressions_and_non_text() {
    let ok = "<script lang=\"ts\">const a = 'literal in script'; if (1 < 2) {}</script>\n\
              <style>.a > .b { color: red }</style>\n\
              <!-- a comment with words -->\n\
              {#each rows as r}<td>{t('nodes.column.name')}</td>{/each}\n\
              <input placeholder={t('a.b.c')} title=\"{t('a.b.c')}\" aria-label=\"{x}\">\n\
              <p>&nbsp;·&mdash;</p><img alt=\"\" src=\"a.png\" />";
    assert_eq!(scan(ok), Vec::<String>::new());
}
