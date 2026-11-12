#!/usr/bin/env bash
# The release's `sign` step on Buildkite's hosted macOS agent: load the signing identity, export
# the archive GitHub built, upload it to App Store Connect, and clean up; see the 2026-11-12
# amendment to .claude/decisions/2026-11-11-release-ios-app-buildkite-testflight.md.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
build="${PIGEON_IOS_BUILD:?the build number}"
[[ "$build" =~ ^[0-9]+$ ]] || { echo "PIGEON_IOS_BUILD must be digits" >&2; exit 2; }

want="$(tr -d '[:space:]' < src/pigeon-ios/.xcode-version)"
if [ -d "/Applications/Xcode_${want}.app" ]; then
  export DEVELOPER_DIR="/Applications/Xcode_${want}.app/Contents/Developer"
fi
xcodebuild -version
xcrun --find iTMSTransporter >/dev/null || { echo "iTMSTransporter is missing" >&2; exit 1; }

# The harness, from this commit, with the toolchain mise.toml pins; built before any secret is here.
rust="$(sed -n 's/^rust = { version = "\([^"]*\)".*/\1/p' mise.toml)"
if ! command -v rustup >/dev/null; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain "$rust"
fi
export PATH="$HOME/.cargo/bin:$PATH"
rustup toolchain install "$rust" --profile minimal
cargo +"$rust" build --release --locked -p pigeon --no-default-features --features signing \
  --manifest-path src/pigeon-cli/Cargo.toml
pigeon="$PWD/src/pigeon-cli/target/release/pigeon"

work="$(mktemp -d)"
trap '"$pigeon" backplane signing clean || true; rm -rf "$work"' EXIT

plist="$("$pigeon" backplane signing load)"

token="$(buildkite-agent oidc request-token --audience https://platform.pigeon.dev --lifetime 300)"
curl -fsS -H "Authorization: Bearer $token" -o "$work/archive.zip" \
  https://platform.pigeon.dev/v0/apple/release/archive
unset token
ditto -x -k "$work/archive.zip" "$work"

xcodebuild -exportArchive \
  -archivePath "$work/Pigeon.xcarchive" \
  -exportOptionsPlist "$plist" \
  -exportPath "$work/export"
"$pigeon" backplane signing upload --ipa "$work/export/Pigeon.ipa"
echo "uploaded build $build"
