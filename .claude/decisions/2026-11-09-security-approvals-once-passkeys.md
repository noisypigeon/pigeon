## Approvals: routine ones automatic, one proof per change, the owner for what destroys or touches secrets, and passkeys

* Date: 2026-11-09
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: security
* Governs: `.config/approvals.yaml`, `src/pigeon-cli/crates/backplane/src/approvals.rs`, `src/pigeon-cli/crates/backplane/src/passkeys.rs`, `src/pigeon-cli/crates/backplane/src/worker/passkey*`, `src/pigeon-cli/crates/backplane/tests/passkeys.rs`

## Context

[`2026-11-08-security-tiered-approvals-task-mandates.md`](2026-11-08-security-tiered-approvals-task-mandates.md)
gave every operation a tier: auto, confirm or code. Mandates approve a task
once, and approval pages work from a phone. A day of using it showed four
problems:

- **Confirm still waits for the owner.** A routine merge, such as docs or
  tests, sits on a tap until the owner is back, though §1 already holds that a
  confirm can approve only what isn't dangerous.
- **A code-tier change asks for the code three times.** Authorize, approve and
  merge each mint their own grant. On 2026-11-09, changes 93 to 109 took about
  fifty codes. Each code is a 30-second TOTP step used once, so approving two
  links in a row means waiting for the next step.
- **The owner's attention goes to the wrong places.** Among those fifty, the
  ones that mattered were a platform apply that could rotate live service
  tokens, and a pasted account token. A tier keyed on paths can't say that
  touching a secret, or destroying something, is different.
- **The code is the only proof.** It can't be typed faster than it rolls, and
  it means unlocking a second app for every link.

The owner asked for four things:

1. Low-risk approvals, today's confirm tier, are approved without them.
2. A code-tier change is approved once for all its stages.
3. Destructive actions, and changes that add, read or expose secrets, still
   need them every time.
4. Passkeys in the browser instead of the code.

What the design rests on:

- **The bound job classifies every head from `main`'s rules,** and the Worker
  trusts that report (tiers ADR §2). A new class of rules fits the same path.
- **A grant is already bound to one change, one operation and one head.** It
  records its tier when minted, and is used once.
- **Approval pages are the Worker's own,** behind Access, same-origin. They
  check the code with `Instance`'s `/code/verify` guard: each step once, and
  five wrong codes lock the hour.
- **WebAuthn needs a script.** `navigator.credentials` runs only in a page's
  script. Today's pages send `default-src 'none'`. The proof itself, a P-256
  signature over the authenticator data and the client data's hash, is
  checked on the server. The `p256` crate is already in the dependency tree,
  and builds for wasm.

## Implementation plan.

### 1. Four tiers

| Tier | Who approves | How |
|---|---|---|
| **auto** | the control plane | no prompt; recorded as `policy:<rule>` or `mandate:<id>`, and listed in the digest |
| **confirm** | the control plane, for now | as auto; see below |
| **code** | the owner, proven, once per change | one proof grants every remaining stage of that change (§3) |
| **owner** | the owner, proven, each time | one proof per operation, never carried over and never in a mandate |

- **Confirm becomes automatic.** The control plane approves a confirm-tier
  operation itself, as `policy:<rule>`.
  - The tier stays as a name, so a rule can later bring the tap back without
    a migration.
  - The agent may still raise any operation to code or owner, to have the
    owner look (tiers ADR §1).
- **A proof is the authenticator code or a passkey (§4).** Both are accepted
  wherever the owner is asked.
- **A tier may be raised, never lowered,** as before. Nothing makes an owner
  operation code, auto or part of a mandate.

### 2. What the owner always approves: the `owner` tier

`.config/approvals.yaml` gains two lists, read from `main` as today:

- **`secrets:` paths whose change adds, reads or exposes a secret:**
  - `**/secrets.config.yaml`;
  - `.mcp.json` and `.claude/settings*.json`;
  - `catalog/cloudflare/capabilities.yaml`;
  - the modules that mint keys or tokens:
    - `catalog/cloudflare/modules/api-token/**`;
    - `catalog/cloudflare/modules/access-service-token/**`;
  - backplane's sealing and secret handling:
    `src/pigeon-cli/crates/backplane/src/worker/platform.rs` and
    `src/pigeon-cli/crates/backplane/src/platform_job.rs`.
- **`secret_lists:` files whose secret lists a change may lengthen,** such as
  `**/worker.yaml`. The bound job compares the head's `secrets:` with the
  base's. Adding a name makes the head's merge `owner`; removing one doesn't.
  Today that means `infra/pigeon/workers/control/worker.yaml`'s list.

**Owner operations, whatever the files:**

- **Destructive:**
  - `change.close` of a change that has pushed commits. Closing a change that
    never pushed is auto, because nothing is lost.
  - a `platform.apply` whose plan deletes or replaces anything. Today that
    plan is refused outright; this tier is for §5's teardown.
  - `infra.apply` with any delete;
  - a `sites.publish` that deletes objects beyond the build's own pruning;
  - a teardown, when §5 of the blue-green ADR adds it.
- **Secrets:**
  - `platform.plan`, which takes an account token;
  - any operation that takes, sets or reveals a secret value, such as
    `workers secrets put`, once the control plane serves it;
  - registering or removing a passkey (§4).
- **Power:**
  - granting, widening or resuming a mandate.

**What each change's tiers become:**

- **Merge:** a head whose files touch `secrets:`, or lengthen a secret list,
  is `owner`. Otherwise, as today:
  - guarded and deploying files are `code`;
  - everything else is `confirm`, now automatic.
- **Authorize:** a scope reaching `secrets:` is `owner`. Otherwise, as today.
- **The rules are guarded.** `approvals.yaml` stays guarded, so a change to
  these lists needs the owner's code to merge. It is also in `secrets:`, so it
  needs `owner` each time.

### 3. One proof per change

- **The first code-tier operation of a change mints a change grant.** That is
  usually its authorize, or its approve when authorizing was automatic. The
  owner sees the page as today, plus "covers every remaining stage of change
  `<n>`". One proof redeems it.
- **The change grant then approves, as the owner who gave it:**
  - the change's later stages, authorize, approve and merge, at the `code`
    tier;
  - only while all of these hold:
    - its head's merge tier is no higher than `code`. A head that becomes
      `owner`, by touching a secret path or lengthening a secret list, asks
      again;
    - its files stay inside the scope it was authorized with. A head reaching
      outside asks again;
    - its checks pass, as today;
    - it's at most 24 hours old, the page grant's lifetime.
  - Each later stage is recorded as `grant:<id>`, with the approver who gave
    the proof, and is listed in the digest.
- **Restacks.** A restack keeps the change grant when the patch and tier are
  the same, as approvals carry over today (concurrent-sessions ADR). A
  changed patch keeps it too, provided the conditions above still hold. The
  grant approves the scope, not one diff.
- **What the owner gives up.** With one proof at authorize, the owner no
  longer sees each head before it merges. That is the same trust a mandate
  gives, for one change and 24 hours.
  - What protects them: the scope, the guarded and secret lists, the checks,
    and the digest.
  - **The owner may ask to review the patch,** with `review: true` on the
    change grant's page. The grant then covers only up to `checked`, and the
    merge asks again.
- **Close** follows §2. Closing a change that has pushed commits is `owner`,
  and a change grant never covers it.

### 4. Passkeys

**What a passkey is here.** A WebAuthn credential for the owner. It proves
the owner wherever a code is accepted: approval pages, the platform page and
the mandate page. Claude's card can't run WebAuthn, so it keeps the code.

**Registering one** happens on `/passkeys`, behind Access:

- Registering is `owner` tier, proven with the authenticator code, so a
  passkey never registers itself.
- The page calls `navigator.credentials.create` with:
  - the relying party's ID: the page's hostname. `workers.dev` is a public
    suffix, so the ID is the full host;
  - `ES256` only (`-7`);
  - user verification required;
  - resident keys preferred;
  - `attestation: "none"`.
- The Worker keeps, in `Instance` storage and never in the repository:
  - the credential's ID;
  - its public key, as SPKI from `getPublicKey()`, so no CBOR is parsed;
  - its sign counter;
  - a name;
  - when it was created and last used.
- A passkey is bound to its hostname. When `control.pigeon.dev` serves the
  pages (blue-green ADR, step 4c-3), passkeys are registered again there.
  `/passkeys` lists them, and removes one with a proof.

**Proving with one,** on any page that asks for a proof:

- The Worker mints a challenge: 32 random bytes, kept with the grant for 5
  minutes, used once.
- The page's script calls `navigator.credentials.get` and posts the result to
  the same URL as the code field would.
- The Worker checks:
  - `clientDataJSON`: its `type` is `webauthn.get`, its challenge is the
    grant's, and its origin is `https://<host>`;
  - the authenticator data: its RP ID hash is SHA-256 of the host, and the UP
    and UV flags are both set;
  - the signature, verified with `p256` over the authenticator data followed
    by SHA-256 of `clientDataJSON`;
  - the counter: if both counters are non-zero, it must go up. Synced passkeys
    report 0.
- Then it redeems the grant, as a valid code would.
- Failures count toward the same lock as wrong codes: five in an hour.

**The script:**

- It is one static file, `/passkey.js`, served by the Worker with an exact
  hash.
- Pages that ask for a proof send `script-src 'self'`; everything else stays
  `default-src 'none'`. `connect-src` is `'self'`.
- It reads nothing from the page except the challenge and the form's action.
- The page works without it: the code field stays.

### 5. Told what happened

- **The digest lists every automatic approval:**
  - by rule, as `policy:<rule>`;
  - by change grant, as `grant:<id>` with its approver;
  - by mandate, as `mandate:<id>`.

  The owner reads it hourly, as today.
- **Push notifications** go out only for `code` and `owner` requests, a
  mandate pausing, or a failed deploy.

### Execution

Each step is its own change. Each touches guarded paths, so each is approved
at today's code tier, three codes each, until step 3 lands.

1. **This ADR,** with a `CLAUDE.md` pointer.
2. **Tiers:**
   - `owner` in `approvals::Tier`;
   - `secrets:` and `secret_lists:` in `approvals.yaml`, and their
     classification in the bound job;
   - confirm approved automatically;
   - closing a pushed change is `owner`;
   - digest entries.

   Durable Object state and wire types keep tolerating unknown fields, with
   fixtures in `backplane/tests/state/`.
3. **Change grants:** minting, the page's wording and `review`, and the
   conditions in §3.
4. **Passkeys:** `/passkeys`, `/passkey.js`, the challenge, verification,
   storage, and the pages' proof field.
5. **The owner's operations outside changes** (`platform.plan`, a
   `platform.apply` that destroys, mandate grants) take a proof from either
   source, and are labelled `owner` on their pages.

### Verification

- **A docs-only change** is proposed, checked, approved and merged with no
  prompt, and appears in the next digest as `policy:routine`.
- **A deploying change** asks once, at authorize. After that it is approved
  and merged without the owner, and the digest shows `grant:<id>`.
- **The same change, with a later head that adds a Worker secret,** asks
  again at merge, labelled `owner`.
- **Closing a pushed change** asks every time. Closing an unpushed one
  doesn't.
- **A passkey registered with the code** approves a code-tier link from a
  phone. A replayed assertion is refused, and so is one for another grant or
  origin. Five failures lock the hour, as codes do.
- **With the script blocked,** the code field still works.

## Amendments

### 2026-11-12: Approving a head consents to its merge

A change asked the owner to approve its head, then again to merge it. Below
the `owner` tier the change grant already answered the second; at `owner` it
asked again for the same head.

- **An approval of a head is consent to merge it.**
  - Approving enqueues the merge as the approver. The merge still waits for
    its turn and its gate.
  - At the `owner` tier, one proof covers approve and merge of the same head.
    It is still one proof per operation for everything else §2 lists.
  - `change.merge` on an approved head answers its place in the queue.
- **The merge asks again only when the head moved under the approval:**
  - a session push, or a restack that isn't disjoint with the same patch;
  - a reconcile;
  - a merge tier higher than the one approved;
  - an approval older than 24 hours, the page grant's lifetime.
- **The owner can still hold a merge,** with `hold: true` on the approval
  page, as `review: true` holds a change grant. A held approval doesn't
  enqueue.

**Execution:** `approve` sets the merge to start after the gate, unless held;
`by_grant` and the owner check accept the merge of a head approved by proof,
under the conditions above; the approval page gains `hold`.
