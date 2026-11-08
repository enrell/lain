#!/bin/sh
# Runs the conformance suite against both servers, each in its own
# throwaway data dir on its own port; no real data dir is touched.
#
#   conformance/run.sh            # Go and Rust
#   conformance/run.sh go|rust    # one server
#
# Go:   fresh (v1) + seed + seeded
# Rust: fresh (v1+v2) + seeded on a `lain-rs import` of the Go seed
#       (Rust alone: the seed comes from crates/lain-store/testdata)
#
# Env: GO_PORT (19460), RS_PORT (19461), KEEP=1 keeps the temp dir.
set -eu

which="${1:-both}"
root=$(cd "$(dirname "$0")/.." && pwd)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lain-conformance.XXXXXX")
go_port="${GO_PORT:-19460}"
rs_port="${RS_PORT:-19461}"
cleanup() {
	if [ -f "$tmp/pids" ]; then
		while read -r p; do kill "$p" 2>/dev/null || true; done <"$tmp/pids"
	fi
	sleep 0.2
	if [ "${KEEP:-0}" = 1 ]; then echo "kept $tmp"; else rm -rf "$tmp"; fi
}
trap cleanup EXIT INT TERM

cd "$root"
cargo build -q -p lain-conformance -p lain-server
runner="$root/target/debug/lain-conformance"
lain_rs="$root/target/debug/lain-rs"

# Fixture media: fictional names only.
mkdir -p "$tmp/media/Frieren" "$tmp/media2" "$tmp/seedmedia/Frieren"
: >"$tmp/seedmedia/Frieren/[Fansub-A] Frieren - 01 [1080p].mkv"
: >"$tmp/seedmedia/Frieren/[Fansub-A] Frieren - 02 [1080p].mkv"

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

	pid=$(LAIN_AUTO_ENRICH=0 start go-seed "$url" "$tmp/lain" serve --data-dir "$tmp/go-seed" --port "$go_port")
	LAIN_URL=$url "$runner" --var media_dir="$tmp/seedmedia" --save-vars "$tmp/seed-vars.json" conformance/cases/seed
	out=$(LAIN_URL=$url "$runner" --vars "$tmp/seed-vars.json" conformance/cases/seeded) || { echo "$out"; exit 1; }
	echo "$out"
	note "Go   seeded:   $(echo "$out" | tail -1)"
	stop "$pid"
}

run_rust() {
	url="http://127.0.0.1:$rs_port"
	pid=$(start rs-fresh "$url" "$lain_rs" serve --data-dir "$tmp/rs-fresh" --port "$rs_port")
	out=$(LAIN_URL=$url "$runner" --tier v1,v2 --var media_dir="$tmp/media" --var media_dir2="$tmp/media2" --var data_dir="$tmp/rs-fresh" conformance/cases/fresh conformance/cases/v2) || { echo "$out"; exit 1; }
	echo "$out"
	note "Rust fresh v1+v2: $(echo "$out" | tail -1)"
	stop "$pid"

	if [ -f "$tmp/go-seed/lain.db" ]; then
		cp -a "$tmp/go-seed" "$tmp/go-seed-copy"
		src="$tmp/go-seed-copy"
		vars="$tmp/seed-vars.json"
	else
		echo "no Go seed (rust only): skipping the seeded suite; cargo test covers it with the bbolt fixture"
		return 0
	fi
	before=$(cd "$src" && find . -type f -exec cksum {} + | sort)
	"$lain_rs" import --from "$src" --data-dir "$tmp/rs-seed" >"$tmp/import.json"
	after=$(cd "$src" && find . -type f -exec cksum {} + | sort)
	[ "$before" = "$after" ] || { echo "import modified the Go data dir" >&2; exit 1; }
	note "Import: the Go data dir is unchanged (cksum of every file); $(tr -d ' \n' <"$tmp/import.json" | sed -n 's/.*"counts":{\([^}]*\)}.*/\1/p')"
	pid=$(start rs-seed "$url" "$lain_rs" serve --data-dir "$tmp/rs-seed" --port "$rs_port")
	out=$(LAIN_URL=$url "$runner" --vars "$vars" conformance/cases/seeded) || { echo "$out"; exit 1; }
	echo "$out"
	note "Rust seeded (imported, Go token): $(echo "$out" | tail -1)"
	stop "$pid"
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
