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

**What each tool requires.** Calling a tool and answering it are two
checks, and `main` already has both (b63):

- **`Operation::capability()`** is the permission to *call*: `read` for a
  read, `platform` for an apply outside the control plane, and `propose`
  otherwise. So an agent may call `change.authorize`, and the call
  answers with an approval link.
- **`Operation::asks()`** says who *answers*: `Grant` and `Page`
  operations wait for a proof, and only the Administrator proves. The
  answering permission is `approve`, or `approve.owner` at the `owner`
  tier.

This ADR turns `capability()` into `requires() -> Requires`. It keeps
b63's rule and adds three things:

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

| Operations | Requires |
|---|---|
| b63's rule: reads, agent work, platform applies | `read`, `propose` or `platform`, unchanged |
| Each operation a `who.human` check guards today, such as `owner.answer`, `lane.end`, `pool.resume`, minting and the device and passkey writes | an Administrator-only permission for its family: `approve` for answers, `auto` for windows and lanes, `mandates`, `platform` for the platform, the pool and minting, `members` for others' devices |
| `member.*` and `service.*` writes on another member | `members` |
| One's own credentials and membership: `account.show`, `member.join`, `member.leave`, own `passkey.*` and `device.*`, `push.subscribe` | `Own` |
| `learner.*` | `Capability { name: "icbc", scope_arg: None }` |
| Subdomain, Domain and Blog tools, as their ADRs add them | `Capability { … }`, with the argument naming the subdomain, domain or blog |

- **Step 5 moves each `who.human` check into `requires()`,** so the
  refusal lives in one place and the catalog says it.
- **A reviewer checks each operation's row at its change.** The tests
  (§8) check invariants over the whole catalog, not this table.
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
  `wire::Operation::requires()` (§7 steps 3 and 4), so that R3 wires one
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
4. **`wire::Operation::requires()`** in place of `capability()`, b63's
   rule kept and §2's rows added, with §8's invariant tests in
   `wire/registry_tests.rs`. It lands before R3. (brief)
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

- **Catalog:** every operation that redeems a proof, or answers for the
  Administrator (an `owner.answer`, a window, a lane's end), requires an
  Administrator-only permission; a `Grant` or `Page` operation may be
  called with `propose`, and only answered with `approve` or
  `approve.owner`. No `Capability` operation is one of those. Every
  `scope_arg` names one of the operation's arguments.
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

## Amendments

### 2026-11-18: Agents are Service users, the platform's features are split into base and capabilities, and checks read after replay

The owner's request arrived in full after this ADR was proposed. It adds two
things. The blue-green planner adds a third.

#### 1. Agents are Service users

Every autonomous agent is a Service user: a planner or executor session, a
pool runner, a job container. §3's table gains the session classes, and a
ticket is how a session acts as one:

| Service user | Who acts as it | Tools, within `read` and `propose` |
|---|---|---|
| `agent-planner` | an `opus` session, by its ticket | the agent's tools, briefs (`brief.create`, `brief.amend`, `brief.review`), `owner.ask` and `owner.withdraw`, ADR changes |
| `agent-executor` | a `haiku` or `sonnet` session, by its ticket | the agent's tools less briefs' writes and `owner.*`; only the change and task its brief names |
| `pool` | a pool runner | the pool's tools, and the change its task names |
| `jobs` | a job container | `build.list`, `build.read`, its own job's results |
| `ci` | Buildkite and GitHub Actions | `release.apple`, `release.apple.archive`, check results |

- **One credential, two users, for now.** `pigeon-agent`'s token proves the
  caller is an agent. `session_start`'s `model` picks which Service user
  the ticket acts as. A ticket for one class never acts as the other.
  A credential per agent session stays the members ADR §6's later work.
- **The executor rules become the check.** CLAUDE.md's "an executor never
  writes or amends an ADR, and never calls an owner operation" is
  enforced by the `agent-executor` tool list and by `run()`, not only by
  the brief.
- **Never above Service.** No Service user, ticket or tool list holds an
  approval permission, proves, or receives a capability grant (§2, §4).
- §7 step 9 seeds these five in place of `agent`, `pool`, `jobs` and `ci`.

#### 2. The base level and the features behind capabilities

The owner asked to restructure the platform's non-base features as
capabilities. Subdomain, Domain, ICBC and Blog are the ones that came to
mind, so the list below is longer.

- **Base** is what the platform needs to build, run and govern itself, and
  what every user needs for their own identity. A level decides it, and
  no grant is needed.
- **A capability** is a feature that delivers something to a user beyond
  that. The Administrator holds every capability; anyone else holds only
  what is granted.

| Area | Operations | Class | Who |
|---|---|---|---|
| Identity | `account.show`, `member.join`, `member.leave`, own `passkey.*`, `device.*`, `push.subscribe`, own feed items | base | every member (`Own`) |
| Changes and agents | `change.*`, `brief.*`, `task.*`, `session.*`, `bench.*`, `ci.*`, `policy.check`, `git.tokens`, `build.*`, the merge queue | base | Administrator, Service |
| Governance | approvals, `owner.*`, mandates, windows, lanes, `grant.*`, minting | base | Administrator (answers); Service (asks) |
| Running the platform | `platform.*`, `infra.*`, `deploy.*`, `instance.show`, the architecture, insights, tracker, ops, digest, the docs site | base | Administrator; Service reads |
| The platform's own app release | `ios.release`, `ios.status`, `apple.check`, `release.apple*` | base | Administrator; `ci` |
| Members | `member.*`, `service.*` | base | Administrator |
| ICBC | `learner.*` | capability `icbc` | granted |
| Subdomain | a hostname `<label>.pigeon.dev` (the standalone-hostnames ADR) | capability `subdomain` | granted, §9 answer 3 |
| Domain | a zone or a custom domain (the zone-vendor and zoned-standalone ADRs) | capability `domain` | granted, §9 answer 4 |
| Blog | posts on a blog site (`workloads/noisypigeon.com/blog`) | capability `blog` | granted, §9 answer 5 |
| Sites | `sites.build`, `sites.publish` for a site other than the platform's own | proposed capability `sites` | unclear (below) |
| Standalone instances | `instance.standup`, the private listing | proposed capability `instances` | unclear |
| Provider connections | `provider.call`, `provider.tools`, connecting a provider account | proposed capability `providers` | unclear |
| The capacity pool | `pool.*`, running a task on the pool, the AI Gateway | proposed capability `pool` | unclear |

- **Unclear rows are one owner question.** Sites, standalone instances,
  provider connections and the pool each serve the platform's own work
  today *and* could serve a user. The owner decides, row by row, whether
  each is base or a capability. Until then each stays base, as today, and
  nothing moves.
- **Moving a feature behind a capability** takes one change per feature:
  - its name joins `CAPABILITIES`;
  - its operations declare `Requires::Capability` with their scope
    argument;
  - its scope's check joins `core/member`;
  - it joins `GRANTABLE` once a grant of it means something.

  The platform's own use is unaffected, since the Administrator holds
  every capability, and a Service user calls only base operations.
- **A capability operation stays off every Service user's tool list.** An
  agent building the blog's code works through changes (base); it never
  publishes a post as a user.
- §7 gains step 12: one change per feature the owner moves, after step 4.

#### 3. A check reads after replay

From the blue-green planner, for
[`2026-11-18-runtime-independent-blue-green-workers.md`](2026-11-18-runtime-independent-blue-green-workers.md)
§A.3: **a capability check reads `Members` after replay, never from a
value cached across calls.** A grant or a revocation is an `Instance`
write, so it is an act: it is fenced by the lease's epoch and journaled.
The standby answers only after replaying to the journal's head. So a
revocation holds on both colors from the request after its append,
including through `pigeon-front`'s 5-second lease cache. Step 10 of that
ADR checks replay parity for both colors.
### 2026-11-18: !1148 (`approval.list`) agrees with this ADR

Review !1166 asked whether !1148 agrees with this ADR. !1148 carries out
R6 of
[`2026-11-17-runtime-mcp-feed-view-contract.md`](2026-11-17-runtime-mcp-feed-view-contract.md).
The two agree, and neither changes the other:

- !1148 adds `approval.list`, a read of the owner's waiting approvals,
  grants and offers and the open fast lanes, each row opening its proof.
  It changes no member, grant, level or learner code; its files are
  `worker/app.rs`, `worker/mod.rs`, the shell, and pigeon-core's `wire.rs`,
  `control.rs` and presenters.
- Its rows carry each grant's page path, the token the owner's link
  carries, so under §5 its requirement is Administrator-only. Step 4's
  `requires()` gives it that, and R3 enforces it when it wires
  `member::check`. Until then it is listed on `/mcp` for whoever Access
  lets through, as `approval.wait`'s links are today.

### 2026-11-18: Reviewed !1162 against !1213: they agree

[`2026-11-18-runtime-independent-blue-green-workers.md`](2026-11-18-runtime-independent-blue-green-workers.md)'s
amendment for step 9a (!1213) shapes `pigeon-front`: its own crate, a
service binding per color, its own `/healthz` check, and only hosts that
Access covers. !1162 is §7 step 2: `src/pigeon-core/src/member/**` joins
`guarded` and `gate`, and the roles policy holds `standard` to no
permission.

- **No file in common.** !1162 changes `.config/approvals.yaml`,
  `.config/policies/roles.yaml` and the judge's roles test. !1213 governs
  the Worker specs, the deploy and the front's crate.
- **The front makes no authorization decision.** It forwards each request
  unchanged to the lease's color, which runs §5's check. This is what the
  first amendment's §3 already assumes. The front reads no `Members`, so
  "a check reads after replay" holds through it.
- **Access stays the outer gate for pages.** !1213 has the one Access
  application cover exactly `platform/blue`'s `domains`. The members ADR
  §8 and §5 here keep Access as the outer gate, and admit no Standard
  user to the platform instance until step 5 lands. Neither changes the
  other.

### 2026-11-18: The owner's answers to §9 and to the feature inventory

The owner answered every question this ADR filed, on !1155.

| # | Question | Answer | What changes |
|---|---|---|---|
| 37 | Is ICBC the learner app? | Yes; the capability is `icbc` | nothing |
| 38 | Where is June a member? | June's own learner standalone only | nothing: §6 stands |
| 39 | Subdomain | A user publishes their own static site at `<label>.pigeon.dev` | `subdomain`'s scope is one DNS label |
| 40 | Domain | A user attaches a domain they own to a site or instance they hold | `domain`'s scope is one domain |
| 41 | Blog | A user's own blog, from the blog static-site template, deployed to a subdomain or to a domain's apex | `blog`'s scope is one blog site |
| 43 | Standard's clients | The iOS app only, through TestFlight | nothing: §6 stands, and `/app` answers a Standard user 403 |
| 44 | A grant's default lifetime | No expiry; the Administrator revokes by hand | §4 changes, below |
| 45 | Sites, standalone instances, providers, the pool | Sites and standalone instances become capabilities; providers and the pool stay base | the first amendment's §2 table changes, below |

**§4, expiry.** `member.grant` sets no expiry unless it names one. The
digest lists every active grant with its age, and names any grant
expiring within 14 days. `GRANT_TTL` is not added.

**The vocabulary** becomes six capabilities: `subdomain`, `domain`,
`blog`, `icbc`, `sites` and `instances`.

- **`sites`:** a user holds a static site of their own: they create,
  build and publish its content. It is served where a `subdomain` or
  `domain` grant puts it.
- **`instances`:** a user stands up and holds a standalone instance
  (`instance.standup`, the private listing).
- **How they combine:**
  - `subdomain` alone covers one static site at its label, as answer 39
    says.
  - `domain` attaches to a site or instance the user holds.
  - `blog` is a site from the blog template, so it is served under a
    `subdomain` or `domain` grant the user also holds.
  - Each feature's ADR (§7 step 12) writes its tools' `scope_arg` and
    checks the pair.
- **Base, unchanged:** provider connections and the pool, for the
  Administrator and agents only. A capability never covers a spend on the
  pool.
- **`GRANTABLE` stays `icbc`** until each feature's ADR adds its tools.
  Until then a grant of the other five would mean nothing.

**The step-3 brief** follows: `CAPABILITIES` holds the six, and a grant's
`expires` is `None` unless named.

### 2026-11-18: Reviewed !1226 against !1219: they agree

[`2026-11-18-runtime-independent-blue-green-workers.md`](2026-11-18-runtime-independent-blue-green-workers.md)'s
amendment for owner item #42 (!1219) journals secret-bearing `Instance`
state sealed, hashes a grant request's token, and never journals the APNs
token. !1226 is §7 step 3: levels, capability grants and the learner's
`icbc` in `core/member`.

- **What they share.** The only overlap is `wire/registry_tests.rs`. There
  !1226 renames one import, `CAPABILITIES` to `PERMISSIONS`, and !1219
  changes no test.
- **Grants hold no secret.** A `Grant` is a capability's name, its scope,
  who granted it and when. It lives in `members`, which !1219 journals as
  is, as the blue-green planner confirmed. `members.joins`, which holds
  enrolment challenges, is among the sealed keys, and !1226 doesn't touch
  it.
- **A check still reads after replay.** `learner.rs` reads `members` from
  storage on each call and writes it back only when `migrate` adds a
  grant. Both are ordinary `Instance` writes, journaled whether or not
  `journal-seal` exists.

### 2026-11-18: Step 9's shape: tool lists, hashed credentials bound by the Administrator, and R3 carries step 5

Steps 3 (!1233) and 4 (b174) put `Requires` and `requires()` in place.
The routes→MCP planner wires `member::check` into `run()` as the feed-view
ADR's R3. It also carries this ADR's step 5, turning each `who.human` check
into an Administrator check, since both are the same gate code. R3 runs in
shadow until this step seeds the Service users, or every agent call would
be `NotMember`.

#### 1. What narrows a Service user

`Membership` gains two lists, both empty by default. Each entry is an
operation name, or `<area>.*` for every operation in an area.

- **`tools`:** when not empty, a call passes only if it matches an entry.
- **`denied`:** a call matching an entry is refused, whatever `tools`
  says.

Both only narrow, below the scopes and the ceiling. `Request` gains
`operation`, the name being called, so `check` can apply them. They are
checked after the permission and before the ticket.

#### 2. The five Service users, seeded

`Members::seed_services(now)` adds each one that is missing, with
`admitted_by: seed`:

| Service user | Scopes | `tools` | `denied` |
|---|---|---|---|
| `agent-planner` | `read`, `propose` | (all within scopes) | none |
| `agent-executor` | `read`, `propose` | (all within scopes) | `brief.create`, `brief.amend`, `brief.review`, `owner.*`, `mandate.*`, `lane.*`, `grant.*` |
| `pool` | `read`, `propose` | (all within scopes) | as `agent-executor`, since a pool runner does an executor's work |
| `jobs` | `read` | `build.*` | none |
| `ci` | `read`, `propose` | `build.*`, `release.*`, `ci.*` | none |

- A **ticket picks the agent's user.** An `opus` ticket acts as
  `agent-planner`, a `haiku` or `sonnet` ticket as `agent-executor`. A
  call with no ticket acts as `agent-executor`, the narrower of the two.
- **The lists are tuned in shadow.** R3 logs each call the new check would
  refuse, as `member.shadow`, naming the operation and the Service user,
  never a client ID. Before R3 decides, each refusal is either expected
  or fixed in these lists, in an amendment here.

#### 3. Credentials, bound by the Administrator

- `Members` gains `credentials`, each with:
  - the Service user (or `agent`, which stands for both agent users);
  - the kind, `access`;
  - the client ID's SHA-256, never the ID itself;
  - who bound it, when, and whether it is revoked.

  Neither the repository nor the journal holds a client ID.
- **`service.bind {account, client_id}`** binds an Access service token to
  a Service user. It is a page operation at `owner` tier: the
  Administrator proves with a passkey, and no agent or window binds a
  credential. `service.unbind` revokes one, signed by the Administrator's
  device.
- `Members::service_for(client_id)` hashes the presented ID and answers
  the Service user bound to it. An unbound token is no member, and is
  refused once R3 decides.

#### 4. Execution, replacing step 9

- **9a (planner, gate):** in `core/member`: `tools`, `denied`,
  `Request.operation`, `seed_services`, `credentials`, `service_for`, and
  the ticket's class to user, with tests.
- **9b (planner, gate):** the Worker seeds the Service users on load;
  `service.bind` and `service.unbind` as page operations.
- **Owner action:** Willow binds each of the four Access service tokens
  with a passkey, as `owner_ask` items once 9b is deployed.
- **R3 decides** only after 9b, the bindings, and a shadow day with every
  refusal explained.
