## Blue-green deployments, and a platform that stands up a fresh copy of itself under the owner's grants

* Date: 2026-11-08
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: runtime

## Context

The owner wants two things:

- **Blue-green deployments of the engine.**
- **A fresh, end-to-end deployment of the engine, stood up by the engine
  itself.** Every step is bounded by the owner's grants. This proves the
  platform can be rebuilt from nothing, and lets a new stack replace the old
  one.

### How the platform is deployed today

- **One stack.** Each of these exists once, under fixed names:
  - one Worker, `pigeon-control`, on its `workers.dev` hostname;
  - five Durable Object classes: `Instance`, `ChangeRun`, `Checker`,
    `Runner` and `MainRun`;
  - two Container classes;
  - one D1 database, `pigeon`;
  - the buckets `pigeon-state`, `pigeon-plans` and `pigeon-ci`;
  - one queue, `pigeon-ci-events`;
  - one Access application.
- **A deploy is in place.** Per
  [`2026-11-07-development-control-plane-deploys-itself.md`](2026-11-07-development-control-plane-deploys-itself.md),
  `MainRun`'s deploy job:
  - runs D1 migrations first;
  - runs `wrangler deploy` and checks `/healthz`;
  - deploys the recorded earlier version at 100% if that health check fails.

  So there is a rollback, but no blue-green: the new version takes all
  traffic before anything has tested it.
- **Much is made by hand, once:**
  - `infra/pigeon/` is applied by the owner with a bootstrap token. Its
    state lives in the private `noisypigeon/pigeon-platform-state`.
  - The Access application and its three service tokens
    (`pigeon-agent`, `pigeon-deployer` and `pigeon-builds`) were made in the
    dashboard.
  - The Worker's 15 secrets were each set by hand, or piped from a cobot
    command.
  - `CHANGE_FLOOR`, `PUBLIC_HOST` and the names are fixed in
    `worker.yaml`.
- **Rolling back the stored state is unsafe.** Every type kept in Durable
  Object storage is `#[serde(deny_unknown_fields)]`: `Change`, `Tiers`,
  `Grant`, `Mandate` and `Pipeline`.
  - Once a new version writes a new field, the old version can't read that
    record.
  - So a rollback after a release that added a field breaks every change,
    grant and mandate written in between.
  - D1 already avoids this with expand-then-contract migrations. Durable
    Object storage has no such rule.

### What Cloudflare provides

- **Versions and gradual deployments.**
  - A version can be uploaded beside the current one and deployed at any
    percentage.
  - The `Cloudflare-Workers-Version-Overrides: <worker>="<version-id>"`
    header sends one request to a version in the current deployment, even
    one at 0%.
- **Durable Objects in a gradual deployment.** Each object is given one
  version per deployment, by the deployment's percentages, and keeps it
  until the next deployment.
  - With the new version at 0%, every object stays on the old one. So a
    request sent to the new version with the override still reaches old
    Durable Object code.
  - An upload that changes a class's lifecycle (a migration) can't go
    through `versions upload`. It needs `wrangler deploy`, and it can't be
    rolled back past.
- **Tokens can mint tokens.** An account token with *Account API Tokens
  Edit* can create account-owned tokens through
  `POST /accounts/{id}/tokens`.
  - The first such token is made in the dashboard. No API creates a
    Cloudflare account or its first credential.
- **Not yet known:** how Containers behave under a gradual deployment. Each
  Container belongs to a Durable Object, so it likely follows that object's
  version. Step 2 runs a spike to find out.

## Implementation plan.

### 1. State that both versions can read (first, and on its own)

- **A stored type tolerates fields it doesn't know.** Durable Object state
  types lose `deny_unknown_fields`. Wire and config inputs keep it.
- **A new stored field is optional, with a default,** so the version
  before reads what the version after writes, and the reverse.
- **Fields are removed in two steps,** as D1 migrations are:
  - the first release stops reading the field;
  - a later release, after the rollback window, stops writing it.
- **Fixtures guard it.**
  - `tests/state/` holds a JSON fixture of each stored type, as the current
    release writes it.
  - A test decodes the fixtures with the new types, and decodes the new
    types' output against the old fixtures.
  - A change to a stored type that doesn't update its fixture fails.
- **This makes today's rollback safe, before any blue-green.**

### 2. Version blue-green, on every deploy

- **The deploy job uploads green beside blue.**
  - Green is the new version. It is uploaded with `wrangler versions upload`
    and deployed at 0%, blue at 100%.
  - Before any traffic moves, the job tests green through the override
    header:
    - `/healthz`;
    - the MCP `tools/list`;
    - a read-only `change.status` and `deploy.status`.
- **Then the job shifts traffic to green:** 100% green.
  - All Durable Objects reach green at this step, because objects keep one
    version per deployment.
  - `/healthz` is checked again. A failure rolls back to blue, as today,
    which §1 now makes safe.
- **This doesn't apply to a migration.** A deploy that changes a Durable
  Object class runs `wrangler deploy`, as today. Its merge tier is code,
  through the `deploying` rule on `**/migrations/**` and the class list in
  `worker.yaml`.
  - Its report says it can't be rolled back past.
  - Such a change ships on its own, as Cloudflare advises.
- **A spike** checks green's Containers: which version a `Checker` runs
  while green is at 0%, and after it moves to 100%.

### 3. A stack is a parameter, declared from nothing

- **A deployment has a name:** `pigeon-<color>`, such as `pigeon-blue` or
  `pigeon-green`. The name is applied to every resource that holds state or
  takes traffic:
  - the Worker, so its Durable Object namespaces and Containers are its
    own;
  - the D1 database, the three buckets and the queue;
  - the Access application, its policies and its three service tokens;
  - the hostname.
- **A new kind, `pigeon/platform`,** under `catalog/pigeon/kinds/`, per
  [`2026-11-05-runtime-instance-config-kind-units.md`](2026-11-05-runtime-instance-config-kind-units.md):
  - Each deployment is one unit of it in `infra/pigeon/config.yaml`,
    holding the name, the hostname and the change floor.
  - It renders the D1 database, buckets and queue from today's modules, and
    the Access resources from new ones (§4).
  - It renders the Worker spec, whose `worker.yaml` gains `$deployment`
    references in place of fixed names.
- **The hostname moves to a custom domain,** `control.pigeon.dev`, which
  points at whichever stack serves. MCP clients and approval links name the
  domain, never a Worker, so a cutover changes no client.
- **Platform state moves to R2.** `infra/pigeon/`'s state leaves the
  private GitHub repository for `pigeon-state`, encrypted as instance state
  is. So the engine can plan and apply it.
  - The hand apply stays as break-glass, and for the very first stack.

### 4. Access and secrets as code

- **The Cloudflare catalog gains three modules:**
  - `cloudflare/access-application`;
  - `cloudflare/access-policy`;
  - `cloudflare/access-service-token`.
- **A service token's ID and secret go from encrypted state straight into
  the Worker's secrets.** No log, report or person sees them.
- **Each of the 15 secrets is one of four kinds:**
  - **Generated** by the run: `SESSION_KEY` and `BACKPLANE_TOKEN`.
  - **Minted** by the run from the stand-up grant (§5), each scoped as
    today: `DEPLOY_TOKEN`, `WORKERS_DEPLOY_TOKEN` and `ARTIFACTS_TOKEN`.
  - **Derived** from the stack's own resources:
    - `ACCESS_TEAM` and `ACCESS_AUD`;
    - the deployer's and builds' service tokens;
    - `DEPLOY_OUTPUTS` and `DEPLOY_IMAGES`.
  - **The owner's:** `APPROVER` and `APPROVER_TOTP`.
    - Blue's deploy job passes its own values to green inside the platform,
      so the same authenticator works.
    - For a stack built from nothing, the owner enrolls again with
      `pigeon backplane totp-secret`.
- **The agent's service token is per stack.** A cutover hands the owner the
  new client ID and secret on the approval page, once, to put in the MCP
  client's configuration.

### 5. `platform.standup`: the engine stands up a stack

- **A new operation** plans and builds a named stack. It runs as one
  `Runner` job, which survives a stop.
- **Its phases are idempotent.** A failed run resumes from the phase that
  failed. The phases are:
  1. **Plan.** The stack's resources (`cobot plan`), its Worker spec, its
     secrets by kind with no values, and its images by reference. The plan
     cites this ADR.
  2. **Grant.** The owner approves at the code tier on the approval page.
     - The page also takes the stand-up credential: an account token with
       *Account API Tokens Edit*, D1 Edit, R2 Edit, Workers Scripts Edit,
       Queues Edit and Access Edit, made in the dashboard.
     - The credential goes only to this run, encrypted in the run's bundle
       under `BACKPLANE_TOKEN`. It is never stored afterwards.
     - The page asks the owner to revoke it once the run finishes. A later
       ADR may keep it in Secrets Store for the stand-up job alone.
  3. **Apply** the stack's resources, with state in R2.
  4. **Mint** the scoped tokens, and record their IDs for teardown.
  5. **Fill** the secrets, by kind (§4).
  6. **Images:** reuse blue's by digest, or build them in the job as the
     deploy job does.
  7. **Deploy** the Worker with `wrangler deploy`. A new Worker has no blue
     to protect. Then check `/healthz` on the stack's own hostname.
  8. **Smoke test, end to end.** The stack proposes, authorizes (under a
     smoke grant), pushes, checks and closes one change on a scratch
     Artifacts repository, `pigeon-smoke`. It also renders an approval page
     and lists the MCP tools.
  9. **Report** each phase's result, with the stack's hostname and versions.
- **`platform.teardown`** removes a stack:
  - the code tier, every time;
  - it deletes the stack's resources and revokes its minted tokens;
  - it refuses the stack that serves the custom domain.
- **A rehearsal** runs `pigeon-scratch` through standup and teardown. This
  proves a stack can be built from nothing, without moving any traffic.
  The owner chooses when; a scheduled rehearsal is a later amendment.
- **The very first stack** in an empty account runs the same phases locally,
  as `pigeon backplane standup --local`, with the credential in the
  environment. The engine then stands up every later stack.

### 6. Stack blue-green: cutover with state

Use this path when version blue-green isn't enough: a Durable Object class
changes, a stack's resources change, or the platform is rebuilt.

- **Freeze blue.**
  - Blue refuses every changing operation, and lets running jobs finish.
  - Reads still answer, and blue marks itself `frozen` in `/healthz`.
- **Export** blue's state:
  - D1, through its export;
  - each Durable Object, through a new internal route, `/state/export`.
    It answers the object's storage keys as JSON and is open only to the
    cutover job;
  - R2 buckets are copied object by object.
- **Import** into green through `/state/import`, then verify: counts and a
  hash per object and table.
- **Cut over.**
  - The owner approves at the code tier.
  - The custom domain and the Access application's domain move to green,
    and green unfreezes.
- **Blue stays frozen for seven days,** so going back is the same cutover
  in reverse. After that, `platform.teardown` removes blue at the code tier.

### Execution

1. `fix(backplane)`: stored types tolerate unknown fields; state fixtures
   and their test (§1).
2. `feat(cobot)`: version blue-green in the deploy job, with the override
   smoke and two health checks; migrations stay on `wrangler deploy` (§2).
   The Containers spike comes first.
3. `feat(catalog)`: the Access modules and the `pigeon/platform` kind;
   `worker.yaml` by deployment name; platform state to R2 (§3, §4).
4. `feat(infra)`: today's stack declared as `pigeon-blue` and imported into
   state, renaming nothing, then moved to `control.pigeon.dev`.
5. `feat(backplane)`: `platform.standup`, the credential on the approval
   page, minting, and the smoke test; `platform.teardown`; a rehearsal on
   `pigeon-scratch` (§5).
6. `feat(backplane)`: freeze, `/state/export` and `/state/import`, verify,
   and the cutover (§6).
7. The first real cutover, to `pigeon-green`, when a release needs one.

Steps 1, 2 and 5 deploy the Worker, and steps 3, 4 and 6 touch `infra/` or
guarded paths, so each is approved at the code tier. Nothing here goes into
a mandate that may deploy until step 5's rehearsal has passed.

### Verification

- **A release that adds a stored field is rolled back.** Blue reads every
  record green wrote.
- **A deploy whose green fails the override smoke** never takes traffic.
  Its report says so.
- **A rehearsal stands up `pigeon-scratch` from the credential alone.** Its
  smoke change is checked and closed, and teardown leaves nothing: no
  resource and no minted token.
- **A cutover to `pigeon-green`:**
  - moves every change, grant, mandate and approval;
  - keeps the owner's authenticator;
  - leaves the approval links on `control.pigeon.dev` working.

## Amendments

### 2026-11-08: Step 1 lands: stored types tolerate later fields

- **Tolerant now:** every type in Durable Object storage, and every type
  that the Worker and a job (which may run a different build) exchange:
  - `Change`, `Event`, `Grant`, `Waiting`, `Mandate`, `Pipeline` and `Queue`;
  - the types nested in them;
  - `JobSpec` and `Report`.
- **Strict still:** inputs a person writes or calls with, namely
  `approvals::Rules`, `Instance`, `Paths` and `wire::Request`.
- **`tests/state/<type>/<date>.json`** holds each stored type as this
  release writes it. `tests/state.rs` checks three things:
  - every fixture still decodes;
  - the newest fixture round-trips, so adding, renaming or dropping a field
    fails until a new fixture is added beside the old one;
  - the newest fixture decodes with a field added to every struct, as a
    later release might write it.
- **A new enum variant follows two releases,** as a dropped field does:
  - the first release reads the variant;
  - a release after the rollback window writes it.

  Because the version before can't decode a variant it doesn't know, the
  owner reviews any change that adds a stored variant. A fixture can't
  catch it.

### 2026-11-08: Step 2's spike: what Cloudflare does at 0%, and the design it gives

The spike read Cloudflare's current documentation on gradual deployments,
version overrides, Durable Object code updates and Container rollouts. Its
findings change §2 in five places. Step 2's first deploy checks them live
(see "Proof" below).

#### Findings

1. **Objects follow the deployment, never the override.**
   - Each Durable Object is assigned one version from the deployment's
     percentages, and keeps it until the next deployment.
   - So with green at 0%, every object runs blue, `Checker` and `Runner`
     included. The override header changes only the Worker's own code.
   - The override smoke therefore tests green's Worker against blue's
     objects, which is what §1 promises to hold. It must start no job.
2. **An override applies only to a version in the current deployment.**
   - A deployment serves at most two versions, so green must be deployed
     at 0% beside blue, not merely uploaded.
   - Version URLs aren't generated for a Worker with Durable Objects, so the
     smoke sends the header to the production host.
3. **`wrangler versions upload` doesn't roll out Containers.**
   - It uploads the Worker only: no image is published, and no instance is
     replaced.
   - So a deploy whose container configuration changed (an image reference,
     an instance type) takes the `wrangler deploy` path, as a migration does.
4. **Cloudflare bounds a busy object's old code.**
   - A Worker's `durable_objects.code_update_strategy` (`deferred` with
     `max_delay`, 300 seconds by default, or `immediate`) resets an object
     still running old code once the delay passes.
   - `wrangler versions deploy` takes it per deployment, as
     `--durable-objects-code-update-mode`.
5. **The version handshake's reset loops under an override.**
   - Green, at 0%, stamps its calls as newer than blue's objects. So every
     smoke call would abort an object, which then starts again on blue,
     its assigned version.
   - The native strategy does the handshake's job without that loop.

#### Design

- **Objects no longer abort themselves.**
  - `platform::stale` is removed, along with `MainRun`'s guard.
  - The deploy sets the code update strategy to `deferred`, 60 seconds.
    `HEALTH_WAIT` stays longer than that.
  - Objects keep `/version`, and calls keep the `X-Pigeon-Version` header,
    so `/healthz` still reports which version each object runs.
- **The path is chosen per deploy.**
  - **`wrangler deploy`, as today:** for a migration, a Durable Object class
    change, a container configuration change, or a Worker with no earlier
    version.
  - **Version blue-green:** for everything else.
- **Version blue-green, in order:**
  1. `wrangler versions upload`. Its output names green.
  2. `wrangler versions deploy blue@100% green@0%`.
  3. The smoke, through `Cloudflare-Workers-Version-Overrides`, on
     `/healthz/smoke` (below). On failure, the job deploys `blue@100%`
     alone, and its report says green never took traffic.
  4. `wrangler versions deploy green@100%`, with the deferred strategy.
  5. `/healthz` as today: `ok`, green's version, and the `Instance`
     current within `HEALTH_WAIT`. On failure, the job rolls back to
     `blue@100%`, with the `immediate` strategy.
- **`/healthz/smoke`: the smoke runs inside the Worker.**
  - The deployer's service token still reads only `/healthz` and
    `/healthz/smoke`. Its reach doesn't grow to MCP or `/v0/run`.
  - Green answers from its own code, with no write:
    - the version it serves, D1, and whether the `Instance` and `MainRun`
      answer, whatever version they run;
    - MCP `tools/list`;
    - a read-only `change.status` of the newest change;
    - `deploy.status`.
  - The answer is JSON with one entry per probe. The job requires every
    probe to pass, and the version to be green's.
- **The report** names the path taken and, for blue-green, each step's
  outcome. It gains no stored field; the step outcomes go to the deploy
  log.

#### Proof

Step 2's first blue-green deploy logs the smoke's answer. It shows green's
Worker version beside blue's object versions, which confirms finding 1 live.
If Cloudflare refuses a 0% version (finding 2), the job falls back to
`wrangler deploy` and says so, and this amendment is amended again.
