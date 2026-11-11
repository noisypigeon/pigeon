## A native iOS app in src/pigeon-ios, built on Buildkite macOS agents and delivered to TestFlight

* Date: 2026-11-11
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: release
* Governs: `src/pigeon-ios/**`, `.gitignore`

## Context

The owner's direction:

- a native Swift application, as a new project `src/pigeon-ios/`;
- its first delivery is a blank screen with the text "Pigeon" centred;
- CI on Buildkite macOS agents builds and signs it, and delivers it to TestFlight;
- the control plane gains the connection to the owner's Apple Developer account;
- a `pigeon backplane` harness lets a Buildkite runner request the signing certificate from Cloudflare and load it for Xcode's tools.

What earlier ADRs fix, and this one keeps:

- **A project under `src/` is self-contained**: its manifest and build output stay inside it ([`2026-11-02-development-repository-area-layout.md`](2026-11-02-development-repository-area-layout.md) §1). `layout.yaml` already allows `pigeon-ios`.
- **Code checks hold no credentials and only test** ([`2026-11-10-development-code-checks-github-actions.md`](2026-11-10-development-code-checks-github-actions.md) §1). A change's head is public on GitHub once `bound` passes (§2 there).
- **Buildkite is a routed backend** whose pipelines' steps are fixed in its settings from `tooling/ci/buildkite/`, never uploaded from a checkout, and whose builds the control plane creates and reads with `BUILDKITE_TOKEN` ([`2026-11-08-development-ci-runners-github-buildkite.md`](2026-11-08-development-ci-runners-github-buildkite.md) §3, §5).
- **Delivery is the control plane's**: one deploy at a time from `MainRun`, of the newest push to `main`, for the units whose inputs changed (`cobot::scope::deploys`).
- **Long-lived credentials are Worker secrets**, and what a job gets is minted per job and never stored. The GitHub App key is the precedent: `GITHUB_APP_KEY` signs a short JWT in the Worker.

What is new, and why it needs a decision:

- **Neither runner can build iOS today.** Swift for iOS, `xcodebuild` and `codesign` need macOS. GitHub's and Cloudflare's runners here are Linux.
- **Signing needs a private key on the machine that signs.** That breaks "CI holds no credentials". This ADR narrows the break to one step, on `main`, that runs no project code, and holds the key only for that step's life.
- **App Store Connect's API key never leaves Cloudflare.** The control plane signs a JWT of at most 20 minutes, Apple's limit, for the one upload that needs it.

These facts were gathered when this ADR was written, and are checked in §9 step 2:

- Buildkite offers hosted macOS agents on Apple silicon with Xcode installed; the free plan's hosted minutes may be Linux only. A self-hosted agent on a Mac the owner runs serves the same queues.
- Buildkite issues OIDC tokens to a job (`buildkite-agent oidc request-token`), signed RS256 by `https://agent.buildkite.com`, carrying `organization_slug`, `pipeline_slug`, `build_number`, `build_branch`, `build_commit` and `step_key`.
- App Store Connect API keys are ES256 (`.p8`, PKCS#8 P-256); a token's `exp` is at most 20 minutes after `iat`, with `aud: appstoreconnect-v1`.
- `iTMSTransporter` accepts `-jwt` in place of a key file.
- macOS's `security` tool creates, unlocks, lists and deletes a keychain, and imports a `.p12` into it; `set-key-partition-list` lets `codesign` use the key without a prompt.
- `xcodebuild -exportArchive` re-signs an archive built with `CODE_SIGNING_ALLOWED=NO`, without running its build phases.
- An internal TestFlight group with automatic distribution receives each processed build without an API call.

## Implementation plan.

### 1. `src/pigeon-ios/`

```
src/pigeon-ios/
  README.md              # one line, naming this ADR
  .xcode-version         # the Xcode the agents select
  project.yml            # XcodeGen; the .xcodeproj is generated, never committed
  Pigeon/
    PigeonApp.swift      # @main App, one WindowGroup
    ContentView.swift    # Text("Pigeon"), centred
    Assets.xcassets/     # AccentColor; AppIcon arrives in §9 step 7
```

- **SwiftUI, Swift 6 language mode,** iPhone and iPad, deployment target iOS 18.0.
- **XcodeGen.** `project.yml` is the manifest; `xcodegen generate` writes `Pigeon.xcodeproj`, ignored by `.gitignore` with `/src/pigeon-ios/build`. XcodeGen is pinned in `mise.toml`.
- **Info.plist is generated** (`GENERATE_INFOPLIST_FILE`), with `ITSAppUsesNonExemptEncryption: NO`, so TestFlight asks no export-compliance question.
- **Identity.** Bundle ID `dev.pigeon.Pigeon`; `MARKETING_VERSION` in `project.yml`. The team ID, build number and profile are passed at signing (§4, §6), so no Apple account identifier is in the tree.
- **`mise run ios`** generates and builds for the simulator, unsigned. It runs on macOS only and is not part of `mise run ci`; on Linux, a change to `src/pigeon-ios/` is compiled only by its `ios` check (§3).

### 2. Buildkite: two macOS pipelines, two queues

| Pipeline | Builds | Queue | Credentials |
| --- | --- | --- | --- |
| `pigeon-ios-check` | any change head, and `main` | `macos` | none |
| `pigeon-ios-release` | `main` only (branch filter) | `macos` for `archive`, `macos-sign` for `sign` | the signing identity, for `sign` alone |

- **Steps are fixed in each pipeline's settings,** from `tooling/ci/buildkite/ios-check.yml` and `ios-release.yml`, as for `pigeon-ci`.
- **Checkout** is the GitHub mirror, `noisypigeon/pigeon`, anonymously, at the dispatched commit. Each script fails unless `HEAD` equals the `sha` it was dispatched with.
- **Queues.** `macos-sign` agents run nothing but `sign`. On a self-hosted Mac it is a separate agent with its own user and its own keychain.
- **Hosted or self-hosted** is the owner's choice in §9 step 2; nothing else here depends on it.

### 3. The change check: `ios`

- `change.scope` maps `src/pigeon-ios/**` to one check, `ios`, with `argv` `tooling/ci/buildkite/ios-check.sh`.
- The router sends `ios` to Buildkite's `pigeon-ios-check`, and only there: no other backend runs macOS. When Buildkite is down, the check waits as `waiting: Buildkite`.
- `ios-check.sh`: select the Xcode in `.xcode-version`, `xcodegen generate`, then `xcodebuild build-for-testing` and `test-without-building` on an iOS simulator, with `CODE_SIGNING_ALLOWED=NO`.
- Results return as §5 of the runners ADR decides: build ID from creation, jobs and the failed log read back with `BUILDKITE_TOKEN`.

### 4. The release

**Trigger.** `MainRun`'s deploy gains the unit `ios`: when `src/pigeon-ios/**` or `tooling/ci/buildkite/ios-*` changed since the last released commit, and the GitHub mirror holds that commit, it creates a `pigeon-ios-release` build of it. A D1 row in `ios_releases` holds `(sha, build_number, buildkite_build, state, nonce)`; `build_number` is the previous one plus one, so `CFBundleVersion` always rises.

**Step `archive`** (`macos`, no credentials): `xcodebuild archive` with `CODE_SIGNING_ALLOWED=NO` and `CURRENT_PROJECT_VERSION=<build_number>`, then uploads `Pigeon.xcarchive.zip` as a build artifact. All project code, build phases included, runs here.

**Step `sign`** (`macos-sign`, `depends_on: archive`), `tooling/ci/buildkite/ios-sign.sh`. It runs no app code, only `main`'s `pigeon`:

1. builds `pigeon` from `main`'s `src/pigeon-cli` with `--no-default-features --features signing`, before any secret is on the machine;
2. downloads the archive artifact;
3. `pigeon backplane signing load` (§6): the certificate and profile from Cloudflare, loaded for `xcodebuild`;
4. `xcodebuild -exportArchive -exportOptionsPlist <the plist load wrote>`;
5. `pigeon backplane signing upload --ipa Pigeon.ipa`;
6. `pigeon backplane signing clean`, in an `EXIT` trap.

**After the upload** the control plane, on `MainRun`'s alarm, polls `GET /v1/builds` for the build number until `processingState` is `VALID`, then sets the build's *What to Test* (`betaBuildLocalizations`) from the titles of the changes merged since the last release. The internal group distributes it. `deploy_status` shows the release's state and links the Buildkite build.

### 5. The Apple connection

**Worker secrets on `pigeon-control`,** added to `worker.yaml`'s `secrets:` with the owner's approval, put with `pigeon cobot workers secrets put`:

| Secret | What |
| --- | --- |
| `APPLE_ASC_KEY` | the App Store Connect Team API key, `.p8`, role App Manager |
| `APPLE_ASC_KEY_ID`, `APPLE_ASC_ISSUER` | its key ID and issuer ID |
| `APPLE_TEAM_ID` | the developer team |
| `APPLE_DIST_P12`, `APPLE_DIST_P12_PASSWORD` | the Apple Distribution certificate and key, base64 |
| `BUILDKITE_TOKEN` | as the runners ADR §5, plus `read_artifacts` |

None is in the tree, a log, a change row or R2.

**`backplane::apple`, the functional core:**

- `asc_claims(key_id, issuer, now, ttl)`: the header and claims; `ttl` is capped at 20 minutes.
- `release_grant(claims, row)`: accepts a Buildkite OIDC token's claims only when `organization_slug` is `noisypigeon`, `pipeline_slug` is `pigeon-ios-release`, `step_key` is `sign`, `build_branch` is `main`, and `build_commit` and `build_number` equal an `ios_releases` row in state `archiving` that has not been redeemed.
- the release row's state machine: `queued`, `archiving`, `signing`, `uploaded`, `processing`, `delivered`, `failed`.

**`backplane::worker::apple`, the shell:**

- ES256 through WebCrypto (`importKey` PKCS#8, ECDSA P-256), whose raw `r‖s` signature is already JWS's form.
- A small App Store Connect client: `apps`, `profiles`, `builds`, `betaBuildLocalizations`, each request with a fresh JWT.
- **`POST /v0/apple/release`**, the one route Access bypasses. Its only authentication is the OIDC token: verified RS256 against Buildkite's JWKS (reusing `access.rs`'s verifier), then `release_grant`. It redeems the row once, moves it to `signing`, and answers a `Grant` (§6): the `.p12`, its password, the App Store profile for `dev.pigeon.Pigeon` fetched from `profiles` with its UUID and name, the team ID and a 20-minute upload JWT. A refusal is an event on the instance and a line in the digest.
- **`apple.check`**, a backplane operation (`pigeon backplane run apple.check`): a JWT is accepted, the app exists for the bundle ID, an App Store profile is active, and the certificate is valid. The hourly digest names a certificate or profile expiring within 30 days.

### 6. The runner harness: `pigeon backplane signing`

The runner's half of §5's exchange. It runs on `macos-sign` agents only, and is the only thing there that touches the certificate.

**A feature of its own.** `pigeon`'s new feature `signing` holds only this command, its HTTP client and `serde_json`, so it builds on macOS in a minute without the engines. `mise run features` clippies it alone.

**`backplane::signing`, the functional core,** testable on Linux:

- `Grant`: the route's answer, `{p12, p12_password, profile, profile_uuid, profile_name, team_id, bundle_id, upload_jwt, expires}`, with `Debug` redacting every secret field.
- `load_plan(grant, dirs) -> Vec<Cmd>` and `clean_plan(state) -> Vec<Cmd>`: the exact `openssl` and `security` invocations, as data. No secret is in a `Cmd`'s argv; secrets go in its environment or stdin.
- `export_options(grant)`: the `ExportOptions.plist`: `method: app-store-connect`, `signingStyle: manual`, `teamID`, and `provisioningProfiles` mapping the bundle ID to the profile's name.

**`pigeon backplane signing load`:**

1. reads `BUILDKITE_JOB_ID` and `BUILDKITE_STEP_KEY`, and stops unless the key is `sign`;
2. `buildkite-agent oidc request-token --audience https://platform.pigeon.dev --lifetime 300`;
3. `POST https://platform.pigeon.dev/v0/apple/release` with the token as `Authorization: Bearer`; anything but a `Grant` is a failure, printed without the body;
4. re-wraps the `.p12` under a one-time random password (`openssl pkcs12 -passin env:… -passout env:…`), so the long-lived password never reaches argv;
5. creates `pigeon-sign-<job>.keychain-db` in a fresh `0700` directory under `$TMPDIR`, with a random password; sets it to lock after 30 minutes; unlocks it; imports the re-wrapped `.p12` with `-T /usr/bin/codesign -T /usr/bin/security`; runs `set-key-partition-list -S apple-tool:,apple:`; and prepends it to the user's keychain search list;
6. writes the profile to `~/Library/MobileDevice/Provisioning Profiles/<profile_uuid>.mobileprovision`;
7. writes `ExportOptions.plist`, and the upload JWT to a `0600` file, in the same directory;
8. writes `state.json` (the keychain, the previous search list, the files it wrote) there, and prints only the plist's path and the grant's expiry.

**`pigeon backplane signing upload --ipa FILE`:** runs `iTMSTransporter -m upload -assetFile FILE -jwt <token>`, reading the token from the file; the JWT expires 20 minutes after it was signed.

**`pigeon backplane signing clean`:** from `state.json`, restores the search list, deletes the keychain, the profile and every file written, then the directory. It is idempotent, and exits 0 when there is nothing to clean. The `macos-sign` agents also run it from a `pre-exit` hook, so a killed job leaves nothing behind.

### 7. Security

- **A change's code never meets a credential.** Checks and `archive` hold none. `sign` runs only `ios-sign.sh` and the harness, both from `main`, and `release_grant` refuses any other step, pipeline, branch or commit.
- **One redemption per release.** A second exchange for the same row is refused and reported.
- **What a stolen grant gives:** the distribution key, until the owner revokes it in the developer portal, and 20 minutes of App Manager access. The App Store Connect key itself is never handed out.
- **Approval tiers.** `.config/approvals.yaml`'s `secrets:` gains `tooling/ci/buildkite/ios-sign.sh`, `tooling/ci/buildkite/ios-release.yml`, `src/pigeon-cli/crates/backplane/src/apple.rs`, `src/pigeon-cli/crates/backplane/src/worker/apple.rs` and `src/pigeon-cli/crates/backplane/src/signing.rs`, so the owner approves every change to what handles the key.
- **Revocation runbook** in `src/pigeon-ios/README.md`: revoke the certificate and the API key in Apple's portals, put new secrets, run `apple.check`.

### 8. What this does not decide

- App Store review and public release; TestFlight external groups.
- Any feature of the app beyond the blank screen.
- A release unit in `.config/releases.yaml` and versioning by the release bot; `MARKETING_VERSION` is edited by hand until then.

### 9. Execution

1. **This ADR,** with `src/pigeon-ios/README.md`.
2. **The owner, once:**
   1. Check §Context's facts against Apple's and Buildkite's documentation; correct this ADR by amendment where they differ.
   2. Register the App ID `dev.pigeon.Pigeon`, create the app in App Store Connect, and an internal TestFlight group with automatic distribution.
   3. Create an Apple Distribution certificate, export it as `.p12`, and an App Store profile for the App ID.
   4. Create a Team API key with the App Manager role.
   5. Create the Buildkite agents for `macos` and `macos-sign`, hosted or self-hosted, and the two pipelines from §2.
   6. Put §5's secrets.
3. **`feat(ios)`: the blank screen,** §1: `project.yml`, the two Swift files, assets, `.xcode-version`, the `.gitignore` lines and `mise run ios`.
4. **`feat(backplane)`: the Apple connection,** §5: the core with its tests, the shell, `apple.check`, the route, and the new secret names; then the `approvals.yaml` change of §7 in its own change.
5. **`feat(backplane)`: the runner harness,** §6: the `signing` feature, `backplane::signing` with its tests, and the three subcommands.
6. **`ci`: the `ios` check,** §3: `tooling/ci/buildkite/ios-check.{yml,sh}`, its scope rule and routing.
7. **`feat(backplane)`: the release,** §4: `ios_releases`, `ios-release.yml`, `ios-sign.sh`, the `ios` deploy unit and its polling; with an AppIcon, which App Store Connect requires.

### 10. Verification

- `apple.release_grant`'s tests: each wrong claim is refused, a second redemption is refused, and a valid grant moves the row to `signing`. `asc_claims` never exceeds 20 minutes.
- `pigeon backplane run apple.check` passes against the owner's account.
- A change touching only `src/pigeon-ios/` runs `bound` and `ios`; its `ios` job links to a green `pigeon-ios-check` build.
- `backplane::signing`'s tests: `load_plan` puts no secret in any argv, `clean_plan` undoes every step of `load_plan` and is empty for an empty state, `export_options` matches a fixture, and `Grant`'s `Debug` shows no secret.
- `pigeon backplane signing load` outside a `sign` step stops before any request; `clean` after a failed `load` leaves the search list as it was.
- After step 7's merge, `main`'s deploy releases build 1, and it appears in the internal group, showing "Pigeon" centred.
- An OIDC token from `archive`, from `pigeon-ios-check`, or replayed after `sign`, is refused at `/v0/apple/release`.
- No Apple key, certificate, key ID or team ID appears in the tree, a Buildkite log, a change row or R2.

## Amendments

### 2026-11-11: The bundle ID is dev.pigeon.app

The owner registered the App ID as `dev.pigeon.app`. It replaces `dev.pigeon.Pigeon` in §1, §5 and §9 step 2.2; that text stays as written.

- **Lowercase,** so no tool's case handling can split the App ID from its profile.
- **Platform-neutral,** so a later Mac app or extension sits under it (`dev.pigeon.app.<name>`).
- `src/pigeon-ios/project.yml`'s `PRODUCT_BUNDLE_IDENTIFIER` follows. The home-screen name stays `Pigeon` (`CFBundleDisplayName`); the App Store name is set in App Store Connect and is not in the tree.
- The App ID has no capabilities beyond Apple's defaults. The first capability's change adds an entitlements file, and checks that `-exportArchive` applies its entitlements to the unsigned archive (§4).

### 2026-11-11: GitHub Actions builds and tests; Buildkite's hosted agents only sign

At the owner's direction, a change's checks and the release's build move to GitHub Actions' macOS runners, and Buildkite is kept for signing alone. This supersedes §2, §3, the `archive` step of §4 and the `ios` check of §9 step 6; that text stays as written.

#### 1. Tests: `main`'s `ci.yml`

- `change.scope` maps `src/pigeon-ios/**` to the check `ios`, which runs in `.github/workflows/ci.yml` like every code check ([`2026-11-10-development-code-checks-github-actions.md`](2026-11-10-development-code-checks-github-actions.md)): `main`'s workflow, dispatched on the head, holding no secret.
- `ci.yml` runs an `ios` check on a `macos` runner, and every other check on Linux, as now.
- The `ios` job selects the Xcode in `.xcode-version`, runs `xcodegen generate`, and builds and tests on a simulator with `CODE_SIGNING_ALLOWED=NO`.
- There is no `pigeon-ios-check` pipeline, and no `macos` queue.

#### 2. The release's archive: GitHub Actions

- `MainRun`'s `ios` unit dispatches `ci.yml` on the released commit with the one check `ios.archive`, once the GitHub mirror holds it.
- `ios.archive` runs §4's `archive` command on a `macos` runner and uploads `Pigeon.xcarchive.zip` as the run's artifact.
- The control plane copies it to R2 at `builds/<sha>/ios/Pigeon.xcarchive.zip`, as it copies the test map, and only then creates the Buildkite build.

#### 3. Signing: one hosted queue, one step

- Buildkite organization `willow-graysen`; cluster `pigeon`; one hosted macOS queue, `macos-sign`, with the Xcode of `.xcode-version`.
- One pipeline, `pigeon-ios-release`: the one step `sign`, fixed in its settings; no build triggers; branch filtering `main`; rebuilds disabled.
- Each job is a fresh machine, so no `pre-exit` hook; `ios-sign.sh` still runs `signing clean` on exit.
- `ios-sign.sh` installs the Rust toolchain `mise.toml` pins to build the harness, and stops before `signing load` unless `xcrun --find iTMSTransporter` succeeds.
- The archive comes in the grant: `Grant` gains `archive_url`, an R2 URL for `builds/<sha>/ios/Pigeon.xcarchive.zip` valid for 15 minutes, which `signing load` downloads into its directory.
- `release_grant` requires `organization_slug` `willow-graysen`, in place of `noisypigeon`.
- `BUILDKITE_TOKEN` is scoped to `willow-graysen` with `read_builds`, `write_builds`, `read_build_logs` and `read_artifacts`, and no GraphQL access.

#### 4. The secret names, ahead of step 4

`infra/pigeon/workers/control/worker.yaml` declares §5's secrets now, so the owner can put them before step 4 lands. Until each is put, a deploy names it as missing; nothing reads them before step 4.

#### 5. Execution, from here

1. This amendment, with the secret names.
2. The owner: puts §5's secrets.
3. Step 4, the Apple connection, with `archive_url` and the `willow-graysen` check.
4. The `ios` check in `ci.yml` and `change.scope`.
5. The release: `ios.archive`, its copy to R2, `ios_releases`, `ios-sign.sh`, the `ios` deploy unit, and an AppIcon.
