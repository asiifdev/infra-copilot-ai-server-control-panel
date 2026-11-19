use std::fmt;

/// Why a TLS handshake or certificate check failed. Carries no peer-supplied text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TlsFailure {
    /// The handshake failed for a reason other than the certificate (protocol, signature, I/O).
    Handshake,
    /// The certificate chain does not lead to a trusted root (includes self-signed).
    UntrustedIssuer,
    /// The certificate has expired.
    Expired,
    /// The certificate is not valid yet.
    NotYetValid,
    /// The certificate is not valid for the requested host name.
    NameMismatch,
    /// Another certificate problem (bad signature, revoked, unsupported, ...).
    InvalidCertificate,
    /// The server sent no certificate, or one that could not be parsed.
    MalformedCertificate,
}

/// Errors of this crate. The `Display` text is fixed per variant: it never contains the URL, the
/// host, response content or any text received from the peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NetError {
    /// The URL cannot be parsed, is too long, or has no host.
    InvalidUrl,
    /// The URL contains credentials (`user:pass@host`); they are never accepted.
    UserInfoNotAllowed,
    /// Scheme other than `https` (or `http` when `allow_http`), or an `https` to `http` redirect.
    SchemeNotAllowed,
    /// Every resolved address is in a blocked range (see `Policy::block_private_addresses`).
    BlockedAddress,
    /// The total time budget (DNS, connect, TLS, response) ran out.
    Timeout,
    /// More redirects than `Policy::max_redirects`.
    TooManyRedirects,
    /// A redirect without a usable `Location`.
    InvalidRedirect,
    /// Name resolution failed or returned no address.
    Dns,
    /// TCP connect failed.
    Connect,
    /// TLS failure.
    Tls(TlsFailure),
    /// The peer did not speak valid HTTP/1.1 (or exceeded the header size cap).
    Protocol,
    /// The system trust store has no usable root certificate.
    NoRootCertificates,
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::InvalidUrl => "invalid URL",
            Self::UserInfoNotAllowed => "credentials in URL are not allowed",
            Self::SchemeNotAllowed => "URL scheme not allowed",
            Self::BlockedAddress => "destination address is blocked",
            Self::Timeout => "operation timed out",
            Self::TooManyRedirects => "too many redirects",
            Self::InvalidRedirect => "invalid redirect",
            Self::Dns => "name resolution failed",
            Self::Connect => "connection failed",
            Self::Tls(TlsFailure::Handshake) => "TLS handshake failed",
            Self::Tls(TlsFailure::UntrustedIssuer) => "TLS certificate issuer is not trusted",
            Self::Tls(TlsFailure::Expired) => "TLS certificate has expired",
            Self::Tls(TlsFailure::NotYetValid) => "TLS certificate is not valid yet",
            Self::Tls(TlsFailure::NameMismatch) => "TLS certificate does not match the host",
            Self::Tls(TlsFailure::InvalidCertificate) => "TLS certificate is invalid",
            Self::Tls(TlsFailure::MalformedCertificate) => {
                "TLS certificate is missing or malformed"
            }
            Self::Protocol => "invalid HTTP response",
            Self::NoRootCertificates => "no root certificates available in the system trust store",
        };
        f.write_str(text)
    }
}

impl std::error::Error for NetError {}
