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

## Amendments

### 2026-11-02: A bot user, a recoverable range, and publication from tags

The first `release` run, after #11, failed to push. The `main` ruleset requires three status checks, and a push made with `GITHUB_TOKEN` cannot bypass that. A review of #11 also found four defects:

- **New tier files are never committed.** `git commit -a` skips untracked files, so the first release would have dropped four generated changelogs.
- **A rejected push loses entries.** The range came from the push event's `before`. If another merge lands while the bot runs, the bot's push is rejected and the next run's range starts after the missed merge.
- **A failed publish strands its release.** Tags were pushed before the GitHub Releases and images were made. If a later step failed, a rerun found the tags already present and had nothing left to publish.
- **Non-ASCII paths are missed.** `git diff-tree` quotes them, so a change to one did not match its unit's prefix.

#### 1. The bot user

- The bot pushes as `istx25` (`6799989+istx25@users.noreply.github.com`), as runway's bot does, with a `BOT_GITHUB_TOKEN` Actions secret.
  - For a repository owned by a personal account, this is a classic PAT with the `repo` scope.
  - For an organization, it is a fine-grained PAT with Contents read and write.
- `istx25` must be able to bypass the `main` ruleset. That is a settings change outside this repository.
- Images still push with the job's own `GITHUB_TOKEN` and `packages: write`.
- A push with a PAT does trigger workflows. `release.yml` still skips `chore(release):` head commits, and `rust` and `lifecycle` simply run on the bot's commit.

#### 2. The range comes from `main`, not the event

- `--from` defaults to the newest first-parent commit whose subject starts with `chore(release):`. With no such commit, it covers all of history.
- The workflow no longer passes the event's `before`.
- Each run first fetches and checks out the current `origin/main`.
- If the push is rejected, it resets to the new `origin/main`, regenerates and retries, up to three times.
- Date tiers already skip lines that are present, so a wide range is safe.

#### 3. The bot stages exactly what it wrote

- The bot runs `git add` on the paths in `written`, plus `src/pigeon-cli/Cargo.lock`, then commits from the index.
- It decides whether anything changed with `git diff --cached --quiet`, so a run that only creates new files still commits.

#### 4. Publication follows tags, and the GitHub Release marks it done

- `release` pushes the commit and tags, and does nothing else.
- A new `pigeon sentinel units` prints each unit's `unit`, `image`, `features`, newest `tag` and `version` as JSON.
- A new `pending` job selects the units whose newest tag has no GitHub Release yet.
- `image` runs once per pending unit:
  1. checks out the tag;
  2. builds and pushes the image;
  3. creates the GitHub Release last, with notes taken from that version's section of the unit's `CHANGELOG.md`.
- A failure anywhere leaves the release pending, and the next run of `release.yml`, or a rerun, finishes it.
- `--notes-dir` is no longer used by the workflow.

#### 5. Paths are read NUL-delimited

- `git diff-tree` runs with `-z`, and its output is split on NUL. Paths reach `touches` exactly as committed.
- A test commits a non-ASCII path inside a unit and expects that unit to release.

#### 6. Execution

1. This amendment lands first.
2. Then `fix(sentinel): recoverable release bot`, holding steps 1–5. Once the `BOT_GITHUB_TOKEN` secret and the ruleset bypass exist, its merge cuts `sentinel/v0.1.0` and `gantry/v0.1.0`.
3. Step 9's PR follows, as before.

### 2026-11-02: Publish every unpublished tag, latest only for the newest

A review of #13 found that publication still strands releases. `pigeon sentinel units` reports only each unit's newest tag, so `pending` sees only that one. Suppose `sentinel/v0.1.1` is tagged and its image fails to build. Then `v0.1.2` is tagged and published before anyone notices. `v0.1.1` is never picked up again.

Publishing every unpublished tag fixes that, but publishing an old version late must not move `:latest`, or GitHub's "Latest release", backwards.

This supersedes `units` in step 4 of the previous amendment. That text stays as written.

#### 1. `pigeon sentinel tags`

- It replaces `pigeon sentinel units`, whose only caller is `release.yml`.
- It prints one JSON row per `<unit>/vX.Y.Z` tag of each configured unit, with `unit`, `image`, `features`, `tag`, `version` and `newest`.
- `newest` is true only for each unit's highest version, ordered by version (`git tag --sort=-v:refname`), not as strings.
- A unit with no tags has no rows.

#### 2. `pending`

It selects every row whose tag has no GitHub Release, not only each unit's newest.

#### 3. `image`, per pending row

1. Push `:<version>` always.
2. Push `:latest` only when `newest` is true.
3. Create the GitHub Release with `--latest=<newest>`.

So a late-published old version never becomes the latest image or release.

#### 4. Tests

`tags` is tested against a temporary repository tagged `v0.1.0`, `v0.1.1` and `v0.1.10`. All three are listed, and only `v0.1.10` is newest.

#### 5. Execution

1. This amendment lands first.
2. Then `fix(sentinel): publish every unpublished release tag`, holding steps 1–4.

### 2026-11-02: A unit releases only when its own build changed

Every unit's `paths` in §3 include `crates/pigeon/`, `Cargo.toml` and `Cargo.lock`. Almost every Rust change touches one of them, so every unit releases together. #20, #21 and #23 were cobot changes, and each cut new `sentinel` and `gantry` versions whose changelogs list nothing but cobot PRs. §4 makes it worse on a first release, which lists all of history: `cobot/v0.1.0` lists the sentinel PRs that preceded it.

A unit's version should move when, and only when, its image or binary could differ. The entries already written stay; they are generated history. This applies from the next release.

#### 1. The predicate

A unit releases when, since its last tag, any of these changed:

1. **Its own paths, minus `exclude`.**
   - `paths` narrows to the unit's crate, its own slice of `pigeon` (`crates/pigeon/src/commands/<unit>.rs`, `crates/pigeon/src/cli/<unit>.rs`), and, for an imaged unit, `Dockerfile` and `.dockerignore`.
   - A new `exclude` list removes what never ships: `tests/`, `benches/` and `*.md`, under any listed path.
2. **`pigeon`'s shared code.** `crates/pigeon/Cargo.toml` and the files every feature build compiles, `src/main.rs`, `src/lib.rs`, `src/cli/mod.rs` and `src/commands/mod.rs`, stay in every unit's `paths`.
3. **Its resolved dependencies.**
   - sentinel runs `cargo tree --locked -p pigeon --no-default-features --features <unit> -e normal,build,features --prefix none` at the last tag and at the new head, and compares the sorted, de-duplicated rows.
   - Both revisions are read the same way, from `git archive <rev> src/pigeon-cli` unpacked into a temporary directory, never from the checkout.
   - `features` edges make each activated feature a row of its own, so enabling a feature that adds no crate still changes the tree.
   - An added, removed or re-versioned crate, or a changed feature set, releases the unit. `Cargo.lock` and `[workspace.dependencies]` count only through this, so adding a dependency to one crate releases only the units that build it.
   - Workspace crates are compared by name alone. Their `(path)` differs between the two temporary directories and their version moves with every release, so neither is compared.
   - Their paths, relative to the archive, are kept: each one's directory, except `pigeon`'s, joins the unit's paths for this run, so a change to a library a unit depends on releases it even though its version did not move.
   - A release caused by dependencies alone lists the first commit in the range that changed `Cargo.lock` or a `Cargo.toml`, since a feature change may leave `Cargo.lock` alone.
4. **Workspace-wide build settings.** Any line of `src/pigeon-cli/Cargo.toml`'s `[profile.*]` tables, or `[workspace.package]`'s `edition` and `rust-version`, or the `rust` line of `mise.toml`, releases every unit. They are compared as text, with no TOML parser added.

#### 2. `cli.rs` is split by feature

- Today's feature-gated items are interleaved in `crates/pigeon/src/cli.rs`, so a cobot flag would still touch every unit.
- It becomes `cli/mod.rs` (the root command and anything ungated), `cli/sentinel.rs` and `cli/cobot.rs`, each gated once on its `mod` line. Gantry has no CLI items of its own today and gets a file only when it does.
- The move is pure: no behaviour or interface changes.

#### 3. Changelog lines

- A release's lines are only the commits that satisfied the predicate.
- A release caused by dependencies alone lists that commit, suffixed ` (dependencies)`.
- A release caused by build settings alone lists the commit that changed them.
- **A first release lists only the commits that touched the unit's own crate.** This replaces §4's "all of history", and its "a release caused only by shared paths lists those shared changes".
- Date tiers are unchanged.

#### 4. `.config/releases.yaml`

- Each unit gains `exclude: [tests/, benches/, '*.md']` and its narrowed `paths`.
- `Cargo.toml` and `Cargo.lock` leave every unit's `paths`; §1.3 and §1.4 cover them.

#### 5. Tests

Against temporary repositories with a small path-only workspace, so they run offline:

- a change to one unit's command file releases that unit only;
- a `tests/`-only or `README.md`-only change releases nothing;
- a dependency added to one crate releases only the units that build it;
- a `[profile.release]` change releases every unit;
- a first release lists only commits touching that unit's crate.

#### 6. A one-time correction of the entries already written

The entries the old rule wrote are corrected once, by the bot, so no pull request edits a changelog and `pull-request.yaml` keeps its rule.

- A hand-written `.config/changelog-corrections.yaml` lists, per unit and version, the PR numbers whose lines do not belong there. A PR belongs when it changed the unit's own crate, or its image inputs for an imaged unit:

  | Unit | Version | Remove |
  | --- | --- | --- |
  | sentinel | v0.1.3 | #20 |
  | sentinel | v0.1.4 | #21 |
  | gantry | v0.1.0 | #9, #13 |
  | gantry | v0.1.1 | #16 |
  | gantry | v0.1.2 | #20 |
  | gantry | v0.1.3 | #23 |
  | cobot | v0.1.0 | #9, #11, #13, #16 |

- `pigeon sentinel release` applies it on its next run: it drops each listed line from that version's section and reports the file in `written`, so the bot commits it with the rest.
- A section left empty keeps its heading, because the tag and GitHub Release exist. Its one line becomes `- No changes of its own; released by a shared-path change in <subject> ([#N](…))`, naming the removed PR. That line satisfies `release-changelog.yaml` as it stands.
- Applying it is idempotent: a line already gone is skipped, so later runs change nothing.
- The GitHub Releases' notes are not edited; they are a record of what was published.
- The file and the code that reads it are deleted by the next PR that changes `.config/releases.yaml`, once the bot's commit shows the corrected files.

#### 7. Execution

1. This amendment lands first, with the cargo-workspace amendment of the same date.
2. Then `fix(sentinel): release a unit only when its build changed`, holding steps 1–6 and that amendment's CI change.
3. Its own merge still bumps every unit, because splitting `cli.rs` changes `pigeon`'s shared code. The next cobot-only change is the first to show the new rule: it should bump `cobot` alone.

### 2026-11-02: The release machinery moves to cobot

[`2026-11-02-development-sentinel-policy-cobot-actions.md`](2026-11-02-development-sentinel-policy-cobot-actions.md) decides that sentinel is policy and cobot performs the actions policies govern. Releasing is an action, so:

- `release.rs`, `changelog.rs` and `git.rs` move to `cobot::release`. Their behaviour is unchanged.
- `pigeon sentinel release` and `pigeon sentinel tags`, named in §4, §5 and the amendments above, become `pigeon cobot release` and `pigeon cobot tags`, with the same flags and JSON.
- `release.yml` calls the cobot subcommands.
- Wherever this ADR says sentinel generates, bumps or tags, read cobot. Sentinel still enforces the changelog policies of §6 and §7.

The steps are in that ADR's implementation plan.
