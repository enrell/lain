//! HLS packaging (Go's `transcode/hls.go`): ffmpeg writes segments and
//! a raw event playlist; the server owns the playlist the client reads,
//! so throttling, segment deletion and idle cleanup agree with what the
//! player can fetch. Every helper works on a session directory.

use std::path::Path;

pub const RAW_PLAYLIST: &str = "raw.m3u8";
pub const INDEX_PLAYLIST: &str = "index.m3u8";

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
pub fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_file_name(format!("{}.tmp", path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()));
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
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

/// The bytes of every file in a session directory.
pub fn dir_size(dir: &Path) -> i64 {
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    rd.flatten().filter_map(|e| e.metadata().ok()).filter(|m| m.is_file()).map(|m| m.len() as i64).sum()
}

/// N of `segNNNNN.m4s` or `segNNNNN.ts`.
pub fn segment_index(name: &str) -> Option<i64> {
    let rest = name.strip_prefix("seg")?;
    let digits = rest.strip_suffix(".m4s").or_else(|| rest.strip_suffix(".ts"))?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// The servable names of a session: the playlist, the init segment and
/// media segments; nothing else (no raw playlist, no traversal).
pub fn valid_file(name: &str) -> bool {
    name == INDEX_PLAYLIST || name == "init.mp4" || segment_index(name).is_some()
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
    fn names() {
        for ok in ["index.m3u8", "init.mp4", "seg00000.m4s", "seg00042.ts"] {
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
        ] {
            assert!(!valid_file(bad), "{bad}");
        }
        assert_eq!(segment_index("seg00007.m4s"), Some(7));
    }
}
