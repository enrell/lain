//! The library folder picker (`GET /api/browse`, admin): server-side
//! subdirectories only (no files, no dotfiles), sorted, capped so huge
//! trees stay responsive.

use std::path::{Component, Path, PathBuf};

use lain_api::{BrowseDir, BrowseResult};

/// Auto-detect roots, most specific first.
pub const CANDIDATES: &[&str] = &[
    "/media/videos",
    "/media",
    "/videos",
    "/mnt/media",
    "/srv/media",
    "/data/media",
    "/mnt",
    "/srv",
    "/data",
    "/home",
    "/",
];

/// The most entries listed.
pub const MAX_DIRS: usize = 500;

/// Lexical clean (Go's `filepath.Clean`): `.` dropped, `..` resolved
/// against the parent, never above the root.
pub fn clean(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() && !p.has_root() {
                    out.push("..");
                }
            }
            other => out.push(other),
        }
    }
    if out.as_os_str().is_empty() { PathBuf::from(".") } else { out }
}

/// The first candidate that is a directory. On Windows `/` is not
/// absolute, so the system drive's root stands in for the list.
fn detect() -> Option<String> {
    if cfg!(windows) {
        let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        return Some(format!("{drive}\\")).filter(|d| Path::new(d).is_dir());
    }
    CANDIDATES.iter().find(|c| Path::new(c).is_dir()).map(|c| c.to_string())
}

/// Lists `path`'s subdirectories; an empty path auto-detects the media
/// root. The error texts are the ones clients read.
pub fn browse(path: &str) -> Result<BrowseResult, &'static str> {
    let raw = path.trim();
    let (raw, detected) =
        if raw.is_empty() { (detect().ok_or("no browsable directory found")?, true) } else { (raw.to_string(), false) };
    let dir = clean(Path::new(&raw));
    if !dir.is_absolute() {
        return Err("path must be absolute");
    }
    let rd = std::fs::read_dir(&dir).map_err(|_| "path is not a readable directory")?;
    let mut names: Vec<String> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !n.starts_with('.'))
        .collect();
    names.sort();
    names.truncate(MAX_DIRS);
    let dirs =
        names.into_iter().map(|n| BrowseDir { path: dir.join(&n).to_string_lossy().into_owned(), name: n }).collect();
    let parent = dir.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(BrowseResult { path: dir.to_string_lossy().into_owned(), parent, detected, dirs })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_sorted_visible_subdirectories() {
        let root = std::env::temp_dir().join(format!("lain-browse-{}", std::process::id()));
        for d in ["Movies", "Anime", ".hidden"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::write(root.join("file.txt"), b"x").unwrap();
        let got = browse(&root.to_string_lossy()).unwrap();
        assert_eq!(got.path, root.to_string_lossy());
        assert_eq!(got.parent, root.parent().unwrap().to_string_lossy());
        assert!(!got.detected);
        let names: Vec<_> = got.dirs.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec!["Anime", "Movies"]);
        assert_eq!(got.dirs[0].path, root.join("Anime").to_string_lossy());
        assert_eq!(browse("relative/path").unwrap_err(), "path must be absolute");
        assert_eq!(browse(&root.join("missing").to_string_lossy()).unwrap_err(), "path is not a readable directory");
        assert!(browse("").unwrap().detected);
        std::fs::remove_dir_all(root).ok();
    }

    #[cfg(unix)]
    #[test]
    fn cleans_like_go() {
        assert_eq!(clean(Path::new("/a/./b/../c/")), PathBuf::from("/a/c"));
        assert_eq!(clean(Path::new("/../..")), PathBuf::from("/"));
        assert_eq!(browse("/").unwrap().parent, "");
    }
}
