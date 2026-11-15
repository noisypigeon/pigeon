## Job Containers: a start that fails backs off instead of counting as lost, and a stalled change wakes itself

* Date: 2026-11-15
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development
* Governs: `src/pigeon-cli/crates/backplane/src/core/change/jobs.rs`, `src/pigeon-cli/crates/backplane/src/worker/change.rs`

## Context

A change's jobs run in `Checker` and `Runner` Containers, one Durable Object
per job ID
([`2026-11-05-runtime-control-plane-worker.md`](2026-11-05-runtime-control-plane-worker.md)
§3,
[`2026-11-06-development-control-plane-approvals-images-state.md`](2026-11-06-development-control-plane-approvals-images-state.md)
§4). A job whose Container is lost runs again, up to `MAX_ATTEMPTS` (3). A
merge or restack lost on every attempt gives its merge turn back
([`2026-11-08-development-concurrent-agent-sessions.md`](2026-11-08-development-concurrent-agent-sessions.md),
2026-11-11 and 2026-11-13 amendments).

### What happened on 2026-11-15

From the event log and `deploy_status`, all times UTC:

- **!461's merge job "lost its Container 3 times", three times.** Events
  17401, 17405 and 17407, at 01:00:43, 01:00:55 and 01:01:10. A fourth merge
  started at 01:03:25 on the same head and merged.
- **!464's restack job did the same once,** at 01:00:50 (event 17403). The
  session then rebased by hand, and the push reset the owner's approval. The
  owner approved again on efd44b8. The approval on a2ea33d, 25 s after the
  control plane's restack and with no `approval.pending`, was the clean-restack
  carry.
- **!461's bound job sat `queued` with 0 attempts from 00:16:25 to 00:41:47.**
  In that window other changes' bound, mirror, restack and merge jobs, all on
  `Runner`, started within seconds. Bounds on !421 and !431 earlier had
  waited hours, though !421's matches the `ios` alarm fault that !454 fixed.

### What is proved

- **Not the Worker redeploys.** `pigeon-control` was last deployed at
  00:48:39, 12 minutes before the first failure. None overlapped.
- **No Container was lost, because none started.** `ChangeRun::start_container`
  applies `Started` and then `Lost` whenever `start` returns an error that
  `control::no_capacity` doesn't recognise. The `Lost` queues the next attempt
  at once, so all three attempts ran in 5–15 s. The "9 Containers" were 9
  failed starts.
- **Only the jobs that mint an Artifacts write token failed.** In `start`,
  merge and restack call `platform::credentials(write: true)`. The read-token
  `Runner` jobs either side started: !461's mirror at 00:55:57 and !464's
  bound at 01:01:24. !175 failed the same way while the git front door returned
  502 (the concurrent-sessions ADR, 2026-11-11).
- **The reason is thrown away.** The start error reaches only
  `console_warn!` in Workers Logs. `change_status`, the event log and the digest
  say "lost", which reads as a Container fault.
- **A queued job can stall with capacity free.** `!461`'s 25 minutes. The
  alarm starts only what `PENDING` holds. Any `?` in the alarm drops the whole
  run, and nothing re-arms the alarm while one is still set. `ci_rerun` refuses
  a job on a head that isn't finished, so nothing can re-dispatch it.

### What is suspected, not proved

- **Which start step failed.** A write-token mint failing for about 30 s fits
  best. `credentials` retries only 250 ms and then 1 s. Runner capacity
  refused in words `no_capacity` doesn't match would look the same. Workers
  Logs' "job … did not start: …" lines from 01:00:30 to 01:01:15 would settle
  it. They need the dashboard, or `logs_query` once `OBSERVABILITY_TOKEN`
  lands
  ([`2026-11-10-runtime-platform-hostname-dashboard-observability.md`](2026-11-10-runtime-platform-hostname-dashboard-observability.md)).
  The decisions below hold either way.
- **Why !461's alarm stalled.** It resumed 84 s after the 00:40:23 deploy.
  That fits an alarm that kept failing on the older code, but nothing
  recorded it.

## Implementation plan.

### 1. A start that fails is not a lost Container

- **A new event, `StartFailed { job, sha, why }`,** replaces `Started` then
  `Lost` when `start` fails for any reason but capacity, which waits as today.
  - The job stays `queued`, and its `attempts` are unchanged.
  - Its `start_failures` rises by one, and its `last_error` is `why`, cut to
    300 characters. Errors from `credentials` never carry the token.
  - It starts again after a backoff of 15 s, 30 s, 60 s, 120 s, then 240 s.
- **The 8th failed start fails the job as a lost one does today,** with the
  violation "the <job> job's Container failed to start 8 times over 16
  minutes: <last_error>". A restack's ends as a failed restack, and a merge's
  gives its turn back.
- **A queued merge or restack already holds its turn**
  (`core::change::queue::holds_turn`), so the turn is kept through the backoff.
  No session or owner acts in that time.

### 2. A Container lost after it started

- `Lost` keeps its 3 attempts. The second starts 30 s after the loss, and the
  third 60 s after.
- A Worker deploy that kills a running job shows as `Lost`, with the deploy's
  time beside it in the event log. If that is ever seen, a drain is decided
  then. None is built now.

### 3. The reason is kept

- **`change_status`** shows each job's `start_failures` and `last_error`.
- **A `job.retrying` event** records each failed start and each loss, with
  the reason and the next try's time. It doesn't wake the session
  (`wake=agent`); the final `job.failed` does, as today.
- **The digest** gets one line when the past hour had failed starts or lost
  Containers: their count by job and the last reason. It is never pushed for
  that line alone.

### 4. A stalled change wakes itself

- **One failing step doesn't drop the alarm.** Each pending start or stop
  that errs is logged, kept in `PENDING`, and retried on the next alarm. The
  alarm still saves what the other steps did.
- **While any job is `queued` or `running`, the alarm is set within 60 s.**
  `rearm` sets it then, also when a later alarm is already set.
- **`ci_rerun` re-dispatches a job queued with 0 attempts for 2 minutes or
  more,** on any head, by queueing its start again. A queued or running job
  with attempts is still refused, as before.

### 5. Not decided here

- **An approval carried across a session's own rebase** whose own lines are
  unchanged. Session_015XFA1kiQE9H6hakkC6p7H8's approvals work owns the carry
  rule. This ADR makes the forced hand rebase rare instead.
- **Draining deploys, and job identities that survive a version change.** No
  evidence shows either is needed. §2's events would show it.
- **Freeing a never-polled Container,** such as `unmirror`'s. Nothing shows it
  held capacity: Runner jobs started in seconds throughout.

### Execution

1. This ADR.
2. `fix(backplane)`, brief for `sonnet`: `StartFailed`, `start_failures`,
   `last_error`, the backoff and the 8-failure end in `core::change`, with
   tests that replay !461's three failed starts and !464's restack.
3. `fix(backplane)`, brief for `sonnet`: `ChangeRun` applies `StartFailed`
   and keeps the backoff's due time in `PENDING`. It adds the 30 s and 60 s
   spacing after `Lost`, §4's alarm, and the `job.retrying` event.
4. `feat(backplane)`, brief for `sonnet`: `ci_rerun` on a job queued with 0
   attempts, and the digest line.

### Verification

- !461's sequence replayed in `core::change` tests: three failed starts keep
  the merge queued, with attempts 0, and the fourth starts it.
- Live: the next failed start shows `job.retrying` with its reason, and the
  merge keeps its turn.
- Live: a queued job with no `PENDING` start begins within 60 s, or by
  `ci_rerun` after 2 minutes.

## Amendments

### 2026-11-15: The steps as drafted: step 2 in two, a start only of a queued job, and `ci_rerun` with no wait

Each step was written and tested in full before its brief.

- **Step 2 is two changes,** each under the change policy's 200 lines:
  - **2a** (brief b13, !475): `StartFailed`, `StartAfter`, `Lost`'s spacing,
    and `control::merge_ended` ending a merge's turn on its last failed
    start, as on its last loss.
  - **2b:** the core table tests, and `job.retrying` in `events::of_change`.
  - Until step 3, the worker starts a `StartAfter` at once.
- **Step 3 also starts only a job still `queued`.** A duplicate start would
  post the job to its Container again, and run a merge twice.
  - A poll that errs is logged and asked again next alarm.
  - The alarm sits at the sooner of 20 s and the next due start.
- **Step 4's `ci_rerun` re-dispatches a job queued with 0 attempts at once,**
  not after 2 minutes. A start already pending is not doubled, and a job that
  started is refused as before, so the wait guarded nothing. The digest line is
  `Digest::containers`.
- **Executors install Rust 1.99.1 with rustup.** `mise.run` is refused from
  their containers.
- **`pigeon-control` doesn't deploy now.** `wrangler deploy` refuses the
  `GRANTER` Secrets Store binding (code 10021). Steps 3 and 4 reach production
  once that is fixed, which is not this ADR's.

### 2026-11-15: A start queued while an alarm saves is not lost, and a job left queued is dispatched again

#### What happened

Reported by the approvals session (session_015XFA1kiQE9H6hakkC6p7H8):

- **!506 forecast-yielded to !500.** When !500 merged, the yield ended and
  !506's restack job was queued.
- **The restack sat `queued` with 0 attempts for about 55 minutes,** with no
  `job.started`.
- **The session's rebased push, 33cb1a5, was held behind the open restack.**
- **Neither §4's alarm nor !525's re-dispatch caught it.** A manual
  `ci_rerun job: restack` started it at once. So the job had no `PENDING`
  start, and capacity was free.

#### Cause: fits best, not proved

- **`save_after` wrote `CHANGE`, then awaited D1, events, grants, the breaker
  and the merge queue, and only then wrote `PENDING`.**
- **The yield's end arms !506's alarm.** `release_yields` posts `/yield`
  `off`, and that save sets the alarm. The merge queue's `Merge` reaches !506
  moments later, through `merges::ended`.
- **If that alarm is inside those awaits when `Merge` lands,** `on_event`
  queues the restack and its `Start`. The alarm then writes its own `PENDING`
  over them.
- **The change keeps a queued restack with nothing pending.** The alarm
  starts only what `PENDING` holds, so §4's 60 s re-arm moves nothing.
- **A second gap of the same kind:** an event applied while `forecast_step`
  awaits is dropped from the alarm's change when the journal is cleared.

#### Decisions

- **`PENDING` is written with `CHANGE`, before any await in `save_after`.** A
  start queued meanwhile then reads and extends the alarm's list.
- **The alarm catches up on the journal again after `forecast_step`.**
- **Each alarm dispatches every queued job with no `Start` or `StartAt`
  pending** (`changes::undispatched`), whatever path queued it. So any queued
  job starts within the alarm bound, 60 s.
  - Jobs that GitHub Actions holds (`Actions::owns`), and a mirrored head's
    `ios` check, are excluded.
  - `start_container` still starts only a job that is `queued`, so the
    dispatch is never doubled.
- **The guard: a job queued with 0 attempts and no failed start for 5
  minutes** (`STALL_SECS`, kept per head and job in `QUEUED`) has its pending
  start replaced by one now. Its time then restarts.
  - A job in a start backoff is excluded, since `job.retrying` already
    reports it.
  - A job waiting for capacity is dispatched again too, which changes
    nothing but is named.
- **Each re-dispatch is a `job.redispatched` event,** which doesn't wake the
  session.
- **`Digest::containers` counts these events beside failed starts and
  losses, and names the last.** It is never pushed for that line alone, as
  before. The line is built in `src/pigeon-cli/crates/backplane/src/tasks.rs`.

#### Execution

The planner wrote and tested each step itself. A race in a Durable Object's
storage leaves an executor nothing to follow to the letter.

1. This amendment.
2. `fix(backplane)`: `undispatched`, `stalled` and `STALL_SECS` in
   `core::change::jobs`, with core tests that replay !506. The tests cover the
   yield, `main` moving, the yield ending, the restack queued and its start
   lost. They also cover the `job.redispatched` digest line.
3. `fix(backplane)`: `ChangeRun` writes `PENDING` with `CHANGE`, catches up
   after `forecast_step`, and calls `redispatch` at the top of each alarm.

#### Verification

- Core tests: a restack queued as a yield ends is started. With its start
  lost, it is found and dispatched again. One held by GitHub is not.
- Live: no job sits `queued` with 0 attempts beyond 60 s with capacity free.
  If one does, `job.redispatched` names it within 5 minutes, and the digest
  names it too.
