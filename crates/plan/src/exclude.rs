//! What a backup leaves out by default, and what a person must always be told about it
//! (skill `migration-planning`: rebuildable data is excluded, but the list is shown).

use i18n::MessageKey;

use crate::error::PlanError;

/// Exclusion ids a fresh plan starts with: data that can be rebuilt on the target. Each id has a
/// description in the catalog (`plan.exclude_<id>.text`).
pub const DEFAULT_EXCLUDES: [&str; 5] = ["logs", "cache", "node_modules", "tmp", "pycache"];

/// Longest accepted exclusion pattern, in bytes.
pub(crate) const MAX_PATTERN_LEN: usize = 200;

/// Whether a pattern would exclude (almost) everything. A plan carrying one would back up nothing
/// while looking healthy, so it is refused. Equivalent spellings (`/**`, `./*`, `***`, `//`,
/// `**/**`, ...) are caught by ignoring separators and `.` segments: if nothing but `*` remains,
/// the pattern matches everything.
fn matches_everything(pattern: &str) -> bool {
    pattern
        .split(['/', '\\'])
        .map(str::trim)
        .filter(|seg| !seg.is_empty() && *seg != ".")
        .all(|seg| seg.chars().all(|c| c == '*'))
}

/// The default exclusion ids as owned strings, for building a spec.
#[must_use]
pub fn default_excludes() -> Vec<String> {
    DEFAULT_EXCLUDES.iter().map(|id| (*id).to_owned()).collect()
}

/// The catalog key that describes a well-known exclusion id, or `None` for a custom pattern.
#[must_use]
pub fn describe_key(id: &str) -> Option<MessageKey> {
    match id {
        "logs" => Some(MessageKey::new("plan.exclude_logs.text")),
        "cache" => Some(MessageKey::new("plan.exclude_cache.text")),
        "node_modules" => Some(MessageKey::new("plan.exclude_node_modules.text")),
        "tmp" => Some(MessageKey::new("plan.exclude_tmp.text")),
        "pycache" => Some(MessageKey::new("plan.exclude_pycache.text")),
        _ => None,
    }
}

/// Checks one exclusion entry: not blank, bounded, no control characters, no `..` segment and not
/// a pattern that matches everything.
///
/// # Errors
/// [`PlanError::Invalid`] naming `backup.exclude`.
pub fn validate_pattern(pattern: &str) -> Result<(), PlanError> {
    let bad = PlanError::Invalid("backup.exclude");
    let trimmed = pattern.trim();
    if trimmed.is_empty()
        || pattern.len() > MAX_PATTERN_LEN
        || pattern.chars().any(char::is_control)
        || matches_everything(trimmed)
        || trimmed.split(['/', '\\']).any(|seg| seg == "..")
    {
        return Err(bad);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_and_described() {
        for id in DEFAULT_EXCLUDES {
            assert_eq!(validate_pattern(id), Ok(()));
            assert!(describe_key(id).is_some(), "{id} has no description");
        }
        assert_eq!(default_excludes().len(), DEFAULT_EXCLUDES.len());
    }

    #[test]
    fn rejects_patterns_that_exclude_everything_or_are_malformed() {
        for bad in [
            "", "  ", "*", "**", "/", ".", "**/*", " * ", "/**", "**/", "***", "//", "./*", "./**",
            "**/**", "*/", "/**/*", "./", "\\*", ". / .", "a\nb", "../etc", "a/../b", "a\\..\\b",
        ] {
            assert!(validate_pattern(bad).is_err(), "{bad:?} should be refused");
        }
        assert!(validate_pattern(&"x".repeat(MAX_PATTERN_LEN + 1)).is_err());
        assert!(validate_pattern("*.log").is_ok());
        assert!(validate_pattern("var/cache").is_ok());
    }

    #[test]
    fn custom_patterns_have_no_description() {
        assert!(describe_key("*.log").is_none());
    }
}
