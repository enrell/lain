//! `lain-source-filesystem` (`lain.source.enumerate@1`): walks a
//! library root and returns recognized files with walk statistics. It
//! discovers files and never decides what they are. A port of
//! `internal/plugins/source`.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use lain_core::{Error, Provider};
use lain_identify::Candidate;

pub const SOURCE_ID: &str = "lain-source-filesystem";
pub const CAP_SOURCE_ENUMERATE: &str = "lain.source.enumerate@1";

const VIDEO: &[&str] = &["mkv", "mp4", "avi", "mov", "m4v", "webm"];

/// A folder that holds more than one kind: every video and reading
/// extension. Each file is still identified by its own extension.
const AUTO: &[&str] = &["mkv", "mp4", "avi", "mov", "m4v", "webm", "cbz", "cbr", "cb7"];

/// The extensions a library type recognizes; unknown types are video.
/// `auto` takes videos and comic archives alike (D-269).
pub fn extensions(kind: &str) -> &'static [&'static str] {
    match kind.to_lowercase().as_str() {
        "auto" => AUTO,
        "music" => &["mp3", "flac", "m4a", "aac", "ogg", "opus", "wav"],
        "book" => &["pdf", "epub", "mobi", "azw3"],
        "comic" | "manga" => &["cbz", "cbr", "cb7"],
        "photo" => &["jpg", "jpeg", "png", "webp", "gif", "heic", "avif"],
        _ => VIDEO,
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnumerateInput {
    pub root: String,
    #[serde(default)]
    pub library_id: String,
    #[serde(default, rename = "type")]
    pub kind: String,
}

/// What a walk saw. `accessible == false` means the root itself could
/// not be read (unmounted drive, deleted path): nothing may be marked
/// missing then.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnumStats {
    pub root: String,
    pub accessible: bool,
    pub dirs: i64,
    pub entries: i64,
    pub walk_errors: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnumerateOutput {
    pub candidates: Vec<Candidate>,
    pub stats: EnumStats,
}

fn mtime(m: &std::fs::Metadata) -> i64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Walks the root in lexical order without following symlinks (so loops
/// end). Unreadable entries are counted in `walk_errors` and skipped;
/// only an unreadable root is an error.
pub fn enumerate(input: &EnumerateInput) -> Result<EnumerateOutput, Error> {
    let mut stats = EnumStats { root: input.root.clone(), ..EnumStats::default() };
    let root = Path::new(&input.root);
    match std::fs::metadata(root) {
        Ok(m) if m.is_dir() => {}
        Ok(_) => return Err(Error::unavailable(format!("inaccessible root {}: not a directory", input.root))),
        Err(e) => return Err(Error::unavailable(format!("inaccessible root {}: {e}", input.root))),
    }
    stats.accessible = true;
    let allow = extensions(&input.kind);
    let kind = input.kind.to_lowercase();
    let mut out = Vec::new();
    walk(root, &mut stats, &mut |path: &Path, meta: &std::fs::Metadata| {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let ext = match name.rfind('.') {
            Some(i) => name[i + 1..].to_lowercase(),
            None => return,
        };
        if !allow.contains(&ext.as_str()) {
            return;
        }
        out.push(Candidate {
            path: path.to_string_lossy().into_owned(),
            size: meta.len() as i64,
            mod_time: mtime(meta),
            library_id: input.library_id.clone(),
            library_type: kind.clone(),
        });
    });
    Ok(EnumerateOutput { candidates: out, stats })
}

/// Go's `filepath.WalkDir` accounting: every visited entry counts, a
/// directory that cannot be listed counts once more as an error.
fn walk(dir: &Path, stats: &mut EnumStats, file: &mut dyn FnMut(&Path, &std::fs::Metadata)) {
    stats.entries += 1;
    stats.dirs += 1;
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => {
            stats.entries += 1;
            stats.walk_errors += 1;
            return;
        }
    };
    let mut ents: Vec<(String, std::fs::DirEntry)> = Vec::new();
    for e in rd {
        match e {
            Ok(e) => ents.push((e.file_name().to_string_lossy().into_owned(), e)),
            Err(_) => {
                stats.entries += 1;
                stats.walk_errors += 1;
            }
        }
    }
    ents.sort_by(|a, b| a.0.cmp(&b.0));
    for (_, e) in ents {
        let path = e.path();
        match e.file_type() {
            Ok(ft) if ft.is_dir() => walk(&path, stats, file),
            Ok(_) => {
                stats.entries += 1;
                match std::fs::symlink_metadata(&path) {
                    Ok(m) => file(&path, &m),
                    Err(_) => stats.walk_errors += 1,
                }
            }
            Err(_) => {
                stats.entries += 1;
                stats.walk_errors += 1;
            }
        }
    }
}

/// The provider.
pub struct Filesystem;

impl Provider for Filesystem {
    fn id(&self) -> &str {
        SOURCE_ID
    }

    fn capabilities(&self) -> Vec<String> {
        vec![CAP_SOURCE_ENUMERATE.into()]
    }

    fn health(&self) -> Result<(), Error> {
        Ok(())
    }

    fn invoke(&self, cap: &str, op: &str, input: Value) -> Result<Value, Error> {
        if cap != CAP_SOURCE_ENUMERATE {
            return Err(Error::invalid(format!("unsupported cap {cap}")));
        }
        if !op.is_empty() && op != "enumerate" {
            return Err(Error::invalid(format!("capability {cap} has no op {op}")));
        }
        let i: EnumerateInput =
            serde_json::from_value(input).map_err(|e| Error::invalid(format!("EnumerateInput required: {e}")))?;
        Ok(json!(enumerate(&i)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("lain-source-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn walk_counts_and_filters() {
        let d = tmp("walk");
        std::fs::create_dir_all(d.join("Show/S1")).unwrap();
        for f in ["Show/S1/b.MKV", "Show/S1/a.mkv", "Show/notes.txt", "c.cbz", "top.mp4"] {
            std::fs::write(d.join(f), b"xy").unwrap();
        }
        let out = enumerate(&EnumerateInput {
            root: d.to_string_lossy().into(),
            library_id: "lib".into(),
            kind: "Anime".into(),
        })
        .unwrap();
        let names: Vec<_> =
            out.candidates.iter().map(|c| c.path.rsplit(['/', '\\']).next().unwrap().to_string()).collect();
        assert_eq!(names, vec!["a.mkv", "b.MKV", "top.mp4"]);
        assert_eq!((out.candidates[0].size, out.candidates[0].library_type.as_str()), (2, "anime"));
        assert_eq!((out.stats.dirs, out.stats.walk_errors, out.stats.accessible), (3, 0, true));
        assert_eq!(out.stats.entries, 8);
        let manga = enumerate(&EnumerateInput {
            root: d.to_string_lossy().into(),
            library_id: "l".into(),
            kind: "manga".into(),
        })
        .unwrap();
        assert_eq!(manga.candidates.len(), 1);
        // A mixed folder: videos and archives together, text still left out.
        let auto = enumerate(&EnumerateInput {
            root: d.to_string_lossy().into(),
            library_id: "l".into(),
            kind: "auto".into(),
        })
        .unwrap();
        assert_eq!(auto.candidates.len(), 4);
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn missing_root_is_an_error() {
        let e = enumerate(&EnumerateInput { root: "/no/such/lain/root".into(), ..Default::default() }).unwrap_err();
        assert!(e.msg.contains("inaccessible root"));
        let d = tmp("file");
        std::fs::write(d.join("f"), b"").unwrap();
        assert!(
            enumerate(&EnumerateInput { root: d.join("f").to_string_lossy().into(), ..Default::default() }).is_err()
        );
        assert_eq!(Filesystem.invoke(CAP_SOURCE_ENUMERATE, "x", json!({})).unwrap_err().code, "invalid-message");
        std::fs::remove_dir_all(d).ok();
    }

    #[cfg(unix)]
    #[test]
    fn symlink_loops_end_and_dead_links_are_files() {
        let d = tmp("links");
        std::os::unix::fs::symlink(&d, d.join("loop")).unwrap();
        std::os::unix::fs::symlink(d.join("gone.mkv"), d.join("dead.mkv")).unwrap();
        let out = enumerate(&EnumerateInput { root: d.to_string_lossy().into(), ..Default::default() }).unwrap();
        assert_eq!(out.candidates.len(), 1);
        std::fs::remove_dir_all(d).ok();
    }
}
