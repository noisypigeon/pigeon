# pigeon-ios

The native iOS app, built on Buildkite macOS agents and delivered to TestFlight; see [`2026-11-11-release-ios-app-buildkite-testflight.md`](../../.claude/decisions/2026-11-11-release-ios-app-buildkite-testflight.md).

Phase one is Home (insights and approvals), Face ID approvals, Auto, and Profiles with Settings;
see [`2026-11-12-runtime-ios-app-approvals-insights.md`](../../.claude/decisions/2026-11-12-runtime-ios-app-approvals-insights.md).

- `PigeonCore/`: pure Swift, tested by `check.sh` on a simulator.
- `Pigeon/`: the app.
