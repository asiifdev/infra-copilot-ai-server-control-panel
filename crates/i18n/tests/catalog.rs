//! Catalog consistency and lookup behavior tests.

use i18n::{
    Catalog, Locale, MessageKey, ParseErrorKind, TemplateError, is_valid_key, parse_catalog,
    placeholders,
};
use std::collections::BTreeSet;
use std::error::Error;

type TestResult = Result<(), Box<dyn Error>>;

const EN: &str = include_str!("../locales/en.txt");
const ID: &str = include_str!("../locales/id.txt");

fn keys(source: &str) -> Result<BTreeSet<String>, Box<dyn Error>> {
    Ok(parse_catalog(source)?.into_iter().map(|e| e.key).collect())
}

#[test]
fn key_sets_are_identical() -> TestResult {
    let (en, id) = (keys(EN)?, keys(ID)?);
    let only_en: Vec<_> = en.difference(&id).collect();
    let only_id: Vec<_> = id.difference(&en).collect();
    assert!(only_en.is_empty(), "keys only in en: {only_en:?}");
    assert!(only_id.is_empty(), "keys only in id: {only_id:?}");
    assert!(!en.is_empty());
    Ok(())
}

#[test]
fn placeholders_match_per_key() -> TestResult {
    let catalog = Catalog::builtin()?;
    for key in catalog.keys(Locale::En) {
        let (Some(en), Some(id)) = (catalog.raw(Locale::En, key), catalog.raw(Locale::Id, key))
        else {
            return Err(format!("key {key} missing in id").into());
        };
        assert_eq!(
            placeholders(en)?,
            placeholders(id)?,
            "placeholders differ for {key}"
        );
    }
    Ok(())
}

#[test]
fn no_empty_or_duplicate_and_keys_well_formed() -> TestResult {
    for source in [EN, ID] {
        for e in parse_catalog(source)? {
            assert!(is_valid_key(&e.key), "bad key {}", e.key);
            assert!(!e.value.trim().is_empty(), "empty value for {}", e.key);
        }
    }
    // Parser rejects duplicates, empties and malformed lines, with line numbers.
    let dup = parse_catalog("# c\na.b.c = x\n\na.b.c = y\n").err();
    let dup = dup.ok_or("duplicate accepted")?;
    assert_eq!(dup.line, 4);
    assert_eq!(dup.kind, ParseErrorKind::DuplicateKey { first_line: 2 });
    let empty = parse_catalog("a.b.c =\n")
        .err()
        .ok_or("empty value accepted")?;
    assert_eq!((empty.line, empty.kind), (1, ParseErrorKind::EmptyValue));
    let nokey = parse_catalog(" = x\n").err().ok_or("empty key accepted")?;
    assert_eq!(nokey.kind, ParseErrorKind::EmptyKey);
    let nosep = parse_catalog("a.b.c x\n")
        .err()
        .ok_or("missing = accepted")?;
    assert_eq!(nosep.kind, ParseErrorKind::MissingSeparator);
    let upper = parse_catalog("A.b.c = x\n")
        .err()
        .ok_or("uppercase key accepted")?;
    assert!(matches!(upper.kind, ParseErrorKind::InvalidKey(_)));
    let two = parse_catalog("a.b = x\n")
        .err()
        .ok_or("two-segment key accepted")?;
    assert!(matches!(two.kind, ParseErrorKind::InvalidKey(_)));
    let open = parse_catalog("a.b.c = hi {name\n")
        .err()
        .ok_or("unclosed accepted")?;
    assert_eq!(
        open.kind,
        ParseErrorKind::Template(TemplateError::UnclosedBrace)
    );
    assert!(open.to_string().starts_with("line 1:"));
    Ok(())
}

#[test]
fn fallback_and_missing() -> TestResult {
    let catalog = Catalog::from_sources(
        "a.b.c = English {n}\nonly.in.en = Only English\n",
        "a.b.c = Indonesia {n}\n",
    )?;
    let k = MessageKey::new("a.b.c");
    let t = catalog.get(Locale::Id, k, &[("n", "1")]);
    assert_eq!(
        (t.text.as_str(), t.fell_back, t.missing),
        ("Indonesia 1", false, false)
    );

    let t = catalog.get(Locale::Id, MessageKey::new("only.in.en"), &[]);
    assert_eq!(
        (t.text.as_str(), t.fell_back, t.missing),
        ("Only English", true, false)
    );

    let t = catalog.get(Locale::Id, MessageKey::new("no.such.key"), &[]);
    assert_eq!(
        (t.text.as_str(), t.fell_back, t.missing),
        ("no.such.key", false, true)
    );
    let t = catalog.get(Locale::En, MessageKey::new("no.such.key"), &[]);
    assert!(t.missing);
    Ok(())
}

#[test]
fn substitution_rules() -> TestResult {
    let catalog = Catalog::from_sources("a.b.c = {x} and {y} and {x} {{lit}}\n", "a.b.c = {x}\n")?;
    let k = MessageKey::new("a.b.c");
    let t = catalog.get(Locale::En, k, &[("x", "{y}")]);
    // Values are not re-scanned; unresolved placeholders stay and are reported once.
    assert_eq!(t.text, "{y} and {y} and {y} {lit}");
    assert_eq!(t.unresolved, vec!["y".to_owned()]);
    let t = catalog.get(Locale::En, k, &[("x", "1"), ("y", "2")]);
    assert_eq!(t.text, "1 and 2 and 1 {lit}");
    assert!(t.unresolved.is_empty());
    Ok(())
}

#[test]
fn locale_parsing() {
    assert_eq!(Locale::parse("id"), Some(Locale::Id));
    assert_eq!(Locale::parse("id-ID"), Some(Locale::Id));
    assert_eq!(Locale::parse("ID_id"), Some(Locale::Id));
    assert_eq!(Locale::parse("en-US"), Some(Locale::En));
    assert_eq!(Locale::parse(" en "), Some(Locale::En));
    assert_eq!(Locale::parse("fr-FR"), None);
    assert_eq!(Locale::parse(""), None);
    assert_eq!(Locale::parse_or_default("de"), Locale::En);
    assert_eq!(Locale::default(), Locale::En);
    assert_eq!(Locale::Id.tag(), "id");
}

#[test]
fn builtin_loads_and_renders() -> TestResult {
    let catalog = Catalog::builtin()?;
    let t = catalog.get(
        Locale::En,
        MessageKey::new("error.node.unreachable"),
        &[("name", "n1")],
    );
    assert_eq!(t.text, "Node n1 is unreachable.");
    Ok(())
}
