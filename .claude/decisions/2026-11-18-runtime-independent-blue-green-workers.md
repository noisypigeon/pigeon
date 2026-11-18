## Independent blue and green Workers over shared truth, deploys per unit, and tests that don't grow with features

* Date: 2026-11-18
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: runtime
* Governs: `infra/pigeon/workers/**`, `catalog/pigeon/kinds/platform/**`, `config.yaml`, `src/**/worker/main_run.rs`, `src/**/worker/platform.rs`, `src/**/deploy/**`, `src/**/ship.rs`, `src/**/checks/scope.rs`, `src/**/migrations/**`, `src/**/wire/registry_tests.rs`, `src/**/tests/suite/instance/architecture.rs`, `src/**/tests/suite/instance/instance.rs`, `.config/policies/change.yaml`

## Context

The owner, 2026-11-18: "Currently we don't really have a real blue green
deployment … Please create independent workers pigeon-green and pigeon-blue.
Each has its own Instance, ChangeRun, MainRun and R2. They share the D1 DB
and Secrets Store, Artifacts and AI Gateway, unless any make sense not to.
Blue should run all verifications before it replaces green as the primary
entry point." They also asked for deploys that don't redo everything, and
for far fewer tests that grow with every feature, with a guard on new ones.

### 1. What "blue-green" is today

- **One Worker, two versions.** `pigeon-control`
  (`infra/pigeon/workers/control/worker.yaml`) is uploaded as a new version
  at 0%, smoked through a version override, then moved to 100%
  ([`2026-11-08-runtime-blue-green-self-standup.md`](2026-11-08-runtime-blue-green-self-standup.md)
  §2 and its spike amendment).
- **Both versions share everything but code:** all seven Durable Object
  namespaces (`Instance`, `ChangeRun`, `Checker`, `Runner`, `MainRun`,
  `Bench`, `Pool`), their Containers, D1, R2, the queue, Access, the cron.
- **So it breaks both ways:**
  - at 0%, the smoke reaches the *old* version's objects, since objects
    follow the deployment, not the override; the new objects are untested;
  - at 100%, every object flips at once; a bad object version is found only
    once it serves;
  - a class, migration or Container change is a whole deploy, past which
    there is no rollback;
  - `MainRun` deploys the code that replaces itself (the 2026-11-13
    stale-slot bug).
- **The design already decided, not built.**
  [`2026-11-12-development-grants-applies-rescue-yield.md`](2026-11-12-development-grants-applies-rescue-yield.md)
  §7 settled "two warm colors over shared truth": a D1 `journal`, a fenced
  `lease`, `pigeon-front` on `platform.pigeon.dev`, and `platform/shared`.
  Its steps 11 to 15 never ran. This ADR carries it out, with the owner's
  constraints where they differ (§A.2).

### 2. What a deploy redoes today

- One baseline for everything: each unit diffs from the last deploy that
  *passed* (`control.rs`, `Pipeline`). One failed site fails the whole
  deploy and makes every unit redo its diff; `deploy_retry` after a pass
  clears the baseline and redoes everything.
- The Worker's inputs are the declared dependency closure of
  `pigeon-agent` (agent, core, common, eviction) plus `Cargo.lock`,
  `mise.toml` and `.config/releases.yaml` (`checks/scope.rs` `workers()`),
  not what the wasm compiles.
- The stored build rebuilds `pigeon` and the judge together on any `RUST`
  path, which still lists the old `src/pigeon-cli/` layout
  (`deploy/scope/mod.rs`).
- An image change rebuilds every Container class, though Checker, Runner
  and Bench share one Dockerfile target.
- The architecture site publishes on every deploy, only to show the
  deployed SHA.

### 3. Tests that grow with features

1936 test functions in 199 files. Five places pin a hand-written count or
name list of operations or routes, so each new tool edits two or three of
them. !1103 was re-proposed as !1106 for missing one (`instance.rs`'s list
of writes) from its scope. `registry_tests.rs` was edited by 7 of the last
50 commits.

## Implementation plan.

### A. Two independent Workers

#### A.1 What each color owns, and what is shared

| Per color: `pigeon-blue`, `pigeon-green` | Shared |
|---|---|
| its Worker and every Durable Object class: `Instance`, `ChangeRun`, `MainRun`, `Checker`, `Runner`, `Bench`, `Pool` | D1 `pigeon`: the journal, the lease, `changes`, `jobs` |
| its Containers, which belong to its objects | Secrets Store (`granter`) |
| its CI logs bucket, `pigeon-ci-<color>`, binding the other's read-only | Artifacts and its repositories |
| its queue, `pigeon-ci-events-<color>`, since a queue has one consumer | AI Gateway `pigeon-models` |
| its hostname, `<color>.platform.pigeon.dev` | the state and plans buckets, the docs bucket |
| its cron, alarms and in-memory caches | Access: one application, the four service tokens |
| | `pigeon-front`, on `platform.pigeon.dev` and `*.pigeon.dev/*` |

Why each shared resource stays shared:

- **D1** holds the truth both colors read. Two databases would need
  replication; the lease (A.3) decides who writes acts.
- **Secrets Store, Artifacts and AI Gateway** are external services the
  colors call, not state they own. Splitting them buys no isolation.
- **The state and plans buckets** hold OpenTofu state. One per color would
  let two colors apply the same unit from different state.
- **Access and its service tokens stay one.** Per-color tokens, as §7
  planned, would mint four new credentials and change what the iOS app,
  the deployer and the pool hold. No token is created or rotated by this
  ADR. The one application lists both color hostnames.
- **The pool's ledger moves to the journal.** The hard spend cap
  ([`2026-11-15-development-capacity-pools-hard-spend-cap.md`](2026-11-15-development-capacity-pools-hard-spend-cap.md))
  must hold across a flip; two ledgers would each allow the full cap.

#### A.2 Where this departs from grants §7

- Workers are `pigeon-blue` and `pigeon-green`, as the owner named them.
- Access and service tokens stay shared (A.1). AI Gateway is shared.
- The pool ledger is journaled.
- Everything else in §7 stands: the journal, replay, the daily replay
  check, the lease and its fencing, each color's queue subscribed to
  Artifacts' push events, `pigeon-front`.

#### A.3 One acting color

- **The lease**, D1's one row `lease (color, epoch, state)`, names the
  primary. Every act is fenced by the epoch it was started under: a merge,
  a deploy, a push to Artifacts or the mirror, a platform run, a pool
  dispatch, a job start, a Web Push or APNs send, an event-log append.
- **Fencing is in the write.** An act's journal append is a D1 batch whose
  first statement checks the epoch; a stale epoch fails the batch. A color
  that lost the lease can't act, even from a late alarm.
- **Facts are not acts.** A job that finishes after a flip appends its
  result unique on the job's ID, under any epoch, so no work is lost.
- **The standby only reads.** Its cron and alarms check the lease first and
  return. Its objects replay the journal on each call, so it stays warm.
- **Every color's `MainRun` deploys only the other color.** No Worker
  replaces its own code.

#### A.4 The front, and promotion

- **`pigeon-front`** is a small Worker on `platform.pigeon.dev` and
  `*.pigeon.dev/*`, with a service binding to each color. It forwards each
  request to the lease's color, read from D1 and held for at most 5
  seconds. It has no state of its own and changes rarely.
- **A Worker deploy goes to the standby:**
  1. D1 migrations, expand-only (`.config/policies/migrations.yaml`); the
     primary ignores what it doesn't know.
  2. `wrangler deploy` of the standby, Containers included. The primary is
     untouched.
  3. **Verify the standby, on its own objects**, through its hostname:
     - `/healthz`: D1, every object answering at the new version;
     - `/healthz/smoke`, now on the standby's own objects and a no-op job
       on its own Checker;
     - replay parity: the standby's objects, replayed to the journal's
       head, hash as the primary's do;
     - the inventory: its routes, tools and bindings
       (`architecture.describe`) equal the primary's, plus what the
       change's scope adds, minus what it removes.
  4. **Promote:** the lease goes to `promoting`; the primary drains (no new
     acts; running jobs finish, at most 10 minutes); the lease moves to the
     standby at `epoch + 1`, `serving`. The front follows within 5 seconds.
  5. **Watch:** 10 minutes of `/healthz` on the new primary. A failure flips
     the lease back, which is instant: the old color is warm and every
     migration was expand-only.
- **The old primary stays warm** at the old version until the next Worker
  deploy, so a rollback is a lease flip, also open to the owner as
  `lease.flip` in `pigeon-rescue`.
- **A deploy that changes no Worker input never flips** (B).

#### A.5 Cutover from `pigeon-control`, without losing in-flight work

Each step is a change; each infra step is a plan the owner applies. No
`tofu apply` by hand.

1. **Prerequisite:** the pool token is adopted unrotated (!1108), and a
   platform/blue plan shows no change to it. Until then, no platform/blue
   apply.
2. **Journal in shadow** on `pigeon-control`: every object appends, then
   applies; the event log, tickets, tasks, merges, mandates, lanes, owner
   items and the pool ledger are journaled; existing state is backfilled as
   one snapshot entry per object. The daily replay check runs. Nothing
   reads the journal yet.
3. **The lease and fencing**, with `pigeon-control` as the only color, so
   nothing changes in behavior.
4. **`pigeon-front`**, first on a test hostname forwarding to
   `pigeon-control`, then given `platform.pigeon.dev` and `*.pigeon.dev/*`;
   `pigeon-control` keeps `control.platform.pigeon.dev`, and `PUBLIC_HOST`
   becomes `platform.pigeon.dev`.
5. **Units:** `platform/blue` keeps today's shared resources, renaming
   nothing; a new `pigeon/color` kind declares `color/green` and
   `color/blue`. Renaming `platform/blue` to `platform/shared` is a later
   state move with no resource change.
6. **Stand up `pigeon-green`:** its Worker, objects, Containers, CI bucket
   and queue; its Worker secrets are put by the owner (an owner action, as
   the values are theirs). It replays the journal and runs A.4's
   verifications.
7. **Rehearse:** flip `pigeon-control` → `pigeon-green` and back. Then flip
   for real. In-flight changes rebuild by replay; sessions keep their
   tickets and event cursors, which are in D1; jobs still running on
   `pigeon-control` finish and append their results.
8. **Stand up `pigeon-blue`** the same way, as the standby. From here every
   Worker deploy is A.4.
9. **Retire `pigeon-control`:** kept drained for 7 days as a cold fallback,
   then torn down by the owner (destructive, asked where it happens).

### B. Deploys per unit

- **One deploy plan.** `deploy::scope::plan(files) -> Vec<Unit>`, in
  `pigeon-core`, names every unit a set of paths deploys:
  `worker:<name>`, `site:<name>`, `build:pigeon`, `build:judge`,
  `image:<target>`, `ios`. `pigeon dev scope --deploys` and
  `change.scope` print it, so a change shows what it will deploy.
- **A baseline per unit.** The pipeline keeps, per unit, the SHA it last
  deployed. Each unit diffs from its own baseline, passes or fails on its
  own, and advances only on its own pass. `deploy_retry` takes `unit`, and
  without it retries only the units that failed.
- **Inputs are what the artifact compiles:**
  - the Worker's are its crate's closure for `wasm32` with `--features
    worker`, from `cargo metadata`, not the declared closure;
  - `Cargo.lock` counts only for crates whose resolved dependencies
    changed, as
    [`2026-11-02-release-crate-units-and-images.md`](2026-11-02-release-crate-units-and-images.md)
    decided for releases;
  - `mise.toml` and `.config/releases.yaml` leave the Worker's inputs.
- **The build splits:** `build:pigeon` on the harness's closure,
  `build:judge` on the eviction closure, each stored with its own SHA. The
  stale `src/pigeon-cli/` entries go.
- **One image per target:** Checker, Runner and Bench share target `ci`, so
  it builds and pushes once. A Worker deploy carrying new images is still a
  standby deploy (A.4); it is no longer a "whole deploy" exception.
- **The architecture site** publishes when its inputs change. The deployed
  SHA it shows is read from the primary's `/healthz` at view time.
- So a change to `src/pigeon-harness` alone deploys `build:pigeon`; to
  `src/pigeon-site-templates`, the sites using it; to one site's content,
  that site.

### C. Lean tests

#### C.1 What goes, and what replaces it

Removal is the owner's call per batch. Batch 1:

| Test | Replaced by |
|---|---|
| `wire/registry_tests.rs` `every_view_count_is_pinned` | nothing: `writes_are_cards_or_details` and `present::PENDING == 0` already hold the rule |
| `wire/registry_tests.rs` `the_views_are_as_decided` (36 names) | derived: List iff the result has an array of items, else Detail |
| `instance/instance.rs` `catalog_names_are_unique_and_dotted`'s 49-entry list of writes | every write declares an effect; the short list of `Apply`-effect operations stays, as security-relevant |
| `instance/instance.rs` seven `the_control_plane_runs_*_itself` lists | one test: every operation in a control-plane domain is `control_plane()`, by prefix |
| `instance/instance.rs` `the_control_plane_runs_benches_itself`'s count of 7 | its loop, without the count |
| `instance/architecture.rs` `the_route_inventory_is_pinned` (55 routes) | every non-MCP route is a shim of a catalog operation or on a short exempt list; coverage stays in `every_route_binding_and_outbound_host_is_in_the_network` |
| `provider/registry.rs` `exempt.len() == 10` | `every_exemption_names_code_evidence` |
| `present/{rest,change,approval}_tests.rs` per-operation fixtures (about 86) | a sample generated from each result's schema, checked by `checked()` for every presented name; hand fixtures only for money, links and the never-shown ticket |
| `eviction/tests/suite/policies.rs` one test per policy (23) | `examples: {pass, fail}` in each policy, run by one test |

Each removal records its line in `tooling/ci/tests.moves`, as
[`2026-11-16-development-one-test-binary-per-crate.md`](2026-11-16-development-one-test-binary-per-crate.md)
§4 requires.

#### C.2 The guard

- **`.config/policies/tests.yaml`**, target `change`: a change that adds a
  test function (`#[test]` or `#[tokio::test]`) carries a `Test-Reason:`
  trailer in its first commit, saying what the test guards that nothing
  else does.
- **Per-feature enumerations are refused:** an added test line that pins a
  count of operations, routes, tools or policies (`len()` or `count()`
  against a literal), or lists catalog names as literals, fails unless its
  `Test-Reason:` starts `enumeration:` with why.
- **Mechanics:** `PrField` gains `added_tests`, `enumerations` and
  `test_reason`; `bound` computes them from the head's diff, and `pigeon
  dev policy-change` from the branch's; the `change` target gains `when:`,
  as `provider-call` has.

### Execution steps

Each is a brief to a sonnet executor, after the owner's answers, except
step 1. Parts B and C don't wait on A.

1. **!1108**, `fix(infra)`: the pool token adopted unrotated (A.5.1). Then
   a platform/blue plan for the owner.
2. **`feat(eviction)`: the tests guard** (C.2).
3. **`test`: batch 1 removed and replaced** (C.1), after the owner's
   answer.
4. **`feat(deploy)`: the deploy plan** and `pigeon dev scope --deploys`
   (B).
5. **`feat(deploy)`: baselines and outcomes per unit**, `deploy_retry
   unit`.
6. **`feat(deploy)`: inputs narrowed**, the build split, one image per
   target, the architecture site's SHA at view time.
7. **`feat(core)`: the journal in shadow** (A.5.2).
8. **`feat(core)`: the lease and fencing** (A.5.3).
9. **`feat(infra)`: `pigeon-front`** (A.5.4), then the `pigeon/color` kind
   (A.5.5).
10. **`feat(deploy)`: deploy to the standby, verify, promote, watch, flip
    back** (A.4).
11. **Cutover**, A.5.6 to A.5.9, each by an owner-applied plan.

## Amendments

### 2026-11-18: !1110 agrees with this ADR

!1110 carries out
[`2026-11-15-development-capacity-pools-hard-spend-cap.md`](2026-11-15-development-capacity-pools-hard-spend-cap.md)'s
2026-11-18 amendment, and is step 1 here (A.5.1). The two agree:

- `pigeon-pool` moves to the `pigeon/platform` kind's new
  `adopted_service_tokens`, in its own module that leaves the secret as
  Cloudflare holds it. No token is created or rotated, as A.1 requires of
  Access and its tokens.
- That list stays on `platform/blue`, the unit A.5.5 keeps for the shared
  resources; the `pigeon/color` kind declares no service token.
- Nothing else here changes.

### 2026-11-18: A retired test is a `-` line in tests.moves; batch 1 runs in three parts

`behaviours.sh --check` accepts only renames: a test gone from the ledger
without a `tests.moves` line fails it, and so does a package that runs
fewer tests. The owner's decision #35 retires batch 1 (C.1), so:

- **A retired test is `<old path::name>\t-`** in `tests.moves`. `--check`
  counts it as accounted for and leaves it out of the package's count. Only
  a test this ADR's C.1, or a later batch the owner approves, names is
  retired this way; the planner checks each `-` line in review.
- **Batch 1 runs in three parts:**
  - **3a, first:** the pinned counts and name lists in `registry_tests.rs`,
    `instance.rs`, `architecture.rs`, `provider/registry.rs` and the
    presenters' `NAMES.len()`. The routes-to-MCP planner's new catalog
    operations wait on it.
  - **3b:** the per-operation presenter fixtures, replaced by
    schema-generated samples.
  - **3c:** the per-policy tests, replaced by examples in each policy.
- **The route inventory's pin goes in 3a; its shim invariant waits** until
  the routes-to-MCP removals reach their end list
  ([`2026-11-15-runtime-one-mcp-entry-point.md`](2026-11-15-runtime-one-mcp-entry-point.md)
  §7). Until then `every_route_binding_and_outbound_host_is_in_the_network`
  keeps coverage.
