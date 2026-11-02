#!/usr/bin/env bash
# Proves each check rejects bad input; see .claude/decisions/2026-11-02-development-adr-driven-lifecycle.md.
set -uo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

passed=0 failed=0
expect() { # want(pass|fail) name command...
  local want="$1" name="$2" got
  shift 2
  if "$@" >/dev/null 2>&1; then got=pass; else got=fail; fi
  if [[ "$got" == "$want" ]]; then
    passed=$((passed + 1))
  else
    echo "FAIL: $name: expected $want, got $got" >&2
    failed=$((failed + 1))
  fi
}

good_adr() { # date category title
  cat <<ADR
## ${3:-A sample decision}

* Date: ${1:-2026-01-02}
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: ${2:-development}

## Context

Why.

\`\`\`
## Not a heading, it is inside a fence
\`\`\`

## Implementation plan.

### Steps

How.
ADR
}

good_name="2026-01-02-development-a-sample-decision.md"
case_dir() { # filename content -> prints a fresh dir holding one ADR
  local d
  d="$(mktemp -d "$tmp/case.XXXXXX")"
  printf '%s\n' "$2" >"$d/$1"
  echo "$d"
}
decisions() { # want name filename content
  expect "$1" "decisions: $2" bash "$script_dir/check-decisions.sh" "$(case_dir "$3" "$4")"
}

good="$(good_adr)"
decisions pass "valid ADR" "$good_name" "$good"
decisions pass "5-word slug" "2026-01-02-development-one-two-three-four-five.md" "$good"
decisions fail "unknown category" "2026-01-02-design-a-sample-decision.md" "$good"
decisions fail "2-word slug" "2026-01-02-development-two-words.md" "$good"
decisions fail "6-word slug" "2026-01-02-development-one-two-three-four-five-six.md" "$good"
decisions fail "uppercase slug" "2026-01-02-development-A-sample-decision.md" "$good"
decisions fail "Date mismatch" "$good_name" "$(good_adr 2026-01-03)"
decisions fail "Category mismatch" "$good_name" "$(good_adr 2026-01-02 security)"
decisions fail "literal '## Title'" "$good_name" "$(good_adr 2026-01-02 development Title)"
decisions fail "altered Author" "$good_name" "${good/Willow Graysen (@noisypigeon)./Someone Else.}"
decisions fail "missing Author" "$good_name" "$(grep -v '^\* Author:' <<<"$good")"
decisions fail "empty Model" "$good_name" "${good/Model: Claude Opus 5.5/Model:}"
decisions fail "bullets out of order" "$good_name" "$(sed -e 's/^\* Date:/* TMP:/' -e 's/^\* Model:/* Date:/' -e 's/^\* TMP:/* Model:/' <<<"$good")"
decisions fail "missing Context" "$good_name" "$(grep -v '^## Context$' <<<"$good")"
decisions fail "missing Implementation plan" "$good_name" "$(grep -v '^## Implementation plan\.$' <<<"$good")"
decisions fail "sections out of order" "$good_name" "$(sed -e 's/^## Context$/## TMP/' -e 's/^## Implementation plan\.$/## Context/' -e 's/^## TMP$/## Implementation plan./' <<<"$good")"
decisions fail "extra H2" "$good_name" "$good"$'\n\n## Consequences\n\nMore.'
decisions fail "H1 present" "$good_name" "$good"$'\n\n# Stray\n'
decisions fail "frontmatter" "$good_name" $'---\ntitle: x\n---\n'"$good"

amend=$'\n\n## Amendments\n\n### 2026-01-02: First\n\n#### Detail\n\nText.\n\n### 2026-02-01: Second\n'
decisions pass "valid amendments" "$good_name" "$good$amend"
decisions fail "amendments out of order" "$good_name" "$good"$'\n\n## Amendments\n\n### 2026-02-01: Later\n\n### 2026-01-05: Earlier\n'
decisions fail "amendment before ADR date" "$good_name" "$good"$'\n\n## Amendments\n\n### 2026-01-01: Too early\n'
decisions fail "malformed amendment heading" "$good_name" "$good"$'\n\n## Amendments\n\n### Second thoughts\n'
decisions fail "amendment without summary" "$good_name" "$good"$'\n\n## Amendments\n\n### 2026-01-03:\n'
decisions fail "empty amendments" "$good_name" "$good"$'\n\n## Amendments\n\nNothing.\n'
decisions fail "amendments before the plan" "$good_name" "$(sed 's/^## Implementation plan\.$/## Amendments\n\n### 2026-01-03: Early\n\n## Implementation plan./' <<<"$good")"
decisions fail "H2 after amendments" "$good_name" "$good$amend"$'\n## Consequences\n\nMore.\n'

mkdir -p "$tmp/empty"
expect fail "decisions: empty directory" bash "$script_dir/check-decisions.sh" "$tmp/empty"
mkdir -p "$tmp/readme-only" && echo "# Decisions" >"$tmp/readme-only/README.md"
expect fail "decisions: README only" bash "$script_dir/check-decisions.sh" "$tmp/readme-only"
d="$(case_dir "$good_name" "$good")" && echo "# Decisions" >"$d/README.md"
expect pass "decisions: README is ignored" bash "$script_dir/check-decisions.sh" "$d"
d="$(case_dir "$good_name" "$good")" && echo "x" >"$d/notes.txt"
expect fail "decisions: stray non-ADR file" bash "$script_dir/check-decisions.sh" "$d"

# check-pr.sh, against a fake repo root holding one ADR.
root="$tmp/repo"
mkdir -p "$root/.claude/decisions"
printf '%s\n' "$good" >"$root/.claude/decisions/$good_name"
cite=".claude/decisions/$good_name"
url=https://github.com/noisypigeon/pigeon
printf '# Changelog\n\n## 2026-01-02\n\n- ci: x ([#7](%s/pull/7))\n' "$url" >"$root/CHANGELOG.md"
pr() { # want name title body [number]
  expect "$1" "pr: $2" env PR_TITLE="$3" PR_BODY="$4" PR_NUMBER="${5-7}" bash "$script_dir/check-pr.sh" "$root"
}
pr pass "valid" "ci: enforce the lifecycle" "## Decision"$'\n\n'"$cite"
pr pass "scope and bang" "feat(cli)!: break things" "$cite"
pr fail "non-conventional title" "Enforce the lifecycle" "$cite"
pr fail "unknown type" "feature: x" "$cite"
pr fail "missing summary" "ci: " "$cite"
pr fail "no ADR cited" "ci: x" "No decision here."
pr fail "README is not an ADR" "ci: x" ".claude/decisions/README.md"
pr fail "cites a missing ADR" "ci: x" ".claude/decisions/2026-01-02-development-does-not-exist.md"
pr fail "one of two citations missing" "ci: x" "$cite and .claude/decisions/2026-01-02-other-not-here-at-all.md"
pr fail "PR not in changelog" "ci: x" "$cite" 8
pr fail "missing PR_NUMBER" "ci: x" "$cite" ""
pr fail "non-numeric PR_NUMBER" "ci: x" "$cite" "7; true"
pr fail "injection stays data" 'ci: x"; exit 0; "' "\$(exit 0) \`true\`"

changelog() { # want name content
  local f="$tmp/changelog.$RANDOM$RANDOM.md"
  printf '%s\n' "$3" >"$f"
  expect "$1" "changelog: $2" bash "$script_dir/check-changelog.sh" "$f"
}
sha=0123456789abcdef0123456789abcdef01234567
cl_good="# Changelog

## 2026-01-03

- ci: second ([#2]($url/pull/2))
- feat: first ([#1]($url/pull/1))

## 2026-01-02

- docs(adr): bootstrap ([0123456]($url/commit/$sha))"
changelog pass "valid" "$cl_good"
expect fail "changelog: no such file" bash "$script_dir/check-changelog.sh" "$tmp/nope.md"
changelog fail "wrong title" "${cl_good/\# Changelog/# Changes}"
changelog fail "dates ascending" "${cl_good/2026-01-02/2026-01-04}"
changelog fail "duplicate date" "${cl_good/2026-01-02/2026-01-03}"
changelog fail "malformed date heading" "${cl_good/\#\# 2026-01-02/## Jan 2}"
changelog fail "empty section" "$cl_good"$'\n\n## 2026-01-01\n'
changelog fail "no sections" "# Changelog"
changelog fail "bullet without link" "${cl_good/" ([#1]($url/pull/1))"/}"
changelog fail "link to another repo" "${cl_good/"$url/pull/1"/https://github.com/other/repo/pull/1}"
changelog fail "number mismatch" "${cl_good/"[#1]"/[#3]}"
changelog fail "duplicate PR" "${cl_good/"[#2]($url/pull/2)"/[#1]($url/pull/1)}"
changelog fail "short commit URL" "${cl_good/"commit/$sha"/commit/0123456}"
changelog fail "sha text not a prefix" "${cl_good/"[0123456]"/[fedcba9]}"
changelog fail "multi-line bullet" "$cl_good"$'\n  continued'
changelog fail "text before sections" "# Changelog

Intro.
${cl_good#\# Changelog}"

echo "test-checks: $passed passed, $failed failed"
((failed == 0))
