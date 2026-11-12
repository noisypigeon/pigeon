#!/usr/bin/env bash
# The `ios` code check: generate the project, build it for the simulator unsigned, and test PigeonCore; see
# .claude/decisions/2026-11-11-release-ios-app-buildkite-testflight.md §3 and its amendments.
set -euo pipefail
cd "$(dirname "$0")"
want="$(tr -d '[:space:]' < .xcode-version)"
if [ -d "/Applications/Xcode_${want}.app" ]; then
  export DEVELOPER_DIR="/Applications/Xcode_${want}.app/Contents/Developer"
fi
xcodebuild -version
xcodegen generate --spec project.yml
xcodebuild build \
  -project Pigeon.xcodeproj \
  -scheme Pigeon \
  -destination 'generic/platform=iOS Simulator' \
  -derivedDataPath build \
  CODE_SIGNING_ALLOWED=NO
# PigeonCore's tests, on the first iPhone simulator this Xcode has.
udid="$(xcrun simctl list devices available | grep -m1 -E '^ +iPhone' | grep -oE '[0-9A-F-]{36}')"
[ -n "$udid" ] || { echo "no iPhone simulator is available" >&2; exit 1; }
xcodebuild test \
  -project Pigeon.xcodeproj \
  -scheme Pigeon \
  -destination "id=$udid" \
  -derivedDataPath build \
  -only-testing:PigeonCoreTests \
  CODE_SIGNING_ALLOWED=NO
