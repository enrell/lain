//! Comic and manga archives (`lain.comic.pages@1`, D-085): the page
//! list in reading order, single pages and `ComicInfo.xml`. CBZ is read
//! in-process ([`crate::zip`]); CBR and CB7 go through an external
//! extractor (`bsdtar`, `7z` or `7zz`), the same operator-installed
//! tool category as ffmpeg, and fail with `dependency-unavailable`
//! when none is installed. Pixels are never decoded or rescaled.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use lain_core::Error;

use crate::comicinfo::{self, ComicInfo};
use crate::image;
use crate::zip::{Archive, ZipError};

/// One page read is bounded: a decompression bomb inside an archive
/// must not exhaust memory.
pub const MAX_PAGE_BYTES: u64 = 128 << 20;

/// External extractors get this long per call.
const EXTRACT_TIMEOUT: Duration = Duration::from_secs(30);

/// Leading pages a cover pick inspects.
const COVER_PROBE: usize = 4;

pub const INVALID_ARCHIVE: &str = "invalid-archive";
pub const UNSUPPORTED_FORMAT: &str = "unsupported-format";

fn invalid(e: impl std::fmt::Display) -> Error {
    Error::new(INVALID_ARCHIVE, e.to_string())
}

impl From<ZipError> for Error {
    fn from(e: ZipError) -> Error {
        invalid(e)
    }
}

/// One archive member that is a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub size: u64,
}

/// The format token of a path (`cbz`, `cbr`, `cb7`), or `""` when the
/// extension is not a comic archive.
pub fn format(p: &Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("cbz" | "zip") => "cbz",
        Some("cbr" | "rar") => "cbr",
        Some("cb7" | "7z") => "cb7",
        _ => "",
    }
}

/// The image mime type of an entry name, `""` if it is not an image.
pub fn mime_of(name: &str) -> &'static str {
    let base = name.rsplit('/').next().unwrap_or(name);
    let Some((_, ext)) = base.rsplit_once('.') else { return "" };
    match ext.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "avif" => "image/avif",
        "bmp" => "image/bmp",
        _ => "",
    }
}

/// Whether a member is a page: not a directory, dotfile, macOS
/// resource fork or non-image.
pub fn keep_entry(name: &str) -> bool {
    if name.is_empty() || name.ends_with('/') {
        return false;
    }
    if name.split('/').any(|seg| seg.starts_with('.') || seg == "__MACOSX") {
        return false;
    }
    !mime_of(name).is_empty()
}

/// Natural order: `page2` before `page10`; digit runs compare by value,
/// then by length (`01` after `1`), the rest bytewise, case-insensitive.
pub fn natural_less(a: &str, b: &str) -> bool {
    let (a, b) = (a.to_lowercase(), b.to_lowercase());
    let (mut a, mut b) = (a.as_bytes(), b.as_bytes());
    while !a.is_empty() && !b.is_empty() {
        if a[0].is_ascii_digit() && b[0].is_ascii_digit() {
            let ra = a.iter().take_while(|c| c.is_ascii_digit()).count();
            let rb = b.iter().take_while(|c| c.is_ascii_digit()).count();
            let (na, nb) = (&a[..ra], &b[..rb]);
            let trim = |s: &[u8]| s.iter().position(|&c| c != b'0').map_or(0, |i| s.len() - i);
            let (ta, tb) = (trim(na), trim(nb));
            if ta != tb {
                return ta < tb;
            }
            let (sa, sb) = (&na[ra - ta..], &nb[rb - tb..]);
            if sa != sb {
                return sa < sb;
            }
            if ra != rb {
                return ra < rb;
            }
            a = &a[ra..];
            b = &b[rb..];
            continue;
        }
        if a[0] != b[0] {
            return a[0] < b[0];
        }
        a = &a[1..];
        b = &b[1..];
    }
    a.len() < b.len()
}

fn sort_entries(es: &mut [Entry]) {
    es.sort_by(|x, y| {
        if natural_less(&x.name, &y.name) {
            std::cmp::Ordering::Less
        } else if natural_less(&y.name, &x.name) {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    });
}

/// The pages of an archive in reading order.
pub fn list(path: &Path) -> Result<Vec<Entry>, Error> {
    let mut es = match format(path) {
        "cbz" => Archive::open(path)?
            .entries()
            .iter()
            .filter(|e| keep_entry(&e.name))
            .map(|e| Entry { name: e.name.clone(), size: e.size })
            .collect(),
        "cbr" | "cb7" => list_external(path)?,
        _ => return Err(unsupported(path)),
    };
    sort_entries(&mut es);
    Ok(es)
}

fn unsupported(path: &Path) -> Error {
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    Error::new(UNSUPPORTED_FORMAT, format!("not a comic archive: {ext}"))
}

/// The bytes of one page. Pages are addressed by the names [`list`]
/// produced; anything that is not an image member is refused, so a
/// client index can never reach an arbitrary archive member.
pub fn open_page(path: &Path, name: &str) -> Result<Vec<u8>, Error> {
    if mime_of(name).is_empty() {
        return Err(Error::not_found("not a page"));
    }
    read_member(path, name, MAX_PAGE_BYTES)
}

fn read_member(path: &Path, name: &str, limit: u64) -> Result<Vec<u8>, Error> {
    match format(path) {
        "cbz" => {
            let z = Archive::open(path)?;
            let e = z.find(name).ok_or_else(|| Error::not_found("no such page"))?;
            Ok(z.read(e, limit)?)
        }
        "cbr" | "cb7" => extract(path, name, limit),
        _ => Err(Error::new(UNSUPPORTED_FORMAT, "not a comic archive")),
    }
}

/// Fills page dimensions from each page's header, with one archive
/// open (CBZ only; external formats leave sizes to the client).
pub fn probe_zip(path: &Path, pages: &mut [crate::IndexedPage]) {
    let Ok(z) = Archive::open(path) else { return };
    for p in pages {
        if let Some(e) = z.find(&p.name)
            && let Ok(head) = z.head(e, image::HEAD_BYTES)
        {
            (p.width, p.height) = image::dimensions(&head, &p.mime);
        }
    }
}

/// The archive's `ComicInfo.xml`, `None` when it has none or it does
/// not parse. The copy nearest the root wins.
pub fn read_info(path: &Path) -> Option<ComicInfo> {
    let raw = match format(path) {
        "cbz" => {
            let z = Archive::open(path).ok()?;
            let best = z
                .entries()
                .iter()
                .filter(|e| e.name.rsplit('/').next().is_some_and(|b| b.eq_ignore_ascii_case("ComicInfo.xml")))
                .min_by_key(|e| e.name.matches('/').count())?;
            z.read(best, comicinfo::MAX_INFO_BYTES).ok()?
        }
        "cbr" | "cb7" => extract(path, "ComicInfo.xml", comicinfo::MAX_INFO_BYTES).ok()?,
        _ => return None,
    };
    comicinfo::parse(&raw)
}

/// The page that best represents the archive: the first portrait page
/// among the leading ones (a title banner or a wide spread makes a poor
/// cover), else the first page.
pub fn first_page(path: &Path) -> Result<Vec<u8>, Error> {
    let es = list(path)?;
    let first = es.first().ok_or_else(|| Error::not_found("archive has no pages"))?;
    let mut pick = first.name.clone();
    if format(path) == "cbz" {
        for e in es.iter().take(COVER_PROBE) {
            let Ok(b) = open_page(path, &e.name) else { continue };
            let (w, h) = image::dimensions(&b, mime_of(&e.name));
            if h > w && w > 0 {
                pick = e.name.clone();
                break;
            }
        }
    }
    open_page(path, &pick)
}

// ---- external extractors ---------------------------------------------------

/// The external tool that reads rar and 7z archives.
fn extractor() -> Result<std::path::PathBuf, Error> {
    for tool in ["bsdtar", "7z", "7zz"] {
        if let Some(p) = which(tool) {
            return Ok(p);
        }
    }
    Err(Error::unavailable("cbr/cb7 need bsdtar or 7z installed"))
}

fn which(tool: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exts: Vec<String> = if cfg!(windows) { vec![".exe".into(), String::new()] } else { vec![String::new()] };
    std::env::split_paths(&path)
        .find_map(|dir| exts.iter().map(|x| dir.join(format!("{tool}{x}"))).find(|p| p.is_file()))
}

fn is_bsdtar(tool: &Path) -> bool {
    tool.file_stem().is_some_and(|s| s == "bsdtar")
}

/// Runs a tool and returns its stdout, at most `limit` bytes, within
/// [`EXTRACT_TIMEOUT`].
fn run(mut cmd: Command, limit: u64) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut child =
        cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|e| e.to_string())?;
    let mut out = child.stdout.take().ok_or("no stdout")?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let r = (&mut out).take(limit + 1).read_to_end(&mut buf);
        (r, buf)
    });
    let deadline = std::time::Instant::now() + EXTRACT_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("timed out".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => return Err(e.to_string()),
        }
    };
    let (r, buf) = reader.join().map_err(|_| "reader failed")?;
    r.map_err(|e| e.to_string())?;
    if buf.len() as u64 > limit {
        return Err(format!("member exceeds {limit} bytes"));
    }
    if !status.success() {
        return Err(format!("exit status {}", status.code().unwrap_or(-1)));
    }
    Ok(buf)
}

fn list_external(path: &Path) -> Result<Vec<Entry>, Error> {
    let tool = extractor()?;
    let bsd = is_bsdtar(&tool);
    let mut cmd = Command::new(&tool);
    if bsd {
        cmd.arg("-tf").arg(path);
    } else {
        cmd.args(["l", "-ba", "-slt"]).arg(path);
    }
    let name = tool.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let out = run(cmd, 64 << 20).map_err(|e| invalid(format!("{name}: {e}")))?;
    let mut es = Vec::new();
    for line in String::from_utf8_lossy(&out).lines() {
        let line = line.trim_end_matches('\r');
        let line = match line.strip_prefix("Path = ") {
            Some(p) => p,
            None if bsd => line,
            None => continue,
        };
        if keep_entry(line) {
            es.push(Entry { name: line.to_string(), size: 0 });
        }
    }
    Ok(es)
}

fn extract(path: &Path, name: &str, limit: u64) -> Result<Vec<u8>, Error> {
    let tool = extractor()?;
    let mut cmd = Command::new(&tool);
    if is_bsdtar(&tool) {
        cmd.arg("-xOf").arg(path).arg("--").arg(name);
    } else {
        cmd.args(["e", "-so", "-y"]).arg(path).arg(name);
    }
    match run(cmd, limit) {
        Ok(b) if !b.is_empty() => Ok(b),
        _ => Err(invalid("could not extract page")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        let less =
            [("page2", "page10"), ("ch1/2.jpg", "ch10/1.jpg"), ("1", "01"), ("A1", "a2"), ("x", "xa"), ("007", "8")];
        for (a, b) in less {
            assert!(natural_less(a, b), "{a} < {b}");
            assert!(!natural_less(b, a), "!({b} < {a})");
        }
        assert!(!natural_less("same", "same"));
    }

    #[test]
    fn page_filter_and_formats() {
        for (n, keep) in [
            ("p1.png", true),
            ("dir/p.JPG", true),
            ("__MACOSX/p1.png", false),
            (".hidden.png", false),
            ("d/.x/p.png", false),
            ("ch1/", false),
            ("notes.txt", false),
            ("ComicInfo.xml", false),
            ("noext", false),
        ] {
            assert_eq!(keep_entry(n), keep, "{n}");
        }
        assert_eq!(format(Path::new("/a/b.CBZ")), "cbz");
        assert_eq!(format(Path::new("b.rar")), "cbr");
        assert_eq!(format(Path::new("b.7z")), "cb7");
        assert_eq!(format(Path::new("b.pdf")), "");
    }
}
