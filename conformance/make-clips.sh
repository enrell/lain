#!/bin/sh
# Writes the watch suite's tiny test clips into <dir> with ffmpeg's
# lavfi sources (no media is committed):
#   Alpha.mp4  H.264/AAC at 2 Mbit/s, faststart (browser direct play)
#   Beta.mkv   H.264/AAC plus one SRT subtitle track (remux, subtitles)
# Exits 3 when ffmpeg is not installed.
set -eu
dir="$1"
command -v ffmpeg >/dev/null 2>&1 || exit 3
mkdir -p "$dir"
srt="$dir/.subs.srt"
printf '1\n00:00:00,000 --> 00:00:01,500\nHello\n' >"$srt"
ffmpeg -hide_banner -loglevel error -nostdin \
	-f lavfi -i testsrc=size=320x240:rate=15:duration=2 \
	-f lavfi -i sine=frequency=440:sample_rate=48000:duration=2 \
	-c:v libx264 -pix_fmt yuv420p -b:v 2000k -c:a aac -b:a 128k \
	-movflags +faststart -y "$dir/Alpha.mp4"
ffmpeg -hide_banner -loglevel error -nostdin \
	-f lavfi -i testsrc=size=160x120:rate=10:duration=2 \
	-f lavfi -i sine=frequency=330:sample_rate=48000:duration=2 \
	-i "$srt" -map 0:v -map 1:a -map 2:s \
	-c:v libx264 -pix_fmt yuv420p -c:a aac -b:a 96k -c:s srt -y "$dir/Beta.mkv"
rm -f "$srt"
