#!/bin/sh
# Runs the browser smoke test (web/e2e/smoke.mjs) end to end: the
# synthetic media, a lain server on a fresh data dir, a headless Chrome,
# and the server stopped afterwards. CI runs exactly this.
#
#   web/e2e/run-smoke.sh [OUT_DIR]
#
# OUT_DIR (default: a new temp dir) receives server.log, the fixtures,
# the data dir and the screenshots the test writes (failure.png, and
# failure-mobile.png / failure-player.png of the player settings).
#
# Env:
#   LAIN        the server binary (default: target/release/lain, which
#               must embed the web build: LAIN_WEB_DIR or crates/lain/webui)
#   PORT        the server's port (default 19480)
#   DEBUG_PORT  Chrome's DevTools port (default 19481)
#   CHROME      the browser (default: the first Chromium or Chrome on PATH)
#   CHROME_FLAGS extra browser flags, space separated (CI: --no-sandbox)
#   FIXTURES    an existing fixture dir to reuse instead of making one
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
out="${1:-$(mktemp -d "${TMPDIR:-/tmp}/lain-smoke.XXXXXX")}"
mkdir -p "$out"
out=$(cd "$out" && pwd)
lain="${LAIN:-$root/target/release/lain}"
port="${PORT:-19480}"
debug_port="${DEBUG_PORT:-19481}"

if [ -n "${FIXTURES:-}" ]; then
	fixtures="$FIXTURES"
else
	fixtures="$out/fixtures"
	bash "$root/web/e2e/fixtures.sh" "$fixtures"
fi

# Every run starts from first-run setup.
rm -rf "$out/data" "$out/chrome" "$out/downloads"

# Nothing reaches the network. The metadata providers point at a closed
# port, so the enrichment step meets the local NFO provider (an online
# answer would win over it); no list sync, a free torrent port, and the
# theme does not read the host's own Omarchy colors.
for p in KITSU ANILIST JIKAN TVMAZE; do
	export "LAIN_METADATA_${p}_URL=http://127.0.0.1:9"
done
export LAIN_LISTLINK_ANILIST_GRAPHQL_URL=http://127.0.0.1:9
export LAIN_LISTLINK_ANILIST_OAUTH_URL=http://127.0.0.1:9
export LAIN_LIST_SYNC=0
export LAIN_ACQUIRE_LISTEN_PORT=0
export LAIN_OMARCHY_COLORS="$out/no-colors.toml"

"$lain" serve --data-dir "$out/data" --port "$port" --bind 127.0.0.1 >"$out/server.log" 2>&1 &
server=$!
# Stopped, and waited for, so its log is complete when this exits.
trap 'kill "$server" 2>/dev/null; wait "$server" 2>/dev/null || true' EXIT
trap 'exit 130' INT TERM

# Up means this server has bound the port (its banner, printed once the
# listener is bound, names this run's data dir) and answers there. A
# health answer alone can come from something else already on the port,
# an earlier run's server say, and the test would meet a data dir that is
# not fresh; this server, unable to bind, exits and ends the wait.
i=0
until grep -qF "on http://127.0.0.1:$port (data $out/data)" "$out/server.log" 2>/dev/null &&
	curl -fsS "http://127.0.0.1:$port/health" >/dev/null 2>&1; do
	i=$((i + 1))
	if ! kill -0 "$server" 2>/dev/null || [ "$i" -gt 150 ]; then
		echo "the server did not start (is port $port in use?)" >&2
		cat "$out/server.log" >&2
		exit 1
	fi
	sleep 0.1
done

start=$(date +%s)
status=0
node "$root/web/e2e/smoke.mjs" \
	--base "http://127.0.0.1:$port" \
	--media "$fixtures" \
	--debug-port "$debug_port" \
	--profile "$out/chrome" \
	--downloads "$out/downloads" \
	--screenshot "$out/failure.png" || status=$?
echo "smoke: exit $status after $(($(date +%s) - start))s (logs and screenshots in $out)"
exit "$status"
