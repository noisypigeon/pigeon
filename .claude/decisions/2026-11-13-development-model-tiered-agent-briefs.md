## Model-tiered sessions: Opus plans and writes briefs, Sonnet and Haiku carry them out to the letter

* Date: 2026-11-13
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development
* Governs: `src/pigeon-cli/crates/backplane/src/sessions.rs`, `src/pigeon-cli/crates/backplane/src/tasks.rs`, `tooling/session/session.py`, `tooling/session/models.json`, `src/pigeon-cli/crates/backplane/src/core/brief/**`, `src/pigeon-cli/crates/backplane/src/briefs.rs`, `src/pigeon-cli/crates/backplane/src/worker/briefs.rs`
* Governs: `src/pigeon-cli/crates/backplane/src/sessions.rs`, `src/pigeon-cli/crates/backplane/src/tasks.rs`, `tooling/session/session.py`, `tooling/session/models.json`, `src/pigeon-cli/crates/backplane/src/core/brief/**`, `src/pigeon-cli/crates/backplane/src/briefs.rs`, `src/pigeon-cli/crates/backplane/src/worker/briefs.rs`

## Context

The owner wants to choose a model for each session, so each part of the
development lifecycle goes to Haiku, Sonnet or Opus. When Haiku or Sonnet
work, Opus does the heavy lifting: it makes every decision, and the smaller
models follow explicit tasks that Opus wrote for them.

### What exists

- **Sessions are named, not trusted.**
  [`2026-11-08-development-concurrent-agent-sessions.md`](2026-11-08-development-concurrent-agent-sessions.md)
  §1 gives each session a label. It routes tasks and grants nothing. Every
  session holds the same agent token.
- **Tickets** bind a label to its changes
  ([`2026-11-08-development-events-tickets-digest-mandate.md`](2026-11-08-development-events-tickets-digest-mandate.md)
  §5). A ticket is `<label>.<generation>.<expires>.<signature>`.
- **Tasks** (`backplane::tasks`) route work to a session, with `task_claim`
  and `task_wait`. Their kinds are `reconcile`, `review`, `restack-failed`,
  and the eviction ADR's `resume`.
- **Stages**
  ([`2026-11-13-development-session-stages-subscriptions-breaker.md`](2026-11-13-development-session-stages-subscriptions-breaker.md))
  say who acts next on a change. The follower wakes a session only for what
  it must act on. The breaker takes a looping change out of Auto.
- **Auto windows and mandates**
  ([`2026-11-08-security-tiered-approvals-task-mandates.md`](2026-11-08-security-tiered-approvals-task-mandates.md))
  approve below the `owner` tier without the owner.
- **Starting sessions is out of the control plane's scope** (concurrent ADR
  §5). A session can start another one itself: the remote harness's
  `create_session` takes a `model`. The harness's `Agent` tool takes one too,
  for subagents inside a session.
- **An ADR names the model that wrote it,** in `* Model:`. Nothing else
  records which model did what.

### The gaps

1. **No session says which model it runs on.** The control plane can't route
   by capability, and the digest can't show it.
2. **Nothing stops a small model from deciding.** A Haiku session holding the
   agent token may write an ADR, propose a guarded change, or improvise
   around a failed check. Each one is a decision.
3. **There is no artifact for "an explicit task".** A change's plan is prose
   written for the owner. An ADR step is written for a session that can fill
   gaps. Neither is precise enough for a model that shouldn't.
4. **Nothing checks a small model's work against what it was asked.** Checks
   prove that the code builds and passes. They don't prove it does what the
   step meant.
5. **A small model's fix loop is caught late.** The breaker's thresholds are
   tuned for Opus, which usually fixes a failure in two tries.

### Threat model

The threat is mistakes, not adversaries, as with labels. A session declares
its model, and the control plane can't authenticate the declaration. The
declaration binds the session to what its class may do, so the control plane
can refuse a Haiku session's design change. A session that falsely claims to
be Opus is a bug in the session that started it, and §7 makes it visible.

## Implementation plan.

### 1. A session declares its model class

- **`session.start` takes `model`:** one of `haiku`, `sonnet` or `opus`, the
  *class*, never a model ID. A session that leaves it out is `opus`, so
  today's sessions keep working.
- **The class is kept by the `Instance` beside the ticket's label and
  generation,** not in the ticket. The ticket's format stays the same, so no
  live ticket breaks. `session_start` with `replace` may change the class.
  The new generation carries the new class.
- **`session.start` also takes `planner`,** the label of the session that
  started this one, and `brief` (§3), for executors. Both are stored and
  shown.
- **`session.status`, `change_list` rows and events carry `model`,** the
  class of the session that proposed or pushed.
- **Model IDs live in one file, `tooling/session/models.json`:**
  `{"haiku": "claude-haiku-5-5", "sonnet": "claude-sonnet-5-5", "opus": "claude-opus-5-5"}`.
  Upgrading a class is a one-line change, and the control plane never reads
  an ID.

### 2. Roles: Opus plans, Sonnet and Haiku execute

**A class fixes a role:**

| Class | Role | May |
|---|---|---|
| `opus` | planner | everything an agent may today, and write, amend, review and take back briefs |
| `sonnet` | executor | carry out a `sonnet` or `haiku` brief it has claimed |
| `haiku` | executor | carry out a `haiku` brief it has claimed |

**What only a planner may do.** `change.propose` refuses an executor's
ticket when the change:

- has `affects`, or a scope under `.claude/decisions/` (an ADR or an
  amendment);
- meets the `guarded`, `secrets`, `deploying` or `sentinel` lists in
  `.config/approvals.yaml`;
- names a `mandate` (executors work under the planner's brief, not a
  mandate of their own);
- has no `brief`, or a scope outside its brief's scope (§4).

The same refusals apply to `mandate_propose`, `change_attend`'s Resume
answer, `change_handoff` to an executor, and `task_claim` on `reconcile` and
`review` tasks whose plan requires an ADR amendment. They are refused with
the rule and the brief that would cover the work, if one exists.

**Which component of the lifecycle goes to whom.**

| Component | Class | How |
|---|---|---|
| Writing an ADR, an amendment, a `review` reconciliation | opus | directly |
| Splitting an ADR's execution step into briefs | opus | `brief_create` |
| Any change at the `guarded`, `secrets`, `deploying` or `sentinel` tier | opus | directly, no brief |
| A `reconcile` whose rule requires an ADR amendment | opus | directly |
| A code-only `reconcile` | sonnet | a brief the planner writes from the generated plan |
| Implementing code and tests to a stated interface | sonnet | `sonnet` brief |
| Fixing a failed check within a brief, once | the executor | the brief's own steps |
| Listed edits: renames, moves, literal text, generated files, docs lines | haiku | `haiku` brief |
| `restack-failed` with no conflict | haiku | `haiku` brief |
| Reviewing an executor's change against its brief | opus | `brief_review` (§5) |
| An `attended` executor change, a `resume`, an escalation | opus | the planner first (§6) |
| Read-only search, log triage, summarising a failure | haiku | an `Agent` subagent, model `haiku`, inside any session |

**Subagents are not sessions.** A subagent started with the `Agent` tool
holds no ticket. It never proposes, pushes or calls a change operation. It
reads and reports back to the session that started it, which stays
responsible. Anything that becomes a change goes through a session with its
own ticket and class.

### 3. Briefs: the planner's explicit work orders

**A brief is a work order the planner writes and the control plane keeps.**
It lives in the `Instance` beside tasks (`backplane::briefs`, its pure part
in `core/brief/`). Its MCP tools are `brief_create`, `brief_show`,
`brief_amend` and `brief_list`.

**Fields:**

- `id` (`b<n>`) and `title`;
- `adr`: the ADR path, and `step`: the step it carries out;
- `planner`: the label of the opus session that wrote it;
- `model`: `haiku` or `sonnet`, the least capable class that may take it;
- `base`: the `main` commit the brief was written against;
- `scope`: the exact paths it may touch, at most 12, with no `**`;
- `decisions`: what the planner settled and the executor must not revisit,
  one line each (names, types, error handling, which side of a trade-off);
- `steps`: in order. Each step has:
  - `do`: one imperative sentence;
  - `files`: the paths it touches, all within `scope`;
  - `content`: the literal text, diff or signature, required for `haiku`;
  - `done_when`: a command and its expected result, or one observable fact;
- `checks`: the commands to run before pushing, normally
  `CHANGE_BASE=artifacts/main mise run ci-scoped`;
- `stop_if`: the conditions on which the executor must escalate rather than
  continue, at least one;
- `budget`: the most heads (default 3 for haiku, 4 for sonnet) and hours
  (default 2) before the brief returns to the planner;
- `commit`: the Conventional Commit title and the plan the executor's change
  will carry, written by the planner.

**What makes a brief explicit enough, by class.**

- **A `haiku` brief transcribes.** Every step carries `content`: the exact
  text to write or the exact diff. At most 5 files. No step asks the
  executor to choose a name, a type, a test case or an order.
- **A `sonnet` brief implements.** Every step names the functions, types and
  signatures it adds or changes, the behaviour each must have as a list of
  cases, and the tests by name. The executor writes bodies and test code.
  It adds no public interface, file, dependency or feature flag the brief
  doesn't list.
- **Neither may hold an open question.** A brief whose step leaves a choice
  is the planner's work, not yet done.

**The control plane validates a brief at `brief_create`** (`core/brief`,
pure and tested). It refuses:

- a field missing, `scope` outside the planner's change or ADR area, or a
  step's `files` outside `scope`;
- a `haiku` step without `content`, or more than 5 files;
- a step whose `do` or `done_when` holds an open-choice phrase: `decide`,
  `choose`, `consider`, `as needed`, `if appropriate`, `figure out`,
  `TBD`, `or similar`, `etc.`. The list is a constant beside the validator.
  The check is crude, which is why §5's review exists.
- a `base` that isn't on `main`.

**A brief is append-only, as an ADR is.** `brief_amend` appends a revision
with its reason, and never rewrites earlier text. An executor always works
on the latest revision. The follower wakes it when a revision lands.

**A brief becomes a task.** `brief_create` makes a `brief` task routed by
class. A `brief` task may be claimed by a session whose class is at least
the brief's `model`. After 30 minutes unclaimed, it may be claimed by the
next class up, as any task opens to any session today. A less capable class
is always refused.

### 4. An executor's change is the brief's change

- **`change.propose` from an executor takes `brief`,** and nothing in the
  proposal is the executor's own:
  - the title and plan are the brief's `commit`, with the brief's id and
    revision appended;
  - the scope is the brief's `scope`;
  - `affects` and `mandate` are refused (§2).
- **Its first commit carries a `Brief: b<n>` trailer** beside its
  `Change-Scope` trailers. Commits carry no model ID; the class is the
  control plane's record.
- **One brief, one open change.** A second proposal for a brief with an open
  change is refused, naming that change. This is also the copy refusal the
  breaker ADR wants, for briefs.
- **The executor's rules,** in `CLAUDE.md` under "Executors":
  1. Do the brief's steps in order, and only them. Touch only each step's
     `files`.
  2. After each step, run its `done_when`. If it doesn't hold, try once
     more, within the step.
  3. On a second failure, a `stop_if` condition, anything the brief doesn't
     cover, or a contradiction between the brief and the code: stop and
     `brief_escalate`. Never work around it, never edit the brief's
     decisions, never widen the scope.
  4. Run the brief's `checks` before every push.
  5. Never write or amend an ADR, and never call an owner operation.
  6. When the change is merged or the brief is taken back, `session_end`.

### 5. The planner reviews before Auto approves

- **An executor's change gains the stage `in-review`** between `checked` and
  approval, with `needs: session:<planner>`. The stage table in the
  session-stages ADR §1 gains this row.
- **A `brief-review` task goes to the planner** when the change reaches
  `checked`. It holds the brief, the diff against `base`, and the checks'
  results.
- **`brief_review {change, head, verdict, notes}`, planner only:**
  - `accept`: the change moves to `checked` for approval as any change. A
    window or mandate may then approve it;
  - `revise`: the planner amends the brief (§3) with what was wrong, and the
    executor pushes again. That counts against the brief's budget;
  - `take`: the planner takes the change over (`change_handoff` to itself),
    and the executor's session ends.
- **Windows and mandates skip an executor's change until it is accepted.**
  `window_approves` and the mandate check answer none for `in-review`. They
  are guarded, so that step is the owner's.
- **The review checks fidelity, not style:** every step done, nothing
  outside them, the decisions kept, the `done_when`s true. Style is
  `mise run ci`'s job.

### 6. Escalation goes to the planner before the owner

- **`brief_escalate {brief, change, question, tried, evidence}`,** executor
  only, makes an `escalation` task for the planner. The change's stage
  becomes `escalated`, `needs: session:<planner>`. Its branch refuses pushes
  until the planner answers, so the executor can't keep trying.
- **The planner answers with one of:**
  - `brief_amend`, which reopens the branch, and wakes the executor;
  - `take`, as in §5;
  - `change_attend`, when the question needs the owner, with the executor's
    `tried` and `evidence` carried in.
- **The breaker trips sooner for executors.** In `core/change/breaker.rs`,
  per class:

  | Signal | opus | sonnet | haiku |
  |---|---|---|---|
  | B1, the same check failing on heads in a row | 3 | 2 | 2 |
  | B2, the same fingerprint on heads | 2 | 2 | 2 |
  | B4, heads without reaching `checked` | 8 | 4 | 3 |
  | the brief's budget, heads or hours | - | 4 / 2h | 3 / 2h |

- **An executor's tripped change goes to its planner first.** It becomes
  `escalated`, not `attended`, with the signal as its evidence. The owner is
  pushed only if the planner's session isn't listening, or the planner calls
  `change_attend`. Two escalations on one brief take it back to the planner
  without asking.
- **A planner that isn't listening hands its briefs over.** After 30
  minutes, its `escalation` and `brief-review` tasks may be claimed by any
  opus session, as any task opens up today.

### 7. Starting executors, and seeing what ran

- **The planner starts executors; the control plane doesn't.** That keeps
  the concurrent ADR's §5. The planner:
  1. writes the briefs for a step (`brief_create`);
  2. for each one, runs `python3 -I tooling/session/session.py executor b<n>`,
     which prints the arguments for the harness's `create_session`: the
     model ID from `models.json`, a title, and a prompt naming the brief,
     the class and the "Executors" rules;
  3. calls `create_session` with them, and records the new session's ID in
     the brief (`brief_amend`, `executor`);
  4. follows its own events, which carry its executors' reviews and
     escalations, since the executors' changes are in its family.
- **An executor's first acts** are `session_start` with `model`, `planner`
  and `brief`, then `task_claim` of its brief, then `change_propose` with
  `brief`.
- **Checking the class.** The planner calls `get_session` on each executor
  it started, at start and at review. When `last_served_model` isn't the
  class's ID, as after a fallback, it notes it on the brief. If it is a
  larger model, the work stands. If it is a smaller one, the planner
  reviews with that in mind or takes the change. `session.py brief` also
  prints a warning when the `SessionStart` hook's input names a model whose
  class differs from the ticket's.
- **The digest and `session_status` show each session's class,** each
  brief's state, and per ADR step the heads, reviews and escalations by
  class. The owner can then see whether a class is right for a kind of
  step. Tuning the table in §2 is an amendment here.

### 8. Limits

- A planner holds at most 8 open briefs. An executor holds one brief and its
  change.
- Executor changes count against the instance's open-change limit, but not
  against their planner's per-session limit.
- A brief whose `base` falls more than 50 commits behind `main`, or whose
  scope a merged change touched, is marked `stale`. Its executor
  escalates before pushing again.

### Execution

Each step is one change. Steps 1 to 4 are opus work. Steps 5 to 7 are the
first to be split into briefs, as the trial of this ADR.

1. **`feat(backplane)`: classes on sessions.** `model`, `planner` and `brief`
   on `session.start`; the class in the `Instance` and on `session.status`,
   `change_list` and events; `tooling/session/models.json`.
2. **`feat(backplane)`: briefs.** `core/brief/` (fields, validation, the
   open-choice list, revisions) and its tests; `backplane::briefs`;
   `brief_create`, `brief_show`, `brief_amend`, `brief_list`; `brief` tasks
   routed by class. `Governs` gains `core/brief/**`, `briefs.rs` and
   `worker/briefs.rs`.
3. **`feat(backplane)`: executor proposals.** The §2 refusals and the §4
   binding on `change.propose`, `mandate.propose`, `task_claim` and
   `change_handoff`; the one-change-per-brief rule.
4. **`feat(tooling)`: `session.py executor` and the "Executors" section of
   `CLAUDE.md`.**
5. **`feat(backplane)`: review and escalation.** The `in-review` and
   `escalated` stages; `brief_review` and `brief_escalate`; the
   `brief-review` and `escalation` tasks; the push refusal while escalated.
6. **`feat(backplane)`: Auto skips `in-review`,** in `windows.rs` and
   `mandates.rs`, which are guarded: the owner, never a mandate. The
   amendments to the session-stages and tiered-approvals ADRs.
7. **`feat(backplane)`: the per-class breaker, and the digest's classes.**
   The thresholds of §6, after the breaker's shadow run, and the §7 digest
   lines.

### Verification

- **Classes:** a session started with `model: haiku` shows as `haiku` in
  `session_status` and on its change's events. One started without `model`
  shows `opus`.
- **Refusals:** a `haiku` session's proposal with `affects`, a scope under
  `.claude/decisions/`, a guarded path, or no brief is refused, naming the
  rule. A `sonnet` session's `task_claim` of an ADR-amending `reconcile` is
  refused.
- **Briefs:** a `haiku` brief with a step lacking `content`, or a step saying
  "choose a suitable name", is refused at `brief_create`. A `sonnet` session
  claims a `haiku` brief only after 30 minutes. A `haiku` session never
  claims a `sonnet` brief.
- **Review:** an executor's change reaches `checked` and stops at
  `in-review`, under an Auto window that would otherwise approve it. The
  planner's `accept` lets the window approve and merge it. `revise` wakes
  the executor with the new revision.
- **Escalation:** a `haiku` change failing `clippy` twice with one
  fingerprint becomes `escalated`, refuses a third push, and wakes the
  planner, not the owner. The owner is pushed only when the planner calls
  `change_attend`, or isn't listening.
- **End to end:** an opus planner splits one ADR step into a `sonnet` and a
  `haiku` brief, starts two sessions with `create_session`, and both changes
  merge after review. Every decision in both diffs traces to a line of a
  brief.

## Amendments

### 2026-11-13: Step 1 splits in two; its first half lands on `session.start` and `session.status`

- **Paths.** `Governs` gains `tooling/session/models.json`.
- **Why two halves.** A session's class lives in the `Instance`, beside its
  ticket. `session.start` and `session.status` read it there. `change_list`
  reads D1 rows, and an event is appended by whichever object acts, so each
  needs the class carried to it. Those come together in step 1b.
- **Step 1a, this change:**
  - `sessions::Class`: `haiku`, `sonnet` and `opus`, ordered by capability,
    `opus` by default, each with its role and `covers`;
  - `sessions::declare`: an executor must name its `planner` and its `brief`
    (`b<n>`), and an opus session may name neither. A brief's existence
    isn't checked until step 2 makes briefs;
  - `tickets::Issued` keeps what was declared, read as `opus` from a ticket
    stored before, so live sessions keep working;
  - `session.start` takes `model`, `planner` and `brief`, answers `model` and
    `role`, and its event names the class;
  - `session.status` answers `model`, `role`, `planner` and `brief`;
  - `tooling/session/models.json` maps each class to its model ID.
- **Step 1b:** `model` on `change_list`'s rows and on each event.

### 2026-11-13: Reviewed against !386 and !387: they agree, and briefs' tasks are never swept

This reviews !386 and !387 against this ADR. Both cite
[`2026-11-08-development-concurrent-agent-sessions.md`](2026-11-08-development-concurrent-agent-sessions.md)
and
[`2026-11-10-development-change-slot-eviction-policy.md`](2026-11-10-development-change-slot-eviction-policy.md),
and work in `tasks.rs`, which this ADR governs. One review covers both
changes, as !386 itself now plans: one per session and ADR.

- **They agree.**
  - !386 adds `Tasks::sweep`, which closes tasks on changes no longer open.
  - !387 adds the digest's `stalled`.
  - Neither changes a task's routing, its claim rules or its kinds, which
    §3 and §5 extend by class.
- **One point to keep: `tasks::SWEPT`.** The hourly sweep closes only
  `reconcile`, `review`, `restack-failed`, `yield-ended` and `handoff` tasks.
  - §3's `brief`, and §5's `brief-review` and `escalation`, aren't listed,
    so they are never swept.
  - A brief's task outlives the change it was written in. It names a brief,
    and its own change may not exist yet.
  - The step that adds those kinds leaves them out of `SWEPT`.
- **The digest's stalls and §7's classes fit together.** A stall line names
  the change, its session and how long the session has been quiet. When §7
  shows each session's class, the stall line can carry it too. That is §7's
  step, not !387's.

### 2026-11-13: Step 1b lands, and step 1 is done

- **Each event carries `model`,** its session's class when it was appended.
  The `Instance` appends events and holds the tickets, so it fills the class
  in at append time, at no extra request. An event of no session, or of a
  label never started, has none. An event logged before reads as none.
- **`change_list`'s rows carry `model`,** looked up from the tickets once per
  list. A lapsed ticket's class is still known for the week a ticket is
  remembered, so a change outliving its session keeps its class.
- **The class is the session's latest.** A session that replaced its ticket
  with another class shows the new one on its older changes too. Briefs
  (step 2) record the class that carried each one out.

### 2026-11-14: Step 2 splits in three; its first part lands: the brief and its validation

- **Paths.** `Governs` gains `core/brief/**`, and `tooling/session/models.json`
  again: step 1a added it, and !375's restack dropped that line edit.
- **Why three parts.** The pure model can land alone, with no Worker code,
  and nothing else edits its files. Storage and the MCP tools come next, then
  tasks routed by class, which touch `tasks.rs` after !386's sweep.
- **Step 2a, this change:** `core::brief`, pure and tested:
  - `Draft`: §3's fields, a step's `do` serialized as `do`, and a `budget`
    defaulting by class (`haiku` 3 heads, `sonnet` 4, both 2 hours, at most
    8 of each);
  - `validate` answers every problem at once, not the first;
  - a brief is never for `opus`;
  - `scope` holds exact relative paths, no glob, 1 to 12 (`haiku` 5), and a
    step's `files` lie in it;
  - a `haiku` step carries `content`;
  - `do` and `done_when` refuse `OPEN_CHOICE`'s phrases as whole words,
    ignoring case. `content` is the planner's literal text, so it isn't
    checked;
  - `base` is 7 to 40 hex digits. Whether it is on `main` needs git, so step
    2b checks it;
  - `commit.title` is a Conventional Commit title;
  - a draft is at most 16 KiB serialized.
- **A brief is append-only:** `Brief::create` makes revision 1, and
  `Brief::amend` appends one, by its planner only, with a reason. No revision
  is rewritten.
- **Step 2b:** the `Instance`'s store; `brief_create`, `brief_show`,
  `brief_amend` and `brief_list`; `base` checked against `main`; and `scope`
  checked against the planner's ADR.
- **Step 2c:** `brief` tasks, routed and claimed by class.

### 2026-11-14: Step 2b lands: the store and the `brief.*` operations

- **Paths.** `Governs` gains `briefs.rs` and `worker/briefs.rs`.
- **The `Instance` keeps every brief** (`backplane::briefs::Briefs`, under
  `briefs`), numbered `b<n>`. A refused brief takes no number.
- **Four operations,** as MCP tools:
  - `brief.create {session, draft}`: `draft` is the brief as JSON text, since
    an argument is text;
  - `brief.amend {id, session, draft, reason}`: appends a revision, by its
    planner only;
  - `brief.show {id}`: every revision, oldest first;
  - `brief.list {planner?}`: newest first, one row each.

  Each write answers every problem at once (422), and logs `brief.created`
  or `brief.amended` for the planner's session.
- **Only an opus session writes a brief.** The class is its ticket's (§1),
  and a label never started counts as opus, as at `session.start`. An
  executor is refused with 403. The ticket guard already covers the writes,
  since each names its `session`.
- **Two of §3's checks are left out,** because the Worker reads neither
  `main`'s history nor its files when asked:
  - whether `base` is on `main`. An executor rebases onto `main` anyway, and
    §8's `stale` covers a base that falls behind;
  - whether `scope` lies in the planner's ADR area. §5's review sees every
    path the executor touched.

  A later step may check both in the bound job, which has the repository.
- **Step 2c:** `brief` tasks, routed and claimed by class.

### 2026-11-14: Step 2c lands, and step 2 is done: brief tasks claimed by class

- **`brief.create` makes the brief's task** (`Tasks::add_brief`): kind
  `brief`, with the brief's id and class, on no change. There is one open
  task per brief. `brief` isn't in `SWEPT`, so the sweep never closes it, as
  the 2026-11-13 review here asked.
- **Who may take it, by the session's ticket class:**
  - its own class at once;
  - a more capable class once it has waited 30 minutes unclaimed (`OWN`);
  - a less capable class never.

  §3 said "at least the brief's `model`" at once. That would let opus
  sessions take Haiku's work before any Haiku session looked, so a more
  capable class waits.
- **`task.list`, `task.wait` and `task.claim` read the session's class** and
  show or allow only what that class may take. A task with no class keeps
  today's rules for every class. A claimed task stays visible to its claimer.
- **Not yet:** checking that a session's declared `brief` (§1) matches the
  task it claims. That comes with step 3, which binds an executor's proposal
  to its brief.

### 2026-11-14: Step 3 splits in two; its first part lands: an executor proposes only its brief's change

- **Why two parts.** Refusing an executor the `guarded`, `secrets`,
  `deploying` and `sentinel` paths needs `.config/approvals.yaml`'s lists,
  which the bound job reads, not the Worker at propose time. They come in
  step 3b, from the bound job's tiers.
- **Step 3a, this change:** `core::brief::executor::check`, pure. It reads
  the session's ticket (class and declared brief), the brief, and the open
  change already carrying the brief:
  - an opus session proposes as before, and may name a brief too;
  - an executor must name a brief that exists, that its class covers, and
    that matches its declared one;
  - it may name no `affects`, no mandate, and no scope under
    `.claude/decisions/`;
  - its scope, if given, is exactly the brief's;
  - **one open change per brief:** a second one is refused, naming the first.
- **The brief writes the change.** With `brief`, `change.propose` takes its
  title from the brief's `commit`. Its plan is the brief's plan plus a line
  citing the brief, its revision, class, ADR and step. Its scope is the
  brief's. Without `brief`, title, plan and scope are required as before.
  The brief records each change proposed for it (`/briefs/bind`).
- **A brief never names an ADR in its scope:** validation refuses it, since
  only the planner writes ADRs.
- **Not in this part:**
  - the `Brief:` commit trailer (§4), which is the executor's to write by
    `CLAUDE.md`'s rules in step 4;
  - `mandate.propose`, `change_attend`'s Resume, `change_handoff` to an
    executor, and the `reconcile` and `review` claim refusals of §2, which
    come with step 3b.
