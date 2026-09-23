// HLS packaging: ffmpeg writes segments and a raw event playlist; the
// plugin owns the playlist the client reads, so throttling, segment
// deletion and idle cleanup stay consistent with what the player can
// actually fetch.
//
// Every helper here works on a directory, because a session's artifacts
// live either flat under the data dir or in a directory of their own
// under the operator's configured temp path.
package transcode

import (
	"bufio"
	"fmt"
	"math"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
)

const (
	hlsRawPlaylist   = "raw.m3u8"
	hlsIndexPlaylist = "index.m3u8"
	hlsInitSegment   = "init.mp4"
	hlsSegmentPrefix = "seg"
	hlsSegmentExt    = ".m4s"
	hlsSegmentExtTS  = ".ts"
)

type hlsSegment struct {
	URI       string
	Duration  float64
	StartSec  float64
	EndSec    float64
	MediaSeq  int
	IsDiscont bool
}

// parseHLSPlaylist extracts media segments from an ffmpeg-written
// playlist. Unknown tags are ignored; only what the server needs to
// filter and time segments is kept.
func parseHLSPlaylist(path string) ([]hlsSegment, bool, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, false, err
	}
	defer f.Close()
	var out []hlsSegment
	var pendingDuration float64
	var pendingSeq int
	var pendingDiscont bool
	var elapsed float64
	mediaSeq := 0
	ended := false
	scanner := bufio.NewScanner(f)
	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		isEmpty := line == ""
		isSeqTag := strings.HasPrefix(line, "#EXT-X-MEDIA-SEQUENCE:")
		isInfTag := strings.HasPrefix(line, "#EXTINF:")
		isDiscont := line == "#EXT-X-DISCONTINUITY"
		isEnd := line == "#EXT-X-ENDLIST"
		isOtherTag := strings.HasPrefix(line, "#")
		switch {
		case isEmpty:
		case isSeqTag:
			mediaSeq, _ = strconv.Atoi(strings.TrimPrefix(line, "#EXT-X-MEDIA-SEQUENCE:"))
			pendingSeq = mediaSeq
		case isInfTag:
			value := strings.TrimPrefix(line, "#EXTINF:")
			if i := strings.IndexByte(value, ','); i >= 0 {
				value = value[:i]
			}
			pendingDuration, _ = strconv.ParseFloat(strings.TrimSpace(value), 64)
			// ParseFloat accepts NaN and ±Inf without error, and a
			// negative EXTINF would push the timeline backwards — all of
			// it poisons the throttle's ahead computation, so clamp to 0
			// like any other malformed value.
			if math.IsNaN(pendingDuration) || math.IsInf(pendingDuration, 0) || pendingDuration < 0 {
				pendingDuration = 0
			}
		case isDiscont:
			// The tag precedes the segment it applies to: the next media
			// segment starts a new timeline.
			pendingDiscont = true
		case isEnd:
			ended = true
		case isOtherTag:
			// header/metadata tags are not needed here
		default:
			seg := hlsSegment{URI: line, Duration: pendingDuration, MediaSeq: pendingSeq, IsDiscont: pendingDiscont}
			seg.StartSec = elapsed
			elapsed += pendingDuration
			seg.EndSec = elapsed
			out = append(out, seg)
			pendingSeq++
			pendingDuration = 0
			pendingDiscont = false
		}
	}
	if err := scanner.Err(); err != nil {
		return nil, false, err
	}
	return out, ended, nil
}

// writeIndexPlaylist regenerates the client-facing playlist from the
// raw one, dropping segments that no longer exist (segment deletion)
// and appending ENDLIST once ffmpeg finished.
func writeIndexPlaylist(dir string, fallbackTarget int) error {
	raw := filepath.Join(dir, hlsRawPlaylist)
	segments, ended, err := parseHLSPlaylist(raw)
	if err != nil {
		return err
	}
	kept := segments[:0]
	for _, seg := range segments {
		if _, err := os.Stat(filepath.Join(dir, filepath.Base(seg.URI))); err == nil {
			kept = append(kept, seg)
		}
	}
	segments = kept

	var b strings.Builder
	b.WriteString("#EXTM3U\n#EXT-X-VERSION:7\n")
	target := 0
	for _, seg := range segments {
		if d := int(seg.Duration + 0.5); d > target {
			target = d
		}
	}
	if target == 0 {
		target = fallbackTarget
	}
	fmt.Fprintf(&b, "#EXT-X-TARGETDURATION:%d\n", target)
	b.WriteString("#EXT-X-PLAYLIST-TYPE:EVENT\n")
	b.WriteString("#EXT-X-INDEPENDENT-SEGMENTS\n")
	// Only CMAF (fMP4) playlists carry an init segment; a TS playlist has
	// no #EXT-X-MAP, so this stays absent for mpegts.
	if mapURI := hlsMapURI(raw); mapURI != "" {
		b.WriteString(fmt.Sprintf("#EXT-X-MAP:URI=%q\n", filepath.Base(mapURI)))
	}
	if len(segments) > 0 {
		fmt.Fprintf(&b, "#EXT-X-MEDIA-SEQUENCE:%d\n", segments[0].MediaSeq)
	}
	for i, seg := range segments {
		// Re-emit the timeline break ffmpeg wrote, so a client resets its
		// timestamps exactly where the source broke instead of stitching
		// across the jump. The first listed segment has nothing before it
		// to be discontinuous from, so the tag is dropped there (segment
		// deletion can move the boundary to the front of the playlist).
		if seg.IsDiscont && i > 0 {
			b.WriteString("#EXT-X-DISCONTINUITY\n")
		}
		fmt.Fprintf(&b, "#EXTINF:%.3f,\n%s\n", seg.Duration, filepath.Base(seg.URI))
	}
	if ended {
		b.WriteString("#EXT-X-ENDLIST\n")
	}
	return writeFileAtomic(filepath.Join(dir, hlsIndexPlaylist), []byte(b.String()))
}

// hlsProduced returns the number of produced segments and their total
// duration from the raw playlist.
func hlsProduced(dir string) (int, float64) {
	segments, _, err := parseHLSPlaylist(filepath.Join(dir, hlsRawPlaylist))
	if err != nil {
		return 0, 0
	}
	total := 0.0
	for _, seg := range segments {
		total += seg.Duration
	}
	return len(segments), total
}

// hlsPlayable reports whether the client can start playing: the served
// index playlist exists with at least one retained segment, and, when it
// references one, the init segment is present. It reads the index (not
// the raw ffmpeg playlist), so a session whose segments were all deleted
// is honestly reported unplayable rather than served as an empty
// playlist. It is container-agnostic: an mpegts playlist has no
// #EXT-X-MAP, so no init segment is required.
func hlsPlayable(dir string) bool {
	index := filepath.Join(dir, hlsIndexPlaylist)
	if _, err := os.Stat(index); err != nil {
		return false
	}
	if mapURI := hlsMapURI(index); mapURI != "" {
		if _, err := os.Stat(filepath.Join(dir, filepath.Base(mapURI))); err != nil {
			return false
		}
	}
	segments, _, err := parseHLSPlaylist(index)
	return err == nil && len(segments) > 0
}

// hlsMapURI returns the #EXT-X-MAP URI a playlist references, or "" for
// a container without an init segment (mpegts).
func hlsMapURI(path string) string {
	f, err := os.Open(path)
	if err != nil {
		return ""
	}
	defer f.Close()
	scanner := bufio.NewScanner(f)
	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if !strings.HasPrefix(line, "#EXT-X-MAP:") {
			continue
		}
		for _, part := range strings.Split(strings.TrimPrefix(line, "#EXT-X-MAP:"), ",") {
			part = strings.TrimSpace(part)
			if strings.HasPrefix(part, "URI=") {
				return strings.Trim(strings.TrimPrefix(part, "URI="), "\"")
			}
		}
	}
	return ""
}

// clientPositionSec converts the highest fetched segment index into the
// playback time it covers. An index past the produced segments (a client
// that fetched everything, or a bogus request) means the client is caught
// up, not at the start; returning 0 there would strand the throttle
// paused forever.
func clientPositionSec(dir string, segmentIndex int) float64 {
	segments, _, err := parseHLSPlaylist(filepath.Join(dir, hlsRawPlaylist))
	if err != nil || segmentIndex < 0 || len(segments) == 0 {
		return 0
	}
	if segmentIndex >= len(segments) {
		return segments[len(segments)-1].EndSec
	}
	return segments[segmentIndex].EndSec
}

// applyThrottle pauses ffmpeg when it produced far more than the client
// has fetched, and resumes once the client catches up. This is the
// Jellyfin "throttle transcodes" behavior; it only ever pauses the
// process, never changes the output.
func (t *Transcoder) applyThrottle(j *job) {
	if !j.settings.Throttle || j.delivery != "hls" {
		return
	}
	t.mu.Lock()
	cmd := j.cmd
	paused := j.paused
	clientSeg := j.clientSegment
	session := j.spec.Session
	settings := j.settings
	t.mu.Unlock()
	if cmd == nil || cmd.Process == nil {
		return
	}
	dir := t.pathsFor(session, settings).hlsDir
	_, producedSec := hlsProduced(dir)
	clientSec := clientPositionSec(dir, clientSeg)
	ahead := producedSec - clientSec
	limit := float64(j.settings.ThrottleAheadSec)
	shouldPause := !paused && ahead > limit
	shouldResume := paused && ahead < limit/2
	switch {
	case shouldPause:
		if err := pauseProcess(cmd.Process); err == nil {
			t.mu.Lock()
			j.paused = true
			t.mu.Unlock()
			t.logger().Debug("transcode throttled", "session", session, "ahead_sec", int(ahead))
		}
	case shouldResume:
		if err := resumeProcess(cmd.Process); err == nil {
			t.mu.Lock()
			j.paused = false
			t.mu.Unlock()
			t.logger().Debug("transcode resumed", "session", session, "ahead_sec", int(ahead))
		}
	}
}

// deleteConsumedSegments removes segments fully behind the client minus
// the retained window, then regenerates the index playlist. Rewind
// beyond the window needs a fresh session, which is the documented
// tradeoff of the setting.
func (t *Transcoder) deleteConsumedSegments(j *job) {
	if !j.settings.SegmentDeletion || j.delivery != "hls" {
		return
	}
	t.mu.Lock()
	clientSeg := j.clientSegment
	session := j.spec.Session
	settings := j.settings
	t.mu.Unlock()
	dir := t.pathsFor(session, settings).hlsDir
	segments, _, err := parseHLSPlaylist(filepath.Join(dir, hlsRawPlaylist))
	if err != nil {
		return
	}
	clientSec := 0.0
	if clientSeg >= 0 && clientSeg < len(segments) {
		clientSec = segments[clientSeg].EndSec
	}
	cutoff := clientSec - float64(j.settings.SegmentKeepSec)
	removed := false
	for _, seg := range segments {
		if seg.EndSec <= cutoff {
			if err := os.Remove(filepath.Join(dir, filepath.Base(seg.URI))); err == nil {
				removed = true
			}
		}
	}
	if removed {
		if err := writeIndexPlaylist(dir, j.settings.HLSSegmentSeconds); err != nil {
			t.logger().Debug("hls playlist rewrite failed", "session", session, "err", err.Error())
		}
	}
}

// hlsDirSize sums every artifact of one HLS session directory.
func hlsDirSize(dir string) int64 {
	var total int64
	entries, err := os.ReadDir(dir)
	if err != nil {
		return 0
	}
	for _, e := range entries {
		if info, err := e.Info(); err == nil && info.Mode().IsRegular() {
			total += info.Size()
		}
	}
	return total
}

// listHLSFiles returns the servable file names of a session directory
// in a stable order (for tests and diagnostics).
func listHLSFiles(dir string) []string {
	entries, err := os.ReadDir(dir)
	if err != nil {
		return nil
	}
	var names []string
	for _, e := range entries {
		if e.Type().IsRegular() {
			names = append(names, e.Name())
		}
	}
	sort.Strings(names)
	return names
}

func writeFileAtomic(path string, data []byte) error {
	tmp := path + ".tmp"
	if err := os.WriteFile(tmp, data, 0o600); err != nil {
		return err
	}
	if err := os.Rename(tmp, path); err != nil {
		_ = os.Remove(tmp)
		return err
	}
	return nil
}

// parseSegmentIndex extracts N from segNNNNN.m4s or segNNNNN.ts.
func parseSegmentIndex(name string) (int, bool) {
	if !strings.HasPrefix(name, hlsSegmentPrefix) {
		return 0, false
	}
	for _, ext := range []string{hlsSegmentExt, hlsSegmentExtTS} {
		if !strings.HasSuffix(name, ext) {
			continue
		}
		digits := strings.TrimSuffix(strings.TrimPrefix(name, hlsSegmentPrefix), ext)
		n, err := strconv.Atoi(digits)
		if err == nil && n >= 0 {
			return n, true
		}
	}
	return 0, false
}
