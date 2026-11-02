#!/usr/bin/env bash
# Lints every ADR in .claude/decisions/ against
# .claude/decisions/2026-11-02-development-adr-driven-lifecycle.md §1 and §2.
#
# Usage: scripts/check-decisions.sh [decisions-dir]
set -uo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
dir="${1:-$script_dir/../.claude/decisions}"

name_re='^([0-9]{4}-[0-9]{2}-[0-9]{2})-(development|release|runtime|security|other)-[a-z0-9]+(-[a-z0-9]+){2,4}\.md$'
author='Willow Graysen (@noisypigeon).'

failures=0
fail() { # file line reason
  printf '%s:%s: %s\n' "$1" "$2" "$3" >&2
  failures=$((failures + 1))
}

if [[ ! -d "$dir" ]]; then
  echo "check-decisions: no such directory: $dir" >&2
  exit 1
fi

check_file() {
  local f="$1" b date category
  b="$(basename "$f")"
  if [[ ! "$b" =~ $name_re ]]; then
    fail "$f" 1 "filename must be YYYY-MM-DD-<development|release|runtime|security|other>-<3-5 lowercase words>.md"
    return
  fi
  date="${BASH_REMATCH[1]}"
  category="${BASH_REMATCH[2]}"

  local -a lines=()
  mapfile -t lines <"$f"
  if ((${#lines[@]} == 0)); then
    fail "$f" 1 "file is empty"
    return
  fi

  # Line 1: the title, as an H2.
  local first="${lines[0]%$'\r'}"
  if [[ ! "$first" =~ ^##\ (.*[^[:space:]].*)$ ]]; then
    fail "$f" 1 "first line must be the title as an H2 (## <Title>)"
  elif [[ "${BASH_REMATCH[1]}" == "Title" ]]; then
    fail "$f" 1 "replace the template's literal '## Title' with the ADR's title"
  fi

  # The four metadata bullets: the first four non-blank lines after the title.
  local -a keys=(Date Model Author Category)
  local i=1 k line value
  for k in "${keys[@]}"; do
    while ((i < ${#lines[@]})) && [[ -z "${lines[i]//[[:space:]]/}" ]]; do i=$((i + 1)); done
    line="${lines[i]:-}"
    line="${line%$'\r'}"
    if [[ ! "$line" =~ ^\*\ $k:\ *(.*)$ ]]; then
      fail "$f" $((i + 1)) "expected '* $k: <value>' (bullets in order: ${keys[*]})"
      return
    fi
    value="${BASH_REMATCH[1]}"
    value="${value%"${value##*[![:space:]]}"}"
    case "$k" in
    Date) [[ "$value" == "$date" ]] || fail "$f" $((i + 1)) "Date '$value' does not match the filename's date '$date'" ;;
    Model) [[ -n "$value" ]] || fail "$f" $((i + 1)) "Model must name the model that wrote the ADR" ;;
    Author) [[ "$value" == "$author" ]] || fail "$f" $((i + 1)) "Author must be exactly '$author'" ;;
    Category) [[ "$value" == "$category" ]] || fail "$f" $((i + 1)) "Category '$value' does not match the filename's category '$category'" ;;
    esac
    i=$((i + 1))
  done

  # Headings outside fences: no H1; H2s are Context, Implementation plan., then optionally Amendments.
  local -a want=("## Context" "## Implementation plan." "## Amendments")
  local -a got=() got_lines=()
  local in_fence=0 n section="" last="$date" entries=0 d
  for ((n = 1; n < ${#lines[@]}; n++)); do
    line="${lines[n]%$'\r'}"
    if [[ "$line" =~ ^[[:space:]]*(\`\`\`|~~~) ]]; then
      in_fence=$((1 - in_fence))
      continue
    fi
    ((in_fence)) && continue
    if [[ "$line" =~ ^#\  ]]; then
      fail "$f" $((n + 1)) "no H1 headings; the title is the first line's H2"
    elif [[ "$line" =~ ^##\  ]]; then
      section="${line%"${line##*[![:space:]]}"}"
      got+=("$section")
      got_lines+=($((n + 1)))
    elif [[ "$section" == "## Amendments" && "$line" =~ ^###\  ]]; then
      entries=$((entries + 1))
      if [[ ! "$line" =~ ^###\ ([0-9]{4}-[0-9]{2}-[0-9]{2}):\ .*[^[:space:]] ]]; then
        fail "$f" $((n + 1)) "an amendment heading is '### YYYY-MM-DD: <summary>'"
        continue
      fi
      d="${BASH_REMATCH[1]}"
      if [[ "$d" < "$date" ]]; then
        fail "$f" $((n + 1)) "amendment dated $d is earlier than the ADR's Date $date"
      elif [[ "$d" < "$last" ]]; then
        fail "$f" $((n + 1)) "amendment dated $d is out of date order (after $last)"
      fi
      last="$d"
    fi
  done
  local j
  for ((j = 0; j < ${#got[@]}; j++)); do
    if ((j >= ${#want[@]})) || [[ "${got[j]}" != "${want[j]}" ]]; then
      fail "$f" "${got_lines[j]}" "unexpected H2 '${got[j]}'; after the title the H2s are '${want[0]}', '${want[1]}', then optionally '${want[2]}' (use ### inside them)"
      return
    fi
  done
  if ((${#got[@]} < 2)); then
    fail "$f" "${#lines[@]}" "missing '${want[${#got[@]}]}'"
  elif ((${#got[@]} == 3 && entries == 0)); then
    fail "$f" "${got_lines[2]}" "'## Amendments' holds no '### YYYY-MM-DD: <summary>' entry"
  fi
}

count=0
while IFS= read -r -d '' f; do
  [[ "$(basename "$f")" == "README.md" ]] && continue
  count=$((count + 1))
  check_file "$f"
done < <(find "$dir" -mindepth 1 -maxdepth 1 -type f -print0 | sort -z)

if ((count == 0)); then
  fail "$dir" 0 "no ADRs found"
fi

if ((failures > 0)); then
  echo "check-decisions: $failures problem(s) in $dir" >&2
  exit 1
fi
echo "check-decisions: $count ADR(s) OK"
