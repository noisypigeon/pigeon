## A session that ends clean is archived by the session that created it, and the owner sees each archive

* Date: 2026-11-15
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development
* Governs: `src/**/worker/tickets.rs`, `src/**/tickets.rs`, `src/**/sessions.rs`, `src/**/worker/push.rs`

## Context

Finished sessions stay idle in Claude Code Remote until someone archives them.
On the night of 14–15 Nov the owner had a status session list and archive about
40 by hand. Most were executors (b1–b17) whose change had merged or whose
planner had taken the brief back. The rest were planners whose ADR was done.

**What exists (read from the code at `3811916`).**

- `session.end` (`worker/tickets.rs::end`) revokes the label's ticket. It
  appends `session.ended` the first time, and answers `holds`: each open change
  whose `session` is the label, with what it has left. That follows
  [`2026-11-10-development-change-slot-eviction-policy.md`](2026-11-10-development-change-slot-eviction-policy.md)
  §6.
- The ended ticket stays in `Tickets`, lapsed, for `REMEMBER` (7 days), with
  what the session declared (`Declared`: class, planner, brief).
- `Declared.planner` is set only for executors
  ([`2026-11-13-development-model-tiered-agent-briefs.md`](2026-11-13-development-model-tiered-agent-briefs.md)
  §1). The control plane doesn't know which session created a planner.
- `change.handoff` and a brief's `take` add a `handoff` task for the
  recipient. The change stays the offerer's until the recipient claims it.
- An event reaches a follower when its `session` is the follower's label or
  none. With `wake=agent` its kind must also be in `WAKING`
  ([`2026-11-13-development-session-stages-subscriptions-breaker.md`](2026-11-13-development-session-stages-subscriptions-breaker.md)
  §2). `session.ended` isn't in `WAKING`.

**What Claude Code Remote offers (checked in this ADR's session, 15 Nov).**

- This session is seeded (`origin: claude_code_mcp_seed`, with a
  `parent_session_id`), and its tools include `archive_session`. The earlier
  finding that seeded sessions lack it doesn't hold here. Whether every
  executor has it is unverified, and nothing below relies on it.
- `get_session` answers a session's `origin`, `parent_session_id` and
  `status_bucket`. Called with no ID, it describes the caller, so a session can
  learn its own parent.
- Self-archive wasn't tried. It would release the container mid-turn and lose
  the session's last message, which the eviction ADR requires.

**The options.**

- **(a) The control plane archives, through Anthropic's API.** There is no
  credential scoped to archiving. `archive_session` acts as the owner's
  claude.ai account. A token that let the Worker archive could also create
  sessions and read transcripts across the account. That contradicts
  `CLAUDE.md`'s rule that a workload gets only its own secrets and minted
  tokens. Not taken.
- **(b) The creator archives, on an event.** Planners already follow their
  events, and Claude Code Remote records each session's parent, which gives a
  check the control plane can't forge. Taken.
- **(c) (b) now, (a) later.** (a) comes back only by an amendment here, once
  Anthropic offers a credential that can do no more than read and archive the
  account's sessions. The owner's step then:
  `pigeon cobot workers secrets put` of that token for the `control` Worker,
  as `SESSIONS_ARCHIVE_TOKEN`, plus its API host in the Worker's allowed hosts.
  Nothing is provisioned now.

## Implementation plan.

### 1. A session names its creator

- `session.start` takes `parent`: the label of the session that created this
  one, `https://claude.ai/code/<parent_session_id>`, from the session's own
  `get_session`. Any class may name one. `Declared` gains `parent`.
- A session's **archiver** is its `parent`, or else, for an executor, its
  `planner`. A session with neither has no archiver.
- The owner's own sessions (`ios`, `desktop_app`, web) weren't created by
  another session, so they never name a parent and are never archived.

### 2. A clean end, decided by the control plane

- `session.end` judges the end **clean** when both hold:
  - the call ended a live ticket (a second `session.end` judges nothing);
  - every open change the label holds has an open, unclaimed `handoff` task
    for another session. With nothing held, that's trivially true.

  A change waiting on the owner, an escalation, or a change with no hand-off
  makes the end unclean. Unclean ends are never archived.
- The pure decision is `crate::tickets::clean(label, held, tasks)`, in
  `tickets.rs`, with tests. The lapsed `Issued` records the end:
  `ended: {at, clean, archiver}`.
- `session.end` also answers `clean` and `archiver`.
- On a clean end with an archiver, it appends `session.archivable`, with
  `session` set to the archiver's label. Its summary names the ended label and
  its session ID. `session.archivable` joins `WAKING`, so the archiver's
  follower prints it.
- `session.ended` is unchanged. Its summary says "clean" or "with work left".

### 3. The archiver acts, then reports

On `session.archivable` for label L, with session ID X, the archiver:

1. calls `get_session(X)`. It archives only when all of these hold:
   - X's `parent_session_id` is its own ID;
   - X's `origin` isn't `ios` or `desktop_app`;
   - X isn't `working`.

   While X is still working, it is writing its last message. The archiver
   checks once more within 10 minutes (`send_later`). If X is still working
   then, the archiver skips it.
2. calls `archive_session(X)`;
3. calls `session.archived` with `label` L, `outcome` (`archived` or
   `skipped`), `why` and its own ticket.

`session.archived` is accepted only from L's archiver, holding its live
ticket, after L ended clean. It records `archived: {at, by, outcome, why}` on
L's lapsed `Issued`, and appends `session.archived`. A session never archives
itself, and never archives one it didn't create.

### 4. The owner sees each archive

The digest gains two shown, never-pushed lists. Neither changes `needs_owner`:

- **`archived`:** each `session.archived` since the last digest, as
  `archived: <X> (<class>) by <archiver's ID>: <why>`. A skip shows as
  `not archived: …`.
- **`unarchived`:** each label that ended clean between 1 and 24 hours ago,
  with no `archived` record, as `ended clean, not archived: <X> (archiver <ID>
  or none)`. This catches an archiver that had already ended, and a planner
  with no parent. The owner's status session archives these by hand.

### 5. `CLAUDE.md`

- Under "Briefs and model classes": `session_start` takes `parent`, from the
  session's own `get_session`.
- A new rule: on `session.archivable`, the archiver acts as in §3.
- An executor's step 6 doesn't change: it ends when its change merges or the
  planner takes the brief back.

### Execution

1. This ADR. (planner)
2. `feat(backplane)`: `parent` on `session.start`, the clean end and
   `session.archivable` (§1–§2), the `session.archived` operation (§3), the
   digest lists (§4), and `CLAUDE.md` (§5), with tests. (planner) It changes
   the ticket records the guard reads and adds an operation, a decision at
   each line, so it isn't briefed. `Governs` gains `worker/push.rs`,
   `events.rs` and `wire.rs` as it lands.
3. Verify live. After the Worker deploys, an executor's clean end is archived
   by its planner, and the next hourly digest lists it. Recorded here as an
   amendment. (planner)

## Amendments

### 2026-11-15: Step 2 lands: `parent`, the clean end, `session_archived` and the digest's lines

- **Paths.** `Governs` gains `worker/push.rs`. `events.rs` and `wire.rs`, which
  step 2 named, stay with the ADRs that rule them: this change adds one kind and
  one operation there, and decides nothing else in them.
- `parent` lives on the ticket record, `Issued.parent`, not on `Declared` as
  §1 said. `Declared` is built as a literal outside the ticket code, so adding a
  field there broke those builds. `Issued::archiver()` is the parent, else
  `declared.planner`. `Tickets::parented` records the parent at
  `/tickets/start`. `sessions::session_id` is a label's last segment.
- `tickets::clean`, `Tickets::record_end`, `record_archive` and
  `unarchived`. The `Instance` serves them as `/tickets/ended` and
  `/tickets/archived`.
- `session.end` answers `clean` and `archiver`. `session.archivable` is in
  `WAKING`.
- `session.archived` is `Effect::Build`. The general ticket guard checks its
  `session` argument, the archiver's, and `record_archive` checks the rest.
- The digest's `archives` lines are built by `tasks::archive_lines`.
- `control::route` carries `#[expect(clippy::too_many_lines)]`, as
  `worker::mod`'s dispatcher does: one arm per operation.
- **Not yet.** Sessions started before the deploy declare no `parent`, so
  they have no archiver. An executor still gets one from its `planner`. Step
  3 verifies live.

### 2026-11-15: Step 3, live: the archive path works, and the digest now reads every event since its cursor

- **What ran.** After !503 deployed (`e760c08`), the planner went through
  `/v0/run` with a synthetic label, `…/session_verify-archive-503`. No Claude
  Code session stands behind that label, so `archive_session` was never
  called.
  - `session.start` with `parent` set to the planner's label.
  - `session.end`: holding nothing, the end was judged `clean: true`, with
    the planner as `archiver`. Events 21211 and 21212 followed: `session.ended`
    "clean", then `session.archivable` addressed to the planner, which
    `wake=agent` lets through.
  - `session.archived` was refused from a session that isn't the archiver.
    The archiver's own report (`skipped`, with why) was recorded as 21213. A
    second report was refused as already recorded.
- **What it found.** `digest_show` omitted the report.
  - `worker/push.rs::build` read a single `/events/since` page, at most
    `events::PAGE` (200) events after its cursor.
  - The hourly digest then moved the cursor only to that page's end.
  - At about 230 events an hour, the digest fell further behind each hour.
    !503's own window approval was missing from "merged without you" too.
- **The fix.** `build` pages through `/events/since`, up to 50 pages, as
  `worker/stages.rs` and `worker/insights.rs` already do. Every event since
  the cursor counts, and the cursor moves to the last event read.
- **What remains.** A real executor's clean end, archived by its planner with
  `archive_session`, is the first end-to-end run. It shows in the next
  digest's archive lines. Sessions started before !503 declare no `parent`, so
  a planner seeded by the status session has no archiver. Its clean end shows
  as "ended clean, not archived".

### 2026-11-16: A silent session holding nothing lapses after two hours, and an archive is reported without the archiver's ticket

- **What broke.** Archiving a Claude Code session doesn't end its ticket.
  The app's Active sessions lists every live ticket, so on 16 Nov it showed 9
  sessions, all archived on the Claude side. 8 held no open change and hadn't
  polled. Their tickets would have lived out the full `TTL` (24 hours). The
  control plane can't see Claude's archive (option (a) above stays closed), so
  it judges by what it does see: silence.
- **The lapse.** A live ticket **lapses** when all of these hold:
  - its session hasn't polled for `tickets::IDLE`, 2 hours, counted from its
    last poll, or from its ticket's start if it never polled. Two hours is
    four re-arms of a 30-minute `follow` Monitor, so a following session never
    reaches it, and an archived one leaves within about three hours (the
    sweep runs hourly);
  - its label holds no open change;
  - it holds no open task it claimed.

  `Tickets::lapsing` names them (pure, tested), and `Tickets::lapse` ends each
  as `session.end` would, recording `ended: {at, clean: true, archiver,
  lapsed: true}`. Holding nothing, the end is clean by §2.
- **Where.** `push::hourly` runs the sweep before it builds the digest
  (`worker/tickets.rs::lapse`, through the `Instance`'s `/tickets/lapse`), so
  the Active sessions list and that hour's digest read the same tickets. Each
  lapse appends `session.ended` ("lapsed: silent N min, holding nothing"). It
  doesn't append `session.archivable`: a lapsed session may still be working
  without following, and its archiver can't tell.
- **A session that holds an open change never lapses.** It keeps its ticket
  until `TTL`, then today's eviction path (`eviction-session-gone`) applies.
  Only the control plane evicts.
- **A lapsed session that is still working** calls `session_start` with its
  label again. Until then its operations need no ticket, as for any ended
  label.
- **The digest.** `unarchived` skips lapsed ends. The control plane can't
  tell whether a lapsed session was archived, and naming every silent
  session for an hour would repeat the noise this fixes.
- **The archive report.** `session.archived` no longer needs the archiver to
  hold a live ticket. The owner's status session archives by hand and holds
  none, so its reports were refused and its archives stayed listed as "ended
  clean, not archived". The ticket guard still asks for the archiver's ticket
  while it holds one. A label with no archiver (a planner seeded before
  `parent`) takes the report from any session but itself, named as `by`.
  The report only clears a digest line, so the looser check risks no more.
- **Execution.**
  1. This amendment, and the code: `tickets.rs`, `worker/tickets.rs`,
     `worker/push.rs`, with tests in `tests/tickets.rs`. (planner)
  2. Verify live: after the deploy, the first hourly run ends the stale
     tickets, the Active sessions list keeps only !653's session, and the
     status session's reports clear its "ended clean, not archived" lines.
     Recorded here as an amendment. (planner)

### 2026-11-16: Governs globs match the old and the new crate layout

Per [`2026-11-16-development-src-crates-neutral-names.md`](2026-11-16-development-src-crates-neutral-names.md) §6, each
glob under `src/pigeon-cli/` or `src/site-templates/` becomes `src/**/<tail>`
(or `**<file>` for a workspace root file), which matches the file both before
and after the restructure moves it. That ADR's step 12 tightens them to the
new paths.
