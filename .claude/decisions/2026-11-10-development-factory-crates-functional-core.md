## The factory's code: a separate sentinel that judges from main, cobot folded into backplane, a functional core, and tests by code path

* Date: 2026-11-10
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development

## Context

pigeon is built as a factory. Several agent sessions work at once, each on
changes bounded by a scope of globs. Every change is judged by sentinel,
checked by scoped CI, approved, and merged through one queue
([`2026-11-08-development-concurrent-agent-sessions.md`](2026-11-08-development-concurrent-agent-sessions.md)).
Some of the code's shape works against that.

**1. The judge is built and configured by what it judges.**

- Sentinel is its own crate, `sentinel`, but it ships only inside the one
  `pigeon` binary.
- Its release unit in `.config/releases.yaml` lists `pigeon`'s `main.rs`,
  `lib.rs`, `cli/mod.rs`, `commands/mod.rs`, `Cargo.toml` and the Dockerfile.
  So an edit to the CLI releases sentinel.
- The bound job runs on main's newest build of `pigeon`, so the judging code
  comes from `main`. But `jobs::policy` loads the instance with
  `Instance::load(dir)` on the checked-out head.
  - So `.config/policies/` comes from the change being judged.
  - A change can loosen a policy and be judged by the looser one.
  - Only the approval tiers come from `main`.

**2. Two engines with one boundary that no longer holds.**

- `backplane` uses `cobot` in 16 of its files. `cobot` uses nothing of
  `backplane`.
- Both have a `worker` feature built for wasm.
- Deploying is split between them: `cobot/src/ship.rs` and `ship_wire.rs`,
  and `backplane/src/ship.rs`.
- [`2026-11-03-development-backplane-platform-composition-layer.md`](2026-11-03-development-backplane-platform-composition-layer.md)
  says backplane writes and starts nothing. Yet its jobs check out trees and
  run `cargo` and `git`, and its Worker deploys.
- The split costs a crate boundary, two feature matrices and two release
  units, and buys no isolation.
- `gantry` is a 22-line placeholder with a release unit and an image.

**3. State is mutated in place, and logic is tangled with I/O.**

- The change lifecycle in `backplane/src/changes.rs` has 22 `&mut self`
  methods. `platforms.rs` has 9, `mandates.rs` 7 and `control.rs` 7.
- Worker handlers load storage, decide, mutate and write in one function. For
  example, `worker/passkeys.rs::serve` loads the keyring, changes it and
  stores it.
- So the rules can be tested only with storage around them, can't be replayed
  from events, and make the Durable Object compatibility rules
  ([`2026-11-08-runtime-blue-green-self-standup.md`](2026-11-08-runtime-blue-green-self-standup.md))
  harder to keep.

**4. Hot files make changes overlap.**

- On 2026-11-10, changes 131 and 133 overlapped another session's 126 and 132
  on `worker.yaml` and `CLAUDE.md`.
- `CLAUDE.md` gains a line for every ADR.
- `backplane/src/worker/mod.rs` routes every path, in 966 lines.
- Five files have more than 1,300 lines: `cobot`'s `workers.rs`,
  `config.rs`, `engines.rs`, `apex.rs` and `sites.rs`.
- A narrow change's scope still names a wide file, so `overlaps` reports
  conflicts that aren't real, and restacks conflict.

**5. Tests are selected by crate, and nothing runs them all before a merge.**

- `cobot::scope` picks a changed crate and every crate that depends on it. A
  one-line change to `backplane` runs every `backplane` and `pigeon` test. A
  change to `cobot` runs nearly everything.
- The whole suite runs only for workspace-wide paths.
- The merge squashes the checked head, or a restacked one whose approval
  carried over. No check runs the full suite on the tree that becomes `main`.

The owner asked for five things:
- code and file organization normalized for the factory;
- pure functions, immutability, and each piece of mutable state owned by one
  thing;
- sentinel as its own `pigeon-sentinel` crate that runs separately, so changes
  to the CLI don't pass through to it, and sentinel changes are their own
  grant;
- cobot folded into backplane;
- per-push checks that run only the tests on changed code paths, with the full
  suite run once, before the final merge.

## Implementation plan.

### 1. Three crates, by role

| Crate (package) | Binary | Role | Depends on |
|---|---|---|---|
| `crates/sentinel` (`pigeon-sentinel`) | `pigeon-sentinel` | judges a tree against policies, and reports | nothing in the workspace |
| `crates/backplane` (`pigeon-backplane`) | none | the platform: its core (§3), and the shell that acts: CLI commands, the Worker, jobs, Cloudflare, OpenTofu, git, wrangler | nothing in the workspace |
| `crates/pigeon` (`pigeon`) | `pigeon` | the CLI: argument parsing over `backplane` | `pigeon-backplane` |

- **`cobot` becomes part of `backplane`.**
  - Its modules move into `backplane`'s core or shell (§3).
  - `pigeon cobot …` stays as a command group, so scripts and docs keep
    working.
  - Its `worker` feature merges into `backplane`'s.
- **`gantry` is removed,** along with its release unit, image and
  `pigeon gantry`. A later ADR can bring it back with a purpose.
- **`pigeon` no longer links sentinel.**
  - `pigeon sentinel …` runs the `pigeon-sentinel` installed beside it, or
    the one on `PATH`, with the same arguments.
  - `pigeon backplane run policy.check` does the same, and reads its JSON
    report.
- **Release units become two:**
  - `sentinel`, whose paths are only `src/pigeon-cli/crates/sentinel/` and its
    Dockerfile stage;
  - `backplane`, for everything else.

  `cobot` and `gantry` stop releasing. Their tags and changelogs stay as they
  are.
- **Images:**
  - `pigeon-backplane` takes over the cobot stage's tools (`tofu`, `zola`,
    wrangler), and keeps the test that their versions equal `mise.toml`'s.
  - `pigeon-sentinel` is the sentinel binary alone.
- **This supersedes in part:**
  - [`2026-11-02-development-sentinel-policy-cobot-actions.md`](2026-11-02-development-sentinel-policy-cobot-actions.md)'s
    sentinel and cobot split. Sentinel stays policy alone, and its library
    still writes no file and starts no process. Backplane's shell now does
    what cobot did.
  - [`2026-11-03-development-backplane-platform-composition-layer.md`](2026-11-03-development-backplane-platform-composition-layer.md)'s
    "writes and starts nothing". That property now belongs to backplane's
    core (§3).
  - [`2026-11-02-release-crate-units-and-images.md`](2026-11-02-release-crate-units-and-images.md)'s
    four units. They are now the two above.

### 2. Sentinel judges from `main`, and a sentinel change is its own grant

- **The judge is `main`'s.**
  - The bound job runs `pigeon-sentinel` from `main`'s newest build,
    `builds/<sha>/pigeon-sentinel`, pinned by SHA-256 as `pigeon-start` pins
    `pigeon`
    ([`2026-11-07-development-job-images-on-cloudflare.md`](2026-11-07-development-job-images-on-cloudflare.md)).
    It never compiles one from the head.
  - The deploy job stores `pigeon-sentinel` beside `pigeon` whenever the
    sentinel crate changed.
- **So are the rules.** The bound job reads `.config/policies/`,
  `catalog/policies/` and `.config/approvals.yaml` from the base commit, not
  the head.
  - A policy changed by a change takes effect once that change merges.
  - The change's own scoped checks still run its policy tests on the head, so
    a broken policy is caught before it merges.
- **Sentinel's own grant.** `.config/approvals.yaml` gains a `sentinel:` list:
  - `src/pigeon-cli/crates/sentinel/**`;
  - `.config/policies/**`;
  - `catalog/policies/**`;
  - `.config/approvals.yaml`.

  A change that touches the list:
  - touches nothing else, except its ADR and the sentinel crate's tests. The
    change policy refuses a scope that mixes them;
  - is at least `code` tier. Its change grant covers that change alone, and
    no mandate covers it;
  - is `owner` tier when it touches `.config/approvals.yaml`, since that file
    is on the `secrets:` list
    ([`2026-11-09-security-approvals-once-passkeys.md`](2026-11-09-security-approvals-once-passkeys.md) §2).
- **Sentinel releases only on its own paths.** Edits to the CLI, Cargo or the
  Dockerfile release `backplane` alone.

### 3. A functional core and an imperative shell

`backplane`'s source has three parts:

```
crates/backplane/src/
  core/<domain>/   pure: types, transitions, judgements, rendering; no I/O
  shell/<surface>/ effects: cli, worker (Durable Objects, routes), jobs, cloudflare, tofu, git, wrangler
  wire/            the serialized types core and shell share, tolerant per the blue-green ADR
```

- **Core domains:**
  - `change`: the lifecycle, scope, restack and reconcile;
  - `approval`: tiers, grants, mandates, passkeys and TOTP;
  - `deploy`: scope, the ship plan and versions;
  - `infra`: manifest, units, catalog, plans and moves;
  - `sites`;
  - `release`;
  - `platform`: runs;
  - `events`.
- **A transition is a function:**
  `fn step(state: &S, input: Input, at: Now) -> Result<(S, Vec<Effect>), Refusal>`.
  - It returns the next state and the effects to perform.
  - It never takes `&mut`, does I/O, or reads the clock or randomness. Time
    and random bytes are inputs.
- **Each mutable state has one owner.**
  - Every persisted state type names its owner in its doc comment: one
    Durable Object or one file.
  - Only the owner writes it. Others send it a message.
  - The shell loads the state, calls `step`, stores the result, then
    performs the effects in order.
  - An effect that fails is reported as an input to the next `step`, never
    patched into state.
- **Immutability by default:**
  - `let` without `mut`, except a builder local to one function;
  - no `static mut`;
  - interior mutability (`Mutex`, `RefCell`) only in the shell, as for the
    job board's reports.
- **Enforcement.** `.config/policies/layout.yaml` gains a rule: files under
  `crates/backplane/src/core/` contain none of:
  - `std::fs`, `std::process`, `std::net`;
  - `ureq`, `worker::`;
  - `SystemTime::now`, `Instant::now`;
  - `&mut self`.

  Clippy's `needless_pass_by_ref_mut` is denied workspace-wide.
- **Tests.** Core tests are tables of state, input, next state and effects,
  with no temporary directories or storage. Shell tests stay integration
  tests.

### 4. Files sized for parallel work

- **No file grows with every ADR.**
  - Each ADR gains a `* Governs:` line under its header: the globs it rules,
    as `change_propose`'s `affects` names them.
  - `pigeon backplane decisions <path>` lists the ADRs that govern a path.
  - `CLAUDE.md` keeps the workflow, the commands and that one pointer. Its
    per-ADR lines go.
  - The decisions policy checks that every `Governs` glob matches a file.
- **Routes register where they're served.**
  - Each `shell/worker/<surface>` module exports its routes.
  - `worker/mod.rs` becomes the table that joins them.
  - A new page touches its own module, not the router.
- **Smaller files.**
  - A file over 600 lines is split along its domain as it moves (§5's
    execution). The five 1,300-line files become per-domain modules.
  - The layout policy warns at 800 lines, outside `wire/` and generated
    files.
- **Tests sit beside what they test.** Each module's tests sit at
  `core/<domain>/tests.rs`, or in `tests/<domain>/`, mirroring the source. So
  a path maps to its tests (§5).

### 5. Checks by code path, and the full suite once, at merge

- **The test map.**
  - When the full suite runs at merge (below), it runs under coverage
    (`cargo llvm-cov nextest`). It records each test's ID and the source files
    the test executed, as `builds/<sha>/tests.json`, beside the build.
  - The `ci` image gains `cargo-nextest` and `cargo-llvm-cov`.
- **Per push: only the affected tests.** `change.scope` selects the tests
  whose recorded files meet the change's changed `.rs` files, plus every test
  the change adds or edits. They run with `cargo nextest run` and a filter.
- **Today's per-crate selection is the fallback** when any of these hold:
  - there is no map for `main`'s newest build;
  - the change touches `Cargo.toml`, `Cargo.lock`, `build.rs` or `.cargo/`;
  - it touches a file that the map doesn't know, such as a new module, a
    fixture or an `include_str!` target;
  - more than half of the tests would be selected anyway.
- **Clippy and builds stay per crate.**
  - The wasm builds run only when `shell/worker`, `core` or `wire` changed.
  - `fmt` always runs.
- **The merge gate.**
  - The merge job checks the exact head it is about to squash, after any
    restack, against the full set: `fmt`, clippy with every feature set, the
    wasm builds, and the whole test suite.
  - On a failure it refuses the merge. The change returns to `failed`, with
    the failing tests listed, and its session pushes a fix.
  - A head whose tree equals one that already passed the full set skips the
    gate.
  - A change with no Rust input skips its Rust part.
- **What it costs.**
  - The queue's turn per merge grows by the full suite's length.
  - Pushes before merge get much shorter. The full suite runs once per
    change, instead of on every push of a crate-wide change.
- **Locally:** `mise run ci` stays the full set. `mise run ci-scoped` uses the
  map when it can read it.

### What doesn't change

- The change flow, its tiers and grants.
- The ADR lifecycle.
- The Durable Object and wire compatibility rules:
  - serde names and Durable Object class names stay as they are across the
    move;
  - a moved type keeps its field names.
- The deploy pipeline. The release machinery changes only by its unit list.

### Execution

Each step is its own change. Each keeps `main` green and deployable. The
safety and speed steps come first, so the large moves run under them.

1. **This ADR,** with a `CLAUDE.md` pointer.
2. **Rules from `main`:** the bound job reads policies and approvals from the
   base commit. Tests for a change that loosens a policy.
3. **`pigeon-sentinel`:**
   - the binary, and the package rename;
   - `pigeon`'s exec shim;
   - the release unit narrowed to the sentinel crate;
   - the deploy job storing `builds/<sha>/pigeon-sentinel`, and the bound job
     using it;
   - the `sentinel:` approvals list, and the change policy's isolation rule.
     That part is a sentinel change itself, made by the previous step's rules.
4. **The merge gate:** the full set on the head to be squashed.
5. **The test map:** coverage in the merge gate, `tests.json`, and selection
   in `change.scope` with its fallbacks. `nextest` and `llvm-cov` in the `ci`
   image.
6. **cobot into backplane:**
   - a mechanical move, with `pub use` re-exports under `backplane::cobot` for
     one release, so imports and wire paths keep working;
   - `gantry` removed;
   - `.config/releases.yaml` amended, the cobot stage's tools moved, and the
     image test kept.

   It is proposed when few changes are open. Each open change gets a task to
   restack onto it.
7. **The core and shell split,** one domain per change. The `change` lifecycle
   goes first, then `approval`, `deploy`, `platform`, `infra`, `sites` and
   `release`. Each change turns on the core rule (§3) for its domain's
   directory, and splits any file over 600 lines it moves.
8. **ADR pointers:**
   - `Governs:` added to each existing ADR, which counts as an amendment;
   - `pigeon backplane decisions`;
   - the slimmer `CLAUDE.md`;
   - the decisions policy's check that every `Governs` glob matches.
9. **Routes by surface,** and the router as a table.

### Verification

- **The judge:**
  - A change that loosens a policy in `.config/policies/` is judged by
    `main`'s version, and refused when `main`'s refuses it.
  - A change to `pigeon`'s `main.rs` releases no sentinel version.
  - The bound job's log names `builds/<sha>/pigeon-sentinel` and its SHA-256.
- **Sentinel's grant:**
  - A change touching `crates/sentinel/` and `crates/backplane/` is refused
    as mixed.
  - A sentinel-only change asks for its own proof, and no mandate approves it.
- **The core:** the layout policy refuses a `std::fs` or a `&mut self` under
  `core/`. The `change` domain's tests run without a temporary directory.
- **Checks:**
  - A one-line change in `core/approval/` runs only the tests whose map
    entries include that file. Its checks take less time than the same change
    did before this ADR.
  - A merge whose full suite fails is refused, and its change goes back to
    `failed` with the test named.
- **The factory:** after step 8, adding an ADR doesn't touch `CLAUDE.md`, and
  two changes adding pages to different surfaces don't overlap.

## Amendments

### 2026-11-11: The test map is per test binary, and built after each merge on `main`

§5 said the map records each test, and that the merge gate's coverage run
writes it, as the CI ADR's §3 repeats
([`2026-11-10-development-code-checks-github-actions.md`](2026-11-10-development-code-checks-github-actions.md)).
Both are changed.

- **Per test binary, not per test.**
  - A test binary is one crate's library tests, or one `tests/*.rs` file:
    about sixty in the workspace.
  - Mapping each test would rerun the instrumented suite once per test, which
    means hundreds of runs. Mapping each binary takes one instrumented build
    and one run per binary.
  - It is still much finer than per crate: a change to one module runs the few
    binaries that touch it, not every binary of the crate and its dependants.
- **Built after each merge, not in the gate.**
  - The merge gate (step 4) runs the full set uninstrumented, so a merge turn
    doesn't grow by a coverage build.
  - After each merge, the control plane dispatches `ci.yml` on `main`'s new
    head, as change 0, with one check: `test map`. It runs each test binary
    under `cargo llvm-cov` and writes `tests.json`, mapping each binary to the
    workspace source files it executed.
  - The control plane copies that artifact to `builds/<sha>/tests.json`,
    beside the build of the same commit.
- **How a change uses it.**
  - Job Containers get the newest map beside the newest build, through
    `pigeon-start`, as they get `pigeon-sentinel`.
  - `change.scope` runs, for each changed source file under a crate's `src/`,
    each binary whose recorded files include it, as `cargo test -p <package>
    --lib` or `--test <name>`. A changed or added test file runs its own
    binary.
  - The fallback to per-crate selection stays as §5 lists it, for a missing
    map, build files, files the map doesn't know, and more than half of the
    binaries.
  - The map may be one merge behind. A file that a newer merge made reachable
    from a binary is caught by the gate's full set before it merges.
- **Tools.** `ci.yml` installs `cargo-llvm-cov` and `cargo-nextest` from pinned
  release archives checked by SHA-256, as it installs `tofu` and `zola`, and
  `llvm-tools` with rustup.

**Execution,** in place of step 5:

1. **`ci.yml`:** the `test map` check and its script. It writes `tests.json` as
   an artifact. It takes the owner's proof, since `.github/**` is guarded.
2. **The control plane:**
   - `MainRun` dispatches the `test map` check on each new `main`;
   - it copies the artifact to `builds/<sha>/tests.json`;
   - `pigeon-start` gets the newest map beside the newest build, as
     `PIGEON_TEST_MAP`.
3. **Selection:** `change.scope` reads `PIGEON_TEST_MAP` and picks binaries by
   it, with the fallbacks.

