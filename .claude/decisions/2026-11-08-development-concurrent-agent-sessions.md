## Concurrent agent sessions: a merge queue, restacking, and reconciliation planned on conflict

* Date: 2026-11-08
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development

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
