//! `lain-thumbnail-ffmpeg` (`lain.transform.thumbnail@1`): one still
//! frame per (file identity, time, width), cached on disk. It returns
//! a path, never bytes; the gateway streams the file.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use lain_core::Error;
use lain_probe::exec;
use serde::{Deserialize, Serialize};

pub const ID: &str = "lain-thumbnail-ffmpeg";
pub const CAP: &str = "lain.transform.thumbnail@1";

/// Concurrent ffmpeg processes: thumbnails are decoration and must not
/// starve playback or a scan.
const MAX_PARALLEL: usize = 2;
const TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub file_path: String,
    #[serde(default)]
    pub time_sec: f64,
    #[serde(default)]
    pub width: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Thumbnail {
    pub path: String,
    pub width: i64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cached: bool,
}

/// A comic archive: its cover is the read module's first page, scaled
/// by ffmpeg (Go's `extractCover`).
fn comic(path: &str) -> bool {
    !lain_read::archive::format(Path::new(path)).is_empty()
}

/// A still that came with the file, by the conventions downloaders and
/// media managers use: `<name>-poster.*` or `<name>-thumb.*` next to
/// it, or `.metadata/thumbs/thumbnail.jpg` (`banner.jpg`) in its
/// folder. A non-empty regular file, never a link out of the folder.
pub fn local_cover(media_file: &Path) -> Option<PathBuf> {
    let dir = media_file.parent()?;
    let stem = media_file.file_stem()?.to_string_lossy();
    let mut names: Vec<PathBuf> = Vec::new();
    for suffix in ["-poster", "-thumb"] {
        for ext in ["jpg", "jpeg", "png", "webp"] {
            names.push(dir.join(format!("{stem}{suffix}.{ext}")));
        }
    }
    let thumbs = dir.join(".metadata").join("thumbs");
    names.push(thumbs.join("thumbnail.jpg"));
    names.push(thumbs.join("banner.jpg"));
    names.into_iter().find(|p| std::fs::symlink_metadata(p).is_ok_and(|m| m.is_file() && m.len() > 0))
}

/// The content type of a still by its first bytes: a downloader may
/// keep the `.jpg` name whatever the site served.
pub fn image_type(path: &Path) -> &'static str {
    use std::io::Read;
    let mut head = [0u8; 12];
    let n = std::fs::File::open(path).and_then(|mut f| f.read(&mut head)).unwrap_or(0);
    match &head[..n] {
        [0x89, b'P', b'N', b'G', ..] => "image/png",
        [b'G', b'I', b'F', b'8', ..] => "image/gif",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P'] => "image/webp",
        _ => "image/jpeg",
    }
}

pub struct Generator {
    dir: PathBuf,
    slots: Mutex<usize>,
    freed: Condvar,
    init_err: Option<String>,
}

impl Generator {
    pub fn new(dir: &Path) -> Generator {
        let init_err =
            lain_store::ensure_private_dir(dir).err().map(|e| format!("thumbnail cache {}: {e}", dir.display()));
        Generator { dir: dir.to_path_buf(), slots: Mutex::new(0), freed: Condvar::new(), init_err }
    }

    pub fn health(&self) -> Result<(), Error> {
        if let Some(e) = &self.init_err {
            return Err(Error::unavailable(e));
        }
        if exec::look_path("ffmpeg").is_none() {
            return Err(Error::unavailable("ffmpeg not found"));
        }
        if !self.dir.is_dir() {
            return Err(Error::unavailable(format!("thumbnail cache {} unusable", self.dir.display())));
        }
        Ok(())
    }

    /// A still of `req.file_path`; width 0 means 480, a negative time 0.
    pub fn generate(&self, req: &Request) -> Result<Thumbnail, Error> {
        if req.file_path.is_empty() {
            return Err(Error::invalid("file_path required"));
        }
        let width = if req.width <= 0 { 480 } else { req.width };
        let at = if req.time_sec.is_finite() && req.time_sec > 0.0 { req.time_sec } else { 0.0 };
        let meta = std::fs::metadata(&req.file_path).map_err(|_| Error::unavailable("source file unavailable"))?;
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let key = crate::settings::sha256_hex(&format!("{}|{mtime}|{}|{at:.1}|{width}", req.file_path, meta.len()));
        let out = self.dir.join(format!("{key}.jpg"));
        let cached = |p: &Path| std::fs::metadata(p).is_ok_and(|m| m.len() > 0);
        let hit = || Thumbnail { path: out.to_string_lossy().into_owned(), width, cached: true };
        if cached(&out) {
            return Ok(hit());
        }
        let _slot = self.acquire();
        // Another request may have made it while this one waited.
        if cached(&out) {
            return Ok(hit());
        }
        let made = Thumbnail { path: out.to_string_lossy().into_owned(), width, cached: false };
        match extract(&req.file_path, at, width, &out) {
            Ok(()) => Ok(made),
            // A seek past the end yields nothing; the start always exists.
            Err(_) if at > 0.0 && extract(&req.file_path, 0.0, width, &out).is_ok() => Ok(made),
            Err(e) => Err(Error::unavailable(format!("ffmpeg: {e}"))),
        }
    }

    fn acquire(&self) -> Slot<'_> {
        let mut n = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        while *n >= MAX_PARALLEL {
            n = self.freed.wait(n).unwrap_or_else(|e| e.into_inner());
        }
        *n += 1;
        Slot(self)
    }
}

struct Slot<'a>(&'a Generator);

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        let mut n = self.0.slots.lock().unwrap_or_else(|e| e.into_inner());
        *n = n.saturating_sub(1);
        self.0.freed.notify_one();
    }
}

/// Seeks before `-i` (keyframe-fast) and writes one JPEG atomically; a
/// comic archive's cover page is piped in instead.
fn extract(path: &str, at: f64, width: i64, out: &Path) -> Result<(), String> {
    let tmp = out.with_extension("jpg.tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut cmd = Command::new("ffmpeg");
    let stdin: Option<Box<dyn std::io::Read + Send>> = if comic(path) {
        let page = lain_read::archive::first_page(Path::new(path)).map_err(|e| e.to_string())?;
        cmd.args(["-hide_banner", "-loglevel", "error", "-i", "pipe:0"]);
        Some(Box::new(std::io::Cursor::new(page)))
    } else {
        cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-ss", &format!("{at:.3}"), "-i", path]);
        None
    };
    cmd.args(["-frames:v", "1", "-vf", &format!("scale={width}:-2"), "-q:v", "4", "-f", "image2", "-y"]).arg(&tmp);
    let o = exec::run(cmd, stdin, TIMEOUT).map_err(|e| e.to_string())?;
    if !o.status.success() {
        let _ = std::fs::remove_file(&tmp);
        return Err(o.describe());
    }
    if !std::fs::metadata(&tmp).is_ok_and(|m| m.len() > 0) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("no frame at {at:.1}s"));
    }
    std::fs::rename(&tmp, out).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_bad_input() {
        let dir = lain_store::test_dir("thumb-bad");
        let g = Generator::new(&dir.join("cache"));
        assert_eq!(g.generate(&Request::default()).unwrap_err().code, "invalid-message");
        let missing = Request { file_path: dir.join("nope.mkv").to_string_lossy().into_owned(), ..Default::default() };
        assert_eq!(g.generate(&missing).unwrap_err().msg, "source file unavailable");
        std::fs::write(dir.join("a.cbz"), b"zip").unwrap();
        let c = Request { file_path: dir.join("a.cbz").to_string_lossy().into_owned(), ..Default::default() };
        assert_eq!(g.generate(&c).unwrap_err().code, "dependency-unavailable", "a broken archive is no cover");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn comic_cover_is_the_first_page() {
        if lain_probe::exec::look_path("ffmpeg").is_none() {
            assert!(std::env::var("LAIN_REQUIRE_FFMPEG").is_err(), "LAIN_REQUIRE_FFMPEG is set but ffmpeg is missing");
            return;
        }
        let dir = lain_store::test_dir("thumb-comic");
        std::fs::create_dir_all(&dir).unwrap();
        let mut w = lain_read::zip::Writer::new();
        w.add("01.png", &lain_read::image::test_png(40, 60), false);
        let cbz = dir.join("Vol 01.cbz");
        std::fs::write(&cbz, w.finish()).unwrap();
        let g = Generator::new(&dir.join("cache"));
        let t = g
            .generate(&Request { file_path: cbz.to_string_lossy().into_owned(), width: 32, ..Default::default() })
            .unwrap();
        let jpg = std::fs::read(&t.path).unwrap();
        assert_eq!(&jpg[..2], &[0xFF, 0xD8], "a JPEG");
        assert!(
            g.generate(&Request { file_path: cbz.to_string_lossy().into_owned(), width: 32, ..Default::default() })
                .unwrap()
                .cached
        );
        std::fs::remove_dir_all(dir).ok();
    }
}

#[cfg(test)]
mod cover_tests {
    use super::*;

    #[test]
    fn a_still_next_to_the_file_is_its_cover() {
        let dir = std::env::temp_dir().join(format!("lain-cover-{}", std::process::id()));
        let thumbs = dir.join(".metadata").join("thumbs");
        std::fs::create_dir_all(&thumbs).unwrap();
        let media = dir.join("item.mp4");
        assert_eq!(local_cover(&media), None);
        std::fs::write(thumbs.join("thumbnail.jpg"), b"").unwrap();
        assert_eq!(local_cover(&media), None, "an empty file is not a cover");
        std::fs::write(thumbs.join("banner.jpg"), b"jpg").unwrap();
        assert_eq!(local_cover(&media), Some(thumbs.join("banner.jpg")));
        std::fs::write(thumbs.join("thumbnail.jpg"), b"\x89PNG\r\n\x1a\n").unwrap();
        assert_eq!(local_cover(&media), Some(thumbs.join("thumbnail.jpg")));
        assert_eq!(image_type(&thumbs.join("thumbnail.jpg")), "image/png");
        // A poster named after the file comes first.
        std::fs::write(dir.join("item-poster.webp"), b"RIFF\x10\0\0\0WEBPVP8 ").unwrap();
        assert_eq!(local_cover(&media), Some(dir.join("item-poster.webp")));
        assert_eq!(image_type(&dir.join("item-poster.webp")), "image/webp");
        assert_eq!(image_type(&dir.join("missing.jpg")), "image/jpeg");
        std::fs::remove_dir_all(dir).ok();
    }
}
