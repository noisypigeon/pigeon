## Blue-green deployments, and a platform that stands up a fresh copy of itself under the owner's grants

* Date: 2026-11-08
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: runtime
* Governs: `catalog/pigeon/kinds/platform/**`, `src/pigeon-cli/crates/backplane/src/platform*.rs`, `src/pigeon-cli/crates/backplane/src/worker/platform*.rs`, `src/pigeon-cli/crates/backplane/src/versions.rs`, `src/pigeon-cli/crates/backplane/tests/state/**`, `src/pigeon-cli/crates/backplane/tests/state.rs`

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

### 2026-11-08: Step 3's design: where a stack is declared, and how a Worker names it

A survey of the code found six places where §3 and §4 don't fit what cobot
does today. This amendment settles each one, and splits step 3 into two
changes.

#### What the survey found

1. **Kind units live only in the root `config.yaml`.** `infra/pigeon/config.yaml`
   is a directory unit, and can't hold `units:`.
2. **Only an apex or a site may use `derived` blocks.** A platform kind
   uses `singletons`, `each` and per-property blocks, as
   `infra/pigeon/schema.json` already does.
3. **A Worker's `$output.<unit>` must name a directory unit.**
   `validate_bindings` rejects a kind unit.
4. **A Worker spec's names are literals:** the Worker, the D1 database, the
   queue, and the `vars` (`PUBLIC_HOST`, `INSTANCE_NAME`). `Spec` denies
   unknown fields.
5. **Nothing creates the queue or Access.** `pigeon-ci-events`, the Access
   application, its policies and its three service tokens were made by
   hand. The Cloudflare catalog has no D1, queue or Access capability.
6. **New modules touch guarded files.** Each new module's `CHANGELOG.md`
   must be listed in `.config/policies/release-changelog.yaml`.

#### Design

- **A stack is a kind unit in the root `config.yaml`**, as
  `platform/<color>` with `kind: pigeon/platform`. This replaces §3's
  `infra/pigeon/config.yaml`.
- **The `pigeon/platform` kind** lives at
  `catalog/pigeon/kinds/platform/schema.json`.
  - **Its settings name every resource explicitly,** so today's stack can
    be declared without renaming anything:
    - `worker`;
    - `database`;
    - `buckets`;
    - `queue`;
    - `hostname`;
    - `instance` (the `INSTANCE_NAME`);
    - `change_floor`;
    - `access`: the application's name, the owner's email, and the three
      service tokens' names.
  - **It composes only prefixed modules:** `cloudflare/account`,
    `cloudflare/d1-database`, `cloudflare/r2-bucket` with
    `agnostic/random-string`, `cloudflare/queue`, and the Access modules.
  - **Its outputs** are each resource's ID and name, and each service
    token's client ID. A service token's secret is a sensitive output.
    It is never put in `DEPLOY_OUTPUTS`, and §5's fill phase is its only
    reader.
  - **It is `local: true`.** No instance run plans it. Its owner plans and
    applies it by name, as with `infra/pigeon/`, until §5's stand-up does.
- **Four Cloudflare modules,** each a single resource on provider 5.28.0:
  - `cloudflare/queue`;
  - `cloudflare/access-application`;
  - `cloudflare/access-policy` (reusable, for the account);
  - `cloudflare/access-service-token`.
  - `capabilities.yaml` gains `d1.write`, `queues.write` and `access.write`
    for §5's minted credentials. Until then, nothing mints them.
- **A Worker spec names its stack once,** with `deployment: pigeon/platform`.
  - Its values may then use `$deployment.<output or setting>`: the
    Worker's `name`, the D1 database's `name` and `id`, the R2 values,
    a queue consumer's `queue`, `vars`, and `routes[].custom_domain`.
  - A render, plan or deploy picks the stack by its unit ID,
    `platform/<color>`. The deploy job uses the Worker var `DEPLOYMENT`,
    which the render sets. So one `worker.yaml` serves every stack.
  - `validate_bindings` accepts a kind unit of the root `config.yaml`
    wherever it accepts a directory unit.
- **Platform state goes to R2,** with `--state r2://<bucket>/platform`.
  - The bucket is `pigeon-platform-state`, which `infra/pigeon/` creates.
    It isn't any stack's, so tearing a stack down never takes its own
    state with it.
  - Its passphrase is the platform's own, never an instance's.
  - The private `noisypigeon/pigeon-platform-state` repository keeps
    `infra/pigeon/`'s state only. That unit shrinks over time to the
    account's bootstrap.

#### Execution of step 3, in two changes

- **3a. `feat(catalog)`:**
  - the four modules, with their `release-changelog.yaml` and
    `releases.yaml` entries;
  - the capabilities;
  - the `pigeon/platform` kind.

  No unit uses any of them yet, so 3a plans and deploys nothing.
- **3b. `feat(cobot)`:**
  - `deployment:` and `$deployment` in Worker specs;
  - kind units accepted by `validate_bindings`;
  - `--deployment` on `workers render|plan|deploy`;
  - the deploy job reading `DEPLOYMENT`.

  `worker.yaml` keeps its literals until step 4.

Step 4 then declares `platform/blue` with today's names, imports its
resources into the R2 state, and moves `worker.yaml` onto `$deployment`.
Step 4 is unchanged otherwise.

### 2026-11-09: Step 4 runs on the engine: `platform.plan` and `platform.apply`

No agent session holds an account token, a state passphrase or R2
credentials, and step 4 needs all three. Rather than have the owner apply
`platform/blue` by hand, this amendment brings forward the part of §5 that
lets the engine apply a stack under the owner's grant. Step 4's import is
then its first use, and §5's stand-up reuses it.

#### The operations

- **`platform.plan <unit>`** plans one `pigeon/platform` unit.
  **`platform.apply <unit> <plan>`** applies a stored plan of it, exactly.
- **Each is a Runner job,** as the deploy job is, and each is approved at
  the code tier, every time. An agent's call answers with the approval
  page's link.
- **The plan's report** lists each resource's address and action. Sensitive
  values stay hidden, as `cobot show` hides them. The apply grant binds
  the plan's ID and digest. A plan applies only within 24 hours, and only
  while the state hasn't moved since it was made.

#### The credential

- **The approval page for `platform.plan` takes an account token** beside
  the authenticator code. The owner makes it in the dashboard with:
  - Account Settings Read;
  - D1 Edit;
  - Workers R2 Storage Edit;
  - Queues Edit;
  - Access: Apps and Policies Edit;
  - Access: Service Tokens Edit.
- **The Worker verifies the token before accepting it:** active, and for
  this account only.
- **It is stored only for the run,** encrypted with AES-GCM under a key
  derived from `BACKPLANE_TOKEN`, in the run's D1 row. It is erased when
  the apply finishes, when the plan is refused, or after 24 hours,
  whichever comes first.
- **It reaches the job only inside the signed job spec,** and is never in a
  log, event, report or digest. The page reminds the owner to revoke it
  afterwards.

#### State

- **The bucket** is `pigeon-platform-state-<random>`.
  - The job finds it by that prefix with the owner's token.
  - If it doesn't exist, the job creates it with a random suffix.
  - Its full name is never in the repository.
- **R2 access** uses S3 credentials derived from the owner's token, as the
  deploy job derives them for builds. State lives under the prefix
  `platform/`.
- **The passphrase** is a new Worker secret, `PLATFORM_STATE_KEY`.
  - The owner sets it once, from 32 random bytes, and keeps a copy as
    break-glass.
  - It is the platform's alone, never an instance's.

#### Adopting existing resources by name

The repository never holds IDs, so the job looks them up. For each resource
the unit declares that isn't in state yet, the job finds it by its declared
name and writes an OpenTofu `import` block for it:

- the D1 database, by name;
- each bucket, by name or `name-<suffix>`. The suffix's `random_string` is
  imported from the name;
- the queue, by name;
- the Access application, its two policies and its service tokens, by
  name.

A name that matches more than one resource stops the plan. A name that
matches none is created.

- **Imported service tokens have no secret in state,** because Cloudflare
  shows a secret only once. That's fine for `platform/blue`, whose secrets
  are already set; §5 creates fresh ones.
- **The expected plan for `platform/blue`** imports everything and creates
  nothing. The only update allowed is the D1 no-op the `infra/pigeon/`
  README describes. Anything else means a name is wrong, and the owner
  refuses the apply.

#### Execution

- **4a. `feat(backplane)`:**
  - the two operations, and the credential on the approval page;
  - the Runner job;
  - adopting by name in `cobot`;
  - `PLATFORM_STATE_KEY` in `worker.yaml`'s secret list.
- **4b. `feat(infra)`:** `platform/blue` in the root `config.yaml`, with
  today's names. Then the owner sets `PLATFORM_STATE_KEY`, plans, and
  applies.
- **4c. `feat(infra)`:**
  - `worker.yaml` moves onto `$deployment`, bound to `platform/blue`;
  - `DEPLOY_OUTPUTS` comes from the stack's state;
  - the custom domain `control.pigeon.dev` is added.

### 2026-11-09: Step 4a lands: where runs live

Step 4a follows the amendment above, with three differences in the details:

- **Runs live in the `MainRun` object, not in D1.** `MainRun` already starts
  and polls a Runner job, knows `main`'s deployed commit, and has the alarm
  loop. So platform runs share that alarm, and never hold up a deploy. The
  sealed token sits in the object's storage with its run, and no migration
  is needed.
- **The apply's grant is offered when the plan finishes.** It is pushed to
  the owner with the plan's changes on its page. `platform.apply <run>`
  offers it again under a new link, which voids the old one.
- **Refusing needs no code,** because it only takes power away. It fails the
  run and erases the token. A lost job starts again, at most three times.

### 2026-11-09: Step 4b: `platform/blue` declared as today's Access is

Today's Access setup differs from step 3's kind, and the kind now follows
it:

- **The owners' policy admits the account's Cloudflare members.** It holds
  no email list, so no address is committed.
- **Each service token has its own Service Auth policy,** named in the unit
  beside the token.
- **The application is the Worker's own.** Its destinations are the
  Worker's production and preview URLs. A new `cloudflare/worker` module
  looks the Worker's ID up by name, so the ID never enters the repository.

Adopting by name reads a `for_each` item's other fields as well, such as a
token's policy name. `platform/blue` declares today's names exactly, so its
first plan should import every resource and create nothing.

### 2026-11-09: Step 4 adopted `platform/blue`; step 4c in three changes

Platform run 13 applied, so `platform/blue` now holds today's control
plane: its database, buckets, queue, Access application, policies and
service tokens. It imported everything and created nothing. Runs 1 to 12
established six rules on the way:

- **A plan shows its values.** Each change carries its attributes' values
  before and after, masked as `cobot show` masks them. An action and
  attribute names alone hid three of the problems below.
- **The account token also needs Workers Scripts Read,** for the Worker
  lookup.
- **The Worker is found by its immutable ID.** `cloudflare_worker`'s `id`
  echoes its input, so `cloudflare/worker` reads the ID from the
  `cloudflare_workers` list, and fails when no Worker has the name.
- **Access protects the Worker's production URL only.** The kind no longer
  adds a preview destination.
- **The application's login settings stay the dashboard's.** These are
  `allowed_idps`, `auto_redirect_to_identity`, `enable_binding_cookie` and
  `options_preflight_bypass`, which `cloudflare/access-application` ignores.
- **A service token carries its live secret version and previous expiry**
  (`secret_version`, `previous_secret_expires`). The provider defaults the
  version to 1 and requires the expiry with it, so leaving them out would
  lower a rotated token's version. Raising `secret_version` rotates the
  secret deliberately.

Step 4c is three changes:

- **4c-1. `feat(backplane)`: the stack's outputs come from its runs.**
  - A platform apply's job reads the unit's outputs from its state and
    returns them. `MainRun` keeps the newest outputs of each unit.
  - The deploy job's `DEPLOY_OUTPUTS` is the secret's snapshot, with each
    platform unit's outputs taken from `MainRun` in its place.
  - A plan or apply that finishes posts `platform.planned` or
    `platform.applied` to the event log, as a failure posts
    `platform.failed`.
  - `platform.plan` on a unit with an open run names that run.
  - `platform/blue` is then planned and applied once, with no changes, so
    `MainRun` holds its outputs.
- **4c-2. `feat(infra)`: `worker.yaml` binds to the stack.** It names
  `deployment: pigeon/platform` and takes these from `$deployment`:
  - its name;
  - the database's name and ID;
  - the CI bucket;
  - the queue;
  - `INSTANCE_NAME`, `CHANGE_FLOOR` and `PUBLIC_HOST`.

  The running Worker has no `DEPLOYMENT` var until this deploys. So the
  deploy job binds a stack-bound spec to the kind's only unit when
  `DEPLOYMENT` is unset, and refuses when there are several.
- **4c-3. `feat(infra)`: `control.pigeon.dev`.**
  - The kind gains an optional `domain`, which the Access application
    protects as a self-hosted hostname. That is applied first, so the
    hostname is covered before anything serves it.
  - Then `worker.yaml` adds the custom domain from `$deployment.domain`.
  - `PUBLIC_HOST` stays the `workers.dev` hostname until clients have
    moved.

### 2026-11-11: A platform may declare public paths, each a bypass Access application

[`2026-11-11-release-ios-app-buildkite-testflight.md`](2026-11-11-release-ios-app-buildkite-testflight.md) §5 needs one route reachable without an Access login: `POST /v0/apple/release`, which authenticates its caller itself with a Buildkite OIDC token. The kind can't express that: one application covers each hostname, and `cloudflare/access-policy` only allows.

- **`public_paths`, optional on the kind:** a top-level list of `{ name, paths }`, since a kind's `each` blocks iterate top-level lists. `paths` are the path on each of the stack's domains, as hostname and path (`platform.pigeon.dev/v0/apple/release`).
- **An absent list is empty,** for every `each` block, so a unit that declares no public path needs no new key.
- **Each entry is its own Access application,** whose destinations are those `paths` as public URIs and whose one policy bypasses Access for everyone. Access applies the most specific match, so only those paths skip the login, and every other path keeps the owners' application.
- **`cloudflare/access-policy` gains `bypass`** (bool, default false). With it the decision is `bypass` and the policy includes everyone; the module refuses it alongside `emails`, service tokens or `account_members`. The module goes to 0.2.0.
- **The Worker stays the gate.** A public path is served only by a route that authenticates the request itself. The Worker's rule that refuses a request without an Access token names each such route as its exception, beside `/builds/`; any other path under a public one answers 404.
- **Each public path is decided by an ADR,** which names the route and how it authenticates. Declaring one takes a platform apply, which needs the owner's grant.
- **`platform/blue` declares one:** `{ name: pigeon-control apple release, paths: [platform.pigeon.dev/v0/apple/release, blue.platform.pigeon.dev/v0/apple/release] }`.

#### Execution, in this order, so no path is public before its route can refuse

1. This amendment.
2. **`feat(catalog)`:** `bypass` on `cloudflare/access-policy`; `public_paths` on the kind, with its applications and policies under `each`; and an absent `each` list read as empty. `config.yaml` declares nothing yet, so the next plan changes nothing.
3. **The iOS ADR's step 4** merges and deploys: the route, its OIDC check, and its exception in the Worker. Behind Access, it is reachable only by the owner until step 4 here.
4. **`feat(infra)`:** `platform/blue` declares the path. The owner runs `platform.plan`, reads that it creates one application and one policy and changes nothing else, then `platform.apply`.

#### Verification

- The kind's tests: a `public_paths` entry renders one application and one bypass policy; none renders neither; `bypass` beside `emails` is refused.
- After step 4, without an Access login:
  - `POST https://platform.pigeon.dev/v0/apple/release` reaches the Worker, and with no OIDC token answers 401;
  - `GET https://platform.pigeon.dev/v0/apple/release/x` answers 404;
  - `https://platform.pigeon.dev/` and `/mcp` still redirect to the Access login.
