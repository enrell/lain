//! Subtitle files (docs/slices/acquisition.md, A-28/A-29/A-34): sidecar
//! discovery next to a media file by name, and SRT, WebVTT and ASS/SSA
//! parsed into timed cues and written back as WebVTT, the one format
//! browsers play. Input is untrusted: size and cue counts are bounded
//! and nothing panics. A port of `internal/subtitle` (discovery and
//! formats; the subtitles module adds naming and hashing when it is
//! ported).

mod format;
mod sidecar;

pub use format::{Cue, MAX_BYTES, MAX_CUES, ParseError, decode_text, parse, webvtt};
pub use sidecar::{Found, discover, language};
