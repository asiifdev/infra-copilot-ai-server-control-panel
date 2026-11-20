//! Where the command line reads its environment from, so tests can supply one without touching
//! the process environment (which is shared between threads).

use std::collections::BTreeMap;
use std::sync::Arc;
use transport::Runner;

/// Environment variables the command line understands, and (for tests) the runner that stands in
/// for starting processes.
pub struct Env {
    fixed: Option<BTreeMap<String, String>>,
    runner: Option<Arc<dyn Runner>>,
}

impl Env {
    /// The real process environment.
    #[must_use]
    pub fn process() -> Self {
        Self {
            fixed: None,
            runner: None,
        }
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
            runner: None,
        }
    }

    /// Replaces the process runner of `transport check --run` (tests pass a `FakeRunner`, so
    /// nothing is ever started). Without it the real `ProcessRunner` is used.
    #[must_use]
    pub fn with_runner(mut self, runner: Arc<dyn Runner>) -> Self {
        self.runner = Some(runner);
        self
    }

    pub(crate) fn runner(&self) -> Option<Arc<dyn Runner>> {
        self.runner.clone()
    }

    pub(crate) fn get(&self, name: &str) -> Option<String> {
        match &self.fixed {
            Some(map) => map.get(name).cloned(),
            None => std::env::var(name).ok(),
        }
    }
}
