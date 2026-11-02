#!/usr/bin/env bash
# Checks a PR's title, body and changelog line; rules in .claude/decisions/2026-11-02-development-adr-driven-lifecycle.md. Usage: PR_TITLE= PR_BODY= PR_NUMBER= check-pr.sh [root]
set -uo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="${1:-$script_dir/../..}"
title="${PR_TITLE-}"
body="${PR_BODY-}"
number="${PR_NUMBER-}"

title_re='^(feat|fix|docs|chore|ci|refactor|test|build|perf|revert)(\([a-z0-9][a-z0-9-]*\))?!?: .+'

failures=0
fail() {
  printf 'check-pr: %s\n' "$1" >&2
  failures=$((failures + 1))
}

if [[ ! "$title" =~ $title_re ]]; then
  fail "title '$title' is not a Conventional Commit: type(scope)?!?: summary, type one of feat|fix|docs|chore|ci|refactor|test|build|perf|revert"
fi

mapfile -t cited < <(printf '%s\n' "$body" | grep -oE '\.claude/decisions/[A-Za-z0-9._-]+\.md' | grep -v '/README\.md$' | sort -u)
if ((${#cited[@]} == 0)); then
  fail "body cites no ADR; name it as .claude/decisions/<file>.md under '## Decision'"
fi
for p in "${cited[@]}"; do
  [[ -f "$root/$p" ]] || fail "body cites $p, which does not exist in this checkout"
done

if [[ ! "$number" =~ ^[1-9][0-9]*$ ]]; then
  fail "PR_NUMBER '$number' is not a PR number"
elif ! grep -qF "([#$number](https://github.com/noisypigeon/pigeon/pull/$number))" "$root/CHANGELOG.md" 2>/dev/null; then
  fail "CHANGELOG.md has no bullet linking this PR: ([#$number](https://github.com/noisypigeon/pigeon/pull/$number))"
fi

if ((failures > 0)); then
  exit 1
fi
echo "check-pr: title and changelog OK; cites ${cited[*]}"
