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
7. `feat(backplane)`: the API for plans, approvals and applies (§6).
8. `feat(cobot)` and `feat(backplane)`: grant entry, and Cloudflare minting (§8).
9. `feat(infra)`: Access, DNS and the Secrets Store secrets in `infra/pigeon`, and the control-plane spec (§2).
10. The owner deploys with `pigeon cobot workers deploy`, and enters the Cloudflare grant.
11. `feat(backplane)`: Access and audit (§9), then MCP (§10), then drift (§11).
12. `feat(cobot)`: minting for Scaleway, Backblaze and DigitalOcean, one PR each.
13. The proofs in §13, recorded as an amendment here.

### 13. Verification

- An apply naming an expired, unknown, unapproved or already-applied plan is refused.
- A minted credential works only on its run's scopes, and stops working once the run ends.
- A run longer than 20 minutes completes.
- A Container killed mid-apply resumes, applying only its remaining units, against the same R2 store.
- A drift cron tick dispatches a plan.
- A `Response` serialized by the `engines` build deserializes in the `worker` build.
- `.config/policies/backplane.yaml` still holds: no dispatch or minting code in backplane's source.
- No grant, minted credential or ID appears in a response, a log or the repository.
