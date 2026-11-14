## macOS jobs fail over from GitHub Actions to Bitrise, then Buildkite, when a provider gives no machine

* Date: 2026-11-12
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development
* Governs: `src/pigeon-ios/check.sh`, `src/pigeon-ios/archive.sh`, `.github/workflows/ci.yml`, `tooling/ci/buildkite/ios-sign.sh`, `src/pigeon-cli/crates/backplane/src/iosrelease.rs`, `src/pigeon-cli/crates/backplane/src/actions.rs`, `src/pigeon-cli/crates/backplane/src/worker/buildkite.rs`, `.config/insights.yaml`

## Context

Two kinds of job need macOS, and both run only on GitHub's hosted `macos-15` runners today:

- **the `ios` code check,** dispatched in `main`'s `ci.yml` with a change's other checks ([`2026-11-10-development-code-checks-github-actions.md`](2026-11-10-development-code-checks-github-actions.md), and the GitHub/Buildkite amendment of [`2026-11-11-release-ios-app-buildkite-testflight.md`](2026-11-11-release-ios-app-buildkite-testflight.md));
- **a release's `ios.archive`,** which `MainRun` dispatches alone, copies to R2, and hands to Buildkite's `sign` step (that ADR's 2026-11-12 amendments).

**What happened on 2026-11-12.** Two macOS jobs were created, never got a runner, and were cancelled by GitHub 15 minutes and 1 second later, with no log:

| Job | Created | Cancelled | `runner_id`, `runner_name`, `steps` |
|---|---|---|---|
| !300's `ios`, run 43319619862 | 22:24:38 | 22:39:39 | absent |
| build 6's `ios.archive`, run 43323292545 | 22:48:33 | 23:03:34 | absent |

A rerun of !300's check minutes later got a runner and passed in 2 minutes 45 seconds. Build 6 failed as `GitHub's archive failed`, the same text as build 5, whose archive did run and failed on our code. So today a lost machine and a broken build look the same, and each stops a release or a check.

What these jobs show:

- **GitHub sets a queued job's `started_at` to its `created_at`.** So `started_at` doesn't show whether a runner was assigned. `runner_id` does: it's absent until a runner takes the job.
- **The runners ADR's router was never built.** [`2026-11-08-development-ci-runners-github-buildkite.md`](2026-11-08-development-ci-runners-github-buildkite.md) §4 and §6 (`.config/ci.yaml`, `backplane::ci`, the `Backends` object) don't exist; the code-checks ADR took a simpler path. This ADR takes §6's stall rules for macOS jobs only.

**The owner's order:** GitHub Actions first, because it's free; Bitrise second, because it's cheaper than Buildkite; Buildkite last. Buildkite's hosted M4 medium agent, at $0.12 a minute, already runs the release's `sign` step.

**What Bitrise offers.** From [bitrise.io/pricing](https://bitrise.io/pricing), read 2026-11-12:

- **Hobby: free.** It gives 500 build minutes a month on Linux or macOS, 1 private app, unlimited public apps, and unlimited team members. Builds time out at 90 minutes. It needs a verified card, which is charged only if Pay-As-You-Go is on.
- **Hobby's Pay-As-You-Go:** each minute past 500 is billed at 1.5× its rate, up to $15 a month. At the cap, no new builds start until the next cycle. A month under $5 isn't billed.
- **Hobby's macOS machine:** M2 Pro Medium, with 4 CPUs at 3.7 GHz and 6 GB of RAM, at $0.0072 a minute. Pay-As-You-Go makes that $0.0108.
- **Starter:** $99 a month, or $89 a month billed annually. It allows unlimited usage, up to 4 concurrent builds, and 3 private apps. Its macOS Medium (M4 or M2 Pro) is $0.0072 a minute.
- **Pro:** from $218 a month, or $200 billed annually. It gives 250 to 2,000 builds a month and 10 concurrent macOS builds.
- The pricing page doesn't state Hobby's concurrency.

From [docs.bitrise.io](https://docs.bitrise.io/en/bitrise-platform/integrations/oidc-authentication/oidc-authentication-overview.html) and its API pages:

- **API.** It's at `https://api.bitrise.io/v0.1`, with a personal access token or a Workspace API token in `Authorization`.
  - `POST /apps/{app}/builds` triggers a build. It takes `build_params` (`workflow_id`, `branch`, `commit_hash`, `environments`) and `hook_info: {type: bitrise}`.
  - `GET /apps/{app}/builds/{build}` reads a build. It answers `status`: 0 is not finished (starting, running or on hold), 1 succeeded, 2 failed, 3 aborted with failure, 4 aborted with success. It also answers `started_on_worker_at`, `is_on_hold`, `abort_reason`, `machine_type_id`, `stack_identifier` and `finished_at`.
  - `POST /apps/{app}/builds/{build}/abort` aborts a build.
  - `GET /apps/{app}/builds/{build}/artifacts` lists a build's artifacts, and reading one gives a download URL.
  - `GET /apps/{app}/bitrise.yml/config` says whether `bitrise.yml` is stored on bitrise.io or in the repository; `GET /apps/{app}/bitrise.yml` returns it.
- **OIDC.** The *Get OIDC Identity Token* step sets `BITRISE_IDENTITY_TOKEN` for a chosen audience. The issuer is `https://token.builds.bitrise.io`. The token's claims include `sub` (`app:<app>:workflow:<workflow>`), `app_slug`, `workspace_slug`, `workflow`, `build_number`, `sha`, `branch`, `repository_url`, `trigger_by` and `jti`.
- **Build Hub** is Bitrise's runners for GitHub Actions. It isn't used here: its jobs are still scheduled by GitHub, so it doesn't make us independent of GitHub's scheduling.

These are checked again in §8 step 2, as the release ADR's facts were.

## Implementation plan.

### 1. What counts as unavailable

A **macOS job** is one `ios` check of a change head, or one release's `ios.archive`. Each attempt on a provider ends in one of these **outcomes**, all decided by a pure `backplane::macos::outcome` from the provider's own answer:

| Outcome | GitHub (`jobs[]` of the run) | Bitrise (`builds/{build}`) | Buildkite (`jobs[]` of the build) |
|---|---|---|---|
| **ran**, passed or failed | `runner_id` set, `conclusion` `success` or `failure` | `started_on_worker_at` set, `status` 1 or 2 | `started_at` set, `state` `passed` or `failed` |
| **unassigned** | no `runner_id` after `start_within`, or `cancelled` with no `runner_id` | `started_on_worker_at` null after `start_within`, or `status` 3 with it null | no `started_at` after `start_within`, or `canceled` or `expired` with none |
| **lost** | `runner_id` set, `cancelled`, and our step not completed | `status` 3 after starting, with no failed step of ours | `state` `canceled` after starting, or an agent-lost exit status |
| **refused** | dispatch fails 3 times (5xx, timeout, 429), or no run appears within 10 minutes | trigger fails 3 times, or `is_on_hold` after `start_within` | create fails 3 times |

- **`start_within` is 10 minutes** for every provider. That's below the 15 minutes after which GitHub cancelled both jobs, and above the 5 seconds a runner took on the healthy rerun.
- **A failed job that ran is a result,** never a reason to move. This holds even when it timed out. The change's check fails, or the release fails, as now.
- **`unassigned`, `refused` and a first `lost` move the job** to the next provider (§2). A second `lost` for the same job is a failure: two lost machines running the same commit point at the commit.
- **Health.** Two moves away from a provider within 30 minutes mark it **down** for new macOS jobs. After 15 minutes the next job probes it alone. If the probe runs, the provider is healthy again. If not, the wait doubles, up to 2 hours.
- Health is kept on the `Instance` object, beside the CI window. Each change of health is a `macos.provider` event in the instance's event log.

### 2. How a job moves

- **The order is `.config/macos.yaml`'s,** a reviewed file read at build time like `.config/insights.yaml`:

  ```yaml
  # Where macOS jobs run, first available first; see .claude/decisions/2026-11-12-development-macos-builds-provider-failover.md.
  order: [github, bitrise, buildkite]
  start_within_minutes: 10
  github_monthly_minutes: null
  bitrise_monthly_minutes: 450
  buildkite_monthly_minutes: 150
  ```

- **`backplane::macos::next(order, health, usage, tried, now)`** picks the first provider in `order` that is healthy, under its month's minutes, and not yet tried for this job. When none is left, the job waits as `waiting: macOS (<each provider and why>)`, and is offered again every 15 minutes from the start of `order`. The digest names any macOS job that has waited over an hour.
- **Each attempt is a row** in D1's new `macos_jobs` table (migration `0006_macos_jobs.sql`, expand only):
  - `kind` (`check` or `archive`), `number` (the change, or the release's build), `sha`;
  - `provider`, `attempt`, the provider's run or build ID, and `url`;
  - `outcome`, `queued_at`, `started_at`, `finished_at` and `minutes` (rounded up).
- **A moved job's old attempt is abandoned,** and its late result is recorded but ignored, as a result for an old head is today.
  - A release's lone archive dispatch is cancelled through GitHub's API.
  - A change's `ios` check shares a run with its Linux checks, and GitHub can't cancel one job of a run, so it's left to run. It costs nothing.
  - Bitrise builds are aborted, and Buildkite builds cancelled.
- **The change check.** `Actions` stops waiting for `ios` once it moves, and the Linux checks of the same dispatch go on as now. `ChangeRun` keeps the macOS attempts beside `actions`. `change_status` shows the `ios` job's provider, attempt and link. A move is a `ci.moved` event on the change, so `events_wait` sees it.
- **The CI window** (the code-checks ADR's 2026-11-11 amendment) sends code checks to the `Checker`, which can't run macOS. While it's open, `ios` starts at the first provider after `github`.
- **The release.** `ios_releases` gains `archive_provider` and `archive_build`. `MainRun`'s `archiving` stage runs the attempts. A release no longer fails on `unassigned`, `refused` or a first `lost`; it fails only when the archive ran and failed, or the job waited 2 hours. The release ADR's "nothing is retried on its own" still holds for every other failure.
- **The archive is copied from wherever it was built:** GitHub's run artifact (as now), Bitrise's build artifact, or Buildkite's job artifact (with `read_artifacts`, which `BUILDKITE_TOKEN` already has). It goes to the same R2 key, `builds/<sha>/ios/<build>.xcarchive.zip`, and on to the same `sign` step.
- **The owner can force a provider.** `ci_rerun` with `on: github`, `on: bitrise` or `on: buildkite` runs a change's `ios` check there. `ios.release` with `on` archives there. Both are at the owner tier, and both ignore health and minutes.

### 3. The same job everywhere

Every provider runs one entrypoint from the commit it checks, `src/pigeon-ios/ci.sh`:

```
ci.sh check <sha>
ci.sh archive <sha> <build>
```

1. **It stops unless `git rev-parse HEAD` equals `<sha>`.**
2. **It installs XcodeGen** at the version and SHA-256 now in `ci.yml`, moved into `ci.sh`, so one pin serves every provider.
3. **It selects Xcode.** It takes the first `/Applications/Xcode*.app` whose `xcodebuild -version` matches `.xcode-version` to its precision: `26.1` matches 26.1.1, and nothing else. It stops, naming what it found, when none matches. Today `check.sh` and `archive.sh` fall back to whatever Xcode is default; that fallback goes.
4. **It runs `check.sh`, or `archive.sh <build>`,** unchanged in what they build.

What each provider holds:

- **GitHub:** `ci.yml`'s macOS steps call `ci.sh`. Its trigger, permissions and pinned actions are unchanged.
- **Bitrise:** one app on the public mirror `noisypigeon/pigeon`. It has no webhooks or triggers, and its `bitrise.yml` is stored on bitrise.io, not in the repository.
  - The file's source is `tooling/ci/bitrise/bitrise.yml`, with two workflows, `ios-check` and `ios-archive`, on a stack with the Xcode of `.xcode-version`.
  - Each workflow clones `commit_hash` and runs `ci.sh` with `PIGEON_SHA` and `PIGEON_IOS_BUILD` from `environments`. `ios-archive` deploys `src/pigeon-ios/build/Pigeon.xcarchive.zip` as the build's artifact.
- **Buildkite:** a new pipeline, `pigeon-ios-build`, on a new hosted queue, `macos-build`, so `macos-sign` still runs nothing but `sign`.
  - Its one step is fixed in its settings from `tooling/ci/buildkite/ios-build.yml`, and it uploads the archive as an artifact.
  - It has no build triggers, and builds any commit the control plane names.
- **The provider's config must be `main`'s.** Before each attempt, the control plane compares the SHA-256 of the config the provider holds with the one built into the Worker from `main`:
  - Bitrise's `bitrise.yml`, which must also be stored on bitrise.io;
  - Buildkite's pipeline steps.

  A mismatch makes that provider unavailable, with the reason, and the digest asks the owner to paste the file. So a change can never choose how it is checked, as `ci.yml`'s dispatch from `main` already ensures.
- **The sign step checks the archive's Xcode.** `ios-sign.sh` reads `DTXcode` from the archived app's `Info.plist`, and stops unless it matches `.xcode-version`. So whichever provider archived it, the IPA sent to Apple was built with the pinned Xcode.
- **Every provider clones from the GitHub mirror.** This ADR makes GitHub's macOS runner capacity redundant, not GitHub's git. While `main` or a change's head isn't on GitHub, macOS jobs wait, as all code checks do now.

### 4. Trust and secrets

- **No macOS build holds a secret.** Checks and archives hold none, wherever they run. Signing stays on Buildkite's `macos-sign`, and `release_grant` still accepts only `pigeon-ios-release`'s `sign` step. An OIDC token from `pigeon-ios-build`, or from Bitrise, is refused at `/v0/apple/release`.
- **Signing stays on one provider.** A second signing provider would put the distribution key on a second set of machines. It would also mean a second OIDC trust in `release_grant`, and the owner's approval of a second runner. The sign step runs only `main`'s code, for about 4 minutes, and Buildkite's agents haven't failed to start it. If that changes, a later ADR can add Bitrise's OIDC (§Context) as a second issuer in `release_grant`.
- **The control plane calls each provider; no provider calls the control plane.** Bitrise's OIDC isn't used by this ADR. The new Worker secrets, put by the owner with `pigeon cobot workers secrets put`:

  | Secret | What |
  |---|---|
  | `BITRISE_TOKEN` | a Workspace API token of a workspace holding only this app |
  | `BITRISE_APP_SLUG` | the app's slug, kept out of the tree as an account identifier |

  `BUILDKITE_TOKEN` already reaches every pipeline of `willow-graysen`; its scopes don't change.
- **Logs are public on a public Bitrise app.** They hold the build's output and nothing else, as GitHub's do.
- **Approval tiers.** `tooling/ci/bitrise/**` and `ios-build.yml` hold no secret and only build, so `.config/approvals.yaml` doesn't guard them, as the runners ADR §8 decided for `tooling/ci/**`. `ios-sign.sh` stays at the `secrets` tier, and its `DTXcode` check is approved there.

### 5. Cost and limits

**Expected minutes each month.** These are estimated from 2026-11-12's runs: an `ios` check takes about 3 minutes on a runner. The archive and sign times aren't measured yet, and §8 step 9 replaces these figures with measured ones.

| Job | Count a month | Minutes each | Total |
|---|---|---|---|
| `ios` checks | about 90 | 3 | about 270 |
| releases' archives | about 30 | about 5 | about 150 |
| releases' `sign` (Buildkite only) | about 30 | about 4 | about 120 |

| Provider | Expected minutes | Cap (`.config/macos.yaml`) | Cost |
|---|---|---|---|
| GitHub | about 420 | none | $0: macOS runners are free for a public repository |
| Bitrise | a few tens; at most 420 in a month GitHub fails throughout | 450, under Hobby's 500 | $0 on Hobby with Pay-As-You-Go off |
| Buildkite, build | 0 unless both fail | 150 | at most $18 |
| Buildkite, sign | about 120 | none: signing is outside failover | about $14.40 |

- **Keep Pay-As-You-Go off on Bitrise.** The 450 cap keeps every build within the free 500. Bitrise refusing at its limit is a `refused` outcome, and the job moves to Buildkite.
- **A provider at its cap** takes no new macOS jobs until the month turns, by `macos_jobs`' minutes. Running work finishes.
- **`.config/insights.yaml` gains `bitrise_macos_minute: 0`,** the Hobby rate with Pay-As-You-Go off. If the owner turns Pay-As-You-Go on, it becomes 0.0108 in a reviewed change.

**What the owner sees:**

- **Insights** gain the source **Bitrise**:
  - its cost is its `macos_jobs` minutes at `bitrise_macos_minute`;
  - its errors are its `unassigned`, `lost` and `refused` outcomes.

  GitHub's and Buildkite's errors gain the same three kinds. Buildkite's cost gains `pigeon-ios-build`'s minutes beside `sign`'s.
- **The hourly digest** names each moved job (from where, to where, why), each provider marked down, each config mismatch, and each provider past 80% of its cap.
- **`ios.status`** and **`change_status`** show the provider, attempt and link of each macOS job.
- **`deploy_status`** shows each provider's health and its minutes this month against its cap.

### 6. What this does not decide

- Redundancy for signing, or for GitHub's git (§3, §4).
- Linux code checks. They keep the code-checks ADR's `waiting: GitHub` and the CI window.
- Self-hosted Macs.

### 7. Verification

- **`backplane::macos::outcome`'s tests,** with recorded answers:
  - 2026-11-12's two jobs are `unassigned`;
  - !300's rerun is a pass that `ran`;
  - build 5's archive is a failure that `ran`, and doesn't move;
  - Bitrise builds with status 0 and no worker after 10 minutes, and status 3 with and without one;
  - Buildkite jobs that are `expired` and agent-lost.
- **`next`'s tests:** it follows the order, skips down and capped providers and tried ones, and waits when none is left.
- **The health tests:** two moves in 30 minutes mean down, then the probe, then the doubling.
- **Config:** a Bitrise app whose `bitrise.yml` is in the repository, or differs from `main`'s, is skipped with the reason.
- **`ci.sh`** stops on a wrong `HEAD`, and on an Xcode that doesn't match.
- **`ios-sign.sh`** stops on an archive whose `DTXcode` doesn't match.
- **The drill:**
  - `ci_rerun on: bitrise` and `on: buildkite` pass a change's `ios` check;
  - `ios.release on: bitrise` delivers a build to TestFlight;
  - with GitHub's dispatch refused in a test, a release's archive is built on Bitrise with no one stepping in.
- No secret, token, slug or account ID is in the tree, a log, or a change row.

### 8. Execution

Each step is its own change, behind the one before.

1. **This ADR.**
2. **The owner, once:**
   1. Check §Context's Bitrise facts, and correct this ADR by amendment where they differ: Hobby's concurrency, a stack with Xcode 26.1, the M2 Pro Medium's fit for an archive in 6 GB, and the artifact download.
   2. Create a Bitrise workspace and one public app on `noisypigeon/pigeon`, with no webhooks or triggers, `bitrise.yml` stored on bitrise.io, and Pay-As-You-Go off. Create a Workspace API token.
   3. Create Buildkite's hosted queue `macos-build`, with the Xcode of `.xcode-version`, and the pipeline `pigeon-ios-build`, with no triggers and rebuilds disabled.
3. **`feat(backplane)`: the secret names.** `worker.yaml` gains `BITRISE_TOKEN` and `BITRISE_APP_SLUG` (owner tier, since it lengthens a secret list). The owner then puts them.
4. **`ci(ios)`: one job everywhere** (§3).
   - `src/pigeon-ios/ci.sh`, with `check.sh` and `archive.sh` losing their own Xcode selection.
   - `ci.yml`'s macOS steps call `ci.sh`.
   - `tooling/ci/bitrise/bitrise.yml`, `tooling/ci/buildkite/ios-build.yml`, and `ios-sign.sh`'s `DTXcode` check.
   - The owner then pastes both configs, and builds each once by hand on `main`'s head.
5. **`feat(policies)`:** `.config/policies/macos.yaml` for `.config/macos.yaml`'s shape, and `insights.yaml`'s policy gains `bitrise_macos_minute`. This is a sentinel change, alone.
6. **`feat(backplane)`: the core,** `backplane::macos`: `.config/macos.yaml`, `outcome`, `next`, health and caps, with §7's tests. Nothing calls it yet.
7. **`feat(backplane)`: the clients and the record.**
   - `worker/bitrise.rs`: trigger, read, abort, artifacts and config.
   - `worker/buildkite.rs` gains `pigeon-ios-build`, job artifacts and pipeline steps.
   - `0006_macos_jobs.sql` and the `ios_releases` columns, and health on `Instance`.
8. **`feat(backplane)`: failover.**
   - The release's archive in `MainRun`, and `ios.release`'s `on`.
   - The `ios` check in `ChangeRun`, the CI window's routing, and `ci_rerun`'s new `on` values.
   - The `ci.moved` and `macos.provider` events.
9. **`feat(backplane)`: what the owner sees** (§5): the Bitrise source and the new error kinds in insights, the digest's lines, and `deploy_status`. This ADR's minutes are replaced with a month of measured ones, by amendment.
10. **The owner: the drill** of §7.

## Amendments

### 2026-11-13: Steps 5 to 9 as built: the order, one change for the Worker, and what waits

The owner asked for the feature to be built end to end, then tested and tuned together. What differs from §8:

- **Step 6 came before step 5.** Step 5's policy needs `.config/macos.yaml`, which step 6 adds, and its target is optional, so either order holds. Step 5 also bumps the sentinel's count of policies and tests the new one (!321, replacing !317, whose scope missed the test).
- **`insights.yaml` doesn't gain `bitrise_macos_minute` yet.** Its policy's `fields` rule is `only` and in order, so neither the file nor the policy can change first. And a sentinel change holds only policies, so they can't change together. Until the owner decides how to pair them, Bitrise's cost rows are left out (`insights::rate_of` is none), which matches its $0 with Pay-As-You-Go off.
- **Steps 7, 8 and 9 are one change.** The clients have no caller until failover exists, and `clippy -D warnings` refuses dead code. That change holds:
  - `core::macos::job`: a job's attempts, `settle` and `place`, and the `macos_jobs` rows;
  - `worker/bitrise.rs`, and `pigeon-ios-build` in `worker/buildkite.rs`;
  - `worker/macos.rs`: health on `Instance`, `drive`, `abandon` and the archive's download;
  - `0006_macos_jobs.sql`;
  - `MainRun`'s archive across providers;
  - `ChangeRun`'s `ios` check as a macOS job (`worker/change/macos_check.rs`), and `Actions::drop_check`;
  - the digest's `macOS:` lines;
  - `deploy_status`'s `macos`;
  - insights' Bitrise source, and each provider's `No machine`, `Lost machine` and `Refused` errors.
- **The `ios` check moves only when the head is mirrored.** Every provider clones from the GitHub mirror (§3). Without a mirror, it runs on a Container as before.
- **Later changes:**
  - **The owner's overrides.** These are `ci_rerun on: github|bitrise|buildkite` and `ios.release on`, owner tier through the approval code. Until then, the CI window sends `ios` past GitHub, and a reviewed change to `order` sends archives elsewhere.
  - **The app's Bitrise source.** It needs a colour, which `Theme.swift` checks for colour-blind separation.
  - **Measured minutes** (step 9's last item), after a month.
