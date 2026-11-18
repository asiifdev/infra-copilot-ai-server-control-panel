//! Supported locales and BCP 47 tag parsing.

/// A supported display locale. English is the default and the fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Locale {
    /// English (default, fallback for every other locale).
    #[default]
    En,
    /// Indonesian.
    Id,
}

impl Locale {
    /// Every supported locale.
    pub const ALL: [Locale; 2] = [Locale::En, Locale::Id];

    /// Parses a BCP 47 tag such as `id`, `id-ID`, `en-US` or `en_GB`.
    ///
    /// Only the primary language subtag is considered; matching is
    /// case-insensitive. Returns `None` for unsupported languages.
    pub fn parse(tag: &str) -> Option<Locale> {
        let primary = tag.trim().split(['-', '_']).next()?;
        if primary.eq_ignore_ascii_case("en") {
            Some(Locale::En)
        } else if primary.eq_ignore_ascii_case("id") {
            Some(Locale::Id)
        } else {
            None
        }
    }

    /// Like [`Locale::parse`] but falls back to the default locale.
    pub fn parse_or_default(tag: &str) -> Locale {
        Locale::parse(tag).unwrap_or_default()
    }

    /// The canonical short tag (`en`, `id`), as stored in `users.locale`.
    pub const fn tag(self) -> &'static str {
        match self {
            Locale::En => "en",
            Locale::Id => "id",
        }
    }
}
