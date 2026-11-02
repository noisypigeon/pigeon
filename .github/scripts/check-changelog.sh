#!/usr/bin/env bash
# Lints CHANGELOG.md; rules in .claude/decisions/2026-11-02-development-adr-driven-lifecycle.md. Usage: check-changelog.sh [file]
set -uo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
file="${1:-$script_dir/../../CHANGELOG.md}"
repo='https://github.com/noisypigeon/pigeon'

failures=0
fail() {
  printf '%s:%s: %s\n' "$file" "$1" "$2" >&2
  failures=$((failures + 1))
}

if [[ ! -f "$file" ]]; then
  echo "check-changelog: no such file: $file" >&2
  exit 1
fi

mapfile -t lines <"$file"
[[ "${lines[0]:-}" == "# Changelog" ]] || fail 1 "first line must be '# Changelog'"

bullet_re='^- (.*[^[:space:]]) \((\[[^]]+\]\([^)]+\))\)$'
pr_re="^\[#([0-9]+)\]\(${repo//./\\.}/pull/([0-9]+)\)$"
commit_re="^\[([0-9a-f]{7,40})\]\(${repo//./\\.}/commit/([0-9a-f]{40})\)$"

declare -A seen=()
prev="" bullets=0 heading_line=0 sections=0
for ((n = 1; n < ${#lines[@]}; n++)); do
  line="${lines[n]%$'\r'}"
  [[ -z "${line//[[:space:]]/}" ]] && continue
  if [[ "$line" =~ ^##\  ]]; then
    if ((sections > 0 && bullets == 0)); then fail "$heading_line" "'$prev' has no bullets"; fi
    if [[ ! "$line" =~ ^##\ ([0-9]{4}-[0-9]{2}-[0-9]{2})$ ]]; then
      fail $((n + 1)) "a section heading is '## YYYY-MM-DD'"
      prev="" bullets=1 sections=$((sections + 1))
      continue
    fi
    d="${BASH_REMATCH[1]}"
    if [[ -n "$prev" && ! "$d" < "$prev" ]]; then
      fail $((n + 1)) "'$d' must be older than the section above it ('$prev')"
    fi
    prev="$d" bullets=0 heading_line=$((n + 1)) sections=$((sections + 1))
  elif ((sections == 0)); then
    fail $((n + 1)) "content before the first '## YYYY-MM-DD' section"
  elif [[ "$line" =~ $bullet_re ]]; then
    bullets=$((bullets + 1))
    link="${BASH_REMATCH[2]}"
    if [[ "$link" =~ $pr_re ]]; then
      if [[ "${BASH_REMATCH[1]}" != "${BASH_REMATCH[2]}" ]]; then
        fail $((n + 1)) "link text #${BASH_REMATCH[1]} does not match its URL's PR ${BASH_REMATCH[2]}"
      elif [[ -n "${seen[${BASH_REMATCH[1]}]:-}" ]]; then
        fail $((n + 1)) "PR #${BASH_REMATCH[1]} is already listed on line ${seen[${BASH_REMATCH[1]}]}"
      else
        seen[${BASH_REMATCH[1]}]=$((n + 1))
      fi
    elif [[ "$link" =~ $commit_re ]]; then
      [[ "${BASH_REMATCH[2]}" == "${BASH_REMATCH[1]}"* ]] || fail $((n + 1)) "link text ${BASH_REMATCH[1]} is not a prefix of its commit"
    else
      fail $((n + 1)) "link must be ([#N]($repo/pull/N)) or ([sha]($repo/commit/<full-sha>))"
    fi
  else
    fail $((n + 1)) "expected a one-line bullet: '- <change> ([#N]($repo/pull/N))'"
  fi
done
if ((sections == 0)); then
  fail 1 "no '## YYYY-MM-DD' sections"
elif ((bullets == 0)); then
  fail "$heading_line" "'$prev' has no bullets"
fi

if ((failures > 0)); then
  echo "check-changelog: $failures problem(s)" >&2
  exit 1
fi
echo "check-changelog: OK"
