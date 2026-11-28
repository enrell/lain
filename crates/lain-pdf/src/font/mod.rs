//! Fonts (D-304): the font program parsers (TrueType, CFF, Type 1), the
//! standard encodings and glyph list, CMaps, system font substitution
//! and the PDF font dictionary logic that ties them together.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontError(pub String);

impl std::fmt::Display for FontError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for FontError {}

pub mod cff;
pub mod cmap;
pub mod encoding;
pub mod glyphlist;
pub mod pdf;
pub mod system;
pub mod truetype;
pub mod type1;
