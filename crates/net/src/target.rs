//! Minimal URL handling: parse an `http(s)` URL into a connect target and resolve redirects.

use crate::error::NetError;
use http::Uri;

const MAX_URL_LEN: usize = 2048;

/// A validated request target. Holds no credentials by construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Target {
    pub https: bool,
    /// Host without IPv6 brackets.
    pub host: String,
    pub port: u16,
    /// Path and query, always starting with `/`.
    pub path: String,
}

impl Target {
    pub fn parse(url: &str) -> Result<Target, NetError> {
        if url.len() > MAX_URL_LEN || url.bytes().any(|b| b.is_ascii_control() || b == b' ') {
            return Err(NetError::InvalidUrl);
        }
        // Fragments are never sent.
        let url = url.split_once('#').map_or(url, |(head, _)| head);
        let uri: Uri = url.parse().map_err(|_| NetError::InvalidUrl)?;
        let https = match uri.scheme_str() {
            Some(s) if s.eq_ignore_ascii_case("https") => true,
            Some(s) if s.eq_ignore_ascii_case("http") => false,
            Some(_) => return Err(NetError::SchemeNotAllowed),
            None => return Err(NetError::InvalidUrl),
        };
        let authority = uri.authority().ok_or(NetError::InvalidUrl)?;
        if authority.as_str().contains('@') {
            return Err(NetError::UserInfoNotAllowed);
        }
        let raw_host = authority.host();
        let host = raw_host
            .strip_prefix('[')
            .and_then(|h| h.strip_suffix(']'))
            .unwrap_or(raw_host);
        if host.is_empty() {
            return Err(NetError::InvalidUrl);
        }
        let port = authority.port_u16().unwrap_or(if https { 443 } else { 80 });
        let mut path = String::from(uri.path());
        if !path.starts_with('/') {
            path.insert(0, '/');
        }
        if let Some(query) = uri.query() {
            path.push('?');
            path.push_str(query);
        }
        Ok(Target {
            https,
            host: host.to_owned(),
            port,
            path,
        })
    }

    pub fn scheme(&self) -> &'static str {
        if self.https { "https" } else { "http" }
    }

    /// `host[:port]` as used in the `Host` header and in URLs (port omitted when default).
    pub fn authority(&self) -> String {
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        let default = if self.https { 443 } else { 80 };
        if self.port == default {
            host
        } else {
            format!("{host}:{}", self.port)
        }
    }

    pub fn url(&self) -> String {
        format!("{}://{}{}", self.scheme(), self.authority(), self.path)
    }

    /// Resolve a `Location` header value against this target.
    pub fn join(&self, location: &str) -> Result<Target, NetError> {
        let location = location.trim();
        if location.is_empty() {
            return Err(NetError::InvalidRedirect);
        }
        let absolute = if location.starts_with("//") {
            format!("{}:{location}", self.scheme())
        } else if location.starts_with('/') {
            format!("{}://{}{location}", self.scheme(), self.authority())
        } else if location.starts_with('?') {
            let base = self
                .path
                .split_once('?')
                .map_or(self.path.as_str(), |(p, _)| p);
            format!("{}://{}{base}{location}", self.scheme(), self.authority())
        } else if location.contains("://") {
            // Absolute URL (any scheme; `parse` rejects the ones we do not allow).
            location.to_owned()
        } else {
            let base = self
                .path
                .split_once('?')
                .map_or(self.path.as_str(), |(p, _)| p);
            let dir = base.rsplit_once('/').map_or("", |(d, _)| d);
            format!("{}://{}{dir}/{location}", self.scheme(), self.authority())
        };
        Target::parse(&absolute)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_normalises() {
        let t = Target::parse("https://Example.org/a/b?x=1#frag")
            .unwrap_or_else(|_| unreachable_target());
        assert_eq!(
            (t.https, t.host.as_str(), t.port, t.path.as_str()),
            (true, "Example.org", 443, "/a/b?x=1")
        );
        let t = Target::parse("http://[::1]:8080").unwrap_or_else(|_| unreachable_target());
        assert_eq!(
            (t.host.as_str(), t.port, t.path.as_str()),
            ("::1", 8080, "/")
        );
        assert_eq!(t.authority(), "[::1]:8080");
        assert_eq!(t.url(), "http://[::1]:8080/");
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(
            Target::parse("https://user:pw@example.org/"),
            Err(NetError::UserInfoNotAllowed)
        );
        assert_eq!(
            Target::parse("https://user@example.org/"),
            Err(NetError::UserInfoNotAllowed)
        );
        assert_eq!(
            Target::parse("ftp://example.org/"),
            Err(NetError::SchemeNotAllowed)
        );
        assert_eq!(Target::parse("example.org/path"), Err(NetError::InvalidUrl));
        assert_eq!(Target::parse("/relative"), Err(NetError::InvalidUrl));
        assert_eq!(Target::parse("https://"), Err(NetError::InvalidUrl));
        assert_eq!(
            Target::parse("https://exa mple.org/"),
            Err(NetError::InvalidUrl)
        );
        assert_eq!(
            Target::parse(&format!("https://e.org/{}", "a".repeat(3000))),
            Err(NetError::InvalidUrl)
        );
    }

    #[test]
    fn userinfo_never_in_error_text() {
        let err = Target::parse("https://secretuser:secretpw@example.org/?token=abc").err();
        let text = err.map(|e| e.to_string()).unwrap_or_default();
        assert!(!text.contains("secret") && !text.contains("token"));
    }

    #[test]
    fn joins_locations() {
        let base = Target::parse("https://a.example/dir/page?q=1")
            .unwrap_or_else(|_| unreachable_target());
        let cases = [
            ("/x", "https://a.example/x"),
            ("y", "https://a.example/dir/y"),
            ("?z=2", "https://a.example/dir/page?z=2"),
            ("//b.example/p", "https://b.example/p"),
            ("http://c.example:81/", "http://c.example:81/"),
            ("HTTPS://D.example/", "https://D.example/"),
        ];
        for (loc, want) in cases {
            assert_eq!(
                base.join(loc).map(|t| t.url()),
                Ok(want.to_owned()),
                "{loc}"
            );
        }
        assert_eq!(base.join(""), Err(NetError::InvalidRedirect));
        assert_eq!(base.join("ftp://x/"), Err(NetError::SchemeNotAllowed));
        assert_eq!(
            base.join("http://u:p@x/"),
            Err(NetError::UserInfoNotAllowed)
        );
    }

    fn unreachable_target() -> Target {
        Target {
            https: false,
            host: String::new(),
            port: 0,
            path: String::new(),
        }
    }
}
