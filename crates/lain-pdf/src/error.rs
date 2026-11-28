//! The library's typed error: a stable kind for callers to map to their
//! own codes, and a message for logs.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// The file is not a PDF, or too damaged to read.
    Invalid,
    /// The document needs a password other than the empty one.
    Encrypted,
    /// A feature this library does not implement.
    Unsupported,
    /// Over a size or work cap.
    TooLarge,
    /// A page index past the last page.
    NoSuchPage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfError {
    pub kind: ErrorKind,
    pub message: String,
}

pub type Result<T> = std::result::Result<T, PdfError>;

impl PdfError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> PdfError {
        PdfError { kind, message: message.into() }
    }

    pub fn invalid(m: impl Into<String>) -> PdfError {
        PdfError::new(ErrorKind::Invalid, m)
    }

    pub fn unsupported(m: impl Into<String>) -> PdfError {
        PdfError::new(ErrorKind::Unsupported, m)
    }

    pub fn too_large(m: impl Into<String>) -> PdfError {
        PdfError::new(ErrorKind::TooLarge, format!("{} over the limit", m.into()))
    }

    pub fn encrypted(m: impl Into<String>) -> PdfError {
        PdfError::new(ErrorKind::Encrypted, m)
    }
}

impl std::fmt::Display for PdfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "pdf: {}", self.message)
    }
}

impl std::error::Error for PdfError {}
