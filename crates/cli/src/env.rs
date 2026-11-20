//! Where the command line reads its environment from, so tests can supply one without touching
//! the process environment (which is shared between threads).

use std::collections::BTreeMap;

/// Environment variables the command line understands.
pub struct Env {
    fixed: Option<BTreeMap<String, String>>,
}

impl Env {
    /// The real process environment.
    #[must_use]
    pub fn process() -> Self {
        Self { fixed: None }
    }

    /// A fixed environment (tests): only these variables exist.
    #[must_use]
    pub fn fixed(pairs: &[(&str, &str)]) -> Self {
        Self {
            fixed: Some(
                pairs
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                    .collect(),
            ),
        }
    }

    pub(crate) fn get(&self, name: &str) -> Option<String> {
        match &self.fixed {
            Some(map) => map.get(name).cloned(),
            None => std::env::var(name).ok(),
        }
    }
}
