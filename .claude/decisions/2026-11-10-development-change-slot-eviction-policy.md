## Change slots are freed: unauthorized closes, one reconciliation per change, and eviction by policy

* Date: 2026-11-10
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development
* Governs: `src/pigeon-cli/crates/backplane/src/changes.rs`, `src/pigeon-cli/crates/backplane/src/reconcile.rs`, `src/pigeon-cli/crates/backplane/src/sessions.rs`, `src/pigeon-cli/crates/backplane/src/tasks.rs`, `src/pigeon-cli/crates/backplane/src/worker/change.rs`, `src/pigeon-cli/crates/sentinel/src/target.rs`

## Context

[`2026-11-08-development-concurrent-agent-sessions.md`](2026-11-08-development-concurrent-agent-sessions.md) caps open changes at 3 per session and 12 per instance (§6). When a restack conflicts, the control plane proposes a `reconcile-<n>` change for the conflicted change's session; a semantic overlap proposes a `review-<n>-<m>` (§4).

On 2026-11-09 those rules jammed the instance:

- **Change 79 conflicted with `main` six times** as other sessions merged. Each time a new `reconcile-79` was proposed: !85, !86, !104, !106, !108 and !113. Four `review` changes were proposed beside them: !81, !101, !102 and !111. All ten went to change 79's session, and none was ever pushed.
- **None of the ten could be closed.** D1's `changes` table holds `CHECK ((state = 'proposed') = (plan_approver IS NULL))` (`migrations/0002_changes.sql`). A change closed before its plan was authorized has no plan approver, so D1 refused the row and the close failed.
- **The session hit its limit of 3 open changes** with ten it never asked for. It couldn't propose the fix for the close.
- **The instance hit 12.** Every other session was blocked as well, until the owner closed change 79 by hand.

What the existing rules already give:

- [`2026-11-09-security-approvals-once-passkeys.md`](2026-11-09-security-approvals-once-passkeys.md) §2 makes `change.close` of a change that never pushed **auto**, because nothing is lost. Closing a change with pushed commits is **owner**, because it is destructive.
- [`2026-11-08-development-events-tickets-digest-mandate.md`](2026-11-08-development-events-tickets-digest-mandate.md) §5 gives each session a 24-hour ticket. An expired ticket is the control plane's only signal that a session has gone.
- [`2026-11-02-development-sentinel-policy-cobot-actions.md`](2026-11-02-development-sentinel-policy-cobot-actions.md): sentinel judges, and cobot acts. backplane composes the two and acts on neither alone.

The owner's direction:

- slots are released when a session expires, or when a change is outdated;
- the release goes through sentinel, and where the policy allows it, that is the grant;
- a session never evicts;
- work left incomplete is flagged, with what remains, and the owner is asked whether a new session should finish it.

## Implementation plan.

### 1. A change closed before authorization closes

- **`control::mirror` writes `(closed unauthorized)` as the plan approver** of a closed change that has no plan approval. D1's check then holds. `ChangeRun`'s state is unchanged: it still has no plan approval.
- **No migration.** Changing a SQLite `CHECK` means rebuilding the table, which `.config/policies/migrations.yaml` refuses, because it drops a table.
- **A test** applies every migration to SQLite and mirrors a change that is proposed, then closed. Without the fix it fails with the production error.

### 2. One reconciliation per change, and a family holds one slot

- **At most one open `reconcile-<n>` per conflicted change.** A later conflict updates that change's generated plan instead of proposing another. The update covers the new `onto`, the conflicting paths with their hunks, and the merged changes it now reconciles with. An update is an event on the change.
- **At most one open `review-<n>-<m>` per pair.** A second overlap between the same pair updates it.
- **A generated change closes itself when its reason is gone.** Since it never pushed, that close is auto (approvals ADR §2). Its reason is gone when:
  - its parent was restacked or rebased cleanly onto `main`, so the conflict is resolved;
  - its parent is merged or closed;
  - for a review: either side is merged or closed, or the later ADR already carries an amendment citing the other.
- **A family holds one slot.** A change, its open reconciliation and its open reviews count as one toward `max_open_per_session` and `max_open`. They count as two once a generated change is claimed and pushed: it is then real work.
- **The limits name the family.** A refusal lists open changes grouped by family, with each one's age and its session's ticket state.

### 3. Eviction: who, when and why

**Eviction** releases a change's slot without losing its work.

**What it does:**

- the change moves to `closed`, with `evicted: {rule, at, remains}` recorded on it (§5);
- its jobs are cancelled;
- its open approvals expire;
- its branch's head is kept as `refs/evicted/<n>-<slug>`;
- it is listed in the digest.

**What it never does:**

- delete a commit;
- touch `main`;
- act on a merged change.

**Who evicts:** only the control plane, from the hourly cron the digest and the mirror already use. A session never evicts, not even its own changes:

- no MCP tool evicts;
- `change_close` stays the session's way to give up a change, at its tier.

**When a change is a candidate:**

- **Its session has gone.** The change's session has had no live ticket for `session_grace` (default 2 hours), and the change has had no event in that time. A change proposed without a session counts as gone after the same idle time.
- **Or it is outdated,** and its session holds no live ticket:
  - never authorized, and idle for `stale_proposed` (default 24 hours);
  - `behind` or `conflicted`, with its reconciliation task unclaimed, and idle for `stale_conflicted` (default 24 hours);
  - pushed, and idle for `stale_pushed` (default 72 hours), whatever its state short of `approved`.

**Never a candidate:**

- **a change whose session holds a live ticket.** In normal work nothing is evicted, and a session renewing its ticket keeps its changes;
- a change `approved`, or in the merge queue;
- a change with an owner approval page still open;
- a change the owner pinned: `change_pin`, at the owner tier, recorded on the change.

`§2`'s self-closing generated changes are not evictions. They are ordinary auto closes.

### 4. Sentinel judges; that verdict is the grant

- **A new sentinel target, `eviction`.** Its fields come from the change's record and the session's ticket:
  - `number`, `kind` (`change`, `reconcile` or `review`), `state`;
  - `pushed`;
  - `session_live`, and `session_gone_for` (seconds);
  - `idle` (seconds since the change's last event);
  - `pinned`, `approval_open` and `queued`.
- **Field rules gain numeric checks:** `at_least` and `at_most` on a number field, and `equals` on a string or boolean field.
- **A candidate is evictable when one eviction policy has no violations.** That policy's name is the grant, recorded as `policy:<file>` on the change and in the audit. The policies are separate files, one per reason:

  ```yaml
  # .config/policies/eviction-session-gone.yaml
  description: A change whose session has had no live ticket for two hours, idle as long, is evicted; see .claude/decisions/2026-11-10-development-change-slot-eviction-policy.md §3.
  target: eviction
  rules:
    - field: { name: session_live, equals: false }
    - field: { name: session_gone_for, at_least: 7200 }
    - field: { name: idle, at_least: 7200 }
    - field: { name: pinned, equals: false }
    - field: { name: approval_open, equals: false }
    - field: { name: queued, equals: false }
    - field: { name: state, not: '^(approved|merged|closed)$' }
  ```

  The others are `eviction-stale-proposed.yaml`, `eviction-stale-conflicted.yaml` and `eviction-stale-pushed.yaml`, with §3's defaults.
- **The policies are the owner's.** They sit under `.config/policies/**`, which `.config/approvals.yaml` guards, so changing a rule takes the owner's proof. With no eviction policy present, nothing is evicted.
- **Amends the approvals ADR §2.** Closing a pushed change stays `owner`. Eviction is a separate operation: it deletes nothing and keeps the head (§3). An eviction that a policy on `main` allows is approved by that policy. No other path evicts, and no tier, mandate or change grant reaches it.

### 5. Acting: cobot keeps the work, backplane closes the slot

**For each evictable change, the hourly job:**

1. Keeps the work. A pushed change's head is written to `refs/evicted/<n>-<slug>`, by a `Runner` job through `cobot::git`. The ref is the control plane's alone, like `main`: the front door refuses pushes to `refs/evicted/*`.
2. Builds the change's `remains` (§6).
3. Applies `Event::Evict { policy, remains }` to `ChangeRun`. That closes the change, cancels its jobs, expires its approvals and mirrors it to D1 (§1 covers a never-authorized one).
4. Appends an event, `change.evicted`, which the session's `events_wait` sees if it ever returns.

**The pieces:**

- **A dry run first.** `pigeon backplane run change.evictable` (MCP `change_evictable`, read-only) lists each candidate, the policy that would grant its eviction, and its `remains`. The hourly job runs the same evaluation, then acts.
- **At most 6 evictions per hour,** oldest first, so a wrong policy can't empty the instance in one run. The digest names any left over.

### 6. Incomplete work is flagged, with what remains

**A change was incomplete** when it was evicted after it was authorized, pushed, or had a claimed task. Its `remains` lists:

- **what was done:**
  - the plan;
  - the head;
  - each job's last result;
  - the last violations;
- **what is left,** from its state:
  - authorized but nothing pushed: the whole plan;
  - pushed and failing: the failing jobs and their log tails;
  - `conflicted` or `behind`: the paths that conflict, and the reconciliation's plan;
  - `checked`: only the owner's approval;
- **how to resume:** `change_propose` with `resumes: <n>` starts a new change from `refs/evicted/<n>-<slug>`, with the evicted change's plan and scope, and its own number.

**The owner is asked.** An incomplete eviction creates a task of kind `resume`, and a push to the owner (one per hour at most, through the digest):

> !79 `feat(backplane): the control plane mirrors to GitHub on every push to main` was evicted (`eviction-session-gone`). Done: … Left: … Should a new session finish it?

- **The page has two answers:**
  - **Yes:** the task is marked `wanted`. The page shows a ready-to-paste prompt for a new session: the ADR, the remains, the `resumes` call, and the task to claim.
  - **No:** the task closes. `refs/evicted/<n>-<slug>` stays for 30 days, then the hourly job deletes it, which is listed in the digest the week before.
- **Starting the session stays the owner's.** The concurrent-sessions ADR §5 leaves starting sessions to a later ADR, and so does this one. A new session finds the `wanted` task with `task_list` and claims it.
- **A session does the same when it stops.** An agent that stops with incomplete work, whether it ends its ticket, runs out of time, or is asked to stop, says in its last message what is done and what remains, and asks whether a new session should finish it. `CLAUDE.md` gains that rule. `session_end` returns the `remains` of each change the session still holds, so the agent has it to hand.

### 7. Execution

0. **Unjamming, once.** This ADR's own change can't be proposed while the session cap holds. The owner either closes the stuck changes by hand, or allows one change proposed without a session. The first change through is step 2's fix, which lets the ten never-pushed changes close at the auto tier.
1. **This ADR.**
2. **`fix(backplane)`:** §1. It is ready as a commit, with its test.
3. **`feat(backplane)`:** §2's one reconciliation per change, self-closing generated changes, and family slot counting.
4. **`feat(sentinel)`:** the `eviction` target and the numeric field rules, with policy tests. Sentinel runs from source in checks, so step 5 needn't wait for a release.
5. **`feat(backplane)`:** §3 to §6:
   - candidate evaluation through sentinel;
   - `change_evictable`;
   - `Event::Evict`, and the `evicted` record with its state fixture;
   - `refs/evicted/*` through a `Runner` job, and the front door's refusal of it;
   - `remains`, the `resume` task, the owner's question, and `resumes` on `change_propose`;
   - `change_pin`;
   - the hourly job, and the limit of 6 an hour.
6. **`feat(policies)`:** the four eviction policy files. They are guarded, so they take the owner's proof. Nothing is evicted until they merge, and `change_evictable` can be read beforehand.
7. **`docs(claude)`:** `CLAUDE.md`'s pointer, and the rule for stopping with incomplete work.

### 8. Verification

- **Closing.** A change proposed and closed before authorization reaches `closed` in D1 (step 2's test, and the ten changes of 2026-11-09).
- **One reconciliation per change:**
  - six conflicts of one change produce one `reconcile-<n>`, updated six times;
  - it closes itself when its parent rebases cleanly;
  - a family counts as one slot until its reconciliation pushes.
- **Sentinel.** The `eviction` target and the numeric rules are tested in sentinel. Each eviction policy grants exactly the candidates §3 names, and refuses a live session's change, a pinned one, an approved one, and one with an open approval page.
- **Eviction:**
  - an evicted pushed change's head equals `refs/evicted/<n>-<slug>`;
  - a push to `refs/evicted/*` through the front door is refused;
  - `resumes` starts a new change from that ref;
  - no session-held token or MCP tool evicts;
  - the seventh candidate in an hour waits for the next run.
- **Incomplete work.** An evicted incomplete change creates a `resume` task and one owner question. "Yes" gives a prompt that a fresh session follows to finish the change.
