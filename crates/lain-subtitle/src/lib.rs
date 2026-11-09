//! Subtitle files (docs/slices/acquisition.md, A-28/A-29/A-34): sidecar
//! discovery next to a media file by name, and SRT, WebVTT and ASS/SSA
//! parsed into timed cues and written back as WebVTT, the one format
//! browsers play. Input is untrusted: size and cue counts are bounded
//! and nothing panics. Also the naming Lain writes sidecars under, the
//! OpenSubtitles moviehash, and the subtitles module's network
//! provider, `lain-subtitle-opensubtitles` (`lain.subtitle@1`, D-202).
//! A port of `internal/subtitle` and `internal/plugins/subtitles`.

mod format;
pub mod opensubtitles;
mod sidecar;

pub use format::{Cue, MAX_BYTES, MAX_CUES, ParseError, decode_text, parse, webvtt};
pub use opensubtitles::{CAP_SUBTITLE, OPENSUBTITLES_ID, OpenSubtitles};
pub use sidecar::{Found, discover, language, movie_hash, sidecar_path, tag};
