//! The read module (D-155, D-195): comic and manga archives behind
//! `lain.comic.pages@1`. The provider indexes pages; page bytes never
//! cross the provider boundary: the gateway opens the archive with
//! [`archive::open_page`] and answers the bytes itself, exactly like
//! media streams.

pub mod archive;
pub mod comicinfo;
pub mod image;
pub mod zip;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use lain_core::{Error, Provider};

pub use lain_api::{ComicInfo, ComicPage, ComicPages};

pub const CAP_COMIC_PAGES: &str = "lain.comic.pages@1";

/// The built-in provider id (Go's).
pub const PROVIDER_ID: &str = "lain-comic-archive";

/// Library and item kinds of the read module.
pub const KIND_COMIC: &str = "comic";
pub const KIND_MANGA: &str = "manga";

/// The page index cache holds this many archives.
const MAX_CACHED: usize = 64;

/// One page as the provider returns it: the client's [`ComicPage`]
/// plus the member name, which only the gateway uses (to open the
/// page) and never sends to a client.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexedPage {
    pub index: i64,
    pub name: String,
    pub mime: String,
    pub size: i64,
    pub width: u32,
    pub height: u32,
}

impl IndexedPage {
    pub fn public(&self) -> ComicPage {
        ComicPage {
            index: self.index,
            mime: self.mime.clone(),
            size: self.size,
            width: i64::from(self.width),
            height: i64::from(self.height),
        }
    }
}

/// The provider's output: the page index of one archive.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageIndex {
    pub format: String,
    pub pages: Vec<IndexedPage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info: Option<ComicInfo>,
}

/// The input: the archive path, supplied by the gateway after the
/// catalog lookup, never by a client.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PagesInput {
    #[serde(default)]
    pub path: String,
}

/// The default reading direction: the archive's own declaration when
/// it makes one (`ComicInfo.xml`), else the kind's convention.
pub fn default_direction(kind: &str, info: Option<&ComicInfo>) -> String {
    match info {
        Some(i) if !i.direction.is_empty() => i.direction.clone(),
        _ if kind == KIND_MANGA => "rtl".into(),
        _ => "ltr".into(),
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    path: PathBuf,
    mtime: u128,
    size: u64,
}

/// `lain.comic.pages@1`. Indexes are cached by file identity (path,
/// mtime, size), so a replaced archive re-indexes itself.
#[derive(Default)]
pub struct ComicArchive {
    cache: Mutex<HashMap<CacheKey, PageIndex>>,
}

impl ComicArchive {
    pub fn new() -> ComicArchive {
        ComicArchive::default()
    }

    /// The page index of the archive at `path`.
    pub fn index(&self, path: &Path) -> Result<PageIndex, Error> {
        let meta = std::fs::metadata(path).map_err(|_| Error::unavailable("archive unavailable"))?;
        let mtime = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
        let key = CacheKey { path: path.to_path_buf(), mtime, size: meta.len() };
        if let Some(v) = self.cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
            return Ok(v.clone());
        }
        let es = archive::list(path)?;
        let mut out = PageIndex {
            format: archive::format(path).to_string(),
            pages: es
                .iter()
                .enumerate()
                .map(|(i, e)| IndexedPage {
                    index: i as i64,
                    name: e.name.clone(),
                    mime: archive::mime_of(&e.name).to_string(),
                    size: e.size as i64,
                    width: 0,
                    height: 0,
                })
                .collect(),
            info: None,
        };
        // Header probing reads every page; only the in-process reader
        // does that cheaply.
        if out.format == "cbz" {
            archive::probe_zip(path, &mut out.pages);
        }
        out.info = archive::read_info(path);
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if cache.len() >= MAX_CACHED {
            cache.clear();
        }
        cache.insert(key, out.clone());
        Ok(out)
    }
}

impl Provider for ComicArchive {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn capabilities(&self) -> Vec<String> {
        vec![CAP_COMIC_PAGES.into()]
    }

    fn health(&self) -> Result<(), Error> {
        Ok(())
    }

    fn invoke(&self, cap: &str, _op: &str, input: Value) -> Result<Value, Error> {
        if cap != CAP_COMIC_PAGES {
            return Err(Error::invalid(format!("unsupported cap {cap}")));
        }
        let i: PagesInput = serde_json::from_value(input).map_err(|_| Error::invalid("PagesInput required"))?;
        if i.path.is_empty() {
            return Err(Error::invalid("path required"));
        }
        Ok(json!(self.index(Path::new(&i.path))?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::test_png;
    use crate::zip::Writer;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lain-read-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn cbz(path: &Path, members: &[(&str, Vec<u8>)]) {
        let mut w = Writer::new();
        for (i, (n, b)) in members.iter().enumerate() {
            w.add(n, b, i % 2 == 0);
        }
        std::fs::write(path, w.finish()).unwrap();
    }

    fn pages(p: &ComicArchive, path: &Path) -> Result<PageIndex, Error> {
        p.invoke(CAP_COMIC_PAGES, "", json!({"path": path})).map(|v| serde_json::from_value(v).unwrap())
    }

    #[test]
    fn pages_are_natural_ordered_and_filtered() {
        let d = dir("order");
        let p = d.join("a.cbz");
        let names = [
            "page10.png",
            "page2.png",
            "page1.png",
            "__MACOSX/page1.png",
            ".hidden.png",
            "ComicInfo.xml",
            "ch1/",
            "notes.txt",
            "page3.PNG",
        ];
        cbz(&p, &names.iter().map(|n| (*n, test_png(20, 30))).collect::<Vec<_>>());
        let out = pages(&ComicArchive::new(), &p).unwrap();
        let got: Vec<&str> = out.pages.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(got, ["page1.png", "page2.png", "page3.PNG", "page10.png"]);
        assert_eq!(out.format, "cbz");
        let first = &out.pages[0];
        assert_eq!((first.index, first.mime.as_str(), first.width, first.height), (0, "image/png", 20, 30));
        assert_eq!(first.size as usize, test_png(20, 30).len());
        // The client view never carries the member name.
        assert!(!json!(first.public()).to_string().contains("page1"));
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn open_page_refuses_non_images_and_unknown_names() {
        let d = dir("open");
        let p = d.join("a.cbz");
        cbz(&p, &[("1.png", test_png(1, 1)), ("secret.txt", b"x".to_vec())]);
        assert_eq!(archive::open_page(&p, "1.png").unwrap(), test_png(1, 1));
        for n in ["secret.txt", "missing.png", "../1.png"] {
            assert!(archive::open_page(&p, n).is_err(), "{n}");
        }
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn invalid_and_unsupported_archives() {
        let d = dir("bad");
        let bad = d.join("bad.cbz");
        std::fs::write(&bad, b"not a zip").unwrap();
        let p = ComicArchive::new();
        assert_eq!(pages(&p, &bad).unwrap_err().code, archive::INVALID_ARCHIVE);
        std::fs::write(d.join("x.pdf"), b"%PDF").unwrap();
        assert_eq!(pages(&p, &d.join("x.pdf")).unwrap_err().code, archive::UNSUPPORTED_FORMAT);
        assert_eq!(pages(&p, &d.join("missing.cbz")).unwrap_err().code, "dependency-unavailable");
        assert_eq!(p.invoke(CAP_COMIC_PAGES, "", json!({})).unwrap_err().code, "invalid-message");
        assert_eq!(p.invoke("lain.other@1", "", json!({"path": "x"})).unwrap_err().code, "invalid-message");
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn index_cache_follows_file_identity() {
        let d = dir("cache");
        let p = d.join("a.cbz");
        cbz(&p, &[("1.png", test_png(1, 1))]);
        let prov = ComicArchive::new();
        assert_eq!(pages(&prov, &p).unwrap().pages.len(), 1);
        std::thread::sleep(std::time::Duration::from_millis(20));
        cbz(&p, &[("1.png", test_png(1, 1)), ("2.png", test_png(1, 1))]);
        assert_eq!(pages(&prov, &p).unwrap().pages.len(), 2, "a replaced archive re-indexes");
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn info_comes_from_the_root_copy_and_sets_direction() {
        let d = dir("info");
        let p = d.join("a.cbz");
        cbz(
            &p,
            &[
                ("nested/ComicInfo.xml", b"<ComicInfo><Series>Nested</Series></ComicInfo>".to_vec()),
                (
                    "comicinfo.xml",
                    b"<ComicInfo><Series>Root</Series><Manga>YesAndRightToLeft</Manga></ComicInfo>".to_vec(),
                ),
                ("p1.png", test_png(4, 6)),
            ],
        );
        let out = pages(&ComicArchive::new(), &p).unwrap();
        let info = out.info.unwrap();
        assert_eq!((info.series.as_str(), out.pages.len()), ("Root", 1));
        assert_eq!(default_direction(KIND_COMIC, Some(&info)), "rtl");
        assert_eq!(default_direction(KIND_COMIC, None), "ltr");
        assert_eq!(default_direction(KIND_MANGA, None), "rtl");
        let bare = d.join("b.cbz");
        cbz(&bare, &[("p1.png", test_png(4, 6))]);
        assert_eq!(archive::read_info(&bare), None);
        assert_eq!(archive::read_info(&d.join("none.cbz")), None);
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn first_page_prefers_portrait() {
        let d = dir("cover");
        let (banner, portrait) = (test_png(40, 10), test_png(10, 20));
        let p = d.join("a.cbz");
        cbz(&p, &[("0.png", banner.clone()), ("1.png", portrait.clone())]);
        assert_eq!(archive::first_page(&p).unwrap(), portrait);
        let q = d.join("b.cbz");
        cbz(&q, &[("0.png", banner.clone())]);
        assert_eq!(archive::first_page(&q).unwrap(), banner);
        std::fs::remove_dir_all(d).ok();
    }

    /// bsdtar reads zip content whatever the extension, so a zip named
    /// `.cbr` exercises the external path without a rar encoder.
    #[test]
    fn external_extractor_path() {
        if archive::format(Path::new("x.cbr")) != "cbr"
            || std::process::Command::new("bsdtar").arg("--version").output().is_err()
        {
            eprintln!("bsdtar not installed; skipped");
            return;
        }
        let d = dir("external");
        let p = d.join("a.cbr");
        cbz(&p, &[("2.png", test_png(2, 2)), ("1.png", test_png(2, 2))]);
        let out = pages(&ComicArchive::new(), &p).unwrap();
        let got: Vec<&str> = out.pages.iter().map(|x| x.name.as_str()).collect();
        assert_eq!((out.format.as_str(), got), ("cbr", vec!["1.png", "2.png"]));
        assert_eq!(archive::open_page(&p, "2.png").unwrap(), test_png(2, 2));
        std::fs::remove_dir_all(d).ok();
    }
}
