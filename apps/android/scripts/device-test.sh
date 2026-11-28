#!/bin/sh
# The book readers on a device against a real server (the android-device
# CI job; it runs inside android-emulator-runner with the emulator up and
# both APKs built):
#   1. `lain serve` on port 9360 (all addresses: the emulator reaches the
#      host at 10.0.2.2) with a book library holding the conformance
#      fixtures' Tiny Book.epub and Tiny Pages.pdf, set up over the API;
#   2. the app and its test APK installed, then BookReaderDeviceTest run
#      with phase=online (log in, read, save progress, download);
#   3. the server stopped, and the test run again with phase=offline (the
#      downloaded book opens from the device).
# Screenshots, the instrumentation output, the server log and logcat land
# in OUT_DIR. Usage: scripts/device-test.sh LAIN_BINARY OUT_DIR
# (LAIN_DEVICE_PORT changes the port, 9360 by default.)
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
repo=$(cd "$here/../.." && pwd)
lain=$1
out=$2
port=${LAIN_DEVICE_PORT:-9360}
base="http://127.0.0.1:$port"
user=admin
password=password123
app_id=io.github.enrell.lain
runner="$app_id.test/androidx.test.runner.AndroidJUnitRunner"

mkdir -p "$out/books" "$out/screens"
cp "$repo/conformance/fixtures/read/Read/Tiny Book.epub" "$repo/conformance/fixtures/read/Read/Tiny Pages.pdf" "$out/books/"

LAIN_ACQUIRE_LISTEN_PORT=0 LAIN_AUTO_ENRICH=0 "$lain" serve --data-dir "$out/data" --port "$port" --bind 0.0.0.0 \
    >"$out/server.log" 2>&1 &
server=$!
stop_server() {
    kill "$server" 2>/dev/null || true
    wait "$server" 2>/dev/null || true
}
trap stop_server EXIT

up=0
for _ in $(seq 150); do
    if curl -fsS "$base/health" >/dev/null 2>&1; then
        up=1
        break
    fi
    sleep 0.2
done
if [ "$up" != 1 ]; then
    echo "the server did not start" >&2
    cat "$out/server.log" >&2
    exit 1
fi

# post PATH JSON [TOKEN]
post() {
    if [ -n "${3:-}" ]; then
        curl -fsS -X POST -H 'content-type: application/json' -H "authorization: Bearer $3" --data "$2" "$base$1"
    else
        curl -fsS -X POST -H 'content-type: application/json' --data "$2" "$base$1"
    fi
}

post /api/setup "{\"username\":\"$user\",\"password\":\"$password\"}" >/dev/null
token=$(post /api/auth/login "{\"username\":\"$user\",\"password\":\"$password\"}" | jq -r .token)
lib=$(post /api/libraries "$(jq -cn --arg path "$out/books" '{name: "Books", type: "book", path: $path}')" "$token" | jq -r .id)
curl -fsS -X POST -H "authorization: Bearer $token" "$base/api/library/scan" >/dev/null

# The two books, once the scan has them.
epub=""
pdf=""
for _ in $(seq 150); do
    books=$(curl -fsS -H "authorization: Bearer $token" "$base/api/catalog?library_id=$lib" |
        jq -c '(if type == "array" then . else (.items // []) end) | map(select(.kind == "book"))')
    epub=$(printf '%s' "$books" | jq -r 'map(select(.file_path | endswith(".epub"))) | .[0].id // ""')
    pdf=$(printf '%s' "$books" | jq -r 'map(select(.file_path | endswith(".pdf"))) | .[0].id // ""')
    if [ -n "$epub" ] && [ -n "$pdf" ]; then
        break
    fi
    sleep 0.2
done
if [ -z "$epub" ] || [ -z "$pdf" ]; then
    echo "the scan did not find both books: $books" >&2
    exit 1
fi
echo "server up, epub $epub, pdf $pdf"

adb install -r -t "$here/app/build/outputs/apk/debug/app-debug.apk"
adb install -r -t "$here/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk"

# instrument PHASE: one run of the device test; fails unless JUnit says OK.
instrument() {
    log="$out/instrument-$1.txt"
    adb shell am instrument -w -e phase "$1" -e server "http://10.0.2.2:$port" -e username "$user" \
        -e password "$password" -e epub "$epub" -e pdf "$pdf" "$runner" | tr -d '\r' | tee "$log"
    # `am instrument` exits 0 whatever the tests did: the summary says.
    grep -q '^OK (' "$log"
}

# The screenshots the test saved in the app's files.
pull_shots() {
    for name in $(adb shell run-as "$app_id" ls files/screens 2>/dev/null | tr -d '\r'); do
        adb exec-out run-as "$app_id" cat "files/screens/$name" >"$out/screens/$name" || true
    done
}

status=0
instrument online || status=1
pull_shots
if [ "$status" = 0 ]; then
    stop_server
    if curl -fsS "$base/health" >/dev/null 2>&1; then
        echo "the server is still up" >&2
        status=1
    else
        instrument offline || status=1
        pull_shots
    fi
fi
adb logcat -d >"$out/logcat.txt" 2>&1 || true
ls -l "$out/screens"
exit "$status"
