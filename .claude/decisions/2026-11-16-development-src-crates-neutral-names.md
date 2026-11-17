## One crate per `src/` directory, neutral names, one changelog, and no mise

* Date: 2026-11-16
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development
* Governs: `src/**/Cargo.toml`, `src/**/dev.rs`, `rust-toolchain.toml`, `.config/releases.yaml`, `CHANGELOG.md`

## Context

The owner asked for `src/pigeon-cli` to be split into crates named for what
they do. Today (read at `c836a8a`):

- **`src/pigeon-cli`** is one workspace with five directories under
  `crates/`:
  - `backplane`: about 100k lines with its tests;
  - `pigeon`: the binary;
  - `sentinel`: the policy engine;
  - `cobot` and `gantry`: only a `CHANGELOG.md` each, excluded from the
    workspace.
- **`backplane` is four things in one crate:**
  - `core/`, the functional core. It isn't self-contained: it reaches into
    `push`, `sessions`, `control`, `approve`, `events`, `tasks`,
    `reconcile`, `mirroring`, `actions` and, behind `engines`, `cobot`.
  - About 20 pure top-level modules (`control`, `wire`, `tickets`, `routes`,
    `access`, …) that the Worker and the CLI both use.
  - The engines under `cobot/` (tofu, Workers deploy, sites, git, release,
    scope, CI) and the job server (`jobs`, `serve`, `platform_job`,
    `standup_job`, `ship`), behind `engines`.
  - The control-plane Worker under `worker/`, behind `worker`, built for
    wasm.
- **`pigeon` → `backplane`** is the only link between crates. `sentinel` is
  run as a subprocess (`pigeon-sentinel`), never linked.
- **Names leak everywhere.** `cobot`, `backplane`, `sentinel` and `gantry`
  appear in CLI subcommands, Cargo features, env vars (`COBOT_*`,
  `BACKPLANE_*`, `PIGEON_SENTINEL*`), the ciphertext tag `ENC[cobot:v1:…]`,
  the schema key `x-cobot`, the `.cobot/` directory, images, policies,
  `CLAUDE.md` and 67 ADRs' `Governs` lines.
- **35 changelogs.** The release bot (`pigeon cobot release`) writes 35
  `CHANGELOG.md` files and tags `<unit>/vX.Y.Z`. `cobot` and `gantry` are
  orphaned tiers with no unit.
- **mise** pins the tools and defines the tasks (`mise.toml`). The
  bench-containers ADR
  ([`2026-11-16-development-session-bench-containers.md`](2026-11-16-development-session-bench-containers.md))
  resolves `mise run <task>` from it, because its image has no mise.
- **The tests** are being moved to one binary per crate
  ([`2026-11-16-development-one-test-binary-per-crate.md`](2026-11-16-development-one-test-binary-per-crate.md)).
  Its ledger, `src/pigeon-cli/tests.ledger`, is keyed by package and target
  name. Its step 6 waits for this ADR's names.

**The owner's decisions:**

- **Owner item 3:** today's sentinel becomes `src/pigeon-eviction`, lib
  `eviction`, CLI `pigeon eviction check`.
- **Owner item 4:** every `CHANGELOG.md` merges into the root one, catalog
  and infra included.
- **Owner item 5:** the site templates become one crate that embeds and
  validates them, and `pigeon-ios` gets a thin check crate.
- **Backfill scope** (the owner, through the status session): the root
  changelog's sections start at 2026-11-06, with no tags made after the
  fact, and the bot's first run cuts one catch-up release per unit.

## Implementation plan.

### 1. The layout

The workspace root moves to the repository root: `/Cargo.toml` with
`members = ["src/*"]`, and `/Cargo.lock`. Every `src/*` directory is a
crate.

| Directory | Package | Lib | What it is | Depends on |
|---|---|---|---|---|
| `src/common` | `pigeon-common` | `common` | Encoding (base64, hex), HTML escaping and time helpers. No domain types, no I/O. | nothing |
| `src/pigeon-core` | `pigeon-core` | `pigeon_core` | The control plane's pure logic and wire form. No features, no I/O. Builds for native and `wasm32-unknown-unknown`. | common |
| `src/pigeon-eviction` | `pigeon-eviction` | `eviction` | Today's sentinel, renamed, with a bin `pigeon-eviction`. | nothing |
| `src/pigeon-agent` | `pigeon-agent` | `pigeon_agent` | Everything that does the platform's work. With `engines`: the job server, infra (tofu), Workers deploy, sites, git, release and checks. With `worker`: the control-plane Worker (cdylib, wasm). | common, core |
| `src/pigeon-harness` | `pigeon-harness` | `pigeon_harness` | The `pigeon` binary (bin name kept), which sessions and the owner use: its CLI, `pigeon session`, `pigeon dev`. | common, core, agent |
| `src/pigeon-site-templates` | `pigeon-site-templates` | `site_templates` | `architecture/` and `blog/` as data, embedded with `include_dir`; tests validate each `template.yaml` and build each with Zola. | nothing |
| `src/pigeon-ios` | `pigeon-ios` | `pigeon_ios` | The Swift app, unchanged in place. The crate's tests check `project.yml`, the `Info.plist` keys, and that `PigeonCore`'s wire models name only operations in core's `wire::CATALOG`. It never builds Swift; macOS jobs do. | core (dev) |

- **Direction:** common ← core ← agent ← harness. eviction, site-templates
  and ios link none of the others, except ios's dev-dependency on core.
  agent's `worker` feature uses only common and core.
- **`src/pigeon-cli` is gone at the end**, `crates/cobot` and `crates/gantry`
  included.

### 2. Where each module goes

Moves use `git mv`, one crate per change, so history follows. **A file keeps
its file name**, so a `src/**/<file>` glob matches it both before and after
(§6).

Files whose names carry an old word keep them until step 12, which renames
them and tightens the globs that name them in one planner change:
`sentinel.rs`, and the harness's `cli/{backplane,cobot,sentinel}.rs` and
`commands/{backplane,cobot,sentinel,workers}.rs`.

**pigeon-core** (`src/pigeon-core/src/`):

- `backplane/src/core/<m>` becomes `<m>` (the `core/` level goes): account,
  app, approval, bench, brief, change, connect, deploy, infra, insights,
  learner, `ledger.rs`, macos, member, `minting.rs`, owner, platform, pool,
  release, session and sites.
- These pure top-level modules keep their names: access, actions, apple,
  approve, briefs, ci_window, control, decisions, events, gitcreds, gitdoor,
  iosrelease, mcp, mirroring, push, reconcile, routes, sessions, signing,
  tasks, tickets and wire.
- `architecture/{mod,layout,render}.rs`.
- `cobot/{link,ship_wire,artifacts_wire}.rs` go to `wire/`.
- The cobot types that `core/infra`, `core/sites`, `core/deploy/scope`,
  `core/release` and `core/ledger` name (`UnitPlan`, the static-site, apex,
  catalog, generations, workers, release and config types) move into core
  beside their users. Their I/O stays in agent. `ledger.rs`'s file read moves
  to its caller. **core has no features**: what is `engines`-gated in core
  today is either pure, and loses the gate, or moves to agent.
- The 14 three-line re-export shims (approvals, changes, devices, grants,
  insights, macos, mandates, merges, passkeys, platforms, runs, testmaps,
  totp, versions) are deleted, and callers name the core path.
- `push.rs`'s base64 helpers go to `common`.

**pigeon-agent** (`src/pigeon-agent/src/`):

- `worker/**` stays `worker/**`; `cobot/worker.rs` becomes
  `containers/worker.rs`. The bench files (!667) go to `bench.rs`
  (from `cobot/bench.rs`) and `worker/bench.rs`. The `Bench` class and its
  migration tag `v4` don't change.
- Top level: `jobs.rs`, `serve.rs`, `platform_job.rs`, `standup_job.rs`,
  `ship.rs`, `catalog.rs`, `engine.rs`, `instance.rs` and `sentinel.rs`
  (`eviction.rs` at step 12).
- `infra/`: cobot's tofu, plans, state, s3, secrets, providers, unit,
  manifest, config, moves, adopt, catalog, generations, session and engines.
- `workers/`: cobot's workers, imagebuild, cloudflare and ship.
- `sites/`: cobot's sites, static_site and apex.
- `git/`: cobot's git and artifacts.
- `release/`: cobot's `release/{mod,git}.rs`.
- `checks/`: cobot's ci and scope.
- `architecture/{describe,network,place}.rs`.
- `backplane/migrations/` becomes `src/pigeon-agent/migrations/`.
- Features: `engines` (the default) and `worker`.

**pigeon-harness** (`src/pigeon-harness/`): `crates/pigeon/**`. Its CLI
modules take §3's groups in step 7 and their new file names in step 12.

**pigeon-eviction**: `crates/sentinel/**`, unchanged inside.

**pigeon-site-templates**: `src/site-templates/{architecture,blog}` become
`src/pigeon-site-templates/{architecture,blog}`, with the crate's
`Cargo.toml`, `src/lib.rs` and `tests/`.

### 3. The CLI

| Today | New |
|---|---|
| `pigeon sentinel check` | `pigeon eviction check` (and `pigeon-eviction check`) |
| `pigeon cobot validate\|render\|plan\|apply\|show\|lock\|capabilities` | `pigeon infra …` |
| `pigeon cobot secrets set\|check` | `pigeon infra secrets …` |
| `pigeon cobot sites build\|publish` | `pigeon sites …` |
| `pigeon cobot git …` | `pigeon git …` |
| `pigeon cobot workers …` | `pigeon workers …` |
| `pigeon cobot scope` | `pigeon dev scope` |
| `pigeon cobot release`, `tags` | `pigeon release`, `pigeon release tags` |
| `pigeon backplane instance\|engines\|operations\|run\|decisions` | `pigeon instance`, `pigeon engines`, `pigeon ops`, `pigeon run`, `pigeon decisions` |
| `pigeon backplane serve`, `jobs --listen` | `pigeon agent serve`, `pigeon agent listen` |
| `pigeon backplane totp-secret\|enroll-secret\|vapid-key` | `pigeon keys totp\|enroll\|vapid` |
| `pigeon backplane signing …` | `pigeon signing …` |
| `pigeon session …` | unchanged |
| `mise run <task>` | `pigeon dev <task>` (§4) |

- **Cargo features of `pigeon-harness`**, one per group: `eviction`,
  `infra`, `agent`, `signing`, `session`. They replace `sentinel`, `cobot`
  and `backplane`.
- **The old spellings stay as hidden aliases** until step 12, so
  `CLAUDE.md`, `session.sh`, the Worker's Container commands, CI and the
  bench allowlist keep working at every step. Each prints the new spelling
  on stderr.
- **Operation names** (`policy.check`, `architecture.describe`, …) don't
  change. They name no product.

### 4. No mise

- **The tasks become `pigeon dev <task>`**, defined in Rust in
  `pigeon-harness` (`dev.rs`). The names stay, the dependency graph is the
  same, and the commands run from the repository root:
  - `build`, `test`, `fmt`, `fmt-check`, `lint`, `features`, `rust`;
  - `policy`, `policy-pr` and `policy-change`, which were `sentinel*`;
  - `instance`, which was `backplane`;
  - `ci`, `ci-scoped`, `scope`.
- **`pigeon dev` needs only cargo** and the tools on `PATH`. It installs
  nothing.
- **Tool versions:**
  - Rust's version moves to `/rust-toolchain.toml`, with its components
    and the wasm target. rustup reads it, and so do the image and
    `ios-sign.sh`.
  - Tofu, Zola, Node, wrangler and worker-build keep the versions the
    image's `ARG`s already pin, in `/Dockerfile`.
  - `.config/releases.yaml`'s `settings` names `rust-toolchain.toml` in
    place of `mise.toml`.
- **Benches**, jointly with the containers planner (an amendment to the
  bench ADR):
  - `core::bench::allow` accepts `pigeon dev <task>` beside `mise run <task>`
    from the step that adds `pigeon dev` (step 7) until step 12, then only
    the former.
  - `core::bench::tasks` becomes a list of names, because the binary resolves
    them.
- **`src/pigeon-ios/mise.toml`** goes. `check.sh` and `ci.sh` already call
  `xcodegen` themselves.
- **The cutover** (step 10): `CLAUDE.md`'s "`mise run ci` must pass" becomes
  "`pigeon dev ci`" (on a bench, per the bench ADR), and `/mise.toml` is
  deleted.

### 5. Names outside code

| Today | New | How |
|---|---|---|
| `COBOT_*` env vars (state, secrets and workload passphrases, `STATE_R2_*`, `CREDENTIALS`, `DOCKER`, `WRANGLER`, `PRIVATE_ZONES`) | `PIGEON_*`, same suffix | read new, else old with a warning |
| `BACKPLANE_TOKEN`, `BACKPLANE_TLS_*` | `AGENT_TOKEN`, `AGENT_TLS_*` | same |
| `PIGEON_SENTINEL{,_URL,_SHA256}` | `PIGEON_EVICTION…` | same |
| `ENC[cobot:v1:…]` | `ENC[pigeon:v1:…]` | both read; written new; old values are left until their next `secrets set` |
| schema key `x-cobot` | `x-pigeon` | both read; data rewritten in one step |
| `.cobot/` | `.pigeon/` | both ignored; written new |
| images `ghcr.io/noisypigeon/pigeon-{sentinel,backplane}` | `pigeon-{eviction,agent}` | next release pushes the new names |
| Dockerfile stages `sentinel`, `backplane`; `FEATURES=backplane` | `eviction`, `agent`; `FEATURES=agent` | with the agent step |
| state repository `noisypigeon/pigeon-cobot-state` | `noisypigeon/pigeon-state` | owner's rename on GitHub (step 11), then config |

- **Stored secrets are renamed by the owner, not by an agent:** the Worker's
  `BACKPLANE_TOKEN`, and wherever `COBOT_*` values are set. Step 11 files it
  with `owner_ask`. The fallbacks go only after the owner confirms.
- **`.config/policies/secrets*.yaml` keep pinning the same keys.** Their
  value pattern accepts both tags.
- **ADRs and the changelog are history.** Merged ADR text, ADR filenames and
  past changelog lines keep the old words. A new policy,
  `.config/policies/names.yaml` (step 12), forbids `cobot`, `backplane`,
  `gantry` and `sentinel` (case-insensitive) in every tracked file except
  `.claude/decisions/**` and `CHANGELOG.md`.

### 6. Rules and `Governs` globs move with the files

Executors don't amend ADRs, and guarded files take the owner's passkey. So,
as the one-test-binary ADR did for test paths (its 2026-11-16 amendment),
step 2 makes every path rule match the old and the new place before anything
moves:

- **ADRs.** Each `Governs` glob under `src/pigeon-cli/` or
  `src/site-templates/` becomes `src/**/<tail>`. `<tail>` is the part of the
  path that survives §2's move, at least the file name (for example
  `src/**/change/stage.rs`, `src/**/cobot/tofu.rs` → `src/**/tofu.rs`).
  The workspace files that move to the root in step 4 (`Cargo.toml`,
  `Cargo.lock`, `Dockerfile`, `.dockerignore`, `pigeon-start`) become
  `**<file>`, which matches both places.
  Each ADR gets a 2026-11-16 amendment saying so. Step 12 tightens them to
  the new paths.
- **Policies and `.config/approvals.yaml`.** Their path regexes become
  alternations of the old and the new path (`^src/(pigeon-cli/crates/backplane/src/core|pigeon-core/src)/(change|approval|…)/`).
  `approvals.yaml`'s `sentinel:` key reads as `eviction:` too.
  - This guards the same files as before and loosens nothing.
  - Step 12 drops the old half.
- **The behaviour ledger** moves to `tooling/ci/tests.ledger` and
  `tooling/ci/tests.moves`. With the test-strategy planner's agreement,
  `behaviours.sh` counts a moved line under its new package, so moving
  tests between crates doesn't read as a drop. Each move step writes its
  `tests.moves` lines.

### 7. One changelog, and the release bot

Agreed with the changelog-backfill planner. Releases have been paused since
`196e593` (2026-11-06), and 514 commits are unreleased.

- **One file, `/CHANGELOG.md`,** written only by the release bot:
  ```
  # Changelog

  ## YYYY-MM-DD

  ### Released
  - pigeon-eviction v0.1.17

  - <subject> ([<sha7>](<commit link>)) · <unit>[, <unit>…]
  ```
  - Dates are UTC merge dates, newest first.
  - Within a day, lines are in merge order, one per first-parent commit.
    `chore(release)` commits are skipped.
  - A line is known by its link, so a rerun adds nothing.
- **Links:**
  - a subject ending `(#n)` links the GitHub pull request;
  - every other subject links its commit, keeping `(!n)` in the text.
  - A public, read-only change page is left to a later ADR. Until then, no
    line links to the control plane, which is behind Access.
- **The suffix names the unit as it was named when the line merged.** A tag
  names what shipped, so history isn't relabelled.
  - The rename appears once, as a dated line from the bot:
    `- units renamed · backplane → pigeon-agent, sentinel → pigeon-eviction`.
  - `releases.yaml` records it per unit as `previous: <old unit>`. A
    renamed unit finds its old tags through it: `pigeon-eviction` continues
    `sentinel`'s version, and `pigeon-agent` continues `backplane`'s.
  - Old tags keep their names.
- **`releases.yaml`:**
  - drops `tiers` and gains `changelog: CHANGELOG.md`;
  - `retired` lists every other `CHANGELOG.md`, `cobot`'s and `gantry`'s
    included, and the bot deletes them on its next run.
- **`pigeon release --replay`** renders the whole file from git history: it
  walks first-parent from `196e593` (2026-11-06), and takes releases from
  tags and lines from subjects. Its output is the same bit for bit on every
  run. Earlier history stays in git and in the retired files' last
  versions. No tags are made after the fact. The bot's first run under the
  new config cuts one catch-up release per unit.
- **Release notes** (`--notes-dir`) are each unit's lines since its last
  tag, as today.
- **The rule.** `CLAUDE.md` becomes: "No change edits `CHANGELOG.md`; the
  release bot alone writes it." It is amended in step 3, with the owner's
  passkey (a guarded file).
- **The backfill** writes nothing by hand. It is one change that commits
  `--replay`'s output and the retirements, under the owner's approval, and
  it merges right after step 3.
- **`.config/policies/changelog.yaml` and `release-changelog.yaml`** name
  the per-unit files and GitHub link patterns. Step 3 rewrites them for the
  one file and its link kinds.
- **A release job on Cloudflare.** `MainRun` runs the bot after each deploy,
  so releases never pause again. It is step 3's last part.
- **Who does it.** The changelog-backfill planner plans and reviews step 3
  (the bot's single-file renderer, `--replay`, link kinds, the rename line,
  the policies and the release job) under this ADR. This ADR's planner owns
  the `CLAUDE.md` rule.

### 8. Coordination

- **Benches** (the containers planner): no move touches a file under an open
  change of theirs. Either their change merges first, or they
  `change_yield`. The bench ADR's `mise run` allowlist follows §4.
- **Tests** (the test-strategy planner): their step 6 takes §2's module
  names. Each move keeps `behaviours.sh --check` green through
  `tests.moves`.
- **The changelog backfill** (§7).

### Execution

Each step is its own change, and keeps `main` deployable and every check and
ledger line passing. "(planner)" steps change guarded files or settle a
boundary. The other steps are briefs for Sonnet executors.

1. **This ADR.** (planner)
2. **Paths match both places** (§6): the ADRs' `Governs` amendments, the
   policies and `approvals.yaml`, and the ledger's move and counting rule.
   (planner, owner approves)
3. **Release design** (§7): the bot writes one changelog, with `previous`,
   `retired`, `--replay` and the release job; `releases.yaml` and the
   changelog policies; the `CLAUDE.md` rule. Then the backfill change merges.
   (the changelog-backfill planner, with this one for `CLAUDE.md`; owner
   approves)
4. **Workspace at the root:** `git mv` of `Cargo.toml`, `Cargo.lock`,
   `Dockerfile`, `.dockerignore` and `pigeon-start` to `/`, with members at
   `src/pigeon-cli/crates/*` for now. Callers are updated: `mise.toml`'s
   `dir`s, `ci.yml`, `worker.yaml`, `session.sh`, `ios-sign.sh`,
   `test-map.sh`, `.gitignore`. `crates/cobot` and `crates/gantry` are
   deleted. (brief)
5. **pigeon-eviction:** the move, the package and bin rename, `PIGEON_EVICTION*`
   with fallback, the image name, `ship.rs` and `pigeon-start`, and
   `pigeon eviction` with the old alias. (brief)
6. **common and pigeon-core,** in as many changes as the tangle needs:
   - first, core's upward references and the cobot types it names move
     inward (planner);
   - then the `git mv` (brief).
   Done when core builds alone for native and wasm with no features.
7. **pigeon-agent and pigeon-harness:**
   - the rest of backplane goes to `src/pigeon-agent`, and `crates/pigeon`
     to `src/pigeon-harness`;
   - features, `worker.yaml`'s `entry` and `migrations`, the Dockerfile
     stages and `FEATURES`, and `architecture/network.rs`'s path;
   - §3's CLI with aliases, and `pigeon dev`.
   Ordered after the containers planner's open changes on `jobs.rs`,
   `gitdoor` and `worker/`. (brief per crate; the Worker part needs the
   owner's approval, since it is guarded)
8. **pigeon-site-templates** (brief).
9. **pigeon-ios's check crate** (brief).
10. **Callers switch:**
    - `CLAUDE.md`, `.claude/decisions/README.md`, `tooling/`, CI, the
      Worker's Container commands and the bench allowlist use the new
      spellings;
    - `/rust-toolchain.toml`;
    - `mise.toml` (both) deleted.
    (planner, owner approves)
11. **Names outside code** (§5): `x-pigeon`, `.pigeon/`, `ENC[pigeon:…]`
    readers. The owner is asked (`owner_ask`) to rename the stored secrets
    and the state repository; config follows their answer. (planner)
12. **The old names go:**
    - aliases and fallbacks removed;
    - globs and policy regexes tightened to the new paths;
    - `names.yaml` added;
    - this ADR amended with the outcome.
    (planner, owner approves)

## Amendments

### 2026-11-16: Review of !674 against this ADR: they agree

- [`2026-11-16-development-one-test-binary-per-crate.md`](2026-11-16-development-one-test-binary-per-crate.md)
  step 4 (!674) changes only backplane's own manifest:
  - `autotests = false`;
  - `[[test]] suite`;
  - and the moves of its test files into `tests/suite/`.
  It renames no package, and moves no crate.
- **Moving with each crate.** Each crate's `tests/suite/**` moves with its
  crate, unchanged inside.
  - Backplane's suite modules split between `src/pigeon-core` and
    `src/pigeon-agent` by where their code goes. Each move brief writes
    `tests.moves`.
  - `harness/` goes with each suite.
- **Globs.** The test globs in `Governs` that !674 writes take §6's form,
  `src/**/tests/suite/<file>.rs`. They match both layouts, so the crate moves
  need no further ADR edit for them.
- **The behaviour ledger.**
  - `tests.ledger` and `tests.moves` move to `tooling/ci/` in this ADR's
    step 4. `behaviours.sh` reads the base ledger from either path.
  - From !674, its count check counts each moved test under the package its
    move names.
- **Order.**
  - This ADR's steps 4 and 5 may follow !674.
  - Backplane's split (steps 6 and 7) follows the test ADR's briefs 2b, 5a
    and 5b1 to 5b4, which touch only `backplane/tests/suite/**` and
    `tooling/ci/test-map.sh`.

### 2026-11-16: Reviewed against the bench ADR (!669); they agree

Reviews !669, the bench ADR's step 4, against this ADR and
[`2026-11-16-development-session-bench-containers.md`](2026-11-16-development-session-bench-containers.md).

- **They agree. !669 changes nothing either ADR decides differently.**
  - It touches no `Cargo.toml`. Its files are `gitdoor.rs`, `tickets.rs`,
    `worker/gitdoor.rs`, `worker/tickets.rs`, and backplane's
    `tests/gitdoor.rs` and `tests/tickets.rs`.
  - They move with §2's module map, as any file of theirs does.
  - The scratch ref `refs/heads/scratch/<session_id>` is a ref name, not a
    path, so no move renames it.
- **What the two planners settled, so nothing waits:**
  - Bench files move only after the bench change holding them merges:
    - step 4 (workspace to the root, touching `worker.yaml`) after !675;
    - step 6 (`core/bench`) after `pigeon session bench`, brief b57;
    - step 7 after the bench ADR's step 8.
  - From step 7, a bench runs `pigeon dev <task>` through the commit's own
    harness: `cargo run -q --locked -p pigeon-harness -- dev <task>` from the
    root. It accepts `mise run <task>` beside it until step 12.
  - The bench ADR records this in its own amendment when step 7 merges.
  - The `Bench` class and its migration tag `v4` keep their names.

### 2026-11-16: Step 2 as built, and the changelog's `### Merged` heading

- **Step 2** (this ADR's change after !674):
  - 66 ADRs' `Governs` globs take §6's form, each with an amendment. The
    test globs !674 wrote are already in that form and are left alone.
  - `.config/approvals.yaml` guards each new path beside its old one, so it
    guards the same files and loosens nothing.
  - `backplane.yaml`, `sentinel-actions.yaml` and `layout.yaml` match both
    places. `layout.yaml` also allows the root workspace files and
    `rust-toolchain.toml`, ignores `.pigeon/` and `target/`, and no longer
    requires `src/pigeon-cli`.
  - The ledger's count rule is the test ADR's own, shipped in !674.
- **§7's layout.** Each day's commit lines sit under `### Merged`, below
  `### Released`. Without a heading, a blank line doesn't stop CommonMark
  from joining the two lists into one. Agreed with the changelog-backfill
  planner.

### 2026-11-16: The checks read the layout from the tree, before anything moves (step 4 splits)

- **The gate runs `main`'s `pigeon`, not the change's.** The bound job and
  every check job run `main`'s newest build from `builds/`. The GitHub
  checks run `main`'s `ci.yml`. Today both have the layout built in:
  - `scope::WORKSPACE = "src/pigeon-cli"`, the directory every check runs in;
  - crates found by reading `<WORKSPACE>/crates/`;
  - `WORKER` and `WASM_ONLY` (`pigeon-backplane` and its `worker/` directory);
  - `PIGEON_SHARED` and the per-feature clippies of package `pigeon`;
  - `IMAGE_BLOCKS` and `builds()` (`src/pigeon-cli/…` paths);
  - `--root ../..` in check argv, and `actions::TEST_MAP`;
  - `ci.yml`'s `working-directory`, cache paths and `tests.json` path.

  A change that moves the workspace or renames a package would be checked
  by a `main` that looks for the old layout, and would fail its own gate.
- **So the layout becomes data in the checked tree.** Step 4 splits in two:
  - **4a (planner; guarded for `ci.yml`).** Each item above is read from the
    checked-out tree, not from constants:
    - the workspace is the root `Cargo.toml` when it has `[workspace]`, else
      `src/pigeon-cli/Cargo.toml`;
    - crates come from its `members` globs;
    - the worker package, its feature and target, its wasm-only directories,
      the CLI package and its shared files, and the image inputs are in that
      manifest's `[workspace.metadata.pigeon]`;
    - argv's `--root` is the workspace's path back to the root;
    - `ci.yml` finds the workspace the same way, in a first step that sets
      `working-directory`, and keys its cache on `**/Cargo.lock`;
    - `behaviours.sh` and `test-map.sh` find it the same way.

    4a moves nothing. Its tests cover both layouts.
  - **4b (brief)** is the move itself, as step 4 described. It changes only
    data (manifests, metadata, paths), and no code.
- **Order.** 4b, and every later crate move, merges only after a deploy of
  `main` holding 4a has passed. From then on, the checking `pigeon` reads the
  layout from the tree it checks. The same rule makes steps 5 to 7 data
  edits for the checks: each renames or moves a package in the manifests
  and in `[workspace.metadata.pigeon]` together.

### 2026-11-16: The backfill is the release job's first run

§7's backfill change can't pass `main`'s policies:
- `pull-request.yaml` refuses any change whose files include a
  `CHANGELOG.md`, and the bound job applies it to every change;
- `change.yaml`'s `max_lines` refuses a file of about 900 lines.

It isn't needed either. The bot renders the whole file from first-parent
history on every run (!694), so its first run writes the backfill: every day
since 2026-11-02, the 32 retirements of `.config/releases.yaml` (!710), and
one release per changed unit, tagged as usual.

- **No change commits a `CHANGELOG.md`.** `pull-request.yaml` stays as it is.
  The rule in `CLAUDE.md` (!699) holds without an exception.
- **`.config/policies/changelog.yaml`** checks the one file's layout:
  - `## <day>` sections, newest first;
  - `### Released` and `### Merged` only;
  - a released line is `- <unit> v<X.Y.Z>`;
  - a merged line is a linked subject with an optional ` · <units>` suffix,
    or the rename line.
- **`release-changelog.yaml` is deleted.** Every file it named is retired.
- **The release job in `MainRun`** commits the bot's output to `main` after
  each deploy that passes. It is step 3's last part, and its first run is the
  backfill.

### 2026-11-16: Reviewed against the bench ADR (!715); they agree

Reviews !715 against this ADR and
[`2026-11-16-development-session-bench-containers.md`](2026-11-16-development-session-bench-containers.md),
whose demo amendment it adds.

- **They agree. !715 changes nothing either ADR decides differently.**
  - It touches no `Cargo.toml`. Its files are `cobot/bench.rs`, `/Dockerfile`
    and the bench ADR.
  - `cobot/bench.rs` moves to `bench.rs` per §2, as any file of its does.
  - It names the sentinel only through `crate::sentinel::{VAR, BINARY}`: the
    `PIGEON_SENTINEL` variable and the `<work>/bin/pigeon-sentinel` copy.
    So §5's rename to `PIGEON_EVICTION` and `pigeon-eviction` carries them
    with the constants, with no change of its own.
  - Its `/Dockerfile` edit adds `iproute2` to the `ci` stage's
    post-warm-up layer. That isn't a stage §5 renames, nor one of the `ARG`s
    §4 pins.
### 2026-11-16: Reviewed against the test map by module (!714); they agree

!714 carries out step 2b of
[`2026-11-16-development-one-test-binary-per-crate.md`](2026-11-16-development-one-test-binary-per-crate.md)
(§3, §4). It changes no manifest and nothing this ADR governs.
- **The map.** `test-map.sh` maps a `suite` per top-level module. It still
  lists packages and targets with `cargo metadata` from the workspace, so it
  follows §1's layout and the moves of steps 5 to 7 with no edit.
- **Moves.** A unit names its package and files. After a crate moves, the old
  map's files match nothing, and selection falls back to the reached crates
  until the next map. A move needs no map edit.
- **The ledger.** A stale `tooling/ci/tests.ledger` is a warning. A move
  still adds its lines to `tests.moves` (§6), and `--check` still fails on a
  test that is gone or a package that runs fewer tests.
### 2026-11-16: Step 5's sentinel change carries the workspace's bookkeeping (!718 reviewed; they agree)

Step 5 moves and renames the sentinel crate. It was refused (!713) because a
sentinel change could touch only sentinel's paths and its ADR. The owner
decided (question #7) that a sentinel change may also carry the workspace's
bookkeeping. That is recorded in
[`2026-11-10-development-factory-crates-functional-core.md`](2026-11-10-development-factory-crates-functional-core.md)'s
2026-11-16 amendment and built in three changes:

- !718: the approval rules read `sentinel_bookkeeping`;
- !724: `.config/approvals.yaml` lists the bookkeeping files: `Cargo.toml`,
  `Cargo.lock`, `tooling/ci/tests.ledger`, `tooling/ci/tests.moves` and
  `.config/releases.yaml`;
- !719: the test harnesses and `mise`'s `sentinel-build` build the judge with
  `--bin pigeon-sentinel`, and the scope tests find its crate wherever it is.

This agrees with §7 as amended by !711. Step 5 leaves the sentinel crate's
`CHANGELOG.md` at its old path, where !710 retires it. It touches no
`CHANGELOG.md`, so `src/**/Cargo.toml` is the only path that !711 and !718
both name. Step 5 then holds only the judge's paths and that bookkeeping, as
the owner's own grant, never a mandate. Step 12's binary rename follows the
same rule.
### 2026-11-16: Steps 8 and 9 both join the workspace (!726 reconciles !697)

- **Neither side's decision changes.** The site-templates crate (step 8,
  !697) and the pigeon-ios check crate (step 9, !702) both join the root
  workspace: `members` names `src/pigeon-ios` and
  `src/pigeon-site-templates`.
- **`.dockerignore`** keeps !702's lines that admit pigeon-ios's
  `Cargo.toml`, `src/` and `tests/`. It drops `src/site-templates/`, which
  holds only the two template changelogs now, so the image's workspace finds
  `src/pigeon-site-templates/`.
- **`tooling/ci/tests.moves`** keeps both changes' lines.
- **!697, rebased onto main, carries those three files.** This change carries
  only this entry.
### 2026-11-16: `Governs` names `rust-toolchain.toml` and `dev.rs` in place of `mise.toml`

Step 10 of
[`2026-11-16-development-src-crates-neutral-names.md`](2026-11-16-development-src-crates-neutral-names.md)
deletes `mise.toml`. The toolchain pin it held is now `/rust-toolchain.toml`
(!728), and its task graph is now `pigeon dev`'s, in `src/**/dev.rs` (!696).
This ADR's `Governs` line names whichever of the two took over the part it
rules.

### 2026-11-16: Review of !713 against !757: they agree

!757 amended
[`2026-11-15-runtime-one-mcp-entry-point.md`](2026-11-15-runtime-one-mcp-entry-point.md)
over `src/**/mcp.rs`, `src/**/wire.rs`, `src/**/routes.rs` and the Worker's
`api`, `app`, `devices` and `mod` files. !713 (step 5) touches none of them:
it moves only the policy engine to `src/pigeon-eviction`, with the
workspace's bookkeeping. Step 6 moves those files into `pigeon-core` and
`pigeon-agent` with their names kept, so the `src/**/…` globs still match them.
Neither decision changes.
### 2026-11-16: Review of !736 against !757: they agree

!736, step 10e, touches none of the files
[`2026-11-15-runtime-one-mcp-entry-point.md`](2026-11-15-runtime-one-mcp-entry-point.md)
governs (`mcp.rs`, `wire.rs`, `routes.rs`, `worker/{api,app,devices,mod}.rs`,
`API.swift`). It deletes both `mise.toml` files, and edits `dev.rs`,
`cli/mod.rs`, the `Dockerfile`'s comments and the tests that read
`mise.toml`. !757 re-briefs that ADR's steps against this ADR's layout after
step 12, which !736 doesn't change.

### 2026-11-16: Step 11 as the owner decided: two stored names stay

The owner answered action #8:

- **The control Worker's signing secret keeps its stored name, `BACKPLANE_TOKEN`.**
  It is not re-put as `AGENT_TOKEN`, because platform tokens are sealed with
  its value and no session should handle it. Code names it `AGENT_TOKEN`
  everywhere. The mapping to the stored name lives in one place: a single
  constant and accessor in the Worker. A Cloudflare secret binds only under
  its stored name, so `worker.yaml` cannot alias it. That accessor is the only
  reader of the stored secret. The Worker hands Containers the value under
  `AGENT_TOKEN`. `worker.yaml`'s `secrets:` lists the stored name alone, so the
  `AGENT_TOKEN` entry !752 added goes. A later rotation or rename of the
  stored secret then changes that one constant and the `secrets:` entry.
- **The state repository keeps its name.** Nothing names it but `CLAUDE.md`'s
  rule against applying by hand, and the goal is to retire that repository
  altogether, in a later ADR.
- The owner renames the old variable names on their own machines (`COBOT_*` →
  `PIGEON_*`, `BACKPLANE_TLS_*` → `AGENT_TLS_*`) without any value passing
  through a session.

Step 12's names list records these as stored names that keep their old
spelling, beside `.cobot/types.json` and tofu's `cobot` key provider:
`BACKPLANE_TOKEN`, with its one accessor, and the state repository's name. It
does the one-place alias above, and removes the `AGENT_TOKEN` entry from
`worker.yaml`.
### 2026-11-16: Review of !746 against !757: they agree

!757 amended
[`2026-11-15-runtime-one-mcp-entry-point.md`](2026-11-15-runtime-one-mcp-entry-point.md)
over `src/**/mcp.rs`, `src/**/wire.rs`, `src/**/routes.rs` and the Worker's
`api`, `app`, `devices` and `mod` files. !746 (step 6) touches none of them:
it rewrites import paths in the guarded callers to name `core::` modules and
### 2026-11-16: Review of !746 against !766: they agree

!766 amended
[`2026-11-15-development-sessions-stop-only-for-oversight.md`](2026-11-15-development-sessions-stop-only-for-oversight.md)
over `src/**/change/stage.rs`, `src/**/wire.rs`, `src/**/mcp.rs`,
`src/**/sessions.rs`, `src/**/tasks.rs`, `src/**/session/launch.rs` and the
Worker's `mod`, `api` and `push` files. !746 (step 6) touches none of them: it
rewrites import paths in the guarded callers to name `core::` modules and
deletes the 14 three-line re-export shims. The `src/**/…` globs still match.
Neither decision changes.

### 2026-11-16: Review of !772 against !766: they agree

!766 amended
[`2026-11-15-development-sessions-stop-only-for-oversight.md`](2026-11-15-development-sessions-stop-only-for-oversight.md)
over `src/**/change/stage.rs`, `src/**/worker/{mod,api,push}.rs`,
`src/**/wire.rs`, `src/**/mcp.rs`, `src/**/sessions.rs`, `src/**/tasks.rs`
and `src/**/session/launch.rs`. !772 (step 7a) moves the `pigeon` binary's
crate to `src/pigeon-harness` with its file names kept. It changes no code in
those files, and the `src/**/…` globs match them at either path. Neither
decision changes.

### 2026-11-16: Review of !736 against !766: they agree

!736, step 10e, touches none of the files
[`2026-11-15-development-sessions-stop-only-for-oversight.md`](2026-11-15-development-sessions-stop-only-for-oversight.md)
governs (`change/stage.rs`, `worker/{mod,api,push}.rs`, `wire.rs`, `mcp.rs`,
`sessions.rs`, `tasks.rs`, `session/launch.rs`). It deletes both `mise.toml`
files, and edits `dev.rs`, `cli/mod.rs`, the `Dockerfile`'s comments and the
tests that read `mise.toml`. !766 changes how a stage counts a mandate and how
a generated change closes, which !736 neither relies on nor changes.

### 2026-11-16: Review of !768 against !766: they agree

- !768 renames `x-cobot` to `x-pigeon` in the catalog and `infra/pigeon`
  schemas (§5). It also changes three tests that read the real schemas:
  `tests/suite/{architecture,cobot_apex,cobot_manifest}.rs`.
- [`2026-11-15-development-sessions-stop-only-for-oversight.md`](2026-11-15-development-sessions-stop-only-for-oversight.md),
  as !766 amended it, governs none of those paths. The two changes met only
  through !766's scope `tests/**`. Its own tests are `mandates.rs` and
  `tasks.rs`.
- Nothing either ADR decides changes.

### 2026-11-16: Review of !797 against !801: they agree

- !797 renames agent's library from `backplane` to `agent` (§1). It touches
  none of the four paths
  [`2026-11-15-security-owner-approvals-always-listed.md`](2026-11-15-security-owner-approvals-always-listed.md)
  governs: `worker/approve.rs`, `app/approvals.rs`, `approval/listed.rs`
  and `worker/listed.rs`. The two changes met only through !797's scope
  `src/**`.
- Its one shared file is `tests/suite/grants.rs`, where `backplane::` paths
  become `agent::`. The tests !801 added there follow that rename when
  !797 restacks, and nothing they assert changes.
### 2026-11-16: Review of !797 against !818: they agree

- !797 renames agent's library from `backplane` to `agent` (§1). It touches
  none of the paths
  [`2026-11-16-development-merge-path-measured-faster.md`](2026-11-16-development-merge-path-measured-faster.md)
  governs: `worker/main_run.rs`, `worker/merges.rs`, `worker/github.rs`,
  `cobot/ci.rs` and `release/**`. The two changes met only through !797's
  scope `src/**`, and !818 changed no code.
- a7 (!803) moves `cobot/ci.rs` to `checks/ci.rs` (§2). !835 widens that
  ADR's glob to `src/pigeon-agent/src/*/ci.rs` first, and (d) narrows it
  afterwards, as !806 did for eight other ADRs.
- Nothing either ADR decides changes.

### 2026-11-17: Review of !797 against !820: they agree

- !820 changed no code. It amends
  [`2026-11-09-security-approvals-once-passkeys.md`](2026-11-09-security-approvals-once-passkeys.md)
  (an owner approval covers its head's merge) and
  [`2026-11-16-development-merge-path-measured-faster.md`](2026-11-16-development-merge-path-measured-faster.md)
  (§5 decided; its execution steps 6a and 6b wait for this ADR's step 12).
- !797 renames agent's library from `backplane` to `agent` (§1). Of the paths
  those ADRs govern, it touches only `tests/suite/approvals.rs` and
  `tests/suite/passkeys.rs` (through `src/**/approvals.rs` and
  `src/**/passkeys.rs`), and there only renames `backplane::` paths to
  `agent::`. Nothing those tests assert changes.
- Steps 6a and 6b build on the module paths step 12 leaves:
  `checks/ci.rs` after a7 (!803), whose `Governs` glob !835 widens first.
- Nothing either ADR decides changes.
### 2026-11-17: Review of !847 against !849: they agree

- !849 amended
  [`2026-11-16-development-merge-path-measured-faster.md`](2026-11-16-development-merge-path-measured-faster.md)
  over `src/**/change/queue.rs` and `src/**/macos/job.rs`. !847 touches
  neither: it changes core's `deploy/scope/{map,mod}.rs` and agent's
  `checks/scope.rs`.
- !847 is the step-12 follow-up to !792 under
  [`2026-11-16-development-one-test-binary-per-crate.md`](2026-11-16-development-one-test-binary-per-crate.md)
  §3 and
  [`2026-11-15-development-code-checks-by-reach.md`](2026-11-15-development-code-checks-by-reach.md):
  a test-map unit naming a binary its package no longer builds picks no
  tests, so a change checked across b2b's binary rename runs tests by crate.
  It changes which tests a change's checks run, not when a merge takes its
  turn or how a macOS job is placed.
- Nothing any of the three ADRs decides changes.
### 2026-11-17: Review of !842 against !849: they agree

- !849 changed no code. It amends
  [`2026-11-16-development-merge-path-measured-faster.md`](2026-11-16-development-merge-path-measured-faster.md)
  with the merge path's next steps: a waiting macOS gate gives up the queue
  head, and a re-push keeps its place.
- !842 is this ADR's step 12 (e): the job images' `FEATURES`, the `agent`
  release unit, `ship.rs`'s job build and the Dockerfile's comment. It touches
  neither `change/queue.rs` nor `macos/job.rs`, which that ADR governs. The two
  changes met only through the review rule's scope.

### 2026-11-17: Review of !886 against !862: they agree

- !862 changed no code. It amends
  [`2026-11-15-development-capacity-pools-hard-spend-cap.md`](2026-11-15-development-capacity-pools-hard-spend-cap.md)
  with step 3's brief (b67), split in three changes.
- !886 (the sweep's A1) touches one of the paths that ADR governs,
  `jobs.rs`, and there only renames a log line's prefix, `sentinel:` to
  `eviction:`. It touches neither `brief/mod.rs` nor `insights/mod.rs`.
- Nothing either ADR decides changes.

### 2026-11-17: Review of !886 against !877: they agree

- !877 changed no code. It amends
  [`2026-11-15-development-capacity-pools-hard-spend-cap.md`](2026-11-15-development-capacity-pools-hard-spend-cap.md):
  step 4's spike held, and step 5 runs `claude` and the checks in the
  sandbox.
- !886 (the sweep's A1) touches one of the paths that ADR governs,
  `jobs.rs`, and there only renames a log line's prefix, `sentinel:` to
  `eviction:`. It touches neither `brief/mod.rs` nor `insights/mod.rs`.
- Nothing either ADR decides changes.

### 2026-11-17: Review of !948 against !970: they agree

- !970 amended
  [`2026-11-15-development-capacity-pools-hard-spend-cap.md`](2026-11-15-development-capacity-pools-hard-spend-cap.md)
  over `src/**/jobs.rs`, `src/**/brief/mod.rs`, `src/**/insights/mod.rs`
  and `src/pigeon-core/src/pool/**`: how a brief routes to a pool and how
  its task is dispatched and followed.
- !948, part A3 of step 12, touches only `src/pigeon-agent/src/jobs.rs` of
  those. There it renames the bound and eviction jobs' `sentinel` parameter
  to `judge` and rewords two doc comments. It changes no behaviour, and
  nothing about pools, briefs or insights.
- Nothing either ADR decides changes.
