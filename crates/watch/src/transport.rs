//! `notifier::HttpTransport` on top of `net::Client::post_json`.
//!
//! The request URL contains the bot token. It is passed straight to `net`, never stored, never
//! formatted, and no error returned from here carries it (`NetError` and `TransportError` are
//! closed enums without text).

use net::{Client, NetError, Policy};
use notifier::{HttpTransport, TransportError};
use std::sync::Arc;
use std::time::Duration;

/// Timeout for one Bot API call.
const TIMEOUT: Duration = Duration::from_secs(10);
/// Bot API replies are tiny; anything longer is cut off.
const MAX_BODY_BYTES: usize = 16 * 1024;

/// Sends Telegram requests through the hardened client. Private addresses stay blocked.
#[derive(Debug, Clone)]
pub struct NetTransport {
    client: Arc<Client>,
    policy: Policy,
}

impl NetTransport {
    /// HTTPS only, no redirects, 10 s, 16 KiB reply cap, private addresses blocked.
    pub fn new(client: Arc<Client>) -> Self {
        NetTransport {
            client,
            policy: Policy {
                timeout: TIMEOUT,
                max_redirects: 0,
                max_body_bytes: MAX_BODY_BYTES,
                block_private_addresses: true,
                allow_http: false,
            },
        }
    }

    /// Test-only: a policy that can reach a local server.
    #[cfg(test)]
    fn with_policy(client: Arc<Client>, policy: Policy) -> Self {
        NetTransport { client, policy }
    }
}

/// Closed mapping of transport failures. Everything unlisted is `Other`.
pub fn map_transport_error(err: NetError) -> TransportError {
    match err {
        NetError::Timeout => TransportError::Timeout,
        NetError::Dns | NetError::Connect | NetError::BlockedAddress => TransportError::Connect,
        NetError::Tls(_) | NetError::NoRootCertificates => TransportError::Tls,
        _ => TransportError::Other,
    }
}

impl HttpTransport for NetTransport {
    async fn post_json(&self, url: &str, body: &[u8]) -> Result<(u16, Vec<u8>), TransportError> {
        match self.client.post_json(url, body, &self.policy).await {
            Ok(resp) => Ok((resp.status, resp.body)),
            Err(e) => Err(map_transport_error(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use net::TlsFailure;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn fake_token() -> String {
        // Built at run time so no literal token exists in the source.
        format!("{}:{}", 4_242_424_242_u64, "ZZtokenZZ".repeat(3))
    }

    type R = Result<(), Box<dyn std::error::Error>>;

    /// A client whose only trust root is a throwaway certificate (no system store needed).
    fn test_client() -> Result<Arc<Client>, Box<dyn std::error::Error>> {
        let key = rcgen::KeyPair::generate()?;
        let cert =
            rcgen::CertificateParams::new(vec!["localhost".to_owned()])?.self_signed(&key)?;
        Ok(Arc::new(Client::with_root_certificates([cert
            .der()
            .to_vec()])?))
    }

    #[test]
    fn every_net_error_maps_to_a_transport_class() {
        let table = [
            (NetError::Timeout, TransportError::Timeout),
            (NetError::Dns, TransportError::Connect),
            (NetError::Connect, TransportError::Connect),
            (NetError::BlockedAddress, TransportError::Connect),
            (NetError::Tls(TlsFailure::Handshake), TransportError::Tls),
            (NetError::NoRootCertificates, TransportError::Tls),
            (NetError::InvalidUrl, TransportError::Other),
            (NetError::UserInfoNotAllowed, TransportError::Other),
            (NetError::SchemeNotAllowed, TransportError::Other),
            (NetError::TooManyRedirects, TransportError::Other),
            (NetError::InvalidRedirect, TransportError::Other),
            (NetError::Protocol, TransportError::Other),
        ];
        for (err, want) in table {
            assert_eq!(map_transport_error(err), want, "{err:?}");
        }
    }

    #[tokio::test]
    async fn default_policy_blocks_loopback_without_leaking_the_url() -> R {
        let transport = NetTransport::new(test_client()?);
        let token = fake_token();
        let url = format!("https://127.0.0.1:9/bot{token}/sendMessage");
        let result = transport.post_json(&url, b"{}").await;
        assert_eq!(result, Err(TransportError::Connect));
        assert!(!format!("{result:?} {transport:?}").contains(&token));
        Ok(())
    }

    #[tokio::test]
    async fn posts_json_and_returns_status_and_body() -> R {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        tokio::spawn(async move {
            if let Ok((mut s, _)) = listener.accept().await {
                let mut buf = vec![0u8; 4096];
                let _ = s.read(&mut buf).await;
                let _ = s
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    )
                    .await;
            }
        });
        let client = test_client()?;
        let policy = Policy {
            block_private_addresses: false,
            allow_http: true,
            ..Policy::default()
        };
        let transport = NetTransport::with_policy(client, policy);
        let got = transport
            .post_json(
                &format!("http://127.0.0.1:{port}/bot{}/x", fake_token()),
                b"{}",
            )
            .await;
        assert_eq!(got, Ok((200, b"ok".to_vec())));
        Ok(())
    }
}
