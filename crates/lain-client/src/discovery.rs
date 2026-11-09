//! Finding a server. [`Discovery`] yields candidates; the client then
//! probes one with [`crate::Client::health`] before using it. Shipped:
//! [`ManualDiscovery`] (a URL the user typed). mDNS on the LAN and the
//! web UI's QR pairing code are later implementations of the same trait
//! (they need crates; Q-045).

use crate::error::ClientError;
use crate::http::ServerUrl;

/// Where a candidate came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoverySource {
    Manual,
    Mdns,
    Pairing,
}

/// One server a [`Discovery`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerCandidate {
    /// The canonical URL ([`ServerUrl::canonical`]).
    pub url: String,
    /// A human name when the source has one (mDNS instance name).
    pub name: String,
    pub source: DiscoverySource,
}

/// A way to find servers. `discover` may block for a browse window
/// (mDNS), so async callers run it on a blocking thread.
pub trait Discovery: Send + Sync {
    fn discover(&self) -> Result<Vec<ServerCandidate>, ClientError>;
}

/// The URL the user entered (`host`, `host:port` or a full URL).
pub struct ManualDiscovery {
    url: ServerUrl,
}

impl ManualDiscovery {
    pub fn new(raw: &str) -> Result<Self, ClientError> {
        Ok(ManualDiscovery { url: ServerUrl::parse(raw)? })
    }
}

impl Discovery for ManualDiscovery {
    fn discover(&self) -> Result<Vec<ServerCandidate>, ClientError> {
        Ok(vec![ServerCandidate {
            url: self.url.canonical(),
            name: self.url.host.clone(),
            source: DiscoverySource::Manual,
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_yields_the_canonical_url() {
        let d = ManualDiscovery::new("lain.lan").unwrap();
        let c = d.discover().unwrap();
        assert_eq!(
            c,
            vec![ServerCandidate {
                url: "http://lain.lan:9360".into(),
                name: "lain.lan".into(),
                source: DiscoverySource::Manual
            }]
        );
        assert!(ManualDiscovery::new("ftp://x").is_err());
    }
}
