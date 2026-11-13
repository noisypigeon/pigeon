## Where a session is: one stage per change, subscriptions held by the control plane, the harness that shows them, and a breaker that takes a stuck change out of Auto

* Date: 2026-11-13
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development
* Governs: `src/pigeon-cli/crates/backplane/src/core/change/lifecycle.rs`, `src/pigeon-cli/crates/backplane/src/core/change/stage.rs`, `src/pigeon-cli/crates/backplane/src/core/approval/windows.rs`, `src/pigeon-cli/crates/backplane/src/sessions.rs`, `src/pigeon-cli/crates/backplane/src/worker/tasks.rs`

## Context

The owner asked for three things in the Claude harness:

1. better notifications, and subscriptions to change management;
2. a way to see where a session is in the pipeline;
3. a session that is stuck in a loop, unable to fix its change, gives up
   Auto gracefully instead of looping.

### What exists

- **Events.**
  [`2026-11-08-development-events-tickets-digest-mandate.md`](2026-11-08-development-events-tickets-digest-mandate.md)
  gives the instance an event log, `GET /v0/events` and `events_wait`
  (a long poll filtered by `session=` and `change=`), Web Push to the owner,
  and the hourly digest. APNs joined it in
  [`2026-11-12-runtime-ios-app-approvals-insights.md`](2026-11-12-runtime-ios-app-approvals-insights.md) §6.
- **Following.** A session arms `Monitor` with a Python loop over `curl`,
  copied from that ADR's step-2 amendment. It carries `AFTER` itself and is
  re-armed every 30 minutes.
- **Sessions.** Tickets bind a label to its changes (same ADR, §5).
  [`2026-11-08-development-concurrent-agent-sessions.md`](2026-11-08-development-concurrent-agent-sessions.md)
  adds the merge queue, restacks, reconciliations and tasks.
- **Loops the control plane already breaks.**
  [`2026-11-12-development-grants-applies-rescue-yield.md`](2026-11-12-development-grants-applies-rescue-yield.md)
  §5 plans yields, conflict forecasts and a loop breaker for queue loops:
  3 restacks without a merge, 3 reconciliations in an hour, 2 released turns.
  It flags the change with a `loop` task. It isn't built yet (its step 7).
- **Auto.** The owner's Auto window (iOS ADR §5, `core/approval/windows.rs`)
  authorizes, approves and merges every change below the `owner` tier for 1
  to 24 hours. Task mandates do the same inside a scope. A failed deploy
  pauses both.
- **Continuing.** Per
  [`2026-11-02-development-adr-driven-lifecycle.md`](2026-11-02-development-adr-driven-lifecycle.md)'s
  2026-11-11 amendment, an agent carries an ADR's steps through unasked, and
  stops on "a failure it can't fix within the step".
- **The harness.** `.claude/settings.json` doesn't exist. It is guarded and in
  `secrets:` (`.config/approvals.yaml`), so adding it is the owner's, each
  time. Its hooks are written by the agent's own repository, so they can show
  things but never decide anything (the control-plane ADR's 2026-11-06
  amendment).

### The gaps

1. **No one answer to "where is it".** `change_status` returns a state, jobs,
   `behind`, a conflict and the queue separately. `checked` doesn't say
   whether the change waits for the owner, a window, the queue or another
   change. The agent and the owner each piece it together, and the iOS app
   shows only approvals.
2. **The subscription is the session's to hold.**
   - Its filter is its label. A deploy carrying its merge, and a change
     yielded to it or handed to it, aren't named by that label.
   - Its cursor, `AFTER`, lives in a shell variable. A lost monitor, a
     forgotten re-arm, or a context compaction loses it, and the session
     either replays from 0 or misses events.
3. **Every event wakes the session.** `job.started` and `job.passed` for each
   of a change's checks wake it, though only a few events need it to act.
4. **Silence is ambiguous.** A dead monitor and a quiet pipeline look the
   same, to the session and to the owner. Nothing records who is listening.
5. **Nothing stops a session's own fix loop.** The queue loop breaker covers
   restacks, reconciliations and turns. It doesn't cover a session pushing a
   sixth fix for the same clippy error, rerunning a flaky job until it
   passes, or closing a change and proposing a copy (reconcile-255 was
   proposed 17 times, !260 to !289). Auto and mandates keep authorizing each
   new head. "Can't fix within the step" isn't defined anywhere.
6. **Giving up is all or nothing.** The only brake on Auto is pausing the
   whole window, which stops every session's changes for one stuck change.

## Implementation plan.

### 1. One stage per change

**A change's stage is derived, never stored.** `core/change/stage.rs` is a
pure function of the change's record, its queue entry, its tasks, its
yield, and the deploy that carries its merge commit. It answers:

- `stage`: one name from the table below;
- `needs`: who acts next: `agent`, `owner`, `platform` (the control plane
  is working), `provider` (GitHub Actions, Buildkite or a Container queue),
  `change:<n>` (another change), or `none`;
- `next`: one line saying what that is, e.g. "push a fix for `clippy`",
  "approve, or let the Auto window", "turn 2 of 3, behind !m";
- `since`: when the change entered the stage.

| Stage | Means | `needs` |
|---|---|---|
| `proposed` | its plan waits for authorization | `owner`, or `platform` under a window or mandate |
| `authorized` | nothing pushed yet | `agent` |
| `bounding` | the bound job and policies run | `platform` |
| `refused` | a policy refused the head | `agent` |
| `checking k/n` | `k` of `n` code checks done | `provider` |
| `failed` | a check failed on the head | `agent` |
| `checked` | waits for its approval | `owner`, or `platform` under a window or mandate |
| `approved` | approved, merge not asked | `agent` |
| `queued p/q` | turn `p` of `q`, behind the holder | `change:<holder>` |
| `restacking` | a restack or its checks run | `platform` |
| `conflicted` | its reconciliation is planned or open | `agent`, or the session that claimed the task |
| `yielded` | parked behind another change | `change:<to>` |
| `merging` | its merge job runs | `platform` |
| `deploying` | the deploy that carries it runs | `platform` |
| `done` | merged, and deployed or nothing to deploy | `none` |
| `deploy-failed` | the deploy carrying it failed or rolled back | `owner` |
| `attended` | the breaker took it out of Auto (§4) | `owner` |
| `closed` | closed or evicted | `none` |

- **`yielded`** comes with the yield ADR's step 7. Until then it is never
  answered.
- **`change_status` and `change_list` gain `stage`, `needs`, `next` and
  `since`.** Nothing is removed, so callers keep working.
- **Every event gains `change`, `stage` and `needs`,** as of the moment it
  was appended, so one line says where the change is now.

**A session's position is its changes' stages.** `session.status` (MCP
`session_status`, read-only, by label) answers:

- each open change of its family (§2), with its stage, oldest first;
- its tasks, claimed or routed to it;
- whether it is listening (§2), and its last poll;
- its breaker (§4);
- the window or mandate covering each change, if any.

`pigeon backplane session status [--json]` prints the same.

### 2. Subscriptions held by the control plane

**`session.start` opens the session's subscription.** The `Instance` keeps,
per live ticket, a filter, a cursor and the time of the last poll.

- **The filter is the session's family,** worked out at each event, not
  listed by the session:
  - changes whose `session` is its label, and their generated
    `reconcile-<n>` and `review-<n>-<m>` changes;
  - changes handed to it (`change_handoff`), and changes yielded to or by
    its own;
  - tasks routed to it or claimed by it;
  - the deploys that carry its merges;
  - notes left on its changes (`change_note`).
- **The cursor is the server's.** `after=session` resumes from the highest
  `after` the session has polled with. A re-armed monitor, or one armed
  after a compaction, misses nothing and replays nothing. A number still
  works, as today.
- **`wake=agent` answers only what the session must act on:** events whose
  `needs` is `agent`, and outcomes it reports: `done`, `closed`,
  `deploy-failed` and `attended`. Everything else is still in the log, and
  `wake=all` is today's behaviour.
- **Listening is recorded.** A poll in the last 2 minutes means the session
  is listening. `session.status`, the digest and `/approvals` show it.
- **A session whose ticket ends** loses its subscription. Its cursor is kept
  7 days, so `session_start` with `replace` resumes it.

**The follower is a committed script, not a heredoc in an ADR.**
`tooling/session/follow.py`, stdlib only and run with `python3 -I`:

- long-polls `?session=<label>&after=session&wake=agent&wait=50` through the
  agent's proxy, and prints one line per event:
  `<id> !<n> <stage> needs=<who> <summary>`;
- writes the latest `session.status` to `~/.cache/pigeon/status.json` after
  each answer, for §3;
- reads the label from `~/.config/pigeon/session.json`, which the agent
  writes after `session_start`, outside the repository, beside its ticket.

`CLAUDE.md`'s recipe becomes one line: arm `Monitor` with
`python3 -I tooling/session/follow.py`, 30 minutes, re-armed on expiry with
nothing to carry.

### 3. The harness: brief on start, a status line, and a Stop reminder

**A committed `.claude/settings.json`,** in its own change, at the owner's
tier (it is guarded and in `secrets:`). Its hooks only show things; nothing
the control plane decides reads them.

- **`SessionStart`, on `startup`, `resume` and `compact`,** runs
  `tooling/session/brief.py`. It prints into the session's context:
  - with no `~/.config/pigeon/session.json`: `CLAUDE.md`'s four start steps,
    in three lines;
  - otherwise: `session.status` as a short table, the changes that need the
    agent first, and whether the follower is listening.

  After a compaction the agent learns where it is from the control plane,
  not from its summary.
- **`statusLine`** runs `tooling/session/statusline.py`. It reads only the
  cache file, so it makes no request:
  `!n checking 3/5 · !m queued 2/3 · !k attended · Auto 3h · listening`.
  It shows in the terminal. The web and the app follow events instead.
- **`Stop`** runs `tooling/session/stop.py`. When the session holds a change
  that needs it and the follower isn't listening, it prints a reminder as a
  `systemMessage`.
  - **It never answers `block`.** A Stop hook that refuses to stop is itself a
    loop, and the breaker (§4) is where stopping is decided.
- **Each script exits 0 and prints nothing on any error,** so a broken hook
  never stops a session.

**Who notifies whom.**

- **The control plane is the only thing that pushes to the owner:** Web Push
  and APNs, for what they push today, plus `attended` (§4).
- **The agent pushes (`PushNotification`) twice at most per ADR:** when its
  execution is done, and when it stops with work left. Never for an approval,
  a failure or a merge, which the control plane already pushes or the digest
  holds.

### 4. The breaker: a stuck change leaves Auto, and nothing else does

**`core/change/breaker.rs` counts, per change,** from what `ChangeRun`
already sees. A change trips when, since its last resume:

| | Signal |
|---|---|
| B1 | the same check failed on 3 heads in a row |
| B2 | the same failure fingerprint on 2 heads |
| B3 | the same policy rule refused 3 heads |
| B4 | 8 heads pushed without one reaching `checked` |
| B5 | one job on one head run 3 times (`ci_rerun` twice) |
| B6 | 3 hours in stages that need the agent, across 3 or more heads |
| B7 | the queue loop breaker's flags (the yield ADR §5) |

- **A fingerprint** is the job's name and its log tail's first error line,
  with numbers, hashes, paths and times normalized. Two heads with the same
  fingerprint mean the fix didn't touch the failure.
- **B7 replaces the yield ADR's `loop` task.** A queue loop and a fix loop end
  the same way. The yield ADR is amended to say so.
- **The thresholds are constants in `breaker.rs`,** tested, as
  `MAX_PER_SESSION` is.

**The agent trips it itself first.** `change_attend {number, reason, tried,
remains}` is the session's own act, on its own change, at the `auto` tier.
`CLAUDE.md` says to call it when:

- a second attempt at a failure fails the same way;
- or it can't explain a failure;
- or the fix needs a decision the ADR doesn't make.

**What tripping does.** The change becomes `attended`, recorded with the
signal, its evidence (the heads, the fingerprints, the refusals) and the
session's `tried` and `remains`.

- **It leaves Auto, alone.**
  - The window and every mandate skip it: `window_approves` answers none for
    an attended change.
  - Its open approvals expire, and its merge turn is released
    (`merge.released`, `attended`).
  - The window, the mandates and every other change carry on.
- **The loop stops mechanically,** not by the agent's restraint:
  - the front door refuses pushes to its branch;
  - `ci_rerun` and the agent's `change_merge` refuse it;
  - running jobs finish, and no new ones start.
  - Owner operations still work.
- **The owner is pushed once:**

  > !n left Auto: `clippy` failed on 3 heads with the same error
  > (`E0599: no method named …`). Tried: … Left: … Resume, hand over, or close?

- **Its page, `/attend/<n>`, offers four answers:**
  - **Resume:** at the `confirm` tier. It clears the counters and reopens
    the branch. An optional hint goes to the session as a `change_note`,
    which the session treats as data from the owner's page, not an
    instruction from elsewhere.
  - **Resume, asking each time:** as Resume, but no window or mandate covers
    the change again.
  - **Hand over:** a `resume` task, as the eviction ADR §6 makes, for another
    session, with the evidence as its brief.
  - **Close:** at `change_close`'s tier. The head is kept as
    `refs/evicted/<n>-<slug>`.
- **An attended change is never evicted** while its page is open: 7 days.
  The digest then asks again.

**A session that trips twice gives up its own Auto.** Two attended changes
in 6 hours make the session attended:

- `change_propose` with its ticket is refused, so it can't loop by proposing
  copies. `change_attend`, `change_note`, the read-only tools and
  `session_end` still work;
- its execution of an ADR stops, whatever the lifecycle amendment says;
- the owner resumes it on `/attend/session/<label>`, at the `confirm` tier.

**A copy is refused too.** A proposal whose scope and plan match an attended
change's is refused, naming it. The way back to that work is Resume or Hand
over.

**Windows and mandates pause on a pattern, not a change.** A window or a
mandate pauses, and pushes, when 2 changes it covered are attended, or as
today on a failed deploy, a rollback or a spent budget. The iOS ADR §5's "a
check fails twice on one head" pause becomes B5 and B1, which take out only
that change. That ADR, and the tiered-approvals ADR for mandates, are
amended to say so.

**What the session does on `attended`.** Its follower wakes it once. It:

1. stops work on that change, and on the ADR steps that depend on it;
2. calls `change_attend` with what it tried and what remains, if the breaker
   tripped first;
3. says the same in the chat, then carries on with independent work, if any,
   or stops with its remains, per the eviction ADR §6;
4. keeps its follower armed, so a Resume wakes it.

### 5. The owner's view

- **The digest gains a Sessions section:** each live session, whether it is
  listening, and its changes by stage, attended first.
- **`/approvals` shows the same,** and lists attended changes beside waiting
  approvals.
- **The app** gains them in `GET /v0/app/home` as a later step of the iOS
  ADR, which this ADR doesn't govern.

### Execution

Each step is one change.

1. **`feat(backplane)`: stages.** `core/change/stage.rs` and its tests;
   `stage`, `needs`, `next` and `since` on `change.status`, `change.list`
   and events; `session.status`. `Governs` gains `stage.rs`.
2. **`feat(backplane)`: subscriptions.** The family filter, the stored
   cursor, `after=session`, `wake=agent`, listening.
3. **`feat(tooling)`: the scripts and `CLAUDE.md`.** `follow.py`,
   `brief.py`, `statusline.py` and `stop.py` under `tooling/session/`;
   `CLAUDE.md`'s recipe, the `change_attend` rule and the notification rule.
4. **`chore(claude)`: `.claude/settings.json`,** with the three hooks and
   the status line. Guarded and in `secrets:`: the owner, with the code,
   never a mandate.
5. **`feat(backplane)`: the breaker, in shadow.** `breaker.rs`,
   fingerprints, and a `breaker.would_trip` event, with no effect, for 48
   hours. The thresholds are tuned from what it would have tripped, in an
   amendment here. `Governs` gains `breaker.rs`.
6. **`feat(backplane)`: attending.** `change_attend`, the `attended` stage,
   the front door's and `ci_rerun`'s refusals, the released turn, the push,
   `/attend/<n>` and its four answers, the session breaker and the copy
   refusal.
7. **`feat(backplane)`: Auto skips attended changes,** in `windows.rs` and
   `mandates.rs`, which are guarded: the owner. The 2-attended pause. The
   amendments to the iOS, tiered-approvals and yield ADRs.
8. **`feat(backplane)`: the Sessions digest and `/approvals`.**

Steps 1 to 3 need nothing else and come first. Step 6 builds B7 only once
the yield ADR's step 7 lands; until then B1 to B6 trip alone.

### Verification

- **Stages:** a replayed change's events answer each stage in turn, from
  `authorized` to `done`, with the `needs` of the table. A merged change
  whose deploy fails answers `deploy-failed`.
- **Subscriptions:**
  - a monitor killed mid-check and re-armed with `after=session` prints the
    events it missed, once;
  - a deploy carrying the session's merge reaches it, with no `change=` in
    its query;
  - with `wake=agent`, a change's run from push to `checked` wakes the
    session once, at `checked` or `failed`.
- **Listening:** a session whose follower stops shows as not listening in
  `session_status` and the digest within 2 minutes.
- **Harness:** after `/compact`, the session's next turn starts with its
  stages, and the status line matches `session_status`. A `Stop` with a
  failed change and no follower prints the reminder and stops.
- **Breaker:**
  - three heads failing `clippy` with one fingerprint trip B2 on the second;
    the third push is refused at the front door;
  - an Auto window running for two sessions: one change attended leaves the
    other session's changes merging under the window; a second attended
    change pauses it and pushes once;
  - a session with two attended changes is refused a third proposal, and a
    proposal copying an attended change is refused, naming it;
  - Resume reopens the branch, and the session's follower prints the
    owner's hint as a note.

## Amendments

### 2026-11-13: Step 1 splits in two; its first half lands on `change.status`

- **Paths.** `Governs` gains `core/change/stage.rs`.
- **Why two halves.** `change.status` reads the change and the merge queue,
  which is enough for every stage up to `merged`. `change.list` reads D1 rows
  without jobs; `deploying`, `done` and `deploy-failed` need `MainRun` to say
  which deploy carries a merge commit; events need the stage when they are
  appended; `since` needs the event log. Those come together in step 1b.
- **Step 1a, this change:**
  - `stage::of(change, place)`, pure, and `stage::place(queue, number)`;
  - `change.status` answers `stage`: `stage`, `needs`, `waits_on` for
    `needs: change`, and `next`;
  - until 1b, a merged change answers `merged`, `needs: none`.
- **Step 1b:** `since`; the deploy stages; `stage` on `change.list` and on
  every event; `session.status`; and `needs: platform` for `proposed` and
  `checked` when a window or mandate covers the change.
- **One case the table didn't have,** seen live on !322: an approved change
  whose merge job failed and whose turn was released, with no merge asked
  again. It answers `approved`, `needs: agent`, and `next` names the failure
  and says to ask to merge again.
