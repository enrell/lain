//! Image codecs (D-304): JPEG decode and encode, JPEG 2000, PNG encode, CCITT
//! fax and JBIG2, written in-tree.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecError(pub String);

impl std::fmt::Display for CodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for CodecError {}

pub mod ccitt;
pub mod jbig2;
pub mod jpeg;
pub mod jpx;
pub mod png;
