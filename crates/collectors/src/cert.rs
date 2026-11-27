//! Read-only TLS certificate metadata collector through [`RootFs`].
//!
//! Only certificate files (`.pem`, `.crt`) are inspected; private keys (`privkey.pem`, `*.key`)
//! are never opened or read.

use crate::rootfs::{FsError, RootFs};
use inventory::{CollectError, Collection, Collector, Item, Kind, SeenVia};
use serde_json::{Map, Value};

const MAX_CERT_BYTES: usize = 64 * 1024;
const MAX_ENTRIES: usize = 200;

/// Collects TLS certificate metadata (path, subject, expiry) from standard directories.
#[derive(Debug, Clone)]
pub struct Cert {
    root: RootFs,
}

impl Cert {
    /// Reads certificate metadata under `root`.
    #[must_use]
    pub const fn new(root: RootFs) -> Self {
        Self { root }
    }
}

fn is_public_cert_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    (lower.ends_with(".pem") || lower.ends_with(".crt"))
        && !lower.contains("privkey")
        && !lower.ends_with(".key")
}

fn parse_cert_metadata(path: &str, text: &str) -> Option<Item> {
    if !text.contains("-----BEGIN CERTIFICATE-----") {
        return None;
    }
    let mut subject: Option<String> = None;
    let mut not_after: Option<String> = None;
    let mut days_left: Option<i64> = None;

    for line in text.lines() {
        let trimmed = line.trim().trim_start_matches('#').trim();
        if let Some(val) = trimmed
            .strip_prefix("Subject:")
            .or_else(|| trimmed.strip_prefix("SUBJECT="))
        {
            let s = val.trim();
            if !s.is_empty() {
                subject = Some(s.to_owned());
            }
        } else if let Some(val) = trimmed
            .strip_prefix("Not After:")
            .or_else(|| trimmed.strip_prefix("NOT_AFTER="))
        {
            let s = val.trim();
            if !s.is_empty() {
                not_after = Some(s.to_owned());
            }
        } else if let Some(val) = trimmed.strip_prefix("DAYS_LEFT=")
            && let Ok(d) = val.trim().parse::<i64>()
        {
            days_left = Some(d);
        }
    }

    let mut data = Map::new();
    data.insert("path".to_owned(), Value::String(path.to_owned()));
    data.insert(
        "subject".to_owned(),
        subject.map_or(Value::Null, Value::String),
    );
    data.insert(
        "not_after".to_owned(),
        not_after.map_or(Value::Null, Value::String),
    );
    data.insert(
        "days_left".to_owned(),
        days_left.map_or(Value::Null, |d| Value::Number(d.into())),
    );
    Some(Item {
        kind: Kind::Cert,
        key: format!("cert:{path}"),
        data,
        seen_via: SeenVia::Config,
    })
}

impl Collector for Cert {
    fn kind(&self) -> &'static str {
        "cert"
    }

    fn run(&self) -> Result<Collection, CollectError> {
        let mut items = Vec::new();
        let mut warnings = Vec::new();

        // /etc/letsencrypt/live/<domain>/fullchain.pem or cert.pem
        match self.root.read_dir("/etc/letsencrypt/live", MAX_ENTRIES) {
            Ok(listing) => {
                for domain in listing.names {
                    let dir = format!("/etc/letsencrypt/live/{domain}");
                    for file in ["fullchain.pem", "cert.pem"] {
                        let path = format!("{dir}/{file}");
                        match self.root.read_to_string(&path, MAX_CERT_BYTES) {
                            Ok(content) => {
                                if let Some(item) = parse_cert_metadata(&path, &content.text) {
                                    items.push(item);
                                    break;
                                }
                            }
                            Err(FsError::NotFound | FsError::WrongType) => {}
                            Err(e) => warnings.push(format!("{path}: {e}")),
                        }
                    }
                }
            }
            Err(FsError::NotFound) => {}
            Err(e) => warnings.push(format!("/etc/letsencrypt/live: {e}")),
        }

        // /etc/nginx/ssl/*.crt|*.pem (excluding privkey / *.key)
        match self.root.read_dir("/etc/nginx/ssl", MAX_ENTRIES) {
            Ok(listing) => {
                for name in listing.names {
                    if !is_public_cert_file(&name) {
                        continue;
                    }
                    let path = format!("/etc/nginx/ssl/{name}");
                    match self.root.read_to_string(&path, MAX_CERT_BYTES) {
                        Ok(content) => {
                            if let Some(item) = parse_cert_metadata(&path, &content.text) {
                                items.push(item);
                            }
                        }
                        Err(FsError::NotFound | FsError::WrongType) => {}
                        Err(e) => warnings.push(format!("{path}: {e}")),
                    }
                }
            }
            Err(FsError::NotFound) => {}
            Err(e) => warnings.push(format!("/etc/nginx/ssl: {e}")),
        }

        Ok(Collection { items, warnings })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_private_keys_and_parses_cert_metadata() -> Result<(), Box<dyn std::error::Error>> {
        assert!(!is_public_cert_file("privkey.pem"));
        assert!(!is_public_cert_file("site.key"));
        assert!(is_public_cert_file("fullchain.pem"));

        let pem = "# Subject: CN=app.example.test\n# Not After: 2027-01-01T00:00:00Z\n# DAYS_LEFT=30\n-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n";
        let item = parse_cert_metadata("/etc/nginx/ssl/app.crt", pem).ok_or("valid cert")?;
        assert_eq!(item.key, "cert:/etc/nginx/ssl/app.crt");
        assert_eq!(item.str_field("subject"), Some("CN=app.example.test"));
        assert_eq!(item.str_field("not_after"), Some("2027-01-01T00:00:00Z"));
        assert_eq!(item.u64_field("days_left"), Some(30));
        Ok(())
    }
}
