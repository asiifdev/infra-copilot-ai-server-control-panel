//! DNS provider adapter (`DnsProvider`), Cloudflare implementation, zone snapshots, and
//! domain ownership checks (§12.4).
//!
//! All DNS mutations are designed to run under tier-3 approval with pre-change snapshots and
//! deterministic rollback diffs. API tokens are wrapped in [`DnsApiToken`] (backed by
//! `core_domain::secret::Secret`) so they never appear in `Debug`, `Display`, logs, or errors.

use core_domain::secret::Secret;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

/// Default Cloudflare API v4 base URL.
pub const CLOUDFLARE_API_BASE_URL: &str = "https://api.cloudflare.com/client/v4";

/// Maximum length of a DNS zone or record identifier.
pub const MAX_ID_LEN: usize = 64;
/// Maximum length of a DNS hostname.
pub const MAX_DOMAIN_LEN: usize = 253;
/// Maximum length of a DNS record content payload.
pub const MAX_CONTENT_LEN: usize = 2048;
/// Maximum length of an API token.
pub const MAX_TOKEN_LEN: usize = 512;

/// An opaque, validated DNS provider API token. `Debug` and `Display` never expose the secret.
pub struct DnsApiToken(Secret);

impl DnsApiToken {
    /// Validates and wraps a raw API token.
    ///
    /// # Errors
    /// Returns [`DnsError::InvalidInput`] if the token is empty, too long, or contains whitespace
    /// or control characters.
    pub fn new(raw: impl Into<String>) -> Result<Self, DnsError> {
        let s = raw.into();
        if s.is_empty()
            || s.len() > MAX_TOKEN_LEN
            || s.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            return Err(DnsError::InvalidInput("invalid_api_token"));
        }
        Ok(Self(Secret::new(s)))
    }

    /// Exposes the raw token solely for injecting into an outbound `Authorization` header.
    #[must_use]
    pub fn expose_secret(&self) -> &str {
        self.0.expose_secret()
    }
}

impl fmt::Debug for DnsApiToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DnsApiToken(***)")
    }
}

impl fmt::Display for DnsApiToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

/// Errors returned by a [`DnsProvider`].
#[derive(Debug, Error)]
pub enum DnsError {
    /// Caller-supplied input failed validation.
    #[error("invalid dns input: {0}")]
    InvalidInput(&'static str),
    /// Outbound HTTP/TLS transport failure.
    #[error("dns http error: {0}")]
    Http(#[from] net::NetError),
    /// Provider API rejected the request or returned an error payload.
    #[error("dns api error (status {status}): {message}")]
    Api {
        /// HTTP status code.
        status: u16,
        /// Optional provider error code.
        code: Option<i64>,
        /// Sanitized error message (never includes credentials).
        message: String,
    },
    /// Response JSON could not be parsed.
    #[error("dns response parse error: {0}")]
    Parse(&'static str),
    /// Target zone or record was not found.
    #[error("dns resource not found")]
    NotFound,
    /// Provider rejected the API token.
    #[error("dns provider unauthorized")]
    Unauthorized,
    /// Provider rate-limited the request.
    #[error("dns provider rate limited")]
    RateLimited,
}

/// Supported DNS record types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum DnsRecordKind {
    A,
    Aaaa,
    Cname,
    Txt,
    Mx,
    Ns,
    Srv,
    Caa,
    Ptr,
}

impl DnsRecordKind {
    /// Canonical uppercase DNS record type string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::Aaaa => "AAAA",
            Self::Cname => "CNAME",
            Self::Txt => "TXT",
            Self::Mx => "MX",
            Self::Ns => "NS",
            Self::Srv => "SRV",
            Self::Caa => "CAA",
            Self::Ptr => "PTR",
        }
    }

    /// Parses a DNS record type string (case-insensitive).
    ///
    /// # Errors
    /// Returns [`DnsError::InvalidInput`] if unsupported.
    pub fn parse(raw: &str) -> Result<Self, DnsError> {
        match raw.trim().to_ascii_uppercase().as_str() {
            "A" => Ok(Self::A),
            "AAAA" => Ok(Self::Aaaa),
            "CNAME" => Ok(Self::Cname),
            "TXT" => Ok(Self::Txt),
            "MX" => Ok(Self::Mx),
            "NS" => Ok(Self::Ns),
            "SRV" => Ok(Self::Srv),
            "CAA" => Ok(Self::Caa),
            "PTR" => Ok(Self::Ptr),
            _ => Err(DnsError::InvalidInput("unsupported_record_kind")),
        }
    }
}

/// A DNS zone managed by a provider account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnsZone {
    /// Provider-assigned zone identifier.
    pub id: String,
    /// Apex domain name (e.g. `example.com`).
    pub name: String,
    /// Zone status (e.g. `active`, `pending`).
    pub status: String,
    /// Optional owning account identifier.
    pub account_id: Option<String>,
    /// Assigned authoritative name servers.
    pub name_servers: Vec<String>,
}

/// A DNS record inside a zone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnsRecord {
    /// Provider-assigned record identifier (`None` when creating a new record).
    pub id: Option<String>,
    /// Owning zone identifier.
    pub zone_id: String,
    /// Record type.
    pub kind: DnsRecordKind,
    /// Record FQDN (or `@` for apex).
    pub name: String,
    /// Record value / target.
    pub content: String,
    /// Time to live in seconds (`1` = automatic in Cloudflare, or `60..=86400`).
    pub ttl: u32,
    /// Whether traffic is proxied through the provider (Cloudflare orange cloud).
    pub proxied: Option<bool>,
    /// Optional priority (for MX/SRV records).
    pub priority: Option<u16>,
}

/// A point-in-time snapshot of a DNS zone and all its records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZoneSnapshot {
    /// Zone metadata at snapshot time.
    pub zone: DnsZone,
    /// Sorted list of records in the zone.
    pub records: Vec<DnsRecord>,
    /// Unix epoch timestamp (seconds) when the snapshot was captured.
    pub captured_at_unix: u64,
}

/// Difference between two [`ZoneSnapshot`] states (`self` = baseline, `current` = later state).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotDiff {
    /// Records present in `current` that were not in `baseline`.
    pub added: Vec<DnsRecord>,
    /// Records present in `baseline` that are missing from `current`.
    pub removed: Vec<DnsRecord>,
    /// Records present in both with different values (`(baseline, current)`).
    pub modified: Vec<(DnsRecord, DnsRecord)>,
}

/// A single operation required to roll a zone back to a baseline [`ZoneSnapshot`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RollbackOp {
    /// Recreate or restore a record to its baseline state.
    Upsert(DnsRecord),
    /// Delete a record that was added after the baseline.
    Delete {
        /// Owning zone ID.
        zone_id: String,
        /// Record ID to delete.
        record_id: String,
    },
}

impl ZoneSnapshot {
    /// Computes the diff from `self` (baseline) to `current`.
    #[must_use]
    pub fn diff(&self, current: &Self) -> SnapshotDiff {
        let base_map = index_records(&self.records);
        let curr_map = index_records(&current.records);

        let mut added = Vec::new();
        let mut removed = Vec::new();
        let mut modified = Vec::new();

        for (key, base_rec) in &base_map {
            match curr_map.get(key) {
                None => removed.push((*base_rec).clone()),
                Some(curr_rec) if *base_rec != *curr_rec => {
                    modified.push(((*base_rec).clone(), (*curr_rec).clone()));
                }
                Some(_) => {}
            }
        }
        for (key, curr_rec) in &curr_map {
            if !base_map.contains_key(key) {
                added.push((*curr_rec).clone());
            }
        }

        SnapshotDiff {
            added,
            removed,
            modified,
        }
    }

    /// Generates the ordered rollback operations needed to restore `current` back to `self`.
    #[must_use]
    pub fn rollback_plan(&self, current: &Self) -> Vec<RollbackOp> {
        let diff = self.diff(current);
        let mut ops = Vec::new();
        for rec in diff.added {
            if let Some(id) = rec.id {
                ops.push(RollbackOp::Delete {
                    zone_id: rec.zone_id,
                    record_id: id,
                });
            }
        }
        for (base_rec, _curr_rec) in diff.modified {
            ops.push(RollbackOp::Upsert(base_rec));
        }
        for base_rec in diff.removed {
            ops.push(RollbackOp::Upsert(base_rec));
        }
        ops
    }
}

fn index_records(records: &[DnsRecord]) -> BTreeMap<String, &DnsRecord> {
    let mut map = BTreeMap::new();
    for (idx, rec) in records.iter().enumerate() {
        let key = rec.id.as_deref().map_or_else(
            || {
                format!(
                    "anon:{}:{}:{}:{idx}",
                    rec.kind.as_str(),
                    rec.name,
                    rec.content
                )
            },
            |id| format!("id:{id}"),
        );
        map.insert(key, rec);
    }
    map
}

/// Result of checking whether a domain is controlled by the connected DNS provider account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OwnershipStatus {
    /// Domain belongs to an active/known zone in the connected account.
    Owned {
        /// Matching zone in the connected account.
        zone: DnsZone,
    },
    /// Domain is not in the connected account (managed externally or in another account).
    External {
        /// Normalized domain queried.
        domain: String,
        /// Instructions for the domain owner to verify or delegate DNS control.
        verification_hint: String,
    },
}

/// Trait implemented by DNS provider adapters (Cloudflare, etc.).
pub trait DnsProvider: Send + Sync {
    /// Lists all zones accessible to the configured credentials.
    fn list_zones(&self) -> impl Future<Output = Result<Vec<DnsZone>, DnsError>> + Send;

    /// Lists all DNS records in `zone_id`.
    fn list_records(
        &self,
        zone_id: &str,
    ) -> impl Future<Output = Result<Vec<DnsRecord>, DnsError>> + Send;

    /// Creates or updates a DNS record.
    fn upsert_record(
        &self,
        record: &DnsRecord,
    ) -> impl Future<Output = Result<DnsRecord, DnsError>> + Send;

    /// Deletes a DNS record by `zone_id` and `record_id`.
    fn delete_record(
        &self,
        zone_id: &str,
        record_id: &str,
    ) -> impl Future<Output = Result<(), DnsError>> + Send;

    /// Captures a point-in-time [`ZoneSnapshot`] of `zone_id`.
    fn snapshot_zone(
        &self,
        zone_id: &str,
    ) -> impl Future<Output = Result<ZoneSnapshot, DnsError>> + Send;

    /// Checks whether `domain` is owned by the connected provider account.
    fn check_ownership(
        &self,
        domain: &str,
    ) -> impl Future<Output = Result<OwnershipStatus, DnsError>> + Send;
}

/// Validates a provider zone or record ID (alphanumeric, `_`, `-` only; prevents path injection).
///
/// # Errors
/// Returns [`DnsError::InvalidInput`] if invalid.
pub fn validate_identifier(id: &str, field: &'static str) -> Result<(), DnsError> {
    if id.is_empty()
        || id.len() > MAX_ID_LEN
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(DnsError::InvalidInput(field));
    }
    Ok(())
}

/// Validates a DNS domain name (lowercase normalized, no scheme, no path, no wildcards).
///
/// # Errors
/// Returns [`DnsError::InvalidInput`] if invalid.
pub fn validate_domain(raw: &str) -> Result<String, DnsError> {
    let domain = raw.trim().trim_end_matches('.').to_ascii_lowercase();
    if domain.is_empty() || domain.len() > MAX_DOMAIN_LEN || !domain.contains('.') {
        return Err(DnsError::InvalidInput("invalid_domain"));
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
            return Err(DnsError::InvalidInput("invalid_domain"));
        }
    }
    Ok(domain)
}

/// Validates a DNS record `name` (apex `@`, wildcard `*.example.com`, or `_acme-challenge.example.com`).
///
/// # Errors
/// Returns [`DnsError::InvalidInput`] if invalid.
pub fn validate_record_name(raw: &str) -> Result<String, DnsError> {
    let name = raw.trim().trim_end_matches('.').to_ascii_lowercase();
    if name == "@" {
        return Ok(name);
    }
    if name.is_empty() || name.len() > MAX_DOMAIN_LEN {
        return Err(DnsError::InvalidInput("invalid_record_name"));
    }
    let labels: Vec<&str> = name.split('.').collect();
    for (idx, label) in labels.iter().enumerate() {
        if *label == "*" && idx == 0 && labels.len() > 1 {
            continue;
        }
        if label.is_empty()
            || label.len() > 63
            || label.starts_with('-')
            || label.ends_with('-')
            || !label
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        {
            return Err(DnsError::InvalidInput("invalid_record_name"));
        }
    }
    Ok(name)
}

/// Validates all fields of a [`DnsRecord`].
///
/// # Errors
/// Returns [`DnsError::InvalidInput`] if any field is invalid.
pub fn validate_record(record: &DnsRecord) -> Result<(), DnsError> {
    validate_identifier(&record.zone_id, "invalid_zone_id")?;
    if let Some(ref id) = record.id {
        validate_identifier(id, "invalid_record_id")?;
    }
    validate_record_name(&record.name)?;
    if record.content.is_empty()
        || record.content.len() > MAX_CONTENT_LEN
        || record.content.chars().any(char::is_control)
    {
        return Err(DnsError::InvalidInput("invalid_record_content"));
    }
    if record.ttl != 1 && !(60..=86_400).contains(&record.ttl) {
        return Err(DnsError::InvalidInput("invalid_ttl"));
    }
    Ok(())
}

fn validate_base_url(raw: &str, allow_http: bool) -> Result<String, DnsError> {
    let trimmed = raw.trim().trim_end_matches('/');
    let valid_scheme =
        trimmed.starts_with("https://") || (allow_http && trimmed.starts_with("http://"));
    if !valid_scheme
        || trimmed.contains('?')
        || trimmed.contains('#')
        || trimmed.contains('@')
        || trimmed.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(DnsError::InvalidInput("invalid_base_url"));
    }
    Ok(trimmed.to_owned())
}

/// Minimal HTTP transport abstraction for [`CloudflareDnsProvider`], backed by [`net::Client`]
/// in production and mockable in unit tests without opening TCP sockets.
pub trait DnsHttpTransport: Send + Sync {
    /// Executes an HTTP request with the given method (`"GET"`, `"POST"`, `"PUT"`, `"DELETE"`),
    /// full URL, optional JSON body, and headers.
    fn request(
        &self,
        method: &str,
        url: &str,
        body: Option<&[u8]>,
        headers: &[(&str, &str)],
    ) -> impl Future<Output = Result<DnsHttpResponse, DnsError>> + Send;
}

/// Minimal HTTP response returned by [`DnsHttpTransport`].
#[derive(Debug, Clone)]
pub struct DnsHttpResponse {
    /// HTTP status code.
    pub status: u16,
    /// Raw response body bytes.
    pub body: Vec<u8>,
}

/// Production [`DnsHttpTransport`] backed by the hardened [`net::Client`].
#[derive(Debug)]
pub struct NetDnsTransport {
    client: Arc<net::Client>,
    policy: net::Policy,
}

impl NetDnsTransport {
    /// Wraps a [`net::Client`] and [`net::Policy`].
    #[must_use]
    pub fn new(client: Arc<net::Client>, policy: net::Policy) -> Self {
        Self { client, policy }
    }
}

impl DnsHttpTransport for NetDnsTransport {
    async fn request(
        &self,
        method: &str,
        url: &str,
        body: Option<&[u8]>,
        headers: &[(&str, &str)],
    ) -> Result<DnsHttpResponse, DnsError> {
        let resp = match method {
            "GET" => self.client.get_with(url, headers, &self.policy).await?,
            "POST" => {
                self.client
                    .post_json_with(url, body.unwrap_or_default(), headers, &self.policy)
                    .await?
            }
            "PUT" => {
                self.client
                    .put_json_with(url, body.unwrap_or_default(), headers, &self.policy)
                    .await?
            }
            "DELETE" => self.client.delete_with(url, headers, &self.policy).await?,
            _ => return Err(DnsError::InvalidInput("unsupported_http_method")),
        };
        Ok(DnsHttpResponse {
            status: resp.status,
            body: resp.body,
        })
    }
}

/// Cloudflare DNS v4 API adapter backed by [`net::Client`] (or a custom [`DnsHttpTransport`]).
pub struct CloudflareDnsProvider<T: DnsHttpTransport = NetDnsTransport> {
    transport: T,
    token: DnsApiToken,
    base_url: String,
}

impl<T: DnsHttpTransport> fmt::Debug for CloudflareDnsProvider<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CloudflareDnsProvider")
            .field("base_url", &self.base_url)
            .field("token", &self.token)
            .finish()
    }
}

impl CloudflareDnsProvider<NetDnsTransport> {
    /// Builds a Cloudflare DNS provider targeting [`CLOUDFLARE_API_BASE_URL`].
    ///
    /// # Errors
    /// Returns [`DnsError`] if the HTTP client cannot be initialized.
    pub fn new(token: DnsApiToken) -> Result<Self, DnsError> {
        let client = Arc::new(net::Client::new()?);
        Self::with_client(
            client,
            token,
            CLOUDFLARE_API_BASE_URL,
            net::Policy::default(),
        )
    }

    /// Builds a Cloudflare DNS provider with an explicit `net::Client`, `base_url`, and `policy`
    /// (used for testing against local mock servers).
    ///
    /// # Errors
    /// Returns [`DnsError::InvalidInput`] if `base_url` is malformed.
    pub fn with_client(
        client: Arc<net::Client>,
        token: DnsApiToken,
        base_url: &str,
        policy: net::Policy,
    ) -> Result<Self, DnsError> {
        let allow_http = policy.allow_http;
        Self::with_transport(
            NetDnsTransport::new(client, policy),
            token,
            base_url,
            allow_http,
        )
    }
}

impl<T: DnsHttpTransport> CloudflareDnsProvider<T> {
    /// Builds a Cloudflare DNS provider with a custom [`DnsHttpTransport`].
    ///
    /// # Errors
    /// Returns [`DnsError::InvalidInput`] if `base_url` is malformed.
    pub fn with_transport(
        transport: T,
        token: DnsApiToken,
        base_url: &str,
        allow_http: bool,
    ) -> Result<Self, DnsError> {
        let base_url = validate_base_url(base_url, allow_http)?;
        Ok(Self {
            transport,
            token,
            base_url,
        })
    }

    fn auth_header_value(&self) -> String {
        format!("Bearer {}", self.token.expose_secret())
    }

    async fn get_json(&self, path: &str) -> Result<serde_json::Value, DnsError> {
        let url = format!("{}{path}", self.base_url);
        let auth = self.auth_header_value();
        let headers = [
            ("Authorization", auth.as_str()),
            ("Accept", "application/json"),
        ];
        let resp = self.transport.request("GET", &url, None, &headers).await?;
        parse_cf_response(resp)
    }

    async fn send_json(
        &self,
        is_put: bool,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, DnsError> {
        let url = format!("{}{path}", self.base_url);
        let auth = self.auth_header_value();
        let headers = [
            ("Authorization", auth.as_str()),
            ("Accept", "application/json"),
        ];
        let bytes = serde_json::to_vec(body).map_err(|_| DnsError::Parse("serialize_failed"))?;
        let method = if is_put { "PUT" } else { "POST" };
        let resp = self
            .transport
            .request(method, &url, Some(&bytes), &headers)
            .await?;
        parse_cf_response(resp)
    }

    async fn delete_json(&self, path: &str) -> Result<serde_json::Value, DnsError> {
        let url = format!("{}{path}", self.base_url);
        let auth = self.auth_header_value();
        let headers = [
            ("Authorization", auth.as_str()),
            ("Accept", "application/json"),
        ];
        let resp = self
            .transport
            .request("DELETE", &url, None, &headers)
            .await?;
        parse_cf_response(resp)
    }
}

impl<T: DnsHttpTransport> DnsProvider for CloudflareDnsProvider<T> {
    async fn list_zones(&self) -> Result<Vec<DnsZone>, DnsError> {
        let val = self.get_json("/zones?per_page=50").await?;
        let arr = val
            .get("result")
            .and_then(serde_json::Value::as_array)
            .ok_or(DnsError::Parse("missing_zones_result"))?;
        arr.iter().map(parse_cf_zone).collect()
    }

    async fn list_records(&self, zone_id: &str) -> Result<Vec<DnsRecord>, DnsError> {
        validate_identifier(zone_id, "invalid_zone_id")?;
        let val = self
            .get_json(&format!("/zones/{zone_id}/dns_records?per_page=100"))
            .await?;
        let arr = val
            .get("result")
            .and_then(serde_json::Value::as_array)
            .ok_or(DnsError::Parse("missing_records_result"))?;
        arr.iter()
            .map(|item| parse_cf_record(item, zone_id))
            .collect()
    }

    async fn upsert_record(&self, record: &DnsRecord) -> Result<DnsRecord, DnsError> {
        validate_record(record)?;
        let normalized_name = validate_record_name(&record.name)?;
        let mut payload = serde_json::json!({
            "type": record.kind.as_str(),
            "name": normalized_name,
            "content": record.content,
            "ttl": record.ttl,
        });
        if let Some(proxied) = record.proxied {
            payload["proxied"] = serde_json::Value::Bool(proxied);
        }
        if let Some(priority) = record.priority {
            payload["priority"] = serde_json::Value::Number(serde_json::Number::from(priority));
        }

        let target_id = if let Some(ref id) = record.id {
            Some(id.clone())
        } else {
            let existing = self.list_records(&record.zone_id).await?;
            existing
                .into_iter()
                .find(|r| r.kind == record.kind && r.name == normalized_name)
                .and_then(|r| r.id)
        };

        let val = if let Some(id) = target_id {
            self.send_json(
                true,
                &format!("/zones/{}/dns_records/{id}", record.zone_id),
                &payload,
            )
            .await?
        } else {
            self.send_json(
                false,
                &format!("/zones/{}/dns_records", record.zone_id),
                &payload,
            )
            .await?
        };
        let result = val
            .get("result")
            .ok_or(DnsError::Parse("missing_record_result"))?;
        parse_cf_record(result, &record.zone_id)
    }

    async fn delete_record(&self, zone_id: &str, record_id: &str) -> Result<(), DnsError> {
        validate_identifier(zone_id, "invalid_zone_id")?;
        validate_identifier(record_id, "invalid_record_id")?;
        let _ = self
            .delete_json(&format!("/zones/{zone_id}/dns_records/{record_id}"))
            .await?;
        Ok(())
    }

    async fn snapshot_zone(&self, zone_id: &str) -> Result<ZoneSnapshot, DnsError> {
        validate_identifier(zone_id, "invalid_zone_id")?;
        let zone_val = self.get_json(&format!("/zones/{zone_id}")).await?;
        let zone_obj = zone_val
            .get("result")
            .ok_or(DnsError::Parse("missing_zone_result"))?;
        let zone = parse_cf_zone(zone_obj)?;
        let mut records = self.list_records(zone_id).await?;
        records.sort_by(|a, b| {
            (a.kind, &a.name, &a.content, &a.id).cmp(&(b.kind, &b.name, &b.content, &b.id))
        });
        let captured_at_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        Ok(ZoneSnapshot {
            zone,
            records,
            captured_at_unix,
        })
    }

    async fn check_ownership(&self, domain: &str) -> Result<OwnershipStatus, DnsError> {
        let normalized = validate_domain(domain)?;
        let zones = self.list_zones().await?;
        let matched = zones.into_iter().find(|z| {
            let zname = z.name.to_ascii_lowercase();
            normalized == zname || normalized.ends_with(&format!(".{zname}"))
        });
        match matched {
            Some(zone) => Ok(OwnershipStatus::Owned { zone }),
            None => Ok(OwnershipStatus::External {
                verification_hint: format!(
                    "Domain '{normalized}' is not in the connected Cloudflare account. \
                     Connect the owning DNS account or add a TXT verification record at \
                     _nodivara-verify.{normalized} before cutover."
                ),
                domain: normalized,
            }),
        }
    }
}

fn parse_cf_response(resp: DnsHttpResponse) -> Result<serde_json::Value, DnsError> {
    if resp.status == 401 || resp.status == 403 {
        return Err(DnsError::Unauthorized);
    }
    if resp.status == 404 {
        return Err(DnsError::NotFound);
    }
    if resp.status == 429 {
        return Err(DnsError::RateLimited);
    }
    let val: serde_json::Value =
        serde_json::from_slice(&resp.body).map_err(|_| DnsError::Parse("invalid_json"))?;
    let success = val
        .get("success")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if !success || resp.status >= 400 {
        let first_err = val
            .get("errors")
            .and_then(serde_json::Value::as_array)
            .and_then(|arr| arr.first());
        let code = first_err
            .and_then(|e| e.get("code"))
            .and_then(serde_json::Value::as_i64);
        let message = first_err
            .and_then(|e| e.get("message"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("cloudflare_api_error")
            .chars()
            .take(200)
            .collect();
        return Err(DnsError::Api {
            status: resp.status,
            code,
            message,
        });
    }
    Ok(val)
}

fn parse_cf_zone(val: &serde_json::Value) -> Result<DnsZone, DnsError> {
    let id = val
        .get("id")
        .and_then(serde_json::Value::as_str)
        .ok_or(DnsError::Parse("zone_missing_id"))?
        .to_owned();
    let name = val
        .get("name")
        .and_then(serde_json::Value::as_str)
        .ok_or(DnsError::Parse("zone_missing_name"))?
        .to_owned();
    let status = val
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    let account_id = val
        .get("account")
        .and_then(|a| a.get("id"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let name_servers = val
        .get("name_servers")
        .and_then(serde_json::Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    Ok(DnsZone {
        id,
        name,
        status,
        account_id,
        name_servers,
    })
}

fn parse_cf_record(val: &serde_json::Value, fallback_zone_id: &str) -> Result<DnsRecord, DnsError> {
    let id = val
        .get("id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let zone_id = val
        .get("zone_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(fallback_zone_id)
        .to_owned();
    let kind_str = val
        .get("type")
        .and_then(serde_json::Value::as_str)
        .ok_or(DnsError::Parse("record_missing_type"))?;
    let kind = DnsRecordKind::parse(kind_str)?;
    let name = val
        .get("name")
        .and_then(serde_json::Value::as_str)
        .ok_or(DnsError::Parse("record_missing_name"))?
        .to_owned();
    let content = val
        .get("content")
        .and_then(serde_json::Value::as_str)
        .ok_or(DnsError::Parse("record_missing_content"))?
        .to_owned();
    let ttl = val
        .get("ttl")
        .and_then(serde_json::Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(1);
    let proxied = val.get("proxied").and_then(serde_json::Value::as_bool);
    let priority = val
        .get("priority")
        .and_then(serde_json::Value::as_u64)
        .and_then(|v| u16::try_from(v).ok());
    Ok(DnsRecord {
        id,
        zone_id,
        kind,
        name,
        content,
        ttl,
        proxied,
        priority,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// In-memory mock provider to verify the [`DnsProvider`] contract.
    struct InMemoryDnsProvider {
        zones: Vec<DnsZone>,
        records: Mutex<HashMap<String, Vec<DnsRecord>>>,
        next_id: Mutex<u64>,
    }

    impl InMemoryDnsProvider {
        fn seeded() -> Self {
            let zone = DnsZone {
                id: "zone_1".to_owned(),
                name: "example.com".to_owned(),
                status: "active".to_owned(),
                account_id: Some("acct_1".to_owned()),
                name_servers: vec!["ns1.example.net".to_owned()],
            };
            let rec = DnsRecord {
                id: Some("rec_1".to_owned()),
                zone_id: "zone_1".to_owned(),
                kind: DnsRecordKind::A,
                name: "example.com".to_owned(),
                content: "192.0.2.10".to_owned(),
                ttl: 1,
                proxied: Some(true),
                priority: None,
            };
            let mut map = HashMap::new();
            map.insert("zone_1".to_owned(), vec![rec]);
            Self {
                zones: vec![zone],
                records: Mutex::new(map),
                next_id: Mutex::new(2),
            }
        }
    }

    impl DnsProvider for InMemoryDnsProvider {
        async fn list_zones(&self) -> Result<Vec<DnsZone>, DnsError> {
            Ok(self.zones.clone())
        }

        async fn list_records(&self, zone_id: &str) -> Result<Vec<DnsRecord>, DnsError> {
            validate_identifier(zone_id, "invalid_zone_id")?;
            let guard = self
                .records
                .lock()
                .map_err(|_| DnsError::Parse("lock_poisoned"))?;
            guard.get(zone_id).cloned().ok_or(DnsError::NotFound)
        }

        async fn upsert_record(&self, record: &DnsRecord) -> Result<DnsRecord, DnsError> {
            validate_record(record)?;
            let mut guard = self
                .records
                .lock()
                .map_err(|_| DnsError::Parse("lock_poisoned"))?;
            let list = guard.get_mut(&record.zone_id).ok_or(DnsError::NotFound)?;
            if let Some(existing) = list.iter_mut().find(|r| {
                (record.id.is_some() && r.id == record.id)
                    || (record.id.is_none() && r.kind == record.kind && r.name == record.name)
            }) {
                let id = existing.id.clone();
                *existing = record.clone();
                existing.id = id;
                return Ok(existing.clone());
            }
            let mut id_guard = self
                .next_id
                .lock()
                .map_err(|_| DnsError::Parse("lock_poisoned"))?;
            let id = format!("rec_{}", *id_guard);
            *id_guard += 1;
            let mut created = record.clone();
            created.id = Some(id);
            list.push(created.clone());
            Ok(created)
        }

        async fn delete_record(&self, zone_id: &str, record_id: &str) -> Result<(), DnsError> {
            validate_identifier(zone_id, "invalid_zone_id")?;
            validate_identifier(record_id, "invalid_record_id")?;
            let mut guard = self
                .records
                .lock()
                .map_err(|_| DnsError::Parse("lock_poisoned"))?;
            let list = guard.get_mut(zone_id).ok_or(DnsError::NotFound)?;
            let before = list.len();
            list.retain(|r| r.id.as_deref() != Some(record_id));
            if list.len() == before {
                return Err(DnsError::NotFound);
            }
            Ok(())
        }

        async fn snapshot_zone(&self, zone_id: &str) -> Result<ZoneSnapshot, DnsError> {
            let zone = self
                .zones
                .iter()
                .find(|z| z.id == zone_id)
                .cloned()
                .ok_or(DnsError::NotFound)?;
            let records = self.list_records(zone_id).await?;
            Ok(ZoneSnapshot {
                zone,
                records,
                captured_at_unix: 1_700_000_000,
            })
        }

        async fn check_ownership(&self, domain: &str) -> Result<OwnershipStatus, DnsError> {
            let normalized = validate_domain(domain)?;
            if let Some(zone) = self
                .zones
                .iter()
                .find(|z| normalized == z.name || normalized.ends_with(&format!(".{}", z.name)))
            {
                return Ok(OwnershipStatus::Owned { zone: zone.clone() });
            }
            Ok(OwnershipStatus::External {
                domain: normalized.clone(),
                verification_hint: format!("Add TXT _nodivara-verify.{normalized}"),
            })
        }
    }

    async fn run_contract_tests<P: DnsProvider>(provider: &P) -> Result<(), DnsError> {
        let zones = provider.list_zones().await?;
        assert_eq!(zones.len(), 1);
        assert_eq!(zones[0].name, "example.com");

        let owned = provider.check_ownership("app.example.com").await?;
        assert!(matches!(owned, OwnershipStatus::Owned { .. }));

        let external = provider.check_ownership("other-domain.org").await?;
        assert!(matches!(external, OwnershipStatus::External { .. }));

        let baseline = provider.snapshot_zone("zone_1").await?;
        assert_eq!(baseline.records.len(), 1);

        let created = provider
            .upsert_record(&DnsRecord {
                id: None,
                zone_id: "zone_1".to_owned(),
                kind: DnsRecordKind::Cname,
                name: "www.example.com".to_owned(),
                content: "example.com".to_owned(),
                ttl: 1,
                proxied: Some(true),
                priority: None,
            })
            .await?;
        assert!(created.id.is_some());

        let after_create = provider.snapshot_zone("zone_1").await?;
        let plan = baseline.rollback_plan(&after_create);
        assert_eq!(plan.len(), 1);
        assert!(matches!(plan[0], RollbackOp::Delete { .. }));

        let created_id = created.id.as_deref().unwrap_or_default();
        provider.delete_record("zone_1", created_id).await?;
        let after_delete = provider.snapshot_zone("zone_1").await?;
        assert_eq!(baseline.diff(&after_delete).added.len(), 0);
        assert_eq!(baseline.diff(&after_delete).removed.len(), 0);
        Ok(())
    }

    #[test]
    fn token_never_prints_in_debug_or_display() -> Result<(), DnsError> {
        let token = DnsApiToken::new("cf_secret_token_99999")?;
        assert_eq!(format!("{token:?}"), "DnsApiToken(***)");
        assert_eq!(format!("{token}"), "***");
        assert!(!format!("{token:?}").contains("cf_secret_token_99999"));
        assert!(!format!("{token}").contains("cf_secret_token_99999"));
        assert!(DnsApiToken::new("").is_err());
        assert!(DnsApiToken::new("bad token").is_err());
        assert!(DnsApiToken::new("bad\ntoken").is_err());
        Ok(())
    }

    #[test]
    fn strict_input_validation_rejects_injection() {
        assert!(validate_identifier("../etc", "id").is_err());
        assert!(validate_identifier("zone/1?x=1", "id").is_err());
        assert!(validate_identifier("", "id").is_err());
        assert!(validate_identifier("zone_123-abc", "id").is_ok());

        assert!(validate_domain("localhost").is_err());
        assert!(validate_domain("-bad.com").is_err());
        assert!(validate_domain("a..b.com").is_err());
        assert!(validate_domain("Sub.Example.COM.").is_ok());

        assert!(validate_record_name("*.example.com").is_ok());
        assert!(validate_record_name("@").is_ok());
        assert!(validate_record_name("bad;cmd.example.com").is_err());

        let bad_ttl = DnsRecord {
            id: None,
            zone_id: "zone_1".to_owned(),
            kind: DnsRecordKind::A,
            name: "app.example.com".to_owned(),
            content: "192.0.2.1".to_owned(),
            ttl: 5,
            proxied: None,
            priority: None,
        };
        assert!(validate_record(&bad_ttl).is_err());
    }

    #[tokio::test]
    async fn contract_tests_pass_on_in_memory_provider() -> Result<(), DnsError> {
        let provider = InMemoryDnsProvider::seeded();
        run_contract_tests(&provider).await
    }

    struct MockCfTransport {
        state: Mutex<Vec<serde_json::Value>>,
        forced_status: Option<u16>,
    }

    impl DnsHttpTransport for MockCfTransport {
        async fn request(
            &self,
            method: &str,
            url: &str,
            body: Option<&[u8]>,
            headers: &[(&str, &str)],
        ) -> Result<DnsHttpResponse, DnsError> {
            assert!(
                headers
                    .iter()
                    .any(|(k, v)| k.eq_ignore_ascii_case("authorization")
                        && *v == "Bearer cf_test_token_123")
            );
            if let Some(status) = self.forced_status {
                return Ok(DnsHttpResponse {
                    status,
                    body: serde_json::json!({
                        "success": false,
                        "errors": [{ "code": i64::from(status), "message": "forced_error" }]
                    })
                    .to_string()
                    .into_bytes(),
                });
            }
            let path = url
                .strip_prefix("https://mock.cloudflare.test/client/v4")
                .unwrap_or(url);
            let (status, json_body) = if method == "GET" && path.starts_with("/zones?") {
                (
                    200,
                    serde_json::json!({
                        "success": true,
                        "result": [{
                            "id": "zone_1",
                            "name": "example.com",
                            "status": "active",
                            "account": { "id": "acct_1" },
                            "name_servers": ["ns1.example.net"]
                        }]
                    }),
                )
            } else if method == "GET" && path == "/zones/zone_1" {
                (
                    200,
                    serde_json::json!({
                        "success": true,
                        "result": {
                            "id": "zone_1",
                            "name": "example.com",
                            "status": "active",
                            "account": { "id": "acct_1" },
                            "name_servers": ["ns1.example.net"]
                        }
                    }),
                )
            } else if method == "GET" && path.starts_with("/zones/zone_1/dns_records") {
                let recs = self.state.lock().map(|g| g.clone()).unwrap_or_default();
                (200, serde_json::json!({ "success": true, "result": recs }))
            } else if method == "POST" && path == "/zones/zone_1/dns_records" {
                let mut parsed: serde_json::Value =
                    serde_json::from_slice(body.unwrap_or_default()).unwrap_or_default();
                parsed["id"] = serde_json::json!("rec_2");
                parsed["zone_id"] = serde_json::json!("zone_1");
                if let Ok(mut guard) = self.state.lock() {
                    guard.push(parsed.clone());
                }
                (
                    200,
                    serde_json::json!({ "success": true, "result": parsed }),
                )
            } else if method == "PUT" && path.starts_with("/zones/zone_1/dns_records/") {
                let rec_id = path.rsplit('/').next().unwrap_or("rec_1");
                let mut parsed: serde_json::Value =
                    serde_json::from_slice(body.unwrap_or_default()).unwrap_or_default();
                parsed["id"] = serde_json::json!(rec_id);
                parsed["zone_id"] = serde_json::json!("zone_1");
                if let Ok(mut guard) = self.state.lock() {
                    for item in guard.iter_mut() {
                        if item.get("id").and_then(|v| v.as_str()) == Some(rec_id) {
                            *item = parsed.clone();
                        }
                    }
                }
                (
                    200,
                    serde_json::json!({ "success": true, "result": parsed }),
                )
            } else if method == "DELETE" && path == "/zones/zone_1/dns_records/rec_2" {
                if let Ok(mut guard) = self.state.lock() {
                    guard.retain(|r| r.get("id").and_then(|v| v.as_str()) != Some("rec_2"));
                }
                (
                    200,
                    serde_json::json!({ "success": true, "result": { "id": "rec_2" } }),
                )
            } else {
                (
                    404,
                    serde_json::json!({ "success": false, "errors": [{ "code": 404, "message": "not_found" }] }),
                )
            };

            Ok(DnsHttpResponse {
                status,
                body: json_body.to_string().into_bytes(),
            })
        }
    }

    #[tokio::test]
    async fn cloudflare_provider_passes_contract_against_mock_http() -> Result<(), DnsError> {
        let transport = MockCfTransport {
            state: Mutex::new(vec![serde_json::json!({
                "id": "rec_1",
                "zone_id": "zone_1",
                "type": "A",
                "name": "example.com",
                "content": "192.0.2.10",
                "ttl": 1,
                "proxied": true
            })]),
            forced_status: None,
        };
        let token = DnsApiToken::new("cf_test_token_123")?;
        let provider = CloudflareDnsProvider::with_transport(
            transport,
            token,
            "https://mock.cloudflare.test/client/v4",
            false,
        )?;
        assert_eq!(
            format!("{provider:?}"),
            "CloudflareDnsProvider { base_url: \"https://mock.cloudflare.test/client/v4\", token: DnsApiToken(***) }"
        );

        run_contract_tests(&provider).await
    }

    #[tokio::test]
    async fn cloudflare_error_statuses_never_leak_token() -> Result<(), DnsError> {
        for (status, expected_unauth, expected_rl) in
            [(401, true, false), (403, true, false), (429, false, true)]
        {
            let transport = MockCfTransport {
                state: Mutex::new(vec![]),
                forced_status: Some(status),
            };
            let token = DnsApiToken::new("cf_test_token_123")?;
            let provider = CloudflareDnsProvider::with_transport(
                transport,
                token,
                "https://mock.cloudflare.test/client/v4",
                false,
            )?;
            let Err(err) = provider.list_zones().await else {
                return Err(DnsError::Parse("expected_error"));
            };
            let msg = format!("{err}");
            assert!(!msg.contains("cf_test_token_123"));
            assert_eq!(matches!(err, DnsError::Unauthorized), expected_unauth);
            assert_eq!(matches!(err, DnsError::RateLimited), expected_rl);
        }
        Ok(())
    }

    #[test]
    fn snapshot_diff_detects_added_removed_modified_and_builds_rollback() {
        let zone = DnsZone {
            id: "z1".to_owned(),
            name: "example.com".to_owned(),
            status: "active".to_owned(),
            account_id: None,
            name_servers: vec![],
        };
        let base_a = DnsRecord {
            id: Some("r1".to_owned()),
            zone_id: "z1".to_owned(),
            kind: DnsRecordKind::A,
            name: "example.com".to_owned(),
            content: "192.0.2.1".to_owned(),
            ttl: 300,
            proxied: Some(false),
            priority: None,
        };
        let base_txt = DnsRecord {
            id: Some("r2".to_owned()),
            zone_id: "z1".to_owned(),
            kind: DnsRecordKind::Txt,
            name: "example.com".to_owned(),
            content: "v=spf1 -all".to_owned(),
            ttl: 300,
            proxied: None,
            priority: None,
        };
        let baseline = ZoneSnapshot {
            zone: zone.clone(),
            records: vec![base_a.clone(), base_txt.clone()],
            captured_at_unix: 100,
        };

        let mut mod_a = base_a.clone();
        mod_a.content = "192.0.2.2".to_owned();
        let added_cname = DnsRecord {
            id: Some("r3".to_owned()),
            zone_id: "z1".to_owned(),
            kind: DnsRecordKind::Cname,
            name: "www.example.com".to_owned(),
            content: "example.com".to_owned(),
            ttl: 1,
            proxied: Some(true),
            priority: None,
        };
        let current = ZoneSnapshot {
            zone,
            records: vec![mod_a, added_cname],
            captured_at_unix: 200,
        };

        let diff = baseline.diff(&current);
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.removed.len(), 1);
        assert_eq!(diff.modified.len(), 1);

        let plan = baseline.rollback_plan(&current);
        assert_eq!(plan.len(), 3);
        assert!(matches!(plan[0], RollbackOp::Delete { ref record_id, .. } if record_id == "r3"));
        assert!(matches!(plan[1], RollbackOp::Upsert(ref r) if r == &base_a));
        assert!(matches!(plan[2], RollbackOp::Upsert(ref r) if r == &base_txt));
    }
}
