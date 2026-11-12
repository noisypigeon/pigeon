#!/usr/bin/env bash
# `ios.archive`: a release's archive, ad-hoc signed so it carries its entitlements, for build number
# $1, zipped for the control plane; see .claude/decisions/2026-11-11-release-ios-app-buildkite-testflight.md,
# its 2026-11-12 amendment, and .claude/decisions/2026-11-12-runtime-ios-app-approvals-insights.md.
set -euo pipefail
build="${1:?the build number}"
[[ "$build" =~ ^[0-9]+$ ]] || { echo "the build number must be digits" >&2; exit 2; }
cd "$(dirname "$0")"
want="$(tr -d '[:space:]' < .xcode-version)"
if [ -d "/Applications/Xcode_${want}.app" ]; then
  export DEVELOPER_DIR="/Applications/Xcode_${want}.app/Contents/Developer"
fi
xcodebuild -version
xcodegen generate --spec project.yml
rm -rf build/Pigeon.xcarchive build/Pigeon.xcarchive.zip
xcodebuild archive \
  -project Pigeon.xcodeproj \
  -scheme Pigeon \
  -configuration Release \
  -destination 'generic/platform=iOS' \
  -archivePath build/Pigeon.xcarchive \
  CODE_SIGN_IDENTITY=- \
  AD_HOC_CODE_SIGNING_ALLOWED=YES \
  CODE_SIGNING_REQUIRED=NO \
  CODE_SIGN_STYLE=Manual \
  CURRENT_PROJECT_VERSION="$build"
(cd build && ditto -c -k --keepParent Pigeon.xcarchive Pigeon.xcarchive.zip)
ls -l build/Pigeon.xcarchive.zip
