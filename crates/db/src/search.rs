//! Full-text search over inventory items, findings and active memories (ADR 0026).
//!
//! The index is maintained by database triggers (migration 0009), so it cannot drift from the
//! tables it reads. Every hit is filtered by the org in SQL, and only the newest inventory of each
//! node and the newest comparison are searchable (older snapshots are history).
//!
//! The query text comes from a model, so it never reaches FTS5 as written: [`fts_query`] reduces
//! it to quoted alphanumeric words. No operator, column filter, prefix star, NEAR group or quote
//! can survive.

use crate::error::Result;
use crate::ids::OrgId;
use rusqlite::{Connection, params};

/// Most words kept from a query.
pub const MAX_QUERY_TOKENS: usize = 12;
/// Longest word kept, in characters.
pub const MAX_TOKEN_CHARS: usize = 64;
/// Longest part of the raw query examined, in characters.
const MAX_RAW_CHARS: usize = 500;
/// Most hits one search returns.
pub const MAX_HITS: u32 = 50;

/// What a hit is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchKind {
    /// An inventory item of a node's latest inventory.
    Item,
    /// A finding of the latest comparison.
    Finding,
    /// An active organization memory.
    Memory,
}

/// One search result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    /// What it is.
    pub kind: SearchKind,
    /// Stable reference such as `item:listener:tcp:8080`, `finding:PAR-001:cron:backup` or
    /// `memory:<id>`.
    pub reference: String,
    /// Short title (the item key, the rule and subject, the memory kind).
    pub title: String,
    /// The start of the indexed text.
    pub snippet: String,
    /// The node an item belongs to.
    pub node: Option<String>,
}

/// Reduces untrusted text to an FTS5 query of quoted words, or `None` when no word is left.
/// Words are runs of alphanumeric characters; the operator words AND, OR, NOT and NEAR are
/// dropped; at most [`MAX_QUERY_TOKENS`] distinct words of [`MAX_TOKEN_CHARS`] characters stay.
/// `any` joins them with OR (recall), otherwise they are all required.
#[must_use]
pub fn fts_query(raw: &str, any: bool) -> Option<String> {
    let limited: String = raw.chars().take(MAX_RAW_CHARS).collect();
    let mut words: Vec<String> = Vec::new();
    for word in limited.split(|c: char| !c.is_alphanumeric()) {
        if word.is_empty() {
            continue;
        }
        let word: String = word.chars().take(MAX_TOKEN_CHARS).collect();
        let lower = word.to_lowercase();
        if matches!(lower.as_str(), "and" | "or" | "not" | "near") || words.contains(&lower) {
            continue;
        }
        words.push(lower);
        if words.len() == MAX_QUERY_TOKENS {
            break;
        }
    }
    if words.is_empty() {
        return None;
    }
    let quoted: Vec<String> = words.iter().map(|w| format!("\"{w}\"")).collect();
    Some(quoted.join(if any { " OR " } else { " " }))
}

const SEARCH_SQL: &str = "\
SELECT d.kind, d.ref, d.title, substr(d.body, 1, 200), \
       (SELECT n.name FROM inventories i JOIN nodes n ON n.org_id = i.org_id AND n.id = i.node_id \
         WHERE i.org_id = d.org_id AND i.id = d.owner_id AND d.kind = 'item') \
FROM search_fts JOIN search_docs d ON d.id = search_fts.rowid \
WHERE search_fts MATCH ?1 AND d.org_id = ?2 \
  AND (d.kind = 'memory' \
       OR (d.kind = 'item' AND d.owner_id IN ( \
            SELECT (SELECT l.id FROM inventories l WHERE l.org_id = i.org_id AND l.node_id = i.node_id \
                    ORDER BY l.taken_at DESC, l.created_at DESC, l.id DESC LIMIT 1) \
            FROM inventories i WHERE i.org_id = ?2)) \
       OR (d.kind = 'finding' AND d.owner_id = ( \
            SELECT c.id FROM comparisons c WHERE c.org_id = ?2 \
            ORDER BY c.created_at DESC, c.id DESC LIMIT 1))) \
ORDER BY bm25(search_fts, 4.0, 1.0), d.id \
LIMIT ?3";

fn run(conn: &Connection, org: &OrgId, query: &str, limit: u32) -> Result<Vec<SearchHit>> {
    let mut stmt = conn.prepare(SEARCH_SQL)?;
    let rows = stmt.query_map(params![query, org.as_str(), limit], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
        ))
    })?;
    let mut hits = Vec::new();
    for row in rows {
        let (kind, reference, title, snippet, node) = row?;
        hits.push(SearchHit {
            kind: match kind.as_str() {
                "item" => SearchKind::Item,
                "finding" => SearchKind::Finding,
                _ => SearchKind::Memory,
            },
            reference,
            title,
            snippet,
            node,
        });
    }
    Ok(hits)
}

/// Ranked hits for `query` among this org's current inventory items, latest findings and active
/// memories, at most `limit` (capped at [`MAX_HITS`]). All words must match; when nothing does,
/// any word may. Text that holds no word returns no hits.
///
/// # Errors
/// Database failure.
pub fn search(conn: &Connection, org: &OrgId, query: &str, limit: u32) -> Result<Vec<SearchHit>> {
    let limit = limit.clamp(1, MAX_HITS);
    let Some(all) = fts_query(query, false) else {
        return Ok(Vec::new());
    };
    let hits = run(conn, org, &all, limit)?;
    if !hits.is_empty() {
        return Ok(hits);
    }
    match fts_query(query, true) {
        Some(any) if any != all => run(conn, org, &any, limit),
        _ => Ok(hits),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_query_keeps_only_quoted_words() {
        assert_eq!(
            fts_query("nginx 8080", false).as_deref(),
            Some("\"nginx\" \"8080\"")
        );
        assert_eq!(
            fts_query("nginx 8080", true).as_deref(),
            Some("\"nginx\" OR \"8080\"")
        );
    }

    #[test]
    fn operators_filters_quotes_and_stars_cannot_survive() {
        for evil in [
            "\"unbalanced",
            "a OR b",
            "NEAR(a b, 2)",
            "title:secret",
            "body : x",
            "ngi*",
            "^start",
            "a AND NOT b",
            "-- ; DROP TABLE memories",
            "(((",
            "\"\"\"",
        ] {
            for any in [false, true] {
                if let Some(q) = fts_query(evil, any) {
                    let stripped = q.replace(" OR ", " ");
                    for part in stripped.split(' ') {
                        assert!(
                            part.len() >= 3
                                && part.starts_with('"')
                                && part.ends_with('"')
                                && part[1..part.len() - 1].chars().all(char::is_alphanumeric),
                            "{evil:?} -> {q:?}"
                        );
                    }
                }
            }
        }
        assert_eq!(fts_query("OR AND NOT NEAR", false), None);
        assert_eq!(fts_query("  \"*:()  ", false), None);
        assert_eq!(fts_query("", false), None);
    }

    #[test]
    fn size_is_capped() {
        let many = (0..100)
            .map(|i| format!("w{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let q = fts_query(&many, false).unwrap_or_default();
        assert_eq!(q.matches('"').count(), MAX_QUERY_TOKENS * 2);
        let long = "x".repeat(10_000);
        let q = fts_query(&long, false).unwrap_or_default();
        assert!(q.len() <= MAX_TOKEN_CHARS + 2);
        // Words past the raw limit are never examined.
        let late = format!("{} needle", "a ".repeat(400));
        assert!(
            !fts_query(&late, false)
                .unwrap_or_default()
                .contains("needle")
        );
    }
}
