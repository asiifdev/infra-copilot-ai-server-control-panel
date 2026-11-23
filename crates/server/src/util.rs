//! Small helpers: query strings, the request language, JSON bodies and text cleaning.

use i18n::Locale;
use serde_json::{Map, Value};

/// Decodes `%XX` escapes (and `+` as a space, as in a query string). `None` for a malformed
/// escape or a result that is not UTF-8.
pub(crate) fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = text.get(i + 1..i + 3)?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// The value of the query parameter `name` (the first one), decoded.
pub(crate) fn query_get(query: Option<&str>, name: &str) -> Option<String> {
    query?.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        (percent_decode(key).as_deref() == Some(name))
            .then(|| percent_decode(value))
            .flatten()
    })
}

/// The language of a request: `?lang=`, then the best supported `Accept-Language` entry, then
/// English.
pub(crate) fn locale_of(query: Option<&str>, accept_language: Option<&str>) -> Locale {
    if let Some(locale) = query_get(query, "lang").and_then(|tag| Locale::parse(&tag)) {
        return locale;
    }
    let mut entries: Vec<(&str, u32)> = accept_language
        .unwrap_or_default()
        .split(',')
        .filter_map(|entry| {
            let mut parts = entry.split(';');
            let tag = parts.next()?.trim();
            let weight =
                parts
                    .find_map(|p| p.trim().strip_prefix("q="))
                    .map_or(Some(1000), |q| {
                        q.trim()
                            .parse::<f32>()
                            .ok()
                            .map(|f| (f.clamp(0.0, 1.0) * 1000.0) as u32)
                    })?;
            (weight > 0 && !tag.is_empty()).then_some((tag, weight))
        })
        .collect();
    // Stable: equal weights keep the order the client wrote them in.
    entries.sort_by_key(|(_, weight)| std::cmp::Reverse(*weight));
    entries
        .iter()
        .find_map(|(tag, _)| Locale::parse(tag))
        .unwrap_or_default()
}

/// A positive number from the query (`limit`), within `1..=max`, or `default` when absent.
/// `Err` when present but not a number in range.
pub(crate) fn limit_param(
    query: Option<&str>,
    name: &str,
    default: u32,
    max: u32,
) -> Result<u32, ()> {
    match query_get(query, name) {
        None => Ok(default),
        Some(text) => match text.parse::<u32>() {
            Ok(n) if (1..=max).contains(&n) => Ok(n),
            _ => Err(()),
        },
    }
}

/// Replaces control characters except newline and tab (text from servers and models is data, but
/// it must not carry terminal or markup surprises into a log or a title).
pub(crate) fn clean_text(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_control() && c != '\n' && c != '\t' {
                '\u{FFFD}'
            } else {
                c
            }
        })
        .collect()
}

/// Redacts secret-looking values in every string of a JSON value (inventory data comes from
/// servers: it should hold no secrets, and this is the second lock).
pub(crate) fn redact_json(value: &mut Value) {
    match value {
        Value::String(text) => *text = core_domain::redact::redact(text).text,
        Value::Array(items) => items.iter_mut().for_each(redact_json),
        Value::Object(map) => map.values_mut().for_each(redact_json),
        _ => {}
    }
}

/// A JSON object from a request body. An empty body is an empty object when `allow_empty`.
pub(crate) fn json_object(body: &[u8], allow_empty: bool) -> Option<Map<String, Value>> {
    if body.is_empty() {
        return allow_empty.then(Map::new);
    }
    match serde_json::from_slice::<Value>(body).ok()? {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

/// A string field: absent is `Ok(None)`, a non-string is an error.
pub(crate) fn opt_str<'a>(map: &'a Map<String, Value>, name: &str) -> Result<Option<&'a str>, ()> {
    match map.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text)),
        Some(_) => Err(()),
    }
}

/// Whether `text` has a control character.
pub(crate) fn has_control(text: &str) -> bool {
    text.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_decoding_is_strict() {
        assert_eq!(percent_decode("a%20b+c").as_deref(), Some("a b c"));
        assert_eq!(percent_decode("%zz"), None);
        assert_eq!(percent_decode("%e2%28"), None);
        assert_eq!(percent_decode("%"), None);
    }

    #[test]
    fn query_values_are_found_and_decoded() {
        assert_eq!(
            query_get(Some("a=1&lang=id"), "lang").as_deref(),
            Some("id")
        );
        assert_eq!(query_get(Some("a=1"), "lang"), None);
        assert_eq!(query_get(None, "lang"), None);
    }

    #[test]
    fn language_comes_from_the_query_then_the_header_then_english() {
        assert_eq!(locale_of(Some("lang=id"), Some("en")), Locale::Id);
        assert_eq!(
            locale_of(Some("lang=xx"), Some("id-ID,en;q=0.5")),
            Locale::Id
        );
        assert_eq!(
            locale_of(None, Some("fr;q=0.9, id;q=0.8, en;q=0.1")),
            Locale::Id
        );
        assert_eq!(locale_of(None, Some("en;q=0.2, id;q=0.9")), Locale::Id);
        assert_eq!(locale_of(None, Some("fr")), Locale::En);
        assert_eq!(locale_of(None, None), Locale::En);
    }

    #[test]
    fn limits_are_range_checked() {
        assert_eq!(limit_param(None, "limit", 10, 50), Ok(10));
        assert_eq!(limit_param(Some("limit=50"), "limit", 10, 50), Ok(50));
        assert!(limit_param(Some("limit=51"), "limit", 10, 50).is_err());
        assert!(limit_param(Some("limit=0"), "limit", 10, 50).is_err());
        assert!(limit_param(Some("limit=x"), "limit", 10, 50).is_err());
    }

    #[test]
    fn control_characters_are_replaced() {
        assert_eq!(clean_text("a\u{1b}[31mb\nc"), "a\u{fffd}[31mb\nc");
    }
}
