## The control plane: phase 4

* Date: 2026-11-05
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: runtime

## Context

Phase 4 of [`2026-11-04-development-platform-instance-separation.md`](2026-11-04-development-platform-instance-separation.md) builds the control plane: §4 (grants and minting), §6 (the control plane) and §7's phase 4 row. Its amendments have already settled four things:

- **"The control plane is Rust, built from backplane".** One crate, two builds: backplane's `worker` feature is the control plane, and cobot's `worker` feature does Container dispatch and minting.
- **"Worker–Container mutual TLS".** `serve` gained mutual TLS. Its §1 records the token, `state` and `--network` decisions.
- **"Phase 3 spike report".**
  - Every binding works from Rust.
  - The Worker reaches its Container over plain HTTP only, so the link uses signed requests (`serve --signed`).
  - A Container can be stopped at any time, so phase 4 builds every run to survive a stop, and proves it before any apply is dispatched.
- **[`2026-11-05-development-cobot-workers-area.md`](2026-11-05-development-cobot-workers-area.md).** The Worker and its Container deploy through `pigeon cobot workers`. OpenTofu keeps every declarative resource.

This ADR decides phase 4 in the detail its PRs need. Phase 5, migrating noisypigeon, starts once it is done.

## Implementation plan.

### 1. Builds

- **backplane `worker`**, built for `wasm32-unknown-unknown` with `workers-rs`, holds:
  - the HTTP API (§6) and the MCP server (§10);
  - the instance Durable Object (§3);
  - D1, R2 and Secrets Store access;
  - the drift Cron Trigger (§11).

  It decides and records. It never starts a Container or mints a credential itself.
- **cobot `worker`**, also wasm-safe, holds:
  - Container dispatch: starting a run's Container, and signing every request to it (§5);
  - minting and revoking each provider's credentials (§8).

  It carries none of cobot's process-starting code.
- **Shared types** (`Operation`, `Effect`, `Request`, `Response`, plan and run records) live in a module with no feature, so both builds compile one definition of the API.
- **CI.** `mise run features` gains a wasm32 clippy line for each `worker` build, and `rust.yml` builds the bundle on every Rust PR without deploying it.

### 2. Deploy

- **OpenTofu.** `infra/pigeon` gains, still applied locally by its owner:
  - the Cloudflare Access application and its service tokens;
  - DNS for `pigeon.dev`;
  - the Secrets Store secrets the Worker binds (`BACKPLANE_TOKEN`, `GRANT_KEY`), created empty, with values set at deploy.
- **wrangler, through cobot.** The control plane is `infra/pigeon/workers/control/worker.yaml`, deployed with `pigeon cobot workers deploy`. It binds:
  - the released backplane Worker bundle and the released backplane image;
  - the `Instance` Durable Object and the `Runner` Container;
  - D1, the state and plans R2 buckets, and the Secrets Store secrets;
  - the cron.
- **This supersedes** the separation ADR's §6 and amendment 1 §2 on how the Worker is deployed.

### 3. The instance Durable Object

There is one `Instance` Durable Object per instance.

- **One run at a time.** It serializes runs, holding the rest in a queue in its own storage.
- **Liveness.** For each run it holds pending I/O on the Container (`monitor()`) for the run's whole length, and sets `sleepAfter` above the longest run.
- **A lost Container.** When a run's Container stops before the run ends, the object:
  - re-dispatches a plan;
  - re-dispatches an apply with the same plan ID, so only its unapplied units run (§7);
  - records the attempt in `runs`.
- **Retries are bounded.** After 3 attempts a run fails, and the failure is recorded in `audit`.

### 4. D1 schema v0

The SQL migrations live in backplane, and `pigeon cobot workers deploy` applies them before deploying.

| Table | Holds |
| --- | --- |
| `instances` | ID, name, created, status. |
| `desired_state` | Instance, unit, version, the unit's `config.yaml` as JSON, author, created. One row per unit per version. |
| `plans` | ID, instance, desired-state version, plan digest, summary (addresses, actions and attribute names; never values), requester, created, expires. |
| `approvals` | Plan, approver, created. |
| `runs` | ID, instance, kind (plan or apply), plan, status, attempts, started, finished. |
| `grants` | Instance, provider, the grant encrypted under `GRANT_KEY` (§8), created, revoked. |
| `audit` | Time, actor, operation, instance, target and outcome. Never a value or credential. |

### 5. The link to the Container

- Each run's Container runs `pigeon backplane serve --network --signed`, with `BACKPLANE_TOKEN` from the Secrets Store.
- cobot `worker` signs each request, and verifies each response's signature against the request's nonce, per the spike report's §2.
- An unsigned or unverified answer fails the run. It is never retried as success.

### 6. Plan, approve, apply

The API routes mirror backplane's operations and answer in its `Response` form:

- **`POST /v0/instances/:id/plans`** dispatches a plan.
  - Its stored plan goes to R2, as phase 3's stored plans do, and its summary to `plans`.
  - It expires 24 hours after it is made.
- **`POST /v0/plans/:id/approvals`** records an approval. The approver is the identity Cloudflare Access verified. An approver may not approve their own plan unless the instance allows it.
- **`POST /v0/plans/:id/applies`** dispatches the apply of exactly that plan. It refuses a plan that is expired, unknown, unapproved, or already applied.
- **The other read operations** (`instance.show` and so on) answer from D1 without starting a Container.
- **`workers.*` operations** from [`2026-11-05-development-cobot-workers-area.md`](2026-11-05-development-cobot-workers-area.md) take the same plan, approve, apply path.

### 7. Runs that survive a stop

These are engine changes, released before the Worker relies on them:

- **cobot pushes as it goes.** Each unit's state and applied marker are pushed to R2 before the next unit starts.
  - Today `apply_stored` writes them to the local mirror, and backplane pushes the mirror only after the whole apply.
- **`serve` drains on SIGTERM.** It takes no new request, and lets a running `tofu` step finish within the 15-minute grace.
- **A re-dispatched apply** applies only the units without an applied marker.
- **A completed apply is recorded before its plan is removed.**
  - cobot writes a `complete` record to R2, holding the plan ID and its result summary, before deleting the plan's files.
  - A re-dispatched apply of a completed plan returns that recorded result as success, and applies nothing.
  - The record is kept until the Durable Object has recorded the run as finished.
- **A unit cut off by SIGKILL** mid-`tofu` may have changed resources its state does not record.
  - Its run stops and is reported to the owner, with a fresh plan of that unit.
  - It is never retried blindly.

### 8. Grants and minting

- **Where grants live.** A grant is encrypted with AES-256-GCM under `GRANT_KEY` (a Secrets Store secret) and stored in `grants`.
  - A Worker binds Secrets Store secrets by name at deploy, so it cannot add one per instance at run time.
  - This refines §4's "into the Secrets Store": the key lives there, not each grant.
- **Entering a grant.** `PUT /v0/instances/:id/grants/:provider`, owner only, takes the grant once.
  - It checks the grant against the provider before storing it.
  - It never returns the grant.
- **Minting** follows §4's table. It runs in cobot `worker`, at dispatch, from the run's capabilities.
  - The minted credentials reach only that run's Container environment, as `COBOT_CREDENTIALS`.
  - They are revoked when the run ends; DigitalOcean's are swept after 1 day.
  - Every mint and revocation writes an `audit` row: the capability names, the scopes and the minted ID, never the value.
- **Providers in order:**
  1. Cloudflare: an account token holding API Tokens Write, minting tokens with `expires_on` set to the run plus 1 hour.
  2. Scaleway.
  3. Backblaze.
  4. DigitalOcean.

  Each provider is its own PR.

### 9. Access and audit

- **No route answers without a verified Access identity**, from the first deploy on. The Worker verifies the Access JWT on every request, besides Access's own edge check.
- **Cloudflare Access** protects every API and MCP route.
  - People sign in.
  - The CLI and MCP clients use Access service tokens.
- **Every call writes an `audit` row**: the actor, operation, instance, target and outcome.

### 10. The MCP server

- JSON-RPC over streamable HTTP, on `/mcp`.
- Its tools are backplane's catalog operations, under the same names and with the same arguments.
- Reading and planning are open to any authenticated client. Approving and applying follow §6.

### 11. Drift

- A daily Cron Trigger dispatches a plan for every instance.
- A plan with changes is a deviation: it is recorded on the instance and in `audit`.
- This replaces `cobot.yml`'s daily apply at phase 6.

### 12. Execution

Each step is released before the next relies on it, and each passes `mise run ci`.

1. This ADR, and its `CLAUDE.md` pointer.
2. `feat(cobot)`: push per unit (§7).
3. `feat(backplane)`: `serve` drains on SIGTERM (§7).
4. `refactor(backplane)`: the shared types, and both `worker` builds compiling in CI (§1).
5. `feat(backplane)`: the `Instance` Durable Object and the D1 migrations (§3, §4).
6. `feat(cobot)`: signed Container dispatch (§5).
7. `feat(backplane)`: Access verification and audit on every route (§9).
8. `feat(backplane)`: the API for plans, approvals and applies (§6).
9. `feat(cobot)` and `feat(backplane)`: grant entry, and Cloudflare minting (§8).
10. `feat(infra)`: the Access application, DNS and the Secrets Store secrets in `infra/pigeon`, and the control-plane spec (§2).
11. The owner applies `infra/pigeon`, so Access is in place, then deploys with `pigeon cobot workers deploy`, and enters the Cloudflare grant.
12. `feat(backplane)`: MCP (§10), then drift (§11).
13. `feat(cobot)`: minting for Scaleway, Backblaze and DigitalOcean, one PR each.
14. The proofs in §13, recorded as an amendment here.

### 13. Verification

- An apply naming an expired, unknown, unapproved or already-applied plan is refused.
- A minted credential works only on its run's scopes, and stops working once the run ends.
- A run longer than 20 minutes completes.
- A Container killed mid-apply resumes, applying only its remaining units, against the same R2 store.
- A Container stopped between its final apply and its reply leaves a run recorded as succeeded, and nothing is applied twice.
- A request without a verified Access identity is refused on every route, including the first deploy's.
- A drift cron tick dispatches a plan.
- A `Response` serialized by the `engines` build deserializes in the `worker` build.
- `.config/policies/backplane.yaml` still holds: no dispatch or minting code in backplane's source.
- No grant, minted credential or ID appears in a response, a log or the repository.

## Amendments

### 2026-11-05: A run's instance root, built from D1

`serve` loads `.config/instance.yaml`, and the files it names, from `--root` before it answers. The backplane image holds only the binary and its tools. This amendment decides where a run's root comes from.

The separation ADR has already decided the direction:
- the platform holds each instance's desired state (decision 2);
- credentials live only in the Secrets Store (decision 4);
- an instance repository holds site content and a read-only export (its §1).

So a run never clones an instance repository, and no instance file is baked into an image.

#### 1. Two sources

| Part of the root | Owner | Source |
| --- | --- | --- |
| `.config/instance.yaml`; each unit's `config.yaml` (and `schema.json` where the unit holds its own); each `<unit>/sites/<name>/config.yaml` and `<unit>/workers/<name>/worker.yaml` | The instance | D1 `desired_state`, at the version the run names |
| `catalog/`, including its policies (2 below), `.config/providers.yaml` and `providers.lock.hcl` | The platform | The released backplane image |

- Nothing personal to noisypigeon is in the image. Nothing of the platform's is in an instance's desired state.
- No secret is in either. A unit names capabilities, and a run gets minted credentials in `COBOT_CREDENTIALS` (§8).
- A site's `site/` content is not desired state; 5 below leaves it open.

#### 2. Rules: the platform defines them, an instance invokes them

- **Definitions.** Every rule an instance can apply is a sentinel policy that the platform defines, in `catalog/policies/<name>.yaml`. It is released in the backplane image with the rest of the catalog.
- **Invocation.** An instance's `.config/instance.yaml` replaces `paths.policies` with `policies`, a list of invocations. For example, with illustrative names:

  ```yaml
  policies:
    - use: static-site-names
    - use: dns-zones
      with: { zones: [noisypigeon.com] }
  ```

  - An unknown name fails validation.
  - A parameter is checked against the schema the policy declares.
  - An instance cannot add, change or skip a rule beyond what its invocations say.
- **sentinel** gains parameterized policies (`with`) and checks invocations. It is released before any instance relies on it.
- **This repository's own `.config/policies/`** stay. They are the platform's rules for its own development (ADRs, pull requests, layout, engine boundaries), not an instance's.
  - The noisypigeon-specific ones, `secrets.yaml`, `secrets-apex.yaml` and `secrets-cloudflare.yaml`, leave in phase 7 with the `ENC` backend.
  - Any rule about noisypigeon's units that remains moves into the catalog as a definition, which noisypigeon's manifest then invokes.
- **The architecture site.** Per [`2026-11-05-runtime-architecture-site-through-backplane.md`](2026-11-05-runtime-architecture-site-through-backplane.md), `architecture.describe` draws `policy → target` edges from two sources:
  - this repository's own `.config/policies/`;
  - an instance's invocations, each pointing at the catalog policy it uses.

#### 3. The bundle

- **Writing it.** Before dispatch, the backplane `worker` build writes the instance's part of the root, at one desired-state version, as a bundle:
  - its format is a sorted tar;
  - it goes to the instance's state store, at `bundles/<sha256>`, which is content-addressed;
  - writing an existing digest is a no-op.
- **Recording it.** The plan records the bundle's digest, beside its desired-state version, in D1 `plans` and in the stored plan's metadata.
- **In the Container.** `Launch` names the bundle, not a bare root. Before `serve` starts, cobot fetches the bundle and checks its digest.
  - It unpacks the bundle into a fresh directory, and lays the image's platform part over it.
  - It then runs `serve --root` on that directory.
  - A bundle whose digest does not match refuses to start.
- **An apply** uses its plan's bundle digest, so it runs against exactly the files that were planned and approved.
- **Lifetime.** A bundle is kept while a plan or run references it, and is swept 7 days after the last of them ends.

#### 4. How desired state gets in

- **API.** `PUT /v0/instances/:id/desired-state` writes a new `desired_state` version.
  - The CLI calls it, as `pigeon instance push`; the MCP tool and the UI call it too.
  - Access verifies the caller (§9), and the call is audited.
- **Phase 5's import** is the first such write, from today's `.config/` and `workloads/`.
- **Who edits.** After phase 7, the owner may keep editing in the instance repository and push from the CLI. D1 stays the platform's record, and the repository receives the read-only export.

#### 5. Open: static site content

Phase 7 says "the platform builds sites from the instance repository", which would put a Git read credential back into a run. The alternative is for the CLI to upload site content as content-addressed bundles too. This is decided before phase 7, not here.

#### 6. Execution

Step 8 of §12 gains:
- the bundle writer;
- `PUT /v0/instances/:id/desired-state`;
- the plan's bundle digest.

Before step 8, each released first:
1. `feat(sentinel)`: parameterized policies and instance invocations.
2. `feat(backplane)`: `instance.yaml`'s `policies` invocations, which replace `paths.policies`. noisypigeon's manifest moves to them, with its rules moved into `catalog/policies/`.
3. `feat(cobot)`: the Launch step, which fetches, verifies and unpacks the bundle, then runs `serve`.

#### 7. Verification

- A bundle's digest is stable for a given desired-state version.
- A Container refuses a bundle with a wrong digest.
- An apply runs against its plan's bundle, even after a newer version is pushed.
- No bundle contains `ENC[`, a credential, an ID or a policy definition.
- An instance invoking an unknown policy, or passing a parameter its schema refuses, fails validation.
