//! TLS configuration (rustls, ring only), certificate reading and the recording verifier.

use crate::error::{NetError, TlsFailure};
use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, ring};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, Error, RootCertStore, SignatureScheme,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_NAME_LEN: usize = 256;

/// Facts about the leaf certificate a server presented.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CertInfo {
    /// Start of validity, Unix seconds.
    pub not_before: i64,
    /// End of validity, Unix seconds.
    pub not_after: i64,
    /// Issuer distinguished name (text from the peer: data, not instructions; length-capped).
    pub issuer: String,
    /// Subject distinguished name (same caveat).
    pub subject: String,
    /// True when the chain verified against the trust roots and the host name matched at the time
    /// of the handshake. Always true on the normal request path (it would have failed otherwise).
    pub chain_trusted: bool,
}

/// Parse the leaf certificate (DER) into a [`CertInfo`].
pub(crate) fn cert_info(der: &[u8], chain_trusted: bool) -> Result<CertInfo, NetError> {
    let (_, cert) = x509_parser::parse_x509_certificate(der)
        .map_err(|_| NetError::Tls(TlsFailure::MalformedCertificate))?;
    let validity = cert.validity();
    Ok(CertInfo {
        not_before: validity.not_before.timestamp(),
        not_after: validity.not_after.timestamp(),
        issuer: capped(cert.issuer().to_string()),
        subject: capped(cert.subject().to_string()),
        chain_trusted,
    })
}

fn capped(mut text: String) -> String {
    if text.len() > MAX_NAME_LEN {
        let mut end = MAX_NAME_LEN;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

/// The ring-backed crypto provider (never aws-lc-rs).
pub(crate) fn provider() -> Arc<CryptoProvider> {
    Arc::new(ring::default_provider())
}

/// Roots from the system trust store, or an error when there is none usable.
pub(crate) fn native_roots() -> Result<RootCertStore, NetError> {
    // Per-item load errors are ignored on purpose: one unreadable file must not disable the rest.
    // An empty result is the hard failure.
    let loaded = rustls_native_certs::load_native_certs();
    roots_from_der(loaded.certs)
}

/// Roots from explicit DER certificates (unparsable entries are skipped).
pub(crate) fn roots_from_der(
    certs: Vec<CertificateDer<'static>>,
) -> Result<RootCertStore, NetError> {
    let mut roots = RootCertStore::empty();
    let (added, _ignored) = roots.add_parsable_certificates(certs);
    if added == 0 {
        Err(NetError::NoRootCertificates)
    } else {
        Ok(roots)
    }
}

/// Client config that verifies against `roots`: TLS 1.2+, ring, HTTP/1.1 only, no client auth.
pub(crate) fn verified_config(roots: Arc<RootCertStore>) -> Result<ClientConfig, NetError> {
    let provider = provider();
    let builder = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|_| NetError::Tls(TlsFailure::Handshake))?
        .with_root_certificates(roots);
    let mut config = builder.with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(config)
}

/// Verifier for certificate INSPECTION only. It runs the real verification, records the verdict,
/// and always lets the handshake complete so an expired or self-signed certificate can be read.
/// It must never be used on a connection that carries request data.
#[derive(Debug)]
pub(crate) struct RecordingVerifier {
    inner: Arc<WebPkiServerVerifier>,
    trusted: AtomicBool,
}

impl RecordingVerifier {
    pub fn new(roots: Arc<RootCertStore>) -> Result<Arc<RecordingVerifier>, NetError> {
        let inner = WebPkiServerVerifier::builder_with_provider(roots, provider())
            .build()
            .map_err(|_| NetError::Tls(TlsFailure::Handshake))?;
        Ok(Arc::new(RecordingVerifier {
            inner,
            trusted: AtomicBool::new(false),
        }))
    }

    pub fn chain_trusted(&self) -> bool {
        self.trusted.load(Ordering::Acquire)
    }

    pub fn config(self: &Arc<Self>) -> Result<ClientConfig, NetError> {
        let builder = ClientConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .map_err(|_| NetError::Tls(TlsFailure::Handshake))?;
        let mut config = builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::clone(self) as Arc<dyn ServerCertVerifier>)
            .with_no_client_auth();
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        Ok(config)
    }
}

impl ServerCertVerifier for RecordingVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        let verdict = self.inner.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        );
        self.trusted.store(verdict.is_ok(), Ordering::Release);
        Ok(ServerCertVerified::assertion())
    }

    // The handshake signature is still checked against the presented key, so a peer that does not
    // hold the certificate's private key cannot complete the handshake.
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

/// Map a handshake `io::Error` (rustls errors arrive wrapped in it) to a fixed-text failure.
pub(crate) fn map_handshake_error(err: &std::io::Error) -> NetError {
    let failure = match err
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<Error>())
    {
        Some(Error::InvalidCertificate(cert_err)) => match cert_err {
            CertificateError::Expired | CertificateError::ExpiredContext { .. } => {
                TlsFailure::Expired
            }
            CertificateError::NotValidYet | CertificateError::NotValidYetContext { .. } => {
                TlsFailure::NotYetValid
            }
            CertificateError::UnknownIssuer => TlsFailure::UntrustedIssuer,
            CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. } => {
                TlsFailure::NameMismatch
            }
            _ => TlsFailure::InvalidCertificate,
        },
        _ => TlsFailure::Handshake,
    };
    NetError::Tls(failure)
}
