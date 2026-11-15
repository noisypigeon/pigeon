## An owner's approval is listed, pushed and re-issued by the control plane whenever a change needs the owner

* Date: 2026-11-15
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: security
* Governs: `src/pigeon-cli/crates/backplane/src/worker/approve.rs`, `src/pigeon-cli/crates/backplane/src/core/app/approvals.rs`, `src/pigeon-cli/crates/backplane/src/core/approval/listed.rs`, `src/pigeon-cli/crates/backplane/src/worker/listed.rs`

## Context

Owners now approve with passkeys only
([`2026-11-14-security-passkey-only-owner-proofs.md`](2026-11-14-security-passkey-only-owner-proofs.md)),
on `/approvals` or in the app. An approval the owner can't see blocks the change
until someone notices.

**What lists an approval today (proven from the code).**

- `/approvals`, the app's `GET /v0/app/approvals` and `digest_show.approvals`
  all read the same source: `worker/approve.rs::live` over the `Instance`'s
  inbox (`WAITING`). They can't disagree with each other.
- Only two paths add a change approval to that inbox:
  - an agent calling an owner operation (`worker/mod.rs::ask` →
    `approve::pending`);
  - `approval_wait` (`worker/mod.rs::wait`).
- `push::notify` runs only inside `ask`.
- Nothing lists or pushes when a change's stage becomes `needs: owner`
  (`core/change/stage.rs`).
- An entry is bound to its grant's head. `live` drops it once the head moves
  (`grants::still_applies`), and nothing lists the new head.

This follows from
[`2026-11-08-security-tiered-approvals-task-mandates.md`](2026-11-08-security-tiered-approvals-task-mandates.md)
§4–§5: "the agent passes it on".

**The cases.**

- **!464 (proven, from events).**
  - Checked, `needs: owner`, at event 17349. `approval.pending` came only at
    17361, 162 s later, when an agent called `change_approve`. The owner
    approved 11 s after that.
  - Its restack Container was lost three times (17402–17403). The session
    rebased and pushed, which made a new head. That head was checked at 17421,
    and listed only at 17424, 151 s later, again on an agent's call.
  - An approval already given did carry across the later same-patch restack
    (17431).
- **!438 (suspected; its events have aged out).** `approval_wait` listed an
  entry, and it was dropped once the head it named stopped being current.
- **!439 was listed** because an agent asked promptly and its head didn't move.
- No list filter hides `behind` or yielded changes, and none depends on tier.

## Implementation plan.

### 1. The invariant

For every open change whose stage is `needs: owner` and whose `next` is an owner
grant operation (`change.authorize` while proposed, `change.approve` while
checked), there is exactly one inbox entry. That entry:

- is for that change and operation;
- has a page grant (`PAGE_TTL`) minted for the change's current head;
- is unexpired and unused;
- is pushed to the owner once per head.

No agent action is needed. A change covered by a window or mandate is
`needs: agent` and is not listed.

### 2. `ChangeRun` publishes

- After every save, `ChangeRun` computes what it owes the owner with a pure
  `core/approval/listed.rs::owed(change, stage) -> Option<Owed>`, where
  `Owed` is `{number, operation, head}`. It reconciles that against its own
  entry:
  - **owed, none listed, or the listed head differs:** mint a page grant
    in-process (it holds `GRANTS`, so there is no re-entrant call), revoke the
    entry's old grant, and `PUT` the entry to the `Instance`'s
    `/approvals/put`. That route replaces any entry for the same change and
    operation. Then append `approval.pending`, attributed to `control-plane`,
    and `push::notify`.
  - **owed and listed for this head with under 2 hours left:** re-issue as
    above, without a second push.
  - **nothing owed:** remove the change's entries.
- The pure decision is `listed::reconcile(owed, listed, now) -> Action`, with
  `Action` one of `Keep`, `Issue { push }` or `Remove`. Its tests reproduce:
  - !464: checked → `Issue{push}` with no call; a session push to a new head →
    `Issue{push}` for the new head;
  - !438: listed for head A, now at head B → `Issue`;
  - expiry;
  - a covered change → nothing;
  - approved → `Remove`.
- `worker/listed.rs` holds the effectful side.

### 3. An agent's call reuses the listed approval

`ask` and `wait` look up the live entry for `(number, operation)` before
minting. When the head matches, they answer its link. Otherwise they publish
through `ChangeRun` (§2). There is never a second entry.

### 4. Restacks

- An approval already **given** carries across a clean or disjoint restack of
  the same patch, as `core/change/restack.rs` does today, unchanged.
- A session's own push resets an approval and §2 lists the new head, as
  today. This was !464's third link (`5f1bccf` → `efd44b8`, same patch). Whether
  a session push with an identical patch should carry the approval is the
  owner's call. It would come as an amendment here.
- This ADR owns the carry rule, both restack and session push. The
  job-Container session (`pigeon:reliability`) owns Container start, loss and
  back-off, by the status-review session's ruling on 2026-11-15.
- A **pending** grant is never carried. It names the head the owner saw. A new
  head while `needs: owner` re-issues per §2, and the old link answers "ask
  for a new one".

### 5. The digest checks itself

- On each hourly digest, the `Instance` lists the open changes whose stage is
  `needs: owner` with an owner grant operation. It compares them with `live`.
- Each change missing from `live`:
  - gives a digest line, `!<n> needs you but had no approval listed;
    re-issued`;
  - appends `approval.unlisted`;
  - sets `needs_owner`;
  - is asked to republish (`ChangeRun` `/listed`).
- `digest_show.unlisted` lists them, so a regression is visible at once.

### Execution

1. This ADR, and an amendment to the tiered-approvals ADR saying §4–§5's
   "the agent passes it on" is replaced. (planner)
2. `core/approval/listed.rs`: `owed`, `reconcile` and their tests, including the
   !438 and !464 sequences. (brief, sonnet)
3. `worker/listed.rs`, `ChangeRun`'s publish after save, `Instance`
   `/approvals/put`, and `ask`/`wait` reuse. (brief, sonnet; planner reviews)
4. The digest's self-check, `unlisted` and `approval.unlisted`. (brief, sonnet)
5. Verify live. The next owner-tier change is listed, and pushed, with no
   agent call. Record that here. (planner)

`Governs` gains each step's files as it lands.

## Amendments

### 2026-11-15: Review of !492 against !484: they agree

!484 amended
[`2026-11-07-development-control-plane-deploys-itself.md`](2026-11-07-development-control-plane-deploys-itself.md),
[`2026-11-08-security-tiered-approvals-task-mandates.md`](2026-11-08-security-tiered-approvals-task-mandates.md),
[`2026-11-12-runtime-ios-app-approvals-insights.md`](2026-11-12-runtime-ios-app-approvals-insights.md)
and
[`2026-11-13-development-session-stages-subscriptions-breaker.md`](2026-11-13-development-session-stages-subscriptions-breaker.md).
A failed deploy now holds Auto windows and mandates, and a held change is in
the new stage `held`.

- **They agree.**
  - `held` is only for an approved change. It has `needs: platform` and
    lists nothing for the owner.
  - §1's invariant covers only `needs: owner` with `change.authorize` or
    `change.approve`, so `listed::owed` owes nothing for a held change, and
    `worker/listed.rs` removes any entry left for it.
- **A held window still authorizes and approves,** so a change it covers stays
  `needs: agent` and is not listed, as before.
- **`worker/mod.rs`,** where both work: !492 adds only `mod listed;`. That
  doesn't touch !484's hooks.
- **!492 changes nothing in those ADRs.**
### 2026-11-15: Steps 2 and 3 land, by the planner; a plan owes its authorize before any push

- **The planner carries both steps.** `core/approval/**`,
  `worker/approve.rs` and `worker/elicit.rs` are guarded, so a brief's change
  is refused there. Step 2 started as brief b12 (!471), which the planner took
  back. Step 3 is !487.
- **`Owed.head` is optional.** A proposed change has no head until it is
  pushed, and its authorize is owed all the same. `change.approve` still needs
  a head, and a listed entry matches only the same head, `None` included.
- **Step 3 as built:**
  - `ChangeRun::save_after` calls `worker/listed.rs::reconcile` for a change
    that is, or was, proposed or checked;
  - `POST /listed` runs the same reconcile on demand, for step 4;
  - the `Instance`'s `/approvals/put` keeps one entry per change and
    operation;
  - `approve::pending` answers the listed entry while it applies, and puts
    rather than adds.
- **Paths.** `Governs` gains `core/approval/listed.rs` and
  `worker/listed.rs`.
