use std::fmt;

/// Engine errors. Display texts follow Go's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Bencode syntax (`bencode: syntax error at N: ...`).
    Syntax(String),
    /// Metainfo or magnet validation.
    Meta(String),
    /// Peer protocol violation.
    Wire(String),
    /// Tracker failure, including a "failure reason".
    Tracker(String),
    /// The torrent is already added.
    Exists,
    /// The client is closed.
    Closed,
    /// A bad [`crate::Spec`] or engine configuration.
    Spec(String),
    /// Disk or socket failure.
    Io(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Syntax(m) => write!(f, "bencode: syntax error {m}"),
            Error::Meta(m) => write!(f, "torrent: bad metainfo: {m}"),
            Error::Wire(m) => write!(f, "torrent: wire: {m}"),
            Error::Tracker(m) => write!(f, "torrent: tracker: {m}"),
            Error::Exists => f.write_str("torrent: already added"),
            Error::Closed => f.write_str("torrent: client closed"),
            Error::Spec(m) => write!(f, "torrent: {m}"),
            Error::Io(m) => write!(f, "torrent: {m}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e.to_string())
    }
}

pub(crate) fn meta(m: impl Into<String>) -> Error {
    Error::Meta(m.into())
}

pub(crate) fn wire(m: impl Into<String>) -> Error {
    Error::Wire(m.into())
}

pub(crate) fn tracker(m: impl Into<String>) -> Error {
    Error::Tracker(m.into())
}
