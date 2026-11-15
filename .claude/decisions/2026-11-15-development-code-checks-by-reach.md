## Code checks by reach: a change runs only the checks its paths can affect

* Date: 2026-11-15
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development
* Governs: `src/pigeon-cli/crates/backplane/src/core/deploy/scope/**`, `src/pigeon-cli/crates/backplane/src/core/deploy/gate.rs`, `src/pigeon-cli/crates/backplane/src/core/change/gate.rs`

## Context

The owner asked for less CI per change, as a priority: every session waits on
it. A change should run only the checks its paths can affect. The full set
still runs where it must: on `main`, at release, and wherever the scope
can't decide.

This amends the merge-gate rules of
[`2026-11-10-development-factory-crates-functional-core.md`](2026-11-10-development-factory-crates-functional-core.md)
(§5 and its 2026-11-12 amendments), and the cache step of
[`2026-11-10-development-code-checks-github-actions.md`](2026-11-10-development-code-checks-github-actions.md).

### What was measured, before

These were measured on 2026-11-15 from `artifacts/main`'s last 80 merges
(!415 to !476), replayed through `TestMap::select` with `main`'s live test
map (`38206f9`, 81 binaries, 144 source files). GitHub's run times are from
the last 57 `ci.yml` runs.

- **Of 80 merges, 23 touched Rust.** 57 had no Rust input, and ran only
  `bound` and `mirror`, like !493.
- **20 of the 23 gated in full.** 19 of those fell back with "the test map
  can't choose this change's tests", and 1 because of a fixture file.
  - **13 of the 19** touch `crates/backplane/src/worker/**`. That module is
    `#[cfg(feature = "worker")]`, so it compiles only for wasm. No native
    test can run it, so it is never in the map.
  - **3** touch a unit-test module, `src/**/tests.rs`. Coverage lists no
    file whose only lines are test code.
  - **3** touch a file with no line that runs, such as `core/mod.rs`, a new
    module, or `pigeon`'s `cli/*.rs`, which runs only in a subprocess.
- **A typical push runs 8 checks**: fmt, clippy on the affected crates, four
  `pigeon` feature clippies, the wasm clippy, and per-crate tests. A full
  gate then runs 4 more: workspace clippy, `pigeon` with no features,
  `--features sentinel`, and `cargo test --workspace`.
  - !476 changed only `worker/passkeys.rs`, so the wasm clippy was its one
    meaningful check. It ran 12 jobs: 6.8 minutes on push, then 6.1 minutes
    of gate holding the merge queue.
- **Run times.**
  - Median change run: 4.9 minutes, mean 4.7, over 46 runs.
  - Runs the map scoped (!463, !465): 1.1 to 1.5 minutes.
  - `test map` on `main`: 11 minutes, after each Rust merge.
- **Cache saves.** In !500's run, 6 of 8 jobs spent 1:44 to 2:26 in "Post Run
  actions/cache", saving a target directory. Each job's work took 5 to 10
  seconds of clippy, and 3:53 for tests. The key is the check's name, and
  per-binary test names multiply those keys. Most jobs missed the exact key
  and saved a new entry, which, under the repository's cache limit, likely
  evicts others.
- **Totals.**
  - The 23 Rust merges ran about 183 push jobs and 89 gate jobs: 272 in all.
  - With the rules below, they would run about 148, and no gate job except
    on a carried head. The full gate's 6 minutes per merge leave the queue.

### Answers to the questions asked

- **Can the gate reuse a passed check on a restacked head?**
  - It already does: the 2026-11-12 amendment carries checks across a
    disjoint, same-patch restack, and !500 carries the approval too.
  - What's new here: the gate no longer re-runs checks under other names.
    A carried head runs its clippy once more, since a disjoint restack can
    still break the build.
- **Can the feature clippies be skipped?** Only where nothing native
  changed, as for a wasm-only path (§1).
  - Otherwise, each run builds a different part of `pigeon`'s code against
    a different feature set of `backplane`, whose workspace dependency has
    `default-features = false`.
  - For example, `signing`'s code is built against a backplane without
    `engines` only by `--features signing`.
  - Their cost is runner overhead, in parallel: 5 to 10 seconds of clippy
    each, beside the per-crate tests' 4 minutes. §5 cuts that overhead
    instead.
- **Can the test map choose in the common cases?** Yes. Wasm-only paths
  choose no native test, and a unit-test module chooses its lib binary.
  - When it still can't choose, the push's per-crate tests already cover
    every package the change can reach, its dependants included.
  - So "can't choose" no longer makes the gate full.
- **Batching gates across the queue?** Deferred.
  - With the rules below, a scoped gate runs no job, or one clippy on a
    carried head.
  - Stacking and bisecting would save little, and adds a failure mode to
    the merge queue.
  - Revisit when, a day after step 4, more than a quarter of merges still
    gate in full.

## Implementation plan.

### 1. Wasm-only paths

- `scope` gains `WASM_ONLY`: `src/pigeon-cli/crates/backplane/src/worker/`.
- A changed file under it:
  - adds fmt and the wasm clippy of its crate;
  - adds no native clippy, no `pigeon` feature clippy and no test, and no
    dependant, since nothing native compiles it;
  - is skipped by `TestMap::select`, so it doesn't make the map "can't
    choose".
- A change with both kinds of file runs the union.

### 2. Tests the map chooses

- A changed `src/**/tests.rs` or `src/**/tests/*.rs` that the map doesn't
  know chooses its package's `lib` binary.
- Any other unknown file still falls back to that package's whole tests.
  The fallback stays per crate.

### 3. When the gate runs the whole suite

- `gate_of` returns `Full` only for:
  - a path in `EVERYTHING`;
  - tests the map chose that span more than `MAX_PACKAGES` packages.
- The map not choosing, or no map, is `Scoped`. The push's per-crate tests
  cover every reachable package.
- These rules still make a gate full, unchanged:
  - a clean restack;
  - a red `main`;
  - `full: true` on `change_propose`, `change_merge` or the approval page.

### 4. The gate's set is the head's own checks

- The bound job reports as `full`:
  - the head's own code checks, which `jobs_of` gives;
  - plus `test`, `cargo test --workspace`.
  It no longer reports `full_of`'s workspace set.
- A scoped gate leaves `test` out, as it does today. So a head whose checks
  passed starts its merge at once.
- A head whose checks a restack carried (`Job.carried`) runs its `clippy`
  checks again as gate jobs, but not its tests.
  - `Job.carried` is `#[serde(default)]`, so an older stored change reads as
    not carried.
- A path in `EVERYTHING` already makes `jobs_of` the whole set. So such a
  change, and a release, still run everything.

### 5. `ci.yml`'s cache by check kind

- The key's last part becomes the check's kind (`wasm`, `map` for the test
  map's instrumented build, `clippy`, `test` or `other`) instead of its name.
- `fmt` restores no cache.
- A run saves only when the exact key missed. So a lockfile gets about five
  entries, saved once each.
- `.github/**` is guarded, so this step takes the owner's proof.

### What must always run

- `bound`, on every head: the policies, the tiers, the scope.
- On `main`:
  - `test map` after each Rust merge. Its failure opens a fix task and
    makes every later gate full, until a run passes.
  - The deploy's own builds.
- The whole set:
  - for `Cargo.toml`, `Cargo.lock`, `.cargo/`, `mise.toml` and
    `.config/releases.yaml`;
  - at release;
  - on a clean restack, a red `main`, or a request.
- `mise run ci`, locally, stays the full set.

### What it risks

- **A native change that breaks a wasm-only caller.** The wasm clippy still
  runs whenever backplane is affected.
- **A test in a package that isn't affected, failing on the change.** That
  can't happen, since the dependant closure includes every package that
  uses the change.
- **A semantic conflict across a disjoint restack.** The carried head's
  clippy re-runs in the gate. A failure in tests is caught by the
  `test map` on `main`.

### Execution

Each step is its own change, carried out from a brief. Steps 2 to 4 are
under 200 lines each.

1. **This ADR.**
2. **Brief: wasm-only paths, test modules and "can't choose".**
   - `core/deploy/scope`: §1, §2 and §3.
   - Tests replay !476 (wasm-only), !415 (a `tests.rs`) and !453 (by crate,
     scoped gate).
   - It lands before step 3, since the bound job runs `main`'s `pigeon`.
3. **Brief: the gate's set.**
   - §4: `jobs.rs` reports the head's checks plus `test`.
   - `core/change`: `Job.carried`, set on kept jobs, and the gate re-runs
     carried clippy jobs.
   - Tests cover a scoped head merging with no gate job, a carried head
     running `gate: clippy …`, and a full gate adding only `test`.
4. **§5 in `ci.yml`**, with the owner's proof.
5. **After.** A day after step 4 merges, an amendment records the same
   measures, taken the same way:
   - checks per Rust change;
   - full gates;
   - median run minutes;
   - push to merge, from `change_status` and `deploy_status`;
   - and whether to batch.

## Amendments

### 2026-11-15: Step 5 measured early; a carried head re-runs two clippy checks, not six

The owner asked for step 5 now, not a day after step 4. It was measured 8
hours after !515 merged (`main` at `8173faf`, 07:15 UTC), across 24 merges,
the same ways as §"What was measured, before".

- **Change CI runs** (GitHub `ci.yml`, change runs only):
  - median 4.4 minutes before, over 26 runs from 04:38 UTC;
  - median 1.45 minutes after, over 33 runs.
- **Last push to merge** (the event log, from the head's last
  `change.pushed` to `change.merged`):
  - median 4.8 minutes before, over 12 merges;
  - median 2.9 minutes after, over 25 merges.
  - For example, !539 changed only wasm-only code: fmt, the wasm clippy and
    the merge took 2.9 minutes.
- **Full gates.** 20 of 23 Rust merges before; 8 of 24 merges after.
  - Each of the 8 came from a clean restack.
  - Those with code checks added only `gate: test`, one or two gate jobs,
    where the old gate added four.
- **The gap between merges** stayed near 8 minutes: a median of 7.6
  before and 8.3 after.
  - Before, a queue about 19 deep set it.
  - After, the queue drained, so changes arriving set it, not the turn.
- **Batching** stays deferred: fewer than a quarter of merges gate in
  full for any reason but a clean restack.
- **A day of data** would add no different case, since every rule fired
  within these 24 merges. The scheduled check stands down.

**Carried heads.** The three carried heads (!507, !524 and !525) each
re-ran six clippy jobs in the gate, as §4 worded it.

- A `pigeon` feature clippy builds `pigeon`'s own code, which a disjoint
  restack carries unchanged.
- The crate clippy and the wasm clippy build every crate that `main`'s new
  code reaches.
- So a carried head re-runs only those two: a clippy without
  `--no-default-features`, or the one with `--target`.
- **What it risks.** A break that only one feature's build shows, from
  `main`'s new code meeting this change's. Any later change that reaches
  `pigeon` runs every feature clippy, and so does `mise run ci`.

**Execution:** one change. It holds this amendment, `rerun_carried` in
`core/change/gate.rs`, and a test in `tests/merges.rs` where a carried head
re-runs its crate clippy and not a feature one.
