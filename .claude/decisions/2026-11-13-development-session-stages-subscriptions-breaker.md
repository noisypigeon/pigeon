## Where a session is: one stage per change, subscriptions held by the control plane, the harness that shows them, and a breaker that takes a stuck change out of Auto

* Date: 2026-11-13
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development
* Governs: `src/pigeon-cli/crates/backplane/src/core/change/lifecycle.rs`, `src/pigeon-cli/crates/backplane/src/core/change/stage.rs`, `src/pigeon-cli/crates/backplane/src/worker/stages.rs`, `src/pigeon-cli/crates/backplane/src/core/change/breaker.rs`, `src/pigeon-cli/crates/backplane/src/core/approval/windows.rs`, `src/pigeon-cli/crates/backplane/src/sessions.rs`, `src/pigeon-cli/crates/backplane/src/worker/tasks.rs`, `tooling/session/session.py`, `.claude/settings.json`

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

### 2026-11-13: Step 1b lands on `change.status` and `session.status`; lists and events are step 1c

- **Paths.** `Governs` gains `worker/stages.rs`.
- **A window or mandate acts only when asked.** Seen on !312 and !322: a
  change Auto window 1 covered sat at `checked` until the session called
  `change_approve`, and was then approved as `window:1`. So a covered
  `proposed` or `checked` change answers `needs: agent`, with `next` naming
  the tool to call and the window or mandate that allows it, not
  `needs: platform` as §1's table had it. An uncovered one answers
  `needs: owner`. Coverage is asked of the window with the same `decide` the
  operation would use, which spends nothing for an authorization or an
  approval.
- **The deploy carrying a merge, without git.** `MainRun`'s `Pipeline` keeps
  the last 100 pushes to `main` in order (`pushes`, not shown by
  `deploy.status`). A deploy of a push at or after the merge's carries it:
  - `done` once the last deploy that passed carries it;
  - `deploying` while a deploy carrying it runs, or before one starts;
  - `deploy-failed`, `needs: owner`, when a deploy carrying it failed and none
    has passed since;
  - `merged` for a merge older than the pushes kept, as before.
- **`since`** is the time of the change's last event that isn't a job's.
- **`session.status` (MCP `session_status`)** answers each open change of a
  session, oldest first, with its stage, and the session's tasks. Listening
  comes with step 2.
- **Step 1c:** `stage` on `change.list` and on each event. A list reads D1
  rows without jobs, and an event is appended by `ChangeRun`, which doesn't
  hold the queue, so each needs its own source of the stage.

### 2026-11-13: Step 1c lands, and step 1 is done

- **Each event a change produces carries its stage** as it was appended:
  `Event.stage`, read as none from an event logged before, as the events
  ADR's log keeps them. It has no `since`; the event's `at` is that time.
- **`change.list`'s rows carry `stage`.** An open change's is derived in full,
  as `change.status` derives it; a merged or closed change's is its state.
  `since` is the row's last update.
- **`session.status` lists only the session's tasks:** those for it first,
  or claimed by it. It listed every open task before, 24 of them from other
  sessions, back to !81.
- **A bounded head whose checks aren't listed yet** answers `checking`, not
  `checking 0/0`.
- **Seen live after step 1b:** the deploy of !340 failed on Cloudflare's 500,
  and the next push's deploy carried it. Its stage answers `merged`, not
  `done`, because its push reached the code from before `pushes`; merges from
  now on follow their deploy.

### 2026-11-13: Step 2 lands: cursors, `wake=agent` and listening

- **The family filter needed nothing new.** A generated `reconcile-<n>` or
  `review-<n>-<m>` change takes its parent's session, a handed change takes
  its new session, and an event of no session, such as a deploy's, reaches
  every session. So `session=<label>` already reads the family. A change
  yielded to another session's change sees only its own `change.yielded` and
  `change.unyielded`, not the other change's events.
- **The cursor is the control plane's.** The `Instance` keeps, per label, the
  highest numbered `after` it polled with and the time of its last poll
  (`events::Cursors`, under `event-cursors`, dropped a week after its last
  poll).
  - `after=session` resumes from it; a label never seen starts at the log's
    last event, not at its start.
  - A numbered `after` moves it forward. The follower sends the last id it
    printed, so a lost answer is read again: at least once, never lost.
- **`wake=agent`** answers only what the agent acts on (`events::wakes`): an
  event whose stage needs the agent, or is `done`, `closed`, `deploy-failed`
  or `attended`; or one of the kinds that need the agent whatever their stage:
  merged, closed, conflicted, evicted, handed, a note, yielded and unyielded,
  a released merge turn, a task made, a deploy's end, an Auto window paused,
  and a replaced ticket.
- **Listening.** `session.status` answers `events`: the cursor, its last poll
  and `listening`, a poll within two minutes. The digest's Sessions line waits
  for step 8.
- **Step 3** writes `tooling/session/follow.py` on these:
  `after=session` on (re)arm, then the last id, with `wake=agent`.

### 2026-11-13: Step 3 lands: one script, `tooling/session/session.py`

- **One stdlib script with four commands,** not four scripts: `follow`,
  `brief`, `statusline` and `stop`. `python3 -I` keeps the script's own
  directory off the import path, so four scripts couldn't share their code.
  - `follow` long-polls `after=session&wake=agent`, then the last id it
    printed, and prints `<id> !<n> <stage> needs=<who> <kind>: <summary>`.
    After each answer it saves `session.status` to `~/.cache/pigeon/status.json`.
    Five failed polls in a row print one line, and it keeps trying.
  - `brief` prints the session's changes, those needing the agent first, its
    tasks, and whether it follows events; with no
    `~/.config/pigeon/session.json`, CLAUDE.md's start steps.
  - `statusline` reads only the cache.
  - `stop` prints a `systemMessage` when a change needs the session and it
    isn't listening, and never blocks.
  - Every command but `follow` exits 0 on any error.
- **CLAUDE.md** says to write the session file after `session_start`, to arm
  `Monitor` with `follow`, what `stage` and `session_status` answer, and who
  pushes to the owner.
- **The `change_attend` rule waits for step 6,** which adds the tool.
- **Found while following !346:** an approval's event carries the stage from
  just before its merge is queued, `approved`, `needs: agent`, "ask to
  merge", so it wakes the session once for nothing. A later change computes
  the stage after queuing.

### 2026-11-13: What following its own changes showed, and three fixes

This session followed !347 and !354 with `session.py follow` from the moment
step 2 was live.

- **It woke when it had to act, and only then, once two leaks were closed:**
  - **job events** carried their change's stage, so a stage needing the
    agent woke the session once per job as well. !354: `wakes` never answers
    a `job.*` event; a job's end that matters comes with its change's own.
    An approval still wakes: after a restack whose merge isn't asked again, it
    is the only signal;
  - **`merge.released` and `change.note` were logged with no session,** so
    they reached and woke every session: this one was woken for another
    session's !349. They now carry the change's session. `merge.skipped`
    doesn't wake, so it stays as it is.
- **!347's merge was refused twice as behind, and nothing restacked it.** The
  concurrent-sessions ADR's 2026-11-12 amendment says a merge refused while
  behind restacks the head; on !347 it didn't. `session.status` named it at
  once (`approved`, `needs: agent`, "ask to merge again: main has moved past
  its base"). Asking again was refused the same way, so the session rebased
  and pushed the change itself rather than ask a third time, which is the
  breaker's rule (§4) kept by hand until step 6. Why the restack didn't fire
  is left to the concurrent-sessions ADR.

### 2026-11-13: Step 4: `.claude/settings.json`

- **Paths.** `Governs` gains `tooling/session/session.py` and
  `.claude/settings.json`.
- **The settings** run `session.py` from `$CLAUDE_PROJECT_DIR`, with
  `python3 -I`:
  - `SessionStart`, matching `startup|resume|compact`: `brief`, with a
    20-second timeout. `brief` asks the control plane for at most 10 seconds,
    then falls back to the cache, so a slow control plane never holds up a
    session's start;
  - `Stop`: `stop`, with a 10-second timeout, which never answers `block`;
  - `statusLine`: `statusline`, which reads only the cache.
- **Nothing in them grants or decides anything.** They show the control
  plane's answers to the session and to whoever watches its terminal; the
  approvals ADR's 2026-11-06 amendment holds, since the agent's own
  repository writes them.
- **The file is guarded and in `secrets:`,** so this change is the owner's,
  with the code, and no window or mandate covers it.

### 2026-11-13: Step 5 lands: the breaker in shadow

- **Paths.** `Governs` gains `core/change/breaker.rs`.
- **What it counts.** `ChangeRun` keeps a `Breaker` beside each change (under
  `breaker`): its last 10 heads, each with the checks that failed on it, their
  fingerprints (from the failure log tails `ChangeRun` already keeps), the
  policy that refused it, each job's dispatches and whether it reached
  `checked`. Every save updates it (`breaker::observe`).
- **The signals**, with §4's thresholds as constants: B1 (one check failing
  on 3 heads in a row), B2 (one fingerprint on 2 heads), B3 (one policy
  refusing 3 heads in a row), B4 (8 heads since one reached `checked`), B5
  (a job dispatched 3 times on one head), B6 (3 or more unchecked heads over
  3 hours). B7 waits for the yield ADR's loop breaker.
- **A fingerprint** is the tail's first error line (one that starts with
  `error`, or holds `panicked` or ` failed`), else its last line, with digits
  as `#`, hashes as `<hash>` and paths as `<path>`.
- **Shadow.** Each signal is logged once as `breaker.would_trip`, under the
  change's session: "!n would leave Auto (B2): …". It doesn't wake a
  session, and nothing acts on it. After 48 hours the thresholds are tuned
  from what it logged, in an amendment, and step 6 makes it act.
- **B5 counts a lost Container's redispatch** as a run, as `attempts` does. If
  the shadow shows that tripping on platform failures, B5 counts only
  `ci_rerun`s.
- **A passed deploy no longer wakes every session.** Deploy events carry no
  session, so each passed deploy woke every listening session. A failed one
  still does. A session sees its own merge reach `done` in its stage.

### 2026-11-13: Long polls fit under a 30-second cut, and step 8: sessions in the digest

- **A request is cut at about 30 seconds on its way to the control plane,**
  answering 502 "upstream request failed". Measured from a session:
  `seconds=25` answers at 25.4 s, `seconds=29` and `35` fail at 30.2 s.
  `events.wait` and `task.wait` waited up to 50 seconds, so each poll with
  nothing new failed: the follower's "5 failed polls in a row" all day, for
  every session. Another session reported it in a note on !346. !365: both
  wait at most 25 seconds; `session.py` asks for `seconds=25` (it had sent
  `wait=50`, which nothing reads).
- **A cursor never passes the log's last event** (!365). A probe with
  `after=999999` had moved this session's cursor out of reach of every later
  event; one past the end is now pulled back.
- **Step 8.** The digest gains `sessions`: each session with open changes,
  whether it listens, and each change's stage and who it needs, those needing
  the agent first. `lines()` shows them, so `/approvals` and `digest_show` do;
  `owner_lines()` leaves them out, and the hourly push hashes and sends only
  those, so a session's movement alone never pushes. Attended changes join it
  with step 6.
- **Merges by an Auto window now reach the digest.** "Merged without you"
  counted approvals by `mandate:`, `policy:` and `grant:`, not `window:`, so
  no window merge was listed, though the iOS ADR §5 says each is.
### 2026-11-13: The follower and the hooks move to `pigeon session`

- [`2026-11-13-development-rust-session-event-follower.md`](2026-11-13-development-rust-session-event-follower.md)
  supersedes §2's follower script and §3's scripts: `pigeon session
  follow|brief|statusline|stop`, run through `tooling/session/session.sh`,
  which falls back to `session.py`.
- What §2 and §3 show, and when, stays. The follower prints the same lines.

### 2026-11-13: A queued, yielded change waits on the change it yields to

- **`stage::approved`** answers a queued change that yields `queued k/n`,
  `needs: change`, `waits_on` the change it yields to, "it keeps its place",
  not the turn's holder. The queue now keeps a yielded change's place (the
  yield ADR's 2026-11-13 amendment on the queue).
- **The approved arm moved into its own function,** so `stage::of` stays under
  clippy's line limit.

### 2026-11-13: The digest names changes stalled on a session that isn't listening, and pushes them

- **What went wrong.** !364, !371 and !373, and !313, !321 and !324, each
  waited on a session that wasn't listening. The breaker couldn't see it,
  since nothing failed. Step 8's Sessions section showed "not listening", but
  never pushed. So nobody acted, and the changes yielded to them waited too.
- **A stall** is an open change that needs the agent while its session hasn't
  polled for `STALL` (30 minutes), or never has (`tasks::stalled`).
- **The digest lists each stall** (`Digest.stalled`), with:
  - its stage and what its session must do;
  - how long the session has been quiet;
  - the open changes yielded to it (`/yields/list`);
  - the owner's way out: start a session to take it over, or close it.
- **Stalls are in `owner_lines()` and `needs_owner()`.** Nothing automatic
  moves a stalled change, so the hourly push carries them. The push sends a
  set of lines once, as before.
- **This is not the breaker,** and doesn't take the change out of Auto. A
  session that polls again clears its stalls at the next digest.
- **Execution:** one `feat(backplane)` change, built on the queue fix and the
  generated-work fix (the yield ADR's and the concurrent-sessions ADR's
  2026-11-13 amendments).

### 2026-11-14: A window or mandate the owner granted is their approval, and a change that needs its session is nudged until it acts

- **What went wrong.** !419, a planner's ADR change, reached `checked` at
  about 06:10 UTC on 2026-11-14 with `needs: agent`, `next` "ask
  (`change_approve`); window:6 allows it". Window 6 was the owner's Auto
  window. The planner's seed said to stop for the owner's approval of the
  design, and it read that as excluding the window. It never asked, and sat
  idle for about 12 hours while its follower stayed armed. Its follower
  delivered nothing: `wakes` answers the one event that moved the change into
  its stage, and nothing after it until main moved (`change.behind`).
- **The rule.** A session stops only when `needs` is `owner`. When `needs`
  is `agent`, it does `next` now. Any other `needs` waits on someone else, and
  the follower says when that changes.
  - An Auto window or a mandate the owner opened is the owner's approval of
    what it covers.
  - A seed's or brief's "stop for", or "wait for", "the owner's approval" is
    met when `next` names a `window:<id>` or `mandate:<id>`. The exception is
    a seed or brief that says "even under a window" (or "mandate").
  - `CLAUDE.md` says so in its Rules.
- **`next` says it.** `stage::ask` answers "ask now (`<tool>`): `<by>`, the
  owner's, approves it", in place of "ask (`<tool>`); `<by>` allows it".
- **A nudge.** `change.nudged` re-wakes a session that missed or misread its
  change's transition.
  - **When.** Each hourly run (`scheduled`) nudges every open change whose
    stage needs the agent and has done so for at least `NUDGE_AFTER`
    (15 minutes) (`events::due`).
  - **What.** The event carries the stage, so `wake=agent` answers it. Its
    summary reads "!n has needed its session for 2h10m: `<next>`". It goes to
    the session whether it listens or not. The digest's stalls stay as they
    are, for a session that doesn't listen.
  - **At most one per change per hour.** The `Instance` appends it only when
    the log holds no `change.nudged` for that change within the last
    `NUDGE_EVERY` (an hour) (`Log::nudge`). So a doubled cron run nudges once.
  - **A nudge isn't a move.** `since` skips `change.nudged`, as it skips a
    job's events (`events::moves`), so the time waited keeps growing.
  - **The hourly cron sets the delay.** A change is first nudged 15 to
    75 minutes after it starts needing the agent. A finer cron would run the
    digest, insights and evictions more often too, so this stays hourly.
- **Execution.**
  1. This amendment and `CLAUDE.md`'s line, as one `docs(claude)` change.
  2. One `feat(backplane)` change, after it, with:
     - `events::{due, moves, NUDGE_AFTER, NUDGE_EVERY}` and `Log::nudge`,
       tested in the core;
     - `stage::ask`'s text;
     - `stages::since` skipping nudges;
     - `/events/nudge` on the `Instance`;
     - `events::nudge`, called from `scheduled`.

### 2026-11-14: The settings let a session act on the owner's grant, and ask before destructive actions

- **Why.** The briefs ADR's amendment of the same date sets the rule: under
  the owner's window or mandate, a brief runs end to end, and only
  destructive actions stop. Claude Code's auto mode put its own classifier
  in front of the pigeon tools, and it refused `change_authorize` under
  window 6. Its documentation says a call matching an allow, ask or deny rule
  resolves before the classifier runs. Auto mode drops only broad
  code-execution allow rules, and an MCP tool's rule isn't one. A cloud
  session with one repository reads the committed `.claude/settings.json`.
- **`permissions.allow`** names each pigeon tool whose effect the control
  plane itself bounds by a window, mandate, tier or review. It doesn't use a
  wildcard, so a new tool is asked about until it's listed:
  - changes: `change_propose`, `change_authorize`, `change_approve`,
    `change_merge`, `change_pushed`, `change_status`, `change_list`,
    `change_scope`, `change_note`, `change_handoff`, `change_yield`,
    `change_evictable`, `ci_rerun`, `approval_wait`;
  - briefs, tasks and sessions: `brief_create`, `brief_amend`,
    `brief_review`, `brief_escalate`, `brief_show`, `brief_list`, `task_claim`,
    `task_done`, `task_list`, `task_wait`,
    `session_start`, `session_status`, `session_end`, `events_wait`;
  - reads and proposals the owner grants: `mandate_propose`,
    `mandate_status`, `mandate_end`, `deploy_status`, `digest_show`,
    `instance_show`, `architecture_describe`, `policy_check`,
    `platform_status`, `ios_status`, `infra_validate`, `sites_build`.
- **`permissions.ask`** names the destructive ones, so they prompt even under
  auto mode: `change_close`, `change_pin`, `grant_revoke`, `grant_rotate`,
  `grant_request`, `git_tokens`, `platform_apply`, `infra_apply`,
  `deploy_retry`, `sites_publish`, `ios_release`. Tools in neither list,
  such as `ci_window`, stay with the classifier.
- **The control plane still decides.** An allowed call that the window,
  mandate or tier doesn't cover is refused or asks the owner, as before.
  `change_authorize` is never by its proposer, and a brief's head awaiting
  review refuses windows and mandates.
- **`next` names the grant.** `stage::ask` names the window's or mandate's
  grantor and its expiry: "ask now (`change_authorize`): window:6, the
  owner's, until 19:40 UTC, approves it". The session building the
  2026-11-14 amendment's `stage::ask` text adds it, so `next` has one
  author.
- **This step is the owner's.** The file is guarded and in `secrets:`. The
  owner approves the settings with their passkey, and no window or mandate
  covers them, as for step 4.
- **Execution.**
  1. This amendment, with the briefs ADR's, as one `docs(adr)` change.
  2. One `chore(claude)` change with the settings, approved by the owner.
  3. A haiku executor, seeded on a one-step brief, calls `change_authorize`
     under a window. That shows auto mode no longer refuses it.

### 2026-11-14: An Auto window re-approves a restacked head, unasked

- **The owner decided** that Auto windows should re-approve restacked
  changes automatically. The decision was relayed by the session that fixed
  the restack loop, and the owner authorizes this change with their code.
- **What happens.** A restack whose approval doesn't carry leaves the change
  `checked`, unapproved, with its parked merge dropped, so its turn goes
  back (`merge.released`). Now, in the same event, `ChangeRun` asks whether
  the head would be approved now:
  - first by policy (`approvals::by_policy`);
  - then by the Auto window (`window_approves`, the same `decide` an asked
    approval uses).

  If one would, it applies `Approve` as `policy:<rule>` or `window:<n>`, then
  the parked `Merge`, and journals both. The change keeps its turn, and no
  `merge.released` is logged.
- **It is a narrow exception** to the 2026-11-13 amendment's "a window or
  mandate acts only when asked": only for a merge already asked, whose
  approval a restack took. A head that first reaches `checked` is still
  approved only when asked.
- **What still refuses it:**
  - a paused window, or one that doesn't cover the head's tier, answers none,
    and the merge drops as before;
  - the model-tiered briefs ADR's review gate: a brief's head awaiting its
    planner's review is refused, so it drops, and comes back once accepted.
- **Mandates aren't asked yet:** a mandate's coverage is per change and
  per operation in `mandates.rs`, which is guarded. Re-approving under a
  mandate is a separate, owner-tier change.
