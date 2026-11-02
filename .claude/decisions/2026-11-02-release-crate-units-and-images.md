## Independently released crates, images and changelogs

* Date: 2026-11-02
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: release

## Context

`src/pigeon-cli` holds the `pigeon` binary and the `sentinel` library. It is about to gain a third crate, `gantry`. Until now the workspace has shipped nothing: every crate shares one version, nothing is tagged, and the only artifact is a binary built in CI.

This ADR decides how the crates are released. Five things follow from it:

- **`gantry` exists.** It is a stub, and a later amendment decides its purpose.
- **The binary is feature-selected.** `pigeon` compiles in only the subcommands asked for, so an image built for one subcommand carries nothing of another.
- **Each subcommand is a release unit with its own version.** A release is cut only when the unit's files change. The bot bumps versions on `main`, because a release is an outcome of merging, not something a PR author should have to remember.
- **Each unit ships a container image to GHCR.** The workflows then run `pigeon sentinel` from its latest image rather than compiling it on every run.
- **Changelogs become machinery.** The lifecycle ADR's 2026-11-02 amendment had each PR hand-write its own `CHANGELOG.md` line in a follow-up commit, which is prose a model generates and a check merely validates. Instead, sentinel generates every line from the merged commits, in five tiers: one for the repository, one for `src/pigeon-cli`, and one for each crate. Sentinel also blocks hand edits.

This supersedes the per-PR changelog rule in that amendment, and the `contains CHANGELOG` rule in `.config/policies/pull-request.yaml`. Both texts stay as written.

**The release-first loop.** CI runs the latest released sentinel image. So a policy that needs a sentinel feature that is not yet released cannot land in the same PR as that feature. The engine change merges first, the bot releases it, and a later PR uses it. `rust.yml` keeps testing each PR's own sentinel source.

Out of scope, for later amendments or ADRs:

- `gantry`'s purpose;
- releasing `pigeon` itself;
- multi-architecture images, signing and provenance attestation;
- publishing to crates.io;
- `main` branch rules. If PRs become required there, GitHub Actions needs a bypass so the bot's release commits can land.

## Implementation plan.

### 1. `gantry`

- `src/pigeon-cli/crates/gantry/` is a library crate.
  - It exposes `pub const VERSION: &str = env!("CARGO_PKG_VERSION")`.
  - Its crate doc says that a later amendment decides its purpose.
- `pigeon gantry` prints `gantry <version>: not implemented yet` and exits 0.

### 2. A feature-selected binary

- In `crates/pigeon/Cargo.toml`:
  - `sentinel` and `gantry` are optional dependencies;
  - `[features]` declares `sentinel = ["dep:sentinel"]`, `gantry = ["dep:gantry"]` and `default = ["sentinel", "gantry"]`.
- Each subcommand's `cli.rs` variant, `commands/` module and integration tests are behind `#[cfg(feature = "<name>")]`.
- Each subcommand reports its own crate's version, so `pigeon sentinel --version` prints sentinel's.
- A one-subcommand build is `cargo build -p pigeon --no-default-features --features <name>`.
- A new Mise task, `features`, runs clippy with each feature alone and with none. `rust` depends on it.

### 3. Release units and tiers: `.config/releases.yaml`

Sentinel reads this file and validates it the way it validates a policy: unknown keys and invalid values fail with exit `2`.

```yaml
description: Release units and changelog tiers; only the release bot writes versions and changelogs.
repository: https://github.com/noisypigeon/pigeon
units:
  sentinel:
    manifest: src/pigeon-cli/crates/sentinel/Cargo.toml
    features: sentinel
    image: ghcr.io/noisypigeon/pigeon-sentinel
    paths: [src/pigeon-cli/crates/sentinel/, <shared>]
  gantry:
    manifest: src/pigeon-cli/crates/gantry/Cargo.toml
    features: gantry
    image: ghcr.io/noisypigeon/pigeon-gantry
    paths: [src/pigeon-cli/crates/gantry/, <shared>]
tiers:
  - { file: CHANGELOG.md, paths: [''] }
  - { file: src/pigeon-cli/CHANGELOG.md, paths: [src/pigeon-cli/] }
  - { file: src/pigeon-cli/crates/pigeon/CHANGELOG.md, paths: [src/pigeon-cli/crates/pigeon/] }
  - { file: src/pigeon-cli/crates/sentinel/CHANGELOG.md, unit: sentinel }
  - { file: src/pigeon-cli/crates/gantry/CHANGELOG.md, unit: gantry }
```

- `<shared>` stands for the inputs every image is built from:
  - `src/pigeon-cli/crates/pigeon/`;
  - `src/pigeon-cli/Cargo.toml` and `src/pigeon-cli/Cargo.lock`;
  - `src/pigeon-cli/Dockerfile` and `src/pigeon-cli/.dockerignore`.

  A change to any of them releases both units.
- `sentinel` and `gantry` each set their own `version = "0.1.0"` instead of the workspace's.
- `pigeon` keeps the workspace version and is not a release unit.
- A tier with `paths` is sectioned by UTC date, `## YYYY-MM-DD`, newest first. An empty path matches everything.
- A tier with `unit` is sectioned by release, `## vX.Y.Z (YYYY-MM-DD)`, newest first.

### 4. `pigeon sentinel release`

```
pigeon sentinel release [--root DIR] [--config FILE] --from SHA --to SHA [--notes-dir DIR] [--dry-run]
```

- **Commits.** The first-parent commits in `from..to`, oldest first. A `--from` of zeroes, or an unknown one, means all of history.
  - Commits whose subject starts with `chore(release):` are skipped.
  - From each commit come:
    - its subject. `title (#N)` becomes `- title ([#N](<repository>/pull/N))`, and any other subject becomes `- subject ([<sha7>](<repository>/commit/<sha>))`;
    - its UTC commit date;
    - its changed paths, from `git diff-tree --no-commit-id --name-only -r`.
- **Units.** A unit releases when:
  - any commit since its last `<unit>/v*` tag touched its paths, in which case the patch in `manifest` is bumped; or
  - it has no tag yet, in which case its current version ships as is.
- **Date tiers.**
  - Each line goes under its commit's date heading, and a missing heading is created in descending order.
  - A line whose link is already in the file is skipped, so a rerun changes nothing.
  - A missing tier file is created as `# Changelog`, backfilled from all of history.
- **Release tiers.**
  - Each released version prepends `## vX.Y.Z (YYYY-MM-DD)`, dated by the last commit in the range. Its lines are the commits since the unit's last tag that touched the unit's paths; for a first release, that is all of history.
  - A release caused only by shared paths lists those shared changes, because they are what changed in its image.
- **Notes.** `--notes-dir` receives `<unit>.md`, which is each release's section body.
- **Output.** On stdout, `{"released":[{"unit":..,"version":..,"tag":..,"image":..,"features":..}]}`.
- **Dry run.** `--dry-run` prints the same output and writes nothing.
- **Code.** Git runs through `std::process::Command`, in a new `git.rs`. The release logic lives in `release.rs`, and changelog editing in `changelog.rs`. `serde_json` joins the workspace dependencies, at its newest release that is at least two weeks old.

### 5. The release bot: `.github/workflows/release.yml`

- **Trigger:** `push` to `main`.
  - Skipped when the head commit starts with `chore(release):`.
  - `concurrency: { group: release, cancel-in-progress: false }`.
- **Job `release`** (`contents: write`):
  1. checkout with `fetch-depth: 0`, then Mise;
  2. `pigeon sentinel release --from "$BEFORE" --to "$AFTER" --notes-dir "$RUNNER_TEMP/notes"`, with both SHAs passed through `env:`;
  3. `cargo update -w --offline`, so `Cargo.lock` records the bumped versions;
  4. if anything changed, a single commit by `github-actions[bot]`, titled `chore(release): <unit> vX.Y.Z, ...`, or `chore(release): changelogs` when nothing released;
  5. one tag `<unit>/vX.Y.Z` per released unit;
  6. `git push --atomic origin HEAD:main <tags>`. A push made with `GITHUB_TOKEN` triggers no further workflow runs, by design;
  7. `gh release create <tag> --title <tag> --notes-file <notes>`.
  - Its output is the matrix of released units.
- **Job `image`** (`needs: release`, a matrix over released units, `contents: read`, `packages: write`):
  1. checkout of the tag;
  2. `docker login ghcr.io` with the password read from stdin;
  3. `docker build --build-arg FEATURES=<features> -t <image>:<version> -t <image>:latest src/pigeon-cli`;
  4. push both tags.
  - It uses the docker CLI only, so there are no third-party actions to pin.

### 6. Hand edits are blocked

- Pull-request fields gain `files`, read from `PR_FILES`. The workflow computes it as `git diff --name-only <base>...<head>`, one path per line.
- The `field` rule gains a generic `not` option: the field must not match.
- `.config/policies/pull-request.yaml`:
  - drops `contains`;
  - adds `field: { name: files, not: '(?m)(^|/)CHANGELOG\.md$' }`.

  Only the bot writes changelogs.
- The release config itself is a hand-written file, and stays editable.

### 7. Changelog policies

- A new generic target, `{ files: [PATH, ...], optional: true }`, runs a policy's rules against each listed file. With `optional`, a missing file is skipped.
- `.config/policies/changelog.yaml` targets the three date tiers. It keeps its rules, except that `unique: pr` applies within each file.
- A new `.config/policies/release-changelog.yaml` targets the two release tiers:
  - line 1 is `# Changelog`;
  - level-2 headings match `^v(?<v>[0-9]+\.[0-9]+\.[0-9]+) \((?<day>[0-9]{4}-[0-9]{2}-[0-9]{2})\)$`, with `day` descending (not strict) and `v` unique;
  - each section holds linked bullets in the same form as the date tiers.

### 8. Docker: `src/pigeon-cli/Dockerfile` and `.dockerignore`

```dockerfile
FROM docker.io/library/rust:1.99.1-slim-bookworm@sha256:<digest> AS build
ARG FEATURES
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p pigeon --no-default-features --features "$FEATURES"

FROM gcr.io/distroless/cc-debian12:nonroot@sha256:<digest>
COPY --from=build /src/target/release/pigeon /usr/local/bin/pigeon
ENTRYPOINT ["/usr/local/bin/pigeon"]
```

- Both bases are pinned by digest, each at least two weeks old. The Rust version equals the Mise pin.
- `.dockerignore` excludes `target/`.
- `rust.yml` gains a job, `image`, that builds the `sentinel` and `gantry` images on every PR without pushing them.

### 9. Workflows run the sentinel image

This lands in its own PR, after the first release exists.

- In `.github/workflows/lifecycle.yml`, the `sentinel` and `sentinel-pr` jobs (`contents: read`, `packages: read`):
  1. check out;
  2. `docker login ghcr.io`;
  3. run `docker run --rm -v "$PWD:/repo:ro" -e PR_TITLE -e PR_BODY -e PR_NUMBER -e PR_FILES ghcr.io/noisypigeon/pigeon-sentinel:latest sentinel check --root /repo`, adding `--pull-request` in `sentinel-pr`.
- `sentinel-pr` sets the `PR_*` variables through `env:`. `PR_FILES` comes from a `git diff` over a `fetch-depth: 0` checkout.
- Mise leaves `lifecycle.yml`.
- The local `mise run sentinel` and `rust.yml` keep running sentinel from source.

### 10. Docs

`CLAUDE.md` says:

- changelogs are generated by the release bot, and a PR never edits them;
- the units, the features, `mise run features` and the image names;
- the release-first loop.

### 11. Execution

1. **PR A** holds this ADR. Under the rules it supersedes, it adds its own root changelog line by hand, for the last time.
2. **PR B**, titled `feat(sentinel): release units, generated changelogs, gantry and images`, holds steps 1–8 and 10. It adds no changelog line.
   - After merging, the first `release` run:
     - tags `sentinel/v0.1.0` and `gantry/v0.1.0`;
     - creates both GitHub Releases and pushes both images;
     - creates the four new changelog files, and adds PR B's line to the root.
3. **PR C**, titled `ci: run lifecycle checks from the pigeon-sentinel image`, holds step 9. It touches only `.github/`, so its merge adds a changelog line and releases nothing.

### 12. Verification

- `mise run ci` passes, including `features`.
- A build with only `gantry` has no `sentinel` subcommand.
- `release` is tested against temporary git repositories:
  - a first release, with backfill;
  - no change, which releases nothing;
  - a change to a shared path, which releases both units;
  - a crate-only change, which releases one unit;
  - an idempotent rerun;
  - a commit without a PR number;
  - section ordering.
- A PR touching any `CHANGELOG.md` fails `sentinel-pr`.
- After PR B merges, the `release` run is green, both tags, releases and images exist, and the bot's commit holds the generated tiers.
- PR C's jobs pass using `pigeon-sentinel:latest`, and its merge releases nothing.
