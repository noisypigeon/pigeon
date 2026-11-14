## A passkey is the only proof: authenticator codes are removed, without a lockout

* Date: 2026-11-14
* Model: Claude Fable 5.2, planning as the `opus` class in a model-variance test
* Author: Willow Graysen (@noisypigeon).
* Category: security
* Governs: `src/pigeon-cli/crates/backplane/src/core/approval/passkeys.rs`, `src/pigeon-cli/crates/backplane/src/core/approval/totp.rs`, `src/pigeon-cli/crates/backplane/src/totp.rs`, `src/pigeon-cli/crates/backplane/src/worker/elicit.rs`, `src/pigeon-cli/crates/backplane/src/worker/passkeys.rs`, `src/pigeon-cli/crates/backplane/src/worker/passkey.js`, `src/pigeon-cli/crates/backplane/tests/totp.rs`, `src/pigeon-cli/crates/backplane/tests/passkeys.rs`

## Context

The owner proves an approval in one of two ways today:

- **A 6-digit authenticator code** (TOTP), checked against the `APPROVER_TOTP`
  Worker secret, from the 2026-11-07 amendment of
  [`2026-11-06-development-control-plane-approvals-images-state.md`](2026-11-06-development-control-plane-approvals-images-state.md)
  and
  [`2026-11-08-security-tiered-approvals-task-mandates.md`](2026-11-08-security-tiered-approvals-task-mandates.md).
- **A passkey**, from
  [`2026-11-09-security-approvals-once-passkeys.md`](2026-11-09-security-approvals-once-passkeys.md)
  §4, on `platform.pigeon.dev` and in the app
  ([`2026-11-12-runtime-ios-app-approvals-insights.md`](2026-11-12-runtime-ios-app-approvals-insights.md)
  §3).

The owner asked for the code to go, so that a passkey is the only proof.

### Why the code goes

- **It can be phished and relayed.** A code typed into any page or card works
  for whatever grant the receiver redeems within the step. A passkey signs
  only for `platform.pigeon.dev`, over a challenge the Worker made.
- **It is a shared secret.** `APPROVER_TOTP` sits in the Worker. Whoever reads
  it approves everything. A passkey's private half never leaves its
  authenticator.
- **The card's text is the client's.** The code card is drawn by the MCP
  client, which the 2026-11-07 amendment accepted as a residual risk.
- **Three sessions assume it.** The users and roles, profile and provider
  OAuth designs each take an account to authenticate with passkeys only.

### What exists, read from `main`

- **One check for every code:** `Instance`'s `/code/verify`
  (`worker/elicit.rs`), under `totp::Guard`: each step once, five wrong in an
  hour lock it. `worker/passkeys.rs` counts passkey failures on the same
  guard.
- **One keyring,** `core/approval/passkeys.rs`: any number of ES256
  credentials for one user, `pigeon-owner`. A challenge is 32 random bytes,
  kept 5 minutes with a `purpose`, used once.
- **A grant binds the proof.** It names one operation, one target and, for a
  change, one head. The purpose of a change approval's challenge is
  `grant:<key>`.
- **A person's direct call needs no proof.** `access::authorize` lets a
  person's Access identity run an owner's operation, as with `cloudflared`.
- **`/attend` isn't on `main`.** It is covered by §1's rule for new surfaces.

### The inventory

Every place a code is accepted or asked for, or the owner is proven without a
passkey:

| # | Surface | Where | Today | Becomes |
|---|---|---|---|---|
| 1 | The MCP card | `elicit::card`, `approval_wait`, `mcp.rs`'s form | code only | a link to the approval page (§2) |
| 2 | Change approvals, `/approve/<grant>` | `worker/approve.rs` | passkey or code | passkey |
| 3 | Mandates, `/mandate/<id>`: grant, widen, resume | `worker/mandates.rs` | code only | passkey |
| 4 | Minting grants, `/grants/<id>`: request, rotate, revoke | `worker/minting.rs` | code only | passkey |
| 5 | The CI window, `/ci-window/<id>` | `worker/ci_window.rs` | code only | passkey |
| 6 | Platform plans and applies, `/platform/<id>` | `worker/platforms.rs` | code only; the plan also takes an account token | passkey; the token field stays |
| 7 | `/passkeys`: register, remove | `worker/passkeys.rs` | code only | an existing passkey, or an enrolment (§3) |
| 8 | `/devices`: unlink | `worker/devices.rs` | code only | passkey |
| 9 | `/app/link` | `worker/devices.rs` | passkey or code | passkey |
| 10 | The app: approvals, Auto windows | `worker/app.rs` | passkey only | unchanged |
| 11 | The docs gate, `/docs/` | `worker/docs.rs` | passkey only | unchanged |
| 12 | The CLI | `pigeon backplane totp-secret` | prints the secret | removed; `enroll-secret` (§3) |
| 13 | The secret | `APPROVER_TOTP` in `worker.yaml` | set | removed |
| 14 | A person's direct call | `access::authorize` | no proof | pending, as an agent's call is (§2) |
| 15 | `/attend` and any later page | not on `main` | | passkey from its first change (§1) |

Out of scope:

- who may approve what, which the users and roles ADR decides;
- how a person signs in to the pages, which stays Access until the profile
  ADR changes it;
- renaming the `code` tier. It stays as a name in `approvals.yaml`, stored
  state and the wire, as `confirm` did. Text calls it "proven with a passkey".

## Implementation plan.

### 1. One proof, one way to ask for it

- **A proof is a passkey assertion.** Nothing else proves a person: not a
  code, not an Access identity, not a device session, not a provider's OAuth
  sign-in.
- **One function takes it:** `proof::on_page(env, settings, form, grant)`, in
  `worker/passkeys.rs`. Every page in the inventory calls it and nothing
  else. It mints no grant and runs no operation.
- **A new surface asks through it.** A page or route that proves a person in
  any other way is refused in review. `/attend` starts this way.
- **Every page that asks sends the proof pages' policy:** `script-src 'self'`
  for `/passkey.js`, and `connect-src 'self'`. Without the script the page
  says that approving needs a passkey.

### 2. How an MCP client asks the owner

A Claude session can't run WebAuthn, and nothing it shows can be trusted to
describe the operation.

**The decision: the session gets a link to the Worker's own page, and the
owner's passkey signs there or in the app.** No form is shown, and nothing is
typed.

- **An owner's operation called by a session answers `pending` at once,** as
  today, with the page's link, an `approval.pending` event, and a push to the
  app and the browser.
- **`approval_wait` shows that link as a URL card** (`-32042`, URL-mode
  elicitation) on a client that declared it, and as text otherwise. It
  returns at once. The session then follows events, as it already does.
- **The form card goes:** `elicitation/create`, the held-open SSE call, the
  one pending card, and the confirm card. A hook can answer a form. It can
  only skip a link.
- **The app is the second path.** The push opens the approval in the app,
  which reaches `/v0/app/*` by its device key, outside Access. With Access
  down, change approvals still work there.
- **A person's direct call is asked the same way.** After the cutover (§4), a
  person's Access identity calling an owner's operation gets `pending` and the
  link. Access says who is signed in; it isn't a proof.

**What keeps it phishing-resistant:**

- **The origin.** The assertion is made for the relying party
  `platform.pigeon.dev`. A link to any other host gets no signature the
  Worker accepts.
- **The page is the Worker's.** It shows the operation, title, head, scope
  and checks from the server's state, never the session's words.
- **The challenge is bound.** It is no longer bare random bytes:

  ```
  challenge = SHA-256("pigeon-proof-v1" \n purpose \n operation \n target \n head \n nonce)
  ```

  - `nonce` is 32 random bytes, kept 5 minutes, used once.
  - `operation`, `target` and `head` come from the grant, never from the
    request. For a change, `head` is its head SHA. For a mandate, a minting
    grant, a CI window or a platform run, it is the SHA-256 of what the page
    shows. A purpose without a grant (`view:docs`, `window:start`,
    `passkey:add`) leaves them empty.
  - The Worker computes it again from the grant when the answer arrives. A
    head that moved, or another grant, gives another challenge, and the
    assertion fails.
- **The evidence is kept.** An approval records the passkey's ID, the nonce,
  the client data, the authenticator data and the signature. Anyone holding
  the public key can check later what was signed.
- **The failure lock is per passkey.** Five refused assertions from one
  passkey in an hour lock that passkey for the hour. An unknown credential is
  refused and locks nothing. A signature can't be guessed, so no shared lock
  is needed, and none can be used to lock the owner out.

### 3. Recovery

- **Several passkeys per account.** Up to 8. `/passkeys` lists each with its
  name, when it was added and last used, and whether it is synced.
- **Synced and device-bound are both accepted.**
  - The Worker records the authenticator data's backup flags at registration:
    eligible means synced, such as iCloud Keychain; clear means device-bound,
    such as a security key.
  - Passkeys synced through one cloud account are lost together. So the
    cutover (§4) needs two passkeys, at least one device-bound.
  - Attestation stays `none`, and user verification stays required. New
    registrations ask for a resident key, so a later sign-in needs no name.
- **Adding and removing take a passkey.** Adding one is proven by an existing
  one (`passkey:add`), and removing by any (`passkey:remove:<id>`). After the
  cutover, a removal that would leave fewer than two is refused. Each is
  `owner` tier, pushed to every device and listed in the digest.
- **A lost device.** The owner proves with another passkey and removes the
  lost one. A lost phone's device session is unlinked on `/devices`, or lapses
  in 30 days.
- **Break-glass, when no passkey is left: a one-time enrolment.**
  - `pigeon backplane enroll-secret` prints a random token on stderr and
    `<expiry>.<SHA-256 of the token>` on stdout. The expiry is at most one
    hour ahead.
  - The owner pipes stdout to
    `pigeon cobot workers secrets put --root . pigeon-control PASSKEY_ENROLL`.
    That needs the Cloudflare account's credential, the same root of trust
    that set `APPROVER_TOTP`.
  - `/passkeys/enroll`, behind Access and for a person only, takes the token
    and registers one passkey. The `Instance` records the hash as used.
  - A used, expired or absent value does nothing, so nothing standing is left
    to steal. Five wrong tokens lock enrolment for the hour, and only
    enrolment.
  - An enrolment is pushed to every device and leads the next digest.
  - This also registers the first passkey of a fresh instance.
- **The last resort is unchanged:** the local `pigeon cobot workers deploy`.
  The `infra/` keys stay break-glass and are never a proof.
- **The relying party.** Passkeys are bound to `platform.pigeon.dev`. A
  change of hostname needs an enrolment on the new one first.

### 4. The migration, with no lockout

The order is fixed. Each stage is proven before the next begins.

1. **Passkeys everywhere, codes still accepted.** Every page in the inventory
   takes a passkey through §1's function. The card becomes the link.
   Enrolment exists.
2. **The owner proves it.** The `Instance` keeps the evidence, and
   `/passkeys` and the digest show it as `proofs`:
   - two passkeys, at least one device-bound, each used in the last 14 days;
   - a change approved on the page, and one in the app;
   - a passkey added with a passkey;
   - an enrolment drill: a third passkey enrolled with `enroll-secret`, then
     removed.
3. **Codes are refused.** `worker.yaml` sets the var `PROOFS: passkey`.
   - **The Worker obeys it only while stage 2 holds.** Otherwise it keeps
     taking codes, and the digest says what is missing. The gate is in code,
     not in anyone's memory.
   - The first time it holds, the cutover is recorded and never undone by
     itself.
   - Then: no page shows a code field, `/code/verify` refuses, and a person's
     direct call goes to `pending`.
   - `APPROVER_TOTP` and `totp.rs` stay for a soak of three days. The way
     back in that time is an enrolment, which needs no code and no passkey.
4. **Codes are removed.** `totp.rs`, its core and tests, `/code/verify`, the
   code field, `totp-secret`, and `APPROVER_TOTP` from `worker.yaml`. The
   owner deletes the secret's value and the authenticator's entry. `PROOFS`
   goes too: passkeys are the only behaviour.
5. **The words.** `CLAUDE.md`, tool summaries and page text say "with a
   passkey". The three ADRs named in Context gain an amendment pointing here.

A page other than the change approval that fails with a passkey isn't a
lockout: its fix is a change, and the change approval page still approves it.

### 5. Several users

For the users and roles, profile and provider OAuth designs:

- **A passkey belongs to one account.** The keyring is kept per account, and
  the WebAuthn user handle is the account's ID. Today's keyring becomes the
  first Administrator's; its credentials keep working.
- **A proof names its account and its passkey.** An operation runs as the
  account proven, not as `APPROVER`. That secret goes when accounts land.
- **An Administrator approves with their own passkey.** Which operations a
  role may approve is the roles ADR's. This ADR only says how a person is
  proven.
- **Recovery between Administrators.** An Administrator with no passkey left
  is given an enrolment by another, at `owner` tier with that one's passkey.
  The secret's break-glass is for when no Administrator can prove.
- **§4's gate is per instance:** the cutover needs one account that passes
  it. An account added later starts with two passkeys.
- **A provider's OAuth sign-in identifies and never proves.** Linking one to
  an account is `owner` tier, with a passkey.

### Execution

Each step is its own change, in this order. The planner does the guarded,
secret and security steps. Briefs go to executors per
[`2026-11-13-development-model-tiered-agent-briefs.md`](2026-11-13-development-model-tiered-agent-briefs.md).

| # | Step | Who |
|---|---|---|
| 1 | This ADR | planner |
| 2 | The core: the bound challenge, the backup flags, the per-passkey lock in its own `guard.rs`, `passkey:add` and `passkey:remove`, the evidence | planner |
| 3 | `proof::on_page`, and the guarded pages on it: approvals, mandates, `/passkeys` | planner |
| 4 | The other pages on it: minting grants, the CI window, platform runs, `/devices`, `/app/link`, with a test per page | sonnet |
| 5 | The card: `approval_wait`'s link, the form card removed | planner |
| 6 | Enrolment: the core, `/passkeys/enroll`, and `PASSKEY_ENROLL` in `worker.yaml` (`owner`) | planner |
| 7 | `pigeon backplane enroll-secret`, on step 6's core function, with tests | sonnet |
| 8 | `proofs` on `/passkeys` and in the digest; the gate | planner |
| 9 | The owner's stage 2, by hand | owner |
| 10 | `PROOFS: passkey`, and the person's direct call | planner |
| 11 | After the soak: the removal of stage 4, with `approvals.yaml`'s list and this ADR's `Governs` | planner |
| 12 | The words outside guarded files: `CLAUDE.md`, `wire.rs`'s summaries, the app's link text, `worker.yaml`'s comments | haiku |
| 13 | The pointer amendments, and this ADR's "Model variance" amendment | planner |

- Steps 2 to 8 each keep the code working.
- Step 10 doesn't start before the owner says stage 2 is done and `proofs`
  shows it.
- Step 11 doesn't start before the soak ends.

### Verification

- **The card.** `approval_wait` on a pending approval answers a URL card or a
  link. No client is sent a form.
- **Binding.** An assertion replayed, made for another grant, or made before
  the head moved is refused. The stored evidence verifies against the
  passkey's public key.
- **Every page.** Each of the inventory's pages redeems its grant with a
  passkey, in a test and once by the owner.
- **Recovery.** A passkey is added with a passkey. An enrolment works once,
  and is refused when used, expired or wrong. Removing the second-to-last
  passkey is refused after the cutover.
- **The gate.** With `PROOFS: passkey` and one passkey, a code still works and
  the digest says why. With stage 2 met, a code is refused everywhere.
- **Direct calls.** After the cutover, a person's `change_approve` through
  `cloudflared` is pending.
- **Removal.** No file names `totp`, `APPROVER_TOTP` or `/code/verify`, and
  `mise run ci` passes.
