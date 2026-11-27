//! Safe `.test` and production site vhost validation and config generation.

use crate::ManagedPath;
use crate::exec::validate_managed_path;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

/// Longest `.test` domain accepted.
pub const MAX_TEST_DOMAIN_LEN: usize = 120;
/// Longest production or `.test` site domain accepted (RFC 1035).
pub const MAX_SITE_DOMAIN_LEN: usize = 253;

/// Reserved local/internal TLDs that cannot be used for public ACME TLS certificates.
const NON_PUBLIC_TLDS: [&str; 5] = [".test", ".localhost", ".local", ".invalid", ".internal"];

/// A validated domain name for a site (`.test` or production).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteDomain(String);

impl SiteDomain {
    /// Validates a `.test` or production domain name (lowercase normalized, no scheme/path/port).
    ///
    /// # Errors
    /// Returns `"invalid_domain"` if the domain is empty, too long, lacks a dot, or has malformed
    /// labels.
    pub fn new(raw: &str) -> Result<Self, &'static str> {
        let domain = raw.trim().to_ascii_lowercase();
        if domain.is_empty()
            || domain.len() > MAX_SITE_DOMAIN_LEN
            || !domain.contains('.')
            || domain.starts_with('.')
            || domain.ends_with('.')
            || domain.contains("..")
        {
            return Err("invalid_domain");
        }
        for label in domain.split('.') {
            if label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            {
                return Err("invalid_domain");
            }
        }
        Ok(Self(domain))
    }

    /// Validates a public production domain (rejects `.test`, `.localhost`, `.local`, `.invalid`,
    /// `.internal`).
    ///
    /// # Errors
    /// Returns `"invalid_domain"` if invalid or reserved for local use.
    pub fn new_production(raw: &str) -> Result<Self, &'static str> {
        let domain = Self::new(raw)?;
        if NON_PUBLIC_TLDS.iter().any(|tld| domain.0.ends_with(tld)) {
            return Err("invalid_domain");
        }
        Ok(domain)
    }

    /// Whether this domain is a local `.test` development domain.
    #[must_use]
    pub fn is_test_domain(&self) -> bool {
        self.0.ends_with(".test")
    }

    /// The normalized lowercase domain string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Allowlisted PHP-FPM runtime versions for M5 sites.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhpVersion {
    /// PHP 8.1
    V8_1,
    /// PHP 8.2
    V8_2,
    /// PHP 8.3
    V8_3,
    /// PHP 8.4
    V8_4,
}

impl PhpVersion {
    /// Canonical version string (e.g. `"8.3"`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::V8_1 => "8.1",
            Self::V8_2 => "8.2",
            Self::V8_3 => "8.3",
            Self::V8_4 => "8.4",
        }
    }

    /// Parses a PHP version from the strict allowlist (`8.1`, `8.2`, `8.3`, `8.4`).
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "8.1" => Some(Self::V8_1),
            "8.2" => Some(Self::V8_2),
            "8.3" => Some(Self::V8_3),
            "8.4" => Some(Self::V8_4),
            _ => None,
        }
    }
}

/// Renders an nginx server block for either a `.test` local site or a production site.
#[must_use]
pub fn render_site_vhost_config(
    domain: &SiteDomain,
    upstream_port: u16,
    php_version: Option<PhpVersion>,
    tls_enabled: bool,
    root_dir: &ManagedPath,
) -> String {
    let mut out = String::new();
    let d = domain.as_str();
    let root = root_dir.as_str();
    let _ = writeln!(out, "# Managed by Nodivara site manager");
    let _ = writeln!(out, "server {{");
    if domain.is_test_domain() {
        let _ = writeln!(out, "    listen 127.0.0.1:8080;");
        if tls_enabled {
            let _ = writeln!(out, "    listen 127.0.0.1:8443 ssl;");
        }
    } else {
        let _ = writeln!(out, "    listen 80;");
        if tls_enabled {
            let _ = writeln!(out, "    listen 443 ssl http2;");
            let _ = writeln!(
                out,
                "    ssl_certificate /etc/letsencrypt/live/{d}/fullchain.pem;"
            );
            let _ = writeln!(
                out,
                "    ssl_certificate_key /etc/letsencrypt/live/{d}/privkey.pem;"
            );
        }
    }
    let _ = writeln!(out, "    server_name {d};");
    let _ = writeln!(out, "    root {root};");
    let _ = writeln!(out);
    if let Some(php) = php_version {
        let ver = php.as_str();
        let _ = writeln!(out, "    location ~ \\.php$ {{");
        let _ = writeln!(out, "        include fastcgi_params;");
        let _ = writeln!(out, "        fastcgi_pass unix:/run/php/php{ver}-fpm.sock;");
        let _ = writeln!(out, "    }}");
        let _ = writeln!(out);
    }
    let _ = writeln!(out, "    location / {{");
    let _ = writeln!(out, "        proxy_pass http://127.0.0.1:{upstream_port};");
    let _ = writeln!(out, "        proxy_set_header Host $host;");
    let _ = writeln!(
        out,
        "        proxy_set_header X-Forwarded-For $remote_addr;"
    );
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "}}");
    out
}

/// Validates a local development domain strictly ending in `.test`.
///
/// # Errors
/// Returns `"invalid_test_domain"` if the domain is empty, too long, does not end with `.test`,
/// contains invalid characters, or has empty/malformed labels.
pub fn validate_test_domain(raw: &str) -> Result<String, &'static str> {
    let domain = raw.trim().to_ascii_lowercase();
    if domain.len() < 6 || domain.len() > MAX_TEST_DOMAIN_LEN {
        return Err("invalid_test_domain");
    }
    if !domain.ends_with(".test") {
        return Err("invalid_test_domain");
    }
    let prefix = domain.strip_suffix(".test").ok_or("invalid_test_domain")?;
    if prefix.is_empty()
        || prefix.starts_with('.')
        || prefix.ends_with('.')
        || prefix.contains("..")
    {
        return Err("invalid_test_domain");
    }
    for label in prefix.split('.') {
        if label.is_empty()
            || label.starts_with('-')
            || label.ends_with('-')
            || !label
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err("invalid_test_domain");
        }
    }
    Ok(domain)
}

/// Renders a safe, read-only/local-loopback nginx server block for a `.test` domain.
#[must_use]
pub fn render_test_vhost_config(domain: &str, upstream_port: u16) -> String {
    format!(
        "# Managed by Nodivara local .test site manager\n\
server {{\n\
    listen 127.0.0.1:8080;\n\
    server_name {domain};\n\n\
    location / {{\n\
        proxy_pass http://127.0.0.1:{upstream_port};\n\
        proxy_set_header Host $host;\n\
        proxy_set_header X-Forwarded-For $remote_addr;\n\
    }}\n\
}}\n"
    )
}

/// Writes a `.test` vhost config safely inside `sites_dir` (never outside, no symlinks).
///
/// # Errors
/// Returns `"invalid_test_domain"`, `"invalid_upstream_port"`, `"path_not_allowed"`,
/// `"symlink_not_allowed"`, or `"io"`.
pub fn write_managed_test_site(
    sites_dir: &Path,
    domain: &str,
    upstream_port: u16,
) -> Result<PathBuf, &'static str> {
    let domain = validate_test_domain(domain)?;
    if upstream_port == 0 {
        return Err("invalid_upstream_port");
    }
    fs::create_dir_all(sites_dir).map_err(|_| "io")?;
    let target = sites_dir.join(format!("{domain}.conf"));
    let target_str = target.to_str().ok_or("path_not_allowed")?;
    let validated = validate_managed_path(target_str, &[sites_dir.to_path_buf()])?;
    let content = render_test_vhost_config(&domain, upstream_port);
    fs::write(&validated, content.as_bytes()).map_err(|_| "io")?;
    Ok(validated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDir;

    #[test]
    fn test_domain_validation_is_strict() {
        assert_eq!(validate_test_domain("App.Test"), Ok("app.test".to_owned()));
        assert_eq!(
            validate_test_domain("api.my-app.test"),
            Ok("api.my-app.test".to_owned())
        );
        assert!(validate_test_domain("example.com").is_err());
        assert!(validate_test_domain(".test").is_err());
        assert!(validate_test_domain("-bad.test").is_err());
        assert!(validate_test_domain("a..b.test").is_err());
        assert!(validate_test_domain("bad;rm.test").is_err());
    }

    #[test]
    fn writes_vhost_into_managed_dir_only() -> Result<(), Box<dyn std::error::Error>> {
        let dir = TestDir::new("sites")?;
        let path = write_managed_test_site(dir.path(), "billing.test", 3000)
            .map_err(|e| format!("write: {e}"))?;
        let text = fs::read_to_string(path)?;
        assert!(text.contains("server_name billing.test;"));
        assert!(text.contains("proxy_pass http://127.0.0.1:3000;"));
        Ok(())
    }
}
