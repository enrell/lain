package contracts

import (
	"sort"
	"strings"
)

// ClientCapabilities is a client's claim about what it can decode. It is
// deliberately not a verdict about one file: the plan is requested
// before the client knows the file's streams, so the client reports what
// it has verified it can decode and the planner applies that claim to
// the probed streams (D-058).
//
// Tokens are lowercase and come in two shapes:
//
//	"mkv"      the container itself opens
//	"mkv/h264" the codec family decodes inside that container
//
// The pairing is not cosmetic. A browser can decode HEVC through a
// platform decoder inside MP4 and still fail on HEVC inside Matroska —
// measured here as a black screen with no error event, an advancing
// clock and zero decoded frames. A flat codec list would license exactly
// that, so a codec token is only ever claimed for a container the client
// also opened.
//
// A claim never buys more than the plan decision: the player verifies
// the first decoded frame and falls back to a transcode when none
// arrives (D-058), and only tokens this server knows are acted on.
type ClientCapabilities struct {
	Tokens []string `json:"tokens"`
}

// capabilityContainers maps the file extensions this server serves
// directly to the container token a client reports for them. An
// extension absent here is never direct-played, whatever a client
// claims: the gateway would have no honest Content-Type for it.
var capabilityContainers = map[string]string{
	"mp4": "mp4", "m4v": "mp4", "mov": "mp4", "m4a": "mp4",
	"mkv":  "mkv",
	"webm": "webm",
	"ogg":  "ogg", "ogv": "ogg", "oga": "ogg", "opus": "ogg",
	"mp3":  "mp3",
	"flac": "flac",
}

// capabilityCodecs maps the ffprobe codec names that reach this server
// to the family token a client reports for them. Codecs absent here
// (pcm_*, mpeg4 part 2, ...) have no token at all, so no client can
// claim them and they always remux or transcode.
var capabilityCodecs = map[string]string{
	"h264": "h264", "avc1": "h264",
	"hevc": "hevc", "h265": "hevc",
	"vp8": "vp8", "vp9": "vp9", "av1": "av1",
	"aac": "aac", "mp3": "mp3", "opus": "opus", "vorbis": "vorbis",
	"flac": "flac", "ac3": "ac3", "eac3": "eac3", "dts": "dts",
}

// DirectContainer returns the container token for a file extension
// (".mkv" and "mkv" are both accepted). The second result is false when
// this server has no directly playable container for that extension.
func DirectContainer(ext string) (string, bool) {
	container, ok := capabilityContainers[strings.ToLower(strings.TrimPrefix(ext, "."))]
	return container, ok
}

// CapabilityCodec returns the codec family token for an ffprobe codec
// name. The second result is false for codecs no browser decodes.
func CapabilityCodec(codec string) (string, bool) {
	family, ok := capabilityCodecs[strings.ToLower(codec)]
	return family, ok
}

// KnownCapabilityToken reports whether a token is inside this server's
// vocabulary. Everything else is dropped on the way in, so a client
// cannot widen the set the planner reasons about.
func KnownCapabilityToken(tok string) bool {
	container, codec, paired := strings.Cut(tok, "/")
	if !knownContainerToken(container) {
		return false
	}
	if !paired {
		return true
	}
	for _, family := range capabilityCodecs {
		if family == codec {
			return true
		}
	}
	return false
}

func knownContainerToken(tok string) bool {
	for _, container := range capabilityContainers {
		if container == tok {
			return true
		}
	}
	return false
}

// ParseCapabilities normalizes a raw client capability list — one query
// value, or several comma-separated ones — into the set the planner
// acts on: lowercase, deduplicated, sorted, and stripped of tokens this
// server does not know. Unknown tokens are dropped rather than rejected:
// the list is a claim, and a client from a future version must not have
// its request fail for naming something this server cannot use.
//
// A nil result means unknown (nothing was reported) and the planner
// keeps its conservative rules. A non-nil result with no tokens means
// the client claims it can decode nothing, which is a decision.
func ParseCapabilities(raw []string) *ClientCapabilities {
	if len(raw) == 0 {
		return nil
	}
	seen := make(map[string]bool, len(raw))
	tokens := make([]string, 0, len(raw))
	for _, value := range raw {
		for _, tok := range strings.Split(value, ",") {
			tok = strings.ToLower(strings.TrimSpace(tok))
			if tok == "" || seen[tok] || !KnownCapabilityToken(tok) {
				continue
			}
			seen[tok] = true
			tokens = append(tokens, tok)
		}
	}
	sort.Strings(tokens)
	return &ClientCapabilities{Tokens: tokens}
}

// Has reports whether the client claimed a token. A nil capability set
// claims nothing; callers that must tell "unknown" from "empty" check
// for nil before asking.
func (c *ClientCapabilities) Has(token string) bool {
	if c == nil {
		return false
	}
	for _, tok := range c.Tokens {
		if tok == token {
			return true
		}
	}
	return false
}
