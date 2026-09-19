#!/usr/bin/env bash
# Generate the synthetic media library used by web/e2e/smoke.mjs.
# No real release names, no copyrighted content: ffmpeg test patterns
# and a public-domain title, plus a local NFO sidecar.
#
#   web/e2e/fixtures.sh /tmp/lain-e2e-fixtures
set -euo pipefail

DIR="${1:-/tmp/lain-e2e-fixtures}"
command -v ffmpeg >/dev/null || {
	echo "ffmpeg is required to generate the fixtures" >&2
	exit 1
}

rm -rf "$DIR"
mkdir -p "$DIR/Anime/Frieren" "$DIR/Anime/Other Show" "$DIR/Movies"

webm() {
	local out="$1" freq="$2"
	local vtt
	vtt="$(mktemp)"
	cat > "$vtt" <<'EOF'
WEBVTT

00:00:01.000 --> 00:00:04.000
Procedural subtitle line one

00:00:05.000 --> 00:00:08.000
Procedural subtitle line two
EOF
	# The WebVTT track makes the file exercise on-the-fly subtitle
	# extraction (D-047) during direct play, not only transcode sidecars.
	ffmpeg -hide_banner -loglevel error \
		-f lavfi -i "testsrc2=size=640x360:rate=24" \
		-f lavfi -i "sine=frequency=${freq}:sample_rate=48000" \
		-i "$vtt" \
		-map 0:v -map 1:a -map 2:s \
		-t 30 -c:v libvpx-vp9 -deadline realtime -cpu-used 8 -row-mt 1 -b:v 400k \
		-c:a libopus -b:a 64k -c:s webvtt "$out"
	rm -f "$vtt"
}

# Browser-playable direct-play files (webm is decodable everywhere).
# The bracketed group is a fictional placeholder (AGENTS.md naming
# hygiene); it is also what makes the identifier read these as one show
# with episodes instead of two unrelated "Frieren 01/02" files, which is
# what the title page and the player sidebar are about.
webm "$DIR/Anime/Frieren/[Fansub-A] Frieren - 01.webm" 440
webm "$DIR/Anime/Frieren/[Fansub-A] Frieren - 02.webm" 523

# A container the browser plan honestly reports as transcode-required.
ffmpeg -hide_banner -loglevel error \
	-f lavfi -i "testsrc2=size=320x180:rate=12" -t 10 \
	-c:v libx264 -pix_fmt yuv420p -preset ultrafast \
	"$DIR/Anime/Other Show/[Fansub-A] Other Show - 01.mkv"

# A direct-play mp4 for the movies library.
ffmpeg -hide_banner -loglevel error \
	-f lavfi -i "testsrc2=size=640x360:rate=24" \
	-f lavfi -i "sine=frequency=330:sample_rate=48000" \
	-t 30 -c:v libx264 -pix_fmt yuv420p -preset ultrafast -c:a aac -b:a 64k \
	-movflags +faststart "$DIR/Movies/Some Movie (2019).mp4"

cat > "$DIR/Anime/Frieren/tvshow.nfo" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<tvshow>
  <title>Frieren: Beyond Journey's End</title>
  <plot>An elf mage revisits old friends after the hero's party saves the world.</plot>
  <premiered>2023-09-29</premiered>
  <genre>Adventure</genre><genre>Fantasy</genre>
  <thumb aspect="poster">data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='240' height='360'%3E%3Crect width='240' height='360' fill='%23122a33'/%3E%3Ccircle cx='120' cy='180' r='56' fill='%235ce1c4' opacity='0.5'/%3E%3C/svg%3E</thumb>
</tvshow>
EOF

echo "fixtures at $DIR"
