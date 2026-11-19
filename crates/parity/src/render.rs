//! Turns a finding into localized text. All wording lives in the catalog (docs/design.md 14.1).

use i18n::{Catalog, Locale, MessageKey};
use inventory::{Finding, Severity};

use crate::honesty::PARTIAL_ARG;
use crate::rules::UNKNOWN_ARG;

const SEVERITY_CRITICAL: MessageKey = MessageKey::new("parity.severity.critical");
const SEVERITY_IMPORTANT: MessageKey = MessageKey::new("parity.severity.important");
const SEVERITY_INFO: MessageKey = MessageKey::new("parity.severity.info");
const NOTE_PARTIAL: MessageKey = MessageKey::new("parity.note.partial");
const SIDE_SOURCE: MessageKey = MessageKey::new("parity.side.source");
const SIDE_TARGET: MessageKey = MessageKey::new("parity.side.target");
const REASON_DEV: MessageKey = MessageKey::new("parity.reason.dev");
const REASON_UNSUPERVISED: MessageKey = MessageKey::new("parity.reason.unsupervised");
const ARG_UNKNOWN: MessageKey = MessageKey::new("parity.arg.unknown");

/// Localized text for one finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedFinding {
    /// Localized severity name.
    pub severity_label: String,
    /// Short headline.
    pub title: String,
    /// Plain-language explanation.
    pub explanation: String,
    /// What to do about it.
    pub advice: String,
    /// Present when the finding rests on an incomplete inventory.
    pub note: Option<String>,
}

struct RuleKeys {
    title: MessageKey,
    explanation: MessageKey,
    advice: MessageKey,
}

/// Catalog keys of a rule: `PAR-001` -> `parity.par001.title|explanation|advice`. Keys are
/// `const` (not built at runtime) so they stay greppable and checked by the catalog tests.
fn keys_for(rule_id: &str) -> RuleKeys {
    match rule_id {
        "PAR-001" => RuleKeys {
            title: MessageKey::new("parity.par001.title"),
            explanation: MessageKey::new("parity.par001.explanation"),
            advice: MessageKey::new("parity.par001.advice"),
        },
        "PAR-006" => RuleKeys {
            title: MessageKey::new("parity.par006.title"),
            explanation: MessageKey::new("parity.par006.explanation"),
            advice: MessageKey::new("parity.par006.advice"),
        },
        "PAR-008" => RuleKeys {
            title: MessageKey::new("parity.par008.title"),
            explanation: MessageKey::new("parity.par008.explanation"),
            advice: MessageKey::new("parity.par008.advice"),
        },
        "PAR-010" => RuleKeys {
            title: MessageKey::new("parity.par010.title"),
            explanation: MessageKey::new("parity.par010.explanation"),
            advice: MessageKey::new("parity.par010.advice"),
        },
        // A rule id this build has no text for (for example from a newer rule set).
        _ => RuleKeys {
            title: MessageKey::new("parity.unknown.title"),
            explanation: MessageKey::new("parity.unknown.explanation"),
            advice: MessageKey::new("parity.unknown.advice"),
        },
    }
}

/// Renders `finding` in `locale`. Missing keys fall back to English, then to the key itself
/// (see [`Catalog::get`]).
#[must_use]
pub fn render(finding: &Finding, catalog: &Catalog, locale: Locale) -> RenderedFinding {
    let mut args: Vec<(String, String)> = finding
        .args
        .iter()
        .map(|(name, value)| {
            // `side` holds a wire value and `UNKNOWN_ARG` a sentinel; show localized words instead.
            let key = match (name.as_str(), value.as_str()) {
                ("side", "source") => Some(SIDE_SOURCE),
                ("side", "target") => Some(SIDE_TARGET),
                ("reason", "dev") => Some(REASON_DEV),
                ("reason", "unsupervised") => Some(REASON_UNSUPERVISED),
                (_, UNKNOWN_ARG) => Some(ARG_UNKNOWN),
                _ => None,
            };
            let shown = key.map_or_else(|| value.clone(), |k| catalog.get(locale, k, &[]).text);
            (name.clone(), shown)
        })
        .collect();
    args.push(("rule_id".to_owned(), finding.rule_id.clone()));
    let pairs: Vec<(&str, &str)> = args.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect();

    let keys = keys_for(&finding.rule_id);
    let text = |key| catalog.get(locale, key, &pairs).text;
    let severity_key = match finding.severity {
        Severity::Critical => SEVERITY_CRITICAL,
        Severity::Important => SEVERITY_IMPORTANT,
        Severity::Info => SEVERITY_INFO,
    };
    RenderedFinding {
        severity_label: text(severity_key),
        title: text(keys.title),
        explanation: text(keys.explanation),
        advice: text(keys.advice),
        note: (finding.args.get(PARTIAL_ARG).map(String::as_str) == Some("true"))
            .then(|| text(NOTE_PARTIAL)),
    }
}
