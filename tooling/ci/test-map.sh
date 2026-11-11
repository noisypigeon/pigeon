#!/usr/bin/env bash
# The test map: each test binary and the workspace files it runs, as tests.json; see
# .claude/decisions/2026-11-10-development-factory-crates-functional-core.md, 2026-11-11 amendment.
# Run from src/pigeon-cli with cargo-llvm-cov and llvm-tools installed; TEST_MAP_OUT names the output.
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

: >"$work/entries.jsonl"
while IFS=$'\t' read -r package kind name; do
  case $kind in
    lib) select=(--lib) ;;
    *) select=("--$kind" "$name") ;;
  esac
  argv=(cargo test -p "$package" "${select[@]}" --locked)
  echo "test map: ${argv[*]}" >&2
  cargo llvm-cov clean --profraw-only
  "${argv[@]}" -- --quiet
  cargo llvm-cov report --json --summary-only --output-path "$work/report.json"
  jq -c --arg root "$root/" --arg package "$package" --arg kind "$kind" --arg name "$name" \
    --arg argv "${argv[*]}" '{
      package: $package, kind: $kind, name: $name, argv: ($argv | split(" ")),
      files: [.data[].files[] | select(.summary.lines.covered > 0)
        | .filename | select(startswith($root)) | ltrimstr($root)] | unique
    }' "$work/report.json" >>"$work/entries.jsonl"
done <"$work/binaries.tsv"

jq -s --arg sha "$sha" '{version: 1, sha: $sha, binaries: .}' "$work/entries.jsonl" >"$out"
jq -r '"test map: \(.binaries | length) binaries, \([.binaries[].files[]] | unique | length) files"' "$out" >&2
