## Concurrent agent sessions: a merge queue, restacking, and reconciliation planned on conflict

* Date: 2026-11-08
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development
* Governs: `src/pigeon-cli/crates/backplane/src/sessions.rs`, `src/pigeon-cli/crates/backplane/src/merges.rs`, `src/pigeon-cli/crates/backplane/src/reconcile.rs`, `src/pigeon-cli/crates/backplane/src/tasks.rs`, `src/pigeon-cli/crates/backplane/src/worker/merges.rs`, `src/pigeon-cli/crates/backplane/migrations/0003_change_sessions.sql`

## Context

The owner wants several agent sessions at once, each with its own task. Some
design ADRs, some build, and some implement them. When two sessions' work
overlaps at merge, the control plane should plan the reconciliation itself:
an ADR amendment, or a change that fixes the code. It should not leave the
second session stuck.

Today the process works for one session at a time:

- **Every session is the same identity.** Each session uses the
  `pigeon-agent` Access service token, so a change's `proposer` doesn't
  say which session proposed it.
  - The rule "never approved by its proposer" still holds, because the owner
    or a mandate approves, never the agent.
- **Each change is its own `ChangeRun`,** numbered per instance, with a
  scope of globs and its own `change/<n>-<slug>` branch. Per
  [`2026-11-06-security-git-front-door-spike.md`](2026-11-06-security-git-front-door-spike.md),
  pushes go only to an authorized change's branch. Two sessions can't push
  over each other's work.
- **A merge squashes the approved head onto `main`** (`cobot::ci::squash_onto`),
  and only when `main` is an ancestor of the head.
  - The second of two concurrent changes is refused at merge until its
    session rebases.
  - The rebase makes a new head, which needs new checks and a new approval
    at its tier.
  - Nothing tells the session that its change is behind `main`.
- **Two merges can race.** Each `ChangeRun` merges for itself, so two
  merges can start against the same `main`. The ancestry check makes the
  loser fail. The control plane doesn't order merges.
- **ADRs are append-only** per
  [`2026-11-02-development-adr-driven-lifecycle.md`](2026-11-02-development-adr-driven-lifecycle.md).
  - Two amendments to one ADR both append at its end, which is a textual
    conflict with an obvious resolution.
  - Two new ADRs that decide the same area don't conflict in text, but do
    conflict in meaning. Git can't see that.
- **Mandates**
  ([`2026-11-08-security-tiered-approvals-task-mandates.md`](2026-11-08-security-tiered-approvals-task-mandates.md))
  let a session finish a task without the owner. A conflict with another
  session's work is exactly the "something unexpected" the owner wants to
  hear about, but only once nothing can resolve it automatically.
- **Capacity is finite:**
  - `Checker` runs at most 12 Containers and `Runner` 8;
  - each pushed head starts about five checks;
  - `MainRun` already folds a burst of merges into one deploy, of the
    newest.

## Implementation plan.

### 1. Sessions are named, not trusted

- **`change.propose` and `mandate.propose` take `session`:** a short label,
  the session's URL or name.
  - It is stored on the change or mandate.
  - `change_list` filters by it.
  - It routes tasks (§5). It grants no authority: any session holding the
    agent token may still act on any change, as today.
- **Each session works only on its own changes.** The agent's instructions
  (CLAUDE.md) say so. The control plane records which session pushed each
  head, for the digest.

### 2. Overlap is announced at propose, never blocked

- **`change.propose` answers `overlaps`:** the open changes whose scopes
  overlap the new scope, by the approval tiers' overlap rule, each with its session
  and state.
- **A design change declares what it will decide.** `change.propose` takes
  an optional `affects`: the globs the ADR will later govern. It is stored
  on the change and compared like a scope. So an ADR in design reserves its
  area before any code touches it.
- **Overlap doesn't refuse the change.** Two sessions may knowingly work
  side by side. Blocking would deadlock sessions that each wait on the
  other.
  - The exception is a new file both changes would create. Two changes
    that both create the same ADR filename can never both merge, so the
    second one is refused, with the first one's number.
- **The overlap is shown** in `change_status` and in the owner's digest.

### 3. One merge queue per repository

- **A `MergeQueue` Durable Object per repository** takes every merge.
  `change.merge` enqueues the change and answers its place in the queue.
  The queue merges one change at a time, which ends the race.
- **After each merge, the queue restacks:**
  - It looks at every open change that has a head and is `checked` or
    `approved`.
  - Earlier states are only marked `behind`. Their sessions rebase on their
    next push, which costs no checks now.
- **A restack sorts each change by what changed on `main` since its base:**
  - **current:** `main` is still its base;
  - **disjoint:** none of its files changed;
  - **clean:** some of its files changed, but git merges them without
    conflict;
  - **conflicting:** git can't merge them.
- **Disjoint and clean changes are rebased by the control plane.**
  - A `Runner` job replays the change onto the new `main` and pushes the
    result to the change's branch as a new head.
  - The push is the control plane's own, recorded as `restack`. The
    session sees it on its next fetch.
  - Checks run on the new head as on any push.
- **An approval carries over only when the change itself is unchanged:**
  - the change is disjoint;
  - its `git patch-id --stable` is the same before and after;
  - and its merge tier is the same.
  
  Otherwise the new head asks again at its tier, which a mandate that
  covers it answers automatically. A carried-over approval is recorded
  with both SHAs.
- **ADR amendments merge by union.** `.gitattributes` gives
  `.claude/decisions/*.md` git's `union` merge driver, so two appends to one
  ADR both land.
  - The `decisions` policy then checks that the amendment headings are in
    date order. Both entries carry the same date, so the merged ADR passes.
  - If it fails, the change is conflicting instead.

### 4. A conflict plans its own reconciliation

- **A conflicting change becomes `conflicted`,** a new state. It can't be
  approved or merged, and it keeps its branch.
- **The control plane proposes a reconciliation change for it,**
  `reconcile-<n>`, with the conflicted change as its parent. Its generated
  plan holds:
  - the two changes, the one merged and the one conflicted, with their
    titles, plans and sessions;
  - the ADRs each one's plan cites;
  - each conflicting path, with both sides' hunks;
  - the rule it must follow:
    - **When either change cites an ADR, or a conflicting path is under
      `.claude/decisions/`,** the reconciliation must append an amendment
      to the later ADR. The amendment says what was reconciled and which
      side's decision stands.
    - **Otherwise** it fixes the code.
- **Its scope is the union of both changes' conflicting paths and the later
  ADR.** It is authorized like any change:
  - automatically, under a mandate that covers the union and cites either
    ADR;
  - otherwise at its tier.
- **A semantic overlap gets the same plan.** When a change that declared
  `affects` merges, every open change whose scope or `affects` overlaps it
  is told about it, even with no textual conflict.
  - If the open change cites a different ADR, it gets a `review`
    reconciliation.
  - That reconciliation's plan is an amendment to the later ADR, citing the
    earlier one. It says whether the two agree, and if not, what changes.
- **Reconciliation never edits the merged side.** `main` is settled.
  The conflicted change is rebased onto it or superseded.
  - It is superseded when its reconciliation lands its intent. Then it is
    closed, as a spike is.

### 5. Tasks: how a session learns it has work

- **The control plane keeps a task list per instance.** A task is one of:
  - a reconciliation to plan or carry out;
  - a restack that failed;
  - a check that failed twice on one head;
  - an overlap notice.
- **MCP tools:**
  - `task_list`;
  - `task_claim`: one session takes a task, so two don't race;
  - `task_wait`: a long poll, up to 50 seconds, for new tasks routed to
    this session, or for unclaimed ones.
- **Routing:**
  - a task goes first to the session that owns the conflicted change;
  - after 30 minutes unclaimed, any session may take it;
  - after 2 hours, it is in the owner's digest.
- **The owner is told only when nothing automatic can resolve it.**
  - A conflict whose reconciliation was planned, authorized, checked and
    merged under a mandate is only a line in the digest.
  - A reconciliation that falls outside every active mandate is a
    pending approval, with its plan.
- **Starting sessions is out of scope.** Sessions run where the owner
  starts them. A later ADR may let the control plane start one for an
  unclaimed task.

### 6. Limits

- **`.config/sessions.yaml` on `main`, guarded,** holds:
  - `max_open_per_session`, default 3;
  - `max_open`, default 12;
  - `restack_checks`, default `scoped`. A restacked head runs only
    `cobot::scope`'s checks for the paths that moved.
- **Over a limit, `change.propose` is refused,** with the open changes
  named.
- **Checks queue for Containers, as today.** The digest shows each
  change's wait.

### Execution

1. `feat(backplane)`: `session` and `affects` on proposals, `overlaps` in
   the answer, and the same-new-file refusal (§1, §2).
2. `feat(backplane)`: `MergeQueue`, which merges in order and marks open
   changes `behind` (§3, without restacking).
3. `feat(backplane)`: restacking for disjoint and clean changes, carried
   approvals by patch-id, and the union driver for ADRs (§3).
4. `feat(backplane)`: the `conflicted` state, reconciliation changes and
   their generated plans, and `review` reconciliations (§4).
5. `feat(backplane)`: tasks, `task_list`, `task_claim` and `task_wait`,
   routing and the digest lines (§5).
6. `feat(backplane)`: `.config/sessions.yaml`, guarded, and its limits (§6).

Steps 1, 2 and 6 are guarded or deploy the Worker, so they're merged with the
code. Steps 3 to 5 may run under one mandate scoped to backplane's change,
merge and task modules.

### Verification

- **Two sessions propose overlapping scopes.** Each sees the other in
  `overlaps`.
- **Two changes are approved at once and both merged.** The queue merges
  one, restacks the other, carries its approval when disjoint, and merges
  it with no prompt.
- **Two changes edit the same lines.** The second becomes `conflicted`,
  and `reconcile-<n>` is proposed with both hunks and the ADR rule. A
  session claims it with `task_claim` and lands it.
- **Two amendments to one ADR** both merge, through the union driver.
- **An ADR declared with `affects` merges.** An open change in that area
  gets a `review` reconciliation.

## Amendments

### 2026-11-08: Step 1 lands: sessions, `affects` and overlaps

- **`change.propose` takes `session` and `affects`,** kept on the change
  and in its D1 row. Migration `0003_change_sessions.sql` adds two nullable
  columns, so an older Worker reads past them.
- **The answer's `overlaps`** lists each open change whose scope or
  `affects` meets the proposal's: its number, state, session and globs.
- **The same new file is flagged, not refused.** At propose, the Worker
  can't tell creating a file from amending one, since that would mean
  reading `main`'s tree.
  - An overlap lists in `same` the exact paths both changes name.
  - If both changes do create the same file, the merge queue sees it as a
    conflict (add/add) and reconciles it, as §4 does any conflict.
- **Sessions name themselves:** the label is one line of at most 200
  characters, and grants nothing.

### 2026-11-08: Step 2 lands: the merge queue, in the `Instance` object

- **The queue lives in the `Instance` Durable Object, not a new
  `MergeQueue` class.** A new class is a Durable Object migration, which
  can't be rolled back past
  ([`2026-11-08-runtime-blue-green-self-standup.md`](2026-11-08-runtime-blue-green-self-standup.md)).
  The instance has one repository, so one queue per instance is one per
  repository.
- **`change.merge` asks the queue for a turn.**
  - If nothing merges, the merge starts as before.
  - Otherwise the answer is `queued` with the change's place in line. When
    a merge ends, merged or failed, the change's `ChangeRun` starts the
    next one.
  - A queued change whose merge refuses to start, for example because a
    new push took its approval, leaves the queue, and the next one starts.
  - A merge running longer than 20 minutes is moved past.
- **`behind` is a field, not a state.**
  - After a merge, each other open change with a head gets `behind`: the
    commit `main` now holds. Its next push clears it.
  - A new state would be a new stored enum variant, which needs two
    releases.
  - The merge job still refuses a head whose base `main` has moved past.
    Restacking (step 3) will act on `behind`.

### 2026-11-08: Step 3 lands: restacking, carried approvals and ADR unions

- **A merge marks every other open change `behind` before the next queued
  merge starts.** Each `checked` or `approved` head whose merge isn't
  running gets a `restack` job, on the `Runner` with a write token, as a
  merge does.
- **The restack job (`cobot::ci::restack`):**
  - checks out `main` and merges the head into it with `git merge-tree`;
  - writes one commit on `main` that carries the message of the change's
    first commit, so its `Change-Scope` trailers stay first;
  - pushes it to the change's branch with `--force-with-lease`, so a newer
    push by the session wins.
- **The report gives the relation:**
  - current: nothing happens;
  - disjoint or clean: the new head is pushed and checked like any push;
  - conflicting: the change keeps its head and gets a violation naming the
    conflicting paths, until step 4's reconciliation.
- **An approval carries over** when three things hold:
  - the head was approved before the restack;
  - the restack was disjoint, with the same `git patch-id --stable`;
  - the new head's merge tier is no higher than the approved one.

  The approval is kept on the change as `carried`, and it is applied when
  the new head reaches `checked`. Artifacts' push event may arrive before
  the job's report, so the approval is applied only once the report names
  the new head, whichever arrives first. The approver and time stay the
  original ones.
- **ADRs merge by union, set by the job, not the repository.**
  - The layout policy allows no root `.gitattributes`, so the job writes
    `.claude/decisions/*.md merge=union` into its clone's
    `.git/info/attributes`.
  - Two amendments appended to one ADR then merge without conflict.
  - Union drops the blank line between the two blocks; the bound job's
    policies still check the result.
- **Not yet:**
  - a restacked change that was waiting in the merge queue leaves it, since
    its merge refuses to start until it is approved again, so it is asked
    to merge again;
  - a failed restack job leaves the change as it was, still `behind`.

### 2026-11-08: A queued merge waits through its restack

- **A merge asked for while the change is restacking no longer loses its
  turn.** That covers an approved head whose restack job runs, and a head
  waiting for a carried approval.
  - The change records who asked (`merge_after`) and answers without
    starting the merge job, so the queue keeps the turn.
  - When the restack reports `current`, or the carried approval lands, the
    merge starts.
- **A merge that can no longer happen gives its turn back:**
  - a restack conflict;
  - an approval that didn't carry;
  - a new push that took the head.

  `ChangeRun` sees `merge_after` cleared with no merge running and ends
  the change's turn, so the next merge starts.
- **The turn is held while the restacked head is checked,** a few
  minutes. The queue still moves past a turn held longer than 20 minutes.

### 2026-11-08: Step 4 lands: reconciliation planned on conflict and on review

- **`conflicted` is a field, not a state,** for the same reason `behind` is
  one: a new state would need two releases.
  - `Change.conflict` holds `main`'s commit, the paths and the
    reconciliation change.
  - While it is set, the change is refused approval and merge. Its next
    push clears it.
- **The restack job brings the conflict's context:**
  - for each path: the subjects of `main`'s commits that touched it, and
    both sides' diffs, each cut to 4 KiB;
  - the messages of those commits, which name the changes merged and the
    ADRs their plans cited.
- **`backplane::reconcile` plans the reconciliation** (`reconcile-<n>`,
  `fix: reconcile !<n> with main`), proposed by `pigeon` for the conflicted
  change's session, with `parent` naming that change. The plan:
  - cites this ADR;
  - lists the conflicting paths and the merged changes behind them;
  - states the rule: with any ADR cited on either side, append an
    amendment to the later one and make the paths agree with it; otherwise
    fix the paths, `main`'s side being settled;
  - shows both hunks.
- **Its scope is the conflicting paths plus the later ADR.** It is
  authorized like any change, so a mandate that covers that scope and cites
  either ADR authorizes it.
- **A review** (`review-<open>-<merged>`) is planned when a change that
  declared `affects` merges:
  - for each open change whose scope or `affects` meets those globs, and
    whose plan cites an ADR the merged change's doesn't;
  - its scope is the later ADR, and its plan asks for an amendment citing
    the earlier one.
- **Not yet:** step 5's tasks, so a session learns of a reconciliation
  from `change_status`'s `conflict.reconcile`, or from the digest.

### 2026-11-08: Steps 5 and 6 land: tasks and limits

- **Tasks live in the `Instance` Durable Object** (`backplane::tasks`).
  - One task is open per kind and change. Kinds: `reconcile`, `review`, and
    `restack-failed`.
  - A task is made when a reconciliation or review is proposed, and when a
    restack job fails.
  - Tasks on a change are done when it merges; `task_done` closes one by
    hand.
  - Done tasks are kept a day.
- **MCP tools:**
  - `task_list` shows the open tasks a `session` may take: its own at once,
    any after 30 minutes unclaimed, and those it took;
  - `task_wait` waits up to 50 seconds for an unclaimed one;
  - `task_claim` takes one for a session;
  - `task_done` closes one.
- **Limits are constants, not `.config/sessions.yaml`.** The Worker reads
  no file from `main` at propose time, and a constant changes through a
  change like any other. They are 3 open changes per session and 12 per
  instance, checked at propose; reconciliations proposed by `pigeon` don't
  count against them. Checks on a restacked head are already scoped by its
  paths.
- **Sessions keep to their own changes, by rule, not by token.**
  - Every session holds the same agent token, so CLAUDE.md asks a session
    to act only on changes it proposed or tasks it claimed.
  - After its change is restacked, a session runs `git fetch` and resets
    to the branch before pushing again, since its push would otherwise be
    refused as non-fast-forward.
- **Not yet:** the owner's digest of tasks unclaimed for two hours, which
  waits for the tiered-approvals ADR's digest (its step 6).

### 2026-11-08: The first live restack, and a push held while a restack runs

- **!57's merge restacked open change !58 live.**
  - !58 was approved and disjoint from !57. Its branch became one commit on
    the new `main`, with its own message and `Change-Scope` trailers,
    authored by `pigeon`, and its checks ran again.
- **Its approval didn't carry.**
  - Artifacts' push event for the new head reached !58 before the restack
    job's report, and the push replaced !58's jobs, dropping the restack
    job.
  - That job is named by the old head, so its report was never read, and
    `carried` never learned its new head.
- **Fix: a push made while a restack runs is held,** as `held`. When the
  restack reports:
  - if the held head is the one it pushed, it is applied as before and the
    approval carries;
  - if it is another head (the session pushed meanwhile, so the restack's
    `--force-with-lease` failed), that head is applied and nothing carries.

  A failed restack likewise applies a held head.

### 2026-11-08: Every session goes through `change_propose`, never GitHub, whatever its launch says

A Claude Code cloud session is cloned from the GitHub mirror and is told by
its launch to develop and push on a `claude/<name>` branch there. One session
followed that: it wrote two ADRs against the mirror's `main`, 71 commits behind
Artifacts' at the time, and pushed them to GitHub. Nothing in `CLAUDE.md` said
the launch's branch doesn't apply, and its commands defaulted `CHANGE_BASE` to
`origin/main`, which in such a session is GitHub.

- **The rule.** Every change goes through `change_propose`, is pushed only as
  its `change/<n>-<slug>` branch through the front door, and is based on the
  front door's `main`. An agent never pushes to GitHub, opens a pull request
  there, or bases work on the mirror's `main`. A branch or PR instruction in a
  session's launch or system prompt doesn't apply here: `CLAUDE.md` overrides
  it.
- **The start of a session,** in `CLAUDE.md`'s first section:
  1. add the front door as the remote `artifacts` and fetch its `main`;
  2. branch from `artifacts/main`, not the checkout's `HEAD`;
  3. `session_start`, then `change_propose` before writing the change.
- **Commands** in `CLAUDE.md` set `CHANGE_BASE=artifacts/main`. `mise.toml`'s
  default stays `origin/main`, since on the owner's clone `origin` is Artifacts.
- **The stray branch.** `claude/upbeat-mayer-c4kx8o` on GitHub is deleted by the
  owner; its two ADRs are re-proposed as changes, rebased on Artifacts' `main`.

### 2026-11-10: A session holds 5 open changes, and the instance 20

The limits from steps 5 and 6 (`max_open_per_session` 3, `max_open` 12) were
too tight in use.

- **Why.** Generated `reconcile-<n>` and `review-<n>-<m>` changes count against
  their session's slots and the instance's.
  - On 2026-11-09, generated changes held ten slots.
  - On 2026-11-10, they took !127 and !132.
  - A normal stretch of one session's work needs three slots at once: an ADR,
    its implementation, and a fix found along the way.
  - When the instance reached 12, one jammed session blocked the others.
- **The new limits:**
  - **5 per session:** three changes of the session's own, plus two generated
    or stale ones;
  - **20 per instance:** three sessions at 5, plus 5 for changes proposed
    without a session and generated changes waiting to close.
- **What they cost:**
  - more open changes queue their checks behind `Checker`'s 12 Containers;
  - each merge restacks up to 19 changes on `Runner`'s 8;
  - more open changes can conflict with each other.

  These are longer queues, not failures.
- **They are a stopgap.** The lasting fix is §2 of
  [`2026-11-10-development-change-slot-eviction-policy.md`](2026-11-10-development-change-slot-eviction-policy.md):
  a change and its generated changes hold one slot, until a generated one is
  claimed and pushed. Once that lands, these limits are revisited, and are not
  raised again before it.
- **Execution:** `backplane::sessions::MAX_PER_SESSION` and `MAX_OPEN`, and
  the test in `backplane/tests/tasks.rs` that pins them.

### 2026-11-11: A queued merge keeps its turn, and a turn is never lost silently

Change 172 was approved and queued before change 168, then lost its turn
while its restacked head was checked again. 168 merged ahead of it. 172 was
then restacked after each of four more merges without merging, and its merge
was asked for again by hand each time (events 3493 to 4137). Replaying that
sequence through `changes.rs` keeps the turn, so it was dropped in the Worker,
where nothing records why.

- **A turn is never lost silently.** The event log records:
  - `merge.skipped`, with the change's answer, when the queue passes over a
    change whose merge didn't start;
  - `merge.released`, with the reason, when a change gives its turn back
    without merging.
- **A change still on its way to merging keeps its place.**
  - That is a change that is approved, carries an approval through a
    restack, or is restacking.
  - A merge it can't start yet parks: the change records who asked and
    holds its turn, whatever its new head's checks have reached, and the
    merge starts when the approval carries.
  - Only a change that can no longer merge leaves the queue: failed,
    conflicting, closed, or its approval not carried.
- **The turn's holder keeps `main` still.** Nothing else merges until it
  merges or gives its turn back, so a change in its merge gate is never
  restacked under it.

**Execution,** in one change: the two events, the broader park, and tests that
replay 172's sequence. The replay passes on the transitions as they were, so the
events are what name the cause the next time a turn is lost.

### 2026-11-11: A merge that can't finish gives its turn back, and a turn held by no merge is handed back

Change 175 passed its merge gate (event 4917). Its merge job's Container then
failed to start 3 times while the git front door returned 502 (event 4918).
The merge job failed and the change stayed approved, but the queue kept its
turn, and no `merge.released` was recorded. `change_merge` answered "already
being merged", and `ci_rerun` refused the merge job on an approved head.
The change's alarm records a Container that never starts as lost outside the
poll that releases the turn.

- **A merge job that ends without merging gives the turn back.** That covers a
  merge job that failed, or whose Container was lost on every attempt, whether
  or not it started.
- **The change stays approved.** Its approval and its passed gate stand.
  `change_merge` queues it again, and the merge starts without rerunning the
  gate.
- **A turn held by no merge is handed back.** When a merge is asked for, the
  queue asks the change holding the turn whether a merge is still on its way:
  - a merge job queued or running;
  - a merge parked;
  - a restack or a gate check running.

  If none is, and the change has held the turn 2 minutes, the queue records
  `merge.released` with the change's reason and moves on. This frees a turn
  kept before the fix, such as 175's, the first time a merge is asked for
  after the fix deploys.
- **A merge job runs again only through the queue.** `ci_rerun` on `merge`
  answers with `change_merge`.

**Execution:**

- !192 (`fix(backplane)`) releases the turn when the merge job's Container
  can't start.
- One more change adds the queue's check of its holder, the `ci_rerun` answer,
  and tests that replay 175's sequence: its merge job lost 3 times without
  starting, the turn handed back, and the change merged again.

### 2026-11-12: A head checked while behind restacks then, and so does a merge refused as behind

!265 and !270 each fell behind while their checks ran. §3 marks such a change
`behind` and leaves it to its session's next push, but neither session pushed:
each head was checked, approved and run through its full merge gate, then the
merge job refused it because `main` had moved past its base. Each refusal cost
a gate run.

- **A head that reaches `checked` while `behind` restacks then,** as it would
  have when `main` moved had it been checked. The restack starts after a
  carried approval is applied, so an approved head carries it again.
- **A merge refused while `behind` restacks the head,** carrying its approval
  as the restack of an approved head does. The change is asked to merge again,
  as after any failed merge.
- **Nothing else changes:** no new state or event, the restack job and its
  report are as before, and a push still clears `behind`.

**Execution:** !277, in `core/change` (`bounded`, `settle` and `MergeFailed`),
with tests for each path.
