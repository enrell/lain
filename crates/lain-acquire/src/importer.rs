//! Import: where each media file of a finished download goes in its
//! library (A-10, A-11), and placing it without ever overwriting.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use lain_api::acquire::{IMPORT_COPY, IMPORT_HARDLINK, IMPORT_MOVE};
use lain_api::{Library, Release};

use crate::errf;
use crate::error::{CODE_IMPORT, Result};
use crate::util::{base_name, ext_lower, loose_key, settle, within};

pub const VIDEO_EXTS: &[&str] = &[".mkv", ".mp4", ".avi", ".m4v", ".webm", ".mov", ".ts"];
pub const READING_EXTS: &[&str] = &[".cbz", ".cbr", ".cb7"];

pub fn reading_kind(kind: &str) -> bool {
    kind == "manga" || kind == "comic"
}

/// One file of a download on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    /// Absolute.
    pub path: PathBuf,
    /// Inside the torrent.
    pub rel: String,
    pub length: i64,
}

/// One file to place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportItem {
    pub src: PathBuf,
    pub dst: PathBuf,
    pub release: Release,
    /// `dst` is already there (an earlier import); never overwritten.
    pub exists: bool,
}

/// Parses one name (the `lain.release.parse@1` call).
pub type Parser = dyn Fn(&str, &str) -> Release + Send + Sync;

/// Decides where each media file goes. `titles` are the library's
/// existing titles, so a new episode lands under the folder the show
/// already uses.
pub fn plan_import(
    lib: &Library,
    files: &[File],
    grab: &Release,
    parse: &Parser,
    titles: &[String],
) -> Result<Vec<ImportItem>> {
    let reading = reading_kind(&lib.kind);
    let exts = if reading { READING_EXTS } else { VIDEO_EXTS };
    let mut media: Vec<&File> =
        files.iter().filter(|f| exts.contains(&ext_lower(&f.path.to_string_lossy()).as_str())).collect();
    if media.len() > 1 {
        media.retain(|f| !base_name(&f.rel).to_lowercase().contains("sample"));
    }
    if media.is_empty() {
        let what = if reading { "comic archive" } else { "video" };
        return Err(errf!(CODE_IMPORT, "the download has no {what} files"));
    }
    if lib.kind == "movie" {
        // A movie is its largest video; the rest are extras.
        media.sort_by_key(|x| std::cmp::Reverse(x.length));
        media.truncate(1);
    }
    let by_key: HashMap<String, &String> = titles.iter().map(|t| (loose_key(t), t)).collect();
    // A file belongs to the release that was grabbed: its title and year
    // decide between the parser's readings of the file name (D-279). The
    // library's other titles do not, or a sequel would land in the
    // folder of the show it follows.
    let grabbed = loose_key(&grab.title);
    let single = media.len() == 1;
    let root = Path::new(&lib.path);
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for f in media {
        let name = base_name(&f.path.to_string_lossy());
        let mut r = settle(parse(&name, &lib.kind), |t| !grabbed.is_empty() && loose_key(t) == grabbed, grab.year);
        fill(&mut r, grab, single);
        let title = by_key.get(&loose_key(&r.title)).map(|t| t.to_string()).unwrap_or_else(|| r.title.clone());
        let rel = destination(&lib.kind, &title, &r, &name);
        let dst = root.join(&rel);
        if !within(root, &dst) {
            return Err(errf!(CODE_IMPORT, "refusing a path outside the library: {}", rel.display()));
        }
        if !seen.insert(dst.clone()) {
            return Err(errf!(CODE_IMPORT, "two files would land on {}", rel.display()));
        }
        let exists = fs::symlink_metadata(&dst).is_ok();
        out.push(ImportItem { src: f.path.clone(), dst, release: r, exists });
    }
    Ok(out)
}

/// Completes a file's parse with what the release name said: packs name
/// the show once, files often only carry the episode.
fn fill(r: &mut Release, grab: &Release, single: bool) {
    if r.title.is_empty() {
        r.title = grab.title.clone();
    }
    if r.year == 0 {
        r.year = grab.year;
    }
    if r.season == 0 && grab.season > 0 {
        r.season = grab.season;
        if !r.episodes.is_empty() && r.absolute {
            r.absolute = false;
        }
    }
    if single {
        if r.episodes.is_empty() {
            r.episodes = grab.episodes.clone();
            r.absolute = grab.absolute;
        }
        if r.volume == 0 {
            r.volume = grab.volume;
        }
        if r.chapter == 0 {
            r.chapter = grab.chapter;
        }
    }
    if r.group.is_empty() {
        r.group = grab.group.clone();
    }
    if r.title.is_empty() {
        r.title = "Unknown".into();
    }
}

/// The naming scheme v1 (A-11).
pub fn destination(kind: &str, title: &str, r: &Release, original: &str) -> PathBuf {
    let ext = ext_lower(original);
    let t = safe_component(title);
    let p = |parts: &[&str]| parts.iter().collect::<PathBuf>();
    if kind == "movie" {
        let name = if r.year > 0 { format!("{t} ({})", r.year) } else { t.clone() };
        return p(&[&name, &format!("{name}{ext}")]);
    }
    if reading_kind(kind) {
        let mut parts = Vec::new();
        if r.volume > 0 {
            parts.push(format!("v{:02}", r.volume));
        }
        if r.chapter > 0 {
            parts.push(format!("c{:03}", r.chapter));
        }
        if parts.is_empty() {
            return p(&[&t, &safe_component(original)]);
        }
        return p(&[&t, &format!("{t} {}{ext}", parts.join(" "))]);
    }
    if r.episodes.is_empty() {
        if r.season > 0 {
            return p(&[&t, &format!("Season {:02}", r.season), &safe_component(original)]);
        }
        return p(&[&t, &safe_component(original)]);
    }
    let span = episode_span(&r.episodes);
    let mut eps = format!("E{:02}", span[0]);
    if span.len() > 1 {
        eps += &format!("-E{:02}", span[span.len() - 1]);
    }
    if r.season > 0 && !r.absolute {
        return p(&[&t, &format!("Season {:02}", r.season), &format!("{t} - S{:02}{eps}{ext}", r.season)]);
    }
    // Absolute numbering is season 0 in the catalog: the anime identifier
    // reads "[Group] Title - 03" that way, and "S00E03" without a group.
    if !r.group.is_empty() && span.len() == 1 {
        let width = match span[0] {
            n if n >= 1000 => 4,
            n if n >= 100 => 3,
            _ => 2,
        };
        return p(&[&t, &format!("[{}] {t} - {:0width$}{ext}", safe_component(&r.group), span[0])]);
    }
    p(&[&t, &format!("{t} - S00{eps}{ext}")])
}

/// The first and last of a run: one file holding several episodes is
/// named E02-E03 (the identifier reads the first).
fn episode_span(eps: &[i64]) -> Vec<i64> {
    if eps.len() <= 2 { eps.to_vec() } else { vec![eps[0], eps[eps.len() - 1]] }
}

/// A title usable as one path component everywhere.
pub fn safe_component(s: &str) -> String {
    let mapped: String = s
        .chars()
        .filter_map(|c| match c {
            c if (c as u32) < 0x20 || c == '\u{7f}' => None,
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => Some(' '),
            c => Some(c),
        })
        .collect();
    let mut s = mapped.split_whitespace().collect::<Vec<_>>().join(" ");
    s = s.trim_matches(|c| c == '.' || c == ' ').to_string();
    if s.len() > 180 {
        s = crate::util::clip(&s, 180).trim().to_string();
    }
    if s.is_empty() || s == "." || s == ".." {
        s = "Unknown".into();
    }
    s
}

/// Puts `src` at `dst`: a hardlink when possible (seeding continues on
/// the same bytes), else a copy; move renames (or copies then removes).
/// Never overwrites `dst`. Answers the mode actually used.
pub fn place(src: &Path, dst: &Path, mode: &str) -> Result<&'static str> {
    if fs::symlink_metadata(dst).is_ok() {
        return Err(errf!(CODE_IMPORT, "{} already exists", dst.display()));
    }
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).map_err(|e| errf!(CODE_IMPORT, "create folder: {e}"))?;
    }
    match mode {
        IMPORT_HARDLINK => {
            if fs::hard_link(src, dst).is_ok() {
                return Ok(IMPORT_HARDLINK);
            }
            // Different filesystems (or no link support): copy instead.
            copy_file(src, dst).map(|_| IMPORT_COPY)
        }
        IMPORT_MOVE => {
            if fs::rename(src, dst).is_ok() {
                return Ok(IMPORT_MOVE);
            }
            copy_file(src, dst)?;
            let _ = fs::remove_file(src);
            Ok(IMPORT_MOVE)
        }
        _ => copy_file(src, dst).map(|_| IMPORT_COPY),
    }
}

fn copy_file(src: &Path, dst: &Path) -> Result<()> {
    let mut input = fs::File::open(src).map_err(|e| errf!(CODE_IMPORT, "open source: {e}"))?;
    let tmp = PathBuf::from(format!("{}.lain-import", dst.display()));
    let mut out = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .map_err(|e| errf!(CODE_IMPORT, "create: {e}"))?;
    let copied = io::copy(&mut input, &mut out).and_then(|_| out.sync_all());
    drop(out);
    if let Err(e) = copied {
        let _ = fs::remove_file(&tmp);
        return Err(errf!(CODE_IMPORT, "copy: {e}"));
    }
    // Link the finished temp file into place: unlike rename it fails if
    // dst appeared meanwhile, so an existing file is never replaced.
    match fs::hard_link(&tmp, dst) {
        Ok(()) => {
            let _ = fs::remove_file(&tmp);
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            let _ = fs::remove_file(&tmp);
            Err(errf!(CODE_IMPORT, "{} already exists", dst.display()))
        }
        Err(_) => {
            // No hardlinks on this filesystem: check, then rename.
            if fs::symlink_metadata(dst).is_ok() {
                let _ = fs::remove_file(&tmp);
                return Err(errf!(CODE_IMPORT, "{} already exists", dst.display()));
            }
            fs::rename(&tmp, dst).map_err(|e| {
                let _ = fs::remove_file(&tmp);
                errf!(CODE_IMPORT, "finish copy: {e}")
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::CODE_IMPORT;
    use lain_identify::{Candidate, identify_anime, identify_comic, identify_generic};

    fn parse(s: &str, kind: &str) -> Release {
        lain_identify::tokenize(s, kind)
    }

    fn tmp(tag: &str) -> PathBuf {
        lain_store::test_dir(&format!("acq-import-{tag}"))
    }

    fn touch(p: &Path, n: usize) -> File {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, vec![0u8; n]).unwrap();
        File { path: p.into(), rel: base_name(&p.to_string_lossy()), length: n as i64 }
    }

    #[test]
    #[allow(clippy::type_complexity)]
    fn plan_import_naming() {
        let src = tmp("naming-src");
        let cases: Vec<(&str, Vec<&str>, &str, Vec<&str>, Vec<&str>)> = vec![
            (
                "anime",
                vec!["[Fansub-A] Sousou no Frieren - 05 [1080p].mkv"],
                "[Fansub-A] Sousou no Frieren - 05 [1080p]",
                vec![],
                vec!["Sousou no Frieren/[Fansub-A] Sousou no Frieren - 05.mkv"],
            ),
            (
                "anime",
                vec!["[Fansub-A] One Piece - 1100 [720p].mkv"],
                "[Fansub-A] One Piece - 1100",
                vec!["ONE PIECE"],
                vec!["ONE PIECE/[Fansub-A] ONE PIECE - 1100.mkv"],
            ),
            (
                "series",
                vec![
                    "Show.Name.S01E02.1080p.WEB-DL.x264-GRP.mkv",
                    "Show.Name.S01E03.1080p.WEB-DL.x264-GRP.mkv",
                    "sample.mkv",
                ],
                "Show.Name.S01.1080p.WEB-DL.x264-GRP",
                vec!["Show: Name"],
                vec!["Show Name/Season 01/Show Name - S01E02.mkv", "Show Name/Season 01/Show Name - S01E03.mkv"],
            ),
            (
                "movie",
                vec!["Movie.Title.2019.2160p.BluRay.x265-GRP.mkv", "Featurette.mkv"],
                "Movie.Title.2019.2160p.BluRay.x265-GRP",
                vec![],
                vec!["Movie Title (2019)/Movie Title (2019).mkv"],
            ),
            (
                "manga",
                vec!["Frieren v01 c001.cbz", "notes.txt"],
                "Frieren v01",
                vec![],
                vec!["Frieren/Frieren v01 c001.cbz"],
            ),
            ("comic", vec!["Saga 054.cbr"], "Saga #054 (2018)", vec![], vec!["Saga/Saga c054.cbr"]),
            ("anime", vec!["Show - 07.mkv"], "Show - 07 [1080p]", vec![], vec!["Show/Show - S00E07.mkv"]),
            (
                "series",
                vec!["Show.S02E05E06.720p.mkv"],
                "Show.S02E05E06.720p",
                vec![],
                vec!["Show/Season 02/Show - S02E05-E06.mkv"],
            ),
        ];
        for (i, (kind, names, grab, titles, want)) in cases.into_iter().enumerate() {
            let lib = Library {
                id: "lib".into(),
                kind: kind.into(),
                path: tmp(&format!("naming-lib{i}")).join("lib").to_string_lossy().into(),
                ..Default::default()
            };
            let files: Vec<File> =
                names.iter().enumerate().map(|(j, f)| touch(&src.join(kind).join(f), 1000 - j)).collect();
            let titles: Vec<String> = titles.iter().map(|s| s.to_string()).collect();
            let plan = plan_import(&lib, &files, &parse(grab, kind), &parse, &titles).unwrap();
            let got: Vec<String> = plan
                .iter()
                .map(|it| it.dst.strip_prefix(&lib.path).unwrap().to_string_lossy().replace('\\', "/"))
                .collect();
            assert_eq!(got, want, "case {i}");
        }
    }

    #[test]
    fn plan_import_reads_a_file_as_the_grabbed_title() {
        use lain_api::ReleaseIdentity;
        // A parser that reads "Show 2 - 03" as the title "Show 2", and
        // offers "Show", season 2 (of 2014), as its other reading.
        let two = |s: &str, kind: &str| {
            let mut r = lain_identify::tokenize(s, kind);
            if s.starts_with("Show 2 - ") {
                r.alternatives = vec![
                    ReleaseIdentity {
                        title: "Show".into(),
                        year: 2014,
                        season: 2,
                        episodes: r.episodes.clone(),
                        ..Default::default()
                    },
                    ReleaseIdentity { title: "Other".into(), ..Default::default() },
                ];
                r.title = "Show 2".into();
            }
            r
        };
        let src = tmp("settle-src");
        let lib = Library {
            id: "lib".into(),
            kind: "anime".into(),
            path: tmp("settle-lib").to_string_lossy().into(),
            ..Default::default()
        };
        let files = [touch(&src.join("Show 2 - 03.mkv"), 10)];
        let titles = ["Other".to_string()];
        let plan_of = |grab: &str, year| {
            let grab = Release { title: grab.into(), year, ..Default::default() };
            let p = plan_import(&lib, &files, &grab, &two, &titles).unwrap();
            assert!(p[0].release.alternatives.is_empty(), "advice is not stored");
            (p[0].release.title.clone(), p[0].release.season, p[0].release.episodes.clone())
        };
        let plan = |grab: &str| plan_of(grab, 0);
        assert_eq!(plan("Show"), ("Show".into(), 2, vec![3]), "the grab names the show");
        assert_eq!(plan("Show 2"), ("Show 2".into(), 0, vec![3]), "the answer already is the grab");
        assert_eq!(plan("Silver Tide"), ("Show 2".into(), 0, vec![3]), "a library title alone decides nothing");
        assert_eq!(plan_of("Show", 2015), ("Show".into(), 2, vec![3]), "a year either way is the grab's");
        assert_eq!(plan_of("Show", 2009), ("Show 2".into(), 0, vec![3]), "another year is another show");
    }

    #[test]
    fn plan_import_rejects_wrong_media() {
        let src = tmp("wrong");
        let lib = Library {
            id: "lib".into(),
            kind: "manga".into(),
            path: tmp("wrong-lib").to_string_lossy().into(),
            ..Default::default()
        };
        let f = touch(&src.join("Show - 01.mkv"), 10);
        let grab = Release { title: "Show".into(), ..Default::default() };
        assert_eq!(plan_import(&lib, &[f], &grab, &parse, &[]).unwrap_err().code, CODE_IMPORT);
    }

    #[test]
    fn safe_components() {
        for (input, want) in [
            ("../../etc", "etc"),
            ("a/b\\c:d*e?", "a b c d e"),
            ("  ..  ", "Unknown"),
            ("Show\u{0}Name", "ShowName"),
            ("Re:Zero Season", "Re Zero Season"),
        ] {
            assert_eq!(safe_component(input), want, "{input:?}");
        }
    }

    #[test]
    fn place_never_overwrites() {
        let dir = tmp("place");
        let src = dir.join("src.mkv");
        fs::write(&src, b"payload").unwrap();
        let dst = dir.join("lib/Show/Show - 01.mkv");
        assert_eq!(place(&src, &dst, IMPORT_HARDLINK).unwrap(), IMPORT_HARDLINK);
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(fs::metadata(&src).unwrap().ino(), fs::metadata(&dst).unwrap().ino());
        }
        assert_eq!(place(&src, &dst, IMPORT_COPY).unwrap_err().code, CODE_IMPORT);
        let dst2 = dir.join("lib/Show/Show - 02.mkv");
        place(&src, &dst2, IMPORT_COPY).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_ne!(fs::metadata(&src).unwrap().ino(), fs::metadata(&dst2).unwrap().ino());
        }
        let left: Vec<_> = fs::read_dir(dir.join("lib/Show"))
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".lain-import"))
            .collect();
        assert!(left.is_empty());
        let dst3 = dir.join("lib/Show/Show - 03.mkv");
        place(&src, &dst3, IMPORT_MOVE).unwrap();
        assert!(!src.exists());
    }

    #[test]
    fn imported_names_round_trip() {
        let r = |title: &str, season, eps: Vec<i64>, absolute, group: &str, volume, chapter| Release {
            title: title.into(),
            season,
            episodes: eps,
            absolute,
            group: group.into(),
            volume,
            chapter,
            ..Default::default()
        };
        let cases = [
            ("anime", r("Sousou no Frieren", 0, vec![5], true, "Fansub-A", 0, 0)),
            ("anime", r("One Piece", 0, vec![1100], true, "Fansub-B", 0, 0)),
            ("anime", r("Show", 0, vec![7], true, "", 0, 0)),
            ("anime", r("Show", 0, vec![1, 2], true, "Fansub-A", 0, 0)),
            ("anime", r("Show", 2, vec![3], false, "", 0, 0)),
            ("series", r("Show Name", 1, vec![2], false, "", 0, 0)),
            ("series", r("Show Name", 1, vec![5, 6], false, "", 0, 0)),
            ("manga", r("Frieren", 0, vec![], false, "", 1, 1)),
            ("manga", r("Frieren", 0, vec![], false, "", 3, 0)),
            ("comic", r("Saga", 0, vec![], false, "", 0, 54)),
        ];
        for (kind, rel) in cases {
            let ext = if reading_kind(kind) { ".cbz" } else { ".mkv" };
            let dest = destination(kind, &rel.title, &rel, &format!("x{ext}"));
            let path = Path::new("/lib").join(&dest).to_string_lossy().replace('\\', "/");
            let cand = Candidate { path, library_type: kind.into(), ..Default::default() };
            if reading_kind(kind) {
                let p = identify_comic(&cand).unwrap();
                assert_eq!((p.title.as_str(), p.season, p.episode), (rel.title.as_str(), rel.volume, rel.chapter));
                continue;
            }
            let p = identify_anime(&cand).unwrap_or_else(|| panic!("{} unidentified", dest.display()));
            let season = if rel.absolute { 0 } else { rel.season };
            assert_eq!(
                (p.title.as_str(), p.season, p.episode),
                (rel.title.as_str(), season, rel.episodes[0]),
                "{}",
                dest.display()
            );
        }
        let dest = destination("movie", "Movie Title", &Release { year: 2019, ..Default::default() }, "x.mkv");
        let cand = Candidate { path: Path::new("/lib").join(dest).to_string_lossy().into(), ..Default::default() };
        assert_eq!(identify_generic(&cand).title, "Movie Title");
    }
}
