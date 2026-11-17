## A platform run's account token is minted by `granter` with its unit's exact capabilities, under the plan's passkey approval

* Date: 2026-11-17
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: security
* Governs: `catalog/cloudflare/modules/*/module.yaml`, `src/**/platform/platforms.rs`, `src/**/worker/platforms.rs`, `.config/grants.yaml`

## Context

The owner's goal: "These plan tokens are generated with the scopes they need
through the platform and I approve that operation as part of the plan
approval."

- **Today** a `platform.plan` run's page takes an account token the owner
  makes by hand in the dashboard, from a list on the page
  ([`2026-11-08-runtime-blue-green-self-standup.md`](2026-11-08-runtime-blue-green-self-standup.md),
  its 2026-11-09 step 4 amendment). It is sealed for the run and erased
  after the apply, a refusal or 24 hours.
- **On 2026-11-17 it failed three ways.** Run 31 got a 403 on
  `data.cloudflare_secrets_stores`, because the list lacked Secrets Store
  Read. Run 32 got a 403 listing R2 buckets, because a scope was missed in
  the dashboard. And the list lacked AI Gateway Edit, which the
  `ai-gateway` module needs. A hand-made token drifts from what the stack
  needs.
- **!904 and !905 are the stopgap:** the page's list comes from a table of
  the kind's modules, pinned by a test; the token is still hand-made.
- **The grants ADR already decided where this goes**
  ([`2026-11-12-development-grants-applies-rescue-yield.md`](2026-11-12-development-grants-applies-rescue-yield.md)
  §1): "`platform.plan`'s pasted token … become[s] [a] minted grant". Its
  ledger has `from-unit` capabilities and a `job` route for exactly this,
  and its 2026-11-13 amendment left `from-unit` mints waiting for step 4's
  unit runs. Those runs now exist.
- **The minting path works:** `granter` minted `observability` through
  `core::minting::token_body`, `settle` and the Worker's `minting.rs`.

## Implementation plan.

### 1. Each module declares its capabilities

- **`module.yaml` gains `capabilities`:** a list of names from
  `catalog/<provider>/capabilities.yaml`, the least each module needs to
  plan and apply. Modules that call no API, such as `agnostic/*`, declare
  `[]`. The field is required on every `cloudflare/*` module.
- **The catalog gains, account-scoped:** `account.read` (Account Settings
  Read), `workers.read` (Workers Scripts Read), `secrets_store.read`
  (Secrets Store Read), `aigateway.write` (AI Gateway Write),
  `access_apps.write` (Access: Apps and Policies Write) and
  `access_tokens.write` (Access: Service Tokens Write).

  The `pigeon/platform` kind's modules then use `account.read`,
  `workers.read`, `d1.write`, `r2.write`, `queues.write`,
  `access_apps.write`, `access_tokens.write`, `secrets_store.read` and
  `aigateway.write`. `access.write` stays for its existing users.
- **This replaces !905's `MODULE_PERMISSIONS` table.** Its test is kept,
  reading the manifests instead: every module the kind expands declares
  `capabilities`, and each one names a catalog capability.

### 2. A unit's capability set

- **`core::platform::platforms::capabilities(kind, config)`** is the union
  of:
  - each singleton's module's capabilities;
  - each `each` module's capabilities, only when its list in the unit's
    config is non-empty;
  - `r2.write`, for the state bucket and the S3 keys derived from the
    token.

  It is sorted and deduplicated, each capability resolved to its permissions
  and scope from the catalog; a unit with no `ai_gateways` gets no
  `aigateway.write`.
- **It is the platform's counterpart of `x-pigeon.credentials`,** which the
  zone, static-site and git-repo kinds declare
  ([`2026-11-04-development-platform-instance-separation.md`](2026-11-04-development-platform-instance-separation.md),
  phase 3). The platform's provider still reads `CLOUDFLARE_API_TOKEN`, so
  its kind declares no `credentials`; moving it onto them is out of scope.

### 3. The ledger grant

```yaml
platform-run:
  mint: { capabilities: from-unit, ttl: 24h }
  route: { job: platform.plan }
```

- **No `rotate`:** it lives for one run, and `ttl` equals the run's `LIFETIME`.
- **The run's apply reuses the sealed token,** as today, so the `job` route
  covers both.
- **The deploy job resolves `from-unit`** for each `pigeon/platform` unit
  on `main`, and reports each unit's capabilities with their permissions
  in `Report.ledger`. The Worker mints exactly what `main` deployed, and
  never reads a branch.

### 4. One approval: the plan and the mint

When a run is `Offered`, `GRANTER` resolves, and the deployed ledger
declares `platform-run`:

- **The page shows, beside the plan grant,** the token to be minted: each
  capability with its permission names, its account scope, its expiry, and
  that it is revoked when the run ends.
- **The passkey proof binds what was shown.** The purpose is the grant
  token plus a digest of the capability set. If the set changes between
  showing the page and proving, the proof fails.
- **One proof does both.** In the Worker, in order:
  1. read `granter`;
  2. resolve the permission groups;
  3. `token_body`;
  4. mint;
  5. `/tokens/verify`;
  6. seal into the run, as a pasted token is sealed today;
  7. record on the run `minted: { token_id, capabilities, expires }`;
  8. post `grant.granted`, with the run, the token ID and the
     capabilities.
- **A failure seals nothing:** `settle` revokes any token already minted;
  the page refuses with the reasons (`token_body`'s name the account's real
  groups); the run stays `Offered`; `grant.failed` and a `grant-failed` task
  follow.
- **The value stays in the Worker and the job,** never in the run's view, an
  event, a log, the digest or an agent's answer.

### 5. Revocation

- **When a minted run ends** (applied, failed, refused or expired), the Worker
  revokes its token ID through `granter`, posts `grant.revoked`, and erases
  the sealed copy as today.
- **A failed revocation** opens a `grant-failed` task; the 24-hour expiry
  still ends the token.

### 6. The pasted token is break-glass

- **The paste field stays.** It is required when minting is unavailable
  (`GRANTER` unbound or unverified, or no `platform-run` in the ledger), and
  otherwise sits empty under "Break-glass: paste a token instead". A pasted
  value is verified and sealed as today, skips the mint, and is made from
  the page's list of §2's permissions.
- **The run records `credential: minted | pasted`.** `platform.planned`
  carries it, so a break-glass run is visible in the digest.

### 7. The job fails fast on a moved `main`

- **The plan job computes §2's set at its own SHA.** If that set isn't
  within the run's `minted.capabilities`, it fails before OpenTofu runs. It
  names the missing capabilities and says "deploy `main`, then plan
  again". A pasted token isn't checked.
- The merge-speed session's fix for a stale `main` head in platform plan
  runs touches the same job. Whichever lands second rebases onto the
  other.

### 8. Tier

- **A run that mints is a credential grant:** owner tier, by passkey on the
  page, never covered by Auto, a window, a mandate or a policy
  ([`2026-11-15-security-standing-auto-owner-flags.md`](2026-11-15-security-standing-auto-owner-flags.md)).
  A test pins that no non-passkey path grants a platform run.
- **Every change below touches gate or secret paths,** so each merges on
  the owner's passkey.

### 9. `granter`'s reach: open, checked by the first mint

- **`granter` holds Account API Tokens Write,** and minted `observability`.
- **Not yet known:** whether Cloudflare lets it mint Access, Secrets Store
  and AI Gateway groups on an account-owned token, and whether a minted
  token is capped at `granter`'s own permissions. This ADR doesn't widen it.
- **The first minted run is the check.** An unknown group name fails in
  `token_body` before anything is minted; a Cloudflare refusal is shown on
  the page, and the owner can paste instead.
- **If a group is refused for `granter`'s reach,** the planner files an
  `owner_ask` (`decision`): (a) the owner adds exactly the refused groups to
  `granter` in the dashboard; (b) a second, platform-only granter in the
  store; or (c) platform runs stay on the pasted token.

### 10. Execution

Each step is a brief for an executor, reviewed by this ADR's planner, and
merges on the owner's passkey.

1. **`feat(catalog)`:** §1's catalog entries and `capabilities` fields, the
   manifest's parse, and §2's `capabilities(kind, config)`, replacing
   `MODULE_PERMISSIONS`; the page lists the unit's set. After !905 and !903.
2. **`feat(grants)`:** §3's ledger entry, the deploy job's per-unit
   `from-unit` in `Report.ledger`, and a ledger check that a
   `platform.plan` route takes `from-unit`.
3. **`feat(agent)`:** §4 to §7 in `approve.rs`, the Worker's `platforms.rs`
   and `minting.rs`, and the passkey purpose.
4. **The first minted run:** the owner plans `platform/blue` from the page;
   its outcome, and §9's answer, are recorded as an amendment.

### 11. Verification

- Every module the kind expands declares known capabilities;
  `platform/blue`'s set equals !905's list, with `aigateway.write` only
  when `ai_gateways` is set; the ledger resolves `platform-run` per unit.
- The page lists exactly the set, and a proof made for another set is
  refused.
- A failing mint seals nothing and revokes what it made; every way a run
  ends revokes its token ID once.
- No event, log or run view holds a token value; the job refuses a set
  wider than the minted one.

## Amendments

### 2026-11-17: Review of !948 against !909: they agree

- !909 added this ADR, and changed no code. It amends
  [`2026-11-08-runtime-blue-green-self-standup.md`](2026-11-08-runtime-blue-green-self-standup.md)
  and
  [`2026-11-12-development-grants-applies-rescue-yield.md`](2026-11-12-development-grants-applies-rescue-yield.md)
  through it, over `platform/platforms.rs` and `worker/platforms.rs`.
- !948 is part A3 of the step 12 sweep of
  [`2026-11-16-development-src-crates-neutral-names.md`](2026-11-16-development-src-crates-neutral-names.md):
  `x_pigeon`, the Worker outbound scan's engine directories, and comments
  and messages. It touches neither `platforms.rs`. The two met only through
  !948's scope `src/pigeon-core/src/**` and `src/pigeon-agent/src/**`.
- Nothing any of the four ADRs decides changes.
### 2026-11-17: Review of !939 against !909: they agree

- **!939** carries out step 6b-b of
  [`2026-11-16-development-merge-path-measured-faster.md`](2026-11-16-development-merge-path-measured-faster.md):
  a same-patch restack carries only the checks its job names. In `jobs.rs`
  it adds one field, `carries: None`, to the restack job's report.
- **!909** wrote this ADR. In `jobs.rs` it decides only the platform plan
  job (§7), and it relies on
  [`2026-11-08-runtime-blue-green-self-standup.md`](2026-11-08-runtime-blue-green-self-standup.md)
  and
  [`2026-11-12-development-grants-applies-rescue-yield.md`](2026-11-12-development-grants-applies-rescue-yield.md).
- **They agree.** !939 touches no `platforms.rs`, no grant and no platform
  job. This ADR decides nothing about restacks or the merge gate.
### 2026-11-17: The page's switch moves to step 2; `capabilities` is required where a `pigeon/*` kind expands a module

- **The Worker can't compute a unit's set before step 2.** Unit configs
  live in the instance, and the deploy only reports them from step 2 on.
- **Step 1** adds §1's catalog entries and `capabilities` fields, their
  parse, and §2's `capabilities(kind, config)` in core, with tests. It
  leaves the plan page, `MODULE_PERMISSIONS` and `token_permissions()` as
  they are. A test pins that every module the `pigeon/platform` kind
  expands declares `capabilities`, each a catalog name.
- **Step 2** also adds `Resolved.units`, each `pigeon/platform` unit's
  capability names. The plan page lists `Resolved.units[run.unit]` with
  each capability's permissions. `MODULE_PERMISSIONS` and
  `token_permissions()` go then. With no deployed set for the unit, the
  page falls back to the pasted token alone and names that cause.
- **§1's "required on every `cloudflare/*` module"** becomes: required on
  every module a `pigeon/*` kind expands, pinned by that test. Any module
  may declare it, and `infra validate` refuses a declared value that isn't
  a list of names.
- **Capability names take dashes, not underscores.** The catalog's name
  rule is `^[a-z0-9]+(-[a-z0-9]+)*(\.[a-z0-9]+(-[a-z0-9]+)*)+$`, so §1's
  `secrets_store.read`, `access_apps.write` and `access_tokens.write` are
  `secrets-store.read`, `access-apps.write` and `access-tokens.write`.

### 2026-11-17: Review of !939 against !954: they agree

- **!939** carries out step 6b-b of
  [`2026-11-16-development-merge-path-measured-faster.md`](2026-11-16-development-merge-path-measured-faster.md):
  a same-patch restack carries only the checks its job names.
- **!954** moved the plan page's switch to step 2 here, and made
  `capabilities` required where a `pigeon/*` kind expands a module.
- **They agree.** !939 touches no `platforms.rs`, module or plan page.
  !954 decides nothing about restacks or the merge gate.

### 2026-11-17: Review of !948 against !954: they agree

- !954 decided, over `src/**/platform/platforms.rs` and
  `src/**/worker/platforms.rs`, how a platform run's account-token
  permissions come from the kind's modules. !948, part A3 of step 12 of
  [`2026-11-16-development-src-crates-neutral-names.md`](2026-11-16-development-src-crates-neutral-names.md),
  touches neither file: it renames core's `x_cobot` helper to `x_pigeon`,
  narrows the Worker's outbound scan to the engine modules, and gives
  comments, messages and test names the new names.
- The two met only through !948's `src/pigeon-core/src/**` and
  `src/pigeon-agent/src/**` scope. Nothing either ADR decides changes.
