#!/bin/sh
# The behaviour ledger: every test `cargo test --workspace` runs, one line each; see
# .claude/decisions/2026-11-16-development-one-test-binary-per-crate.md §4.
# Run from the workspace. `behaviours.sh` writes tooling/ci/tests.ledger; `behaviours.sh --check [BASE]`
# warns when the ledger is stale, and fails when a test in BASE's ledger is gone without a line
# in tests.moves.
set -eu

root=$(git rev-parse --show-toplevel)
ledger="$root/tooling/ci/tests.ledger"
moves="$root/tooling/ci/tests.moves"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# Each test, as `<package> <kind> <target> <test>`, sorted.
list() {
  cargo metadata --no-deps --format-version 1 --locked >"$work/metadata.json"
  jq -r '.packages[] | [.manifest_path, .name] | @tsv' "$work/metadata.json" >"$work/packages.tsv"
  cargo test --workspace --locked --no-run --message-format=json >"$work/build.json"
  jq -r 'select(.reason == "compiler-artifact" and .profile.test and .executable != null)
      | [.manifest_path,
         (if (.target.kind | index("test")) then "test"
          elif (.target.kind | index("bin")) then "bin" else "lib" end),
         .target.name, .executable] | @tsv' "$work/build.json" >"$work/targets.tsv"
  while IFS="$(printf '\t')" read -r manifest kind name exe; do
    package=$(awk -F '\t' -v m="$manifest" '$1 == m { print $2 }' "$work/packages.tsv")
    "$exe" --list --format terse >"$work/one"
    sed -n "s/^\(.*\): test\$/$package $kind $name \1/p" "$work/one"
  done <"$work/targets.tsv" >"$work/all"
  LC_ALL=C sort "$work/all"
}

# Fails when a top-level module's filter `<module>::` occurs in a test path outside that module:
# libtest's filter is a substring, so it would select that test too.
filters() {
  awk '$3 == "suite" { split($4, p, "::"); if (p[2] != "") print $1, p[1] }' "$1" |
    LC_ALL=C sort -u >"$work/modules"
  awk 'NR == FNR { m[NR] = $0; n = NR; next }
    $3 == "suite" { for (i = 1; i <= n; i++) {
        split(m[i], a, " ")
        f = a[2] "::"
        if (a[1] == $1 && index($4, f) != 1 && index($4, f) > 0) {
          print "behaviours: in " $1 "'"'"'s suite, " f " also selects " $4; bad = 1 } } }
    END { exit bad }' "$work/modules" "$1"
}

case "${1:-}" in
--check)
  base=${2:-${CHANGE_BASE:-origin/main}}
  list >"$work/now"
  if ! cmp -s "$work/now" "$ledger"; then
    echo "behaviours: warning: $ledger is stale; run tooling/ci/behaviours.sh and commit it" >&2
    diff "$ledger" "$work/now" | head -n 20 >&2 || true
  fi
  filters "$work/now"
  # The ledger lived in src/pigeon-cli until the workspace moved to the root.
  if ! git show "$base:tooling/ci/tests.ledger" >"$work/base" 2>/dev/null \
    && ! git show "$base:src/pigeon-cli/tests.ledger" >"$work/base" 2>/dev/null; then
    echo "behaviours: $base has no ledger; $(wc -l <"$work/now") tests listed"
    exit 0
  fi
  touch "$moves"
  # A base test is kept when it is listed now, or tests.moves maps it to one listed now.
  awk -F '\t' 'FILENAME == ARGV[1] { now[$0] = 1; next }
    FILENAME == ARGV[2] { to[$1] = $2; next }
    !($0 in now) && !(($0 in to) && (to[$0] in now)) { print "behaviours: gone: " $0; bad = 1 }
    END { exit bad }' "$work/now" "$moves" "$work/base"
  # No package runs fewer tests than it did, each base test counted where tests.moves moved it.
  awk -F '\t' 'FILENAME == ARGV[1] { to[$1] = $2; next }
    { line = ($0 in to) ? to[$0] : $0; split(line, f, " "); n[f[1]]++ }
    END { for (p in n) print p, n[p] }' "$moves" "$work/base" | LC_ALL=C sort >"$work/base.count"
  awk '{ n[$1]++ } END { for (p in n) print p, n[p] }' "$work/now" | LC_ALL=C sort >"$work/now.count"
  awk 'NR == FNR { now[$1] = $2; next }
    (now[$1] + 0) < $2 { print "behaviours: " $1 " runs " (now[$1] + 0) " tests, " $2 " before"; bad = 1 }
    END { exit bad }' "$work/now.count" "$work/base.count"
  echo "behaviours: $(wc -l <"$work/now") tests, none gone against $base"
  ;;
"")
  list >"$ledger"
  touch "$moves"
  echo "behaviours: $(wc -l <"$ledger") tests in $ledger"
  ;;
*)
  echo "usage: behaviours.sh [--check [BASE]]" >&2
  exit 2
  ;;
esac
