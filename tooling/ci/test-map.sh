#!/usr/bin/env bash
# The test map: each test binary and the workspace files it runs, as tests.json; see
# .claude/decisions/2026-11-10-development-factory-crates-functional-core.md, 2026-11-11 amendment,
# and .claude/decisions/2026-11-16-development-one-test-binary-per-crate.md §3: a `suite` binary
# maps per top-level module.
# Run from the workspace with cargo-llvm-cov and llvm-tools installed; TEST_MAP_OUT names the output.
set -euo pipefail

out=${TEST_MAP_OUT:-tests.json}
root=$(git rev-parse --show-toplevel)
sha=$(git rev-parse HEAD)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# Each workspace package's lib, bin and test targets that have tests, as package, kind, name.
cargo metadata --no-deps --format-version 1 --locked |
  jq -r '.packages[] | .name as $p | .targets[]
    | select(.test)
    | (if (.kind | index("test")) then "test"
       elif (.kind | index("bin")) then "bin"
       elif (.kind | any(. == "lib" or . == "rlib" or . == "cdylib" or . == "proc-macro")) then "lib"
       else empty end) as $k
    | [$p, $k, .name] | @tsv' >"$work/binaries.tsv"

source <(cargo llvm-cov show-env --sh)
cargo llvm-cov clean --workspace
cargo test --workspace --locked --no-run

# One unit: runs argv with `filter`, if any, and records the files it covered.
unit() {
  local package=$1 kind=$2 name=$3 filter=$4
  echo "test map: ${argv[*]} -- $filter" >&2
  cargo llvm-cov clean --profraw-only
  if [ -n "$filter" ]; then
    "${argv[@]}" -- --quiet "$filter"
  else
    "${argv[@]}" -- --quiet
  fi
  cargo llvm-cov report --json --summary-only --output-path "$work/report.json"
  jq -c --arg root "$root/" --arg package "$package" --arg kind "$kind" --arg name "$name" \
    --arg filter "$filter" --arg argv "${argv[*]}" '{
      package: $package, kind: $kind, name: $name, filter: $filter, argv: ($argv | split(" ")),
      files: [.data[].files[] | select(.summary.lines.covered > 0)
        | .filename | select(startswith($root)) | ltrimstr($root)] | unique
    }' "$work/report.json" >>"$work/entries.jsonl"
}

: >"$work/entries.jsonl"
while IFS=$'\t' read -r package kind name; do
  case $kind in
    lib) select=(--lib) ;;
    *) select=("--$kind" "$name") ;;
  esac
  argv=(cargo test -p "$package" "${select[@]}" --locked)
  if [ "$kind/$name" = test/suite ]; then
    "${argv[@]}" -- --list --format terse </dev/null >"$work/list"
    if grep -q '^[^:]*: test$' "$work/list"; then
      echo "test map: $package's suite has a test outside any module" >&2
      exit 1
    fi
    sed -n 's/^\([^:]*\)::.*: test$/\1/p' "$work/list" | sort -u >"$work/modules"
    while read -r module; do
      unit "$package" "$kind" "$name" "$module::" </dev/null
    done <"$work/modules"
  else
    unit "$package" "$kind" "$name" ""
  fi
done <"$work/binaries.tsv"

jq -s --arg sha "$sha" '{version: 2, sha: $sha, binaries: .}' "$work/entries.jsonl" >"$out"
jq -r '"test map: \(.binaries | length) units, \([.binaries[].files[]] | unique | length) files"' "$out" >&2
