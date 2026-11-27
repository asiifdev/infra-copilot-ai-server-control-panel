//! Read-only virtual host collector (`nginx` and `caddy`) through [`RootFs`].

use crate::rootfs::{FsError, RootFs};
use inventory::{CollectError, Collection, Collector, Item, Kind, SeenVia};
use serde_json::{Map, Value};

const MAX_CONFIG_BYTES: usize = 256 * 1024;
const MAX_FILES_PER_DIR: usize = 200;

/// Collects `nginx` and `caddy` virtual host definitions from configuration files.
#[derive(Debug, Clone)]
pub struct Vhost {
    root: RootFs,
}

impl Vhost {
    /// Reads virtual host configurations under `root`.
    #[must_use]
    pub const fn new(root: RootFs) -> Self {
        Self { root }
    }
}

impl Collector for Vhost {
    fn kind(&self) -> &'static str {
        "vhost"
    }

    fn run(&self) -> Result<Collection, CollectError> {
        let mut items = Vec::new();
        let mut warnings = Vec::new();

        for dir in ["/etc/nginx/sites-enabled", "/etc/nginx/conf.d"] {
            match self.root.read_dir(dir, MAX_FILES_PER_DIR) {
                Ok(listing) => {
                    for name in listing.names {
                        if name.starts_with('.') {
                            continue;
                        }
                        let path = format!("{dir}/{name}");
                        match self.root.read_to_string(&path, MAX_CONFIG_BYTES) {
                            Ok(file) => items.extend(parse_nginx(&file.text)),
                            Err(FsError::NotFound | FsError::WrongType) => {}
                            Err(e) => warnings.push(format!("{path}: {e}")),
                        }
                    }
                }
                Err(FsError::NotFound) => {}
                Err(e) => warnings.push(format!("{dir}: {e}")),
            }
        }

        match self
            .root
            .read_to_string("/etc/caddy/Caddyfile", MAX_CONFIG_BYTES)
        {
            Ok(file) => items.extend(parse_caddy(&file.text)),
            Err(FsError::NotFound) => {}
            Err(e) => warnings.push(format!("/etc/caddy/Caddyfile: {e}")),
        }

        Ok(Collection { items, warnings })
    }
}

fn parse_nginx(text: &str) -> Vec<Item> {
    let mut items = Vec::new();
    let mut in_server = false;
    let mut depth: usize = 0;
    let mut server_depth: usize = 0;
    let mut server_names: Vec<String> = Vec::new();
    let mut port: u64 = 80;
    let mut tls = false;
    let mut upstream: Option<String> = None;

    for raw_line in text.lines() {
        let line = raw_line.split('#').next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }
        if !in_server && line.starts_with("server") && line.contains('{') {
            in_server = true;
            server_depth = depth;
            server_names.clear();
            port = 80;
            tls = false;
            upstream = None;
        }
        depth = depth
            .saturating_add(line.matches('{').count())
            .saturating_sub(line.matches('}').count());

        if in_server {
            let stmt = line.trim_end_matches(';').trim();
            if let Some(rest) = stmt.strip_prefix("server_name ") {
                for name in rest.split_whitespace() {
                    if name != "_" && !name.is_empty() {
                        server_names.push(name.to_owned());
                    }
                }
            } else if let Some(rest) = stmt.strip_prefix("listen ") {
                for tok in rest.split_whitespace() {
                    if tok == "ssl" {
                        tls = true;
                    }
                    let port_part = tok.rsplit(':').next().unwrap_or(tok);
                    if let Ok(p) = port_part.parse::<u64>() {
                        port = p;
                        if p == 443 {
                            tls = true;
                        }
                    }
                }
            } else if stmt.starts_with("ssl_certificate ") {
                tls = true;
            } else if let Some(rest) = stmt.strip_prefix("proxy_pass ") {
                let target = rest
                    .trim()
                    .trim_start_matches("http://")
                    .trim_start_matches("https://")
                    .split('/')
                    .next()
                    .unwrap_or_default();
                if !target.is_empty() {
                    upstream = Some(target.to_owned());
                }
            }

            if depth <= server_depth && line.contains('}') {
                in_server = false;
                for name in server_names.drain(..) {
                    let mut data = Map::new();
                    data.insert("server_name".to_owned(), Value::String(name.clone()));
                    data.insert("port".to_owned(), Value::Number(port.into()));
                    data.insert("tls".to_owned(), Value::Bool(tls));
                    data.insert(
                        "upstream".to_owned(),
                        upstream.clone().map_or(Value::Null, Value::String),
                    );
                    data.insert("engine".to_owned(), Value::String("nginx".to_owned()));
                    items.push(Item {
                        kind: Kind::Vhost,
                        key: format!("nginx:{name}:{port}"),
                        data,
                        seen_via: SeenVia::Config,
                    });
                }
            }
        }
    }
    items
}

fn parse_caddy(text: &str) -> Vec<Item> {
    let mut items = Vec::new();
    let mut current_host: Option<String> = None;
    let mut depth: usize = 0;
    let mut tls = true;
    let mut upstream: Option<String> = None;

    for raw_line in text.lines() {
        let line = raw_line.split('#').next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }
        if current_host.is_none() && line.ends_with('{') {
            let host = line.trim_end_matches('{').trim();
            if !host.is_empty() && !host.starts_with(':') {
                let clean = host
                    .trim_start_matches("https://")
                    .trim_start_matches("http://")
                    .to_owned();
                tls = !host.starts_with("http://");
                current_host = Some(clean);
                upstream = None;
            }
        }
        depth = depth
            .saturating_add(line.matches('{').count())
            .saturating_sub(line.matches('}').count());

        if let Some(ref host) = current_host {
            if let Some(rest) = line.strip_prefix("reverse_proxy ")
                && let Some(target) = rest.split_whitespace().next()
            {
                upstream = Some(target.to_owned());
            }
            if depth == 0 && line.contains('}') {
                let mut data = Map::new();
                data.insert("server_name".to_owned(), Value::String(host.clone()));
                data.insert(
                    "port".to_owned(),
                    Value::Number(if tls { 443u64 } else { 80u64 }.into()),
                );
                data.insert("tls".to_owned(), Value::Bool(tls));
                data.insert(
                    "upstream".to_owned(),
                    upstream.take().map_or(Value::Null, Value::String),
                );
                data.insert("engine".to_owned(), Value::String("caddy".to_owned()));
                items.push(Item {
                    kind: Kind::Vhost,
                    key: format!("caddy:{host}"),
                    data,
                    seen_via: SeenVia::Config,
                });
                current_host = None;
            }
        }
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nginx_and_caddy_vhosts() {
        let nginx = r#"
server {
    listen 443 ssl;
    server_name app.example.test;
    ssl_certificate /etc/letsencrypt/live/app.example.test/fullchain.pem;
    proxy_pass http://127.0.0.1:3000;
}
"#;
        let items = parse_nginx(nginx);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].key, "nginx:app.example.test:443");
        assert_eq!(items[0].bool_field("tls"), Some(true));
        assert_eq!(items[0].str_field("upstream"), Some("127.0.0.1:3000"));

        let caddy = r#"
api.example.test {
    reverse_proxy 127.0.0.1:8080
}
"#;
        let c_items = parse_caddy(caddy);
        assert_eq!(c_items.len(), 1);
        assert_eq!(c_items[0].key, "caddy:api.example.test");
        assert_eq!(c_items[0].str_field("upstream"), Some("127.0.0.1:8080"));
    }
}
