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
	ffmpeg -hide_banner -loglevel error \
		-f lavfi -i "testsrc2=size=640x360:rate=24" \
		-f lavfi -i "sine=frequency=${freq}:sample_rate=48000" \
		-t 30 -c:v libvpx-vp9 -deadline realtime -cpu-used 8 -row-mt 1 -b:v 400k \
		-c:a libopus -b:a 64k "$out"
}

# Browser-playable direct-play files (webm is decodable everywhere).
webm "$DIR/Anime/Frieren/Frieren - 01.webm" 440
webm "$DIR/Anime/Frieren/Frieren - 02.webm" 523

# A container the browser plan honestly reports as transcode-required.
ffmpeg -hide_banner -loglevel error \
	-f lavfi -i "testsrc2=size=320x180:rate=12" -t 10 \
	-c:v libx264 -pix_fmt yuv420p -preset ultrafast \
	"$DIR/Anime/Other Show/Other Show - 01.mkv"

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
