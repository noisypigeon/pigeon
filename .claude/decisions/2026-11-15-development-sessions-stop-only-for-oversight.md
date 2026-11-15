## A session stops for the owner only where the owner adds oversight; the control plane carries a window's approvals, and nothing else waits on a person

* Date: 2026-11-15
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development
* Governs: `CLAUDE.md`, `.claude/settings.json`, `src/pigeon-cli/crates/backplane/src/core/change/stage.rs`, `src/pigeon-cli/crates/backplane/src/worker/mod.rs`, `src/pigeon-cli/crates/backplane/src/worker/api.rs`, `src/pigeon-cli/crates/backplane/src/wire.rs`, `src/pigeon-cli/crates/backplane/src/mcp.rs`, `src/pigeon-cli/crates/backplane/src/sessions.rs`, `src/pigeon-cli/crates/backplane/src/tasks.rs`, `src/pigeon-cli/crates/backplane/src/worker/push.rs`, `src/pigeon-cli/crates/backplane/src/core/session/launch.rs`

## Context

On the night of 2026-11-14 to 15, sessions stopped seven ways, waiting for the
owner to say "go", "proceed" or "approve". In most of these the owner had
already decided, or the decision wasn't the owner's. Each stop below is taken
from the session's own events. Times are UTC on 2026-11-15 unless marked.

### The stops

1. **The iOS planner (`…kVXKchUMBvxbUE5dL`) waited for a go-ahead it already
   had.**
   - Its seed said "Owner Auto mode (window:7) is active, so carry the work
     end to end". The owner then said "let's proceed with building this now"
     (02:11).
   - At 03:01 and 04:27 it called `grant_request {name: observability}`.
     That tool is in `.claude/settings.json`'s `ask` list, so the harness
     prompted "Approve or deny pigeon — Grant Request", and the prompt was
     denied.
   - A denied prompt correctly stops the agent. At 04:30 it said "I'm waiting
     for your go-ahead to keep going: filing b17 …", and waited until the
     owner's "Proceed" at 04:49.
   - `grant_request` only asks: it answers with the page where the owner
     mints the grant with their passkey. The prompt was a second ask in front
     of the real one, and its denial halted unrelated, already-approved work.
   - **Kind:** a missing permission, plus a gap in the guidance. One item
     waiting on the owner stopped every other step too.
2. **The job Containers planner (`…jScBJuknjExtvJhB5`) asked the owner to
   pick an approval path** (03:23): "(a) … approve under your open window …
   or (b) you approve each one yourself". The session-stages ADR's
   2026-11-14 amendment, and `CLAUDE.md`, already settle this: an open
   window is the owner's approval.
   - It asked because executors' classifiers kept refusing `change_approve`
     (stop 3).
   - **Kind:** ambiguous guidance, caused by a tooling problem.
3. **The harness classifier refused `change_approve` ("Undermining
   Oversight") twice.**
   - **!475, a sonnet executor on b13.** At 02:35 its stage read `checked`,
     `needs: agent`, `next` "ask now (`change_approve`): window:9, the
     owner's, approves it". Following that, it sent
     `{number: "475", ticket}` and was refused. When the owner allowed a
     retry, the control plane refused it as well: "carries out b13; its
     planner reviews the head first". So the stage named an operation that
     the executor may never call (the briefs ADR §2: "never calls an owner
     operation"), for a head that was still awaiting review.
   - **!507, a planner.** It called `brief_review accept`, then, with no
     text between, `change_approve {number: 507, sha, ticket}`, and was
     refused. That call was unnecessary: under the briefs ADR's 2026-11-14
     amendment (!457), the control plane had already approved the accepted
     head as `window:9`, in the same second.
   - Why the classifier reads these as self-approval: `change_approve`'s
     description is "Approve a checked change's head for merge … never by
     its proposer". The caller is the proposer, or the proposer's executor,
     and nothing it sends names the window. The briefs ADR's 2026-11-14
     amendment found that seeded cloud sessions don't apply
     `permissions.allow`, so the allow-list can't help. The classifier is
     right to refuse an agent that appears to approve its own work. Pigeon's
     job is to stop asking agents to make that call at all.
   - **Kind:** a tooling bug: the stage's `next` and the tool descriptions
     route the window's approval through the agent.
4. **Permission prompts for pigeon tools.** These are the same `grant_request`
   calls as stop 1. `.claude/settings.json` puts `grant_request`,
   `git_tokens` and others under `ask` as "destructive", but:
   - `grant_request`, `ci_window` and `platform_plan` only ask the owner, who
     acts on a page with their passkey;
   - `git_tokens` and `grant_list` are reads that never show a value.

   Meanwhile `ci_cloudflare`, the owner's break-glass, is in neither list.
   - **Kind:** missing permissions. The settings file is guarded, so the
     owner decides.
5. **Approval cards that never close.** `approval_wait` answers with a
   URL-mode elicitation (`-32042`, `worker/mod.rs::wait`). Claude Code shows
   it as "Answer the MCP server's question" and blocks the turn until someone
   answers it there. Nothing ends it from the server side:
   - !500, 03:05:50: the owner approved at 03:09:19. The session logged
     nothing until it was interrupted at 03:44.
   - !473, 02:29:21: the card was for a stale head (1c05c50). It sat for
     about 32 minutes, until a restart ended it.

   Since !492 (the always-listed ADR), every approval the owner owes is
   already listed on `/approvals`, in the app and in the digest, and pushed.
   The card adds no oversight, and costs a blocked session.
   - **Kind:** a tooling bug.
6. **Planners asking "new session, or keep going?":**
   - invites (`…mZ1CqPocouwtrEAjf`), 23:29 on the 14th;
   - profile (`…nVgZ7GHTjGy4QzKkF`), 23:54 and 00:48;
   - browser OAuth (`…m2WtauLWjSQsUdc9F`), at 03:30, 04:00 and 04:30.

   None was at its context limit (435k to 655k of 1M). The reasons given
   were a ticket expiring in about six hours (`session_start` issues a new
   one), another session working on overlapping code (`change_propose`
   answers `overlaps`, and the merge queue restacks), and dependencies on
   other sessions (waiting was the only option). `CLAUDE.md` asks for this
   stop: a session with unfinished work "asks whether a new session should
   finish it".
   - **Kind:** guidance. None of these was the owner's decision.
7. **An executor stopping on capacity.** b19's `change_propose` was refused
   with "20 changes are open (…); merge or close one first". It asked its
   planner whether to wait, and was told "wait for a slot and retry". The
   refusal doesn't say that it is temporary, and nothing tells an executor
   what to do about it.
   - **Kind:** guidance, and the refusal's wording.

### What stays

The owner's oversight that pigeon already enforces is unchanged:

- the owner tier (guarded and `secrets:` paths), which no window or mandate
  covers;
- asks for owner inputs;
- destructive actions (the briefs ADR's 2026-11-14 amendment);
- grants of credentials and spend, which the owner mints with a passkey on a
  page;
- loosening an approval rule, which is the owner's because the rule's files
  are guarded.

This ADR changes who carries an approval the owner has already granted. It
never changes what a window covers.

## Implementation plan.

### 1. When a session stops for the owner

A session stops and waits for the owner only for these:

- a change of its own at `needs: owner`;
- a harness permission prompt that was denied;
- a seed or brief that says "even under a window";
- an owner-only decision: a destructive action, a spend or credential grant,
  or loosening an approval rule.

Even then, it stops only the work that depends on that item. Every other step
it holds that doesn't depend on it goes on.

Anything else is not a stop:

| Situation | What the session does |
| --- | --- |
| `needs: agent` | `next`, now (unchanged). |
| Waits on another change, the platform, a provider or another session | Follows events; says nothing to the owner. |
| A refusal that will clear: the instance's 20 changes, a session's 5, a lost Container, `waiting: GitHub` | Waits for the event that clears it (`change.merged`, `change.closed`, the job's retry) and tries again. Never asks. |
| A choice the rules settle (window or owner, which session acts, retry or not) | Follows the rule. Never offers the owner a choice. |
| A design question inside a brief | An executor uses `brief_escalate` to its planner. A planner decides, writing an ADR amendment when it matters. |
| Ticket expiring | Calls `session_start` again (`replace`) when it expires. |
| Out of context, or the environment can't continue | Ends, saying what is done and what remains, and how to resume. It doesn't ask whether to. |

An owner item being pending is not a stop: the control plane lists and pushes
it (the always-listed ADR), and the session follows events until it lands.

### 2. The control plane carries a window's approvals; agents never call `change_approve` or `change_authorize` under one

This generalizes the briefs ADR's 2026-11-14 amendment (!457) from brief
changes to every change. An agent's call carried the window's approval and
added no check.

- **At proposal.** Once any change is proposed, `worker/mod.rs::propose`
  runs `briefs::by_window(…, "change.authorize", None)`. Today it does so
  only when `brief.is_some()`. The checks hold unchanged: `under_window`'s
  `decide`, `asks_owner`, and the tier.
- **At merge.** The agent's signal that a head is ready is
  `change_merge`, its own act on its own change. That call is never an
  approval.
  - In `worker/mod.rs::ask`, a `change.merge` on a `Checked` head with no
    approval becomes a `change.approve` request of it (`merge_asks_approval`).
    The usual order then runs: mandate, window, policy, the change's grant,
    else pending for the owner, listed and pushed.
  - Every approval, whether automatic or the owner's, already queues the
    merge as its approver (`approve`, without `hold`), so nothing parks.
  - A parked merge (`merge_after`) is not used for this, because it holds
    the merge queue's turn (`queue::holds_turn`) and would block every
    other merge while the owner decides.
  - Nothing else changes. A brief's head awaiting review is still refused
    (`AUTOMATIC`), and approved by the existing `by_window` once its planner
    accepts it.
- **`next` says it.** In `stage.rs`, `ask`'s text names `change_merge` for a
  `checked` change a window or mandate covers: "ask to merge now
  (`change_merge`): window:N, the owner's, approves the head". A `proposed`
  change is authorized at proposal. Only one proposed before its window opened
  still names `change_authorize`. Past proposal, a brief's change never names
  an owner operation in `next`:
  - while awaiting review it is `in-review` (unchanged);
  - otherwise, its `next` is `change_merge`.

  A test drives a brief's change through every state past proposal, accepted
  or not, and asserts that none names `change_approve` or `change_authorize`.
- **The descriptions say who approves** (`wire.rs`):
  - `change.approve`: "The owner's or a reviewer's approval of a checked
    head … A session never calls it on its own change: it asks to merge
    (`change_merge`), and the owner's window, mandate or policy approves in
    the control plane."
  - `change.authorize`: "The owner's or a reviewer's … under a window, the
    control plane authorizes at proposal; a session doesn't call it."
  - `change.merge`: "Ask to merge your change: an approved head merges; a
    checked head is approved first by the window, mandate or policy that
    covers it, else waits, listed, for the owner."
  - `brief.review`: "`accept` … under a window, the control plane approves
    the accepted head at once; don't call `change_approve`."

### 3. `approval_wait` answers with the link, never a card

`worker/mod.rs::wait` returns `{pending: {operation, number, link}}` as an
ordinary result. It never sends the URL-mode elicitation.
`mcp::elicit_url` and its error code go, with their test. `ELICIT` stays,
so answers to old cards are still accepted and dropped. The session then
follows events, and the approval landing moves the stage.

`approval_wait` keeps its use: getting the owner-only link for a change the
session wants the owner to approve personally, as !500 did.

### 4. Capacity is a wait

- `sessions.rs`'s refusals read: "wait: 20 changes are open (…); one merging
  or closing wakes you (`change.merged`, `change.closed`), then propose
  again". A session's limit of 5 reads the same way.
- The executor prompt (`core/session/launch.rs`) adds to step 6: "A refusal
  that says `wait:` isn't an escalation: follow events and retry".

### 5. Owner-stops counted in the digest

- `Digest` gains `stops`. It is computed from the events since the last
  digest, is shown, and is never pushed. One line:
  `stops since last digest: owner <n> (authorize a, approve b, merge c, grant d, deploy e); agent nudged <m>; escalated <k>`.
  - The owner counts are `approval.pending` events by operation,
    `grant.requested`, and `deploy.failed`.
  - `nudged` counts `change.nudged`: each one is a change that needed its
    session for 15 minutes without the session acting, which is the false
    stop this ADR removes.
  - `escalated` counts `brief.escalated`.
- When the window covers the head, a rising `approve` count means a session
  is calling it again. A rising `nudged` count means sessions are stopping
  where they shouldn't.

### 6. `CLAUDE.md` and the settings

- **`CLAUDE.md`'s Rules** carry §1 in three lines, replacing "A session stops
  only for `needs: owner` …".
- **The concurrent-sessions paragraph** replaces "and asks whether a new
  session should finish it" with "and how to resume it; it doesn't ask
  whether to".
- **`.claude/settings.json`**, guarded, so the owner approves it:
  - into `allow`: `grant_request`, `ci_window`, `platform_plan`,
    `infra_plan`, `grant_list`, `git_tokens`, `apple_check`;
  - into `ask`: `ci_cloudflare`;
  - unchanged in `ask`: `change_close`, `change_pin`, `grant_revoke`,
    `grant_rotate`, `platform_apply`, `infra_apply`, `deploy_retry`,
    `sites_publish`, `ios_release`.

### Execution

1. This ADR and `CLAUDE.md` (§6's first two points), as one `docs(adr)` change.
   It changes when sessions stop for the owner, so the owner approves it, not
   a window.
2. Step 2 is two sonnet briefs, each a `feat(backplane)` change. A brief
   holds at most 16 KiB, which one patch for both didn't fit.
   - **2a (b27), for §2:** `worker/mod.rs` (proposal and merge), `stage.rs`
     and `wire.rs`, with their tests. It also adds to `CLAUDE.md`'s Rules
     the line "Under a window a session never calls `change_authorize` or
     `change_approve`: it asks to merge (`change_merge`), and the control
     plane approves", so the rule lands with the behaviour.
   - **2b (b28), for §3:** `worker/mod.rs::wait`, `worker/api.rs`, `mcp.rs`
     and `tests/mcp.rs`.
3. Brief b26, sonnet, one `feat(backplane)` change, for §4 and §5:
   `sessions.rs`, `core/session/launch.rs`, `tasks.rs`, `worker/push.rs` and
   `tests/digest.rs`.
4. One `chore(claude)` change with §6's settings, which the owner makes. When
   this ADR's planner tried to write the edit, its harness refused it as
   undermining oversight, and that refusal stands: an agent doesn't widen its
   own permissions, even through a change the owner would approve.

### Verification

- An executor whose brief's change sits under a window carries it from
  proposal to merge calling neither `change_authorize` nor `change_approve`,
  and the classifier refuses nothing.
- A planner's own change under a window merges after `change_merge` alone.
  Without a window, it waits, listed, and merges when the owner approves.
- `approval_wait` returns at once, and no session shows "Answer the MCP
  server's question".
- The digest's `stops` line shows `nudged` falling over the following nights.

## Amendments

### 2026-11-15: Reviewed against !503: they agree, and a renewed ticket passes `parent` again

!503 carries out
[`2026-11-15-development-archive-cleanly-ended-sessions.md`](2026-11-15-development-archive-cleanly-ended-sessions.md)
step 2. Each of its paths that this ADR governs was checked against this ADR.

- **§1, ending with work left.** A session that ends with work left still
  holds its changes, so `session_end` judges the end unclean, and it is never
  archived. A clean end leaves nothing but hand-offs. Neither ADR asks the
  owner anything at a session's end.
- **§1, owner-only decisions.** Archiving a child that ended clean is not an
  owner-only destructive action. `unarchive_session` reverses it. The owner
  asked for it, and the archive ADR limits it to the archiver's own children,
  never an `ios` or `desktop_app` session. The archiver acts on
  `session.archivable` without stopping. A child still working is looked at
  once more, then skipped and reported. That wait isn't a stop either.
- **§1, ticket renewal.** Renewal is `session_start` with `replace`. !503 keeps
  `parent` on the ticket record, so a new ticket starts without it. A session
  created by another passes `parent` on every `session_start`, renewals
  included, as `CLAUDE.md` says. An executor still gets its archiver from its
  `planner`.
- **§2.** `session_archived` is a report, not an approval. No window covers it,
  and the archiver's own ticket guards it. Nothing in !503 calls
  `change_approve` or `change_authorize`.
- **§4, `sessions.rs`.** !503 adds only `session_id` there. The `wait:`
  refusals are unchanged.
- **§5, `tasks.rs` and `worker/push.rs`.** !503's `archives` lines and §5's
  `stops` line are separate `Digest` fields. Both are shown and never pushed,
  and neither changes `needs_owner`.
- **`wire.rs` and `worker/mod.rs`.** !503 adds `session.archived` and a `parent`
  argument on `session.start`, and changes none of §2's descriptions.

!503 needs no change.
### 2026-11-15: Review of !494 against this ADR: they agree, and a hold is not a stop

!494 carries out the hold in
[`2026-11-12-runtime-ios-app-approvals-insights.md`](2026-11-12-runtime-ios-app-approvals-insights.md)'s
2026-11-15 amendment and
[`2026-11-13-development-session-stages-subscriptions-breaker.md`](2026-11-13-development-session-stages-subscriptions-breaker.md)'s:
a failed deploy holds a window or mandate, and only `fix` and `revert` merges
go through until a deploy passes.

- **They agree.** A `held` change has `needs: platform`. Under §1 that is a
  wait, not a stop: the session follows events, says nothing to the owner, and
  asks to merge again when `auto.resumed` wakes it. A hold lists nothing for
  the owner, and pushes only the hold itself.
- **`stage.rs`:** §2 changes `ask`'s text for a `checked` change. !494 adds
  `held` for an `approved` one. They touch different states, so neither changes
  the other.
- **`tasks.rs` and `worker/push.rs`:** §5's `stops` line and !494's `held`
  lines are separate digest fields. A held change is never nudged, so a hold
  doesn't raise `nudged`.
- **One interaction needs a fix.** An approval by a window or mandate queues
  its merge as its approver, without asking about `change.merge`. §2 makes
  every `change_merge` on a checked head an approval, so under a hold a
  non-fix merge would go through that path. !521 fixes it: during a hold, a
  window's or mandate's approval is made with `hold`, so it doesn't queue the
  merge, and the change is `held` until a deploy passes.

### 2026-11-15: A change is stalled only once its session has been quiet since the change last moved

- **What went wrong.** The digest named !542, !543 and !544 `stalled`, with
  "start a session to take it over, or close it", about five minutes after
  their plans were authorized. Their sessions were writing the code at the
  time. `tasks::stalled` measured quiet only from the session's last poll:
  - a session that had never polled was stalled at once;
  - !544 showed "quiet 8h15m" because its session had slept until a
    scheduled task, then proposed it.
- **The rule.** A session's quiet starts at whichever is later: its last poll
  or its change's last move, the stage's `since`. A change is stalled once
  that quiet reaches `STALL` (30 minutes). A just-authorized change therefore
  has `STALL` to see a push before it's named, and an old poll never makes a
  fresh change look abandoned.
- **Where.** `tasks::stalled` takes `since`. The digest
  (`worker/push.rs::sessions`) and the app's tracker
  (`core/app/tracker.rs`) both pass the change's `stage.since`. The tests
  replay tonight's case in `tests/digest.rs` and the tracker's tests.
- **Execution.** One `fix(backplane)` change, with this amendment.

### 2026-11-15: An owner-only decision reaches the owner as an owner item; Auto is standing

[`2026-11-15-security-standing-auto-owner-flags.md`](2026-11-15-security-standing-auto-owner-flags.md)
builds on §1 without changing it:

- **§1's "owner-only decision"** is filed with `owner_ask` (`kind: decision`
  or `action`), which pushes, enters the digest and shows in the app.
  `owner.answered` wakes the session. Until then it stops only the work that
  depends on the item. A choice the rules settle is still never filed.
- **§2's "under a window"** now means while Auto is on, which it is until the
  owner pauses it. Agents still never call `change_authorize` or
  `change_approve` under it.
- **Step 4's settings edit** is filed as an owner item (`kind: action`).
### 2026-11-15: The control plane's own changes are authorized under the window too, and no stage asks a session to authorize

- **What went wrong.** !559 ("review !549 against !555") and !550 earlier, review
  changes the control plane proposes (proposer `pigeon`), stayed `proposed`
  under window 9. Their `next` read "ask now (`change_authorize`)", which §2
  and `CLAUDE.md` forbid under a window, so the claiming session held a
  committed review it couldn't push. `worker/merges.rs` proposes generated
  changes through the inner `propose`, which skips §2's authorization at
  proposal.
- **The rule.**
  - A generated change (`review-…`, `reconcile-…`) is authorized by the
    owner's window at proposal, as a session's is (`briefs::by_window`).
  - The hourly run (`events::nudge`) asks the window once more for every open
    change still `proposed` with no plan approval (`events::authorize_due`),
    whoever proposed it. That covers a change proposed before its window
    opened, and any already stuck, such as !559.
  - A covered `proposed` change's stage is `needs: platform`, `next`
    "window:N, the owner's, authorizes it in the control plane within the
    hour". This replaces §2's one exception, so no stage ever names
    `change_authorize` to a session.
- **Execution.** One `fix(backplane)` change, with this amendment and a test
  that replays !559.
