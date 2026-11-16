## Benches: each agent session builds and tests only in its own warm Cloudflare Containers

* Date: 2026-11-16
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development
* Governs: `src/**/jobs.rs`, `src/**/gitdoor.rs`, `src/**/worker/gitdoor.rs`, `src/**/worker/checker.rs`, `tooling/session/session.sh`

## Context

Agent sessions build and test inside their own Claude Code container, which
has a fixed disk and CPU allowance. A cargo build of this workspace exhausts it:

- "no space left on device" on a full test build. A dev target of the
  workspace's 81 test binaries was measured at about 26 GB
  ([`2026-11-07-development-job-images-on-cloudflare.md`](2026-11-07-development-job-images-on-cloudflare.md)).
- Builds are slow, and nearly all test time goes to compiling and linking those
  binaries. The test-strategy session measured this and is cutting the count
  separately; this ADR doesn't change tests.
- The build competes with the session's own reasoning for the same CPU.

The owner asked for a pool of worker containers that sessions use only to run
and test code. The Claude session edits files, hands the work to its
containers and reads back the results. Each session owns at least one
container, and one session can fan out to several.

**What exists (read at `912bbc7`).**

- **`Checker`** (`standard-4`, 12 instances) and **`Runner`** (`standard-1`,
  8): one Durable Object per job, named by job ID, whose Container runs
  `pigeon backplane jobs --listen`. The Worker signs every call under
  `BACKPLANE_TOKEN`. The job checks out a head with a read token minted for
  it, runs, reports, and its Container is destroyed once the report is read
  (`worker/checker.rs`, `jobs.rs`).
- **Both run the `ci` image.** Its warm layer holds the third-party
  dependencies only, so a cold check compiles only the workspace's own crates.
  Its `pigeon` is `main`'s newest build, fetched by `pigeon-start`.
- **The image already has rootless prerequisites:** `uidmap`, the `nonroot`
  user (65532) and its subordinate ID ranges. A `Checker` Container allows
  user namespaces (`unshare -Ur`) and has 4 CPUs, 12 GB of memory and 15 to
  18 GB of free disk (the job-images ADR, 2026-11-07 amendments).
- **The git front door** (`gitdoor.rs`) accepts pushes only to
  `change/<n>-<slug>`, with the change's session ticket. Each push starts its
  change's checks.
- **The merge gate** is the bound job and the code checks `change.scope`
  selects, run on GitHub Actions or a `Checker`
  ([`2026-11-15-development-code-checks-by-reach.md`](2026-11-15-development-code-checks-by-reach.md)).
  `mise run ci-scoped` runs the same selection locally.
- **The capacity-pools ADR** plans the same sandbox for a `pool` job: a
  separate uid in a user namespace, with a cleared environment
  ([`2026-11-15-development-capacity-pools-hard-spend-cap.md`](2026-11-15-development-capacity-pools-hard-spend-cap.md) §7).
  It isn't built yet.
- **The open-change cap** becomes the owner's `limits.set`
  ([`2026-11-15-development-owner-sets-open-cap.md`](2026-11-15-development-owner-sets-open-cap.md)),
  which isn't built yet either.

**The options for the container.**

- **(a) Reuse `Checker` for sessions.** Its Containers are per job and
  destroyed after one report, so nothing stays warm. Sessions would also
  compete with the merge gate for its 12 instances. Not taken.
- **(b) The gate reuses a session's warm container.** The session controls
  everything in it, including the target directory a gate would trust. That
  breaks the gate's independence. Not taken, now or later.
- **(c) A new class, `Bench`, beside `Checker` and `Runner`.** It uses the
  same image and job server, but lives as long as its session, has its own
  instance limit, and accepts commands. Taken.

**The options for getting the tree there.**

- **Stream files to the container.** That's a new protocol, with no history
  and no dedupe. Not taken.
- **Push to the change branch.** Every push starts the change's checks, and a
  session may have no change yet. Not taken.
- **Push to a scratch ref of the session's own.** It's git's delta transfer
  through the existing front door, and starts nothing. Taken.

## Implementation plan.

### 1. A bench, and who owns it

- **A bench** is one `Bench` Durable Object and its Container, named
  `<session_id>.<k>`. `session_id` is the label's last path segment, and `k`
  is 0 to 2.
  - Bench 0 is the session's own.
  - Benches 1 and 2 are for fan-out.
- **Only the holder of the label's live ticket** can create, run on, read or
  reset its benches. Every `bench.*` operation takes `ticket`, and the
  control plane derives the bench's name from the ticket's label, never from
  an argument. One session can't name another session's bench.
- **Bench results are the session's alone.** They never reach a change's
  checks, its gate or its stage.

### 2. The working tree reaches the bench through a scratch ref

- **`refs/heads/scratch/<session_id>`** joins the front door's accepted refs.
  - `gitdoor::judge` accepts it only with a live ticket whose label ends in
    that `session_id`.
  - It accepts force pushes and deletes.
  - It starts no `ChangeRun` and no job, and appends no event.
  - Every other ref outside `change/<n>-<slug>` is still refused.
- **`pigeon session bench push`** snapshots the working tree without touching
  the session's index, `HEAD` or branch:
  - `git add -A` into a temporary index (`GIT_INDEX_FILE`), honouring
    `.gitignore`;
  - `git write-tree`, then `git commit-tree` with `HEAD` as the parent;
  - a force push of that commit to the scratch ref, with the ticket header and
    `http.postBuffer` as `CLAUDE.md` says.

  It prints the commit's SHA. A push with no change since the last one is
  skipped.
- **The control plane deletes the scratch ref** when the ticket ends (§4).

### 3. Running commands: `bench.*` operations and MCP tools

| Operation | Args | Answers |
|---|---|---|
| `bench.run` | `ticket`, `sha`, `argv`, `bench` (0 to 2, default 0), `timeout` (minutes, default 30, at most 90) | `run`, `bench`, `sha`, `state` |
| `bench.status` | `ticket`, `run` | `state` (`queued`, `syncing`, `running`, `passed`, `failed`, `timed_out`, `cancelled`), `exit`, `seconds`, the log's last 4 KB, disk used |
| `bench.logs` | `ticket`, `run`, `offset`, `wait` (seconds, at most 25) | up to 64 KB from `offset`, the next offset, `state`; `wait` long-polls for new bytes under the proxy's 30-second cut |
| `bench.cancel` | `ticket`, `run` | `state` |
| `bench.reset` | `ticket`, `bench`, `hard` | soft: `git clean -ffdx` except `target/`, plus the workspace crates' artifacts; hard: destroys the Container |
| `bench.list` | `ticket` | each bench: state, `sha`, last run, disk used, idle minutes |
| `bench.warm` | `ticket`, `bench` | starts the Container and syncs `main`, without running anything |

- **The MCP tools** are `bench_run`, `bench_status`, `bench_logs`,
  `bench_cancel`, `bench_reset`, `bench_list` and `bench_warm`.
- **The CLI is `pigeon session bench …`**, which `session.sh` already
  forwards:
  - `bench push`, as in §2;
  - `bench run [--bench K] -- ARGV…`: push, run, stream the log through
    `bench.logs`, and exit with the command's exit code;
  - `bench ci`: fan-out (§3.3);
  - `bench status|logs|cancel|reset|list|warm`.
- **One run at a time per bench.** A second `bench.run` on a busy bench is
  refused with `wait: bench K is running <run>`. Fan-out uses other benches.
- **Logs are kept.** A finished run's log is written to the `CI_LOGS` bucket
  at `bench/<session_id>/<run>.log`, with job logs' retention, so it outlives
  the Container.

#### 3.1 The allowlist

`bench.run` takes an argv list, never a shell string, and only these:

- `mise run <task>`, for `<task>` in `ci-scoped`, `build`, `test`, `fmt`,
  `fmt-check`, `lint`, `rust`, `features`, `sentinel`, `sentinel-build`,
  `sentinel-change`, `backplane`. The `ci` image has no `mise`, and installing it would bring
  tool installs that need the network. So `core::bench::tasks` reads
  `mise.toml` at the synced commit, resolves the task's `depends` depth
  first, and runs each task's `run` with `sh -c` in its `dir`, in the
  sandbox. That gives no more than a `build.rs` already can. A task outside
  the list, or one whose `depends` reach outside it, is refused. `mise run ci`
  is refused with "use `bench ci`": the gate runs the full set;
- `cargo <sub> …`, for `<sub>` in `fmt`, `check`, `clippy`, `build`, `test`,
  `tree`, `metadata`, refusing `--config`, `-Z`, `--manifest-path` and
  `--target-dir`;
- `pigeon cobot validate|render|scope …`, and `pigeon backplane decisions …`;
- `git diff|log|status …`, read-only.

The environment it gets is fixed:

- `CHANGE_BASE=artifacts/main`, since the bench fetches `main` before each
  run;
- `CARGO_NET_OFFLINE=true`;
- `CARGO_TERM_COLOR=never`;
- `RUST_BACKTRACE=1`.

The pure check is `core::bench::allow(argv) -> Result<Command, Refusal>`.

**Why an allowlist, and why it isn't the boundary.** A session controls the
commit, so a `build.rs` or a test runs its code anyway. The allowlist keeps
the operation honest about what benches are for: no daemons, no package
installs, no `pigeon cobot plan` (which needs state and its passphrase). The
security boundary is §5's sandbox, and the fact that the Container holds
nothing worth stealing.

#### 3.2 What a run does, in the Container

The job server, started with `--bench`, serves `/v0/bench/*` beside
`/v0/jobs`; a `Checker` or `Runner` doesn't serve these routes. For each run:

1. **Sync (as the server, with the network).**
   - `git fetch` of the scratch ref's `sha` and of `main`, with a read token
     the Worker minted for this run. The token is passed per command and never
     written to `.git/config`.
   - `git checkout --force <sha>` in the bench's one long-lived checkout,
     `/work/pigeon`. The target directory is the image's `CARGO_TARGET_DIR`,
     `/cache/target`, which already holds the warmed dependencies, so it
     stays warm across syncs.
   - `cargo fetch --locked`, when `Cargo.lock` changed since the last sync.
2. **Disk guard.**
   - When `/cache/target` is above `TARGET_CEILING` (10 GB) or the free disk
     is below 3 GB, the bench runs `cargo clean -p` for each workspace crate.
     The image's warmed dependencies in the same directory are kept.
   - The run's report says that it cleaned.
3. **Run (in the sandbox, without the network).** See §5.
4. **Report.** The server records the exit code, the time and the disk used.
   It reports the end to the Worker, which stores the log and appends
   `bench.finished` to the session's events (`wake=agent`).

#### 3.3 Fan-out: `bench ci`

- **`pigeon session bench ci`** pushes once, then asks bench 0 for
  `pigeon cobot scope --json` against `artifacts/main`: the checks the gate
  will select.
- **It deals those checks across the benches the session may hold**, longest
  first, by `core::bench::deal(checks, benches)`:
  - fmt and clippies on bench 0;
  - tests on bench 1;
  - wasm on bench 2.
- **It streams each check's result**, and exits non-zero when any fails.
- With one bench, it runs `mise run ci-scoped` there.

### 4. Lifecycle

- **Created on first use**, by `bench.run` or `bench.warm`.
  - The `SessionStart` hook (`pigeon session brief`) calls `bench.warm` on
    bench 0, in the background, for an executor whose brief's scope touches
    `src/pigeon-cli/**`.
  - Planners and executors without Rust stay lazy: many sessions never
    build.
- **Kept warm while used.**
  - The Container's `sleepAfter` is the idle timeout: 30 minutes by default,
    and the owner may set 5 to 120 (§6).
  - A run in progress holds the Container, as a job's does.
  - A Container's disk doesn't survive sleep. After the idle timeout, the
    next run starts from the image's dependency layer, as a `Checker` job
    does today.
- **Reset on demand:** `bench.reset` (§3).
- **Torn down with the ticket.** Benches belong to the ticket, not to a
  change. Each `Bench` object's alarm runs every 5 minutes while its
  Container lives. It destroys the Container and clears its storage when:
  - the label's ticket is not live: ended by `session.end`, lapsed after 24
    hours, revoked by `replace`, or revoked by an eviction that revokes it;
  - or the object has been idle past the timeout, and its Container is
    stopped.

  `session.end` also stops the label's benches at once, and deletes the
  scratch ref.
- **Ties into the archive ADR.**
  [`2026-11-15-development-archive-cleanly-ended-sessions.md`](2026-11-15-development-archive-cleanly-ended-sessions.md)
  judges a clean end by the changes a session holds. Benches are never
  "held", so they never make an end unclean. The end tears them down before
  the archiver acts, and an archived session has no bench. A change evicted
  under
  [`2026-11-10-development-change-slot-eviction-policy.md`](2026-11-10-development-change-slot-eviction-policy.md)
  leaves the session's benches alone while its ticket is live.

### 5. Isolation and security

- **What a bench Container holds.**
  - Its own signing key: `HMAC-SHA256(BACKPLANE_TOKEN, "bench:" + name)`,
    derived by the Worker. That key opens only this bench's server, so a leak
    reaches nothing else.
  - The build token `pigeon-start` uses, unset before `pigeon` runs, as on
    every job Container.
  - A read token per sync, minted for the repository, living 5 minutes.
- **What it never holds:**
  - `BACKPLANE_TOKEN` itself;
  - any write token;
  - `infra/` break-glass keys;
  - other workloads' secrets, `WORKERS_DEPLOY_TOKEN`, state passphrases or
    GitHub credentials.

  Nothing from `secrets.config.yaml` reaches it, and no secret enters the
  repository.
- **The sandbox.** Every allowlisted command runs:
  - as `nonroot`'s subordinate uid, in new user, network, PID, IPC and mount
    namespaces (`unshare` through `newuidmap`, as the image's BuildKit
    already does);
  - with the environment cleared, then set to §3.1's;
  - with only `/work/pigeon`, `/cache/target` and a private `/tmp` writable,
    and the cargo and rustup homes read-only;
  - with those two directories owned by the sandbox uid. The server chowns
    them once, from inside the namespace, when the bench starts, and runs
    its own git and `cargo fetch` as that uid too, without the network
    namespace. Step 2's spike settles the exact uid mapping, and an
    amendment records it.

  So the command:
  - can't read the server's `/proc/<pid>/environ`, its key or a git token;
  - leaves nothing running: its PID namespace ends with it, so no process it
    started can watch the next sync's token;
  - has no network at all: only a loopback in its own namespace;
  - can't reach the Worker, another Container or the state repository
    `noisypigeon/pigeon-cobot-state`, for which no credential exists here.
- **Egress, as the server.**
  - The server alone fetches: the Artifacts remote with a minted token, and
    crates.io's index and static hosts for `cargo fetch --locked`.
  - The Container keeps `enableInternet`, as job Containers do today, so the
    server can fetch. Cloudflare's per-Container egress filtering isn't relied
    on.
  - The untrusted part, the command, has no network.
- **Between sessions.**
  - Containers take no inbound traffic. Only the Worker reaches a `Bench`
    object, by a name it derives from a live ticket (§1).
  - Two sessions never share a Container, a checkout or a `target/`.
- **The gate stays independent** (option (b) is refused):
  - no gate job reads a bench, its logs or its cache;
  - the gate runs on GitHub Actions or a clean `Checker`, as before.

### 6. Capacity and cost

- **Per session: 3 benches** (`MAX_BENCHES_PER_SESSION`, a constant).
- **Per instance:**
  - `Bench` has `max_instances: 32` in `worker.yaml`, the hard ceiling;
  - `DEFAULT_MAX_BENCHES` is 24, the cap the control plane enforces.

  At the open-change cap of 50 and at most 5 changes per session, the
  instance has at least 10 sessions with changes at the cap. 24 benches give
  each a bench and room to fan out.
- **Admission** (`core::bench::admit`, pure, in the `Instance`):
  - A session's bench 0 is admitted ahead of any fan-out bench.
  - At the cap, a bench-0 request reclaims the longest-idle fan-out bench of
    another session, and destroys it.
  - With no fan-out bench to reclaim, it answers
    `wait: bench capacity, <n> ahead`, in a FIFO.
  - A fan-out request at the cap is refused with `wait: bench capacity`, and
    `bench ci` falls back to fewer benches.

  So every live session gets one bench while live sessions are at most the
  cap.
- **The owner controls:**
  - the cap, as `max_benches` in `limits.set` once it lands, at most 32;
  - the idle timeout, as `bench_idle_minutes` there, 5 to 120;
  - a daily budget, `bench_hours`, in Container-hours: 80 by default.

  Until `limits.set` lands, the defaults hold. Changing them before then takes
  an amendment here.
- **Metering.**
  - The `Instance` adds each bench's Container-seconds as it stops or every 5
    minutes.
  - At 100% of `bench_hours` for the UTC day, a new bench beyond a session's
    bench 0 is refused, and the control plane files one `owner_ask`
    (`kind: action`) for the day. Running benches finish.
  - The digest shows the day's hours, peak benches and reclaims.
  - This is the capacity-pools ADR's hard cap applied to Container spend:
    a refusal at a line the owner set.
- **Cost, estimated.** These are Cloudflare's Containers list prices, to be
  confirmed against the account's billing during verification:
  - $0.000020 per vCPU-second of active CPU;
  - $0.0000025 per GiB-second of memory, provisioned while running;
  - $0.00000007 per GB-second of disk.

  For `standard-4` (4 vCPU, 12 GiB, 20 GB):

  | Bench state | Per hour |
  |---|---|
  | Building at full CPU | about $0.40 |
  | Warm and idle | about $0.11 (memory and disk) |
  | Asleep | $0 |

  - A session-hour at a quarter busy costs about $0.18.
  - Ten sessions for 8 hours a day cost about $15 a day.
  - The default `bench_hours` of 80 bounds bench time at about $32 a day,
    even at full CPU.

### 7. Deployment

- **Through the pipeline only.**
  - `worker.yaml` gains the `Bench` Container class and its Durable Object
    migration.
  - It deploys through `MainRun` and `pigeon cobot workers`, never by hand.
  - There's no `tofu apply` and no hand apply against
    `noisypigeon/pigeon-cobot-state`.
- **No image change is needed.**
  - The job server's bench routes arrive with `pigeon` from builds.
  - `uidmap` and the subordinate ranges are already in the image.
- **The new class needs its image reference.** If the first deploy reports
  a missing image for `control/Bench`, `deploy_retry` deploys again, which
  builds and pushes for every class, as `CLAUDE.md` describes.
- **No new secret.** The bench key is derived from `BACKPLANE_TOKEN` in the
  Worker, and tokens are minted.
- **`tests/architecture.rs`** gains the `Bench` binding, the
  `/v0/bench/*` routes and the crates.io hosts, per
  [`2026-11-15-runtime-architecture-coverage-agents-deploys.md`](2026-11-15-runtime-architecture-coverage-agents-deploys.md).

### 8. Migration and fallback

- **`CLAUDE.md`'s Commands section.**
  - "`mise run ci` must pass before every change is pushed" becomes "on a
    bench": `sh tooling/session/session.sh bench ci`, or
    `bench run -- mise run ci-scoped`.
  - The section names the `bench_*` tools.
- **A `PreToolUse` hook, `pigeon session guard`,** refuses a Bash command
  that starts with any of:
  - `cargo build`, `cargo test`, `cargo clippy`, `cargo check`;
  - `mise run` other than `fmt`;
  - `cargo nextest`.

  Its message names the bench command to use instead. `cargo fmt` and
  `mise run fmt` stay local: they're cheap and need no build.
- **The executor brief template's `checks`** run through `bench run`, and
  the brief validator refuses a check that's a local build.
- **When the pool is down**, meaning `bench.*` answers `unavailable` or a
  bench-0 request waits more than 10 minutes:
  - the session runs only `cargo fmt --all -- --check` locally, since
    `sentinel-change` builds `pigeon-sentinel`;
  - it pushes, and relies on the bound job for the policy and the gate for
    the checks;
  - it says so in the change's note (`change_note`).

  It never builds locally. The hook allows `cargo fmt` always.

### Execution

Each step is its own change, small and serial where steps share files.
Executors are Sonnet unless marked.

1. **This ADR.** (planner)
2. **Spike, as a check** (`tooling/spikes/bench-sandbox/run.sh`, read, then
   closed). On a `Checker` it runs `unshare` with `newuidmap` and no network,
   then `cargo fetch --locked` outside the sandbox and `mise run ci-scoped`
   and `cargo test --workspace --no-run` inside it. It reports the times,
   `target/` size, free disk, and anything that wanted the network. (brief)
3. **`core::bench`**: names, `allow`, `admit`, `deal`, the disk guard's
   decision and metering, all pure, with tests. (brief)
4. **Scratch refs in the front door** (§2), with tests. (brief)
5. **The job server's bench routes** (§3.2) and the sandbox (§5), behind
   `--bench`. (planner: it sets the sandbox's boundary)
6. **The Worker:**
   - the `Bench` object, its `worker.yaml` class and migration, and the
     derived key;
   - the `bench.*` operations and tools;
   - teardown with the ticket;
   - logs to R2;
   - `tests/architecture.rs`.

   Split as the code requires. (planner, guarded paths: the owner approves)
7. **`pigeon session bench`** (§2, §3, §3.3), and `bench.warm` in the
   `SessionStart` hook. (brief)
8. **`CLAUDE.md`, the brief template's checks, and `pigeon session guard`**
   (§8). (planner)
9. **Verify live** (below), and record it here as an amendment. (planner)

### Verification

- **A session's `bench ci` runs entirely in benches.** The Claude
  container's disk use, measured with `du -sx` of its home and workspace and
  `df` before and after, stays flat within 100 MB.
- **Three sessions at once.** One runs `bench ci` across three benches
  while two others run `mise run ci-scoped` on their bench 0. The wall times
  of each, the cold and warm runs, and the peak benches are recorded.
- **Warm is faster.** A second `ci-scoped` after a one-file change, within
  the idle timeout, takes well under the cold run.
- **Isolation.** A test that reads `/proc/1/environ`, opens a socket to
  1.1.1.1, or pushes to the Artifacts remote fails inside the sandbox.
  `bench.run` with another session's ticket is refused.
- **Teardown.** After `session_end`, `bench.list` answers nothing, the
  scratch ref is gone, and the Containers are stopped within 5 minutes.
- **The gate is untouched.** A change's checks still run on GitHub Actions or
  `Checker`, and no gate job reads a bench.
- **Cost.** The day's metered Container-hours are compared with the
  account's billing, and §6's estimate is amended with the measured figure.

## Amendments

### 2026-11-16: Reviewed against the one-test-binary ADR (!659); bench ci runs scope's checks by name

Reviews !658 (step 3, `core::bench`) against
[`2026-11-16-development-one-test-binary-per-crate.md`](2026-11-16-development-one-test-binary-per-crate.md),
[`2026-11-15-development-code-checks-by-reach.md`](2026-11-15-development-code-checks-by-reach.md)
and
[`2026-11-10-development-factory-crates-functional-core.md`](2026-11-10-development-factory-crates-functional-core.md).

- **What agrees.**
  - !658 adds only `src/` code. Its tests are unit tests in its own files, run
    by backplane's `--lib` binary. It adds no file under `tests/` and no test
    target.
  - `core::bench::deal`'s lanes key on a check's name starting with `test`.
    The one-test-binary ADR §3 keeps scope's test job names as `test …`,
    listing modules instead of targets, so the lanes hold.
  - That ADR §5 keeps `-p` per test job, so one feature set per job. Since
    `bench ci` runs exactly scope's jobs, a bench builds what the gate
    builds. It adds no feature variant beyond the gate's.
- **What doesn't, and what !658 changes.**
  - The one-test-binary ADR §3 makes a test job's argv
    `sh -c "cargo test … && cargo test …"`, and §5 sets `PIGEON_SENTINEL`
    after a `cargo build -p pigeon-sentinel` inside it. §3.1's allowlist
    refuses `sh`, so `bench ci` couldn't run the gate's own test jobs.
  - So `allow` gains a third form: `bench check <name>` →
    `Command::Check { name }`.
    - The server runs `cargo run -q --locked -p pigeon -- cobot scope --root
      ../.. --base artifacts/main --json` at the synced commit, in the
      sandbox.
    - It then runs the check of that name with the argv scope gave it, in
      the sandbox.
    - An unknown name fails the run with "bench: no check `<name>` in this
      change's scope".
  - This trusts the commit as far as `build.rs` or a mise task (§3.1)
    already does. The sandbox stays the boundary.
  - `bench ci` (§3.3) deals `bench check <name>` commands, not raw argv.
  - Brief b52 gains this form as revision 3, before !658 merges.
- **Disk, from the one-test-binary ADR §6.**
  - A workspace target is 16 GB after one build today, more than
    `TARGET_CEILING`'s 10 GB, and near a Container's 15 to 18 GB free. Until
    that ADR's step 4 lands, a bench running the full test set will clean
    its workspace crates on most runs.
  - After step 4, about 4 to 5 GB, so benches stay warm.
  - The spike (!657) measures the bench's side.
  - `TARGET_CEILING` stays 10 GB, and is revisited with the spike's numbers.

### 2026-11-16: The sandbox spike's findings: `--map-auto` with uid 1000, no network, and a 3.7 GB full test target

The spike (!657, `tooling/spikes/bench-sandbox/run.sh`) ran as a check on a
`Checker`, then was closed, per step 2.

- **The Container:**
  - `6.18.56-cloudflare-microvm`, 4 CPUs, 12 GB of memory;
  - user 65532, util-linux 2.38.1, `newuidmap`, and `nonroot:100000:65536`;
  - no `mise`;
  - 14.3 GB free of 18.3 GB;
  - `/cache/target` owned by 65532, at 1.5 GB of warmed dependencies.
- **The sandbox works.**
  - `unshare --user --map-root-user --map-auto --net --pid --fork
    --mount-proc --kill-child` maps 0 to 65532, and 1 to 65535 to 100000
    and up.
  - Inside it, root can `chown` to 1000.
  - As uid 1000 (`setpriv --reuid=1000 --regid=1000 --clear-groups`), a
    command:
    - writes its own directory;
    - is refused the parent's `/proc/<pid>/environ`;
    - sees only `lo` and unconfigured links, with 1.1.1.1 unreachable.
- **So §5's sandbox is settled:**
  - The server runs every command through that `unshare`, then `setpriv` to
    uid 1000.
  - When the bench starts, the server chowns `/work/pigeon` and
    `/cache/target` to 1000 from inside the namespace.
  - Its own `git` and `cargo fetch` run as uid 1000 through `unshare` without
    `--net`, so the files stay that uid's. The token is passed only to those
    processes.
  - `--map-root-user` alone is refused for commands: its root is 65532
    outside, the server's own uid.
- **Builds, offline in the sandbox, from the image's warm dependencies:**

  | Command | Time | `/cache/target` |
  |---|---|---|
  | `cargo fetch --locked`, outside | 2 s | |
  | `cargo clippy --workspace --all-targets -D warnings` | 22 s | 1.5 → 1.8 GB |
  | `cargo test --workspace --no-run` (119 test binaries) | 50 s | 1.8 → 3.7 GB |
  | wasm clippy (warm in the image) | 0 s | |
  | `cargo fmt --all --check` | 2 s | |

  Nothing asked for the network once fetched.
- **Disk.** The image sets `CARGO_PROFILE_DEV_DEBUG=0`, so a full test target
  is about 3.7 GB here, not the 16 GB of a debug build. The one-test-binary
  ADR's figure is for local targets with debug info.
  - `TARGET_CEILING` stays 10 GB.
  - A bench stays warm across runs today, without waiting for that ADR.
- **Not measured, left to step 5's verification:**
  - the `chown` of `/cache/target` at bench start;
  - a build as uid 1000 rather than as the namespace's root.

### 2026-11-16: Step 5 as built: `cobot::bench`, and where it differs from §3.2 and §5

`cobot::bench` (!663) lives in the shell, per `.config/policies/backplane.yaml`. It
is served by `pigeon backplane jobs --listen ADDR --work DIR --bench`.

- **Routes.** Every route is a `POST` with a JSON body, since the signature
  covers the path:
  - `/v0/bench/run` {`run`, `remote`, `token`, `ref`, `sha`, `base`,
    `command`, `timeout`};
  - `/v0/bench/status` {`run`, `offset`, `wait`};
  - `/v0/bench/cancel`;
  - `/v0/bench/reset`;
  - `/v0/bench/info`.
- **`--work` must lie outside `/tmp`.** The sandbox mounts its own `/tmp`. The
  `Bench` object passes `/cache`, so the checkout is `/cache/bench/pigeon`.
- **The cargo home belongs to uid 1000 too**, beside the checkout and
  `/cache/target`. `cargo fetch` writes it, so it isn't read-only for
  commands as §5 said.
  - It holds the public crates already in the image, and nothing secret.
- **git runs with `core.hooksPath=/dev/null` and `core.fsmonitor=false`.**
  A command can write `.git`, and the next sync's fetch runs as the same
  uid with the network and the run's token. Hooks and fsmonitor are the code
  paths that would run there.
  - A command could still plant `.git/config` that sends that token
    elsewhere. The token is a 5-minute read token for this public
    repository, minted per run, so this is accepted.
- **The Worker polls the run, rather than the server reporting its end**
  (§3.2 step 4). The server holds no credential for the Worker. The `Bench`
  object's alarm reads `/v0/bench/status` until the run is done, then stores
  the log and appends `bench.finished`.
- **The tests drive the whole flow with the sandbox off** (`Sandbox::Plain`),
  against a local bare repository: sync, mise tasks, exit codes, one run at a
  time, cancel, paging and refusals. The sandbox's wrapping is tested as
  argv. The sandbox itself is proved live in step 9.

### 2026-11-16: Step 6 as built: the `Bench` object, the fleet, `bench.*`, and teardown

One change carries step 6 (!684). It re-proposes !675, which was closed to free
the merge turn after a stale test map failed its gate, and which itself replaced
!671, whose approved scope lacked `tests/instance.rs`, the pin of every
operation that changes something.

- **The `Bench` object** is named `<session_id>.<k>`.
  - It starts its Container as `jobs --work /cache --bench`, under
    `link::derive(BACKPLANE_TOKEN, "bench:<name>")`, held by its monitor.
  - It forwards `/v0/bench/*`.
  - Its alarm, every 5 minutes, meters its seconds. It stops the bench when
    the label's ticket isn't live, or when it has been idle past its timeout
    with no run going.
- **The fleet** is kept by the `Instance` at
  `/benches/{admit,touch,release,meter,asked,end,list}`, under
  `Limits::default()` until `limits.set` carries `max_benches` and
  `bench_hours`.
- **A finished run's log goes to R2 when it is first read as ended.** That is
  usually `bench run`'s own last `bench.status`, which also appends
  `bench.finished`. A run nobody reads after it ends keeps its log only in the
  Container, until the bench stops. That's the session's own loss, and
  `bench run` always reads to the end.
- **The day's budget files one owner item** (`owner.ask`, `kind: action`), on
  the first refusal of the UTC day. It is filed by the control plane, as the
  refused session.
- **`session.end` stops the session's benches.** When the session ever had a
  bench, it also dispatches an `unscratch` job on `Runner`, with a write
  token minted for it. The job deletes `refs/heads/scratch/<session_id>` and
  refuses any other ref. A lapsed ticket's benches stop through their alarm,
  and its scratch ref stays until the session's next `session.end`. A sweep
  of such refs is left for later.
### 2026-11-16: Governs globs match the old and the new crate layout

Per [`2026-11-16-development-src-crates-neutral-names.md`](2026-11-16-development-src-crates-neutral-names.md) §6, each
glob under `src/pigeon-cli/` or `src/site-templates/` becomes `src/**/<tail>`
(or `**<file>` for a workspace root file), which matches the file both before
and after the restructure moves it. That ADR's step 12 tightens them to the
new paths.
