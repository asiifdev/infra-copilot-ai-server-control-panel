//! Safe `.test` local site vhost validation and config generation (never touches production
//! nginx directories).

use crate::exec::validate_managed_path;
use std::fs;
use std::path::{Path, PathBuf};

/// Longest `.test` domain accepted.
pub const MAX_TEST_DOMAIN_LEN: usize = 120;

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
