#!/usr/bin/env bash
# tools/mutation/run.sh — mutation testing runner (D-069).
#
# Usage:
#   tools/mutation/run.sh                 # all in-scope packages
#   tools/mutation/run.sh internal/store  # one package (repeatable)
#   tools/mutation/run.sh --force ...     # ignore the skip manifest
#
# A package is skipped when its .go source hash matches a "clean" entry
# in tools/mutation/state.json — the hash covers test files too, so a
# new unverified test in a clean package still triggers a run.
#
# Surviving mutants (LIVED, NOT_COVERED, TIMED_OUT) fail the run unless
# they match an `equivalents` entry in the manifest (file+line+type plus
# a one-line justification). NOT_VIABLE mutants never count.
#
# Requires: go-gremlins pinned at v0.6.0, installed outside go.mod:
#   go install github.com/go-gremlins/gremlins/cmd/gremlins@v0.6.0
#
# Reports land in tools/mutation/reports/ (gitignored). Workdirs go to a
# sibling of the repo — gremlins copies the whole module per worker, so
# the temp root must live outside the module (a workdir inside the tree
# would be recursively copied into itself) and off tmpfs (too small for
# N copies of node_modules+qa).

set -euo pipefail
cd "$(dirname "$0")/../.."
ROOT=$(pwd)
STATE=tools/mutation/state.json
REPORTS=tools/mutation/reports
WORKERS="${MUTATION_WORKERS:-4}"
COEFF="${MUTATION_TIMEOUT_COEFFICIENT:-4}"
export TMPDIR="${TMPDIR:-$(dirname "$ROOT")/.lain-mutation-tmp}"
mkdir -p "$TMPDIR" "$REPORTS"

GREMLINS="${GREMLINS:-$(command -v gremlins || echo "$(go env GOPATH)/bin/gremlins")}"
[ -x "$GREMLINS" ] || { echo "gremlins not found; go install github.com/go-gremlins/gremlins/cmd/gremlins@v0.6.0" >&2; exit 2; }

FORCE=0
PKGS=()
for a in "$@"; do
	case "$a" in
		--force) FORCE=1 ;;
		*) PKGS+=("$a") ;;
	esac
done
if [ ${#PKGS[@]} -eq 0 ]; then
	while IFS= read -r f; do
		PKGS+=("${f#./}")
	done < <(find internal cmd -name '*.go' ! -name '*_test.go' -printf '%h\n' | sort -u)
fi

hash_pkg() { # content hash of every .go file in the package dir
	(cd "$1" && find . -maxdepth 1 -name '*.go' -print0 | sort -z | xargs -0 sha256sum | sha256sum | cut -d' ' -f1)
}

mark_clean() { # stamp verified test files with the mutation-clean marker
	find "$1" -maxdepth 1 -name '*_test.go' | while read -r f; do
		grep -q '// mutation-clean' "$f" && continue
		awk -v m="// mutation-clean: gremlins v0.6.0 — package verified $(date +%F)" '
			!done && /^package / { print; print ""; print m; done=1; next } { print }
		' "$f" > "$f.tmp" && mv "$f.tmp" "$f"
	done
}

fail=0
for pkg in "${PKGS[@]}"; do
	[ -d "$pkg" ] || { echo "skip $pkg (no such dir)"; continue; }
	h=$(hash_pkg "$pkg")
	prev=$(jq -r --arg p "$pkg" '.packages[$p] // empty' "$STATE" 2>/dev/null || true)
	if [ "$FORCE" = 0 ] && [ -n "$prev" ]; then
		prev_hash=$(jq -r '.hash' <<<"$prev")
		prev_status=$(jq -r '.status' <<<"$prev")
		if [ "$prev_hash" = "$h" ] && [ "$prev_status" = "clean" ]; then
			echo "SKIP $pkg (unchanged, verified clean)"
			continue
		fi
	fi
	echo "MUTATE $pkg"
	out="$REPORTS/$(echo "$pkg" | tr / _).json"
	rm -f "$out" # gremlins only writes at the end; a stale report must never be parsed
	# Gremlins exits non-zero only on its own thresholds; we judge
	# survivors ourselves, so a gremlins failure still gets parsed.
	# GOFLAGS=-count=1 is load-bearing: the per-mutant timeout is the
	# coverage run's elapsed time times the coefficient, and a cached
	# coverage run would shrink every mutant budget below the real
	# suite duration (mass TIMED OUT false positives).
	GOFLAGS="-count=1 ${GOFLAGS:-}" "$GREMLINS" unleash "./$pkg" --workers "$WORKERS" --timeout-coefficient "$COEFF" -o "$out" --silent || true
	if [ ! -f "$out" ]; then
		echo "  $pkg: ERROR — gremlins produced no report (coverage or workdir failure)"
		fail=1
		continue
	fi
	survivors=$(jq -r '
		[.files[] as $f | $f.mutations[] | select(.status != "KILLED" and .status != "NOT_VIABLE" and .status != "NOT VIABLE")
		 | {file: $f.file_name, line, type, status}]
	' "$out")
	eq=$(jq -r --arg p "$pkg" '.packages[$p].equivalents // []' "$STATE")
	unresolved=$(jq --argjson eq "$eq" '
		[.[] | select([.file, .line, .type] | IN($eq[] | [.file, .line, .type]) | not)]
	' <<<"$survivors")
	n_unresolved=$(jq 'length' <<<"$unresolved")
	killed=$(jq -r '.mutants_killed' "$out")
	total=$(jq -r '.mutants_total' "$out")
	if [ "$n_unresolved" -eq 0 ]; then
		mark_clean "$pkg"
		h=$(hash_pkg "$pkg")
		status="clean"
	else
		status="survivors"
		fail=1
		echo "$unresolved" | jq -r '.[] | "  SURVIVED \(.status) \(.type) \(.file):\(.line)"'
	fi
	jq --arg p "$pkg" --arg h "$h" --arg s "$status" --argjson k "$killed" --argjson t "$total" \
		--argjson eq "$eq" --arg d "$(date +%F)" '
		.packages[$p] = {hash: $h, status: $s, killed: $k, mutants: $t, equivalents: $eq, verified_at: $d}
	' "$STATE" > "$STATE.tmp" && mv "$STATE.tmp" "$STATE"
	echo "  $pkg: $killed/$total killed — $status"
done
exit "$fail"
