#!/bin/sh
# Runs `pigeon session <command>` from main's build, else session.py; see
# .claude/decisions/2026-11-13-development-rust-session-event-follower.md §4.
dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
if [ "${1:-}" = brief ] && command -v timeout >/dev/null 2>&1; then
  timeout 10 python3 -I "$dir/session.py" install >/dev/null 2>&1
fi
for bin in "$HOME/.cache/pigeon/bin/pigeon" "$(command -v pigeon 2>/dev/null)"; do
  if [ -n "$bin" ] && [ -x "$bin" ] && "$bin" session --help >/dev/null 2>&1; then
    exec "$bin" session "$@"
  fi
done
exec python3 -I "$dir/session.py" "$@"
