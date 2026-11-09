//! Connections: TCP to each resolved address in turn, TLS through
//! rustls for `https`, hyper's HTTP/1.1 client handshake, and a small
//! idle pool per `host:port` (Go's defaults: two idle connections per
//! host, dropped after 90 s).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use http_body_util::Full;
use hyper::body::Bytes;
use hyper::client::conn::http1::SendRequest;
use hyper_util::rt::TokioIo;
use rustls::pki_types::ServerName;

use crate::url::Url;
use crate::{Error, ErrorKind};

const MAX_IDLE_PER_HOST: usize = 2;
const IDLE_TIMEOUT: Duration = Duration::from_secs(90);

pub(crate) type Sender = SendRequest<Full<Bytes>>;

/// The client TLS setup: the ring provider, TLS 1.2 and 1.3, the
/// webpki-roots trust anchors plus any extra (DER) roots, ALPN
/// `http/1.1`.
pub(crate) fn tls_config(extra_roots: &[Vec<u8>]) -> Result<Arc<rustls::ClientConfig>, Error> {
    let mut roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
    for der in extra_roots {
        roots
            .add(rustls::pki_types::CertificateDer::from(der.clone()))
            .map_err(|e| Error::new(ErrorKind::Tls, "", format!("extra root: {e}")))?;
    }
    let mut cfg = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::new(ErrorKind::Tls, "", e.to_string()))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(cfg))
}

/// Idle connections by (tls, `host:port`), with when each went idle.
type Idle = HashMap<(bool, String), Vec<(Sender, Instant)>>;

#[derive(Default)]
pub(crate) struct Pool {
    idle: Mutex<Idle>,
}

impl Pool {
    /// A ready idle connection to this host, if one is left.
    pub(crate) fn take(&self, url: &Url) -> Option<Sender> {
        let mut g = self.idle.lock().unwrap_or_else(|e| e.into_inner());
        let list = g.get_mut(&(url.tls, url.host_key()))?;
        while let Some((s, at)) = list.pop() {
            if !s.is_closed() && s.is_ready() && at.elapsed() < IDLE_TIMEOUT {
                return Some(s);
            }
        }
        None
    }

    pub(crate) fn put(&self, url: &Url, s: Sender) {
        if s.is_closed() {
            return;
        }
        let mut g = self.idle.lock().unwrap_or_else(|e| e.into_inner());
        let list = g.entry((url.tls, url.host_key())).or_default();
        list.retain(|(s, at)| !s.is_closed() && at.elapsed() < IDLE_TIMEOUT);
        if list.len() < MAX_IDLE_PER_HOST {
            list.push((s, Instant::now()));
        }
    }
}

fn connect_err(url: &Url, e: impl std::fmt::Display) -> Error {
    Error::new(ErrorKind::Connect, &url.host, e.to_string())
}

/// Opens a new connection and runs its hyper task on the current
/// runtime.
pub(crate) async fn open(
    url: &Url,
    tls: &Arc<rustls::ClientConfig>,
    connect_timeout: Duration,
) -> Result<Sender, Error> {
    let tcp = tokio::time::timeout(connect_timeout, tcp(url))
        .await
        .map_err(|_| Error::new(ErrorKind::Timeout, &url.host, "connect timed out"))??;
    let _ = tcp.set_nodelay(true);
    if url.tls {
        let name = ServerName::try_from(url.host.clone())
            .map_err(|_| Error::new(ErrorKind::Tls, &url.host, "not a valid TLS server name"))?;
        let stream =
            tokio::time::timeout(connect_timeout, tokio_rustls::TlsConnector::from(tls.clone()).connect(name, tcp))
                .await
                .map_err(|_| Error::new(ErrorKind::Timeout, &url.host, "TLS handshake timed out"))?
                .map_err(|e| Error::new(ErrorKind::Tls, &url.host, format!("TLS: {e}")))?;
        handshake(url, TokioIo::new(stream)).await
    } else {
        handshake(url, TokioIo::new(tcp)).await
    }
}

async fn tcp(url: &Url) -> Result<tokio::net::TcpStream, Error> {
    let addrs = tokio::net::lookup_host((url.host.as_str(), url.port)).await.map_err(|e| connect_err(url, e))?;
    let mut last = None;
    for addr in addrs {
        match tokio::net::TcpStream::connect(addr).await {
            Ok(s) => return Ok(s),
            Err(e) => last = Some(e),
        }
    }
    Err(match last {
        Some(e) => connect_err(url, e),
        None => connect_err(url, "no address"),
    })
}

async fn handshake<T>(url: &Url, io: TokioIo<T>) -> Result<Sender, Error>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (sender, conn) = hyper::client::conn::http1::handshake::<_, Full<Bytes>>(io)
        .await
        .map_err(|e| Error::new(ErrorKind::Protocol, &url.host, e.to_string()))?;
    tokio::spawn(async move {
        // Ends when the server closes or the last sender is dropped.
        let _ = conn.await;
    });
    Ok(sender)
}
