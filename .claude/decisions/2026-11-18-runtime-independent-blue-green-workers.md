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

### 2026-11-18: Identity state is journaled, and an authorization check reads after replay

A.5 step 2's "every object appends, then applies" covers every `Instance`
key, so its list is examples, not a whitelist. Named so that other ADRs can
cite it: the identity state, which is `members` (with any capability grants
inside `core::member::Members`), `accounts`, the passkey ring and `devices`,
is journaled like the event log.

- **Order:** 7a is the event log. Then the rest of the `Instance` keys,
  identity included. Then `ChangeRun` and `MainRun`. Each object's first
  journaled release backfills it as one snapshot entry.
- **A grant or revoke is an act** (A.3): only the lease holder writes it,
  fenced by the epoch.
- **An authorization check reads identity state after replaying to the
  journal's head**, never from a value cached across calls. So a revoke
  that has committed answers on either color, including a request
  `pigeon-front`'s 5 s lease cache still routes to the old one. Step 10's
  replay parity checks it.

### 2026-11-18: 3b declares each operation's result first; 3c's policies carry their own examples

Two facts §C.1 relied on were wrong when 3b and 3c were briefed:

- **No operation declares its result.** `Operation` has `args` and `view`,
  and `Response.result` is untyped. So a sample can't be generated yet.
- **`policies.rs` isn't one test per policy.** Its 23 tests cover 13 of the
  27 policies. `Policy` refuses unknown fields, so a policy can't carry
  examples yet.

So:

- **3b, the presenters.** `Operation` gains `result`, the twin of `args`: the
  top-level fields of the operation's result, each with a `Kind`. MCP
  advertises it as the tool's `outputSchema`. One test generates a sample
  from it and runs it through `checked()` for every presented name. Each
  family moves in its own change, rest, then change, then approval: its
  operations declare `result`, and its per-operation fixture tests retire as
  `-` lines in `tests.moves`. The money, non-https link and never-shown
  ticket fixtures stay. `odd_shapes_still_validate` stays.
- **3c, the policies.** `Policy` gains `examples: {pass, fail}`, each a
  list of inputs for the policy's target. One test runs every policy's
  examples, and `every_policy_is_valid` requires each policy to carry at
  least one of each. Policies gain examples in batches, and a policy's
  tests in `policies.rs` retire as `-` lines once its examples cover them.
  The engine change is under `eviction`, and the policy files are guarded
  (`secrets*` is gate). So every 3c change waits for the owner's passkey,
  and nothing else waits on it.

### 2026-11-18: Step 9a's front: its own crate, service bindings, its own check, and only hosts Access covers

A.4 and A.5 step 4 name `pigeon-front` but not its shape. A Worker spec has
no service binding yet, and a deploy checks only `PUBLIC_HOST`. So:

- **Its own crate, `src/pigeon-front`**, depending on `worker`, `serde` and
  `serde_json` only. As a feature of `pigeon-agent` it would redeploy on every
  agent or core change, and A.4 says it changes rarely. It reads the lease
  with its own one-line query, not `pigeon-core`'s.
- **Service bindings in the spec:** `bindings.services` maps a binding name
  to a Worker's name, rendered as wrangler's `services`. The front binds one
  per color, named by the color upper-cased with `-` as `_`
  (`PIGEON_CONTROL`, later `PIGEON_GREEN`, `PIGEON_BLUE`).
- **Forwarding:** each request goes, unchanged, to the binding of the
  lease's `color`, whatever its `state`; promotion moves `color` at
  `epoch + 1`. The row is cached per isolate for at most 5 s. A missing row,
  or a color with no binding, answers 503: the front never guesses.
- **Its own check:** a front deploy is whole (it has no objects to drain),
  then checked with `GET /healthz` on its first custom domain, which reaches
  the primary through it. It never takes the version-override smoke, which
  names a Worker on `PUBLIC_HOST`.
- **Its hosts are the platform's:** the front forwards a request unchanged,
  so the primary sees the front's host. Today a `*.pigeon.dev` host that
  isn't `PASSKEY_RP_ID` or `PUBLIC_HOST` is a standalone's, proxied or a bare
  404, and `/builds/` answers before any identity check, trusting Access at
  the edge. So a host the front serves must be one of `platform/blue`'s
  `domains`: the Access application covers exactly those (its `hostnames` are
  `$config.domains`), and the primary takes each of them as its own host, not
  a standalone's. Its routes still claim only `domains[0]` and `domains[1]`.
- **First host:** `front-test.pigeon.dev`, appended to `domains` and served by
  the front as a `custom_domain`, which takes precedence over the primary's
  `*.pigeon.dev/*` route. Adding it changes the Access application, a plan the
  owner applies, and it lands before the front serves the host.
  `platform.pigeon.dev` and `*.pigeon.dev/*` move to the front in A.5 step 4's
  second half.
- The wasm clippy check's one `[workspace.metadata.pigeon].worker` becomes a
  list, so the front is linted for `wasm32` too.

Changes, in order: 9a.1 service bindings in the spec, and the primary taking
every one of `domains` as its own host; 9a.2 `front-test.pigeon.dev` in
`domains`, whose Access plan the owner applies; 9a.3, once it is applied, the
crate and its spec on the test host, with its check.
### 2026-11-18: !1158 agrees with this ADR

!1158 carries out
[`2026-11-17-runtime-mcp-feed-view-contract.md`](2026-11-17-runtime-mcp-feed-view-contract.md)'s
R8 and R10b amendments: `proof.challenge` mints the passkey for unlinking a
device or removing a passkey, and names the tool that redeems it. The two
agree:

- It changes `worker/passkeys.rs`, core `mcp.rs` and `present/proof.rs` with
  its tests, none of the paths this ADR governs.
- Its challenge is minted and checked in the color's own `Instance`, as every
  proof is, so nothing crosses between blue and green.
- It adds no test of the kinds C.1 retires: its one test checks a card on a fixture.
- Nothing else here changes.

### 2026-11-18: Secret-bearing state is journaled sealed, a grant request's token hashed, and the APNs token never

Owner item #42 chose option A: hash the grant tokens, re-mint the APNs token,
and encrypt the push targets with a new shared Secrets Store key. Read
against the code, the withheld keys are of three sorts:

- **Compared only:** `grant-requests` holds each offer's page token, used
  only to compare with a presented one. It is stored as its SHA-256 hex, as
  ChangeRun's `grants` already is; `grants` is then journaled like any key.
- **Read back:** `waiting` and ChangeRun's `pages` hold grant tokens the
  approvals inbox and `approval_wait` show again, so hashing them would break
  their links. `push-devices`, `devices` (a device's APNs token),
  `members.joins` (live challenges and keys) and MainRun's `platform-outputs`
  (every tofu output, sensitive ones too) are read back too. Each is
  journaled **sealed**: its entry's body is `{"sealed": "<base64 nonce‖ciphertext>"}`,
  AES-256-GCM under a key HKDF-SHA256 derives from the Secrets Store secret
  `journal-seal` (64 hex), with info `pigeon journal v1` and the entry's
  object and key as associated data. Replay opens it with the same key; both
  colors bind the one secret, so nothing is copied between them.
- **Re-minted:** `apns_token` is a cached provider JWT (50 minutes), minted
  from Worker secrets both colors hold. It is never journaled; each color
  mints its own.

So grant tokens shown again are sealed rather than hashed. That is the same
protection the owner chose for push targets, under the same key.

- **`journal-seal` is the owner's to create**, in the account's one Secrets
  Store, as `granter` was: its value never passes through a session.
- **Until it exists** the sealed keys stay withheld. The code lands first and
  skips them while the `JOURNAL_SEAL` binding is missing; the binding is added
  to the colors' specs once the owner has created the secret.

Changes, in order: 42a the grant request's token hashed at rest; 42b sealing
in `journal` and `journal_keys`, skipping without the binding; 42c, once the
owner has created `journal-seal`, the `JOURNAL_SEAL` binding.
### 2026-11-18: !1223 agrees with this ADR

!1223 carries out a follow-up to
[`2026-11-17-runtime-mcp-feed-view-contract.md`](2026-11-17-runtime-mcp-feed-view-contract.md)'s
R10b amendment, on the owner's word after testing the iOS build: the app's
unthemed Feed tab goes. The two agree:

- It changes `src/pigeon-ios/Pigeon/RootView.swift` and removes
  `src/pigeon-ios/Pigeon/Shell/ShellView.swift`, none of the paths this ADR
  governs.
- It adds no test, and retires none of the kinds C.1 names.
- Nothing else here changes.

### 2026-11-18: A deploy ships Workers on the build before its own, so a new spec field or Worker lands in two steps

Step 9a's front took seven failed deploys to land (!1240 to !1250). The
deploy job ships Workers with `main`'s last stored `pigeon` build, and
stores its own commit's build only after its Workers step
(`src/pigeon-agent/src/ship.rs`). So a change to how Workers ship takes
effect one deploy after it merges:

- **A new spec field ships first, unused.** A spec that uses a field the
  stored build doesn't know fails every deploy (`forwards`, !1240). The
  field merges and deploys first; a spec uses it only after a deploy has
  stored that build.
- **A Worker never deployed has no deployments.** The API answers 404 for
  one, which `workers::ship` read as an error, so no new Worker could
  deploy. It now reads as none, and the Worker's first deploy is whole
  (!1246).
- **A spec the stored build can't ship is taken out, then returns.** While
  it is in, every deploy fails and holds Auto, and even the fix's own
  deploy runs on the old build. So the spec is reverted (!1248). Once a
  deploy has stored a build that ships it, the spec returns (!1250). Merging
  the return while the fix's deploy runs would supersede that deploy before
  it stores its build.

`pigeon-green` and `pigeon-blue` (step 9's `pigeon/color` kind) are new
Workers, so they take this path: whatever their specs need ships in a
stored build before the specs merge. An instance's own Workers would take
it too, on that instance's pipeline.

### 2026-11-18: Step 9b's hosts: the front takes the platform host and the zone, each color keeps its own host

A.5 step 4's second half gives `pigeon-front` `platform.pigeon.dev` and
`*.pigeon.dev/*`, keeps `control.platform.pigeon.dev` for `pigeon-control`,
and makes `PUBLIC_HOST` `platform.pigeon.dev`. Today `domains` is
`[platform.pigeon.dev, blue.platform.pigeon.dev, front-test.pigeon.dev]`,
`PUBLIC_HOST` is `domains[1]` and `PASSKEY_RP_ID` is `domains[0]`. So:

- **`PUBLIC_HOST` stays each color's own host.** The deploy's
  version-override smoke and A.4's verification reach one Worker by it, which
  the front can't do. This replaces A.5 step 4's last clause. The front
  serves `platform.pigeon.dev`, which stays `PASSKEY_RP_ID`, so a passkey
  works through the front and on each color's own host under it.
- **The stack's host becomes `control.platform.pigeon.dev`**, replacing
  `blue.platform.pigeon.dev` in `domains[1]`, so `pigeon-blue` can take
  `blue.platform.pigeon.dev` at A.5 step 8 while `pigeon-control` is still
  warm. The new name changes the Access application, a plan the owner
  applies. The old name stays in `domains` until no link or client names
  it.
- **The front takes the zone.** A `subdomains_of` route moves to the front's
  spec. The Worker it forwards to keeps the zone's var, now set from
  `$deployment.instance_zone`, so its standalone proxy is unchanged. "At
  most one stack has `subdomains_of`" still holds.
- **A custom domain moves in one deploy, rehearsed first.** The primary drops
  `domains[0]` and the front adds it, in one change whose deploy ships the
  primary before the front. The move is rehearsed with `front-test`: the
  primary takes it as a custom domain, then the front takes it back. If the
  front's step fails, `deploy_retry` repeats it. `platform.pigeon.dev`
  stays reachable meanwhile, because the primary keeps its own host.

Changes, in order: 9b.1, `domains[1]` renamed, with an Access plan the
owner applies; 9b.2, the rehearsal on `front-test`; 9b.3, the front takes
`subdomains_of`; 9b.4, the front takes `domains[0]`.

**Multi-instance.** The lease is one row per D1 database (`id = 1`), so an
instance with its own colors needs its own database, never the platform's.
Standalones' hosts reach their instance through the platform's front and
primary, so a platform flip briefly delays their proxying (the front's 5 s
cache) without changing where they go.

### 2026-11-18: Step 10's pieces: replay parity, the inventory diff, the standby target, and part B's resolved inputs

A.4 step 3 names the standby's verifications but not their shapes. Each
piece below is built and tested apart, with nothing calling it until the
standby deploy does. Promotion, the drain, the 10-minute watch, the flip
back and `lease.flip` stay the planner's work.

- **Replay parity (E1).**
  - `journal::replay(entries)` folds a `*/state` object's journal entries,
    oldest first, into each key's latest value, where a `null` body removes
    the key. Event-log objects aren't compared.
  - A sealed body is unsealed with the colour's own `journal-seal` key
    before it is digested. No value leaves the Worker; only digests do.
  - `journal::digest(value)` is the digest `journal.digests` already keeps
    per key: SHA-256 hex of the value's JSON text.
  - A colour's `GET /healthz/parity` answers, per object it holds (the
    `Instance`, `MainRun` and each open `ChangeRun`), two maps of key to
    digest: `replayed`, from the journal, and `held`, from the object's
    `journal.digests`.
  - Parity holds when the standby's `replayed` equals the primary's `held`,
    object by object. A difference names its object and key, never a value.
- **The inventory diff (E2).**
  - `architecture::diff(primary, standby)` returns the parts and links each
    `Graph` has that the other lacks, by stable id.
  - `architecture::expected(diff, added, removed)` holds a diff against what
    the change's scope adds and removes. Anything else fails the deploy and
    names the part.
- **The standby target (E3).**
  - A Worker spec gains an optional `color`, which ships unused first, per
    this ADR's two-step amendment.
  - `deploy::standby::target(lease, colors)` picks the colour the lease
    doesn't name. Without a lease row, or with a single colour, it picks
    none, and today's deploy is unchanged.
  - When a target is picked, `ship.rs` deploys only that colour's Worker,
    then runs its `/healthz`, its smoke on its own host, parity and the
    inventory diff, in that order. It stops at the first failure and leaves
    the lease alone.
- **Part B's resolved inputs (E4).**
  - A Worker's crate inputs come from `cargo metadata`'s resolved closure for
    `wasm32-unknown-unknown` with its entry's features, not the declared
    dependencies.
  - `Cargo.lock` counts only where a package in that closure changed its
    resolved version or source.
  - Both are computed by the deploy job, which has the checkout. The scope
    test pins one hit and one miss for each.

**Multi-instance.** Parity and the inventory diff compare two colours of one
instance's deployment, reading only that instance's journal and graph.
Nothing here reads another instance.
### 2026-11-18: A change to the job image's entrypoint or context deploys the Worker that runs it

A deploy rebuilds the job images only inside a Worker's deploy, and a
Worker's inputs counted only its containers' `Dockerfile`. So a change to
`pigeon-start` alone (!1264), or to `.dockerignore` alone (!1260), deployed no
Worker and left the images stale until a `deploy_retry`. Now:

- A Worker with containers also takes the image's other blocking inputs
  (`pigeon-start`, `.dockerignore`, `rust-toolchain.toml`), so their change
  deploys it and rebuilds its images. `mise.toml` still deploys no Worker, as
  §B decided; its image change waits for the next Worker deploy.
- `.dockerignore` blocks the image like `pigeon-start` does, since it decides
  the build's context.

### 2026-11-19: Step 9's colour kind: a colour inherits its platform's shared resources, and reads shared secrets from the Secrets Store

A.5.5 names a `pigeon/color` kind with `color/green` and `color/blue`, but not
their shape. `pigeon-control` holds 35 Worker secrets, and a Worker secret
can't be read back. So a colour that took its own copies would need the owner
to enter every value again for each colour, and again for any later instance.
Instead:

- **A colour unit** (`kind: pigeon/color`) names its `platform` unit,
  `worker` (`pigeon-green`) and own `host` (`green.platform.pigeon.dev`), and
  owns a CI bucket and a queue. Its kind's modules create those two. It
  inherits everything else from its platform unit: D1, the Secrets Store,
  Artifacts, the AI Gateway and the docs bucket.
- **Binding:** a spec whose `deployment` is `pigeon/color` resolves
  `$deployment.<key>` against the colour unit's own settings and outputs
  first, then its platform unit's. One spec, `infra/pigeon/workers/color/`,
  serves both colours, with `color: true`.
- **Shared secrets move to the Secrets Store.** A Worker reads a secret
  through one helper, `platform::secret(env, name)`: its Worker secret if
  set, else the Secrets Store binding of the same name. The colours bind
  every shared name to the store and hold no copy. `pigeon-control` keeps
  its Worker secrets until it retires, so nothing changes for it meanwhile.
  Only what the pipeline writes per Worker (`DEPLOY_OUTPUTS`,
  `DEPLOY_IMAGES`) stays a Worker secret, put by `pigeon workers`.
- **Hosts:** `green.platform.pigeon.dev` and `blue.platform.pigeon.dev` are
  each a colour's own custom domain and `PUBLIC_HOST`. `blue.platform` is
  `pigeon-control`'s `STACK_ALIAS` until `pigeon-blue` stands up (A.5.8),
  when it moves the way 9b.2 rehearsed. `green.platform` joins the
  platform's `domains`.

Changes, in order:

1. `platform::secret` and its use at each call site. Gate-path call sites
   are the planner's.
2. The binder's colour lookup, and the kind's schema and modules.
3. The owner batch, in one sitting: a platform plan adding
   `green.platform.pigeon.dev` to `domains` and creating `color/green`'s
   bucket and queue, then putting each shared secret in the Secrets Store
   once.
4. pigeon-green's spec, shipped to the colour unit.

**Multi-instance.** A colour belongs to one platform unit, of one instance.
Moving shared secrets to the Secrets Store also lets a new instance's
colours bind them without the owner entering values again.

### 2026-11-19: Shared secrets move to the Secrets Store by an in-Worker copy, `secrets.migrate`

The colour-kind amendment moves shared secrets to the Secrets Store. The
owner no longer holds the original values, so they move by copy: the values
already live in `pigeon-control`'s environment, and they are copied from
there.

- **`secrets.migrate`** is an owner-tier, guarded operation of
  `pigeon-control`.
  - Its argument is a list of names, each in `core::platform::secrets::SHARED`.
  - For each name, it reads the Worker secret at runtime and writes it to the
    platform's Secrets Store as `<name in lower-kebab-case>`, through the
    Cloudflare API.
  - It writes with a token the granter mints for that one call: Secrets Store
    write only, at most 10 minutes. The token is revoked when the call ends,
    whether it succeeded or failed.
  - Values never leave the Worker. No session, log, event, journal entry,
    response or error carries one. An error names the secret and the step,
    never the value.
  - The response lists, per name, whether it was written, and a fingerprint:
    the first 12 hex digits of HMAC-SHA-256 under `SESSION_KEY`. A colour can
    later check its store value against that fingerprint without anyone
    seeing either value.
  - It is approved like any owner act: a proof page and a passkey. It refuses
    a name outside `SHARED`, and it never runs from a workload.
- **What is copied, not reissued:** every name in `SHARED`.
  - Copying keeps each value. Some secrets must never be reissued, because
    existing data or enrolments depend on them:
    - `PLATFORM_STATE_KEY` decrypts stored state.
    - `SESSION_KEY` signs live sessions and tickets.
    - `APPROVER_TOTP` is the owner's authenticator.
    - `PUSH_VAPID_KEY` holds existing push subscriptions.
    - The Access service tokens (`*_ACCESS_ID`, `*_ACCESS_SECRET`) are never
      rotated, by the owner's standing instruction.
    - `journal-seal` is already in the store.
  - Reissue is a fallback, for a value that turns out to be missing or wrong.
    It happens in-platform for random keys, and at the third party for
    GitHub, Buildkite, Bitrise, Apple and Cloudflare tokens, never as part of
    this move.
- **What stays per Worker:** `DEPLOY_OUTPUTS` and `DEPLOY_IMAGES`, which the
  pipeline writes for each Worker.
- **Order:**
  1. `platform::secret`'s store fallback lands.
  2. `secrets.migrate` lands (gate paths, the planner's).
  3. The owner runs it once and checks the names and fingerprints.
  4. The colours bind the store.
  5. `pigeon-control` keeps its Worker secrets until it retires.
- **Who reads the store:** only `pigeon-control` and the colour Workers
  (`color: true`) bind a `SHARED` secret. `pigeon-front`, standalones and
  app Workers never do. The architecture test fails if any other Worker spec
  binds one.
- **No wider access:** a Secrets Store value can't be read back, through the
  API or the dashboard; only a Worker that binds it can read it. So the move
  gives no API token or dashboard role a read it lacks today.
  `secrets.migrate`'s token can only write, and only for 10 minutes. The
  count is 33: the control Worker's 35 secrets, less `DEPLOY_OUTPUTS` and
  `DEPLOY_IMAGES`. `journal-seal` and `granter` are already store bindings,
  not Worker secrets.

### 2026-11-19: Review of !1325 against !1319: they agree

!1325 restores pigeon-green's spec and makes the architecture graph, and its
coverage check, skip colour specs. It touches none of the paths
.claude/decisions/2026-11-15-runtime-zoned-standalone-account-imports.md
governs (`apex.rs`, `adopt.rs`, `git.rs`, the zone and git-repo kinds). Where
the two meet, they agree:

- `apex.rs` reads the zone's hosts from `pigeon/platform` units' `domains`.
  green.platform.pigeon.dev is platform/blue's `domains[4]` (!1311), so the
  zone already accounts for the colour's host. A `pigeon/color` unit adds no
  zone record of its own; its custom domain is created by its Worker's deploy.
- Zone imports (§5) and the cutover (§8) see the colour's custom domain as one
  of the Worker custom domains they already support.

### 2026-11-19: Each colour's own host reaches it by an exact-host route: the zone wildcard beats a Custom Domain

Step 9b assumed a colour's Custom Domain outranks pigeon-front's
`*.pigeon.dev/*` zone route. It doesn't, here:

- **Evidence.** pigeon-green has green.platform.pigeon.dev as a Custom Domain,
  and DNS holds it as a Worker record on pigeon-green, with no `*.platform`
  wildcard. Yet its Observability shows no fetch events, only its
  `17 * * * *` cron. Meanwhile each deploy probe of that host logged
  `down (status 404, not JSON)`, the empty 404 pigeon-control's `hosts::serve`
  gives a zone host it doesn't own. So the front's wildcard took the request
  and forwarded it to the lease's colour. blue.platform and control.platform
  reach pigeon-control the same way, so they'd follow the lease to green.
- **Rule.** A colour's own hosts each take an exact-host zone route,
  `{ host, zone }` in its spec, rendered as `<host>/*`. Cloudflare matches the
  most specific route pattern, so `<host>/*` beats `*.<zone>/*`. The Custom
  Domain stays for its DNS record and certificate.
- **Claiming.** A deploy may move a route from another Worker only if it is an
  exact-host pattern. It never moves the front's `*.<zone>/*` or any other
  wildcard (`ship::claimable`).
- **Proof.** pigeon-front marks what it forwards with `x-pigeon-front: <colour>`.
  The serving probe logs `up (direct)` or `up (via front to <colour>)` for each
  colour host and each exact-host route. A colour's hosts count as fixed only
  once they show `direct`.
- **Order.** A1 gives pigeon-control its two host routes. A2, the colour spec's
  restore, gives pigeon-green its route and caps its queue consumer's retries.
  P5 waits for green.platform to answer `direct` with green's version.
- **Fallback.** If wrangler refuses a route on a host that's also a Custom
  Domain, the front routes by host instead, with a service binding per colour.

### 2026-11-20: The remaining steps, as built: per-colour binding, the standby deploy, promote, and `lease.flip`

Willow set the goal on 2026-11-20: green and blue fully capable. Each colour
serves, each is the other's standby, promotion and flip-back work in both
directions, and the probe shows each colour `up (direct)`. Step 10's pieces
(parity, the inventory diff, `standby::target`) exist. These are the shapes
of what joins them.

- **1a, a colour spec binds once per colour unit.**
  - `select_bound(root, [name], deployment)` binds a spec whose kind has
    several units to the unit whose bound `name` is the one asked for. Today
    it binds to the only colour and errors on several.
  - `checks::scope::workers` already lists each unit's name (`known_as`), so
    a colour-spec change deploys pigeon-green and pigeon-blue alike.
  - Code only. `color/blue` joins config.yaml only after a build with 1a is
    stored, per the two-step rule.
- **1b, the standby deploy.**
  - `MainRun` passes `DEPLOY_STANDBY=1` once step 2 is deployed; without it,
    deploys are as today.
  - With it, the deploy job picks
    `standby::target(DEPLOY_LEASE, colours)`, where `colours` is the
    platform's Worker and each colour unit's `worker`. It ships each
    colour-capable Worker only for that colour; pigeon-front and other
    Workers ship as today.
  - It then checks the standby in order, stopping at the first failure:
    `/healthz` on its own host, `up (direct)` at the new version; its smoke
    on its own host; replay parity, where the standby's `/healthz/parity`
    `replayed` equals the primary's `held`.
  - The outcome goes in `Report.standby`: colour, version, passed, and the
    failing check. The job never writes the lease.
- **2, promote, drain, watch, flip-back** (MainRun, gate code).
  - After a deploy whose `Report.standby` passed, MainRun sets the lease to
    `promoting`. While it is promoting, the primary takes no new acts:
    `lease::acting` is false for a colour whose lease state is `promoting`,
    except for jobs already running.
  - MainRun waits until the primary reports no running job, at most
    10 minutes. Then one fenced statement moves the lease to the standby at
    `epoch + 1`, `serving`. The front follows within its 5 s cache.
  - **Watch:** for 10 minutes MainRun probes the new primary's `/healthz` on
    its own host. A failure flips the lease back at `epoch + 2`; the old
    colour is warm. Each transition is an event: `lease.promoting`,
    `lease.promoted`, `lease.flipped_back`.
  - A suite test drives each transition and a forced watch failure through
    the core state machine, `core::lease::step`.
- **Promote's guards** (also step 2):
  - **Drain timeout:** if the primary still has a running job after
    10 minutes, the promote aborts and the lease returns to `serving` on the
    current primary (`lease.promote_aborted`). The lease never flips while a
    job is running.
  - **Migrations are expand-only for one release.** Both colours share
    platform/blue's D1, and the standby deploy applies migrations while the
    old primary still serves. A migration may add tables, columns and
    indexes, but it may not drop, rename or narrow anything the previous
    release reads. A contraction lands in a later deploy, once no colour
    runs the old code. `.config/policies/` gains a rule refusing `DROP` and
    `RENAME` in `src/**/migrations/**` without a `Contract-After:` trailer
    naming the release that stopped reading the column.
  - **Watch scope:** each minute of the watch checks the new primary's own
    `/healthz` (`up (direct)` at its version) and the platform host through
    the front (`up (via front to <new colour>)`). Either failing flips back.
  - **The owner's hold:** an owner switch, `promote.hold` (passkey), stops
    auto-promotion. Deploys still reach and verify the standby, but MainRun
    doesn't flip until the owner releases it, or uses `lease.flip`. The
    hold shows on the approvals page and in `deploy_status`.
  - **The Auto hold rule:** a failed standby check leaves the primary
    serving, so it doesn't count toward Auto's failed-deploy streak (the
    2026-11-19 auto-held ADR). It pushes the owner "standby <colour> failed
    <check>". A promote that flips back counts as one failed deploy, and
    pushes too.
- **3, `lease.flip`.** An owner operation, with a passkey, in
  `pigeon-rescue`'s set. It flips the lease to the other colour, through the
  same drain and watch. It is the break-glass, and the first real cutover
  from pigeon-control to green is started with it.
- **Order and owner acts.**
  - A2 (green's spec with its route; owner merge), then 1a, then 1b, 2 and 3
    (gate merges: owner).
  - Then rehearse control → green → control with `lease.flip`, then the real
    control → green (P5).
  - Then the color/blue config change (owner), `platform_plan(color/blue)`
    and Willow's apply. Then blue's host moves from pigeon-control to the
    colour spec: blue takes its route first, then control drops it.
  - Then pigeon-blue deploys as the standby, and green ↔ blue is proven both
    ways (P6).
- **Recovery notes.**
  - The fail-fast (!1370) needs a fresh container on retry. Until the
    Runner restarts its container when `latest_build` changes, a reused
    stale container can fail several deploys in a row. `deploy_retry` is
    not the remedy: a deploy's job Container is keyed by its job id, per
    commit, so a retry reuses the same stale container. A new commit on main
    gets a fresh one. The durable fix: rebuild the job images, so the baked
    pigeon is current, and have the Runner restart its container when
    `latest_build` differs from the build it started with.
  - If main's stored build becomes unfetchable, every deploy stops. The
    break-glass is the owner's `deploy_retry` with the build check waived
    for that run, which a later change adds as `deploy_retry { stale_ok }`.
- **P7:** pigeon-control is drained once green serves, kept 7 days, then torn
  down by Willow.
- **Multi-instance.** Every check reads one instance's lease, colours and
  journal; nothing reads another instance.

### 2026-11-20: The standby adopts the journal before the flip, and the old colour runs the watch

A.5.7 says in-flight changes rebuild by replay, but nothing yet writes the
journal into a colour's objects: `/healthz/parity` only compares digests. A
flip today would hand the lease to a colour whose `Instance`, `MainRun` and
`ChangeRun`s are empty. These are the shapes that close the gap, ahead of
the 2026-11-20 steps' step 2.

- **`journal::restore(rows, open)`**, in `pigeon-core`, folds a `*/state`
  object's rows as `replay` does, but into each key's latest value, not its
  digest. A sealed key it can't open is named, and the restore fails.
- **Adopt.** Each object (`Instance`, `MainRun`, `ChangeRun`) answers
  `POST /journal/adopt`, given a restored map:
  - it puts every key in the map;
  - it deletes each key its own `journal.digests` names that the map lacks;
  - it sets `journal.digests` to the map's digests, so its `held` equals
    the journal's `replayed`.
  - It refuses (`409`) while its Worker acts, so adopt never overwrites a
    primary. Keys that are never journaled (above `MAX_VALUE`, or the
    APNs token) aren't touched.
- **The colour's `POST /healthz/adopt`**, under the deployer's Access
  token, restores and adopts the `Instance`, `MainRun`, and each
  `ChangeRun` with journal rows. It answers each object's outcome and is
  idempotent. `/healthz/parity` covers the same objects.
- **`POST /healthz/resume`**, under the same token, arms the alarm of each
  object `adopt` wrote, so the new primary picks up in-flight work once it
  holds the lease.
- **Promote, as step 2 runs it.**
  1. After the drain finds no running job, `MainRun` calls the standby's
     `/healthz/adopt`, then its `/healthz/parity`, which must find its
     `held` equal to the primary's `held`, object by object. Either
     failing aborts the promote (`lease.promote_aborted`, naming the
     check), and the primary serves on.
  2. Then the fenced flip, then the new primary's `/healthz/resume`.
  3. The watch runs on the old colour. Its `MainRun` keeps the promote
     under a key that is never journaled, and the standby's early return
     lets the promote's own steps through, and nothing else. A failed
     probe makes the old colour adopt the journal into its own objects,
     then flip back at `epoch + 1` and resume itself.
- **Order.** `restore`, adopt and the two endpoints are one change, before
  step 2's wiring. Every flip, the rehearsal's included, goes through
  adopt.

### 2026-11-20: Colours call each other by service binding, and the front binds every colour

Two gaps stand between the steps above and a real flip. The front binds only
`PIGEON_CONTROL`, so it can't forward to green once the lease names it. And
a promote's calls from one colour to another (adopt, parity, resume, the
watch's probe) can't count on a fetch to a host on the Worker's own zone:
the zone's routes and Custom Domains don't reliably serve a Worker's
subrequests.

- **The front binds each colour** under the name it already derives
  (`PIGEON_GREEN`, then `PIGEON_BLUE` once blue exists). It deletes the
  colour-token header (below) from every request it forwards.
- **Each colour binds the others it may promote to or from**, under the
  same names: pigeon-control binds `PIGEON_GREEN`, and the colour spec
  binds `PIGEON_CONTROL`. `PIGEON_BLUE` and `PIGEON_GREEN` join the colour
  spec only once pigeon-blue exists, since a binding to a missing Worker
  fails the deploy.
- **A colour's call carries `x-pigeon-colour-token`**, the shared
  `agent-token` (`BACKPLANE_TOKEN`), which every colour holds. A request
  that carries it and matches reaches `/healthz`, `/healthz/parity`,
  `/healthz/adopt` and `/healthz/resume` without Access. Nothing else
  takes it. A service-binding request never passes Access, and the front
  strips the header, so it is never accepted from outside.
- **The watch's probes** are the new primary's `/healthz` through its
  binding, and the front's answer as the lease's reader sees it: the
  lease names the new colour, and its `/healthz` answers `ok`.
- **Order.** The bindings and the token check are one change, after the
  adopt change and before step 2's wiring. The front's `PIGEON_GREEN`
  must be deployed before the rehearsal.

### 2026-11-20: The cutover is the rehearsal, flips check secrets first, and blue follows P5

Willow, 2026-11-20, through the status session: combine the rehearsal and
the cutover. What building step 2 found changes four more details.

- **One cutover, rollback proven live.** `lease.flip` control → green is
  the real cutover. After its 10-minute watch, a second `lease.flip` back to
  control proves rollback, and a third flips to green again. Each flip
  drains, adopts, checks parity and watches. This replaces the separate
  control → green → control rehearsal in A.5.7.
- **Secrets before state.** A colour's `/healthz/parity` also answers
  `keys`: each `SHARED` secret's fingerprint (12 hex digits of HMAC-SHA-256
  keyed by the value over a fixed label). It answers them only to the
  deployer's token and to another colour. A promote compares them after
  adopt and aborts on any difference (`lease.promote_aborted`, naming the
  secrets), so a flip never strands tickets signed under another
  `SESSION_KEY`. The remedy is `secrets.migrate`.
- **A standby's smoke checks `change.list`.** Its `ChangeRun`s are empty
  until adopt, so `change.status` of the newest change isn't asked of it
  (!1398).
- **Adopt deletes journaled keys.** An object's adopt deletes each key in
  its journaled set (`INSTANCE_KEYS`, `MAIN_KEYS`, `CHANGE_KEYS`) that the
  restored map lacks, not only those its `journal.digests` names. Parity
  leaves out the event-log keys every read moves (`event-cursors`,
  `push-latest`, `digest-cursor`).
- **Blue follows P5.** While the lease names pigeon-control, the standby
  is picked among control and every colour unit, and `target` takes
  pigeon-blue before green. blue.platform.pigeon.dev is also still
  control's. So `color/blue` joins config.yaml only after green holds the
  lease. Then its plan is applied, its first deploy claims its host's
  route from control, `PIGEON_BLUE` joins the front's and the colours'
  bindings, and green ↔ blue is proven both ways.
