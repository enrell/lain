//! HLS packaging (Go's `transcode/hls.go`): ffmpeg writes segments and
//! a raw event playlist; the server owns the playlist the client reads,
//! so throttling, segment deletion and idle cleanup agree with what the
//! player can fetch. Every helper works on a session directory.

use std::path::Path;

pub const RAW_PLAYLIST: &str = "raw.m3u8";
pub const INDEX_PLAYLIST: &str = "index.m3u8";
pub const INIT_SEGMENT: &str = "init.mp4";
/// The name prefix of a software retry's init segment and media segments
/// (`sw-init.mp4`, `sw-seg00000.m4s`). A retry's files never take a URL
/// the failed hardware attempt served: a player keeps an init segment
/// whose URI did not change (hls.js reuses it across playlist reloads)
/// and the browser caches segments for an hour, so under the old names a
/// viewer of the failed attempt would decode one encoder's segments with
/// the other's init segment.
pub const RETRY_PREFIX: &str = "sw-";

/// The init segment's name for a first attempt or a software retry.
pub fn init_name(retry: bool) -> String {
    if retry { format!("{RETRY_PREFIX}{INIT_SEGMENT}") } else { INIT_SEGMENT.to_string() }
}

/// `init.mp4` or `sw-init.mp4`.
pub fn is_init(name: &str) -> bool {
    name.strip_prefix(RETRY_PREFIX).unwrap_or(name) == INIT_SEGMENT
}

#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub uri: String,
    pub duration: f64,
    pub start_sec: f64,
    pub end_sec: f64,
    pub media_seq: i64,
    pub discontinuity: bool,
}

/// Parses an ffmpeg-written playlist: segments and whether it ended.
/// Unknown tags are ignored; malformed durations count as 0.
pub fn parse_playlist(text: &str) -> (Vec<Segment>, bool) {
    let mut out = Vec::new();
    let (mut pending_dur, mut pending_seq, mut pending_disc) = (0.0f64, 0i64, false);
    let mut elapsed = 0.0f64;
    let mut ended = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(v) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            pending_seq = v.trim().parse().unwrap_or(0);
        } else if let Some(v) = line.strip_prefix("#EXTINF:") {
            let v = v.split(',').next().unwrap_or("");
            let d = v.trim().parse::<f64>().unwrap_or(0.0);
            pending_dur = if d.is_finite() && d >= 0.0 { d } else { 0.0 };
        } else if line == "#EXT-X-DISCONTINUITY" {
            pending_disc = true;
        } else if line == "#EXT-X-ENDLIST" {
            ended = true;
        } else if line.starts_with('#') {
        } else {
            let start = elapsed;
            elapsed += pending_dur;
            out.push(Segment {
                uri: line.to_string(),
                duration: pending_dur,
                start_sec: start,
                end_sec: elapsed,
                media_seq: pending_seq,
                discontinuity: pending_disc,
            });
            pending_seq += 1;
            pending_dur = 0.0;
            pending_disc = false;
        }
    }
    (out, ended)
}

fn read(path: &Path) -> Option<String> {
    std::fs::read(path).ok().map(|b| String::from_utf8_lossy(&b).into_owned())
}

fn base_name(uri: &str) -> &str {
    uri.rsplit(['/', '\\']).next().unwrap_or(uri)
}

/// The `#EXT-X-MAP` URI of a playlist, if any (mpegts has none).
pub fn map_uri(text: &str) -> Option<String> {
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("#EXT-X-MAP:") else { continue };
        for part in rest.split(',') {
            if let Some(v) = part.trim().strip_prefix("URI=") {
                return Some(v.trim_matches('"').to_string());
            }
        }
    }
    None
}

/// Writes `path` through a temp file and a rename.
///
/// The temp name is unique per call: the ticker and a finishing run
/// rewrite the same playlist concurrently, and on Windows two writers
/// of one temp file (or a rename racing a reader) fail outright, so the
/// rename is retried briefly too.
pub fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = path.with_file_name(format!("{name}.{}-{n}.tmp", std::process::id()));
    std::fs::write(&tmp, data)?;
    let mut last = None;
    for _ in 0..5 {
        match std::fs::rename(&tmp, path) {
            Ok(()) => return Ok(()),
            Err(e) => last = Some(e),
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let _ = std::fs::remove_file(&tmp);
    Err(last.unwrap_or_else(|| std::io::Error::other("rename failed")))
}

/// Regenerates the client playlist from the raw one: segments that no
/// longer exist are dropped, ENDLIST appears once ffmpeg finished.
pub fn write_index(dir: &Path, fallback_target: i64) -> std::io::Result<()> {
    let raw = read(&dir.join(RAW_PLAYLIST)).ok_or_else(|| std::io::Error::other("no raw playlist"))?;
    let (segments, ended) = parse_playlist(&raw);
    let kept: Vec<Segment> = segments.into_iter().filter(|s| dir.join(base_name(&s.uri)).exists()).collect();
    let mut b = String::from("#EXTM3U\n#EXT-X-VERSION:7\n");
    let mut target = kept.iter().map(|s| (s.duration + 0.5) as i64).max().unwrap_or(0);
    if target == 0 {
        target = fallback_target;
    }
    b.push_str(&format!("#EXT-X-TARGETDURATION:{target}\n#EXT-X-PLAYLIST-TYPE:EVENT\n#EXT-X-INDEPENDENT-SEGMENTS\n"));
    if let Some(m) = map_uri(&raw) {
        b.push_str(&format!("#EXT-X-MAP:URI=\"{}\"\n", base_name(&m)));
    }
    if let Some(first) = kept.first() {
        b.push_str(&format!("#EXT-X-MEDIA-SEQUENCE:{}\n", first.media_seq));
    }
    for (i, s) in kept.iter().enumerate() {
        if s.discontinuity && i > 0 {
            b.push_str("#EXT-X-DISCONTINUITY\n");
        }
        b.push_str(&format!("#EXTINF:{:.3},\n{}\n", s.duration, base_name(&s.uri)));
    }
    if ended {
        b.push_str("#EXT-X-ENDLIST\n");
    }
    write_atomic(&dir.join(INDEX_PLAYLIST), b.as_bytes())
}

/// Produced segment count and total duration (raw playlist).
pub fn produced(dir: &Path) -> (usize, f64) {
    let Some(raw) = read(&dir.join(RAW_PLAYLIST)) else { return (0, 0.0) };
    let (segs, _) = parse_playlist(&raw);
    (segs.len(), segs.iter().map(|s| s.duration).sum())
}

/// The client can start: the index exists with a retained segment and,
/// when it names one, the init segment is present.
pub fn playable(dir: &Path) -> bool {
    let Some(index) = read(&dir.join(INDEX_PLAYLIST)) else { return false };
    if let Some(m) = map_uri(&index)
        && !dir.join(base_name(&m)).exists()
    {
        return false;
    }
    !parse_playlist(&index).0.is_empty()
}

/// The playback time covered up to segment `index`. An index past the
/// produced segments means caught up (never 0, which would strand the
/// throttle paused).
pub fn client_position_sec(dir: &Path, index: i64) -> f64 {
    let Some(raw) = read(&dir.join(RAW_PLAYLIST)) else { return 0.0 };
    let (segs, _) = parse_playlist(&raw);
    if index < 0 || segs.is_empty() {
        return 0.0;
    }
    match segs.get(index as usize) {
        Some(s) => s.end_sec,
        None => segs.last().map(|s| s.end_sec).unwrap_or(0.0),
    }
}

/// Removes segments that end before the client minus the kept window,
/// then rewrites the index. True when something was removed.
pub fn delete_consumed(dir: &Path, client_segment: i64, keep_sec: i64, fallback_target: i64) -> bool {
    let Some(raw) = read(&dir.join(RAW_PLAYLIST)) else { return false };
    let (segs, _) = parse_playlist(&raw);
    let client =
        if client_segment >= 0 { segs.get(client_segment as usize).map(|s| s.end_sec).unwrap_or(0.0) } else { 0.0 };
    let cutoff = client - keep_sec as f64;
    let mut removed = false;
    for s in &segs {
        if s.end_sec <= cutoff && std::fs::remove_file(dir.join(base_name(&s.uri))).is_ok() {
            removed = true;
        }
    }
    if removed && let Err(e) = write_index(dir, fallback_target) {
        tracing::debug!(err = %e, "hls playlist rewrite failed");
    }
    removed
}

/// Whether `name` is part of an encode's HLS output: the playlists, the
/// init segment, the media segments, and the temp files ffmpeg
/// (`temp_file`) and [`write_atomic`] write them through. A relocated
/// session keeps its subtitle sidecar and burn-in file in the same
/// directory; those are not.
fn is_output(name: &str) -> bool {
    let product = |n: &str| n == RAW_PLAYLIST || n == INDEX_PLAYLIST || is_init(n) || segment_index(n).is_some();
    // `<name>.tmp` (ffmpeg) or `<name>.<pid>-<n>.tmp` (write_atomic).
    product(name)
        || name
            .strip_suffix(".tmp")
            .is_some_and(|n| product(n) || n.rsplit_once('.').is_some_and(|(stem, _)| product(stem)))
}

/// Removes an encode's HLS output from a session directory, so another
/// attempt starts from nothing. The raw playlist goes first, so nothing
/// can publish the old segments again, then the client playlist; a file
/// already gone is fine.
pub fn clear_output(dir: &Path) -> std::io::Result<()> {
    let gone = |r: std::io::Result<()>| match r {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    };
    gone(std::fs::remove_file(dir.join(RAW_PLAYLIST)))?;
    gone(std::fs::remove_file(dir.join(INDEX_PLAYLIST)))?;
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    for e in rd {
        let e = e?;
        if is_output(&e.file_name().to_string_lossy()) {
            gone(std::fs::remove_file(e.path()))?;
        }
    }
    Ok(())
}

/// The bytes of every file in a session directory.
pub fn dir_size(dir: &Path) -> i64 {
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    rd.flatten().filter_map(|e| e.metadata().ok()).filter(|m| m.is_file()).map(|m| m.len() as i64).sum()
}

/// N of `segNNNNN.m4s` or `segNNNNN.ts`, with or without a software
/// retry's prefix.
pub fn segment_index(name: &str) -> Option<i64> {
    let rest = name.strip_prefix(RETRY_PREFIX).unwrap_or(name).strip_prefix("seg")?;
    let digits = rest.strip_suffix(".m4s").or_else(|| rest.strip_suffix(".ts"))?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// The servable names of a session: the playlist, the init segment and
/// media segments (a software retry's too); nothing else (no raw
/// playlist, no traversal).
pub fn valid_file(name: &str) -> bool {
    name == INDEX_PLAYLIST || is_init(name) || segment_index(name).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAW: &str = "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:6\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:6.000000,\nseg00000.m4s\n#EXTINF:6.000000,\nseg00001.m4s\n#EXT-X-DISCONTINUITY\n#EXTINF:4.5,\nseg00002.m4s\n";

    #[test]
    fn parses_segments() {
        let (s, ended) = parse_playlist(RAW);
        assert!(!ended);
        assert_eq!(s.len(), 3);
        assert_eq!((s[1].start_sec, s[1].end_sec, s[1].media_seq), (6.0, 12.0, 1));
        assert!(s[2].discontinuity && !s[1].discontinuity);
        let (s, _) = parse_playlist("#EXTINF:NaN,\na\n#EXTINF:-3,\nb\n#EXTINF:inf,\nc\n");
        assert!(s.iter().all(|x| x.duration == 0.0));
        assert_eq!(map_uri(RAW).as_deref(), Some("init.mp4"));
    }

    #[test]
    fn index_drops_missing_segments_and_keeps_breaks() {
        let dir = lain_store::test_dir("hls-index");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(RAW_PLAYLIST), RAW).unwrap();
        assert!(!playable(&dir));
        for f in ["init.mp4", "seg00001.m4s", "seg00002.m4s"] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        write_index(&dir, 6).unwrap();
        let idx = std::fs::read_to_string(dir.join(INDEX_PLAYLIST)).unwrap();
        assert!(!idx.contains("seg00000"));
        assert!(idx.contains("#EXT-X-MEDIA-SEQUENCE:1\n#EXTINF:6.000,\nseg00001.m4s\n#EXT-X-DISCONTINUITY\n#EXTINF:4.500,\nseg00002.m4s\n"), "{idx}");
        assert!(idx.contains("#EXT-X-MAP:URI=\"init.mp4\""));
        assert!(!idx.contains("ENDLIST"));
        assert!(playable(&dir));
        assert_eq!(produced(&dir), (3, 16.5));
        assert_eq!(client_position_sec(&dir, 1), 12.0);
        assert_eq!(client_position_sec(&dir, 99), 16.5, "past the end is caught up");
        // Deleting behind the client keeps the window.
        assert!(delete_consumed(&dir, 2, 4, 6));
        assert!(!dir.join("seg00001.m4s").exists() && dir.join("seg00002.m4s").exists());
        std::fs::remove_file(dir.join("init.mp4")).unwrap();
        assert!(!playable(&dir), "a missing init segment is not playable");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn clearing_removes_only_the_encode_output() {
        let dir = lain_store::test_dir("hls-clear");
        std::fs::create_dir_all(&dir).unwrap();
        let output = [
            "raw.m3u8",
            "raw.m3u8.tmp",
            "index.m3u8",
            "index.m3u8.42-7.tmp",
            "init.mp4",
            "init.mp4.tmp",
            "seg00000.m4s",
            "seg00001.m4s.tmp",
            "seg00002.ts",
            "sw-init.mp4",
            "sw-seg00000.m4s",
            "sw-seg00001.m4s.tmp",
        ];
        // A relocated session keeps its sidecars beside the segments.
        let kept = ["subs.vtt", "subs.vtt.tmp", "burn.ass", "media.mp4", "media.mp4.tmp", "notes.txt"];
        for f in output.iter().chain(&kept) {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        clear_output(&dir).unwrap();
        let mut left: Vec<String> =
            std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        let mut want: Vec<String> = kept.iter().map(|s| s.to_string()).collect();
        want.sort();
        assert_eq!(left, want);
        // Nothing there, or no directory at all, is not an error.
        clear_output(&dir).unwrap();
        clear_output(&dir.join("missing")).unwrap();
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn names() {
        for ok in [
            "index.m3u8",
            "init.mp4",
            "seg00000.m4s",
            "seg00042.ts",
            "sw-init.mp4",
            "sw-seg00003.m4s",
            "sw-seg00004.ts",
        ] {
            assert!(valid_file(ok), "{ok}");
        }
        for bad in [
            "../raw.m3u8",
            "raw.m3u8",
            "seg.ts",
            "seg-1.ts",
            "seg00000.mp4",
            "index.m3u8/../x",
            "seg00000.ts.tmp",
            "seg+1.ts",
            "sw-index.m3u8",
            "sw-raw.m3u8",
            "sw-sw-seg00000.m4s",
            "sw-sw-init.mp4",
            "sw-",
        ] {
            assert!(!valid_file(bad), "{bad}");
        }
        assert_eq!(segment_index("seg00007.m4s"), Some(7));
        assert_eq!(segment_index("sw-seg00007.m4s"), Some(7));
        assert_eq!((init_name(false).as_str(), init_name(true).as_str()), ("init.mp4", "sw-init.mp4"));
        assert!(is_init("init.mp4") && is_init("sw-init.mp4") && !is_init("seg00000.m4s"));
    }
}
