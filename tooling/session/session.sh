#!/bin/sh
# Runs `pigeon session <command>` from main's build, fetched by `install`; see
# .claude/decisions/2026-11-15-development-no-python-rust-tooling.md §2.
dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(dirname -- "$(dirname -- "$dir")")
host=${PIGEON_HOST:-platform.pigeon.dev}
cache=$HOME/.cache/pigeon/bin

# The first `pigeon` whose `session` works.
pick() {
  for bin in "$cache/pigeon" "$(command -v pigeon 2>/dev/null)"; do
    if [ -n "$bin" ] && [ -x "$bin" ] && "$bin" session --help >/dev/null 2>&1; then
      printf '%s\n' "$bin"
      return 0
    fi
  done
  return 1
}

# `deploy.status`'s answer on stdin: "<sha> <sha256>" of latest_build, hex only.
latest() {
  build=$(sed -n 's/.*"latest_build":{\([^}]*\)}.*/\1/p')
  sha=$(printf '%s\n' "$build" | sed -n 's/.*"sha":"\([0-9a-f]\{40\}\)".*/\1/p')
  sum=$(printf '%s\n' "$build" | sed -n 's/.*"sha256":"\([0-9a-f]\{64\}\)".*/\1/p')
  [ -n "$sha" ] && [ -n "$sum" ] && printf '%s %s\n' "$sha" "$sum"
}

digest() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1"
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1"
  else
    openssl dgst -sha256 -r "$1"
  fi 2>/dev/null | cut -d' ' -f1
}

# The bootstrap: main's newest build into $cache, SHA-256 checked, with no `pigeon` yet.
bootstrap() {
  got=$(curl -sS --max-time 8 -X POST -H 'content-type: application/json' \
    --data '{"operation":"deploy.status","args":{}}' "https://$host/v0/run" 2>/dev/null | latest) ||
    return 1
  sha=${got% *}
  sum=${got#* }
  if [ -x "$cache/pigeon" ] && [ "$(cat "$cache/pigeon.sha" 2>/dev/null)" = "$sha" ]; then
    return 0
  fi
  mkdir -p "$cache" || return 1
  tmp="$cache/pigeon.tmp.$$"
  if curl -sSf --max-time 60 -o "$tmp" "https://$host/builds/$sha/pigeon" &&
    [ "$(digest "$tmp")" = "$sum" ]; then
    chmod 755 "$tmp" && mv -f "$tmp" "$cache/pigeon" && printf '%s' "$sha" >"$cache/pigeon.sha"
  else
    rm -f "$tmp"
    return 1
  fi
}

case ${1:-} in
  latest)
    latest
    exit
    ;;
  install)
    if bin=$(pick) && "$bin" session install; then exit 0; fi
    bootstrap
    exit
    ;;
  brief)
    if command -v timeout >/dev/null 2>&1; then
      timeout 10 sh "$dir/session.sh" install >/dev/null 2>&1
    fi
    ;;
esac
if bin=$(pick); then
  exec "$bin" session "$@"
fi
case ${1:-} in
  brief)
    echo "pigeon: no pigeon binary yet (main's build couldn't be fetched); \`sh tooling/session/session.sh install\` fetches it. CLAUDE.md's start steps still apply."
    exit 0
    ;;
  statusline | stop)
    [ -t 0 ] || cat >/dev/null
    [ "$1" = stop ] || echo "pigeon: no pigeon binary yet"
    exit 0
    ;;
esac
if command -v cargo >/dev/null 2>&1 && [ -f "$root/Cargo.toml" ]; then
  exec cargo run -q --release --locked --manifest-path "$root/Cargo.toml" -p pigeon -- session "$@"
fi
echo "pigeon: no pigeon binary, and no cargo to build one; see CLAUDE.md" >&2
exit 1
