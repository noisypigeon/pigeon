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

## Amendments

### 2026-11-08: The first mandate and the carry-over, proven live

- **Mandate 1 was granted once, with the code.** It then authorized,
  approved and merged !61 (the event log) and !62 (the digest builder) as
  `mandate:1`, with no prompt to the owner.
- **Merging !61 restacked !62.**
  - The two changes were disjoint, so !62 became one commit, `0a91069`, on
    the new `main`, and its checks ran again.
  - Its approval carried to the new head with its original time, and it
    merged.
  - `mandate_status` showed `used: 2`, `merged: [61, 62]`.
- So !59's fix holds: the push for the restacked head was held until the
  restack reported.

### 2026-11-08: Steps 1 and 2 land: producers, the long poll, and how a session follows

- **Producers.**
  - `ChangeRun` compares each change before and after it is saved
    (`events::of_change`). It reports a new state, each job's start and end,
    falling `behind`, and a conflict with its reconciliation.
  - `MainRun` reports each deploy's end.
  - Offering an approval reports `approval.pending` twice: once for
    everyone, with no link, and once for the owner only, with the link.
  - Making a task reports `task.made`.
  - Mandate events wait for the guarded change in step 4.
- **`GET /v0/events?after=&session=&change=&wait=`** and the MCP tool
  `events_wait` share one long poll, up to 50 seconds. They answer `last`
  and the events after `after`. A person sees owner-only events; a service
  token doesn't.
- **There is no `pigeon backplane events --follow` yet.** `pigeon` has no
  client for the control plane, and a `curl` loop through the agent's proxy
  does the same. A session arms `Monitor` (timeout 30 minutes, re-armed on
  expiry) with:

  ```sh
  HOST=pigeon-control.pigeon-dev.workers.dev SESSION="$SESSION_URL" AFTER=0 python3 -u - <<'PY'
  import json, os, subprocess, time, urllib.parse
  after = int(os.environ["AFTER"]); session = urllib.parse.quote(os.environ["SESSION"], safe="")
  while True:
      url = f"https://{os.environ['HOST']}/v0/events?after={after}&session={session}&wait=50"
      out = subprocess.run(["curl", "-sS", "--max-time", "70", url], capture_output=True, text=True).stdout
      try:
          d = json.loads(out)
      except ValueError:
          time.sleep(5)
          continue
      for e in d["events"]:
          print(e["id"], e["kind"], e["summary"], flush=True)
      after = d.get("last", after)
  PY
  ```

  It prints one line per event, which wakes the session, and carries
  `after` forward. On re-arming, `AFTER` is the last id seen.

### 2026-11-08: Step 4 lands: Web Push and the hourly digest

- **Pushes carry no payload.**
  - A push to an empty body needs only VAPID's ES256 signature (RFC 8292),
    not RFC 8291's payload encryption.
  - The service worker, `/sw.js`, then fetches `/approvals/latest.json`
    from the control plane, behind Access, and shows it. A tap opens its
    page.
  - So the Worker signs with `p256` (RustCrypto, deterministic RFC 6979
    nonces, no randomness in wasm). It encrypts nothing.
- **Subscribing.**
  - `/approvals` shows the digest above the waiting approvals, and a
    "Notify this device" button. Its script, `/approvals/push.js`, reads
    the public key from `/approvals/vapid` and posts the subscription to
    `/approvals/push`, same-origin only.
  - The `Instance` keeps up to 10 devices and drops one its push service
    answers 404 or 410 for.
  - The inbox's CSP allows its own script, its fetches and its service
    worker. Every other page still runs no script.
- **The key.** `pigeon backplane vapid-key` prints a private key for
  `pigeon cobot workers secrets put control PUSH_VAPID_KEY`, and its public
  half on stderr. Without the secret, nothing is pushed and `/approvals/vapid`
  answers 503. So this deploys before the owner sets it.
- **What is pushed:**
  - an approval offered: "Approval waiting", opening its page;
  - a mandate paused by a change outside its scope;
  - a deploy that failed. A failed deploy also pauses mandates, so that push
    covers that case;
  - the hourly digest, when it needs the owner and differs from the last
    digest pushed.
- **The digest.**
  - A cron at `17 * * * *` builds it from the instance's state and the
    events after its cursor.
  - The merges made without the owner are the `change.approved` events by
    a mandate or a standing rule.
  - `digest.show` (MCP `digest_show`) returns it now, and `/approvals`
    shows it.

### 2026-11-08: iOS installs the inbox; the inbox and digest drop dead links

- **iOS gives the push API only to a web app opened from the Home Screen**
  (iOS 16.4 and later). The owner's first try, in the Claude app's
  browser, found no `PushManager`.
  - `/approvals` now has a manifest (`/approvals/manifest.json`,
    `display: standalone`) and iOS's standalone flags.
  - The page says to use Share, then Add to Home Screen, and to tap from
    the icon. The inbox's CSP allows `manifest-src 'self'`.
- **The second try saved nothing: "Not saved: 404".** The likeliest cause
  is Access redirecting the POST to its login, which `fetch` followed as a
  GET.
  - The page now posts with `redirect: "manual"` and says when Access asks
    to sign in again.
  - It shows the answer's text, and the Worker's 404 names the method and
    path.
- **Waiting approvals were never pruned.** An entry left the inbox only
  when approved on its page. Those approved in the card, or replaced, stayed
  until their 24 hours ran out.
  - The inbox and the digest now keep only entries whose link still peeks
    (`/peek`, or `/mandates/peek`) and remove the rest.
- **`digest.show` no longer gives the agent approval links.** Only the
  owner gets them, as in the event log.

### 2026-11-08: The owner's iPhone is notified; dead links are judged by their change

- **Installed from the Home Screen, `/approvals` subscribed the owner's
  iPhone:** "This device is notified".
- **The inbox still showed 46 to 49.** Those page links were never used,
  because the card approved those changes instead, so they still peeked. An
  entry now stays only while `grants::still_applies` holds: its operation
  still applies to the change, and the change's head is the one the link was
  made for.
- **The first push reached the owner's iPhone.** Its tap opened
  `/approve/66` without its grant, "this link has no grant". The push now
  opens the approval link itself, grant included. Only the owner fetches
  it, from `/approvals/latest.json`.

### 2026-11-08: Step 5 lands: session tickets

- **`session.start` (MCP `session_start`)** takes a `label`, the session's
  URL, and returns a 24-hour ticket:
  `<label>.<generation>.<expires>.<HMAC-SHA256 under SESSION_KEY>`. The
  `Instance` keeps each label's generation and expiry.
- **The rule: a label that holds a live ticket is used only with that
  ticket, of its current generation.** A label without one works as before,
  so the changes open when this lands are unaffected. The check covers:
  - every changing operation an agent sends: the labels named by its
    `session` argument or by its change;
  - the approvals an agent asks the owner for, checked before the link is
    made;
  - pushes through the git front door to a ticketed session's change
    branch, which carry the ticket in `X-Pigeon-Ticket` (git's
    `-c http.extraHeader="X-Pigeon-Ticket: <ticket>"`).

  The ops that change something take an optional `ticket`.
- **A lost ticket is replaced, not confirmed.**
  - `session.start` refuses a label holding a live ticket unless `replace`.
    Replacing raises its generation, revokes the old ticket, and is
    recorded as `session.replaced` in the event log.
  - That differs from §5's one-tap confirm. The threat is a mistake, and
    replacing a label on purpose isn't one; the event log keeps the record.
- **Owner operations need no ticket.** Access names the owner, and a
  mandate acts as a person.

### 2026-11-08: Plan: busy Durable Objects keep old code, so objects check their version

- **What was seen.**
  - After !67 deployed, the Worker answered `session.start` but the
    `Instance` object answered "not found" for `/tickets/start`, the route
    !67 added. Routes !64 had added answered, so the object still ran !66's
    code.
  - A session's `Monitor` was long-polling the event log. Each poll has the
    Worker call the `Instance` every 2 seconds for up to 50.
  - With the monitor stopped for about a minute, the next call reached !67's
    code.
  - So an object that never goes quiet may keep its old version long after
    a deploy, and every session following events keeps it busy.
- **The cause isn't proven.** Cloudflare restarts Durable Objects on a
  deploy, but what happens to one with a steady stream of requests isn't
  documented. Step 1 measures it before anything relies on it.
- **The fix: every call to an object names the Worker's version, and an
  older object resets itself.**
  - The Worker sends `X-Pigeon-Version`, the version ID from
    `CF_VERSION_METADATA`, on each call to an `Instance`, `ChangeRun`,
    `MainRun` or job object.
  - The object compares it with its own version ID. An older object calls
    `ctx.abort()` (through `js_sys::Reflect`, since workers-rs doesn't wrap
    it) and answers 503 `stale-version`.
  - The caller retries once. Storage survives an abort, so nothing is lost.
  - A newer object never aborts for an older caller. During a deploy, an
    old Worker isolate's call reaches the new object and is served as now.
- **The long poll leans on the object less.** `events.wait` looks every 5
  seconds, not every 2, and returns on the first new event as now. A poll
  of 50 seconds then makes 10 calls, not 25.
- **The deploy checks its objects.**
  - `/healthz` asks the `Instance` and `MainRun` for their versions, and
    answers 503 until both match the Worker's.
  - The deploy job's health check already waits up to two minutes for
    `/healthz` with the new version. It now waits for the objects too, and
    reports a mismatch as a failed deploy. A failed deploy rolls back and
    pauses mandates, as any failed deploy does.
- **Loose ends:**
  - `session.end` revokes a label's ticket, open to the agent since it only
    takes power away. The trial label `ticket-trial` is ended with it.
  - Mandate 1 has one merge and some hours left. Steps 2 to 4 below stay
    inside its scope (backplane's crate) and touch no guarded file, so they
    use its last merge. The mandate is then used up.
  - The session that wrote this ADR starts its ticket with `session_start`
    and passes it from then on. A session already running keeps its MCP
    tool list from when it started, so it sends `ticket` through
    `/v0/run`, or reconnects.

#### Steps

1. **Measure.** With a monitor polling, deploy a trivial Worker change, and
   watch `/healthz`'s new object versions to see how long the `Instance`
   keeps the old one. This runs with step 3's change: `/healthz` reports
   the objects' versions before anything acts on them.
2. **Handshake.** `X-Pigeon-Version` on every call to an object; the
   abort, the 503 and the one retry; tests of the comparison.
3. **`/healthz` and the deploy job.** The objects' versions; the deploy job
   waits for them to match.
4. **Polling and tickets.** `events.wait` looks every 5 seconds;
   `session.end` added; `ticket-trial` ended.

#### Verification

- **A deploy while a monitor polls:**
  - the next call after it reaches the new code, with one
    `stale-version` retry at most;
  - `/healthz` shows matching versions, and the deploy reports passed.
- **A forced mismatch** (an object kept on an old version in a test) makes
  the deploy fail and roll back.
- **`session.end`** on `ticket-trial` frees the label; a second end is a
  no-op.
