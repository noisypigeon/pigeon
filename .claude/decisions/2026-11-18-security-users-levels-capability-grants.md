## Users at three levels, one Administrator, and capabilities granted per user

* Date: 2026-11-18
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: security
* Governs: `src/pigeon-core/src/member/**`, `.config/roles.yaml`, `.config/policies/roles.yaml`, `src/**/worker/learner.rs`, `src/**/worker/members.rs`

## Context

The owner asked for authentication built around a base-level **User**, with
three permission levels and a set of capabilities:

- **Administrator:** Willow, and only Willow.
- **Standard:** June, who will use the ICBC capability.
- **Service:** non-human callers: job containers, the pool, Workers, CI and
  other service tokens.
- **Capabilities:** Subdomain, Domain, Blog and ICBC.

The request arrived cut short. Where it leaves a choice open, this ADR
designs around the choice and files it with `owner_ask` (§9).

### What exists, read from `main` (fceaa15)

- **Members.**
  [`2026-11-14-security-instance-members-roles-invitations.md`](2026-11-14-security-instance-members-roles-invitations.md)
  (the members ADR) defines accounts, roles `administrator`, `user` and
  `service`, invitations (`/join`, match code, admit), and §8's check
  order. `core/member` (`src/pigeon-core/src/member/`) holds all of it as
  pure values, and the `Instance` keeps `Members` under `members` with the
  owner's account seeded as the first administrator (!442). Its §8 check
  (`member::check`) is called by nothing on the platform yet.
- **Roles.** `.config/roles.yaml` lists each role's entries:
  `administrator` everything, `user: [learner]`, `service: [read, propose]`.
  It is guarded and in `secrets:`, so changing it takes the owner each time.
- **ICBC is the learner feature.**
  [`2026-11-15-runtime-learner-practice-standalone-app.md`](2026-11-15-runtime-learner-practice-standalone-app.md)
  (the learner ADR) builds practice for the BC Class 7L knowledge test from
  ICBC's material, on an app-only standalone instance whose one User is the
  learner. `worker/learner.rs` admits a caller whose role lists `learner`.
- **Who is "the owner" in code today.** Cloudflare Access admits one
  person, so the platform treats *any* person as the owner. Seventeen
  checks in 16 files refuse a caller that is not `who.human`, among them `access::authorize`,
  `run()`'s write check (`worker/mod.rs`), approvals, mandates, lanes, the
  CI window, minting, platform runs, the pool and devices. A device
  signature is likewise taken as the owner's.
- **The one authorizer.** The feed-view ADR
  ([`2026-11-17-runtime-mcp-feed-view-contract.md`](2026-11-17-runtime-mcp-feed-view-contract.md))
  plans the members ADR's §8 check inside `run()` as step R3, and the
  member, device and passkey tools as R8. The routes→MCP planner owns both.
- **Blue-green.**
  [`2026-11-18-runtime-independent-blue-green-workers.md`](2026-11-18-runtime-independent-blue-green-workers.md)
  gives each color its own `Instance` and journals every object's writes in
  shared D1. The standby replays the journal; only the lease's color acts.
- **Service credentials.** The four Access service tokens, `pigeon-agent`
  and the pool's among them, are kept in `platform/blue`. Grants
  in `.config/grants.yaml` mint provider tokens for jobs. Session tickets
  delegate to one session.

### Three gaps this ADR closes

1. **A second person would pass every owner check.** Every `who.human`
   check and the device-is-owner rule hold only because one person exists.
   The first Standard user to sign in to the platform instance, by Access
   or by a linked device, would hold the owner's power. So **no Standard
   user joins the platform instance until §7's step 5 has landed.**
2. **The roles policy lets `user` hold anything.**
   `.config/policies/roles.yaml` lets `user` list `approve`,
   `approve.owner`, `auto`, `mandates`, `platform` and `members`, and
   `check_roles` agrees. One owner-tier edit could give a User the
   approval power. No rule should allow that.
3. **The check itself is unguarded.** `src/pigeon-core/src/member/**` is
   in neither `guarded` nor `gate` of `.config/approvals.yaml`, so Auto
   covers a change to the code that decides who may do what.

**Out of scope:** a second Administrator; a quorum; email; Android; the
features behind Subdomain, Domain and Blog, beyond the operations they
declare (each lands under its own ADR, after §9's answers).

## Implementation plan.

### 1. The User

A **User** is the accounts ADR's `person` account plus its membership of
an instance. It is one record across the stores that exist, with no new
store:

| Field | Where it lives |
|---|---|
| `id`, `acct_…`, opaque | `accounts` (the accounts ADR) |
| display name | `accounts`; shown everywhere a person is named |
| level | `Membership.role`, per instance |
| capabilities | `Membership.grants` (§4), per instance |
| passkeys | the passkey ring, each naming its account (the passkey-only ADR §5) |
| devices | `devices`, each naming its account |

- **Names, not emails.** Cards, pages, the feed and `change_status` show
  the display name. An email is kept only on an open invitation, and
  never in the repository, a log or an event.
- **A Service identity** is a `service` account (the members ADR §6) at
  level Service. It has no passkey and no device, and it never proves.
- **Willow is the Administrator.** The account `members` was seeded with
  (!442) is Willow's. It already holds the existing passkeys and devices,
  so nothing moves.

### 2. Levels

`Role` gains its new name: `administrator`, `standard` and `service`. The
stored and wire value `user` is read as `standard` (`#[serde(alias)]`), so
no stored row changes.

**Two vocabularies, kept apart in code:**

- **Permissions** are what a level allows: the members ADR's list (`read`,
  `propose`, `approve`, `approve.owner`, `auto`, `mandates`, `platform`,
  `members`). `core/member::CAPABILITIES` is renamed `PERMISSIONS`, and
  `.config/roles.yaml` keeps listing them per level.
- **Capabilities** are the owner's four, granted per user (§4):
  `subdomain`, `domain`, `blog` and `icbc`. They are a constant in
  `core/member`, `CAPABILITIES`. `learner` leaves the permissions and
  becomes the `icbc` capability.

**Ceilings, checked by `check_roles`, by the roles policy, and again on
every request:**

| Level | Permissions it may hold | Capabilities | Proves |
|---|---|---|---|
| Administrator | all, always; listed in full | all, implicitly, unscoped | yes |
| Standard | none | only those granted, within scope and expiry | yes, for its own credentials |
| Service | `read` and `propose` at most, narrowed by the account's scopes and tool list | never | never |

- **Only one Administrator.** `core/member` refuses inviting, admitting or
  changing anyone to `administrator` while an active one exists. The seed
  is the only way to make one. Adding a second Administrator takes a new
  ADR and Willow's passkey on its `owner`-tier change.
- **No approval power below Administrator.** `approve`, `approve.owner`,
  `auto`, `mandates`, `platform` and `members` are Administrator-only by
  the ceilings, so no grant, scope, role edit or stored value can give them
  to a Standard user or a service. The `owner`, `gate` and `guarded` tiers
  and every proof stay Administrator-only. A proof counts only when its
  passkey's account is the active Administrator at redemption (members
  ADR §8.6).
- **Standard's ceiling is empty.** A Standard user may manage their own
  passkeys, devices and membership, and use the capabilities granted to
  them. Nothing else, `read` of the platform included.

**What each tool requires.** `wire::Operation` gains a required field,
`requires: Requires`, so an operation with no requirement doesn't compile:

```rust
pub enum Requires {
    /// A level permission, by name in `member::PERMISSIONS`.
    Permission(&'static str),
    /// A granted capability, and the argument naming its scope, if any.
    Capability { name: &'static str, scope_arg: Option<&'static str> },
    /// The caller's own credentials or membership.
    Own,
}
```

Each operation's requirement, by family:

| Family | Requires |
|---|---|
| Reads: `*.status`, `*.list`, `*.show`, `feed.read`, `events.wait`, `digest.show`, `insights.show`, `architecture.describe`, `tracker.show` | `read` |
| Agent work: `change.propose`, `change.pushed`, `change.note`, `change.scope`, `change.handoff`, `change.yield`, `change.evictable`, `ci.rerun`, `deploy.retry`, `brief.*`, `task.*`, `session.*`, `bench.*`, `owner.ask`, `owner.withdraw`, `mandate.propose`, `mandate.end`, `grant.request`, `lane.request`, `policy.check`, `infra.plan`, `infra.validate`, `platform.plan`, `sites.build`, `provider.tools`, `provider.call`, `git.tokens`, `build.*`, `apple.check`, `approval.wait` | `propose` |
| Approvals: `change.authorize`, `change.approve`, `change.merge`, `change.close`, `change.pin`, `change.front`, break-glass, `owner.answer`, `proof.submit` | `approve`, or `approve.owner` for the `owner` tier |
| Windows: `ci.window`, `lane.end`, Auto | `auto` |
| Mandates: granting and resuming | `mandates` |
| Platform: `platform.apply`, `infra.apply`, `grant.revoke`, `grant.rotate`, `pool.pause`, `pool.resume`, `ios.release`, `sites.publish`, `ci.cloudflare`, `deploy` writes | `platform` |
| Members: `member.invite`, `member.admit`, `member.role`, `member.remove`, `member.grant`, `member.revoke`, `service.*` | `members` |
| Self: `account.show`, `member.join`, `member.leave`, `passkey.*`, `device.*`, `push.subscribe` | `Own` |
| `learner.*` | `Capability { name: "icbc", scope_arg: None }` |
| Subdomain, Domain and Blog tools, as their ADRs add them | `Capability { … }` with the argument naming the subdomain, domain or blog |

- **A reviewer checks a tool's family at its change.** The tests (§8)
  check invariants over the whole catalog, not this table.
- `tools/list` lists only what the caller may call, so each client shows
  only what the caller may use (§6).

### 3. Service identities, least privilege

| Caller | Account | Holds |
|---|---|---|
| Agent sessions (`pigeon-agent`) | `agent` | `read`, `propose`. A session's ticket narrows it to the session's class and its changes |
| The pool's token | `pool` | `read`, and only the pool's tools |
| Job containers (feed-view §6) | `jobs` | `read`; tools `build.list`, `build.read`, and their job's own results |
| CI: Buildkite OIDC, GitHub Actions | `ci` | `propose`; tools `release.apple`, `release.apple.archive`, and check results |

- **A tool list narrows; it never widens.** A service account's
  membership gains an optional `tools` list. A call passes only if its
  permission is within the ceiling and the account's scopes, **and**, when
  the list is set, the tool is on it. The ceiling is checked on every
  request, not trusted from storage.
- **Workers are not principals.** `pigeon-front` and the colors' service
  bindings forward the caller's identity unchanged. A Worker's own cron
  and alarms act as the control plane, not as a member, and they reach no
  tool through `run()`.
- **Minted tokens are not principals either.** A ledger grant's token acts
  in a provider account, as the grants ADR has it, and never calls the
  control plane. A service account never receives a grant routed to a job
  or a Worker (members ADR §6).
- **No service account is created, scoped or given a credential without
  Willow.** `service.create`, `service.scope` and credential grants are
  `owner` tier.

### 4. Capabilities, granted per user

A grant is a value on the membership:

```rust
pub struct Grant {
    pub capability: String,       // in member::CAPABILITIES
    pub scope: Option<String>,    // the subdomain, domain, blog or instance it covers
    pub granted_by: String,       // the Administrator's account
    pub at: u64,
    pub expires: Option<u64>,
    pub revoked: Option<(u64, String)>,
}
```

- **Who grants.** Only the Administrator, with `member.grant {account,
  capability, scope?, expires?}`, at `owner` tier: a passkey proof each
  time, bound to the account, the capability and its scope. No Auto
  window, mandate, lane or policy answers it. `member.admit` may carry
  grants, shown on its card, so admitting and granting take one proof.
- **To whom.** Only an active Standard member. A grant to a service
  account is refused in `core/member`. An Administrator needs none.
- **Scope.** Each capability names how its scope is checked: a DNS label
  for `subdomain`, a domain for `domain`, a site name for `blog`, and none
  for `icbc`, which is the instance's feature. An operation declaring
  `scope_arg` passes only when that argument is within an active grant's
  scope. What each scope means waits on §9's answers 3 to 5.
- **Expiry.** A grant is active until `expires` or a revocation.
  `member.grant` sets 365 days unless it names another time (§9 answer 7
  may change the default). The digest lists each grant 14 days before it
  expires, and an expired grant stays in the record.
- **Revocation.** `member.revoke {account, capability}` by the
  Administrator, signed by her device, with no passkey, so access can be
  withdrawn at once from anywhere. `member.remove` and the members ADR's
  removal revoke every grant. A revoked or expired grant fails the next
  request, since the check reads the current membership on every call.
- **The record.** Events `member.granted`, `member.revoked` and
  `member.grant_expired` name the actor, the member's account, the
  capability and its scope. Each refusal by capability is a
  `member.refused` event naming the capability.

### 5. Enforcement in one place

- **The check is `core/member::check`.** It gains the requirement,
  the grants and `now`, and its order becomes:
  1. who: the principal, named by the door (members ADR §8.1);
  2. member: an active membership;
  3. requirement:
     - `Permission`: within the level's list and ceiling, and for a service
       its scopes and tool list;
     - `Capability`: Administrator, or an active grant whose scope covers
       the argument;
     - `Own`: the subject is the caller;
  4. ticket, 5. tier, 6. proof, 7. separation, 8. record, as the members
     ADR has them.
- **`run()` calls it, as R3.** The routes→MCP planner owns R3 and its
  wiring. This ADR supplies its inputs in `core/member` and
  `wire::Operation.requires` (§7 steps 2 and 3), so that R3 wires one
  call.
- **Every `who.human` becomes the Administrator check.** Each of the 17
  checks asks `principal.is_administrator()`, which is true for exactly
  Willow's account. With one person, `human` and Administrator are the
  same caller, so behavior doesn't change. A device's signature names its
  account, and only the Administrator's devices act as the owner.
- **In shadow first,** as the members ADR's step 5c has it: the check runs
  beside the old one and logs every difference, then decides.
- **Blue-green.** Grants are inside `Members`, in the `Instance`, so every
  grant and revocation is an `Instance` write, journaled and fenced by the
  lease's epoch, which the standby replays. A revocation is an act: once
  it is appended, no color serves the grant. The standalone has one Worker
  and no colors.

### 6. Onboarding June as Standard with ICBC

June is a member of their own learner standalone, as the learner ADR
designed, and **not of the platform instance** unless §9 answer 2 says
otherwise. The platform's `who.human` checks then never see June, so
onboarding doesn't wait on §5's shadow day.

1. **Invite.** Willow invites from the app (`member.invite`, naming the
   standalone, level Standard). The control plane answers a `/join` link,
   which Willow sends.
2. **Enrol.** June opens the link and enrols a passkey on
   `platform.pigeon.dev`, the relying party. The page shows the match
   code.
3. **Admit and grant.** Willow's card shows June's display name, Standard,
   the passkey's name, the match code and `icbc`. One proof admits June
   and grants `icbc`.
4. **The app.** Willow adds June as a TestFlight tester (an owner action
   outside the repository). June's app links a device with a passkey
   assertion. June's `tools/list` holds `learner.*` and their own tools,
   so the app opens on Progress and shows only the five learner screens:
   no Approvals, Feed, Insights or instance switcher beyond their own.
5. **The web.** `/app` on the standalone shows the same roots for June,
   if §9 answer 6 includes the web.
6. **Audit.** Willow sees `member.invited`, `member.enrolled`,
   `member.admitted`, `member.granted` and every refusal in the feed and
   the digest. June sees changes to their own membership.

### 7. Execution

Each step is one change. "(planner)" is a guarded or gate path, or settles
a boundary; the rest are sonnet briefs.

1. **This ADR,** with amendments to the members ADR and the learner ADR
   saying what changed. (planner)
2. **Guards and the policy:** `src/pigeon-core/src/member/**` joins
   `guarded` and `gate` in `.config/approvals.yaml`.
   `.config/policies/roles.yaml` takes `standard` or `user` as the second
   key, lets that list hold only `learner` or nothing, fixes
   `administrator`'s list, and keeps `service` at `read` and `propose`.
   This tightens the rule: the owner approves with a passkey. (planner)
3. **Core:** in `core/member`:
   - `Role::Standard`, aliased from `user`;
   - `PERMISSIONS` and `CAPABILITIES`;
   - the ceilings in `check_roles`, and the one-Administrator rule in
     `invite`, `admit` and `set_role`;
   - `Grant`, with `grant`, `revoke` and expiry;
   - `Requires`, and `check` taking requirement, grants and `now`;
   - `Members::migrate`: once, each active Standard member of an instance
     with the `learner` feature gains an unexpiring `icbc` grant from
     `migration`.

   In the same change, `.config/roles.yaml` becomes `standard: []`, with
   `learner` gone from `administrator`. `worker/learner.rs` asks for
   `icbc` through `check` and runs `migrate` on load, and
   `worker/members.rs` uses `Role::Standard`. Landing all of these at
   once keeps the standalone's compiled roles and its learner from ever
   disagreeing. (brief; the owner approves, as it touches guarded paths)
4. **`wire::Operation.requires`** on every operation, by §2's table, with
   §8's invariant tests. It lands before R3. (brief)
5. **The Administrator check:** `principal.is_administrator()` in place
   of each `who.human`, and a device acting as the owner only when it is
   the Administrator's. This is gate code, so it is the planner's, and it
   is done with the routes→MCP planner if R3 takes it. (planner)
6. **The grant tools:** `member.grant`, `member.revoke` and the
   grants in `member.list`, with the proof's binding and the admit card's
   grants. (planner for the proof binding; brief for the rest, after R8a)
7. **The clients:** Members shows each member's grants, with grant and
   revoke for the Administrator; a Standard user's roots come from their
   `tools/list`. (brief)
8. **June:** the owner's actions of §6, steps 1 to 4. (owner)
9. **Service accounts:** `agent`, `pool`, `jobs` and `ci` seeded with
   §3's scopes and tool lists. Each Access token is matched to its
   account by client ID, set once by Willow, since the IDs stay out of
   the repository. (planner)
10. **The roles policy drops `user` and `learner`,** once step 3 is
    deployed everywhere. (planner)
11. **This ADR amended with the outcome,** and Subdomain, Domain and Blog
    handed to their own ADRs. (planner)

Steps 2 to 4 need nothing else. Steps 5 and 6 follow R3 and R8a. Step 8
needs step 3 and the learner ADR's standalone, but not step 5.

### 8. Tests

Each change that adds a test carries a `Test-Reason:` trailer. No test
lists operations, tools or capabilities one by one. The tests assert
invariants:

- **Catalog:** every operation whose tier can be `owner`, or that redeems
  a proof, requires an Administrator-only permission. No `Capability`
  operation is one of those. Every `scope_arg` names one of the
  operation's arguments.
- **Ceilings:** a Standard member holding every capability, with every
  scope, is refused every `Permission` operation. A service account with
  every scope is refused everything outside `read` and `propose`, and
  everything off its tool list. Neither ever passes a proof.
- **One Administrator:** inviting, admitting or promoting to
  `administrator` is refused while one is active, including when two
  requests race.
- **Grants:** a grant passes within scope and before expiry. It fails
  outside its scope, after expiry and after revocation, on the next call.
  Only the Administrator's proof grants, and a grant to a service account
  is refused.
- **Migration:** a stored `user` row reads as `standard`. `migrate` gives
  a learner instance's Standard members `icbc` once, and running it twice
  changes nothing. Willow's seeded row and credentials are unchanged.
- **Shadow (step 5):** the old and new checks agree on every real request
  for a day, or each difference is explained in this ADR's amendment.

### 9. Open questions, filed with `owner_ask` on !1155

1. Is ICBC the learner practice app, and should its capability be named
   `icbc`? Assumed yes.
2. Is June a member of their own standalone only, or of the platform
   instance too? Assumed the standalone only (§6).
3. What does Subdomain let a Standard user do, and what is its scope?
4. What does Domain let a Standard user do, and what is its scope?
5. What does Blog let a Standard user do, and what is its scope?
6. Which clients does a Standard user get: the iOS app, the web `/app`,
   or both? Assumed the iOS app.
7. How long does a grant last by default? Assumed 365 days, renewable.

Answers 3 to 5 decide those capabilities' tools and scopes, and each
lands as an amendment here. Until then the three are in the vocabulary
and no operation requires them.

### 10. Rollback

- Each step reverts alone. A reverted core leaves `grants` and the
  `standard` alias unread, since both are `serde(default)` or aliases.
- The Administrator check and the old `who.human` are the same caller
  while only Willow is a person on the platform, so reverting step 5
  changes no one's access.
- A grant that is wrong is revoked at once with `member.revoke` (§4).
  Suspending a membership ends everything that member holds.
- If no Administrator passkey can prove, the passkey-only ADR's
  break-glass enrolment is the way back, unchanged.
