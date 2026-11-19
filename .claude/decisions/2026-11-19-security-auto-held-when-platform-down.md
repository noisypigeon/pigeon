## A failed deploy holds Auto only when no colour serves, or `main` keeps failing to deploy

* Date: 2026-11-19
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: security
* Governs: `src/**/approval/hold.rs`, `src/**/deploy/serving.rs`

## Context

The owner, 2026-11-19: "Can you seed a session so auto is not paused unless
both blue and green are down?"

### What holds Auto today

- [`2026-11-15-security-standing-auto-owner-flags.md`](2026-11-15-security-standing-auto-owner-flags.md)
  §3: any deploy of `main` that fails or rolls back holds Auto. While it is
  held, only `fix` and `revert` merges go through. A passed deploy resumes it.
  The hold pauses Auto at 6 hours or 3 failed deploys (`HOLD_MAX`,
  `HOLD_FAILURES` in `approval/mandates.rs`).
- `MainRun` (`worker/main_run.rs`) calls `mandates::hold_all` on every
  `Failed` report, whatever failed and whatever still serves.
- On 19 November, pigeon-green's first deploy (8e805a6, !1315) failed while
  binding, before any Worker shipped. pigeon-control and pigeon-front kept
  serving, yet Auto was held, and every routine merge waited on the owner.

### What blue-green says about health

[`2026-11-18-runtime-independent-blue-green-workers.md`](2026-11-18-runtime-independent-blue-green-workers.md):

- **The lease** (D1 `lease`, one row) names the colour that acts. Without a
  row, that colour is `pigeon-control` (`lease::FIRST_COLOR`).
- **A colour's `/healthz`** answers `ok` when D1 answers, its `Instance`
  answers, and its objects run its own version (`worker/mod.rs` `health`).
  Each colour has its own host: `pigeon-control` has
  `control.platform.pigeon.dev`, and the colours have
  `green.platform.pigeon.dev` and `blue.platform.pigeon.dev`.
- **`pigeon-front`** forwards each request to the lease's colour, and answers
  503 when the lease names a colour it has no binding for. Its `/healthz` is
  the lease colour's answer, reached through it (step 9a's amendment).
- **A Worker deploy goes to the standby,** verified on its own objects and
  promoted only once it passes. A failed health check rolls back, and a failed
  watch flips the lease back. A deploy that fails before promotion leaves the
  acting colour untouched (A.4).
- **`/healthz/parity`** compares a standby's replay with the primary. It
  decides whether a standby may be promoted, not whether a colour serves.

So a failed deploy usually leaves a healthy colour serving: the old version,
or the colour it never touched. Holding Auto on every failure blocks routine
work when nothing is down.

## Implementation plan.

### 1. What "the platform serves" means

The deploy job checks this at the end of every deploy of `main`, passed or
failed, after any rollback or flip back. It is the last thing the job does.

- **A colour is up** when `GET https://<its own host>/healthz` answers 200
  with `ok: true`. The version doesn't matter: an old version that serves is
  up. The colours are the Worker specs with `color: true`, plus
  `pigeon-control` until it retires.
- **The front forwards** when `GET https://<the platform host>/healthz`
  answers 200 with `ok: true`. The platform host is `PUBLIC_HOST`
  (`platform.pigeon.dev`). Through the front, that is the lease colour's own
  answer. Before the front holds that host, it is `pigeon-control`'s.
- **Each probe gets 3 tries, 10 seconds apart,** with a 10-second timeout. Any
  other answer is not up: an error, a timeout, a non-200, or `ok` not `true`.
- **The platform serves** when the front forwards and at least one colour is
  up. **It is down** otherwise:
  - **No colour is up.** This is the owner's "both blue and green are down".
    Before green exists, or while it is unhealthy, the only colour is
    `pigeon-control`, so "both down" means pigeon-control is down.
  - **The front doesn't forward.** Every session and the owner's app reach
    the platform only through the platform host. A front that answers
    nothing, or forwards to a dead lease colour, leaves nothing reachable,
    even with a standby up. A merge can't even be asked for then. A flip
    (`lease.flip`) or a passed deploy brings it back.
- **The report's `serving`** records what was found: `{lease, front, colors:
  {<worker>: bool}}`, typed as `deploy::serving::Serving`, whose `down()`
  says why the platform can't serve, if it can't.
- **Fail closed.** A report without `serving` counts as down. That covers a
  job image built before this ADR, a probe the job couldn't run, and a lost
  job's missing report. So until the job reports `serving`, every failure holds
  Auto, as it does today.

### 2. When a failed deploy holds Auto

`approval::hold::holds(serving, streak, pipeline, now)` returns why a failed
deploy holds Auto, or `None`. It sits in the gate (`approval/**`), because it
decides when Auto covers merges. `worker/mandates.rs` (also gate) calls it.
`MainRun` only passes it facts. It holds when any of these is true:

| Condition | Why it holds |
| --- | --- |
| `serving` is missing, or `down()` | The platform can't serve. This is the owner's rule. |
| The failed deploy's range touches the deploy pipeline (`pipeline`) | A broken pipeline may not ship its own fix, and every later merge rides on it. |
| 3 deploys of `main` in a row have failed (`STREAK_FAILURES`) | Fixing forward isn't converging. |
| `main` has gone 2 hours without a passed deploy since the streak's first failure (`STREAK_MAX`) | This bounds the undeployed backlog. |

- **The streak** is the run of `Failed` deploys at the head of `MainRun`'s
  history. `Superseded` deploys are skipped, and the run stops at the first
  `Passed` one. Its `failures` count includes this deploy, and `since` is when
  its first failure finished. `history` keeps 20 deploys, which is more than 3.
- **Why 3 failures.** A routine fix-forward lands in one or two deploys. A
  third failure in a row means the fixes aren't working, and Auto stacking
  more merges on top makes the culprit harder to find. That matches today's
  `HOLD_FAILURES`.
- **Why 2 hours.** On 15 November Auto merged about 18 changes an hour. Two
  hours of that is about 36 undeployed merges. Bisecting that many takes about
  six deploys, which is about what a person can still untangle. The rate
  breaker trips at 150 merges in 6 hours (25 an hour), so a busier stretch
  still meets this bound first. Four hours would allow about 70 merges, and
  bisecting those takes too long.
- **The pipeline paths**, `deploy::serving::PIPELINE`, are the code that runs
  a deploy:
  - `src/pigeon-core/src/deploy/**` and `src/pigeon-core/src/control.rs`;
  - `src/pigeon-agent/src/ship.rs`, `src/pigeon-agent/src/jobs.rs`,
    `src/pigeon-agent/src/workers/**` and
    `src/pigeon-agent/src/worker/main_run.rs`;
  - `Dockerfile`, `pigeon-start` and `.dockerignore`.

  The deploy job knows the range (`deployed..sha`), and reports `pipeline:
  true` when the range touches any of them. A change to the list is a change to
  this ADR.
- **Once a hold starts, today's rules apply unchanged.** Only `fix` and
  `revert` merges go through. A passed deploy resumes Auto. The hold pauses
  Auto after 6 hours or 3 more failures. The hold starts when it is decided,
  and counts the failure that started it as its first. So, at worst, Auto is
  paused after 3 + 2 failures, or after 2 + 6 hours.

### 3. A failure is still loud when Auto isn't held

- **Every failed deploy still sends a push and an event,** as `deploy_event`
  does today: `deploy.failed`, "Deploy failed", whatever Auto does.
- **The summary says what serves.** When Auto isn't held, `worker/mandates.rs`
  appends `auto.strained` and pushes "Deploy failed; Auto stays on", with the
  colours that are up, the failures so far, and which condition would hold
  Auto next ("holds at 3 failures or 2 hours").
- **The digest keeps saying it.** While `main` has a failing streak and Auto
  isn't held, each hourly digest carries one line in `held`: "main undeployable for <m> min: <n> failed
  deploys; Auto stays on until 3 failures or 2 hours", from `MainRun`'s
  history (`Streak::of`). That makes `needs_owner` true, so the digest is pushed every hour, not only in
  the hour the failure happened. Its `failed_deploys` still lists that hour's
  failures.
- **Holding and pausing** are told as before: `auto.held` and `auto.paused`,
  each pushed.

### 4. What doesn't change

- **Auto still never covers the owner tier** (secrets, destruction,
  break-glass, `change.front`, `change.pin`), **or the gate.** Guarded and
  sentinel paths outside the gate stay covered, per the standing-Auto ADR's
  2026-11-16 amendments. This ADR changes only when a deploy failure holds
  Auto, not what Auto covers.
- Mandates follow the same rule: `hold_all` holds every window and mandate,
  and `strain` holds none of them.
- The rate breaker, the per-change breaker and a pause by the owner are
  unchanged.
- A deploy that is lost (`MainRun::lost`, no report) holds nothing, as today.

### 5. Coordination with blue-green

This ADR reads blue-green's signals and changes none of them: the lease, each
colour's `/healthz`, the front's forwarding, and the deploy's verify, promote
and flip-back. The deploy job's probe is new. It runs after the deploy's last
step and reads only `/healthz`, which the deployer's token already reaches. It
touches neither `workers::bound` nor `infra/pigeon/workers/color/`. If
blue-green later names a better "serving" signal, such as a lease state the
front refuses, the change that builds it amends §1 here.

### Execution

1. **`feat(backplane)`, by this planner, with the owner's passkey (!1323):**
   - this ADR, the standing-Auto ADR's amendment, and `CLAUDE.md`'s Auto
     line;
   - `approval/hold.rs` with its tests, and `deploy/serving.rs`
     (`Serving`, `down()`, `PIPELINE`) with its tests;
   - `Report.serving` and `Report.pipeline`, and the streak from
     `Pipeline::history`;
   - `worker/mandates.rs` deciding through `holds`, with `auto.strained`, and
     `main_run.rs` passing it the report, the streak and `now`.

   It loosens an approval rule and touches the gate, so the owner approves it
   with a passkey. Until step 2 lands, no report carries `serving`, so every
   failure still holds.
2. **Brief, sonnet, `feat(deploy)`:**
   - the deploy job probes as §1 says, and fills `serving` and `pipeline`
     (`deploy::serving::PIPELINE` over `deployed..sha`), with a test of the
     probe on canned answers;
   - the digest's "main undeployable" line (§3) in `tasks.rs` and
     `worker/push.rs`, read from `MainRun`'s `/status` history.

   It adds no rule: `holds` already decides. A broken probe fails closed,
   because a missing or false `serving` holds.

### Verification

- A deploy that fails before shipping any Worker, while pigeon-control and the
  front serve, doesn't hold Auto. It pushes "Deploy failed; Auto stays on",
  and the digest repeats the line every hour.
- A third failure in a row, or 2 hours without a deploy, or a failure in a
  pipeline change, holds Auto as today.
- A deploy whose probe finds no colour up, or the front not forwarding, holds
  Auto.
- A report from an older job image, without `serving`, holds Auto.
- An owner-tier or gate change still waits for the passkey with Auto on.
