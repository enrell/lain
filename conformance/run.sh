#!/bin/sh
# Runs the conformance suite against both servers, each in its own
# throwaway data dir on its own port; no real data dir is touched.
#
#   conformance/run.sh            # Go and Rust
#   conformance/run.sh go|rust    # one server
#
# Go:   fresh (v1), then scan (v1) on a fresh data dir, then watch (v1)
#       on a scanned library of test clips
# Rust: fresh (v1+v2), scan (v1+v2) on a fresh data dir, then private
#       (private libraries) on a data dir with a `media` root and the
#       fixture catalog (conformance/fixtures/catalog.json,
#       `lain-rs import --in`), then read (comic pages) with `media` at
#       conformance/fixtures/read and conformance/fixtures/read-catalog.json,
#       then watch (v1+v2) over the clips and
#       conformance/fixtures/watch-catalog.json
# The scan suite writes its own fixture media and runs with library
# watching and the post-scan metadata pass off, so no scan races the
# cases; so does read, whose fixture catalog must stay as imported.
# Nothing reaches the network: `lain-conformance upstream` serves the
# recorded answers of the remote APIs (conformance/fixtures/upstream)
# and every server's LAIN_METADATA_<NAME>_URL points at it.
#
# The watch suite needs ffmpeg (conformance/make-clips.sh writes the
# clips); without it the suite is skipped, unless LAIN_REQUIRE_FFMPEG=1
# (CI) makes that a failure.
#
# Env: GO_PORT (19460), RS_PORT (19461), UPSTREAM_PORT (19462), KEEP=1
# keeps the temp dir.
set -eu

which="${1:-both}"
root=$(cd "$(dirname "$0")/.." && pwd)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lain-conformance.XXXXXX")
go_port="${GO_PORT:-19460}"
rs_port="${RS_PORT:-19461}"
upstream_port="${UPSTREAM_PORT:-19462}"
cleanup() {
	if [ -f "$tmp/pids" ]; then
		while read -r p; do kill "$p" 2>/dev/null || true; done <"$tmp/pids"
	fi
	sleep 0.2
	if [ "${KEEP:-0}" = 1 ]; then echo "kept $tmp"; else rm -rf "$tmp"; fi
}
trap cleanup EXIT INT TERM
# The theme cases must not read the host's own Omarchy colors.
LAIN_OMARCHY_COLORS="$tmp/no-colors.toml"
export LAIN_OMARCHY_COLORS

cd "$root"
cargo build -q -p lain-conformance -p lain-server
runner="$root/target/debug/lain-conformance"
lain_rs="$root/target/debug/lain-rs"

# The upstream fixture server, for both servers' network providers.
upstream="http://127.0.0.1:$upstream_port"
"$runner" upstream --port "$upstream_port" >"$tmp/upstream.log" 2>&1 &
echo "$!" >>"$tmp/pids"
i=0
until grep -q "^http://" "$tmp/upstream.log" 2>/dev/null; do
	i=$((i + 1))
	if [ "$i" -gt 100 ]; then echo "upstream fixture server did not start" >&2; cat "$tmp/upstream.log" >&2; exit 1; fi
	sleep 0.1
done
export LAIN_METADATA_KITSU_URL="$upstream/kitsu"
export LAIN_METADATA_ANILIST_URL="$upstream/anilist"
export LAIN_METADATA_JIKAN_URL="$upstream/jikan"
export LAIN_METADATA_TVMAZE_URL="$upstream/tvmaze"

# Fixture media: fictional names only.
mkdir -p "$tmp/media/Frieren" "$tmp/media2" "$tmp/rootmedia/Frieren"
: >"$tmp/rootmedia/Frieren/[Fansub-A] Frieren - 01 [1080p].mkv"
: >"$tmp/rootmedia/Frieren/[Fansub-A] Frieren - 02 [1080p].mkv"
clips=0
if sh conformance/make-clips.sh "$tmp/watch/Clips"; then
	clips=1
elif [ "${LAIN_REQUIRE_FFMPEG:-0}" = 1 ]; then
	echo "LAIN_REQUIRE_FFMPEG=1 but the watch clips could not be made (is ffmpeg installed?)" >&2
	exit 1
fi

wait_up() {
	i=0
	until curl -fsS "$1/health" >/dev/null 2>&1; do
		i=$((i + 1))
		if [ "$i" -gt 100 ]; then echo "server at $1 did not start" >&2; return 1; fi
		sleep 0.1
	done
}

start() { # name url cmd...
	name=$1 url=$2
	shift 2
	"$@" >"$tmp/$name.log" 2>&1 &
	pid=$!
	echo "$pid" >>"$tmp/pids"
	wait_up "$url"
	echo "$pid"
}

stop() {
	kill "$1" 2>/dev/null || true
	while kill -0 "$1" 2>/dev/null; do sleep 0.1; done
}

summary=""
note() { summary="$summary
$1"; }

run_go() {
	# The web UI embed needs its directory (dist is gitignored).
	mkdir -p internal/webui/dist
	[ -e internal/webui/dist/.gitkeep ] || : >internal/webui/dist/.gitkeep
	go build -o "$tmp/lain" ./cmd/lain
	url="http://127.0.0.1:$go_port"
	pid=$(LAIN_AUTO_ENRICH=0 start go-fresh "$url" "$tmp/lain" serve --data-dir "$tmp/go-fresh" --port "$go_port")
	out=$(LAIN_URL=$url "$runner" --tier v1 --var media_dir="$tmp/media" --var media_dir2="$tmp/media2" conformance/cases/fresh) || { echo "$out"; exit 1; }
	echo "$out"
	note "Go   fresh v1: $(echo "$out" | tail -1)"
	stop "$pid"

	mkdir -p "$tmp/go-scanmedia"
	pid=$(LAIN_WATCH=0 LAIN_AUTO_ENRICH=0 start go-scan "$url" "$tmp/lain" serve --data-dir "$tmp/go-scan" --port "$go_port")
	out=$(LAIN_URL=$url "$runner" --tier v1 --var scan_media="$tmp/go-scanmedia" --var upstream="$upstream" conformance/cases/scan) || { echo "$out"; exit 1; }
	echo "$out"
	note "Go   scan v1: $(echo "$out" | tail -1)"
	stop "$pid"

	if [ "$clips" = 1 ]; then
		pid=$(LAIN_WATCH=0 LAIN_AUTO_ENRICH=0 start go-watch "$url" "$tmp/lain" serve --data-dir "$tmp/go-watch" --port "$go_port")
		out=$(LAIN_URL=$url "$runner" --tier v1 --var clips=1 --var scan_dir="$tmp/watch/Clips" conformance/cases/watch) || { echo "$out"; exit 1; }
		echo "$out"
		note "Go   watch v1: $(echo "$out" | tail -1)"
		stop "$pid"
	else
		note "Go   watch v1: skipped (no ffmpeg)"
	fi
}

run_rust() {
	url="http://127.0.0.1:$rs_port"
	pid=$(start rs-fresh "$url" "$lain_rs" serve --data-dir "$tmp/rs-fresh" --port "$rs_port")
	out=$(LAIN_URL=$url "$runner" --tier v1,v2 --var media_dir="$tmp/media" --var media_dir2="$tmp/media2" --var data_dir="$tmp/rs-fresh" conformance/cases/fresh conformance/cases/v2) || { echo "$out"; exit 1; }
	echo "$out"
	note "Rust fresh v1+v2: $(echo "$out" | tail -1)"
	stop "$pid"

	mkdir -p "$tmp/rs-scanmedia"
	pid=$(LAIN_WATCH=0 LAIN_AUTO_ENRICH=0 start rs-scan "$url" "$lain_rs" serve --data-dir "$tmp/rs-scan" --port "$rs_port")
	out=$(LAIN_URL=$url "$runner" --tier v1,v2 --var scan_media="$tmp/rs-scanmedia" --var upstream="$upstream" conformance/cases/scan) || { echo "$out"; exit 1; }
	echo "$out"
	note "Rust scan v1+v2: $(echo "$out" | tail -1)"
	stop "$pid"

	mkdir -p "$tmp/rs-private"
	printf '{"format":"lain.roots@1","roots":{"media":"%s"}}\n' "$tmp/rootmedia" >"$tmp/rs-private/roots.json"
	"$lain_rs" import --in conformance/fixtures/catalog.json --data-dir "$tmp/rs-private"
	pid=$(start rs-private "$url" "$lain_rs" serve --data-dir "$tmp/rs-private" --port "$rs_port")
	out=$(LAIN_URL=$url "$runner" --var media_root="$tmp/rootmedia" --var data_dir="$tmp/rs-private" conformance/cases/private) || { echo "$out"; exit 1; }
	echo "$out"
	note "Rust private libraries: $(echo "$out" | tail -1)"
	stop "$pid"

	mkdir -p "$tmp/rs-read"
	printf '{"format":"lain.roots@1","roots":{"media":"%s"}}\n' "$root/conformance/fixtures/read" >"$tmp/rs-read/roots.json"
	"$lain_rs" import --in conformance/fixtures/read-catalog.json --data-dir "$tmp/rs-read"
	pid=$(LAIN_WATCH=0 LAIN_AUTO_ENRICH=0 start rs-read "$url" "$lain_rs" serve --data-dir "$tmp/rs-read" --port "$rs_port")
	out=$(LAIN_URL=$url "$runner" --var media_root="$root/conformance/fixtures/read" --var data_dir="$tmp/rs-read" conformance/cases/read) || { echo "$out"; exit 1; }
	echo "$out"
	note "Rust read: $(echo "$out" | tail -1)"
	stop "$pid"
	if [ "$clips" = 1 ]; then
		mkdir -p "$tmp/rs-watch"
		printf '{"format":"lain.roots@1","roots":{"media":"%s"}}\n' "$tmp/watch" >"$tmp/rs-watch/roots.json"
		"$lain_rs" import --in conformance/fixtures/watch-catalog.json --data-dir "$tmp/rs-watch"
		pid=$(LAIN_WATCH=0 LAIN_AUTO_ENRICH=0 start rs-watch "$url" "$lain_rs" serve --data-dir "$tmp/rs-watch" --port "$rs_port")
		out=$(LAIN_URL=$url "$runner" --tier v1,v2 --var clips=1 --var media_root="$tmp/watch" conformance/cases/watch) || { echo "$out"; exit 1; }
		echo "$out"
		note "Rust watch v1+v2: $(echo "$out" | tail -1)"
		stop "$pid"
	else
		note "Rust watch v1+v2: skipped (no ffmpeg)"
	fi
}

case "$which" in
go) run_go ;;
rust) run_rust ;;
both) run_go; run_rust ;;
*) echo "usage: $0 [go|rust|both]" >&2; exit 2 ;;
esac
echo
echo "== conformance summary =="
echo "$summary" | sed '/^$/d'
