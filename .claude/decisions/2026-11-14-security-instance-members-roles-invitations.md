## Members of an instance: invited with a passkey, as Administrator, User or Service Account

* Date: 2026-11-14
* Model: Claude Fable 5.2
* Author: Willow Graysen (@noisypigeon).
* Category: security
* Governs: `src/pigeon-cli/crates/backplane/src/access.rs`

## Context

An instance has one person today, "the owner", and the word is everywhere.

- **Who the owner is, in code:**
  - any person Cloudflare Access lets in. `access::authorize` asks only
    whether an identity is `human`, and Access's policy admits one email;
  - the `APPROVER` secret, an email written on approvals made in the card and
    in the app (`worker/elicit.rs`, `worker/app.rs`);
  - whoever holds a passkey in the `Instance`'s one keyring, or a linked
    device. Neither names a person
    ([`2026-11-09-security-approvals-once-passkeys.md`](2026-11-09-security-approvals-once-passkeys.md) §4,
    [`2026-11-12-runtime-ios-app-approvals-insights.md`](2026-11-12-runtime-ios-app-approvals-insights.md) §2).
- **What the owner may do:** every tier, `owner` included; Auto windows;
  granting and resuming mandates; the platform operations; grants
  ([`2026-11-08-security-tiered-approvals-task-mandates.md`](2026-11-08-security-tiered-approvals-task-mandates.md),
  the passkeys ADR §2, and
  [`2026-11-12-development-grants-applies-rescue-yield.md`](2026-11-12-development-grants-applies-rescue-yield.md) §1).
- **Who else there is:** Access service tokens. `pigeon-agent` proposes,
  pushes and reruns; a session it starts holds a ticket (`session_start`).
  A service token is told apart only by `human: false`.
- **Instances are about to be several**
  ([`2026-11-13-runtime-non-platform-profiles.md`](2026-11-13-runtime-non-platform-profiles.md)),
  each keyed in the control plane or on its own stack.

The owner asked to invite other users to an instance, as one of three types:

1. **Administrator:** exactly what the owner may do today.
2. **User:** a basic type whose restrictions come later.
3. **Service Account:** for non-humans, such as Claude agent sessions. Its
   implementation comes later; this ADR designs its identity, credentials and
   scopes.

**Built on two ADRs written beside this one:**

- **The accounts ADR,**
  `2026-11-14-runtime-accounts-profiles-instances.md` (!419), defines the
  account, platform-wide and apart from any instance, of kind `person` or
  `service`, and renames the app's profile to Instance. This ADR's users are
  its accounts, and it defines what that ADR leaves open: membership, and a
  `service` account's credentials. Where its merged text differs from §1, an
  amendment here follows it.
- **The passkey-only ADR** (session `session_015XFA1nmhU1ESwzojBkvi7k`) makes
  a passkey the only proof. Nothing here takes an authenticator code.

**Out of scope:** the User role's capabilities; issuing a Service Account's
credential and wiring agents to one; a quorum of approvers; sending email;
distributing the app to invitees.

## Implementation plan.

### 1. Terms

- **Account:** as the accounts ADR defines it: the identity that signs in,
  platform-wide, with an opaque ID (`acct_…`) and a kind.
  - A `person` account has passkeys and linked devices, and nobody else's.
    Access's email is never its identity.
  - A `service` account has neither. §6 defines its credentials.
- **Member:** an account with one role in one instance. A `person` account
  is an `administrator` or a `user`; a `service` account is always
  `service`.
- **Role:** `administrator`, `user` or `service`.
- **Capability:** a named set of operations (§2). A role is a list of
  capabilities.
- **Principal:** who a request is from, once checked: a member's account, or
  a service account.
- **Proof:** a passkey assertion from a passkey of the principal's own
  account. A service account has no passkey, so it never proves.

### 2. Roles are explicit capability lists

**The vocabulary,** fixed in the functional core (`core/member/`). Every
operation names the one capability it needs:

| Capability | Operations |
|---|---|
| `read` | every read: statuses, lists, home, insights, the digest, events |
| `propose` | what `pigeon-agent` does today: propose, push, rerun, retry a deploy, sessions, briefs and tasks, proposing or ending a mandate |
| `approve` | redeeming a `confirm` or `code` grant with a proof |
| `approve.owner` | redeeming an `owner` grant with a proof |
| `auto` | starting or ending an Auto window |
| `mandates` | granting or resuming a mandate |
| `platform` | `platform.*`, `infra.apply`, `grant.*` writes, the CI window, `profile.*` writes, and connecting or revoking a provider account (the provider-connect ADR's `may_connect`, !416) |
| `members` | everything in §4 to §6 |

- **Managing one's own passkeys and devices needs no capability.** It needs
  membership, and a proof.
- **`.config/roles.yaml`, on `main`,** lists each role's capabilities by
  name. The Worker reads the copy `main` deployed, as it reads the grants
  ledger.

  ```yaml
  # Each role's capabilities; see .claude/decisions/2026-11-14-security-instance-members-roles-invitations.md §2.
  roles:
    administrator: [read, propose, approve, approve.owner, auto, mandates, platform, members]
    user: []
    service: [read, propose]
  ```

- **One core check,** called by a sentinel policy
  (`.config/policies/roles.yaml`) and by the Worker at start:
  - the three roles are present, and no other;
  - every name is in the vocabulary. There is no wildcard and no "all but";
  - `administrator` lists the whole vocabulary, so a new capability is
    added to it in the change that adds it;
  - `service` holds nothing outside `read` and `propose`. This is the
    ceiling of §6.
- **`user: []` is the slot.** A User signs in, enrols passkeys, links a
  device and sees their own membership. Every other operation is refused,
  `read` included. Giving Users a capability later is one line here; no
  stored membership changes.
- **The file is guarded and in `secrets:`** in `.config/approvals.yaml`, so
  changing it is `owner` tier each time, and no mandate or Auto window
  covers it.

### 3. Owner becomes the first administrator

- **"Owner" is no longer a role.** Every administrator holds exactly the
  owner's permissions. No administrator is above another.
- **The first administrator is seeded,** once, when an instance has no
  membership rows: the owner's account, which the accounts ADR's migration
  creates. The existing passkeys, devices and push subscriptions become
  that account's.
  - `APPROVER` then leaves the request path, and is removed with it.
  - When no administrator can prove, the passkey-only ADR's break-glass
    enrolment (!417, `PASSKEY_ENROLL`) is the only way back. It takes the
    Cloudflare account, and enrols a passkey for an existing administrator;
    it never makes a member.
- **An instance always has an administrator.** Removing or demoting the last
  active one is refused, in the same `Instance` transaction that would do it.
- **One administrator's proof approves.** There is no quorum.
  - Each waiting approval is shown to every administrator. The first proof
    redeems its grant; the others then see who approved, and when.
  - "Never by its proposer" stays, compared by account.
  - No one's proof changes their own role. Leaving (§5) is the exception.
- **What an administrator can't be given:** the Cloudflare account, the
  `infra/` break-glass keys, or a Worker secret's value. Those stay with the
  account's holder, outside the app, and are not a role.
- **Names that stay.** The tier `owner`, and `owner` in a stage's `needs`,
  remain wire and state words, now read as "an administrator, proven, each
  time". Stored state needs no migration. What a person reads says
  "administrator".
- **The residual risk.** An administrator can remove every other one. The
  limits are the `owner` tier on each such step, the push to every
  administrator (§7), the event log, and the break-glass enrolment.

### 4. Invitations

Adding a person takes three steps, and one proof, at the step that binds a
passkey to a role.

1. **Invite** (`member.invite`): an administrator, in the app's **Members**
   screen or on `/members`, gives an email and a role.
   - The control plane answers a link,
     `https://<host>/join/<token>`: 32 random bytes, stored only as its
     SHA-256, valid 72 hours, used once.
   - The administrator sends the link themselves, from the share sheet. The
     control plane sends no email.
   - An agent may propose an invitation; it is then a waiting item for an
     administrator, who gets the link. No agent ever sees a link.
   - An invitation grants nothing, so it takes no proof. At most 10 are
     open per instance.
2. **Enrol:** the invitee opens the link. `/join/*` is a public path, as
   `/v0/app/*` is, and shows the instance's name and accent, the role, and
   nothing else.
   - **The invitee enrols a passkey,** with the passkeys ADR §4's
     parameters. Enrolment is the only way in; there is no code and no
     password. An invitee who already has an account proves with a passkey
     of that account, which is the same step.
   - How many passkeys a new account must hold, and its recovery, are the
     passkey-only ADR's (!417) to decide; enrolment here meets them before
     the invitation counts as `enrolled`.
   - The page, or the app when the link opens it, then shows a **match
     code**: six digits derived from the token and the passkey's public key.
   - The invitation is now `enrolled`. The invitee is not yet a member.
3. **Admit** (`member.admit`): every administrator is pushed to. The card
   shows the email, the role, the passkey's name and the match code.
   - The administrator checks the code with the invitee, by any channel, and
     proves. Admitting is `owner` tier.
   - The membership becomes `active`, the account's email joins the
     instance's Access group (§8), and the invitee's device may link.

- **A link that leaks is harmless alone.** Whoever enrols with it still
  waits for an administrator, who sees a match code the invitee doesn't
  confirm.
- **Declining, revoking or expiry** deletes the enrolled passkey when it
  made a new account, and the invitation.
- **A Service Account is not invited.** An administrator creates it (§6).

### 5. Storage, role change and removal

- **Per instance, in D1,** by the stack that runs the instance: pigeon's for
  the platform and managed instances, its own for a standalone. Never in the
  repository, which is public: a membership names a person.

  | Table | Holds |
  |---|---|
  | `memberships` | instance, account or service account, role, status (`active`, `ended`), who admitted it, when, when ended |
  | `invitations` | token hash, instance, email, role, inviter, expiry, status (`open`, `enrolled`, `admitted`, `declined`, `revoked`, `expired`), the enrolled passkey's ID |
  | `service_credentials` | §6: service account, kind, client ID or public key, the grant that minted it, status |

  Accounts, passkeys and devices are stored as the accounts ADR
  decides; this ADR needs each passkey and device to name its account.
- **Every transition is a pure function in `core/member/`,** run inside the
  `Instance`, so two administrators acting at once see one order.
- **`member.role`** changes a role. `owner` tier. To `administrator` it
  shows the same card as admitting.
- **`member.remove`** ends a membership. `owner` tier. At once, for that
  instance:
  - the account's devices are unlinked, and its push tokens dropped;
  - its email leaves the Access group, and its Access sessions are revoked;
  - its passkeys no longer prove there;
  - an Auto window it started ends; a mandate it granted pauses, for
    another administrator to resume;
  - a change grant it gave covers nothing further, so the next stage asks;
  - its waiting invitations are revoked.

  What it did stays in the record. A demotion from `administrator` does the
  last four.
- **`member.leave`:** a member ends their own membership with a proof,
  unless they are the last administrator.
- **A standalone instance** is handed its administrators' public keys at
  stand-up, as the profiles ADR §7 hands over the owner's.

### 6. Service accounts

**What one is.** An account of kind `service`: a name (a slug), a
description, and who created it. It is a member of an instance with the role
`service`, and its scopes are kept on that membership.

- **Scopes** are capabilities, a subset of the `service` role's list. Since
  that list can hold only `read` and `propose`, and a service account can't
  prove, it can never approve, start Auto, grant a mandate, run a platform
  operation or manage members, whatever is stored.
- **Created, suspended and removed by an administrator,** at `owner` tier
  (`service.create`, `service.suspend`, `service.remove`). An agent can't
  create one.

**How it relates to what exists:**

| Today | Becomes |
|---|---|
| The Access service token `pigeon-agent` | the credential of a seeded service account, `agent`, with scopes `read` and `propose`: what it may do today, now written down |
| A session ticket (`session_start`) | a 24-hour delegation of the service account that started it, to one session. It records that account. Its reach is the account's scopes, narrowed by its class (an executor's limits) and by the changes it holds |
| The grants ledger (`.config/grants.yaml`) | where a service account's credential is declared, minted, rotated and revoked |

- **Credentials.** A service account holds up to two at once, so one can
  rotate. Each is one of:
  - **`access`:** a Cloudflare Access service token, matched by its client
    ID. It passes Access and names the account. This is the only kind built
    first, and the only one a managed instance needs;
  - **`key`:** a P-256 public key, with requests signed as a device signs
    them (iOS ADR §2), for a stack with no Access in front. Reserved.
- **A credential is a grant.** The ledger gains
  `route: { service_account: <name> }` on an entry that mints an Access
  service token. `grant.request` mints it; `grant.rotate` and
  `grant.revoke` work as for any grant.
  - Its value is shown once, on the administrator's approval page, to paste
    into the agent's environment. No operation returns it, and no agent
    reads it.
  - This lands as an amendment of the grants ADR, with the implementation.
- **Three things with similar names stay apart:**
  - a **role's capabilities** say what a principal may ask the control
    plane to do;
  - a **grant's capabilities** say what a minted token may do in a provider
    account;
  - a **ticket** says which session is asking.

  A service account never receives a ledger grant routed to a job or a
  Worker.
- **Suspending one** refuses its credentials and every ticket it started,
  at once.
- **Later, in its own ADR:** creating service accounts from the app, a
  credential per agent session, narrower scopes such as "propose in these
  paths only", and the `key` kind.

### 7. What each role sees, and audit

| | Administrator | User | Service Account |
|---|---|---|---|
| The app: instance picker | the instance | the instance | no app |
| The app: Home and insights | all of it | "Your role has no access yet" | |
| The app: approvals, Auto | all of it, as the owner today | nothing | |
| The app: Members | members, invitations, service accounts; invite, admit, change, remove | their own row; leave | |
| The app: Settings | their own passkeys and devices | the same | |
| `/approvals` and `/approve/<grant>` | every waiting approval, and who answered each | 403, with their role | 403, as today |
| `/members` | as the app | their own row | 403 |
| MCP | every tool, as a person | none | tools within its scopes |
| Push | approvals, pauses, failed deploys, the digest, and every member event | a change to their own membership | none |

- **Every approval names an account.** Grants, change grants, windows and
  mandates record the administrator's account ID where they recorded the
  owner. Cards, pages and `change_status` show the display name.
- **A merge's `Change-Approved-By` trailer** still names the approval's kind
  and never an identity, as
  [`2026-11-05-development-changes-through-pigeon-mcp.md`](2026-11-05-development-changes-through-pigeon-mcp.md)
  decides. The repository is public; the account is in the event log.
- **Events:** `member.invited`, `member.enrolled`, `member.admitted`,
  `member.declined`, `member.role_changed`, `member.removed`, `member.left`,
  `member.refused`, `service.created`, `service.suspended` and
  `service.removed`. Each names the actor, the subject and the role, and
  never a token.
- **The digest lists every member event,** and every credential a service
  account holds, with its age.

### 8. What the control plane checks on every operation

In this order, in one function of the core, for every route, page, MCP tool
and app call. A refusal says which step failed, and is an event.

1. **Who.** The request is authenticated as it is today, and then named:
   - a device signature is the device's account;
   - a page's sign-in cookie is the account whose passkey set it: a proof
     with purpose `sign-in`, kept 12 hours, made as the docs gate's view
     cookie is
     ([`2026-11-11-runtime-docs-under-platform-passkey.md`](2026-11-11-runtime-docs-under-platform-passkey.md) §2);
   - an Access token with a client ID is the service account holding that
     credential.

   Anything else is refused. An Access email lets a person reach a page and
   names no one, and `human` alone decides nothing any more.
2. **Member.** The principal has an `active` membership in the instance the
   request names. A service account that is suspended has none.
3. **Capability.** The operation's capability is in the role's list, and for
   a service account also in its scopes.
4. **Ticket.** For a change operation by a session, as today: a live ticket,
   of that service account, holding the change.
5. **Tier.** The operation's tier is decided as today. A mandate, window,
   change grant or policy may answer it; otherwise it waits for a proof.
6. **Proof.** A passkey assertion, bound to the operation and its target,
   from a passkey of an account that is an active member holding the
   capability *when the proof is redeemed*, not when the grant was minted.
7. **Separation.** Not the proposer's account; not one's own role.
8. **Record.** The operation runs as that principal, and the event names
   it.

- **Access stays the outer gate for pages.** People are admitted through
  one Access group per instance, whose emails the control plane sets
  through Cloudflare's API with a token minted for that call (a ledger
  grant, `access-members`). No email enters the repository.
- **A page a member may not see answers 403** with their role; it never
  shows what it would have held.

### 9. Execution

Each step is one change, behind the one before. Steps marked **planner** are
`owner` tier or touch guarded paths. The others are briefs, per
[`2026-11-13-development-model-tiered-agent-briefs.md`](2026-11-13-development-model-tiered-agent-briefs.md).

1. **This ADR.** *Planner.*
2. **`core/member/`:** the vocabulary, the role check, memberships and
   invitations as values with pure transitions (§2, §4, §5), the last
   administrator rule, the match code, and the check of §8 as one function
   over values. Tests only; nothing calls it yet. *Sonnet.*
3. **`.config/roles.yaml`,** its sentinel policy, and its entries in
   `.config/approvals.yaml`. *Planner.*
4. **Storage:** the D1 migration, the store, the seed of the first
   administrator and of `agent`, and `member.list` and `member.show`.
   *Sonnet.*
5. **The gate:** §8 replaces `access::authorize`; passkeys and devices name
   their account; approvals record an account where they read `APPROVER`.
   In shadow first: the new check runs and logs beside the old
   one for a day of real changes, then decides. *Planner.*
6. **Invitations:** `member.invite`, `/join/<token>` and its public path,
   enrolment, the match code, `member.admit`, `member.role`,
   `member.remove`, `member.leave`, the pages' sign-in cookie, and the
   Access group with its grant.
   *Planner.* The `/v0/app/members` routes over step 2's functions, and
   push to every administrator: *Sonnet.*
7. **The app:** Members, the invite sheet, joining from a link, and Home by
   role, as `PigeonCore` states with tests. *Sonnet.*
8. **Wording:** "the owner" becomes "an administrator" in the pages', the
   cards' and the app's strings, their fixtures, and `CLAUDE.md`. *Haiku.*
9. **Service accounts:** the table, `service.create`, `service.suspend` and
   `service.remove`, and tickets recording their account. *Planner* for
   the operations; *Sonnet* for the tickets. No credential is issued beyond
   the seeded `agent`.
10. **An amendment, "Model variance",** recording each brief's class, heads,
    escalations and review findings.

Steps 4 onward wait for the accounts ADR to merge, and step 6
for the passkey-only ADR's enrolment.

### 10. Verification

- **Roles:** the core check refuses a wildcard, an unknown name, a fourth
  role, an `administrator` missing a capability, and a `service` holding
  anything but `read` and `propose`. The policy and the Worker agree, from
  one set of vectors.
- **User:** with `user: []`, every operation but one's own passkeys,
  devices, membership and leaving is refused, `read` included. Adding
  `read` to the file allows reads with no stored row changed.
- **Seed:** an instance with no rows gains one administrator, the owner's
  account, holding the existing passkeys and devices. With rows, nothing is
  seeded.
- **Invitations:** a link works once, and not after 72 hours. An enrolled
  invitee can do nothing before admission. An agent's proposal never
  returns a link. Admission by a passkey of another account, or a replayed
  assertion, is refused.
- **Two administrators:** both see one approval; the second proof answers
  "already approved by …". Neither approves their own change, nor changes
  their own role.
- **The last administrator** can't be removed, demoted or leave, even when
  two requests race.
- **Removal:** the removed account's device, passkey and Access session are
  refused on the next request; its Auto window has ended; a grant it left
  waiting is redeemed by another administrator.
- **Service accounts:** `agent` does what `pigeon-agent` did, and no more. A
  stored scope outside the ceiling is refused at the check, not trusted
  from storage. A suspended account's ticket is refused.
- **Shadow:** for step 5's day, the new check and the old agree on every
  real request, or each difference is explained in the step's amendment.
- `mise run ci` passes at every step.
