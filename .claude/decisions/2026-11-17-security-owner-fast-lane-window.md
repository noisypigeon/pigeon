## The owner's fast lane: a passkey-granted window that waives a named line of work's code checks and fronts it in the merge queue

* Date: 2026-11-17
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: security
* Governs: `src/**/change/jobs.rs`, `src/**/change/gate.rs`, `src/**/change/queue.rs`, `src/**/change/forecast.rs`, `src/**/worker/main_run.rs`, `src/**/feed.rs`, `.config/approvals.yaml`

## Context

On the evening of 2026-11-17 the capacity-pool and AI Gateway fixes needed
several small config iterations (!1049, !1056, !1060, with more to come).
Each one waited 10 to 20 minutes before a platform plan could show whether
the fix worked:

- for its code checks on GitHub (fmt, clippy, tests), which a one-line
  config change rarely affects;
- for a restack and a second round of the same checks at its merge turn;
- behind unrelated changes in the merge queue.

The owner asked for a **fast lane**: for a short window, a named line of work
iterates without waiting on CI and goes to the front of the queue.

### What exists, and what this reuses

- **Mandates**
  ([`2026-11-08-security-tiered-approvals-task-mandates.md`](2026-11-08-security-tiered-approvals-task-mandates.md)
  §3 and its amendments). A mandate is a scoped, timed grant: an agent
  proposes it, the owner grants it on `/mandate/<id>` with a proof, the
  `Instance` keeps it, anyone may end it, and a failed deploy holds it.
  A mandate approves; it waives nothing.
- **The CI window**
  ([`2026-11-10-development-code-checks-github-actions.md`](2026-11-10-development-code-checks-github-actions.md),
  2026-11-11 amendment). An instance-level window with an `until`, stored on
  the `Instance`, read by `ChangeRun` before it routes a code check, closing
  by itself at its end. It moves checks; it skips none.
- **Auto**
  ([`2026-11-15-security-standing-auto-owner-flags.md`](2026-11-15-security-standing-auto-owner-flags.md)).
  It approves and merges what it covers. Its gate (`approvals.yaml`'s
  `gate:`) keeps the owner's passkey on the code that sets tiers and proves
  the owner.
- **Fronts**
  ([`2026-11-08-development-concurrent-agent-sessions.md`](2026-11-08-development-concurrent-agent-sessions.md),
  2026-11-15 amendment;
  [`2026-11-17-security-auto-covers-routine-owner-asks.md`](2026-11-17-security-auto-covers-routine-owner-asks.md)
  §2). `queue::front` reorders the waiting line only. It never pre-empts the
  merge running, and a change that keeps its turn still goes first.
- **Yields and forecasts**
  ([`2026-11-12-development-grants-applies-rescue-yield.md`](2026-11-12-development-grants-applies-rescue-yield.md)
  §5).
- **The feed and the view contract**
  ([`2026-11-17-runtime-mcp-feed-view-contract.md`](2026-11-17-runtime-mcp-feed-view-contract.md)).
  Every tool declares one of four views, and the feed's item types are a
  closed set.

None of these does what the owner asked. A lane is closest to a mandate
(a named line of work, granted by the owner for a time), timed like the CI
window, so it **reuses the mandate's record, request, page and proof**, and
adds only what a lane decides: which jobs it waives, and where its changes
queue.

### What a head's checks are today

- `bound` runs on Cloudflare. It checks the head's bounds and scope, runs
  every policy in `.config/policies/` (the secrets pins among them), the
  conflict forecast, and the tiers.
- The **code checks** that `change.scope` selects (fmt, clippy, `features`,
  tests, wasm and worker builds) run on GitHub Actions, or on the `Checker`
  under the CI window. `policy.check` and `infra.validate` may also be
  selected as code checks.
- `settle` makes the head `checked` when every gating job passed. Only
  `mirror` doesn't gate.
- At its merge turn, `gate_missing` runs the head's full set not yet passed
  as `gate: <name>` jobs, then `merge` squashes it.

## Implementation plan.

### 1. The grant

- **A lane** is kept by the `Instance`, beside its mandates, in
  `core::approval::lanes`:

  ```
  Lane { id, reason, line, minutes, started, until, by, state,
         waived: [(change, head, job)], merged: [change], deferred }
  line  = Session(label) | Changes([n]) | Paths([glob])
  state = Offered | Open | Ended(why)
  ```

- **Asking.** `lane_request {line, minutes?, reason}` may be called by a
  session or the owner. It never opens a lane. It answers a link to
  `/lane/<id>`, lists it in `/approvals` as an `approval.pending` item with
  subject `{lane: id}`, and pushes it, as a mandate's request does.
  - `line` is exactly one of: `session` (a session's URL), `changes` (change
    numbers), or `paths` (globs, with the mandate's coverage rule: a
    change's glob lies inside the lane's glob).
  - `minutes` defaults to **30**. The hard maximum is **120**
    (`lanes::MAX_MINUTES`).
  - `reason` is required, and is shown on the page, the feed and the
    digest.
- **Granting** is the owner's, by passkey, on the page or in a client, under
  the passkey ADR's single proof. Its operation is `lane.grant`, at the
  `owner` tier, never covered by Auto, a mandate or a policy. The minutes
  start at the grant. A request link lasts 1 hour.
- **Widening or extending** is a new request naming the open lane
  (`extends: id`), granted the same way. It may add to `line` and move
  `until` to at most `MAX_MINUTES` from the grant. No session can widen or
  extend a lane itself.
- **Ending** is `lane_end {id}`. It is open to the owner and to the sessions
  the lane names, because ending only takes power away, as for `mandate_end`.
- **At most one lane is open at a time** (`lanes::MAX_OPEN = 1`). A second
  grant is refused while one is open; the owner extends or widens the open
  lane instead. A lane is a short exception, and one at a time keeps its
  deferred checks and its record readable.

### 2. Which changes are in the lane

`lanes::covers(lane, change, head, now) -> Cover` is pure. A change is
covered when all of these hold:

- the lane is `Open` and `now < until`;
- its line names the change: its `session` is the lane's session, or its
  number is listed, or its scope lies inside the lane's globs;
- the head's merge rule, as `tiers::classify` set it, is neither `owner`,
  nor `guarded`, nor `sentinel`, nor marked as a gate path. This holds even
  where Auto's `AUTO_COVERS` mark would let Auto approve it. So a head that
  touches `.config/approvals.yaml`, `.config/policies/**`, a
  `secrets.config.yaml`, any secrets or `secret_code:` file with secret
  material, the gate, or any other guarded or eviction path, is never in a
  lane;
- the change isn't carrying out a brief whose head waits for review or is
  escalated (the mandate ADR's 2026-11-14 amendment holds).

A head that leaves the lane, for example a new push that touches a guarded
path, has its waived jobs requeued (§5) and is checked as any other.

### 3. What a lane waives, and what it never does

**Waived** for a covered head, and only these:

- the code checks `change.scope` selects, except those listed below:
  fmt, clippy, `features`, the test jobs, wasm and worker builds;
- the same checks as the merge gate's `gate: <name>` jobs.

**Never waived** (`lanes::NEVER_WAIVED`, a constant in the gate):

| Job | Why |
| --- | --- |
| `bound` | bounds and scope, every policy (the secrets pins among them), the forecast and the tiers |
| `policy.check`, when the scope selects it | the policies, again, on the head |
| `infra.validate` | an invalid unit fails the deploy itself |
| `restack` | the head must be on `main` |
| `mirror` | the public mirror |
| `merge` | the squash, and the approval it needs |

- **A lane approves nothing.** A covered head still needs its approval:
  Auto's, when Auto is running and covers it, else the owner's. A lane only
  makes the head reach `checked` sooner, and merge sooner.
- **How a job is waived.** When `ChangeRun` would dispatch a job for a
  covered head, it records the job as `waived` instead, naming the lane:
  `JobStatus::Waived { lane }`. `settle` and `gate_missing` count a waived
  job as passed, on that head only. `Change::waive` refuses a name in
  `NEVER_WAIVED`, and a test pins that it does.
- **A job already running** when the lane opens is left to finish. One
  queued and not yet dispatched is waived.

**The risk accepted.** A lane change merges with its code untested. `main`
may stop building, fail its tests, or deploy a Worker that fails at
runtime. The secrets and policy pins, the bounds, the tiers and the
approval still hold, so a lane cannot widen what a change may touch, only
how soon its code is tested. §5 bounds how long a broken `main` lasts.

### 4. Queue priority

- **A covered change goes to the front automatically** when its merge is
  asked for or queued (`queue::lane_front`). The waiting line's order
  becomes:
  1. the merge running, and any change that keeps its turn while its
     restacked head is checked;
  2. lane changes, oldest first;
  3. changes the owner fronted;
  4. changes sessions fronted under Auto;
  5. every other change, in arrival order.
- **Yields still decide.** A lane change that yields to another waits for
  it, as now. A **forecast** between a lane change and one outside the lane
  orders the lane change first, ahead of the "further along" rule. A yield
  never closes a cycle, as now.
- **No automatic pre-emption.** A lane never takes the turn from the change
  holding it.
- **The owner may pre-empt.** `change_front {number, preempt: true}` at the
  owner tier, with a passkey, never covered by a window. It releases the
  current holder's turn when that holder's `merge` job hasn't started
  (`merge.preempted`). The holder keeps its approval and goes back to
  place 1 behind the fronted change. A holder whose squash has started
  can't be pre-empted.
- **When the lane ends,** its changes keep their places. Nothing is moved
  back.

### 5. Deferred checks: waived checks still run

- **After each lane merge,** `MainRun` runs the full set of code checks
  waived on that change against `main`'s newest SHA, as a `lane.deferred`
  run on the `Checker`. Runs coalesce as deploys do: one run, on the
  newest SHA, covers every lane merge up to it.
- **The deploy doesn't wait for it.** The point of the lane is to see the
  platform's plan and deploy without waiting. A deploy that fails ends the
  lane anyway (§6).
- **A deferred failure surfaces at once:**
  - a `lane.broken` feed item, `needs: owner`, pushed, naming the SHA, the
    failed checks and the lane merges since the last pass;
  - `main` is **held** as for a failed deploy: Auto and mandates merge only
    `fix` and `revert` changes, and the lane ends;
  - the control plane proposes a generated `revert-<n>` change for the
    newest lane merge in the failing run, to the lane's session, as a
    task. The session may instead push a `fix`; the first to merge wins,
    and the other closes as superseded (F2).
  - The hold lifts when a deferred run or a deploy's checks pass on a later
    SHA of `main`.
- **At the window's end,** each covered change still open has its waived
  jobs requeued on its current head (`JobStatus::Queued`), so no open change
  keeps a waiver past the lane.

### 6. When a lane ends

A lane ends, recording why, when:

- its `until` passes (`expired`);
- the owner, or a session it names, ends it (`ended by <who>`);
- a deploy of `main` fails or rolls back (`deploy failed`);
- a deferred run fails (`deferred failed`);
- the owner pauses Auto (`auto paused`), since the owner then wants to see
  each merge.

Ending is immediate and needs no proof. A failed deploy ends the lane
rather than holding it, unlike a mandate: a lane is short, and the owner
grants a new one if they still want it.

### 7. Audit

- **On the change:** each waived job shows `waived: lane <id>` with its
  head, and `change_status` lists them under `waived`. The change's events
  gain `job.waived`.
- **In the event log and feed:** `lane.requested` (as `approval.pending`),
  `lane.open`, `lane.extended`, `lane.ended`, `lane.deferred` and
  `lane.broken`. Item types are added to the feed's closed set by this
  ADR's amendment to the feed ADR.
- **On `/approvals`:** an open lane is listed at the top of
  `approval.list`, as a row with its line, reason, time left and an `End`
  action (`lane.end`, `style: destructive`, `proof: false`).
- **In the digest:** a `lane` section for each lane open since the last
  digest: who granted it, its line and reason, each change merged under
  it with the jobs waived, and its deferred runs' results. The digest is
  `needs: owner` while a deferred run failed and isn't resolved.
- **`lane_status`** shows the open lane and the last ten, with all of the
  above.

### 8. The catalog and views

Each new operation declares its view, per the feed ADR's §2:

| Operation | Tool | Tier | View |
| --- | --- | --- | --- |
| `lane.request` | `lane_request` | auto | `card` (the link, and the line, minutes and reason) |
| `lane.grant` | (the page, or an action with `proof: true`) | owner | `form` (line, minutes, reason; submit with proof) |
| `lane.status` | `lane_status` | auto | `list` (one row per lane; `open` gives a `detail`) |
| `lane.end` | `lane_end` | auto | `card` |

`change.front` gains the `preempt` argument; its view is unchanged.

### 9. Where the code lives, and who approves it

- `src/pigeon-core/src/approval/lanes.rs` (already in the gate, as
  `approval/**`): `Lane`, `covers`, `NEVER_WAIVED`, `MAX_MINUTES`,
  `MAX_OPEN`, grant, extend and end, and their tests.
- `src/pigeon-agent/src/worker/lanes.rs`: the `Instance`'s storage, the
  operations and the page. It joins `gate:` in `.config/approvals.yaml`,
  since it applies a lane's waivers.
- `ChangeRun` (`worker/change.rs`, in the gate) asks the `Instance` for the
  lane's cover before dispatching a job.
- The change lifecycle (`change/jobs.rs`, `change/gate.rs`), the queue
  (`change/queue.rs`, `change/forecast.rs`), `MainRun` and the feed are
  outside the gate: they honour `Waived` and the lane band, and decide
  nothing about who is covered.
- `.config/approvals.yaml` gains `lane.grant` at the owner tier and the
  Worker's `lanes.rs` in `gate:`. It is on `secrets:`, so the owner
  approves it.

### Execution

Gate files go to this planner and the owner's passkey. The rest are briefs.

1. **This ADR,** with the feed ADR's amendment. The owner's.
2. **`feat(approval)`, planner:** `core::approval::lanes` with `covers`,
   `NEVER_WAIVED` and the grant's rules, and tests. Gate; the owner's.
   Its amendment adds `src/**/approval/lanes.rs` to `Governs`.
3. **Brief, sonnet, `feat(backplane)`:** `JobStatus::Waived { lane }`,
   `Change::waive` refusing `NEVER_WAIVED`, `settle` and `gate_missing`
   counting it, `job.waived`, and the requeue at a lane's end. After 2.
4. **Brief, sonnet, `feat(backplane)`:** the lane band in `queue.rs`, the
   forecast's lane rule, and `change.front`'s `preempt`
   (`merge.preempted`). After 2. In parallel with 3.
5. **`feat(backplane)`, planner:** `worker/lanes.rs`, the operations and
   their views, the page, and `ChangeRun`'s dispatch asking for the cover.
   Its amendment adds `src/**/worker/lanes.rs` to `Governs`.
   Gate; the owner's. After 2 and 3.
6. **Brief, sonnet, `feat(backplane)`:** `MainRun`'s deferred run, the hold,
   `lane.broken`, and the generated revert. After 5.
7. **Brief, haiku, `feat(backplane)`:** the feed types, the digest's `lane`
   section, and the lanes on `approval.list`. After 5. In parallel with 6.
8. **`chore(approvals)`, planner:** `lane.grant` at the owner tier, and
   `worker/lanes.rs` in `gate:`. The owner's. After 5 deploys, since
   `approvals.yaml` is read with `deny_unknown_fields`.
9. **`docs(claude)`:** `CLAUDE.md`'s Commands name `lane_request`,
   `lane_status` and `lane_end`.

### Verification

- A covered head's fmt, clippy and tests are `waived`, and it reaches
  `checked` once `bound` passes.
- A head touching `.config/approvals.yaml`, a policy, a secrets file or a
  gate path is never covered, and its checks run.
- `bound`, `policy.check`, `infra.validate`, `restack`, `mirror` and
  `merge` are never waived; `Change::waive` refuses each.
- A lane change asking to merge goes ahead of every waiting change outside
  the lane, and behind the merge running.
- A session's `lane.grant` is refused; `lane.request` only lists a link.
- A lane past its `until` covers nothing, and an open lane change's waived
  jobs are requeued.
- A deferred run that fails holds `main`, ends the lane, pushes
  `lane.broken`, and proposes the revert.
- A failed deploy ends the lane.
- Every waived job is in `change_status`, the feed and the digest.

### 2026-11-17: Review of !1064 against !1062: they agree

!1064 amends
[`2026-11-17-runtime-mcp-feed-view-contract.md`](2026-11-17-runtime-mcp-feed-view-contract.md):
its `Governs` gains `src/**/present/**`, the presenters step 5a added.

- **What it touches.** That ADR's header and one amendment; no code, and
  none of this ADR's paths. `src/**/feed.rs` stays governed by both.
- **The feed.** This ADR's `lane.open`, `lane.ended` and `lane.broken`
  join that ADR's §3.2 types, as its amendment here says; !1064 changes
  none of them.
- **The views.** The lane's operations declare views within the four, and
  their presenters will sit in `src/**/present/**`, which both ADRs then
  govern.

Neither ADR changes.
