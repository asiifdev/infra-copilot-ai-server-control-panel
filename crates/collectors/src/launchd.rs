//! Read-only macOS `launchd` `.plist` collector through [`RootFs`].
//!
//! Reads `/Library/LaunchDaemons` and `/Library/LaunchAgents` plist files via `RootFs` (never
//! mutating anything). Cross-platform safe: works on fixture trees on any OS, and skips
//! silently when the directories do not exist.

use crate::rootfs::{FsError, RootFs};
use core_domain::redact::redact;
use inventory::{CollectError, Collection, Collector, Item, Kind, SeenVia};
use serde_json::{Map, Value};

const MAX_PLIST_BYTES: usize = 64 * 1024;
const MAX_PLISTS: usize = 200;

/// Collects `launchd` service definitions as `Kind::Process` (`launchd:<label>`).
#[derive(Debug, Clone)]
pub struct Launchd {
    root: RootFs,
}

impl Launchd {
    /// Reads `launchd` plist files under `root`.
    #[must_use]
    pub const fn new(root: RootFs) -> Self {
        Self { root }
    }
}

fn extract_tag_value<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = text.find(&open)? + open.len();
    let end = text[start..].find(&close)? + start;
    Some(text[start..end].trim())
}

fn parse_plist(text: &str) -> Option<Item> {
    let label_pos = text.find("<key>Label</key>")?;
    let label = extract_tag_value(&text[label_pos..], "string")?;
    if label.is_empty() || label.len() > 200 {
        return None;
    }
    let cmd = text
        .find("<key>ProgramArguments</key>")
        .and_then(|pos| extract_tag_value(&text[pos..], "string"))
        .or_else(|| {
            text.find("<key>Program</key>")
                .and_then(|pos| extract_tag_value(&text[pos..], "string"))
        })
        .map(|s| redact(s).text);

    let mut data = Map::new();
    data.insert("cmd".to_owned(), cmd.map_or(Value::Null, Value::String));
    data.insert("cwd".to_owned(), Value::Null);
    data.insert("mode".to_owned(), Value::String("unknown".to_owned()));
    data.insert("supervised".to_owned(), Value::Bool(true));
    data.insert("env_refs".to_owned(), Value::Array(Vec::new()));

    Some(Item {
        kind: Kind::Process,
        key: format!("launchd:{label}"),
        data,
        seen_via: SeenVia::Config,
    })
}

impl Collector for Launchd {
    fn kind(&self) -> &'static str {
        "launchd"
    }

    fn run(&self) -> Result<Collection, CollectError> {
        let mut items = Vec::new();
        let mut warnings = Vec::new();

        for dir in ["/Library/LaunchDaemons", "/Library/LaunchAgents"] {
            match self.root.read_dir(dir, MAX_PLISTS) {
                Ok(listing) => {
                    for name in listing.names {
                        if !name.ends_with(".plist") {
                            continue;
                        }
                        let path = format!("{dir}/{name}");
                        match self.root.read_to_string(&path, MAX_PLIST_BYTES) {
                            Ok(file) => {
                                if let Some(item) = parse_plist(&file.text) {
                                    items.push(item);
                                }
                            }
                            Err(FsError::NotFound | FsError::WrongType) => {}
                            Err(e) => warnings.push(format!("{path}: {e}")),
                        }
                    }
                }
                Err(FsError::NotFound) => {}
                Err(e) => warnings.push(format!("{dir}: {e}")),
            }
        }

        Ok(Collection { items, warnings })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_launchd_plist_safely() -> Result<(), Box<dyn std::error::Error>> {
        let plist = r#"
<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.example.worker</string>
    <key>ProgramArguments</key>
    <array>
        <string>/usr/local/bin/worker</string>
    </array>
</dict>
</plist>
"#;
        let item = parse_plist(plist).ok_or("valid plist")?;
        assert_eq!(item.key, "launchd:com.example.worker");
        assert_eq!(item.str_field("cmd"), Some("/usr/local/bin/worker"));
        assert_eq!(item.bool_field("supervised"), Some(true));
        Ok(())
    }
}
