## Minted secrets, state on R2, applies in Containers, a rescue plane, sessions that yield, and two warm colors

* Date: 2026-11-12
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development
* Governs: `src/pigeon-cli/crates/backplane/src/cobot/state.rs`, `src/pigeon-cli/crates/backplane/src/cobot/secrets.rs`, `src/pigeon-cli/crates/backplane/src/core/infra/**`, `src/pigeon-cli/crates/backplane/src/core/change/queue.rs`, `src/pigeon-cli/crates/backplane/src/core/change/restack.rs`, `src/pigeon-cli/crates/backplane/src/reconcile.rs`, `src/pigeon-cli/crates/backplane/src/worker/merges.rs`, `src/pigeon-cli/crates/backplane/src/jobs.rs`, `catalog/cloudflare/capabilities.yaml`

## Context

The owner asked for six things:

1. Secrets managed on Cloudflare by the control plane. When an agent needs a
   token, it asks, and the token is minted and routed where it is used. The
   operation waits for an approval, and once approved it is done.
2. Tofu state cut over to Cloudflare, and the GitHub state repository
   retired.
3. Plans and applies run in a Container, behind the same kind of approval.
4. A pass over the factory's bugs and its promotion issues.
5. First-class yielding and merge-queue changes, so two concurrent sessions
   work together instead of looping.
6. Rescue run in Containers, replacing what the owner has done locally.
7. A more durable blue-green.
   - Blue and green each have their own Worker, Access application and
     service tokens, Durable Objects and CI logs bucket.
   - D1, Artifacts and the docs bucket are shared.

### What exists today

**Secrets are set by hand.**

- `infra/pigeon/workers/control/worker.yaml` lists 26 Worker secrets. Each is
  set with `pigeon cobot workers secrets put` from the owner's shell.
  - The list includes `BACKPLANE_TOKEN`, `SESSION_KEY`, `DEPLOY_OUTPUTS`,
    `DEPLOY_IMAGES`, `WORKERS_DEPLOY_TOKEN`, `PUSH_VAPID_KEY` and
    `PLATFORM_STATE_KEY`.
  - `DEPLOY_OUTPUTS` is refreshed by hand after every `infra/pigeon` apply
    ([`2026-11-07-development-control-plane-deploys-itself.md`](2026-11-07-development-control-plane-deploys-itself.md)
    §3).
- **No stored credential mints Cloudflare account tokens.** The
  `provisioner_token` can, but it sits in `infra/cloudflare`'s `ENC` file, and
  nothing applies that unit since `cobot.yml` was removed. The tokens it
  minted aren't rotated.
- **`platform.plan` takes an account token** that the owner makes in the
  dashboard, pastes once per run and revokes afterwards
  ([`2026-11-08-runtime-blue-green-self-standup.md`](2026-11-08-runtime-blue-green-self-standup.md),
  step 4).
  - Blue-green §5 designs a stand-up credential with *Account API Tokens
    Edit*, and leaves "keep it in Secrets Store" to a later ADR. This is that
    ADR.
- **Designed but never built:**
  - per-run tokens minted from a grant
    ([`2026-11-04-development-platform-instance-separation.md`](2026-11-04-development-platform-instance-separation.md)
    §4);
  - `GRANT_KEY` in Secrets Store
    ([`2026-11-05-runtime-control-plane-worker.md`](2026-11-05-runtime-control-plane-worker.md)
    §8).
  - `catalog/cloudflare/capabilities.yaml` already names the capabilities a
    unit may request. No code calls Cloudflare's token-creation API; only
    `/tokens/verify` is called.
- **What is minted today:**
  - Artifacts repository tokens, from `ARTIFACTS_TOKEN`;
  - registry push credentials, from `WORKERS_DEPLOY_TOKEN`;
  - R2 S3 keys, derived from a token.

**State lives in two GitHub repositories.**

- `noisypigeon/pigeon-cobot-state` holds `workloads/noisypigeon.com/{apex,blog}`,
  `infra/cloudflare` and `infra/agnostic`, encrypted under
  `COBOT_STATE_PASSPHRASE`
  ([`2026-11-02-runtime-cobot-infrastructure-as-code.md`](2026-11-02-runtime-cobot-infrastructure-as-code.md)
  §4). Its only writer was `cobot.yml`, which is gone, so it is orphaned.
- `noisypigeon/pigeon-platform-state` holds `infra/pigeon`, applied by hand
  from `infra/pigeon/README.md`.
- `platform/*` state is already in R2, under `PLATFORM_STATE_KEY`.
  - `cobot/state.rs` reads and writes `r2://` with ETag checks and
    tombstones.
  - `pigeon cobot state migrate`, planned by
    [`2026-11-06-development-control-plane-approvals-images-state.md`](2026-11-06-development-control-plane-approvals-images-state.md)
    §3, was never built.

**Plans and applies.**

- The Worker refuses `infra.plan`, `infra.apply` and `infra.validate` over MCP
  (`worker/mod.rs`: "runs in an instance's Container, which the control plane
  doesn't dispatch yet").
- `cobot/worker.rs` can already describe the Container's environment for
  them, but nothing calls it.
- Only `platform.plan` and `platform.apply` run on the `Runner`, polled by
  `MainRun`.

**Factory bugs: the loop, seen live.**

- **`reconcile-255` was proposed 17 times, !260 to !289,** while !255 kept
  conflicting with each merge.
  - `worker/merges.rs::reconcile` proposes a new change every time.
  - §2 of
    [`2026-11-10-development-change-slot-eviction-policy.md`](2026-11-10-development-change-slot-eviction-policy.md),
    one reconciliation per change, was never built. Neither were its steps 3
    to 7: there is no `change_evictable`, no `refs/evicted/*` and no
    `resumes`.
- **Generated changes outlive their reason.** !203, `review-180-199`, is
  still `proposed` though both sides merged. !191 is still `approved` though
  !195 landed the same refactor.
- **A change is restacked after every merge.** Change 172 was restacked
  after four merges without merging
  ([`2026-11-08-development-concurrent-agent-sessions.md`](2026-11-08-development-concurrent-agent-sessions.md),
  amendment of 2026-11-11). Each merge restacks up to 19 changes on the
  `Runner`'s 8 Containers.
- **Overlap is announced, then nothing orders it.** Two sessions whose
  changes conflict each rebase, each conflicts with the other's next merge,
  and both get reconciliations. Neither can step aside.

**Promotion.** Promotion here means a merged head moving from `main` to
serving traffic: merge, `MainRun`'s deploy, the green version's smoke, 100%.
Its known issues:

- `DEPLOY_OUTPUTS` and `DEPLOY_IMAGES` are fed by hand
  ([`2026-11-07-development-job-images-on-cloudflare.md`](2026-11-07-development-job-images-on-cloudflare.md)).
  An image the build can't produce stalls the pipeline until the owner pushes
  one.
- A deploy with a migration, a new Durable Object class or a new Container
  config goes through `wrangler deploy`, with no smoke and no rollback
  (blue-green ADR, step 3).
- `versions upload` doesn't roll Containers. A deploy is reported done
  before the Containers run the new image.
- Green's smoke runs against blue's Durable Objects, because objects follow
  the deployment and not the override header.

**Rescue has always been local.** It needs the owner's shell, an account
token and often Docker. Each of these has been done by hand at least once:

- the first deploy;
- image pushes that outlived their login;
- `DEPLOY_OUTPUTS` refreshes;
- idling an `Instance` out by pausing `Monitor`s;
- re-minting Artifacts tokens;
- closing a jammed change family;
- re-asking for merges whose turn was lost;
- the GitHub mirror's first sync;
- every `infra/pigeon` apply.

There is no tool to export or restore a Durable Object's storage. The
blue-green ADR's §6 `/state/export` is unbuilt.

**Blue-green is one stack with two versions.**

- `platform/blue` is the only stack in `config.yaml`. Its Worker, Durable
  Objects, Containers, D1, `pigeon-ci` bucket, queue and Access application
  are all blue's.
- Version blue-green (blue-green ADR, step 2) runs green's code against
  blue's Durable Objects. Every object moves to green at once at 100%, so:
  - a bad object version is found only once it serves;
  - a class change still goes one way through `wrangler deploy`.
- Blue-green §3 gave each stack its own D1 and buckets, and §6 moves state
  between them by freeze, export and import. With that design:
  - a cutover copies everything;
  - going back means copying it all again in reverse;
  - nothing keeps the other stack warm.

### What this rests on

- **Tiers** ([`2026-11-09-security-approvals-once-passkeys.md`](2026-11-09-security-approvals-once-passkeys.md)):
  - an operation that takes, sets or reveals a secret is `owner`, one proof
    each time;
  - a delete is `owner`;
  - a grant is bound to one operation and one digest.
- **Judge and actor** ([`2026-11-02-development-sentinel-policy-cobot-actions.md`](2026-11-02-development-sentinel-policy-cobot-actions.md)):
  sentinel judges, cobot acts. A guarded policy file on `main` may be a grant,
  as eviction's is.
- **Containers:** `Runner` (8) and `Checker` (12) run `pigeon backplane jobs`,
  signed under `BACKPLANE_TOKEN`. A new Durable Object class is a migration
  that can't be rolled back past (blue-green ADR), so new work prefers new
  job kinds on existing classes.

## Implementation plan.

### 1. Grants: secrets minted, generated and routed by the control plane

**The granter.**

- **One Cloudflare account token with *Account API Tokens Write*,** made once
  by the owner in the dashboard.
- **It is stored in a Secrets Store secret, `granter`,** bound only to the
  control-plane Worker and the rescue Worker (§6).
  - It is never a Worker env secret, never in a job's environment, and never
    in the repository.
  - It is used only to call Cloudflare's token API.
  - It is set once on the owner's approval page, `/grants/granter`, with one
    proof. The value is pasted on that page, not given to an agent.
- **It replaces the per-run paste.** `platform.plan`'s pasted token, and the
  stand-up credential of blue-green §5, become minted grants.
- **The `provisioner_token` retires** once `infra/cloudflare`'s state is on
  R2 (§2) and its tokens are re-minted under §1. The `ENC` files and their
  pinning policies go in a later, owner-tier change.

**The ledger: `.config/grants.yaml`, on `main`, guarded and in `secrets:`.**
Each entry declares one secret by name, never by value:

```yaml
# A grant: what is minted or generated, and where it is put.
grants:
  workers-deploy:
    mint: { capabilities: [workers.write, containers.write], ttl: 30d, rotate: 7d }
    route: { worker: pigeon-control, key: WORKERS_DEPLOY_TOKEN }
  vapid:
    generate: { kind: p256 }
    route: { worker: pigeon-control, key: PUSH_VAPID_KEY }
  infra-run:
    mint: { capabilities: from-unit, ttl: 1h }
    route: { job: infra.apply }
```

- **`mint`** makes a Cloudflare account token from capabilities in
  `catalog/cloudflare/capabilities.yaml`.
  - The capability list gains `workers.write`, `containers.write`,
    `secrets_store.write` and `tokens.read`.
  - Each token is scoped to the account, or to the zone or bucket its unit
    names, and always expires.
  - `from-unit` takes the capabilities the unit's `config.yaml` requests, as
    the separation ADR §4 planned.
- **`generate`** makes a key in the Worker: `random32`, `p256`, `hmac` or
  `age`.
- **`derive`** computes one from another grant or from state: R2 S3 keys, or
  `DEPLOY_OUTPUTS` from a unit's outputs.
- **`route`** is where the value goes. It is exactly one of:
  - `worker`: a Worker secret, put through Cloudflare's API with a minted
    *Workers Scripts Edit* token;
  - `store`: a Secrets Store secret;
  - `job`: one job's environment, sealed under `BACKPLANE_TOKEN` for that job
    only, and revoked when the job ends.

**The operations, through MCP and `pigeon backplane run`:**

| Operation | What it does | Tier |
|---|---|---|
| `grant.request {name}` | asks for a grant the ledger declares on `main` | `owner` |
| `grant.request {name, spec}` | asks for one the ledger doesn't declare; the page shows the spec | `owner`, and the spec is proposed as a ledger change afterwards |
| `grant.rotate {name}` | re-mints or regenerates, puts, verifies, then revokes the old | auto under `.config/policies/grants-rotate.yaml`; otherwise `owner` |
| `grant.revoke {name}` | revokes the token and removes the routed value | `owner` |
| `grant.list` | names, token IDs, capabilities, routes, expiries, last rotation | auto, read-only |

**"Upon approval it is done."**

- **The approval page executes the operation.** Proving on
  `/grants/<request>` runs, in the Worker and in order: mint or generate, put
  at the route, verify, record. It answers with the result. No second call
  is needed from the agent.
  - The verify step is `/tokens/verify` for a token, or a read-back of the
    secret's metadata for a Worker secret.
- **The agent never holds the value.**
  - `grant.request` answers with the approval link, as `change_close` does.
    `approval_wait` then shows the outcome: `granted`, with token ID,
    expiry and route, or `failed`, with why.
  - No operation returns a secret value, and there is no `grant.get`.
- **Failure leaves nothing half-done.** A mint whose put fails is revoked
  before the page answers. A rotation that fails keeps the old value and
  creates a `grant-failed` task.

**Rotation.**

- **The hourly cron rotates each `mint` grant past its `rotate` age.**
- **Rotation is automatic only under a policy.**
  `.config/policies/grants-rotate.yaml`, on target `grant`, allows a rotation
  whose capabilities, scope and route equal the ledger's on `main`.
  - Its name is the grant, recorded as `policy:grants-rotate`, as eviction's
    is.
  - With no such policy, a rotation is a waiting `owner` approval.
- **Amends the passkeys ADR §2.** "Any operation that sets a secret is
  `owner`" holds for every grant except a rotation that changes nothing but
  the value, under a policy the owner merged.

**Events:** `grant.requested`, `grant.granted`, `grant.rotated`,
`grant.revoked` and `grant.failed`, with names and token IDs, never values.

### 2. State cuts over to R2, and the GitHub state repositories retire

**Where state lives.**

- **One R2 bucket per instance,** found by prefix as `platform/*`'s is. Its
  name is never written in the repository.
- **One prefix per unit:** `infra/<unit>/` and `workloads/<unit>/`, beside the
  existing `platform/`.
- **Encryption:** OpenTofu's state encryption, under `STATE_KEY`, a `store`
  grant (§1).
  - `PLATFORM_STATE_KEY` becomes an alias of it after `platform/*` is
    re-encrypted by the cutover's last step.
- **Each write keeps its history:** `<unit>/history/<serial>.tfstate`, kept
  90 days. `state.restore` (§6) rolls a unit back to one of them.
- **One writer per unit.** `Instance` holds a lock per unit (`state.lock`),
  taken by a plan or apply job and released at its end. A lock older than
  its job's deadline is released by the hourly cron, with an event.
  R2's `If-Match` checks stay as the second guard.

**The cutover operation, `state.migrate`: one Runner job, one owner proof,
all units.**

1. **Read.** The job clones `pigeon-cobot-state` and `pigeon-platform-state`
   with a GitHub App installation token.
   - The owner adds the App to both repositories, read-only.
   - Their passphrases are pasted once on the approval page. They are sealed
     to this job and erased after it.
2. **Re-encrypt** each unit's state under `STATE_KEY`, and write it to its
   prefix. The source digest is recorded beside it.
3. **Prove.** For each unit, `cobot plan` runs against the R2 state with a
   read-only minted token.
   - An empty plan proves the move kept the state.
   - A non-empty plan stops the cutover for that unit and shows the diff.
     It is drift, not a migration fault, and is resolved by an apply (§3).
4. **Switch.** Each proven unit gets `state: r2` in its instance config,
   through an ordinary change that the job proposes for the owner. From then
   on, `--state` defaults to the R2 prefix.
5. **Retire.** Once every unit has applied once from R2:
   - the owner archives both GitHub repositories;
   - `infra/pigeon/README.md`'s hand apply becomes the §6 rescue playbook;
   - `CLAUDE.md`'s line about `pigeon-cobot-state` becomes one about
     `state.restore`.

   The repositories are deleted by the owner no sooner than 30 days later.

**Order:**

1. `infra/agnostic`, which is empty;
2. `workloads/noisypigeon.com/blog`;
3. `workloads/noisypigeon.com/apex`;
4. `infra/cloudflare`;
5. `infra/pigeon`, last, because the control plane itself depends on it.

### 3. Plans and applies in Containers

**`infra.plan` and `infra.apply` are dispatched, not refused.**

- **Each is a `Runner` job kind,** polled by `MainRun` as platform runs are.
  `worker/mod.rs` routes them through `cobot/worker.rs`'s environment.
- **The job gets:**
  - the unit's state from R2, under the unit's lock (§2);
  - `STATE_KEY`, sealed to the job;
  - provider credentials as a `job` grant (§1), minted for this run from the
    unit's capabilities, with a 1-hour TTL, and revoked when the job ends.
    - A plan's token is read-only: the capabilities' `.read` forms.
    - An apply's token has the write forms.
    - Units on other providers (DigitalOcean, Scaleway, Backblaze) keep
      their `provisioner_token` until their catalog gains a minting module.
      That token is a `store` grant routed to the job, never to the
      repository.

**A plan.**

- **Stored at `<unit>/plans/<id>/`** in R2, with its digest and the state
  serial it was made against.
- **Its summary is an event and a page:** each resource, the action, and the
  attribute names that change. Sensitive values stay hidden, as
  `cobot show` does today.
- **Tier: auto.** It reads only, and its credential is minted and never
  shown. This lowers `platform.plan` from `owner`, because it no longer takes
  a pasted token. That is the passkeys ADR §2's own reason for the tier.

**An apply.**

- **Approved on the plan's page.** The page shows the summary, and its proof
  starts the apply job at once, bound to the plan's ID and digest.
- **Tier:**
  - `code`;
  - `owner` when the plan deletes or replaces anything, or creates or
    changes a token or a secret.
  - No mandate covers an `owner` apply.
- **The apply refuses a plan whose state serial has moved,** and plans again.
- **After an apply,** `DEPLOY_OUTPUTS` is re-derived from the unit's outputs
  (a `derive` grant). An apply that changed outputs queues a deploy, so a
  hand refresh is never needed.

**Triggers.**

- **A merge that changes `infra/**` or `workloads/**`** makes `MainRun` plan
  every affected unit (`cobot::scope::deploys`). An apply then waits on its
  page.
- **A drift plan runs daily for each unit.** A non-empty one is a digest
  line and an `infra-drift` task.
- **`platform.plan` and `platform.apply` move onto this path:** same job
  kinds, minted credentials, and the same page. Their refusal of deletes
  stays until blue-green §5's teardown.

### 4. The factory and promotion pass

Each item is one change, with a test that replays the failure.

**Changes and reconciliations:**

- **F1. One reconciliation per change.**
  - `reconcile` looks for an open `reconcile-<n>` or `review-<n>-<m>` of the
    same parent or pair, and updates its plan instead of proposing.
  - A test replays !255's 17 conflicts and finds one change, updated 17
    times. This is eviction ADR §2, built now.
- **F2. Generated changes close when their reason is gone:**
  - a parent that merged, closed, or restacked cleanly;
  - a review whose sides have both merged.

  This is eviction ADR §2's self-close.
  - On deploy, the hourly cron closes !203 and the other generated changes
    whose reason is gone, at the auto tier, since none pushed.
- **F3. Superseded changes.** An `approved` change whose patch is already on
  `main`, so that `git cherry` finds it applied, is closed automatically,
  with its head kept. That covers !191 after !195.
- **F4. Eviction ADR steps 3 to 7:**
  - `change_evictable`;
  - `Event::Evict`;
  - `refs/evicted/*`;
  - `remains`, the `resume` task and `resumes` on `change_propose`;
  - `change_pin`;
  - the four eviction policies.

  These land as that ADR's own amendments.

**Promotion:**

- **P1. Outputs and images aren't fed by hand.**
  - `DEPLOY_OUTPUTS` comes from §3's derive.
  - An image the build can't produce makes the deploy job run the `images`
    rescue playbook (§6) at the auto tier. It builds from `main`'s own
    Dockerfile, with a minted registry token. `DEPLOY_IMAGES` stays only as
    the break-glass seed.
- **P2. A deploy is done when Containers run its image.**
  - `/healthz` reports each Container class's image digest.
  - The deploy waits until they match, up to `HEALTH_WAIT` plus Cloudflare's
    rollout window, and reports `rolling` until then.
- **P3. One-way deploys are named and snapshotted.**
  - A deploy that must use `wrangler deploy` (a migration, a Durable Object
    class or a Container config) is marked `one-way` in the ship plan.
  - Its merge is `code` tier.
  - Before it runs, the job records a D1 Time Travel bookmark and exports
    each control-plane object's storage to R2 (`object.export`, §6), so
    `state.restore` and `object.import` can undo it.
- **P4. Smoke on green's own objects.** Green's smoke exercises a canary
  `Instance`, `smoke-<version>`, created and destroyed by the smoke run, so
  new object code runs before traffic moves.
- **P3 and P4 are interim.** §7's colors replace them: a class change
  deploys to the standby color, whose smoke runs on its own objects. Only P3's
  D1 bookmark stays.
- **P5. `deploy_status` shows promotion:**
  - each version's percentage;
  - each Container class's image digest;
  - `one-way` or blue-green;
  - for a stuck promotion, the step it stopped at and the `deploy_retry` or
    rescue that resumes it.

### 5. Sessions yield, and the queue orders instead of looping

**A yield.**

- **`change_yield {number, to, reason}`** parks a session's change behind
  another open change.
- **A yielded change:**
  - isn't restacked;
  - gets no reconciliation;
  - doesn't hold a merge turn;
  - keeps its approval, with no prompt.
- **When `to` merges or closes,** the yielded change is restacked once, then
  carries on as any change.
  - Its session gets a `yield-ended` event and task. A conflict then is a
    single reconciliation (F1), planned with `to`'s merged diff in hand.
- **A yield is the session's own act,** on its own change, at the auto tier.
  Yielding never touches the other change.

**The control plane yields for them: conflict forecasts.**

- **On every push, a `merge-tree` dry run** checks the new head against
  `main` and against every open change it overlaps. It runs in the bound job,
  pushes nothing, and costs no checks.
- **Two open changes that conflict with each other are ordered:**
  - the one further along goes first: approved, then checked, then pushed;
  - between equals, the older one goes first.

  The other is yielded to it automatically (`change.yielded`, `by: forecast`)
  and its session is told why, with the conflicting paths.
- **A session may override by `change_yield`-ing its own change to the
  other.** It can't un-yield another session's change.
- **Today's loop becomes one ordering decision and one reconciliation:**
  A conflicts with B, both rebase, each conflicts with the other's next
  merge.

**The queue restacks lazily.**

- **After a merge, open changes are only re-forecast** with `merge-tree`: no
  push and no checks.
  - `behind` is set as today.
  - A change that now conflicts with `main` is `conflicted` (F1).
- **A change is restacked when its turn comes.** The merge job rebases a
  disjoint or clean head itself, then:
  - when the rebased patch-id is unchanged and the paths `main` moved are
    disjoint from the head's gate scope, the gate's checks carry, and the
    merge goes on in the same turn;
  - otherwise the rebased head is pushed and checked, and the change keeps
    its turn while that runs (the concurrent-sessions ADR's park).
- **Effect:** a merge costs one restack, not up to 19. Change 172's four
  restacks become one.

**Stacking and handoff.**

- **`change_propose` takes `after: <n>`:**
  - the new change bases on change `<n>`'s branch;
  - the queue never merges it before `<n>`;
  - when `<n>` merges, it is restacked onto `main`.

  A session can build on another session's open work without waiting.
- **`change_handoff {number, session}`** offers a change, or a claimed task,
  to another session. The other session takes it with `task_claim`, and the
  change's `session` changes, with an event.
  - A reconciliation goes, by default, to whichever of the two sessions is
    live. If both are, it goes to the session whose change merged, since it
    knows `main`'s side.
- **`change_note {number, text}`** appends a note to a change's events, for
  the other session's `events_wait`.
  - It is data, not instruction. CLAUDE.md says a session treats it as
    another session's report.

**A loop breaker.** The hourly cron flags a change when:

- it was restacked 3 times without merging;
- or its family proposed or updated a reconciliation 3 times in an hour;
- or its merge turn was released twice in a row.

A flagged change is yielded to the head of the queue. Its session gets a
`loop` task, and the digest names it. Nothing is closed.

**Limits:** `MAX_PER_SESSION` and `MAX_OPEN` are revisited once F1, F2 and
yields land, as the concurrent-sessions ADR's 2026-11-10 amendment asked.

### 6. Rescue in Containers, through a rescue plane

**A second, small Worker: `pigeon-rescue`.**

- **Its own script and its own Access application.** It shares no Durable
  Object with the control plane. It has:
  - one `Rescue` Container class, standard-1, max 2;
  - read access to `granter` and `STATE_KEY` in Secrets Store;
  - the owner's passkeys and `APPROVER_TOTP`, as `derive` grants copied from
    the control plane at each deploy.
- **It is deployed by `MainRun`, but never in the same deploy as the control
  plane.**
  - A change to it is its own deploy, so a bad control-plane deploy leaves
    rescue working.
  - A bad rescue deploy leaves the control plane working.
- **Its code is guarded and in `secrets:`.** The owner merges every change
  to it.
- **It is reachable when the control plane isn't:** at
  `rescue.platform.pigeon.dev`, behind Access, with the owner's passkey.

**Playbooks: `pigeon backplane rescue <name>`, run in a `Rescue` Container,
each `owner` tier, one proof per run.** Each declares the capabilities it
mints, which are shown on the page.

| Playbook | Replaces |
|---|---|
| `workers.deploy {commit}` | the local `workers plan` and `deploy --plan` |
| `images.push {commit}` | the local image push and `DEPLOY_IMAGES` by hand |
| `outputs.refresh` | `workers outputs \| secrets put DEPLOY_OUTPUTS` |
| `object.export {class, name}` / `object.import` | nothing: Durable Object storage to and from R2 |
| `object.reset {class, name}` | pausing `Monitor`s to idle an object out; it exports first |
| `queue.release` | re-asking for a merge whose turn was lost |
| `changes.unjam {numbers}` | closing a jammed family by hand; it keeps each head as `refs/evicted/*` |
| `state.unlock {unit}` / `state.restore {unit, serial}` | none |
| `git.mirror` / `git.sync` | the mirror's first sync from the owner's shell |
| `infra.apply {unit, plan}` | `infra/pigeon`'s hand apply, when the control plane is down |
| `grant.rotate {name}` | re-minting a token by hand |

- **`rescue.run {script}` is for what no playbook covers.** It runs a shell
  script in a `Rescue` Container, with a pigeon checkout of `main` and the
  capabilities it names.
  - The page shows the script verbatim and binds its digest.
  - Its output is kept in R2 for 30 days and linked from the event.
  - An agent may write the script. Only the owner's proof runs it.
- **Every run:**
  - is an event on the rescue Worker, mirrored to the control plane's log
    when that is up;
  - is a digest line;
  - revokes its minted credentials when it ends.
- **Local stays the last resort,** for when Cloudflare itself is down. The
  playbooks are the same `pigeon backplane rescue` commands, so the owner
  runs them locally with the same names.

### 7. Durable blue-green: two warm colors over shared truth

This amends
[`2026-11-08-runtime-blue-green-self-standup.md`](2026-11-08-runtime-blue-green-self-standup.md)
§3, §5 and §6. Its steps land as amendments there.

**What each color owns, and what is shared.**

| Per color (`platform/blue`, `platform/green`) | Shared (`platform/shared`) |
|---|---|
| its Worker, `pigeon-<color>` | the D1 database, `pigeon` |
| its Durable Objects: `Instance`, `ChangeRun`, `Checker`, `Runner`, `MainRun` | the Artifacts namespace and its repositories |
| its Containers, which belong to its objects | the docs bucket (`sites/docs`) |
| its CI logs bucket, `pigeon-ci-<color>` | the state bucket (§2) |
| its Access application on `<color>.platform.pigeon.dev`, and its service tokens: agent, deployer and builds | the front Worker, `pigeon-front`, on `platform.pigeon.dev` |
| its queue, `pigeon-ci-events-<color>`, since a queue has one consumer | the lease (below) |
| its minted tokens (§1), each named for its color | `granter` and `STATE_KEY` in Secrets Store |

- **`platform/shared` is a new unit of the `pigeon/platform` kind.** Teardown
  refuses it, so tearing down either color can't take the truth with it.
- **A color holds nothing that can't be rebuilt** from the shared unit and
  the journal.

**Truth lives in D1, as a journal. Objects are a color's working copy.**

- **Every state change is first a journal entry.**
  - `core`'s step functions already turn an input into events
    ([`2026-11-10-development-factory-crates-functional-core.md`](2026-11-10-development-factory-crates-functional-core.md)
    §3).
  - An object appends its events to D1's `journal` table, keyed by
    `(object, seq)` and unique on that key, and only then applies them to
    its own storage.
  - An append that fails fails the operation, which is retried. Nothing is
    applied that isn't journaled.
- **The event log moves to the journal.** `events_wait`, the digest and
  `events_query` read D1, so a session's event cursor survives a flip.
- **Replay rebuilds an object.** `core` folds the journal into an object's
  state. Each object keeps `applied_seq`.
  - A color whose objects are behind replays the tail on its next call.
  - A color built from nothing replays everything.
- **Proof: a replay check runs daily.**
  - It replays every live object's journal into a scratch object, and
    compares a hash of the state with the live one.
  - A mismatch is an owner task, and blocks flips until it is resolved.
- **Push events reach both colors.**
  - Each color's queue is subscribed to the namespace's
    `cf.artifacts.repo.pushed` events.
  - Both consumers write the event to the journal. The entry is unique on
    the event's ID, so the second write is a no-op.
  - Only the lease holder acts on it. A push that arrives mid-flip is
    acted on by whichever color holds the lease next.

**One lease, with fencing.**

- **D1 holds one row,** `lease (color, epoch, state)`, where `state` is
  `serving`, `draining` or `promoting`.
- **Every journal append carries the writer's epoch.** An append whose epoch
  isn't the current one is refused.
  - So a color that has lost the lease can't change state, even if a stale
    object or a late job is still running there.
- **The standby color is warm.**
  - It runs the previous release.
  - It keeps its objects replayed to the journal's head every minute.
  - It answers reads on its own hostname.
  - Its changing operations answer `standby: <serving color>`.

**The front Worker: one hostname, an instant flip.**

- **`pigeon-front` serves `platform.pigeon.dev`.** It holds service bindings
  to both colors. For each request it reads the lease, cached for at most
  5 seconds, and forwards to the serving color.
  - It holds no state, and its code is small and guarded.
  - It deploys on its own, as `pigeon-rescue` does (§6), never in the same
    deploy as a color.
- **Access, per color:**
  - Each color's own Access application covers only its own hostname.
  - The front's Access application admits the owner, and the agent service
    token of each color that exists. So MCP clients and approval links
    always name `platform.pigeon.dev`, and a flip changes no client.
  - This replaces the blue-green ADR §4's per-stack client configuration.
- **Passkeys keep working.** The relying party is `platform.pigeon.dev`, and
  the keyring is journaled, so both colors verify the same passkeys.
  `APPROVER_TOTP` is a `derive` grant (§1) put into both colors.

**A deploy goes to the standby color, then flips.**

1. **Deploy the new release to the standby color,** with `wrangler deploy`.
   Class changes and Container config changes are safe here, because
   nothing serves from that color yet. This retires §4 P3's `one-way` deploy
   for everything except D1 migrations.
2. **Replay** the standby's objects to the journal's head.
3. **Smoke on its own hostname,** against its own objects and Containers.
   This is the end-to-end smoke of blue-green §5, step 8, on the scratch
   repository `pigeon-smoke`. It replaces the override header and §4 P4.
4. **Drain the serving color.**
   - The lease goes to `draining`. The serving color refuses new changing
     operations, which return `retry` with a 2-second hint.
   - Running jobs finish, and their reports are journaled.
   - A job that runs past 60 seconds is left running. Its report is
     journaled when it arrives, and the next lease holder applies it.
5. **Promote.**
   - The standby replays the last entries.
   - The lease moves to it in one D1 transaction: `epoch + 1`, `serving`.
   - The front sends traffic there within 5 seconds.
6. **Check health** through `platform.pigeon.dev`. On failure, the lease
   flips back, which is the same operation with the old color.

**Rollback stays cheap.** The old color stays warm on the previous release
until the next deploy replaces it. Going back is a flip, not a copy, at any
time.

**D1 is the one-way part.** Both colors read one schema, so:

- a migration must be readable by both releases: expand only;
- a contraction (dropping a column or table) ships only after one full
  release cycle with no reader, and its merge is `owner` tier;
- `.config/policies/migrations.yaml` gains the rule that a migration
  touching a column the previous release reads is refused;
- before each migration the deploy records a D1 Time Travel bookmark, as
  §4 P3 does.

**Rescue and teardown:**

- A color is rebuilt from nothing by `platform.standup <color>` and a replay.
  This needs no export or import: blue-green §6's freeze, export and import
  are superseded.
- `platform.teardown <color>` refuses the serving color and `platform/shared`.
  It is `code` tier, and it revokes that color's Access tokens and minted
  tokens.
- `pigeon-rescue` (§6) gains `lease.flip {color}` (`owner`). It flips the
  lease when the control plane can't, for example when the serving color
  is down and the deploy job with it.

**Capacity.**

- **Two colors double the Container ceiling,** but only the serving color
  runs jobs. The standby's `Checker` and `Runner` start only for its own
  smoke.
- **The journal grows by about 10,000 rows a day,** at today's event rate.
  Entries older than 90 days, and below every object's `applied_seq`, move
  to R2 as daily files, which replay reads when it needs them.

### 8. Approvals, at a glance

| Operation | Tier |
|---|---|
| `grant.request`, `grant.revoke` | `owner` |
| `grant.rotate` with an unchanged spec, under `grants-rotate.yaml` | auto (`policy:grants-rotate`) |
| `grant.list` | auto |
| `state.migrate` | `owner` |
| `infra.plan`, `platform.plan` | auto |
| `infra.apply`, `platform.apply` | `code`; `owner` on a delete, replace, token or secret |
| `change_yield` (own change), `change_note`, `change_handoff` (own change) | auto |
| a forecast yield or a loop yield | the control plane, recorded `by: forecast` or `by: loop` |
| any rescue playbook, `rescue.run` | `owner`, each run |
| P1's image playbook, run by the deploy job | auto, `main`'s Dockerfile only |
| a deploy's flip to the standby color, after its smoke passed | auto, as the merge that deployed it was approved |
| a flip back after a failed health check | auto |
| `lease.flip` by hand (rescue) | `owner` |
| `platform.standup <color>`, `platform.teardown <color>` | `code`; `owner` when it mints or revokes tokens |
| a D1 migration that contracts | `owner` |

### 9. Execution

Each step is one change, at its own tier. Steps 1, 9, 10, 13 and 14 touch secrets
and cannot be under a mandate.

1. **`feat(backplane)`: grants.**
   - `.config/grants.yaml` and its sentinel target;
   - the `granter` Secrets Store binding;
   - mint, generate, derive and route;
   - `grant.*` and `/grants/<request>`;
   - the capability additions.

   The owner sets `granter` on its page.
2. **`feat(backplane)`: R2 state for every unit:** the per-unit lock,
   history, `STATE_KEY`, and `state.migrate` as a Runner job.
3. **The cutover,** run by the owner's proof, in §2's order, with one
   `state: r2` change per proven unit.
4. **`feat(backplane)`: `infra.plan` and `infra.apply` as Runner jobs** with
   minted credentials, the apply page, the merge and drift triggers, and the
   derived `DEPLOY_OUTPUTS`. `platform.*` moves onto them.
5. **`fix(backplane)`: F1 to F3.**
6. **`feat(backplane)`: the eviction ADR's steps 3 to 7 (F4),** as that ADR's
   amendments.
7. **`feat(backplane)`:** yields, forecasts, lazy restacks, `after`,
   `change_handoff`, `change_note` and the loop breaker. Then the limits are
   revisited.
8. **`feat(backplane)`: P1 to P5.**
9. **`feat(infra)`: `pigeon-rescue`,** its Container, the playbooks and
   `rescue.run`.
10. **`chore`: retire.**
    - Archive the GitHub state repositories (by the owner).
    - Remove the `ENC` files and the `provisioner_token`, and amend their
      policies (`owner`).
    - Update `CLAUDE.md`'s state and break-glass lines.
11. **`feat(backplane)`: the journal.**
    - D1's `journal` table and the lease row;
    - objects append before they apply;
    - the event log on the journal;
    - `applied_seq`, replay and the daily replay check.

    It runs in shadow first: written and checked, but no flip depends on it
    until a week of replay checks pass.
12. **`feat(catalog)`: the `pigeon/platform` kind split into a color and
    `platform/shared`.**
    - `platform/blue` keeps today's resources.
    - D1, the docs bucket and the state bucket move to `platform/shared` by
      `moved`, renaming nothing.
    - `pigeon-ci` becomes `pigeon-ci-blue` by `moved`.
13. **`feat(infra)`: `pigeon-front`** on `platform.pigeon.dev`, with its
    Access application, forwarding to blue alone. Blue's own application
    narrows to `blue.platform.pigeon.dev`.
14. **`platform.standup green`:** green's Worker, objects, Containers, CI
    logs bucket, queue, Access application and tokens. Green replays the
    journal, and a rehearsal flips to green and back.
15. **`feat(backplane)`: the deploy job deploys to the standby and flips.**
    Version blue-green stays only as the fallback while one color exists.
    `lease.flip` joins `pigeon-rescue`.

Steps 5 and 6 come first in time: they stop the loop that today costs slots
and owner attention, and need nothing else from this ADR. Steps 11 to 15
need step 1 (grants for each color's tokens) and step 9 (rescue can flip),
and come after them.

### 10. Verification

- **Grants:**
  - a `grant.request` for `workers-deploy` is approved once, after which
    the Worker secret holds a new token whose `/tokens/verify` passes, and
    the event shows its ID and no value;
  - a failed put revokes the minted token;
  - no MCP tool returns a value;
  - a rotation without the policy waits for the owner.
- **State:**
  - each unit's R2 plan after migration is empty;
  - a second writer is refused by the lock;
  - `state.restore` to an earlier serial plans the expected diff.
- **Applies:**
  - a merge to `workloads/noisypigeon.com/blog` plans it, and proving the
    page applies it with a token revoked by the end of the job;
  - a plan that deletes asks `owner`;
  - a moved serial refuses the apply.
- **Factory:**
  - a replay of !255 makes one `reconcile-255`;
  - !203 and !191 close themselves;
  - two changes forecast to conflict are ordered, the second is yielded,
    and after the first merges the second takes one reconciliation, not
    one per merge;
  - a merge restacks no other change until its turn.
- **Promotion:**
  - a deploy reports `rolling` until the Containers' digests match;
  - a one-way deploy leaves a D1 bookmark and object exports.
- **Blue-green:**
  - a replay of every object from the journal hashes equal to the live state;
  - a deploy with a new Durable Object class reaches the standby, passes its
    smoke there, and flips with no request lost: those made during the
    drain are retried once and succeed;
  - after a flip, a write from the old color's object is refused by epoch;
  - a push made mid-flip is journaled once and acted on once;
  - a flip back restores the previous release within 10 seconds;
  - green torn down and stood up again replays to the same state, while
    D1, Artifacts and the docs bucket are untouched;
  - `events_wait` with an event cursor from before a flip continues after
    it.
- **Rescue:**
  - with the control plane's script deleted in a test account,
    `pigeon-rescue`'s `workers.deploy` restores it;
  - `rescue.run` refuses a script whose digest differs from the approved
    one.

## Amendments

### 2026-11-13: F1 lands as reuse; the reused plan's update waits two releases

- **A change that conflicts again reuses its open reconciliation.**
  - `merges::reconcile` reads the open changes first.
  - When one already carries the proposal's slug (`reconcile-<n>`, or
    `review-<n>-<m>` for the same pair), it proposes nothing and appends
    `reconcile.reused`. The conflict then points at that change.
  - The open changes' D1 query gains `slug`, which `sessions::Open` carries.
- **The reused change keeps its first plan.**
  - Updating it needs a new stored event variant, which the blue-green ADR's
    rule ships over two releases.
  - Until then, a session claiming the task reads the newest conflict in the
    parent's `change_status`.
- **Execution:** one `fix(backplane)` change, with a test that replays
  !255's duplicates onto one change.

### 2026-11-13: F2 lands: a generated change closes when its reason is gone

- **When a change's saved state shows its reason gone, its generated changes
  close.** The reason is gone when:
  - the change merged or closed, which closes its `reconcile-<n>` and every
    `review-<n>-*`;
  - or its conflict cleared, by a clean restack or its session's push, which
    closes its `reconcile-<n>`.
- **Only never-pushed changes close** (`proposed` or `planned`). Their tasks
  close with them, and each close appends `reconcile.retired`.
  - A close of a change that never pushed is auto (approvals ADR §2).
  - A pushed reconciliation is real work, and is left to its session.
- **A review whose merged side is the change ending is left,** because that
  merge is what made it. It closes when its open side merges or closes.
- **Not yet:** generated changes left open before this lands, such as !203.
  They close the next time their parent's state moves. A one-off sweep waits
  for the eviction ADR's hourly job (F4).

### 2026-11-13: F3 lands: a change whose patch is on main asks to close; Auto approves it

The owner chose that closing a superseded change asks for approval by default,
and that an Auto window from the app approves it.

- **Detected by the restack job.** When the head merges into `main` and the
  tree equals `main`'s, the patch has already landed. The report says
  `landed`, and nothing is pushed.
- **Recorded on the change.** `landed` holds the head found on `main`. It is
  an optional field, so the version before reads it. The change gets a
  violation saying so, gives back any merge turn, and isn't reconciled.
- **The close's tier is `code` while `landed` names the current head.**
  - The patch is already on `main` and nothing is lost, so this amends
    [`2026-11-09-security-approvals-once-passkeys.md`](2026-11-09-security-approvals-once-passkeys.md)
    §2 for this one case.
  - A newer push changes the head, and the close is `owner` again.
- **The control plane asks for the close itself,** through the same chain a
  session's request takes:
  - a mandate, an Auto window, a policy or the change's grant approves it;
  - otherwise it waits on the approval page, with a push to the owner.
- **Auto windows approve `change.close` at the code tier or lower.** An
  `owner` close still asks.
### 2026-11-13: F4 begins: a family holds one slot

- **A change and its unpushed generated changes hold one slot.**
  - A `reconcile-<n>` or `review-<n>-<m>` that never pushed counts toward
    neither limit.
  - Once pushed, it counts, being real work.
- **A refusal names the counted changes,** each with its family beside it:
  `!79 (+!85 +!86)`.
- **This completes the eviction ADR's §2,** after F1 and F2. Its §3 to §7
  follow as F4's next changes.

### 2026-11-13: The factory bugs found while landing F1 to F4

Each fix is its own change.

1. **The front door answered git's flush probe with 400.** Before a large
   pack, git POSTs a lone `0000` and needs a 200. It is now answered 200
   (front-door ADR, 2026-11-13). This, not shallow clones, was why a
   session's first large push failed.
2. **A stale `starting` slot redeployed a deployed head,** which failed with
   no reason and paused Auto. A deployed head is now dropped from the slot,
   and a failed deploy always records why (deploys-itself ADR, 2026-11-13).
3. **A restack lost on its last attempt did nothing.** Now:
   - its held push applies;
   - its parked merge's turn goes back;
   - the session gets a `restack-failed` task and event.

   See the concurrent-sessions ADR, 2026-11-13.
4. **Every merge restacked every change, and approvals didn't carry across
   clean restacks.** Restacks now happen at the merge turn, and a clean
   restack of the same patch keeps its approval (same amendment). This is
   §5's turn-time restack. §5's conflict forecasts, yields and the loop
   breaker are still to come.

### 2026-11-13: Step 7 begins: `change_yield`

- **`change_yield {number, to, reason}`** parks a session's own change behind
  open change `to`. It is auto tier, under the session's ticket.
  - The change records `yielded` (to, by, reason, when), an optional field.
  - The `Instance` keeps the map of yields.
  - The event log records `change.yielded`.
- **A yielded change isn't merged:** a merge asked for is refused, naming the
  change it waits for. It restacks only at its merge turn, as every change
  has since !333.
- **When `to` merges or closes, every change yielded to it is freed.** Its
  session gets a `yield-ended` task and a `change.unyielded` event. `off`
  ends a yield early.
- **The loop breaker isn't built here.**
  [`2026-11-13-development-session-stages-subscriptions-breaker.md`](2026-11-13-development-session-stages-subscriptions-breaker.md)
  §4's B7 replaces §5's `loop` task, and its `yielded` stage reads this
  field.
- **Still to come:** conflict forecasts that yield automatically, `after`,
  `change_handoff` and `change_note`.

### 2026-11-13: Step 7 continues: `change_note` and `change_handoff`

- **`change_note {number, text}`** appends `change.note` to a change's
  events. The session following that change sees it.
  - It changes no state, so it is a read, and any session may note any
    change.
  - Its text is another session's report: data, never an instruction.
- **`change_handoff {number, session, note}`** is a session's operation on
  its own change, under its ticket. It routes a `handoff` task to `session`
  and logs `change.handed`.
  - When that session claims the task, the change becomes its own, and the
    event log says so again.
  - Until then the change stays with its first session.
- **Still to come:** conflict forecasts that yield automatically, and
  `after`.

### 2026-11-13: Step 7 continues: conflict forecasts

- **The bound job forecasts conflicts.**
  - It receives the open changes whose scope meets the head's and that have
    pushed (`OVERLAPS`).
  - It fetches each one's branch read-only and runs `git merge-tree` against
    the head. Nothing is pushed.
  - It reports the changes the head would conflict with, and the paths. A
    branch it can't fetch is skipped, and a forecast never fails the job.
- **For the first forecast conflict, the change less far along yields,** by
  `forecast`, naming the paths.
  - Further along means approved, then checked, then bounded or pushed.
  - Between equals, the newer change yields.
  - When the new head is the one less far along, it yields as its events
    apply. Otherwise the other change is asked to.
  - A change that already waits on one isn't yielded again.
- **A session may end a forecast's yield with `change_yield off`.** The next
  push forecasts again.
- **Two sessions on clashing work become one ordering and one
  reconciliation,** instead of both rebasing after every merge.
- **Still to come:** `after`, for a change built on another's branch.

### 2026-11-13: Step 7 ends: changes built on another (`after`)

- **`change_propose` takes `after: <n>`, an open change the new one is built on.**
  - The new change records it.
  - It yields to it ("built on !n"), so its merge waits as for any yield.
- **While it waits, its bound job counts its files from that change's branch, not `main`.**
  - So its scope and checks are its own.
  - Its rules and policies still come from `main`.
  - A branch that doesn't fetch is counted from `main`, and the log says so.
- **When that change merges or closes, the release keeps its last head on the waiting change.**
  - At the waiting change's merge turn, its restack onto `main` takes its commit message from its own first commit past that head.
  - The restack then forgets `after`: the change now stands on `main`.
  - Its patch differs from the one approved, so it is approved again after the restack.
- **A change built on one that closes unmerged is left to its session.** The session gets the `yield-ended` task, and the bound job counts from `main` again.
- **Step 7 is done.** Next, by §9's order: step 1's grants.

### 2026-11-13: Step 1 begins: the ledger

Step 1 lands in four changes, since its parts sit at different tiers:

- **1a, the ledger:** code tier, with no secret in it.
- **1b, the new capabilities:** owner tier, as `catalog/cloudflare/capabilities.yaml` is on the `secrets:` list.
- **1c, the operations and their page:** owner tier.
- **1d, sentinel's `grant` target and `grants-rotate`:** a sentinel change of its own.

The first, the ledger:

- **`.config/grants.yaml` is the ledger.** It starts with no grants. The
  first grants arrive with 1b's capabilities.
- **`core::ledger` checks each grant.** A grant has:
  - exactly one source;
  - exactly one route;
  - a key in upper snake case for a Worker or store, and none for a job;
  - a `ttl`, and a `rotate` shorter than it;
  - only capabilities a catalog declares;
  - a `job` route only for a minted token.

  Unknown fields are refused, so a typo can't widen a grant.
- **`Instance::load` reports the ledger's problems with the manifest's,** so
  `mise run backplane` and every command that loads the instance refuses a
  bad ledger.
### 2026-11-13: A forecast counts only the two changes' own paths

- **What went wrong:** a conflict forecast merged the head with another
  change's branch at their merge base. When that branch sat on an older
  `main`, `main`'s later commits showed as conflicts, in paths neither change
  edits. Live, !349 was forecast to conflict with !191 in `Cargo.lock` and
  `.config/releases.yaml`.
- **The fix:** a forecast now keeps a path only when both changes edit it
  against `main`. A forecast with no such path is no conflict.
- **ADRs merge by union in a forecast, as in a restack.** So two amendments
  to one ADR no longer make one change yield to the other.

### 2026-11-13: Step 1c begins: the minting core, and where `granter` comes from

- **1c lands in two changes.**
  - The pure core, `core::minting`.
  - Then the Worker wiring: the `granter` binding, `grant.*` and
    `/grants/<id>`, at owner tier.
- **`token_body` builds the account-token body,** as the `api-token` module
  does.
  - Each permission must match exactly one of the account's permission
    groups in its scope.
  - Otherwise nothing is minted, and the error lists the names the account
    does have. So a wrong catalog name, as 1b's could be, fails before any
    token exists.
- **`Mint.zones` names the zones a zone-scoped capability is minted on.**
  Bucket scope isn't minted yet.
- **`settle` decides each step's outcome.**
  - A token minted before a failed put or verify is revoked.
  - The old one stays.
- **`granter` is created by the owner in the Secrets Store, in the
  dashboard.** It isn't pasted on a page, because a Worker can only write the
  store with a token that `granter` itself would mint.
  - `/grants/granter` checks instead that the binding resolves, and that
    `/tokens/verify` calls the token active.
  - This amends §1's "set once on the owner's approval page".

### 2026-11-13: `granter` lives in the platform's own Secrets Store

- **Option A: `granter` lives in a Secrets Store, never as a Worker
  secret.**
  - The new `cloudflare/secrets-store` module makes one store.
  - The `pigeon/platform` kind holds it as a singleton, named after the
    stack's Worker, and outputs `secrets_store_id`.
  - A `worker.yaml` binds it as `$deployment.secrets_store_id`.
- **The order:**
  1. The owner applies `platform/blue` (`platform_plan`, then
     `platform_apply`). That creates the store.
  2. The owner creates `granter` in it from the dashboard.
  3. 1c's last change binds `GRANTER` in `worker.yaml` and adds `grant.*`
     and `/grants/<id>`.

  A binding to a store that doesn't exist yet would fail the deploy, so the
  binding comes last.
- **If the account already has a store and refuses a second,** the plan
  says so. The existing store is then adopted (`pigeon cobot adopt`) rather
  than a second one made.
### 2026-11-13: The Worker's ledger is the one main deployed

- **The deploy job resolves `main`'s ledger and reports it** (`Report.ledger`).
  - The resolved ledger carries each named capability's permissions and
    scope, from the catalogs.
  - `MainRun` keeps the ledger of the last passed deploy and answers it at
    `GET /ledger`.
- **So `grant.*` acts on the ledger exactly as deployed.** It never reads a
  branch, and no repository file is read in the Worker.
- **A ledger that doesn't resolve is logged, never a failed deploy.** The
  instance's checks refuse a bad ledger before it merges.
- **`from-unit` mints wait for step 4's unit runs,** which know their unit.
