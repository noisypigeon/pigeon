## Events instead of polling, session tickets, the owner's digest, and a first mandate

* Date: 2026-11-08
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development

## Context

[`2026-11-08-development-concurrent-agent-sessions.md`](2026-11-08-development-concurrent-agent-sessions.md)
is live, through !53 to !59. Four gaps remain, and the owner asks for a
fifth thing.

1. **The approval carry-over has never worked live.**
   - !57's merge restacked !58 correctly. Then Artifacts' push event
     dropped the restack job before its report was read, so the approval
     didn't carry.
   - !59 holds such a push until the report arrives. No pair of open
     changes has exercised that fix yet.
2. **Sessions aren't separated.**
   - Every session holds the same `pigeon-agent` Access service token.
   - "A session acts only on its own changes" is a CLAUDE.md rule. The
     control plane can't tell sessions apart, because `session` is a label
     anyone can send.
3. **There's no digest.**
   - Tasks unclaimed for two hours, paused mandates, failed deploys and
     automatic merges reach the owner only if the owner asks.
   - The tiered-approvals ADR's step 6 promised a digest. This ADR builds
     it.
4. **No mandate has been granted.**
   - !58 made the mandate tools reachable.
   - Automatic authorize, approve and merge inside a mandate has run only in
     unit tests.
5. **Everything is polled.** The owner asks for notifications when
   operations complete: plans, approvals, jobs and the rest. Today:
   - the agent learns a check finished, a merge landed or a deploy passed
     by calling `change_status` or `deploy_status` in a sleep loop, which is
     minutes of waiting and many calls per change;
   - the owner learns an approval is waiting only when the agent pastes a
     link into the chat.

What the pieces offer:

- **Durable Objects can hold WebSockets** with the hibernation API. A
  socket stays open without keeping the object awake.
- **Claude Code's `Monitor` tool takes a `ws` source.** Each text frame
  becomes a notification that wakes the session, with no polling in the
  turn. A monitor lasts at most 30 minutes and is then re-armed.
- **The agent's HTTPS proxy authenticates requests to the control plane's
  host** with the agent's Access service token. A spike run while writing
  this ADR found:
  - a WebSocket handshake sent with `curl` through the proxy carries the
    token. The Worker, not Access, answered it;
  - `Monitor`'s own WebSocket client doesn't use the proxy. Its upgrade was
    refused with a 302 to the Access login, and the socket closed with
    1006.
- **Web Push** lets a page behind Access, `/approvals`, subscribe the
  owner's phone, so the Worker can push to it with a VAPID key.
- **Cron triggers** are already supported by `worker.yaml`
  ([`2026-11-05-development-cobot-workers-area.md`](2026-11-05-development-cobot-workers-area.md)).

## Implementation plan.

### 1. One event log per instance

- **The `Instance` Durable Object keeps an event log.** Each entry has a
  rising `id`, a time, a `kind`, its subject (a change, task, mandate or
  deploy), the change's session if any, and a one-line summary.
  - It keeps the last 2,000 entries, or 7 days, whichever is less.
- **Every completion that matters is appended:**
  - **changes:** planned, pushed, bounded or refused, checked or failed,
    approved (by whom, and whether carried), behind, restacked,
    conflicted, merged, closed;
  - **approvals:** pending (with the link) and answered;
  - **jobs:** each job's start and its end, with the outcome;
  - **deploys:** started, passed, failed, rolled back;
  - **tasks:** made, claimed, done;
  - **mandates:** proposed, granted, paused, ended.
- **Producers write once.** `ChangeRun` and `MainRun` post each event to
  the `Instance` after saving, and the `Instance` fans it out. No producer
  knows who listens.
- **Events hold no secrets.** A pending approval's link is in its event
  only for subscribers authenticated as the owner. The agent's event names
  the change and the operation, not the grant.

### 2. Agents subscribe instead of polling

- **`GET /v0/events?after=<id>&wait=50`** is a long poll on the `Instance`.
  It answers as soon as an event after `after` exists, or after `wait`
  seconds with none. Query `session=` and `change=` (repeatable) filter it.
  Each event is one JSON line.
- **`pigeon backplane events --follow [--session S] [--change N]`** loops
  over that long poll and prints one line per event, carrying `after`
  forward. It goes through the proxy like `curl`, so the token is attached.
  A shell loop over `curl` does the same where `pigeon` isn't built.
- **How a session uses it:**
  - it arms `Monitor` with `events --follow` as its command, filtered to its
    session, for 30 minutes, and re-arms on expiry with `--after` the last
    id;
  - each line wakes the session, which acts when its change was checked,
    failed, restacked or conflicted, or a task is waiting, instead of
    calling `change_status` on a timer.

  CLAUDE.md says so.
- **The MCP tool `events_wait`** is the same long poll for a client with
  no `Monitor`.
- **No WebSocket route, for now.** `Monitor`'s WebSocket client bypasses
  the proxy, so it can't pass Access. A route behind a credential `Monitor`
  can present waits for a later ADR.

### 3. The owner is pushed to, not pasted to

- **Web Push from `/approvals`.**
  - The page offers "notify this device". It registers a service worker and
    stores the push subscription in the `Instance`, tied to the
    Access-verified owner.
  - The VAPID private key is a new Worker secret, `PUSH_VAPID_KEY`, set with
    `pigeon cobot workers secrets put` like the others. No secret enters the
    repository.
- **What is pushed:**
  - an approval waiting at the confirm or code tier, linking to its page;
  - a mandate paused, with why;
  - a deploy failed or rolled back;
  - the digest, when it has anything (§4).
- **What is not pushed:** every check, every merge made automatically,
  anything the owner didn't need to act on. Those go in the event log and
  the digest.
- **The agent still pastes the link,** so an approval works without a
  subscribed device.

### 4. The digest

- **An hourly cron** builds the digest from the event log since the last
  one:
  - approvals waiting, oldest first;
  - tasks unclaimed for two hours;
  - mandates paused or used up;
  - deploys that failed;
  - changes merged without the owner (by a mandate or a standing rule),
    with what each approved.
- **It is shown at the top of `/approvals`**, returned by a `digest` MCP
  tool, and pushed (§3) only when it holds something new that needs the
  owner: a waiting approval, a stale task, a pause or a failure.
- **This is the tiered-approvals ADR's promised digest.** That ADR is
  amended to point here.

### 5. Session tickets

- **The threat is mistakes, not malice.** Every session belongs to the
  one owner and runs the same agent. The goal is that a session can't act
  on another session's change by accident; it is not a boundary against a
  hostile session.
- **The new MCP tool `session_start`** takes the session's label and
  returns a ticket: `<label>.<expiry>.<HMAC under SESSION_KEY>`.
  - It lasts 24 hours, and the session keeps it outside the repository.
  - `session_start` with the label of a session holding a live ticket asks
    the owner at the confirm tier, so a lost ticket is recovered with one
    tap and never silently.
- **A ticket binds the session.**
  - `change.propose` with a ticket records its label as the change's
    `session`.
  - Every agent operation on that change requires the same ticket:
    `change.pushed`, `ci.rerun`, `task.claim`, `task.done`, and the agent's
    calls that offer an approval or a merge.
  - A wrong or missing ticket is refused, naming the change's session.
- **The git front door checks it too.** A push to `change/<n>-*` carries
  the ticket in a header. `git` sends it through `http.extraHeader`, which
  CLAUDE.md shows.
- **Changes without a session stay as today,** so nothing open breaks.
  Owner operations need no ticket: Access names the owner.

### 6. The first mandate, and the live carry-over trial

- **Steps 1, 2 and 4 are carried out under one mandate:**
  - its scope is the new modules (`events.rs`, `worker/events.rs`,
    `digest.rs`, `worker/digest.rs`, their tests) and this ADR;
  - it may deploy, has a budget of 4 and lasts 12 hours;
  - the owner grants it once with the code. Each change under it is then
    authorized, approved and merged as `mandate:<id>`.
  - The guarded approval pages (§3) and tickets (§5) stay outside it and
    ask the owner as usual.
- **Two of its changes are made disjoint on purpose and approved
  together:**
  - step 1's event-log core, in `events.rs` and `worker/events.rs`;
  - step 4's digest builder, in `digest.rs` and `worker/digest.rs`, wired
    in by a later change.

  Merging the first restacks the second. Its approval must carry, with no
  prompt to the owner and `mandate:<id>` as its merger. The results are
  recorded in this ADR.
- **If the carry fails again,** the mandate pauses itself and the owner
  is told, as the mandate rules say. That is the trial working as
  designed.

### Execution

1. Spike, done while writing this ADR: `curl` through the proxy passes
   Access, and `Monitor`'s WebSocket doesn't (§2).
2. Under the mandate (§6): the event log and its producers (§1); the
   digest builder (§4). These are the two disjoint changes, then the
   wiring.
3. Under the mandate: `/v0/events` (long poll) and `events_wait` (§2);
   `pigeon backplane events --follow`; CLAUDE.md on arming `Monitor` with
   it.
4. Owner-approved, with guarded files: Web Push on `/approvals`,
   `PUSH_VAPID_KEY`, the hourly cron, and the digest on the page (§3, §4);
   an amendment to the tiered-approvals ADR.
5. Owner-approved, with guarded files: session tickets, their checks on
   agent operations and at the git front door (§5).

### Verification

- **Carry-over (§6):** the second disjoint change merges with
  `approval.by` the mandate and no owner prompt after the grant.
  `mandate_status` shows `used: 2`.
- **Events (§2):** a session with `Monitor` armed hears its change go from
  pushed to checked without one `change_status` call. A re-armed monitor
  with `after=` misses nothing.
- **Push (§3):** a code-tier approval reaches the owner's phone within a
  minute of being offered; an automatic merge doesn't.
- **Digest (§4):** a task left unclaimed two hours appears in the next
  hourly digest and is pushed once.
- **Tickets (§5):**
  - a second session's `change_pushed` on the first session's change is
    refused, and so is its push of that branch;
  - a lost ticket is recovered with one confirm tap.
