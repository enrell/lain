//! Lain's PDF library (D-304), written from scratch: the file structure
//! and its repair, the standard security handler, stream filters and
//! image codecs, font programs, the content-stream interpreter and an
//! anti-aliased rasterizer that turns a page into a picture. No C, no
//! crate beyond `miniz_oxide` (DEFLATE) and `sha2`.

pub mod codec;
pub mod color;
pub mod crypt;
pub mod document;
pub mod error;
pub mod filter;
pub mod font;
pub mod function;
pub mod geom;
pub mod image;
pub mod lexer;
pub mod object;
pub mod raster;
pub mod render;
pub mod shading;
pub mod stroke;

pub use document::{Document, Info, OutlineItem, Page};
pub use error::{ErrorKind, PdfError, Result};
