use crate::addr::is_blocked;
use crate::error::{NetError, TlsFailure};
use crate::target::Target;
use crate::tls::{self, CertInfo, RecordingVerifier};
use bytes::{Buf, Bytes};
use http::header::{ACCEPT, ACCEPT_ENCODING, CONNECTION, CONTENT_TYPE, HOST, LOCATION, USER_AGENT};
use http::{Method, Request};
use http_body_util::{BodyExt, Full};
use hyper::client::conn::http1;
use hyper_util::rt::TokioIo;
use rustls::ClientConfig;
use rustls::RootCertStore;
use rustls::pki_types::{CertificateDer, ServerName};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::{TcpStream, lookup_host};
use tokio::task::JoinHandle;
use tokio_rustls::TlsConnector;

/// Cap on the response head (status line + headers).
const MAX_HEAD_BYTES: usize = 64 * 1024;
/// Resolved addresses tried per connect.
const MAX_ADDRS: usize = 8;
/// Fixed, generic user agent.
const FIXED_USER_AGENT: &str = "monitor/1";

/// Limits and switches for one call. The defaults are the safe ones.
#[derive(Debug, Clone)]
pub struct Policy {
    /// Total time budget covering DNS, connect, TLS and the whole response (not per read).
    pub timeout: Duration,
    /// Redirects followed by `get` (default 3). `post_json` never follows redirects.
    pub max_redirects: u8,
    /// Body bytes kept (default 256 KiB). Reading stops one byte past it; the body is then
    /// truncated and `Response::body_truncated` is set.
    pub max_body_bytes: usize,
    /// Refuse loopback, private, link-local, CGNAT, ULA, multicast and other special-purpose
    /// destinations, checked on the address actually connected to (default true).
    pub block_private_addresses: bool,
    /// Allow `http://` URLs and redirects to them (default false). An `https` to `http` redirect
    /// is refused regardless.
    pub allow_http: bool,
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            timeout: Duration::from_secs(10),
            max_redirects: 3,
            max_body_bytes: 256 * 1024,
            block_private_addresses: true,
            allow_http: false,
        }
    }
}

/// Result of a completed HTTP exchange. The body is data from the server, never instructions.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
    /// True when the body was longer than `Policy::max_body_bytes` and was cut.
    pub body_truncated: bool,
    /// Wall time of the whole call, redirects included.
    pub elapsed: Duration,
    /// Leaf certificate of the final hop when it used HTTPS.
    pub tls: Option<CertInfo>,
    /// URL of the final hop (no credentials by construction).
    pub final_url: String,
}

/// Outbound client. Cheap to clone-by-reference; build one and share it. Opens a fresh connection
/// per call (no pooling, no proxy settings, no cookies, no automatic decompression).
#[derive(Debug)]
pub struct Client {
    roots: Arc<RootCertStore>,
    config: Arc<ClientConfig>,
}

struct AbortOnDrop(JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

enum Hop {
    Done(Response),
    Redirect(String),
}

impl Client {
    /// Build a client that trusts the operating system's root certificates.
    ///
    /// Fails with [`NetError::NoRootCertificates`] when the system store yields none (for example
    /// a minimal container without `ca-certificates`); use [`Client::with_root_certificates`] then.
    pub fn new() -> Result<Client, NetError> {
        Client::from_roots(tls::native_roots()?)
    }

    /// Build a client that trusts only the given DER-encoded root certificates (own CA bundle).
    /// Entries that do not parse are skipped; none usable gives [`NetError::NoRootCertificates`].
    pub fn with_root_certificates<I>(certs: I) -> Result<Client, NetError>
    where
        I: IntoIterator<Item = Vec<u8>>,
    {
        let ders = certs.into_iter().map(CertificateDer::from).collect();
        Client::from_roots(tls::roots_from_der(ders)?)
    }

    fn from_roots(roots: RootCertStore) -> Result<Client, NetError> {
        let roots = Arc::new(roots);
        let config = Arc::new(tls::verified_config(Arc::clone(&roots))?);
        Ok(Client { roots, config })
    }

    /// `GET` with redirect following per `policy`.
    pub async fn get(&self, url: &str, policy: &Policy) -> Result<Response, NetError> {
        self.run(Method::GET, url, &[], policy).await
    }

    /// `POST` with `Content-Type: application/json` (for the Telegram Bot API). Redirects are not
    /// followed: a 3xx comes back as the response.
    pub async fn post_json(
        &self,
        url: &str,
        body: &[u8],
        policy: &Policy,
    ) -> Result<Response, NetError> {
        self.run(Method::POST, url, body, policy).await
    }

    /// Time to establish a TCP connection (DNS included) to `host:port`; nothing is sent.
    pub async fn tcp_connect(
        &self,
        host: &str,
        port: u16,
        policy: &Policy,
    ) -> Result<Duration, NetError> {
        let started = Instant::now();
        let work = async {
            connect_checked(host, port, policy).await?;
            Ok(started.elapsed())
        };
        within(policy, work).await
    }

    /// Read the leaf certificate of `host:port` without sending any data.
    ///
    /// Works for expired, self-signed and otherwise untrusted servers: the verdict of the real
    /// verification is returned in `chain_trusted` instead of failing the handshake. The
    /// permissive handshake used for this is confined to this method; the connection is dropped
    /// right after the certificate is read.
    pub async fn tls_certificate(
        &self,
        host: &str,
        port: u16,
        policy: &Policy,
    ) -> Result<CertInfo, NetError> {
        let work = async {
            let verifier = RecordingVerifier::new(Arc::clone(&self.roots))?;
            let connector = TlsConnector::from(Arc::new(verifier.config()?));
            let name = server_name(host)?;
            let tcp = connect_checked(host, port, policy).await?;
            let stream = connector
                .connect(name, tcp)
                .await
                .map_err(|e| tls::map_handshake_error(&e))?;
            let leaf = leaf_der(stream.get_ref().1.peer_certificates())?;
            tls::cert_info(leaf, verifier.chain_trusted())
        };
        within(policy, work).await
    }

    async fn run(
        &self,
        method: Method,
        url: &str,
        body: &[u8],
        policy: &Policy,
    ) -> Result<Response, NetError> {
        let started = Instant::now();
        let work = async {
            let follow = method == Method::GET;
            let mut target = Target::parse(url)?;
            let mut redirects: u8 = 0;
            loop {
                if !target.https && !policy.allow_http {
                    return Err(NetError::SchemeNotAllowed);
                }
                match self.hop(&method, &target, body, policy, follow).await? {
                    Hop::Done(mut response) => {
                        response.elapsed = started.elapsed();
                        return Ok(response);
                    }
                    Hop::Redirect(location) => {
                        if redirects >= policy.max_redirects {
                            return Err(NetError::TooManyRedirects);
                        }
                        redirects += 1;
                        let next = target.join(&location)?;
                        if target.https && !next.https {
                            return Err(NetError::SchemeNotAllowed);
                        }
                        target = next;
                    }
                }
            }
        };
        within(policy, work).await
    }

    /// One connection, one request. Address checks happen inside `connect_checked`, on every hop.
    async fn hop(
        &self,
        method: &Method,
        target: &Target,
        body: &[u8],
        policy: &Policy,
        follow: bool,
    ) -> Result<Hop, NetError> {
        let tcp = connect_checked(&target.host, target.port, policy).await?;
        if target.https {
            let name = server_name(&target.host)?;
            let stream = TlsConnector::from(Arc::clone(&self.config))
                .connect(name, tcp)
                .await
                .map_err(|e| tls::map_handshake_error(&e))?;
            let info = tls::cert_info(leaf_der(stream.get_ref().1.peer_certificates())?, true)?;
            exchange(
                TokioIo::new(stream),
                method,
                target,
                body,
                policy,
                follow,
                Some(info),
            )
            .await
        } else {
            exchange(
                TokioIo::new(tcp),
                method,
                target,
                body,
                policy,
                follow,
                None,
            )
            .await
        }
    }
}

fn leaf_der<'a>(chain: Option<&'a [CertificateDer<'a>]>) -> Result<&'a [u8], NetError> {
    chain
        .and_then(|certs| certs.first())
        .map(|cert| cert.as_ref())
        .ok_or(NetError::Tls(TlsFailure::MalformedCertificate))
}

fn server_name(host: &str) -> Result<ServerName<'static>, NetError> {
    ServerName::try_from(host.to_owned()).map_err(|_| NetError::InvalidUrl)
}

/// Apply the total time budget to a whole operation.
async fn within<T>(
    policy: &Policy,
    work: impl Future<Output = Result<T, NetError>>,
) -> Result<T, NetError> {
    tokio::time::timeout(policy.timeout, work)
        .await
        .map_err(|_| NetError::Timeout)?
}

/// Resolve `host` ourselves and connect to a vetted `SocketAddr`. The block check is made on the
/// very address handed to `connect`, so a resolver that answers differently later cannot swap it.
async fn connect_checked(host: &str, port: u16, policy: &Policy) -> Result<TcpStream, NetError> {
    let addrs: Vec<SocketAddr> = lookup_host((host, port))
        .await
        .map_err(|_| NetError::Dns)?
        .take(MAX_ADDRS)
        .collect();
    if addrs.is_empty() {
        return Err(NetError::Dns);
    }
    let mut attempted = false;
    for addr in addrs {
        if policy.block_private_addresses && is_blocked(addr.ip()) {
            continue;
        }
        attempted = true;
        if let Ok(stream) = TcpStream::connect(addr).await {
            // Latency tuning only; failure is irrelevant.
            let _ = stream.set_nodelay(true);
            return Ok(stream);
        }
    }
    Err(if attempted {
        NetError::Connect
    } else {
        NetError::BlockedAddress
    })
}

async fn exchange<I>(
    io: I,
    method: &Method,
    target: &Target,
    body: &[u8],
    policy: &Policy,
    follow: bool,
    tls: Option<CertInfo>,
) -> Result<Hop, NetError>
where
    I: hyper::rt::Read + hyper::rt::Write + Unpin + Send + 'static,
{
    let mut builder = http1::Builder::new();
    builder.max_buf_size(MAX_HEAD_BYTES);
    let (mut sender, connection) = builder
        .handshake::<_, Full<Bytes>>(io)
        .await
        .map_err(|_| NetError::Protocol)?;
    // The driver is stopped when this function ends or the total timeout drops the future.
    let _driver = AbortOnDrop(tokio::spawn(async move {
        let _ = connection.await;
    }));

    let mut request = Request::builder()
        .method(method.clone())
        .uri(target.path.as_str())
        .header(HOST, target.authority())
        .header(USER_AGENT, FIXED_USER_AGENT)
        .header(ACCEPT, "*/*")
        .header(ACCEPT_ENCODING, "identity")
        .header(CONNECTION, "close");
    if method == Method::POST {
        request = request.header(CONTENT_TYPE, "application/json");
    }
    let request = request
        .body(Full::new(Bytes::copy_from_slice(body)))
        .map_err(|_| NetError::InvalidUrl)?;

    let response = sender
        .send_request(request)
        .await
        .map_err(|_| NetError::Protocol)?;
    let status = response.status();
    if follow && matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308) {
        let location = response
            .headers()
            .get(LOCATION)
            .and_then(|v| v.to_str().ok());
        if let Some(location) = location {
            return Ok(Hop::Redirect(location.to_owned()));
        }
    }

    let (kept, truncated) = read_capped(response.into_body(), policy.max_body_bytes).await?;
    Ok(Hop::Done(Response {
        status: status.as_u16(),
        body: kept,
        body_truncated: truncated,
        elapsed: Duration::ZERO,
        tls,
        final_url: target.url(),
    }))
}

/// Read at most `max + 1` bytes, keep `max`, and stop pulling from the connection.
async fn read_capped<B>(mut body: B, max: usize) -> Result<(Vec<u8>, bool), NetError>
where
    B: hyper::body::Body + Unpin,
{
    let limit = max.saturating_add(1);
    let mut out: Vec<u8> = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|_| NetError::Protocol)?;
        if let Ok(mut data) = frame.into_data() {
            while data.has_remaining() && out.len() < limit {
                let chunk = data.chunk();
                let take = chunk.len().min(limit - out.len());
                out.extend_from_slice(chunk.get(..take).unwrap_or_default());
                data.advance(take);
            }
            if out.len() >= limit {
                break;
            }
        }
    }
    let truncated = out.len() > max;
    out.truncate(max);
    Ok((out, truncated))
}
