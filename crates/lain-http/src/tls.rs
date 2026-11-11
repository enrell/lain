//! The TLS setup on its own, for clients that drive their own
//! connection: `lain-client` reaches a remote lain server through it
//! (D-274), so the trust anchors and protocol versions are the ones of
//! every other outbound call (D-200).

use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::ServerName;
use tokio::net::TcpStream;

use crate::{Error, ErrorKind, conn};

/// A TLS stream over TCP, after the handshake.
pub type TlsStream = tokio_rustls::client::TlsStream<TcpStream>;

/// Starts TLS on connected sockets. Cheap to clone.
#[derive(Clone)]
pub struct Connector {
    cfg: Arc<rustls::ClientConfig>,
}

impl std::fmt::Debug for Connector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connector").finish_non_exhaustive()
    }
}

impl Connector {
    /// The webpki-roots trust anchors plus `extra_roots` (DER), for a
    /// server behind a private CA or a self-signed certificate.
    pub fn new(extra_roots: &[Vec<u8>]) -> Result<Connector, Error> {
        Ok(Connector { cfg: conn::tls_config(extra_roots)? })
    }

    /// The handshake with `host` (a DNS name or an IP address) over
    /// `tcp`, within `timeout`. Failures are [`ErrorKind::Tls`] or
    /// [`ErrorKind::Timeout`].
    pub async fn connect(&self, host: &str, tcp: TcpStream, timeout: Duration) -> Result<TlsStream, Error> {
        let name = ServerName::try_from(host.to_string())
            .map_err(|_| Error::new(ErrorKind::Tls, host, "not a valid TLS server name"))?;
        tokio::time::timeout(timeout, tokio_rustls::TlsConnector::from(self.cfg.clone()).connect(name, tcp))
            .await
            .map_err(|_| Error::new(ErrorKind::Timeout, host, "TLS handshake timed out"))?
            .map_err(|e| Error::new(ErrorKind::Tls, host, format!("TLS: {e}")))
    }
}
