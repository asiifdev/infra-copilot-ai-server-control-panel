//! The network seam: the provider talks to a [`Transport`], tests talk to a fake, production
//! talks to [`NetTransport`] over the hardened `net` client.

use bytes::Bytes;
use net::{BodyStream, Client, NetError, Policy, StreamLimits};
use std::fmt;
use std::future::Future;

/// Why a request failed before or while reading the answer. Fixed text only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportError {
    /// A time limit passed.
    Timeout,
    /// DNS or TCP connect failed.
    Connect,
    /// TLS failed.
    Tls,
    /// The destination is blocked by the network policy (private address, plain HTTP).
    Blocked,
    /// The answer was larger than allowed.
    TooLarge,
    /// The peer did not speak HTTP.
    Protocol,
    /// The request itself was invalid (URL or header).
    Invalid,
}

impl TransportError {
    /// Whether a retry can help.
    #[must_use]
    pub const fn is_transient(self) -> bool {
        matches!(self, Self::Timeout | Self::Connect | Self::Protocol)
    }
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Timeout => "timeout",
            Self::Connect => "connect",
            Self::Tls => "tls",
            Self::Blocked => "blocked",
            Self::TooLarge => "too_large",
            Self::Protocol => "protocol",
            Self::Invalid => "invalid",
        })
    }
}

impl From<NetError> for TransportError {
    fn from(error: NetError) -> Self {
        match error {
            NetError::Timeout => Self::Timeout,
            NetError::Dns | NetError::Connect => Self::Connect,
            NetError::Tls(_) | NetError::NoRootCertificates => Self::Tls,
            NetError::BlockedAddress | NetError::SchemeNotAllowed => Self::Blocked,
            NetError::ResponseTooLarge => Self::TooLarge,
            NetError::InvalidUrl
            | NetError::UserInfoNotAllowed
            | NetError::InvalidHeader
            | NetError::TooManyRedirects
            | NetError::InvalidRedirect => Self::Invalid,
            _ => Self::Protocol,
        }
    }
}

/// A response body read piece by piece.
pub trait ByteStream: Send {
    /// HTTP status.
    fn status(&self) -> u16;

    /// The `Retry-After` header as whole seconds, when present and numeric.
    fn retry_after_secs(&self) -> Option<u32>;

    /// The next piece, `None` at the end.
    fn next_chunk(&mut self) -> impl Future<Output = Result<Option<Bytes>, TransportError>> + Send;
}

/// Sends one JSON POST and hands back the response as a stream.
pub trait Transport: Send + Sync {
    /// The stream type.
    type Stream: ByteStream;

    /// Sends `body` to `url` with the given headers.
    fn post(
        &self,
        url: &str,
        body: &[u8],
        headers: &[(&str, &str)],
    ) -> impl Future<Output = Result<Self::Stream, TransportError>> + Send;
}

/// The production transport over the `net` client.
pub struct NetTransport {
    client: Client,
    policy: Policy,
    limits: StreamLimits,
}

impl NetTransport {
    /// A transport with explicit network policy and stream limits.
    #[must_use]
    pub fn new(client: Client, policy: Policy, limits: StreamLimits) -> Self {
        Self {
            client,
            policy,
            limits,
        }
    }
}

impl Transport for NetTransport {
    type Stream = NetStream;

    async fn post(
        &self,
        url: &str,
        body: &[u8],
        headers: &[(&str, &str)],
    ) -> Result<NetStream, TransportError> {
        let stream = self
            .client
            .post_json_stream(url, body, headers, &self.policy, self.limits)
            .await?;
        Ok(NetStream(stream))
    }
}

/// A `net` body stream seen through [`ByteStream`].
pub struct NetStream(BodyStream);

impl ByteStream for NetStream {
    fn status(&self) -> u16 {
        self.0.status()
    }

    fn retry_after_secs(&self) -> Option<u32> {
        self.0
            .header("retry-after")
            .and_then(|v| v.trim().parse::<u32>().ok())
            .map(|s| s.min(3600))
    }

    async fn next_chunk(&mut self) -> Result<Option<Bytes>, TransportError> {
        Ok(self.0.next_chunk().await?)
    }
}
