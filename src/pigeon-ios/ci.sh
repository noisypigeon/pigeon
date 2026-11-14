#!/usr/bin/env bash
# The one macOS entrypoint every provider runs: `ci.sh check <sha>` or `ci.sh archive <sha> <build>`,
# on the commit it names and the pinned Xcode; see
# .claude/decisions/2026-11-12-development-macos-builds-provider-failover.md §3.
set -euo pipefail
cd "$(dirname "$0")"
die() { echo "ci.sh: $*" >&2; exit 2; }
what="${1:-}"
sha="${2:-}"
case "$what" in
  check) [ $# -eq 2 ] || die "usage: ci.sh check <sha>" ;;
  archive)
    [ $# -eq 3 ] || die "usage: ci.sh archive <sha> <build>"
    [[ "$3" =~ ^[0-9]+$ ]] || die "the build number must be digits"
    ;;
  *) die "usage: ci.sh check <sha> | ci.sh archive <sha> <build>" ;;
esac
[[ "$sha" =~ ^[0-9a-f]{40}$ ]] || die "the sha must be a full commit SHA"
head="$(git rev-parse HEAD)"
[ "$head" = "$sha" ] || die "HEAD is $head, not $sha"

# XcodeGen, one pin for every provider.
XCODEGEN_VERSION=2.44.1
XCODEGEN_SHA256=a2e905fb68446e9bb4008cdfe2e13e3f176d0cbcca828b71770f8e53fca91b73
tools="$(mktemp -d)"
trap 'rm -rf "$tools"' EXIT
curl -fsSLo "$tools/xcodegen.zip" \
  "https://github.com/yonaskolb/XcodeGen/releases/download/${XCODEGEN_VERSION}/xcodegen.zip"
(
  cd "$tools"
  printf '%s  xcodegen.zip\n' "$XCODEGEN_SHA256" | shasum -a 256 -c -
  unzip -q xcodegen.zip
  PREFIX="$tools/install" xcodegen/install.sh >/dev/null
)
export PATH="$tools/install/bin:$PATH"

# The first Xcode matching .xcode-version to its precision: 26.1 takes 26.1.1, nothing else.
want="$(tr -d '[:space:]' < .xcode-version)"
unset DEVELOPER_DIR
found=""
for app in /Applications/Xcode*.app; do
  [ -d "$app" ] || continue
  v="$(DEVELOPER_DIR="$app/Contents/Developer" xcodebuild -version 2>/dev/null | sed -n 's/^Xcode //p')"
  found="$found ${app##*/}=${v:-?}"
  if [ "$v" = "$want" ] || [[ "$v" == "$want".* ]]; then
    export DEVELOPER_DIR="$app/Contents/Developer"
    break
  fi
done
[ -n "${DEVELOPER_DIR:-}" ] || die "no Xcode $want; found:${found:- none}"

case "$what" in
  check) exec bash ./check.sh ;;
  archive) exec bash ./archive.sh "$3" ;;
esac
