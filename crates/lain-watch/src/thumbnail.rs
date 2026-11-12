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

/// A storyboard: one picture holding small frames of a video at a fixed
/// interval, row by row. A seek bar shows the frame it points at by
/// cutting a tile out of it, so a preview costs nothing once the picture
/// is there, where a still per position costs an ffmpeg run each.
pub const TILE_WIDTH: i64 = 160;
pub const TILE_HEIGHT: i64 = 90;
pub const STORYBOARD_COLUMNS: i64 = 10;
/// Enough for a frame every 12 s of a 24 minute episode.
const STORYBOARD_TILES: i64 = 120;
/// The whole file is read once (keyframes only), so a long film on a slow
/// disk needs room.
const STORYBOARD_TIMEOUT: Duration = Duration::from_secs(240);

/// How a storyboard of a video of this length is laid out: tile `i`
/// (row-major) is the frame near `i * interval_sec`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoryboardLayout {
    pub interval_sec: i64,
    pub count: i64,
    pub columns: i64,
    pub rows: i64,
    pub tile_width: i64,
    pub tile_height: i64,
}

/// `None` for a length that is unknown or too short to be worth a sheet.
pub fn storyboard_layout(duration_sec: f64) -> Option<StoryboardLayout> {
    if !duration_sec.is_finite() || duration_sec < 4.0 {
        return None;
    }
    let interval_sec = ((duration_sec / STORYBOARD_TILES as f64).ceil() as i64).max(2);
    let count = ((duration_sec / interval_sec as f64).ceil() as i64).clamp(1, STORYBOARD_TILES);
    let rows = (count + STORYBOARD_COLUMNS - 1) / STORYBOARD_COLUMNS;
    Some(StoryboardLayout {
        interval_sec,
        count,
        columns: STORYBOARD_COLUMNS,
        rows,
        tile_width: TILE_WIDTH,
        tile_height: TILE_HEIGHT,
    })
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StoryboardRequest {
    pub file_path: String,
    pub duration_sec: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Storyboard {
    pub path: String,
    pub layout: StoryboardLayout,
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

    /// The storyboard of `req.file_path`, made once and kept (the key
    /// holds the file's size and time, so a replaced file gets a new one).
    pub fn storyboard(&self, req: &StoryboardRequest) -> Result<Storyboard, Error> {
        if req.file_path.is_empty() {
            return Err(Error::invalid("file_path required"));
        }
        if comic(&req.file_path) {
            return Err(Error::invalid("not a video"));
        }
        let layout =
            storyboard_layout(req.duration_sec).ok_or_else(|| Error::invalid("duration unknown or too short"))?;
        let meta = std::fs::metadata(&req.file_path).map_err(|_| Error::unavailable("source file unavailable"))?;
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let key = crate::settings::sha256_hex(&format!(
            "storyboard|{}|{mtime}|{}|{}|{}",
            req.file_path,
            meta.len(),
            layout.interval_sec,
            layout.count
        ));
        let out = self.dir.join(format!("{key}.sb.jpg"));
        let cached = |p: &Path| std::fs::metadata(p).is_ok_and(|m| m.len() > 0);
        let hit = |cached| Storyboard { path: out.to_string_lossy().into_owned(), layout, cached };
        if cached(&out) {
            return Ok(hit(true));
        }
        let _slot = self.acquire();
        if cached(&out) {
            return Ok(hit(true));
        }
        sheet(&req.file_path, &layout, &out).map_err(|e| Error::unavailable(format!("ffmpeg: {e}")))?;
        Ok(hit(false))
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

/// One ffmpeg run over the file: keyframes only (decoding every frame of
/// a film for 120 small pictures would take minutes), one frame per
/// interval, each fitted into a tile, tiled into one JPEG.
fn sheet(path: &str, layout: &StoryboardLayout, out: &Path) -> Result<(), String> {
    let tmp = out.with_extension("jpg.tmp");
    let _ = std::fs::remove_file(&tmp);
    let (w, h) = (layout.tile_width, layout.tile_height);
    let filter = format!(
        "fps=1/{}:round=down,scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:black,tile={}x{}",
        layout.interval_sec, layout.columns, layout.rows
    );
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-skip_frame", "nokey", "-i", path]);
    cmd.args(["-an", "-sn", "-dn", "-vf", &filter, "-frames:v", "1", "-q:v", "5", "-f", "image2", "-y"]).arg(&tmp);
    let o = exec::run(cmd, None, STORYBOARD_TIMEOUT).map_err(|e| e.to_string())?;
    if !o.status.success() {
        let _ = std::fs::remove_file(&tmp);
        return Err(o.describe());
    }
    if !std::fs::metadata(&tmp).is_ok_and(|m| m.len() > 0) {
        let _ = std::fs::remove_file(&tmp);
        return Err("no storyboard produced".into());
    }
    std::fs::rename(&tmp, out).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storyboard_layouts() {
        assert_eq!(storyboard_layout(0.0), None);
        assert_eq!(storyboard_layout(f64::NAN), None);
        // A 24 minute episode: a frame every 12 s, 120 tiles in 12 rows.
        let ep = storyboard_layout(1440.0).unwrap();
        assert_eq!((ep.interval_sec, ep.count, ep.columns, ep.rows), (12, 120, 10, 12));
        // A short clip keeps the 2 s floor and a partial last row.
        let clip = storyboard_layout(45.0).unwrap();
        assert_eq!((clip.interval_sec, clip.count, clip.rows), (2, 23, 3));
        // A film never exceeds the tile budget.
        let film = storyboard_layout(7200.0).unwrap();
        assert_eq!((film.interval_sec, film.count), (60, 120));
        assert!(film.count * film.interval_sec >= 7200);
    }

    #[test]
    fn storyboard_of_a_video() {
        if lain_probe::exec::look_path("ffmpeg").is_none() {
            assert!(std::env::var("LAIN_REQUIRE_FFMPEG").is_err(), "LAIN_REQUIRE_FFMPEG is set but ffmpeg is missing");
            return;
        }
        let dir = lain_store::test_dir("storyboard");
        let video = dir.join("clip.mp4");
        let made = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=320x180:rate=10:duration=30",
            ])
            .args(["-g", "10", "-pix_fmt", "yuv420p", "-y"])
            .arg(&video)
            .status()
            .unwrap();
        assert!(made.success());
        let g = Generator::new(&dir.join("cache"));
        let req = StoryboardRequest { file_path: video.to_string_lossy().into_owned(), duration_sec: 30.0 };
        let first = g.storyboard(&req).unwrap();
        assert_eq!((first.layout.interval_sec, first.layout.count, first.layout.rows), (2, 15, 2));
        assert!(!first.cached && std::fs::metadata(&first.path).unwrap().len() > 1000);
        // JPEG, and as large as its tiles say.
        let bytes = std::fs::read(&first.path).unwrap();
        assert_eq!(&bytes[..2], &[0xFF, 0xD8]);
        assert!(g.storyboard(&req).unwrap().cached, "the second request reuses the file");
        assert_eq!(
            g.storyboard(&StoryboardRequest { duration_sec: 0.0, ..req.clone() }).unwrap_err().code,
            "invalid-message"
        );
    }

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
