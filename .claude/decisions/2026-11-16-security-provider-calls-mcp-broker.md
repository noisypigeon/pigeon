## Provider calls go through MCP: one brokered operation, `provider.call`, checked like every other and credentialed per call

* Date: 2026-11-16
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: security
* Governs: `src/**/worker/github.rs`, `src/**/worker/buildkite.rs`, `src/**/worker/bitrise.rs`, `src/**/worker/apple.rs`, `src/**/worker/insights.rs`, `src/**/workers/cloudflare.rs`, `src/**/architecture/network.rs`

## Context

The owner's goal:

1. Every place pigeon calls a provider's API directly calls that provider's
   MCP server instead, where one exists and is suitable.
2. Where none is usable, this ADR records the alternative.
3. Every external MCP call goes through one pigeon operation. That operation
   runs the same checks as any other operation before it forwards the call.

It builds on:

- [`2026-11-15-runtime-one-mcp-entry-point.md`](2026-11-15-runtime-one-mcp-entry-point.md),
  for `run()` as the one authorizer and the registry's `asks`, `capability`
  and `doors`;
- [`2026-11-12-development-grants-applies-rescue-yield.md`](2026-11-12-development-grants-applies-rescue-yield.md),
  for `GRANTER` and `core::minting`;
- [`2026-11-14-security-provider-connect-browser-grants.md`](2026-11-14-security-provider-connect-browser-grants.md),
  for provider adapters;
- [`2026-11-15-runtime-architecture-coverage-agents-deploys.md`](2026-11-15-runtime-architecture-coverage-agents-deploys.md),
  for the outbound-host test.

### What ships today (artifacts/main, 2026-11-16)

**The one MCP entry point: only the ADR has merged (!605).**

- Its execution changes were all closed unmerged:
  - !626, the registry's `asks`, `capability` and `doors`;
  - !627, the route inventory;
  - !629, the session check;
  - !637 and !641, the bound app proofs.
- Briefs b42, b43, b45 and b47 have no merged change.
- `/mcp` still has none of its planned checks:
  - it mints an `Mcp-Session-Id` and never checks it;
  - it has no `GET` stream;
  - `/v0/events` is unaudited;
  - 19 of the 20 `/v0/app/*` routes bypass `run()`;
  - the app's proofs are unbound.

**Direct provider calls.** pigeon has no MCP client. Every provider call
below is plain HTTP, made in one of three ways:

- `worker::Fetch` in the Worker;
- `ureq` in Containers and the CLI;
- `curl` in shell scripts.

| # | Provider | Where | What | Credential today |
|---|---|---|---|---|
| G1 | GitHub REST | `worker/github.rs` (ChangeRun, MainRun) | dispatch `ci.yml`, list and get runs and jobs, job logs, artifacts, cancel, compare, get commit | an installation token minted per job from the App key |
| G2 | GitHub git | `cobot/git.rs` (Runner) | mirror push and branch delete | an installation token in the job spec |
| C1 | Cloudflare REST, credential plane | `worker/platform.rs`, `worker/minting.rs`, `worker/mod.rs` | mint, list, verify and revoke tokens; put Worker secrets with a helper token | `ARTIFACTS_TOKEN`, `GRANTER` |
| C2 | Cloudflare GraphQL | `worker/insights.rs` | Analytics, hourly | `OBSERVABILITY_TOKEN` |
| C3 | Cloudflare REST, admin | `cobot/cloudflare.rs`, `cobot/artifacts.rs`, `cobot/ship.rs`, `standup_job.rs` (Containers, CLI) | accounts, zones, R2 buckets, Artifacts repos, deployments, subdomain | `CLOUDFLARE_API_TOKEN` handed to the job |
| C4 | Cloudflare engines | `cobot/tofu.rs`, `cobot/workers.rs` (wrangler) | plan, apply, deploy, secrets, containers | the same, minted |
| C5 | Cloudflare data | R2 S3 (state, sites, mirror), registry push, Artifacts git | bytes | SigV4 keys, minted git tokens |
| B1 | Buildkite REST | `worker/buildkite.rs`, `worker/insights.rs` | create, get, cancel builds; jobs; artifacts; pipeline config | `BUILDKITE_TOKEN` |
| R1 | Bitrise REST | `worker/bitrise.rs` | trigger, get, abort builds; artifacts; `bitrise.yml` | `BITRISE_TOKEN` |
| A1 | App Store Connect | `worker/apple.rs` | bundle IDs, profiles, apps, builds | an ES256 JWT signed in the Worker |
| A2 | App Store Connect upload | `pigeon … signing upload` on the Buildkite agent | Build Uploads and presigned parts | an upload JWT from the Worker's grant |
| P1 | APNs, Web Push | `worker/apns.rs`, `worker/push.rs` | notification delivery | the APNs key, VAPID |
| V1 | Verifiers | Access JWKS, Buildkite agent JWKS | public key fetch | none |
| D1 | Build inputs | `Dockerfile`, `ci.yml`, `imagebuild.rs`, `ios/ci.sh`, `ios-sign.sh` | pinned tarballs, npm, rustup | none |

Anthropic's API is not called anywhere. `core::pool`'s gateway is config
only, and disabled.

**The coverage test has gaps.** `architecture/network.rs` reads only
literal `"https://` hosts in the backplane crate, and skips `cobot/`. So
these hosts are invisible to it:

- the Container and CLI calls;
- R2;
- the Access certs.

### Which providers have a usable MCP server (2026-11)

| Provider | Server | Auth for a headless caller | Fit |
|---|---|---|---|
| GitHub | GitHub's hosted remote server; toolsets `actions`, `repos` | a bearer token; whether it takes an App installation token is unconfirmed (step 2) | G1, except `compare`, which `list_commits` and `get_commit` cover |
| Cloudflare | `mcp.cloudflare.com/mcp`, the whole API, Code Mode or one tool per endpoint; domain servers such as GraphQL and observability | an API token | C2, C3 |
| Buildkite | `mcp.buildkite.com/mcp`; toolsets in `X-Buildkite-Toolsets`; a read-only mode; a token pass-through endpoint for headless agents | `BUILDKITE_TOKEN` | B1 |
| Bitrise | `mcp.bitrise.io`, Streamable HTTP | a PAT | R1 |
| Apple | none of Apple's; only community wrappers | — | none |

Hosted servers change their tools without versions; Buildkite says so.
That is why §3 pins each tool.

## Implementation plan.

### 1. What moves to MCP, and what stays

| # | Decision |
|---|---|
| G1 | GitHub's MCP server, through the broker. |
| C2, C3 | Cloudflare's MCP server, through the broker. It uses one tool per endpoint (`codemode=false`), never Code Mode's `execute`: a policy can read a tool name and arguments, not a script. |
| B1 | Buildkite's MCP server, through the broker. |
| R1 | Bitrise's MCP server, through the broker. |
| A1 | Apple has no usable server. pigeon serves **`asc`**, an in-process MCP shim over the existing client in `worker/apple.rs`. It is listed in the registry like a remote server, and called through the same broker. |
| G2, C5, A2 | **Stay: protocols, not provider APIs.** git smart HTTP, S3, registry pushes and Build Uploads parts move bytes. Their credentials are already minted per job or per call. |
| C1 | **Stays: the credential plane.** Minting and revoking are what the broker itself uses to credential a call. Routing them through the broker would make it its own dependency. |
| C4 | **Stays: the engines.** tofu and wrangler are the deploy path CLAUDE.md requires (`pigeon infra`, `pigeon workers`). They keep their minted tokens. |
| P1, V1, D1 | **Stay.** Delivery protocols, public-key fetches, and digest-pinned downloads have no MCP form and carry no provider authority. |

Every row that stays is listed as an exemption in the registry, with its
evidence, so no direct call is left unaccounted for (§6).

### 2. The broker: `provider.call`

- One registry operation:
  - its arguments are `provider.call {server, tool, arguments, change?}`;
  - its engine is the control plane;
  - its door is `/mcp` only.
- `provider.tools {server}` is a read-only companion. It answers the
  server's live `tools/list` against the registry's pins.
- **In-process callers use the same function.** ChangeRun, MainRun, the
  macOS failover and insights call `provider::call` directly. Their caller
  is the platform's account, so the same checks and events apply as on the
  wire.
- **Containers and the CLI call it over `/mcp`**, with the job's signed
  token or the session's ticket. They no longer get
  `CLOUDFLARE_API_TOKEN` for REST.
- **The client.**
  - `core::provider::mcp` is a pure Streamable-HTTP client:
    - `initialize`;
    - `tools/list`;
    - `tools/call`;
    - session-ID handling.
  - The Worker supplies its `Fetch`.
  - The `asc` shim answers the same messages in-process.

### 3. The registry: `.config/providers/mcp.yaml` (guarded)

Each server entry names:

- `url`;
- the credential's **kind**: `github-installation`, `cloudflare-minted`,
  `secret:<NAME>` or `asc-jwt`. It holds names only, never a value;
- `headers`, such as toolsets or read-only;
- `tools`. Each listed tool has:
  - `effect`: `read`, `write` or `apply`;
  - the `capability` a caller needs;
  - the credential scope it needs. For GitHub that is the installation
    permissions; for Cloudflare, the permission groups;
  - `schema`: the SHA-256 of its `inputSchema`.

A tool the registry doesn't list is refused. A listed tool whose live schema
hash differs is also refused, with `provider.drift`, until the registry is
updated.

`exempt:` lists §1's rows that stay, each with its code evidence, for §6.

### 4. The checks, in `run()`'s order, before anything leaves

1. **Who, membership and capability.** These are the members ADR's §8 steps,
   with the tool's `capability`.
2. **Ticket.**
   - An agent's call carries a live ticket.
   - A call naming `change` must name a change the session holds.
   - A change that is evicted or closed, or a ticket that is revoked or
     lapsed, is refused. These are the eviction rules: only the control
     plane evicts, and an evicted change's calls stop at once.
3. **Tier, by effect:**
   - `read` runs on the capability;
   - `write` needs the change's code tier, or a window, mandate or policy
     grant that covers it;
   - `apply` is owner-tier, so it is `pending`, then a `_meta` proof, per
     the one-MCP ADR's §3.
4. **Policy.** `policy.check` runs `.config/policies/provider-calls.yaml`
   (guarded) on a call document: `{server, tool, effect, arguments, caller,
   change}`. For example:
   - GitHub writes only to the mirror repository;
   - a Cloudflare tool only on the instance's accounts and the zones the
     instance lists;
   - Buildkite only on the two named pipelines;
   - no tool's arguments carry a secret-shaped value.
5. **Credential, per call.**
   - A GitHub installation token is minted with exactly the tool's
     permissions, for one repository.
   - A Cloudflare token is minted from `GRANTER` with the tool's permission
     groups and a 5-minute expiry, and revoked after the call.
   - Buildkite's and Bitrise's tokens stay Worker secrets, injected as a
     header.
   - The `asc` JWT is signed per call.

   A credential never enters an argument, a result, an event or a
   workload.
6. **Record.**
   - A forwarded call writes `provider.called`: server, tool, effect,
     caller, change, outcome and duration, with a SHA-256 of its
     arguments.
   - A refusal writes `provider.refused`, with the step and reason. It shows
     in the digest.

A tool result is returned as data. Nothing in it is instruction to the
caller.

### 5. Credentials and the owner

Minting Cloudflare tokens per call widens what `GRANTER` mints, up to the
union of the registry's permission groups. That is a credential grant, so
it is the owner's.

- It is asked with `owner_ask`.
- It is granted on `/grants/<id>` with a passkey.
- The grants ledger records it.

Until it is granted, C2 and C3 run on the existing tokens through the broker.

### 6. Coverage

- `architecture/network.rs` gains MCP servers as services.
- Its extractor reads every crate, `cobot/` included, plus shell scripts,
  `Dockerfile` and CI. It also reads hosts built with `format!` from
  `const` bases.
- `tests/architecture.rs` fails on any provider host that is neither:
  - an MCP server in the registry; nor
  - an `exempt:` row whose evidence matches its code.

### 7. The one MCP entry point is finished first

`provider.call` needs `run()` as the authorizer and the registry's `asks`,
`capability` and `doors`. Those are the one-MCP ADR's steps 1 to 3. So that
ADR's steps 1 to 5, 8 and 9 are re-briefed against the new crate layout and
land first.

- Its step 6 (the app's `MCPClient`, TestFlight) and its step 10 need the
  owner's phone.
- Its step 7 (the on-device coordinator) is not a dependency.

### Execution

Each step is its own change. "(planner)" steps change guarded files or
settle a boundary. The other steps are briefs. Code briefs start after the
src/ restructure's step 12. Each step that adds a file amends `Governs` with
its glob (`src/**/provider/**`, `src/**/worker/provider.rs`,
`.config/providers/mcp.yaml`, `.config/policies/provider-calls.yaml`).

1. **This ADR.** It comes with an amendment to the one-MCP ADR recording
   what shipped, and its re-briefed steps. (planner)
2. **The one-MCP ADR's steps 1 to 5, 8 and 9**, in its order: the
   inventory; the registry; `run()` as the authorizer; the session check,
   the `GET` stream and `_meta` proofs; the app operations and shims;
   `/v0/app/mcp`; the web shell; the follower on `/mcp`. (briefs; the
   Worker parts need the owner's approval)
3. **`core::provider`:**
   - the registry's types, parse and validation;
   - the call document;
   - the pure decision (steps 1 to 4 of §4);
   - the MCP client's message codec;
   - table tests. (brief)
4. **The registry and policy files:**
   - `.config/providers/mcp.yaml`, with no tools yet;
   - `provider-calls.yaml`;
   - `approvals.yaml` guards both. (planner, owner approves)
5. **`provider.call` and `provider.tools` in the Worker:**
   - the Fetch transport;
   - credential minting by kind;
   - the `provider.*` events;
   - the `asc` shim.

   (brief; Worker, owner approves)
6. **A live `provider.tools` for each server.**
   - The tools and schema hashes are recorded in the registry.
   - Step 2's unknowns are settled: does GitHub's server take an
     installation token, and are Bitrise's and Buildkite's artifact tools
     there?
   - A server that fails gets a shim here, as `asc` has.

   (planner, owner approves)
7. **Migrations, one brief each.** Each moves a module's calls to
   `provider::call` and deletes the direct client:
   - Buildkite (`macos.rs`, `insights.rs`, `main_run.rs`);
   - Bitrise;
   - GitHub Actions (ChangeRun, MainRun);
   - Cloudflare Analytics;
   - App Store Connect through `asc`.
8. **Containers and the CLI.**
   - `cobot` REST moves to `provider.call` over `/mcp`.
   - Jobs no longer get `CLOUDFLARE_API_TOKEN` except for tofu and
     wrangler.
   - This follows §5's grant. (brief)
9. **Coverage:** §6's extractor and test. (brief)
10. **Deploy and verify live.**
    - Each migrated path runs once: a change's CI dispatch, the hourly
      insights, a macOS build on each provider, and `ios.status`.
    - Each shows `provider.called`.
    - A refused call shows `provider.refused`.

    (planner)
11. **This ADR is amended with the outcome.** (planner)

## Amendments

### 2026-11-16: Reviewed against !757: they agree

!757 amends
[`2026-11-15-runtime-one-mcp-entry-point.md`](2026-11-15-runtime-one-mcp-entry-point.md)
with what shipped. It changes no code and none of this ADR's decisions:

- **It records the state §7 assumes.** Only that ADR merged. Its steps 1
  to 3 are not on `main`, so `provider.call` has no authorizer to sit
  behind yet.
- **It orders the same way.** That ADR's steps 1 to 5, 8 and 9 are
  re-briefed after the src/ restructure's step 12, and land ahead of this
  ADR's step 3. This is this ADR's execution step 2.
- **No shared file.** The two ADRs' `Governs` globs name no file in common.
  When `provider.call` is registered (step 5), it is one more operation with
  `asks`, `capability` and `doors` (`/mcp` only), and adds no route.

Neither ADR changes.
### 2026-11-16: Review of !713 against this ADR: they agree

!713 is step 5 of
[`2026-11-16-development-src-crates-neutral-names.md`](2026-11-16-development-src-crates-neutral-names.md).
It moves the policy engine from `src/pigeon-cli/crates/sentinel` to
`src/pigeon-eviction` and renames its package. It touches none of the files
this ADR governs. Its other files are the workspace's bookkeeping: `Cargo.toml`,
`Cargo.lock`, `.config/releases.yaml` and the behaviour ledger. This ADR's
`src/**/…` globs match the files the restructure moves later, both before and
after the move. So neither decision changes.
### 2026-11-16: Reviewed against the src/ restructure's !736 (mise.toml goes); they agree

!736 carries out step 10e of
[`2026-11-16-development-src-crates-neutral-names.md`](2026-11-16-development-src-crates-neutral-names.md)
(§4). It touches none of the files this ADR governs.
- **Build inputs (D1).** With `mise.toml` gone, Rust's version is pinned in
  `/rust-toolchain.toml` (!728). rustup and `ios-sign.sh` read it, and a test
  checks the images' `rust:` tags against it. Tofu and Zola stay pinned in
  the `Dockerfile`'s `ARG`s and `ci.yml`'s `env`; Node, wrangler and
  worker-build in the `Dockerfile` alone. No download, host or credential is
  added. D1 stays an exemption.
- **Coverage (§6).** The extractor reads the `Dockerfile` and CI as planned.
  `rust-toolchain.toml` names no host, so it needs no reading.
- **Order.** Neither waits for the other.
### 2026-11-16: Review of !746 against this ADR: they agree

!746 carries out part of step 6 of
[`2026-11-16-development-src-crates-neutral-names.md`](2026-11-16-development-src-crates-neutral-names.md)
(§2): it rewrites import paths to name `core::` modules and deletes the 14
three-line re-export shims. It touches none of the files this ADR governs
(`worker/{github,buildkite,bitrise,apple,insights}.rs`, `cobot/cloudflare.rs`,
`architecture/network.rs`), so no provider call, route or credential changes,
and the `src/**/…` globs still match. Neither decision changes.

### 2026-11-16: The owner's answers: per-call Cloudflare tokens, the vendors' hosted servers, and a github shim if needed

The owner answered the three questions filed on !756:

- **#12: Cloudflare tokens are minted per call.** Each Cloudflare tool call
  gets a token minted from `GRANTER`, holding only that tool's permission
  groups. The token expires in 5 minutes and is revoked after the call (§4).
  - The widened set is a credential grant. The owner approves it on
    `/grants/<id>` with a passkey, and the grants ledger records it (§5).
  - Until that grant exists, Cloudflare tools run on today's tokens, as §5
    says.
- **#13: the vendors' hosted servers.**
  - Buildkite goes to `mcp.buildkite.com` (its token pass-through endpoint,
    with the `builds` and `pipelines` toolsets).
  - Bitrise goes to `mcp.bitrise.io`.
  - Both use the `BUILDKITE_TOKEN` and `BITRISE_TOKEN` the Worker already
    holds, injected as a header per call.
- **#14: a `github` shim if the hosted server refuses App tokens.**
  - Step 6 tries GitHub's hosted server with a per-call installation token.
  - If it is refused, `github` becomes an `inproc:github` server: an
    in-process shim over `worker/github.rs`'s client, with the same per-call
    installation tokens.
  - No personal access token is created.

**The inventory's last providers.** DigitalOcean, Scaleway and Backblaze have
no direct call in pigeon; they are reached only through tofu's provider
modules in `catalog/`. That is row C4's engine exemption, so each becomes an
`exempt:` entry.

- DigitalOcean has an official hosted MCP server. Backblaze has an official
  local (stdio) one. Scaleway has none.
- A direct call added later goes through `provider.call`.

### 2026-11-16: Reviewed !802 against !801: they agree

!801 amends
[`2026-11-15-security-owner-approvals-always-listed.md`](2026-11-15-security-owner-approvals-always-listed.md):
`approval_wait` hands out the same page grant while a pending approval is
unchanged. !802 (this ADR's step 3) touches none of the files that ADR
governs. It adds only `src/pigeon-core/src/provider/**` and one line in
`lib.rs`: pure code that nothing calls yet.

- **Where they will meet.** An `apply` tool's call waits as `pending` for
  the owner's proof (§4 step 3). When step 5 wires `provider.call` into
  `run()`, that pending answer lists its grant through the always-listed
  path that !801 changed. Its link is then reused while the call's head,
  operation and tier are unchanged, as !801 decides. No other rule applies
  to it.
- **Unchanged.** A grant stays bound and single-use. A moved head still
  refuses the old link. `admit` and `verdict` read no grant.

Neither ADR changes.

### 2026-11-17: Step 5's shape: two operations in the control plane, secret credentials first, and policy.check in a step of its own

The Worker's dispatch path decides how §2 and §4 land.

- **The operations.**
  - `provider.call` and `provider.tools` are control-plane operations:
    - `provider.` joins `wire::Operation::control_plane`;
    - `control::route` gains `Routed::Provider`;
    - `dispatch` calls `worker/provider.rs`.
  - `arguments` is JSON text, because `wire::Kind` holds no object.
- **The caller's capabilities.** Until the one-MCP ADR's §8 authorizer
  lands:
  - every caller holds `read`;
  - a person, or a caller with a live ticket, also holds `propose`;
  - the platform's own calls (ChangeRun, MainRun) are `platform`.

  A tool's `capability` names a member capability: `read`, `propose` or
  `platform`.
- **Change state** comes from the change record:
  - `evicted` set → Evicted;
  - otherwise `Merged` → Merged, `Closed` → Closed;
  - anything else → Open.
- **Tiers.**
  - A write is covered when the named change is held and its plan is
    approved.
  - `apply` stays `pending` until the one-MCP ADR's `_meta` proofs land.
- **Credentials.** Step 5 wires `secret:NAME` only, which covers Buildkite
  and Bitrise. `github-installation`, `cloudflare-minted` and `asc-jwt`
  answer "not wired yet" until their migration step in step 7.
- **Drift.**
  - `provider.tools` compares the live schemas with the pinned ones.
  - `provider.call` passes no live schema, so it makes no second round trip
    per call.
- **`policy.check` gets a step of its own, 5b.** It runs the eviction
  engine, built for wasm or run on the Checker, against §4's call document.
  Until then the gate is the registry's pinned tools plus the verdict.
- **Step 5 splits:**
  - 5a: pure pigeon-core in `provider/call.rs`. It builds the caller, the
    call, the request headers, the event summaries and the MCP exchange.
  - 5b: `policy.check`.
  - 5c: the Worker glue, the catalog entries and the dispatch arm.
### 2026-11-17: Review of !853 against the merge-path ADR (!849): they agree

[`2026-11-16-development-merge-path-measured-faster.md`](2026-11-16-development-merge-path-measured-faster.md)
(!849) governs `change/queue.rs` and `macos/job.rs`. !853 is this ADR's
step 4 (registry file).

- **What it touches.** It adds `.config/providers/mcp.yaml` and a test that
  parses it, and touches neither of those files.
- **Exempt rows.** The file's Buildkite and App Store Connect upload rows
  list direct calls as exempt. They change no merge turn, queue place or
  macOS gate.

Neither ADR changes.
### 2026-11-17: What .config/approvals.yaml guards for the broker

§3 makes the registry guarded. `.config/approvals.yaml` gains three paths:

- **`guarded`:**
  - `.config/providers/**`, the registry: which servers and tools may be
    called, and with what credential kind;
  - `src/pigeon-core/src/provider/**`, the verdict and the registry parser
    that decide what leaves.

  Neither joins `gate`, so an Auto window may cover them, as it does other
  guarded paths.
- **`secrets`:** `src/pigeon-agent/src/worker/provider.rs`, which reads a
  provider credential per call (§4, step 5c). Every change to it is the
  owner's, as for `worker/platform.rs` and `worker/apple.rs`.
### 2026-11-17: Review of !853 against the capacity pools' step 3 and the AI Gateway ADR (!850): they agree

[`2026-11-15-development-capacity-pools-hard-spend-cap.md`](2026-11-15-development-capacity-pools-hard-spend-cap.md)'s
!850 rebuilds the pool's step 3 on
[`2026-11-16-security-ai-gateway-claude-api.md`](2026-11-16-security-ai-gateway-claude-api.md).
!853 is this ADR's step 4: the registry file.

- **The exemption matches.** !853's `ai-gateway-messages` exemption names
  `src/pigeon-core/src/pool` as its evidence. That is the same code !850
  adds to the pool ADR's `Governs`. Both say model traffic stays on the
  pool's route, outside `provider.call`.
- **The check matches.** !850's route calls `verdict::admit` (b59) before
  admission, which is the shared check this ADR's 2026-11-16 answer to the
  AI Gateway planner names.
- **What it leaves alone.** !853 touches none of `jobs.rs`, `brief/mod.rs`,
  `insights/mod.rs` or `pool/**`.
### 2026-11-17: Review of !853 against the capacity pools' !862: they agree

[`2026-11-15-development-capacity-pools-hard-spend-cap.md`](2026-11-15-development-capacity-pools-hard-spend-cap.md)'s !862 governs `jobs.rs`, `brief/mod.rs` and
`insights/mod.rs`. !853, this ADR's step 4 (registry file), adds only
`.config/providers/mcp.yaml` and a parse test, and touches none of them.

Its `ai-gateway-messages` exemption keeps model traffic on the pool's
route, outside `provider.call`, as the pool ADR has it.

Neither ADR changes.
### 2026-11-17: Step 5b's shape: a `provider-call` target in the policy engine, linked into the Worker

§4 step 4 runs `policy.check` on each forwarded call. The Worker doesn't
link the policy engine today; the Checker runs it. A Checker round trip per
call is too slow, so the engine is linked into the Worker.

- **The target.** `pigeon-eviction` gains `target: provider-call`, judged
  like `eviction`, against a flat document of text fields:
  - `server`, `tool`, `effect` (`read`, `write` or `apply`);
  - `caller`, the caller's account;
  - `change`, the named change's number, or empty;
  - `arguments`, the arguments as compact JSON with sorted keys.

  Only `field` rules apply. A field the document lacks is a violation, so
  missing data never passes.
- **Scoping.** A `provider-call` policy may carry `when:`, `field` rules on
  the same fields. It applies only when every `when` rule holds. Every
  applying policy must pass; the first failing one refuses the call.
- **Loading.** The engine loads a policy from text, for wasm. The policies
  are `.config/policies/provider-call-*.yaml`, built into pigeon-core by an
  explicit list. A test fails when the list and the directory differ. A
  policy change ships with the Worker's next deploy, so the Worker judges
  with main's policies, as the factory-crates ADR's judge does.
- **The verdict.** `verdict::Step` gains `Policy`. It runs after Tier and
  before the credential, as §4 orders it. A refusal is a
  `provider.refused` with step `policy` and the policy's name.
- **Repository checks skip it.** `policy.check` on the tree ignores
  `provider-call` policies, as it ignores `eviction` ones.
- **The first policy.** `provider-call-mirror.yaml`: a GitHub write names
  only the mirror repository. The others (Cloudflare accounts and zones,
  Buildkite pipelines) land with their servers in step 6, when their tools'
  argument names are pinned.
- **Who writes what.**
  - 5b-1, a brief: the engine's target, `when`, loading from text and the
    judge, with tests.
  - 5b-2, the planner: pigeon-core's policy list and `Step::Policy`, the
    Worker hook in `worker/provider.rs`, and the first policy. These are
    guarded or secrets paths; the owner approves `worker/provider.rs`.
### 2026-11-17: Hand-off: where this ADR's execution and the one-MCP ADR's stand

The planner session that ran both ADRs ends here, at the owner's request,
with no change open. A fresh planner resumes from this list.

- **Merged, this ADR:**
  - steps 1 and 3 to 5: the ADR, `core::provider` (registry, verdict,
    client) and `.config/providers/mcp.yaml` with no servers (!853);
  - `approvals.yaml` guarding both (!889);
  - 5a, `provider/call.rs` (!869);
  - 5c, `provider.call` and `provider.tools` in the Worker (!966);
  - 5b, the judge's `provider-call` target (!990), pigeon-core linking it
    ([`2026-11-10-development-factory-crates-functional-core.md`](2026-11-10-development-factory-crates-functional-core.md)'s
    2026-11-17 amendment, !1000), and the Worker's policy step (!1010).
- **Merged, the one-MCP ADR's step 2 here:** the route inventory (!852) and
  the session ID bound to its door and caller (!851).
- **Next, in order:**
  1. 5b's first policy, `.config/policies/provider-call-mirror.yaml`, once a
     judge release reads `target: provider-call`. Until then `policy.check`
     refuses the file. The text:

     ```yaml
     description: A GitHub write names only the mirror repository; see .claude/decisions/2026-11-16-security-provider-calls-mcp-broker.md §4.
     target: provider-call
     when:
       - field: { name: server, equals: github }
       - field: { name: effect, equals: write }
     rules:
       - field: { name: arguments, matches: '"repo":"noisypigeon/pigeon"' }
     ```

     The same change adds it to `provider::policy::POLICIES` and raises
     `every_policy_is_valid`'s count by one.
  2. Step 6: live `provider.tools` for Buildkite and Bitrise, then their
     servers and pinned tools in the guarded `mcp.yaml`. Settle step 2's
     unknowns there, and file the grants-ledger entry for per-call
     Cloudflare tokens.
  3. Steps 7 to 11, as written.
  4. The one-MCP ADR's rest:
     - the §8 authorizer in `run()`;
     - the bound app proofs (`core/approval`, a gate path);
     - `_meta` proofs;
     - the `GET` stream and resources;
     - the app operations and `/v0/app/mcp`;
     - the web shell;
     - the follower on `/mcp`.

     Steps 6, 7 and 10 still need the owner's phone.
- **Who carries what.** A brief's change can't touch a secrets path such as
  `worker/provider.rs`, an eviction path such as `src/pigeon-eviction/**`,
  or the provider paths. Brief only the code around them, and let the
  planner carry those files. !934 and !989 were refused for this, and the
  planner re-proposed their commits as !966 and !990.
