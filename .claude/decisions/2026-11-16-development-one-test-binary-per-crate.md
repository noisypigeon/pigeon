## One test binary per crate: mechanism modules, a shared harness, a behaviour ledger, and a test map by module

* Date: 2026-11-16
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development
* Governs: `src/**/tests/**`, `src/**/deploy/scope/map.rs`, `tooling/ci/test-map.sh`

## Context

The owner asked for a faster way to show that the end result works. It must
give the same functional assurance, and keep a change's run filtered by the
mechanism it touches. They added that the end state removes the prior
design's tests in favour of the new one.

This ADR reads that as follows:
- every behaviour proven today is still proven;
- selection by reach and the test map keep working, at a finer grain;
- what changes is how the proof is built and laid out;
- when the migration ends, none of today's test files or harness copies
  remain.

It amends:
- the test-map amendment (2026-11-11) of
  [`2026-11-10-development-factory-crates-functional-core.md`](2026-11-10-development-factory-crates-functional-core.md);
- the selection rules of
  [`2026-11-15-development-code-checks-by-reach.md`](2026-11-15-development-code-checks-by-reach.md).

### What exists today

These were measured on 2026-11-16 at `912bbc7` (`main` after !653): locally
on 4 cores with rust 1.99.1, and from GitHub's `ci.yml` runs for !651 and the
test map of `f7caf4b`.

**Rust, by crate:**

| crate | integration files (binaries) | integration tests | unit tests |
|---|---|---|---|
| backplane | 70 | 670 | 543, 444 of them under `core/` |
| pigeon | 6 | 33 | 0 |
| sentinel | 6 | 50 | 17 |

- `cargo test --workspace` runs 1347 tests, and none fails.
- There are 89 test binaries in `target/debug/deps`; the test map has 87
  entries.
- No test is `#[ignore]`d.
- 22 tests return early, as a pass, when `tofu` or `zola` is missing:
  - cobot_apex 9;
  - cobot_plans 5;
  - cobot_agnostic, cobot_cobot and serve 2 each;
  - cobot_config and cobot_manifest 1 each.
  `ci.yml` installs both tools, so they do run there.

**Other checks that act as tests:**
- `policy.check`: 22 policies, 82 rules.
- `infra.validate`, `sites.build` (zola), `backplane instance`.
- The wasm clippy, the only check on `backplane/src/worker/**`. It has no
  test; only `tests/architecture.rs` reads it, as text.
- 87 Swift Testing cases in `PigeonCoreTests`, through `check.sh` on macOS.

None of these change here.

**Where the time goes.** It goes to building and linking the binaries, not to
running the tests.
- **!651's run:**
  - The job `test pigeon pigeon-backplane` took 4:01 of the run's 5:00, on a
    cache hit.
  - Of its ~80 binaries, every one but `serve` finished its tests in at most
    0.04 s. `serve` took 4.1 s.
  - The clippy jobs took 10 to 35 s each.
- **The test map on `main`:**
  - 8:21 of build and run, plus 47 s installing coverage tools.
  - `tooling/ci/test-map.sh` runs each of the 87 targets one after another,
    each with its own `llvm-cov report`.
- **Locally:**
  - `cargo test --workspace --no-run` took 86 to 95 s cold.
  - After a one-file touch in backplane's `core/`, it took 32 s, nearly all
    relinking.
  - Building one backplane test binary alone took 2.5 s.
  - The whole run took 70 s. Most of that is four binaries:

    | binary | time | why |
    |---|---|---|
    | `jobs` | 20.7 s | a nested `cargo build -p pigeon-sentinel` into `CARGO_TARGET_TMPDIR` (`tests/common/mod.rs`) |
    | `cobot_apex` | 12.4 s | |
    | backplane lib | 9.8 s | |
    | pigeon `backplane` | 9.2 s | the same nested build |

- **Disk:**
  - Each backplane test binary is ~178 MB of debug info, linking the whole
    crate and its dependencies.
  - One workspace build left 16 GB in `target/`. It grew to 29 GB once a
    `-p pigeon-backplane` build added a second backplane feature set, which
    filled this session's disk.
  - The CI cache saves and restores that directory. The reach ADR measured
    1:44 to 2:26 per cache save.
- **The prototype:**
  - A scratch `tests/suite/main.rs` held one `#[path] mod <file>;` for each
    of backplane's 70 files, with `autotests = false`.
  - It compiled with no source change: one 270 MB binary.
  - 669 of 670 tests passed. The one failure was the prototype's own
    `target/` placed inside the repository, which the layout policy refuses.
  - That backplane-only target took 2.5 GB, against ~16 GB for the same
    tests as 70 binaries.
  - The timing comparison was stopped before it finished, at the owner's
    request. Step 2's brief measures it.

**Overlap and duplication** in backplane's tests:
- Five merge-queue files (`merges`, `merge_kept`, `merge_front`,
  `merge_holder`, `merge_gate_early`) drive the same `merges` core through
  `common/step.rs`.
- Four approval-page files overlap: `approve`, `asks`, `asks_approval` and
  `mandates`.
- Ten `cobot_*` files plan infrastructure through the same calls, each with
  its own `repo_root` and write helpers.
- Seven files hand-roll a `127.0.0.1:0` fake HTTP server with an accept
  loop.
- Seven files repeat `tofu_available()`.
- `control` and `migrations` each `include_str!` the D1 migrations and copy a
  JSON-to-SQLite parameter converter.
- `tests/fixtures/connect/*.json` is read by no test yet: it holds the
  provider-connect ADR's response fixtures for that ADR's later steps, so it
  stays where it is.

**How selection works today**, and what assumes many binaries:
- The unit of selection is a test target: `--lib`, `--bin` or `--test`.
  `TestMap::select` (`scope/map.rs`) picks targets whose recorded files
  include a changed file.
- It gives up ("can't choose") when it would pick more than half of all
  targets, which falls back to whole crates.
- It emits no `--` filter.
- `scope/mod.rs` hard-codes five target names for `infra/` changes:
  `cobot_catalog`, `cobot_manifest`, `cobot_apex`, `cobot_git` and
  `cobot_credentials`. `tests/cobot_scope.rs`, `tests/instance.rs` and
  `tests/jobs.rs` assert them.
- With one binary per crate:
  - the map at target grain would pick the whole crate on almost every
    change;
  - the "more than half" rule would always fire.
  So the map's grain has to move below the target. The previous ADR rejected
  per-test mapping as one instrumented run per test.
  - Per module, it costs one run of the same binary per module, with no
    rebuild. That is about 20 runs, against today's 87.
  - libtest runs every test of a process in one profile, so a run is
    attributed as a whole. A module filter is therefore the finest grain
    that one run per unit gives.

### What this does not decide

The owner plans a repository restructure, MCP in place of HTTP calls, and a
Cloudflare AI Gateway.
- **Before or beside the restructure.** Steps 1 to 4 (the ledger, the map by
  module, and one binary per crate) don't depend on it. They make it cheaper:
  one target name per crate instead of 87, and modules that move with their
  code.
- **With the restructure.** Step 6 regroups modules by mechanism. It should
  follow the restructure's module boundaries, so it waits for that ADR and
  takes its mechanism names from it.
- **Contract tests at the Worker/MCP boundary.**
  - Nothing tests `worker/**` today.
  - Contract tests belong with the move to MCP. They would drive
    `wire::CATALOG`'s operations through `mcp.rs` in-process, and the Worker
    would only adapt them.
  - That is new assurance, not a migration of existing tests, so it is left
    to the MCP ADR. This ADR's harness (§3) gives it a place: one more
    module in backplane's suite.
- **The per-session containers ADR (!655)** caches a warm target per
  session. This ADR shrinks that target, from ~16 GB to an expected 4 to
  5 GB per workspace build, and touches none of its files.

## Implementation plan.

### 1. The layout

- **One integration binary per crate:**
  - `crates/<crate>/tests/suite/main.rs`;
  - `[[test]] name = "suite"`;
  - `autotests = false`.
  Each crate's `--lib` unit tests stay where they are. That makes 6 test
  binaries in the workspace, down from 89.
- **One module per mechanism** under `tests/suite/`, each a directory or
  file:
  - backplane's first cut is today's file names, kept as they are (step 4);
  - step 6 regroups them by mechanism, as in §2.
  - A module's name is unique in its crate, and is not a suffix of another
    top-level module's name. Under libtest's substring filter, `state::`
    would match `cobot_state::`. So `state` becomes `stored_state`, and the
    ledger's check (§4) refuses a suffix.
- **One harness per crate**, at `tests/suite/harness/`, in place of
  `tests/common/` and the per-file copies:
  - `http`: one fake server: bind to `127.0.0.1:0`, a thread, a handler
    closure.
  - `tools`: `tofu_available`, `zola_available`, `repo_root`.
  - `sqlite`: the migrations and the parameter converter, once.
  - `step` and `kept`: as today.
  - `sentinel`: the path of a `pigeon-sentinel` built once (§5).
- **Unchanged:**
  - the tests' own assertions and replay tables;
  - `tests/state/` fixtures, read by the stored-state module.

### 2. Mechanisms, and what each must keep proving

Each row is a mechanism module of backplane's suite after step 6, with the
behaviours that stay proven, and today's files whose tests it absorbs. The
ledger (§4) holds each test name. The table says what the tests prove
together.

| mechanism | behaviours that stay proven | today's files |
|---|---|---|
| `merge_queue` | one merge at a time in order; turns kept, released, moved to the front and handed back; yields and cycles; the scoped, full and early gates; carried checks and approvals; the replays of !481, !543, !546, !549 and !598 | merges, merge_kept, merge_front, merge_holder, merge_gate_early |
| `change_lifecycle` | the change state machine; restack and conflict; eviction candidates; reconcile and review changes; overlaps; the git door's push rule | changes, eviction, reconcile, sessions, gitdoor |
| `control_plane` | routing; push and job reports; the D1 schema and its constraints; the SQL statements; MCP tools equal to the catalog; routes; object versions | control, migrations, mcp, routes, versions, access |
| `approvals` | tiers from `.config/approvals.yaml`; the approval and mandate pages, escaped and without scripts; asks only at plan approval; grants used once; mandates' bounds and budget; platform offers | approvals, approve, asks, asks_approval, grants, mandates, platforms, platform_job, platform_shape |
| `owner_proofs` | passkeys (ES256, counters, lockout, bound challenges); TOTP per RFC 6238; enrolment once; VAPID | passkeys, totp, enroll, push |
| `sessions` | tickets, classes, archiving; tasks and limits; events and the long-poll arithmetic; the digest; briefs; session launch | tickets, tasks, events, digest, briefs, session_launch |
| `checks` | code checks for a change; the scope of each path; the test map's selection and storage; the CI window; the bound job, squash and mirror; the jobs server | actions, cobot_scope, testmaps, ci_window, jobs, serve |
| `infra` | units, selection and rendering; agnostic modules; secrets and rotation; credentials by capability; catalog fallback; local units; stored plans and their diffs; adoption; providers mirror; apexes and sites | cobot_cobot, cobot_agnostic, cobot_config, cobot_credentials, cobot_catalog, cobot_local, cobot_manifest, cobot_apex, cobot_plans, cobot_plan_diff, cobot_adopt, cobot_providers_mirror |
| `infra_remote` | git on Artifacts; R2 state; site sync; the Cloudflare run token | cobot_git, cobot_state, cobot_sites, cobot_cloudflare, gitcreds |
| `release` | release units and scope; restack onto a moved base; image tool versions; the ledger | cobot_release, cobot_release_scope, cobot_restack, cobot_image_tools, ledger |
| `workers` | Worker specs, deploys with fake wrangler and docker; ship outputs and versions | cobot_workers, cobot_ship |
| `instance` | the manifest and operation catalog; `policy.check` through `run_with`; the architecture graph; site templates | instance, architecture, cobot_template, decisions |
| `stored_state` | every stored type still reads its dated fixtures | state |

- **pigeon** gets one suite with its six files as modules: `cli`,
  `backplane`, `cobot`, `sentinel`, `session_sh` and `signing`. Each keeps
  its `#[cfg(feature)]`.
- **sentinel** gets one suite with its six files as modules: `change`, `cli`,
  `definitions`, `eviction`, `policies` and `tree`.
- `infra` is listed beside `infra_remote` because `infra/` changes select
  them by name today (§3, selection).

### 3. Selection, by module

**The map, version 2.** Each entry gains `filter`:

```
{package, kind, name, filter, files}
```

- `filter` is `""` for a whole target (as today, and every `--lib`).
- Otherwise it is `"<module>::"` for one top-level module of a `suite`.
- A version 1 map still reads: every `filter` is `""`. A job holding the old
  map keeps working.

**`tooling/ci/test-map.sh`:**
- builds the instrumented workspace once, as today;
- for each target, lists its tests with `-- --list --format terse` and takes
  their top-level modules;
- runs the target once per module, with `-- <module>::`;
- reports each run as today.

**`TestMap::select`:**
- picks `(target, filter)` units whose files include a changed file;
- a changed `tests/suite/<module>.rs` or `tests/suite/<module>/**` picks its
  own unit;
- a changed `tests/suite/harness/**` or `main.rs` picks its crate's whole
  suite;
- a changed unit-test module under `src/` picks `--lib`, as today;
- "more than half" counts units, not targets.

**Argv:**
- one job per package, as today;
- one cargo call per target within it, joined with `&&`, inside `sh -c`;
- `cargo test -p <pkg> --test suite --locked -- <m1>:: <m2>::`. libtest ORs
  its filters.
- `--lib` stays its own call.
- The job name lists the modules, as it lists targets today.

**Hard-coded names:**
- The `infra/` job becomes
  `cargo test -p pigeon-backplane --test suite --locked -- infra:: infra_remote::`
  (before step 6: the five module names it names today).
- `tests/cobot_scope.rs`, `tests/instance.rs` and `tests/jobs.rs` assert the
  new argv.

**Unchanged:**
- `gate_of` and `MAX_PACKAGES` count packages.
- The full set is `cargo test --workspace`.
- `ran_tests_of` already treats a `--test` job as partial.
- `ci.yml` runs any `cargo test` argv; its cache key keys on `test` in the
  name.

### 4. The behaviour ledger: proof that assurance didn't drop

- **`tooling/ci/behaviours.sh`** (POSIX sh) writes `src/pigeon-cli/tests.ledger`.
  - The ledger is one line per test: `<package> <target> <path::name>`,
    sorted, from `cargo test --workspace -- --list --format terse`.
  - Step 1 commits it at today's 1347 lines.
- **`behaviours.sh --check`** fails when:
  - a ledger line has no test of the same package and name;
  - the count of a package's tests drops.
  A moved test is matched through `tests.moves`: one line per rename,
  `<old path::name> <new path::name>`, written by the step that moves it.
  - It also refuses a top-level module name that is a suffix of another's in
    the same suite.
- **Each brief:**
  - runs `--check` before it pushes;
  - rewrites the ledger in the same commit;
  - leaves `tests.moves` growing.
  At step 7 the ledger names only suite tests, and `tests.moves` maps each of
  the 1347 old names to a new one.
- **Tests move one-to-one.** No brief under this ADR merges two tests into
  one. libtest can't list a table's rows, so a merge would hide a dropped
  case from `--check`.
  - The speed comes from the layout, not from fewer tests: every test but
    four runs in milliseconds.
- **Coverage, kept equal.** For each crate, the union of `files` in the test
  map, and `llvm-cov`'s line count, must not fall between the map before and
  after a step.
  - The step's brief compares the two from its own `test-map.sh` run.
  - Step 4 and step 6 are the moves where it could fall.
- **Mutation spot-checks.** Before step 4 and after step 6, a planner runs
  `cargo mutants` on three core files:
  - `core/change/`'s state machine;
  - `merges`;
  - `scope/map.rs`;
  each against its selected tests.
  - The caught count may not fall.
  - The tool runs in the planner's session, not in CI.
- **Soft skips stay visible.** The harness's `tools` prints, as today, when
  it skips. Step 5 adds `PIGEON_REQUIRE_TOOLS=1`, set by the test jobs in
  `ci.yml`, under which a missing `tofu` or `zola` fails instead.
  - That only strengthens: today a CI image without tofu would pass 22 tests
    silently.
  - `ci.yml` is guarded, so this takes the owner's proof.

### 5. Faster builds, with nothing proven less

- **No nested sentinel build.**
  - Today `jobs`, `instance`, `architecture` and pigeon's `backplane` build
    `pigeon-sentinel` into `CARGO_TARGET_TMPDIR`: a second target directory,
    compiled from scratch, 9 to 21 s each.
  - The harness's `sentinel` uses `PIGEON_SENTINEL` when set, as `var` does
    today.
  - A test job that selects one of these modules runs
    `cargo build -p pigeon-sentinel --locked` first, in the same target
    directory, and sets `PIGEON_SENTINEL`.
  - Without it, the harness builds as today, so a local `cargo test` still
    works.
- **One feature set per job.** `cargo test -p pigeon-backplane` and
  `--workspace` build backplane twice, with different features. Scope's test
  jobs keep `-p` (they need their package's features), so nothing changes
  here. The per-session containers ADR is told, since it caches across both.
- **Debug info.** Step 8 sets `[profile.dev] debug = "line-tables-only"` in
  the workspace `Cargo.toml`.
  - A backtrace still names file and line.
  - It is measured first (its brief), and kept only if a cold
    `cargo test --workspace --no-run` and the CI test job both get faster.
  - It touches `Cargo.toml`, so it runs the full set.

### 6. Expected savings

Measured where marked; the rest are estimates for step 2 and step 4's briefs
to confirm or correct.

| | today | expected |
|---|---|---|
| test binaries | 89 (measured) | 6 |
| backplane test target on disk | ~16 GB (measured) | ~2.5 GB (measured, prototype) |
| workspace target after one build | 16 GB (measured) | 4 to 5 GB |
| CI test job, backplane change | 4:01 (measured, !651) | 1:00 to 1:30 |
| relink after a `core/` edit, workspace | 32 s (measured) | under 10 s |
| nested sentinel builds per full run | 4, 9 to 21 s each (measured) | 0 in CI |
| test map on `main` | 8:21 (measured) | 2 to 4 min |
| CI cache saved per lockfile and kind | about the target's size | a quarter of it |

What gets slower:
- An edit to one test file rebuilds its crate's whole suite, not one
  binary: about 2.5 s today for one backplane binary.
- The suite crate's compile is shared by every module, so step 2 measures
  it.
- If it exceeds 30 s, step 4 splits backplane's suite in two: `suite_core`
  for the in-memory mechanisms, and `suite_io` for those that spawn tofu,
  git or servers. Selection is by module either way.

### 7. Execution

Each step is its own change, keeps `main` green, and keeps every ledger line.
Each is a brief for a Sonnet executor, created with `brief_create` once the
owner says to start. None starts with this ADR.

1. **The ledger.**
   - `tooling/ci/behaviours.sh`, `tests.ledger` (1347 lines) and an empty
     `tests.moves`.
   - Done when `--check` passes on `main`, and fails on a branch that deletes
     one test.
   - Checks: `mise run ci-scoped`.
2. **The map by module.**
   - `TestMap` version 2 with `filter`, read alongside version 1; `select`
     by unit; argv with `--` filters, one cargo call per target; the units'
     "more than half".
   - `test-map.sh` maps per module wherever a target holds a `suite`.
   - Unit tests in `scope/map.rs` for each rule in §3. `tests/cobot_scope.rs`
     and `tests/testmaps.rs` cover version 1 and 2 maps.
   - Also: measures the prototype layout's build and run times against
     today's, on the same machine, and adds them to this ADR as an
     amendment's numbers.
   - Done when a version 1 map selects as today. A hand-made version 2 map
     with a `suite` picks only its module's filter.
3. **pigeon and sentinel suites.**
   - Their six files each become modules of `tests/suite/`, with
     `autotests = false`.
   - `tests/common/` becomes `harness/`.
   - `tests.moves` and the ledger are updated.
   - Done when `--check` passes and the test map lists one `suite` per
     crate.
4. **backplane's suite, files as modules.**
   - Its 70 files become `tests/suite/<file>.rs` modules (`state` becomes
     `stored_state`).
   - `common/` becomes `harness/`.
   - The `infra/` job and its three assertions name modules.
   - The ledger and moves are updated; coverage is compared (§4).
   - Done when `--check` passes, a one-file `core/` change selects only its
     modules, and the CI test job of that change is timed.
5. **One harness.**
   - The seven fake servers, seven `tofu_available` copies, two migration
     loaders and their converters become the harness's.
   - The nested sentinel build is replaced (§5).
   - `PIGEON_REQUIRE_TOOLS` is added: `ci.yml`'s part takes the owner's
     proof.
   - Done when `--check` passes, and a full run has no nested cargo build.
6. **Mechanism modules** (§2), after the restructure's ADR merges, under its
   names.
   - Files are regrouped by mechanism, one mechanism per change.
   - Each change rewrites its moves and compares coverage.
   - The planner runs the mutation spot-check after the last one.
7. **The old layout gone.**
   - No `tests/*.rs` outside `suite/` remains, and no `tests/common/`.
   - The ledger names only suite tests.
   - `tests.moves` is kept as the record of where each old test went.
   - The factory ADR's per-binary wording is amended to point here.
8. **Debug info** (§5), measured, kept only if faster.

### 8. What would show that assurance dropped

- `behaviours.sh --check` fails: a test is gone, unmatched by a move, or a
  package's count fell.
- A crate's mapped files, or covered line count, fall between two maps.
- A mutation the spot-check caught before step 4 survives after step 6.
- A module of the suite stops appearing in the test map (no unit), so no
  change could select it.
- In CI, a tofu- or zola-gated test skips under `PIGEON_REQUIRE_TOOLS`.

## Amendments

### 2026-11-16: ADRs' Governs globs move with the tests; timings come from CI

- **Sixteen ADRs name test files in `Governs`.** The decisions policy needs
  each glob to match a file, so moving a test file breaks that ADR's line. An
  executor never amends an ADR, so the planner does it, in two changes.
  - **Before step 3:** each test path becomes `tests/**<file>`. Under
    sentinel's matcher, that matches the file both where it is and under
    `tests/suite/`.
  - **After step 4:** each becomes the suite's path, `tests/suite/<file>`.
    `state.rs` becomes `stored_state.rs`.
- **The timing comparison in step 2's brief is dropped**, at the owner's word.
  The savings in §6 are measured instead from the CI test job and the test
  map on `main`, after step 4.

### 2026-11-16: Governs globs match the old and the new crate layout

Per [`2026-11-16-development-src-crates-neutral-names.md`](2026-11-16-development-src-crates-neutral-names.md) §6, each
glob under `src/pigeon-cli/` or `src/site-templates/` becomes `src/**/<tail>`
(or `**<file>` for a workspace root file), which matches the file both before
and after the restructure moves it. That ADR's step 12 tightens them to the
new paths.

### 2026-11-17: Steps 6 to 8 under the restructure's names

The restructure has merged: backplane's suite is `src/pigeon-agent/tests/suite/`,
with 72 module files under new names. §2's table maps onto them as follows.
- **Mechanisms.** One directory per mechanism, `<mechanism>/mod.rs` declaring
  its files as submodules under their current names:
  - `merge_queue`: merges, merge_kept, merge_front, merge_holder,
    merge_gate_early, merge_release_turn, merge_released;
  - `change_lifecycle`: changes, eviction, reconcile, gitdoor, and `sessions`
    as `overlaps`;
  - `control_plane`: control, migrations, mcp, routes, versions, access;
  - `approvals`: approvals, approve, change_asks, asks_approval, grants,
    ledger (the grants ledger), mandates, platforms, platform_job,
    platform_shape;
  - `owner_proofs`: passkeys, totp, enroll, push, devices;
  - `sessions`: tickets, tasks, events, digest, briefs, session_launch;
  - `checks`: actions, checks_scope, testmaps, ci_window, jobs, serve;
  - `infra`: infra_units, infra_agnostic, infra_config, infra_credentials,
    infra_catalog, infra_local, infra_manifest, sites_apex, infra_plans,
    infra_plan_diff, infra_adopt, infra_providers_mirror;
  - `infra_remote`: git, infra_state, sites, workers_cloudflare, gitcreds;
  - `release`: release, release_scope, git_restack, workers_image_tools;
  - `workers`: workers, workers_ship;
  - `instance`: instance, architecture, decisions;
  - `stored_state` stays one file.
- **Filters.** A libtest filter is a substring, so `m::` must not occur in
  any test path outside module `m`, at any depth. `behaviours.sh --check`
  refuses that, not only a suffix. That is why `sessions` becomes `overlaps`.
- **One change.** Step 6 is one change, not one per mechanism. Every
  mechanism edits `main.rs`, the ledger and `tests.moves`, so the changes
  would serialize anyway. The moves are renames with no edit except an
  `include_str!` path. The same change:
  - sets the infra job's modules to `infra::` and `infra_remote::`;
  - points each moved file's `Governs` glob at `src/**/tests/suite/**/<file>.rs`.
- **Coverage.** No test body changes, so each module covers what it did.
  Two things show it held: the ledger's per-package count, and the next map
  listing a unit for every mechanism.
- **The mutation spot-check** runs after step 6. It applies three mutations
  to `pigeon-core`, one each in the merge queue, the change state machine
  and scope selection. Each must fail the suite before and after.
- **Left out of step 5:**
  - The nested sentinel build stays. CI caches it, so replacing it saves
    little.
  - The two migration loaders stay apart. Sharing them would loosen
    control's strict parameter assertion.
  - `PIGEON_REQUIRE_TOOLS` is in the harness. Setting it in `ci.yml` is a
    later change, under the owner's proof.
- **Step 7** is met: no integration test is outside a `suite/`, and
  `tests/common/` is gone. What remains is the factory ADR's amendment.
- **Step 8** is dropped. The CI image already sets
  `CARGO_PROFILE_DEV_DEBUG=0` and `CARGO_PROFILE_TEST_DEBUG=0`, so
  `line-tables-only` would change only local builds.
