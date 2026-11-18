## Every client on one MCP catalog: four views every tool names, one feed every reader subscribes to, and the owner's passkey signed in the client

* Date: 2026-11-17
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: runtime
* Governs: `src/**/mcp.rs`, `src/**/wire.rs`, `src/**/events.rs`, `src/**/worker/events.rs`, `src/**/push.rs`, `src/**/worker/push.rs`, `src/**/worker/apns.rs`, `src/**/worker/api.rs`, `src/**/worker/devices.rs`, `src/**/tasks.rs`, `src/**/view.rs`, `src/**/view/**`, `src/**/feed.rs`, `src/**/feed/**`, `src/**/present/**`, `src/**/worker/shell.rs`, `src/**/worker/shell/**`

## Context

The owner's goal:

1. `POST /mcp` is the main entry point for the iOS app, the web client and
   the Claude harness.
2. All three use the same MCP tools to show the data and actions they
   support.
3. Notifications and the ways events are told (the digest, approvals,
   grants, events, push) are replaced by one feed that MCP clients
   subscribe to.

The owner approved a design for it (the "One MCP Network" mockup, version
2). This ADR records its six decisions and how they are carried out.

It builds on, and amends:

- [`2026-11-15-runtime-one-mcp-entry-point.md`](2026-11-15-runtime-one-mcp-entry-point.md):
  one handler, two doors, `run()` as the authorizer. Its steps 1 to 3b are
  on `main` (the route inventory, the registry's `asks`, `capability` and
  `doors`, the session ID bound to door and caller). Its §3 (`_meta`
  proofs), §4 (the `GET` stream), §5 (kept routes) and §7 (migration) are
  re-planned here.
- [`2026-11-16-security-provider-calls-mcp-broker.md`](2026-11-16-security-provider-calls-mcp-broker.md):
  its open steps (the mirror policy, step 6, steps 7 to 11) are carried by
  this ADR's planner. Its step 8 (containers over `/mcp`) is joined with
  §6 here.
- [`2026-11-14-security-passkey-only-owner-proofs.md`](2026-11-14-security-passkey-only-owner-proofs.md):
  one proof function and the bound `pigeon-proof-v1` challenge. Its §2
  (the session gets a link to the Worker's page) changes in §4 here.
- [`2026-11-08-development-events-tickets-digest-mandate.md`](2026-11-08-development-events-tickets-digest-mandate.md):
  the `Instance`'s event log, the long poll, Web Push and the digest.
- [`2026-11-13-development-session-stages-subscriptions-breaker.md`](2026-11-13-development-session-stages-subscriptions-breaker.md):
  stages, `needs`, cursors kept by the control plane, `wake=agent`.
- [`2026-11-15-security-owner-approvals-always-listed.md`](2026-11-15-security-owner-approvals-always-listed.md):
  one inbox entry per change that needs the owner.
- [`2026-11-12-runtime-ios-app-approvals-insights.md`](2026-11-12-runtime-ios-app-approvals-insights.md):
  the app's device session, its three tabs and APNs.

### What `main` has (04635c7)

- **Routes.** `architecture::network::inventory` pins 53: mcp 4
  (`/mcp`, `/v0/operations`, `/v0/events`, `/v0/run`), git 1, app 33
  (`/v0/app/*`, the association file), owner 13 (`/approve/`,
  `/approvals*`, `/sw.js`, `/passkeys*`, `/passkey.js`, `/app/link`,
  `/devices`, `/platform/`, `/mandate/`, `/ci-window/`, `/grants/`,
  `/docs/`, `/pool-resume/`), ci 2 (`/builds/`, `/v0/apple/release`).
  `/healthz` and `POST /v0/pool/` are served beside them.
- **The catalog.** `wire::catalog()` has 73 operations. `asks()`,
  `capability()` and `doors()` are methods; `doors()` answers only
  `Access`. There is no notion of a view.
- **`mcp.rs`** answers `initialize`, `ping`, `tools/list`, `tools/call`
  and elicitation answers. It has no resources, no `GET` stream and no
  `_meta` handling.
- **Events.** The `Instance` keeps 2,000 events or 7 days under `events`,
  with cursors per session label. Kinds are free strings (about 90, in
  16 families). `/v0/events` and `/v0/app/events` read them outside
  `run()`. `pigeon session follow` long-polls `/v0/events`.
- **Telling the owner.** Six channels: `events_wait`, the hourly digest
  push, the approvals inbox (`approve::inbox`, `app/approvals.rs`), the
  grants inbox (`/grants/`), APNs (an alert with title, body and a deep
  link to the grant), and Web Push (payloadless, but the service worker
  fetches `/approvals/latest.json` for the text).
- **Proofs.** `worker/passkeys.rs::on_page` takes every proof, from eight
  pages. The app's proof (`POST /v0/app/challenge`) is unbound.
- **Clients.** The app's `Store` has one method per screen and
  `Models.swift` one struct per screen. Each owner page is HTML built in
  `approve.rs`.

## Implementation plan.

### 1. One catalog, three clients

- The iOS app, the web client, the harness, and the job containers call
  the same tools through the same handler, `mcp::serve`. A caller's
  account and role (owner, member, agent, job) decide which tools its
  `tools/list` holds, as the one-MCP ADR's §2 and the members ADR's §8
  decide now.
- The doors stay the one-MCP ADR's §2: `/mcp` behind Access (agents, job
  containers, the owner's browser) and `/v0/app/mcp`, Access bypassed,
  device-signed (the app, a standalone). Both serve the same catalog.
- No client has a private API. Every read and action a client shows is a
  tool. A screen that needs data no tool returns gets a tool first.

### 2. The rendering contract (binding)

Every tool result names one of four views, and each client implements
exactly those four. The contract is versioned (`contract: 1`). Future
design and work stay inside it, or amend it by amending this ADR first.

#### 2.1 The views

A result's `structuredContent` is one view:

| View | Shape | For |
|---|---|---|
| `list` | `title`, `items` (each a row: `id`, `title`, `subtitle?`, `badge?`, `needs?`, `progress?`, `open?`), `next?` (a cursor) | changes, approvals, sessions, the feed |
| `card` | `title`, `body?`, `fields?`, `actions?` | one feed item, one approval, a confirmation |
| `detail` | `title`, `sections` (each `heading?`, `fields`, `text?`), `actions?` | a change, a plan, insights, a deploy |
| `form` | `title`, `fields` (each `key`, `label`, `input`, `required`, `help?`), `submit` (an action) | a mandate, an Auto window, a profile, an owner ask |

- **A field** is `label` and a typed `value`. The value kinds are closed:
  `text`, `code`, `sha`, `time`, `duration`, `count`, `link`, `progress`
  (`stop`, `of`, `mark`), `series` (labelled numbers, drawn as bars or a
  sparkline).
- **An input** is `text`, `choice` (with options), `toggle`, `number` or
  `secret` (never echoed, never logged).
- **An action** is `label`, `tool`, `arguments`, `style` (`primary`,
  `plain`, `destructive`) and `proof` (true when the tool is owner-tier,
  §4). `open` on a row is an action. A client never builds a call the
  server didn't hand it, except the root tools (§2.3) and the feed.
- **Text for the harness.** `content[0].text` is rendered by the server
  from the view (`view::text`), so the harness shows the same view as
  text, with each action's tool and arguments.

#### 2.2 How it is enforced

1. **Declared.** `wire::Operation` gains a required `view: View` field.
   `View` is a closed enum of the four views in `pigeon-core/src/view.rs`,
   with the value kinds and inputs. There is no default, so an operation
   doesn't compile without one. `tools/list` puts it in each tool's
   `_meta["dev.pigeon/view"]`, and `initialize` answers
   `_meta["dev.pigeon/contract"]: 1`.
2. **Checked on every call.** `run()` validates each result against its
   operation's view. A result of another view, or one that doesn't
   validate, is an error (`view.mismatch`), never sent.
3. **Tested.** A registry test in `wire/registry_tests.rs`: every
   operation declares a view. A suite test: every catalog operation has a
   fixture call whose result validates; an operation without one fails
   it.
4. **Guarded.** `pigeon-core/src/view.rs` joins `guarded` in
   `.config/approvals.yaml`, so changing the contract is the owner's,
   and its change cites an amendment here.
5. **The clients hold no tool-specific screen.** A tree policy,
   `.config/policies/view-contract.yaml`:
   - iOS: a SwiftUI `View` is declared only under
     `src/pigeon-ios/PigeonCore/Contract/` (the four renderers and their
     parts) and `src/pigeon-ios/Pigeon/Shell/` (tabs, sign-in, the passkey
     sheet);
   - web: rendering code lives only in the shell's `views.js`;
   - both: a tool name appears as a literal only in each client's
     `roots` file (§2.3).

#### 2.3 Navigation

- Each client's `roots` file lists its root tools, one per tab or menu
  entry: `tracker.show` (list), `approval.list` (list), `insights.show`
  (detail), `feed` (§3). Everything else is reached by an `open` or an
  action the server returned.
- The shell holds only the roots, sign-in, the passkey sheet and the
  feed's badge.

### 3. One feed: `pigeon://feed`

#### 3.1 The resource

- `pigeon://feed` is an MCP resource, filtered per reader:
  - the owner reads every item;
  - a member reads items about their own changes and anything with
    `audience: all`;
  - a session reads items about the changes it holds, the tasks it
    claimed, its briefs and itself;
  - a job reads nothing.
- `resources/subscribe` on it; the server sends
  `notifications/resources/updated` on the session's `GET` stream (the
  one-MCP ADR's §4, which now carries only that) when a new item is for
  that reader.
- `resources/read` takes `pigeon://feed?after=<id>`, `after=session` (the
  reader's kept cursor, advanced by the read), `needs=owner|agent`,
  `open=1` (only items not yet resolved) and `wait=<seconds>` (at most 25,
  for a long poll). It answers a `list` view of cards and `next`.
- **`feed.read`**, a tool with the same arguments and answer, is for a
  client that holds only tools (the harness). It is the feed, not a second
  log.

#### 3.2 Items

The `Instance`'s event log becomes the feed's store. An item is:

```
{ "id": 47729, "at": 1794942973, "type": "approval.pending",
  "needs": "owner", "subject": { "change": 1034 },
  "summary": "one line", "resolves": null,
  "open": { "tool": "approval.show", "arguments": { "grant": "…" } } }
```

- `id` continues the event numbers, so cursors survive the cut-over.
- `type` is one of a closed set in `core::feed`, each with its `needs`
  and audience:

| Type | Needs | Audience |
|---|---|---|
| `approval.pending` | owner | owner |
| `grant.pending` | owner | owner |
| `owner.ask` | owner | owner |
| `owner.answered` | agent | the asking session |
| `change.stage` | the stage's `needs` | the change's session; the owner when `needs` is owner |
| `task.waiting` | agent | the task's session |
| `deploy.failed` | owner | all |
| `deploy.passed` | none | all |
| `pool.paused` | owner | owner |
| `digest` | owner when it holds anything, else none | owner |
| `session.ended` | agent, for its archiver | the parent and the owner |
| `provider.refused` | none | owner |
| `note` | none | the subject's readers |

  Every event kind maps to exactly one type; a kind no one acts on is a
  `note`. `wake=agent` becomes `needs=agent`.
- `resolves` names the item it closes (an answered approval, a granted
  grant, a merged change). `open=1` is the set no later item resolves.
- Items hold no secrets. A grant key is in `open` only for the owner.

#### 3.3 What becomes a view of the feed

| Today | Becomes |
|---|---|
| `events_wait`, `/v0/events`, `GET /v0/app/events` | `feed.read`, then the resource |
| `pigeon session follow` | an MCP client of `/mcp` that subscribes to `pigeon://feed` and prints one line per item for `Monitor` |
| the approvals inbox (`/approvals`, `approval.list`, `GET /v0/app/home`) | `approval.list`: the open `approval.pending` items, as a list |
| the grants inbox (`/grants/`) | the open `grant.pending` items |
| the hourly digest push | a `digest` item, opened with `digest.show` (detail) |
| `approval_wait`'s link card | the agent's `pending` answer names its feed item; the owner approves in the app or the web client |

The always-listed invariant holds unchanged: one open `approval.pending`
item per change that needs the owner, re-issued per head.

### 4. Passkey proofs in the client

Two tools replace the approve pages:

1. **`proof.challenge {grant}`** answers a `card`: what is approved, built
   from the grant's operation, target and head, never from the caller
   (title, head, scope, checks, the plan's digest), and the WebAuthn
   request options. The challenge is the bound `pigeon-proof-v1` one,
   minted from the grant.
2. The client signs: natively in the app (associated domain,
   RP `platform.pigeon.dev`), with `navigator.credentials.get` in the web
   client, which is served from the RP's origin.
3. **`proof.submit {grant, credential, client_data, authenticator_data,
   signature}`** passes the assertion to the one proof function
   (`worker/passkeys.rs::on_page`, renamed `proof::verify`), which
   recomputes the challenge, then redeems the grant, runs its operation
   and records `by`, `via` (`app`, `web`) and the proof.

- **Who may call them.** Both are listed only for a session resolved to a
  person (a linked device, or the owner's browser sign-in), never for an
  Access service token, a ticketed agent or a job. The harness can't
  produce an assertion; it gets the `approval.pending` item, which needs
  the owner.
- **Where the assertion goes.** It is an argument only of `proof.submit`,
  which no model is ever listed, and is recorded as the approval's
  evidence and nowhere else: `run()` redacts it from events and logs.
  This replaces the one-MCP ADR's §3 `_meta` proof.
- **Pages.** `/approve/…`, `/grants/…`, `/mandate/…`, `/ci-window/…`,
  `/platform/…` and `/pool-resume/…` become views in the web client: each
  old path serves the shell, which opens `proof.challenge` for its grant,
  so links already sent still work.
- Windows, mandates and policy still answer before a proof is asked, and
  never cover the owner tier, guarded paths, asks or credential grants.

### 5. Push wakes, and carries no content

- **APNs** sends `{"aps": {"alert": {"title": "pigeon", "body": "<n>
  need you"}, "badge": <n>, "content-available": 1}}`: a count of open
  items that need the owner, and nothing else. No title, change, link or
  grant.
- **Web Push** stays payloadless; the service worker reads the feed
  (`feed.read`, `needs=owner`, `open=1`) instead of `latest.json`, and
  shows the same count.
- One push per new owner item, at most one a minute; an item resolved
  before the push leaves lowers the count.
- The client opens on the feed and reads it.

### 6. Job containers on MCP

- Job containers and CI call `/mcp` with their own service tokens, as the
  broker's step 8 has it; their account lists only their tools.
- **`build.*`**, replacing `/builds/…`:
  - `build.list {sha}` (list): the files and their SHA-256;
  - `build.read {sha, name, offset}` (card): up to 4 MiB of the file,
    base64, with `next`. The caller checks the whole file's SHA-256.
    `session.sh`'s bootstrap reads it with `curl` and `base64 -d`.
- **`release.apple {…}` and `release.apple.archive {…}`**, replacing
  `/v0/apple/release` and its archive: the Buildkite step calls them with
  its OIDC token, verified as now, on the device-signed door, whose
  account lists only those two tools.

### 7. What stays outside MCP

| Route | Why |
|---|---|
| `/git/…` | git smart HTTP |
| `/.well-known/apple-app-site-association` | Apple fetches it |
| `/healthz`, `/healthz/smoke` | the deploy's probe |
| `/`, `/sw.js`, `/passkey.js`, `/docs/` assets | the web client's static shell; no data, no route logic |
| `POST /v0/pool/` | the Messages API's shape, which the harness speaks to its model; asked of the owner (step 1) |

The Worker, the Durable Objects, the stores, the job containers and the
external services don't change.

### 8. Migration

The MCP tools and the feed land first. Each client moves over behind its
own change. A route goes only once no client calls it:

- every route outside §7 counts its calls per client (`X-Pigeon-App`, a
  job's account, a session's label), shown in `insights.show`;
- a route with no calls for 14 days is removed by its own change; the
  app's routes follow the one-MCP ADR's `MIN_APP_BUILD` rule (§7 step
  10) first;
- the inventory test pins each removal, ending at §7's list and the two
  MCP doors.

### Execution

Each step is its own change. "(planner)" steps touch guarded, secrets or
gate paths, or settle a boundary; the rest are briefs, preferring sonnet,
at most three in the merge queue at once. Code starts after !1033. Each
step that adds a file amends `Governs` with its glob.

1. **This ADR**, with amendments to the seven ADRs above, and an
   `owner_ask` for §7's `POST /v0/pool/`. (planner)
2. **The broker's mirror policy**, `provider-call-mirror.yaml`, if
   pigeon-eviction v0.1.23 reads `target: provider-call`. (planner)
3. **The contract in the core:** `view.rs` (views, values, inputs,
   validation, `view::text`), `Operation.view` on all 73 operations, the
   registry test, `tools/list`'s `_meta`, `initialize`'s contract. (brief)
4. **`approvals.yaml` guards `view.rs`; the client policy**
   `view-contract.yaml`, its paths allowed empty until the clients land.
   (planner, owner approves)
5. **Results become views**, in three briefs by family (changes, briefs,
   sessions and tasks; approvals, grants, mandates, platform, pool and
   deploys; the rest), with `run()`'s check and the fixture suite.
   (briefs)
6. **The feed in the core:** `core::feed` (types, needs, audience,
   `resolves`, reader filter, the kind map), tested on fixtures. (brief)
7. **The feed in the Worker:** the `Instance`'s log serves items;
   `resources/list`, `resources/read`, `resources/subscribe`, the `GET`
   stream's notifications, `feed.read`; `events.wait`, `/v0/events` and
   `/v0/app/events` become shims over it. (brief; Worker)
8. **Producers write typed items:** listed approvals, grants, owner asks,
   stages, deploys, pools, the digest item, session ends. (brief)
9. **Proofs in the client:** `proof.challenge`, `proof.submit`,
   `proof::verify`, the redaction. (planner; gate paths, owner approves)
10. **Push wakes only:** APNs payload, the service worker on the feed.
    (brief; owner approves)
11. **The one-MCP ADR's rest:** the §8 authorizer in `run()`; the app's
    operations as tools with views; `/v0/app/mcp`. (briefs;
    `worker/devices.rs` is a gate path, so its part is the planner's)
12. **Job containers:** `build.list`, `build.read`, `release.apple`,
    `release.apple.archive`; the containers, `session.sh` and the harness
    move. (briefs; `worker/apple.rs` and `signing.rs` are the planner's)
13. **The harness:** `pigeon session follow` on the feed; CLAUDE.md's
    follow text. (brief; CLAUDE.md by the planner)
14. **The web client:** the shell, `views.js`, the roots, each owner page
    as a client view, old paths serving the shell. (briefs)
15. **The iOS app:** `MCPClient` in PigeonCore, the four renderers, the
    roots, the passkey sheet on `proof.*`; TestFlight. (briefs; the
    owner's phone)
16. **The broker's step 6 and steps 7 to 11**, interleaved after step 7,
    as its hand-off lists them. (planner and briefs)
17. **Routes go** as §8 counts them idle, one change per group. (briefs)
18. **This ADR is amended with the outcome.** (planner)

## Amendments

### 2026-11-17: The owner's answer: two doors, one handler

The owner chose to keep the one-MCP ADR's §2 doors over a single `/mcp`
that verifies every credential itself:

- `/mcp` stays behind Access, so Access remains the outer gate for agents,
  jobs and the owner's browser;
- `/v0/app/mcp` stays device-signed, with Access bypassed, for the app and
  standalones, and takes Buildkite's OIDC token for `release.apple` (§6);
- both serve one handler and one catalog.

§1 and §6 stand as written.

The owner also answered owner ask #27: `POST /v0/pool/` stays outside MCP,
protocol-bound as `/git/…` is, because the harness speaks the Messages API
to its model there. §7's list is final.

### 2026-11-17: A view is presented beside the result, so no caller breaks mid-migration

Agents, the follower and `session.sh` read today's results: `tools/call`'s
`content` text and `structuredContent` are the operation's `Response`, and
`/v0/run` answers it raw. Replacing them with views would break those
callers between merges. So §2.2's step 2 lands additively:

- **`present`.** Each operation gets a pure presenter in pigeon-core,
  `fn(&Response) -> Value`, building its declared view from its result. It
  sits in `src/pigeon-core/src/present/` (one file per family:
  `change.rs`, `approval.rs`, `rest.rs`), behind
  `present::view(op, &Response) -> Option<Value>`.
- **Where the view goes.** `tools/call`'s result gains
  `_meta["dev.pigeon/view"]`, the presented view. `content` and
  `structuredContent` stay the `Response` until step 13 moves the harness
  to `view::text`. `/v0/run` and in-process callers keep the raw result
  until step 17 removes them.
- **The check, per operation.** `present::PRESENTED` lists the operations
  with a presenter. `mcp::answer` validates each listed operation's view
  against its declared one; a mismatch is the error `view.mismatch`, never
  sent. A suite test fails while `PRESENTED` differs from the catalog, with
  the missing names, and pins how many are missing, so each family's brief
  lowers it and none raises it.
- **Step 5 is four briefs:** 5a, `present`'s frame, `PRESENTED` empty, the
  check and the pinned test; then 5b (changes, briefs, sessions, tasks),
  5c (approvals, grants, mandates, platform, pool, deploys) and 5d (the
  rest), in parallel.
- **Clients read `_meta["dev.pigeon/view"]`** (steps 14 and 15); the
  harness reads `view::text` from step 13.

### 2026-11-17: Governs takes the files steps 3a and 6 added

Steps 3a (!1051) and 6 (!1052) added `src/pigeon-core/src/view.rs` and
`src/pigeon-core/src/feed.rs`, each with tests in a folder of its name.
`Governs` gains `src/**/view.rs`, `src/**/view/**`, `src/**/feed.rs` and
`src/**/feed/**`; step 5a adds `src/**/present/**`.

### 2026-11-17: The fast lane's items join the feed

Per
[`2026-11-17-security-owner-fast-lane-window.md`](2026-11-17-security-owner-fast-lane-window.md)
§7 and §8, §3.2's closed set of types gains:

| Type | Needs | Audience |
|---|---|---|
| `lane.open` | none | all |
| `lane.ended` | none | all; it resolves its `lane.open` |
| `lane.broken` | owner | all |

- A lane's request is an `approval.pending` item with subject `{lane: id}`;
  `lane.extended` and `lane.deferred` are `note`s, and `job.waived` is a
  `note` for the change's readers.
- `approval.list` lists the open `lane.open` items above the pending
  approvals, each with an `End` action.
- The lane's operations declare views as that ADR's §8 lists; the four
  views and their value kinds are unchanged.
### 2026-11-17: Governs takes step 5a's present/

Step 5a (!1059) added `src/pigeon-core/src/present/`; `Governs` gains
`src/**/present/**`, as the presenting amendment said it would.

### 2026-11-17: Step 8's shape: an event names its subject, and the log links a closing event to the item it closes

§3.2's `resolves` needs each closing event to name the event it closes.
No producer keeps the `{id}` that `/events/add` answers, so callers can't
name it. The log links them instead, by a subject both ends share.

- **`Event` gains `subject` and `resolves`,** both optional and left out
  when unset. A producer passes a subject; it never passes `resolves`.
- **The log decides.** `core::events` holds two tables:
  - kinds that open a subject: `owner.asked`, `grant.requested`,
    `approval.pending`, `pool.paused`, `auto.paused`, `auto.held`,
    `task.made`, `mandate.proposed`;
  - kinds that close it: `owner.answered`, `owner.withdrawn`,
    `grant.granted`, `grant.failed`, `grant.rotated`, `grant.revoked`,
    `pool.resumed`, `auto.resumed`, `auto.ended`, `task.done`,
    `mandate.granted`, `mandate.ended`, `ci.window`.

  A closing event's `resolves` is the newest earlier event with the same
  subject, of an opening kind, that nothing resolves yet. An opening event
  never resolves another, so a doubled `approval.pending` stays one open
  item. `feed::item` copies `resolves`.
- **Subjects:** `owner:<item id>`, `grant:<offer id>`, `platform:<run id>`,
  `pool:<name>`, `ci:<offer id>`, `task:<id>`, `mandate:<id>`,
  `window:<id>`. A change's items keep closing by their change, as
  `feed::open` does now.
- **Gaps filled:**
  - `tasks::add` keeps the task id it is answered, and `task.done`
    appends a `task.done` event;
  - `mandate.propose`, the mandate's grant and `mandate.end` append
    `mandate.proposed`, `mandate.granted` and `mandate.ended`;
  - the hourly digest appends a `digest` event, `owner` true, when it
    holds anything; its push stays a count (§5).
- **What has no pair stays as it is:** deploys (keyed by SHA only) and
  `session.ended` close nothing.
- **Step 8 is three changes:**
  - 8a, a brief: the core (`Event.subject` and `resolves`, the tables, the
    link, `feed::item`), with tests;
  - 8b, a brief: `/events/add` reads `subject` and sets `resolves`, and the
    producers outside the gate pass subjects (owner items, platform runs,
    the pool, the CI window, tasks, mandates, the digest);
  - 8c, the planner's: `worker/minting.rs` passes `grant:<offer id>`. It is
    a gate path, so the owner approves it.

### 2026-11-17: Step 13's shape: items carry the stage and today's wake rule, so the follower's lines don't change

Moving `pigeon session follow` onto `feed.read` as §3.3 has it would change
what a session sees:

- its line prints the event's stage (`authorized needs=agent`), which an
  item doesn't carry;
- `needs=agent` isn't `wake=agent`: `deploy.failed` wakes a session today
  but needs the owner in the feed, and `session.ended` is the reverse.

So the feed keeps both, and `needs` stays what it is for the clients:

- **`Item` gains `stage`,** copied from the event (left out when unset), and
  **`wakes`**, `events::wakes` of the event (left out when false).
- **`Query` and `feed.read` gain `wake`:** `wake=agent` keeps the items
  that wake their session, as `/v0/events?wake=agent` does today.
- **Step 13 splits:**
  - 13a, a brief: `Item.stage` and `Item.wakes`, `Query.wake`, and
    `feed.read`'s `wake` argument, with tests;
  - 13b, a brief: `pigeon session follow` calls `feed.read` through
    `POST /v0/run` (`after=session`, `wake=agent`, the session's label and
    hold) and prints each item exactly as it printed each event; the core's
    `classify`, cut and backoff rules are unchanged.
- `/v0/events` and `events.wait` become shims over the feed (7c) only after
  13b has merged and every session runs it.

### 2026-11-18: Steps 14 and 17's shape: owner pages as shell views, an internal page table, route groups

The owner asked for the old routes to go and the owner pages to become
client views that keep the old pages' flow and look. On `main` (955163f) the
shell has four roots and a bare proof screen; every owner page is still
served, `/v0/app/*` still serves the iOS Now and Insights tabs, and §8's
per-route count doesn't exist.

#### 1. The page handlers leave the public router

- `worker/mod.rs`'s `SURFACES` keeps the MCP doors, §7's routes and the
  shell. The owner pages' handlers (`approve`, `mandates`, `lanes`,
  `minting`, `ci_window`, `platforms`, `pool_ops`) move to a `PAGES` table
  that only `page_post` reads, so `proof.submit` still reaches each page's
  decision unchanged.
- `GET` on an old page path (`/approvals`, `/approve/…`, `/mandate/…`,
  `/lane/…`, `/grants/…`, `/ci-window/…`, `/platform/…`, `/pool-resume/…`,
  then `/app/link`, `/devices`, `/passkeys…`) serves the shell's
  `index.html`; `app.js` routes the path and query into its view, so links
  already sent keep working. A `POST` to one answers 410.

#### 2. Each page kind is a view, inside contract 1

- `proof.challenge {page}`'s result is presented as a `card` built from the
  grant, never from the caller: what the old page showed above its form
  (title, head, scope, checks, the plan's digest, the change as an `open`),
  and its buttons as actions: the grant's verb (`proof: true`, tool
  `proof.submit`), `Refuse` (`destructive`) where the page had it, and
  `Back to approvals` (`approval.list`).
- A page with inputs (asks, `review`, `account_token`) gets
  `proof.form {page}`: a `form` view of those inputs whose submit is
  `proof.submit`, `proof: true`. The card's primary action opens it.
- The WebAuthn options stay in `structuredContent`. A client signs when it
  runs an action with `proof: true`, and merges the assertion into that
  action's arguments. `view.rs` doesn't change.
- The presenters are pure, in `present/approval.rs`; their wiring in
  `worker/passkeys.rs` is the planner's (gate).

#### 3. The look is the owner pages'

- `index.html` drops its own palette and inlines `look/tokens.css` and
  `look/style.css` from pigeon-core, with the 40 px avatar header, per
  [`2026-11-16-runtime-owner-pages-app-look.md`](2026-11-16-runtime-owner-pages-app-look.md).
- `views.js` emits the plain elements `style.css` already styles: a `dl`,
  `li` or `pre` as a card, the primary action as a submit-styled button.
  The roots are the app's chips.

#### 4. The tools the clients still lack

- `approval.list` (list): open `lane.open` items with `End`, then one row per
  open `approval.pending`, `open` on `proof.challenge`. It is the Approvals
  root on both clients, replacing the Feed root's stand-in.
- Not gate: `account.show` (whoami), `profile.show`, `profile.update`,
  `instance.list`, `instance.standup`, `member.list`, `member.invite`,
  `member.admit`, `member.join`, `learner.*`.
- Gate, the planner's: `device.list`, `device.link`, `device.remove`,
  `device.redeem` (the link-code redeem, on `/v0/app/mcp` signed by the new
  key), `passkey.list`, `passkey.add`, `passkey.remove`, `push.subscribe`.
  Owner-tier ones are listed only on a person's door, as the proof tools
  are. Registering a passkey is the shell's moment, beside the proof.

#### 5. §8's count

- One step in `worker/mod.rs`'s fetch counts each call outside §7 and the
  MCP doors by route group and client (`X-Pigeon-App` and its build, a
  job's account, a session's label, a browser), daily, in the `Instance`.
- `insights.show` gains an "Old routes" section: each group's calls per
  client and days since the last call.
- Tests check the counting on fixtures; no test lists routes.

#### 6. Groups, removed in this order once idle 14 days

| Group | Routes | Its clients move in |
|---|---|---|
| A, owner pages | the paths in §1, `/passkey.js`, `/passkeys/challenge` | the shell steps |
| B, Web Push | `/approvals/push.js`, `/vapid`, `/push`, `/latest.json`, `/manifest.json` | `push.subscribe`; the manifest is the shell's |
| C, app REST | `/v0/app/*` but `/v0/app/mcp` | the iOS build after `MIN_APP_BUILD` |
| D, agent REST | `/v0/run`, `/v0/operations`, `/v0/events`, `events.wait` | the harness and `session.sh` on `/mcp` |
| E, CI | `/builds/`, `/v0/apple/release` and `/archive` | the containers on `build.*`; `release.apple` |

- Group A's paths are served by the shell until idle, then removed with
  `PAGES`, whose handlers `proof.submit` then calls directly.
- Each removal deletes the code only it used, and edits the route inventory
  as the blue-green ADR's lean tests have it.

#### Execution

Replaces steps 14 and 17's remainder. "(planner)" is a gate path.

- R1. §8's count (§5). (brief)
- R2. Step 8c, `worker/minting.rs` passes `grant:<offer id>`. (planner)
- R3. The §8 authorizer in `run()`. (planner)
- R4. The shell's look (§3), with 390 px screenshots in both schemes.
  (brief)
- R5. The proof presenters and `proof.form` (§2): a, the core (brief); b,
  the wiring (planner).
- R6. `approval.list` and the Approvals root, web and iOS. (brief)
- R7. `PAGES`, and the shell on the old page paths (§1). (brief)
- R8. The tools of §4: a, not gate (brief); b, gate (planner).
- R9. The web views for linking, devices and passkeys. (brief)
- R10. iOS: Now and Insights onto the shell's roots, the proof sheet on
  card and form; TestFlight, then `MIN_APP_BUILD`. (brief; the owner's
  phone)
- R11. The harness and `session.sh` on `/mcp`. (brief)
- R12. The broker's step 6 and steps 7 to 11, as step 16 has them.
- R13 to R17. Groups A to E removed, each as R1 counts it idle. (briefs)
- R18. This ADR amended with the outcome. (planner)

### 2026-11-18: The owner's answers: routes go once replaced and verified, and links move to `/app#`

The owner answered two points of the amendment before this one:

1. **No idle wait.** A route group goes as soon as its replacement is
   deployed and verified live, not after 14 idle days:
   - pages: a Playwright check of the shell view on the Bench, then a live
     look;
   - tools: a live call;
   - group C: the iOS build on its replacement installed on the owner's
     phone, then `MIN_APP_BUILD`.

   §8's count is optional and R1 is dropped.
2. **Old paths don't serve the shell.** Links already sent may stop
   working. Every link minter (change approvals, minting grants, mandates,
   lanes, the CI window, platform runs, pool resumes, the inbox's rows,
   push and feed items) emits `https://<host>/app#<page path>`, e.g.
   `/app#/approve/12?grant=…`. The shell routes the hash into
   `proof.challenge {page}`. The page path stays the grant's name inside
   `proof.challenge` and `proof.submit`.

So the earlier amendment changes:

- §1: the page handlers still move to `PAGES` for `page_post`, but `GET`
  on an old path is not served; the paths go with group A.
- §5 is dropped. §6's table stands; its "once idle 14 days" becomes "once
  verified".
- Execution: R1 is dropped. R7 is now two changes: R7a (planner, gate
  paths), the link minters emit `/app#`; R7b (brief), the shell's proof
  flow (card, form, signing on `proof: true`) for every page kind. R13
  removes group A once R7a and R7b are verified.

### 2026-11-18: Step R8's shape: the app's tools are device-door operations that act as their device

The iOS app's Now and Insights tabs and their sheets still call `/v0/app/*`.
Every handler behind those routes keys the `Instance` by the signing
device's id, which `run()`'s `Identity` doesn't carry.

- **Operations.** Each remaining platform call becomes a catalog operation,
  named as the one-MCP ADR §1 lists them: `whoami`, `device.push`,
  `profile.show`, `profile.update`, `avatar.show`, `avatar.put`,
  `avatar.remove`, `device.unlink`, `passkey.remove`, `instances.list`,
  `instances.standup`, `members.list`. The rest already have tools:
  `home.show`, `tracker.show`, `ops.show`, `insights.show`, `feed.read`,
  `change.status`, `owner.answer`, `approval.list` and `proof.*`.
- **Doors.** `Operation::doors()` answers `[Device]` for these and
  `[Access]` for the rest. `tools/list` on a door lists only the operations
  that name it, beside the proof tools.
- **The device.** `api::Door` carries the device `/v0/app/mcp` verified
  (R8b, !1149). `serve_message` runs a device operation through the handler
  its `/v0/app/*` route calls today, as that device, before `run()`. On
  `/mcp` it is refused.
- **Views.** Each answers a view through a presenter in `present/app.rs`.
- **Not in R8.** The standalone app's `join.*` and `learner.*` routes are
  the standalone's own surface (`worker/standalone.rs`) and stay.
  `POST /v0/app/session`, the link code's redeem, signs with a key no
  device holds yet, so it stays until `device.redeem` (R8c, gate) lets the
  device door take that one call.

Execution: R8a-1 (brief): the operations, `doors()`, the listing filter and
the presenters. R8a-2 (brief): the dispatch. R8c (planner): `device.redeem`.
R10 moves the app onto them; then group C goes, but `/v0/app/mcp` and, until
R8c, `/v0/app/session` stay.
### 2026-11-18: The owner's answer #36: form-answering reads and an `image` input

The owner chose (owner item #36) to move the app's profile sheet (name,
appearance, avatar) fully onto views. Contract 1 gains two additions:

- **Forms from reads.** A Read operation may answer a `form` view whose
  submit is the write it edits, such as `profile.edit` for
  `profile.update`. Writes still answer a card or a detail. The registry
  test `no_operation_is_a_form` becomes `forms_are_reads`.
- **An `image` input.** A picked photo, sent as base64 JPEG of at most
  512 KiB, for `avatar.put`. The web client renders it as a file picker
  that accepts images (R9), the app as a photo picker (R10).
- **The version stays 1.** Older app builds reject an input kind they
  don't know, but only the new profile operations answer one, and only
  builds that know `image` call them.
- `/v0/app/profile` and `/v0/app/profile/avatar` go with group C. The
  profile operations join R8's device-door set: `profile.show`,
  `profile.edit`, `profile.update`, `avatar.show`, `avatar.put`,
  `avatar.remove`.
### 2026-11-18: Step R10b's shape: the app keeps its screens and calls the device door's tools

The owner asked that the app keep its flow and styling. Its screens read
`Store`, and `Store` reads `/v0/app/*`. Each tool on the device door runs
the handler its route runs: `app_read`'s `home.show` builds the same JSON as
`GET /v0/app/home`, and the change sheet's route is `change.status`'s result.

- **Transport, not views.** R10b keeps `NowView`, `InsightsView`, their
  sheets and `ProfileSheet`. `Store` calls the tool, through `MCPClient`, in
  place of each route, and decodes `structuredContent.result` into the type
  it decodes today.

| `Store` | Route today | Tool |
|---|---|---|
| `refresh` | `GET /v0/app/home` | `home.show` |
| `loadTracker` | `GET /v0/app/tracker` | `tracker.show` |
| `loadOps` | `GET /v0/app/ops` | `ops.show {range}` |
| `stacks` | `GET /v0/app/insights` | `insights.show {metric, source, range}` |
| `follow` | `GET /v0/app/events` | `feed.read {after, seconds}` (its `items`) |
| `changeDetail` | `GET /v0/app/changes/{n}` | `change.status {number}` |
| `answer` | `POST /v0/app/items/{id}` | `owner.answer {id, answer}` |
| `loadProfile` | `GET /v0/app/profile`, `…/avatar` | `profile.show`, `avatar.show` |
| `rename`, `setAppearance` | `PATCH /v0/app/profile` | `profile.update` |
| `setAvatar`, `removeAvatar` | `PUT`/`DELETE …/avatar` | `avatar.put`, `avatar.remove` |
| `registerPush` | `PUT /v0/app/device/push` | `device.push` |
| `loadInstances` | `GET /v0/app/instances` | `instances.list` |

- **Proofs go through the proof sheet.** `ApprovalSheet`, `approve`,
  `startAuto` and `endAuto` give way to `ProofSheet`: an approval opens
  `/approve/<id>`, as its `/app#` link does, and Auto opens
  `proof.challenge {purpose: auto|auto-end}`. Unlinking a device and
  removing a passkey use `proof.challenge {purpose, target}` (!1158) and its
  submit, `device.unlink` or `passkey.remove`, once those tools land.
  `Store.proof` and `POST /v0/app/challenge` go with them.
- **Still REST until R8c:** `instances.standup` and the link code's redeem.
- **Group C's gate.** Willow is the one owner with a linked phone, so the
  gate is her phone running a TestFlight build with R10b, not a count. When
  group C goes, the Worker answers each removed `/v0/app/*` path with `426`
  and "Update the app", which is what `MIN_APP_BUILD` was for. The
  standalone's `join.*`, `learner.*` and `members.*` routes are not group
  C, as R8's amendment has it.
- **Names.** R8's `whoami` is `device.show`: the catalog's names are two
  dotted parts. `approval.` and `push.` join `control_plane()`'s domains
  (!1182), so `run()` routes `approval.list` and `push.*`.

Execution: R10b-1 (brief): the reads, items, profile, avatar, push and
instances, once R8a-2's dispatch is live. R10b-2 (brief): approvals and Auto
through `ProofSheet`. R10b-3 (brief): unlink and passkey removal, after
!1158 and those tools. Then TestFlight for the owner, then group C's 426.
### 2026-11-18: Step R8c's shape: the link code's redeem stays a bootstrap, and standing up an instance is a device tool

R8's amendment left two app calls on REST until R8c: `POST /v0/app/session`,
which redeems a link code, and `POST /v0/app/instances/standup`.

- **The redeem joins §7.** It is the bootstrap that makes a device known: it
  is signed by a key the device door doesn't know yet, as `POST /v0/pool/`
  is shaped by its caller. Moving it onto the door would loosen the door's
  signature check for one call. So `POST /v0/app/session` stays outside MCP,
  with `/app/link`, the web page that mints the code, and `device.redeem`
  isn't made.
- **`instances.standup` is a device operation.** Its arguments are `name`,
  `label`, `token` and the proof's `credential`, `client_data`,
  `authenticator_data` and `signature`. It runs `instances::standup` as the
  signing device, which checks the proof against
  `listing::standup_purpose(sha256(token))` as now. The token is sent once
  and never kept, as now.
- **Its proof.** `proof.challenge {purpose: "instance-standup", target:
  <the token's SHA-256, hex>}` mints that purpose's challenge (gate:
  `passkeys.rs`). The app keeps its native stand-up sheet: it hashes the
  token, asks the challenge, signs it, and calls `instances.standup`.
- **Group C then** removes every platform `/v0/app/*` route but
  `/v0/app/mcp` and `/v0/app/session`, each answering 426 and "Update the
  app", as R10b's amendment has it.

Execution: R8c-1 (planner, gate): the `instance-standup` purpose in
`proof.challenge`. R8c-2 (brief): `instances.standup` on the device door and
the app's stand-up sheet through it. Then group C, once the owner's phone runs
the build.
### 2026-11-18: Outcome: one catalog serves the shell and the app, and the old routes are gone but group E

R0 to R18 ran on 2026-11-17 and 18, under the owner's answers to remove each
group once its replacement was live and verified, and to move every link to
`/app#`.

- **The shell.** `/app` renders the four views from `/mcp`, with the look's
  tokens: the owner pages are `proof.challenge`'s cards and `proof.form`'s
  forms, signed in place (R4, R5, R7), Approvals is its first root (R6), and
  its Notify button subscribes through `push.key` and `push.subscribe` (R9b).
  Every link the control plane mints is `/app#/<page>?grant=…`.
- **The app.** It keeps its screens. `Store` calls the device door's tools in
  place of each `/v0/app/*` route (R10b), and approvals, Auto, unlinking,
  removing a passkey and standing up an instance go through the proof sheet
  (R10b, R8c). Its Feed tab, the shell's views in the app, went once the owner
  tested the build; Now and Insights are its tabs.
- **The device door.** Fourteen operations name it and run as the signing
  device (R8): `device.show`, `device.push`, `instances.list`,
  `instances.standup`, `members.list`, `profile.*`, `avatar.*`,
  `device.unlink` and `passkey.remove`. `/mcp` refuses them and doesn't list
  them.
- **Removed.** Group A, the owner pages (!1147); group B, the Web Push pages'
  routes (!1210); group C, every platform `/v0/app/*` route but `/v0/app/mcp`
  and `/v0/app/session`, which answer 426, "Update the app" (!1224); group D,
  `/v0/run`, `/v0/operations` and `/v0/events` (!1176), once every running
  follower read `/mcp` (!1159, !1173, !1175). `/approvals` is gone with no
  redirect, as the owner decided.
- **Kept.** `POST /v0/app/session` joins §7 (R8c). The tool `events.wait`
  stays: it is MCP, not REST, and sessions call it. The standalone's `join.*`,
  `learner.*` and `members.*` routes are its own surface.
- **Names and fixes found on the way.** R8's `whoami` is `device.show`.
  `approval.` and `push.` joined `control_plane()` (!1182), with a registry
  test that every operation the control plane routes runs there.
- **Not done here.** Group E (`/builds/`, `/v0/apple/release` and its
  archive) waits on R12, the job containers' own service tokens.
  `session.sh` already bootstraps through `build.read` (!1173). R1's idle
  counter wasn't needed: the owner's answers replaced it. R3, the authorizer
  in `run()`, moved to the users ADR's planner, who supplies `requires()` and
  `member::check`.

### 2026-11-18: Step R3's shape: `member::check` beside `access::authorize`, in shadow, with the users ADR's step 5

The outcome's note that R3 moved to the users ADR's planner is wrong: R3
stays with this ADR's planner, and that ADR's 2026-11-18 step 9 amendment
says so. Its planner supplies `Operation::requires()` (step 4) and
`member::check` (step 3); R3 wires them in.

- **One place.** `run()` calls `authorize_member` after
  `access::authorize`. It loads `Members` from `member::STORE`, seeds the
  Service users (`seed_services`) and saves them when that changed, reads
  the roles from `standalone::roles()`, and builds `member::Request` with
  the operation's name, its `requires()`, the account, the `Own` subject and
  the scope from `scope_arg`.
- **The account.** On the device door, the device's `account`. On `/mcp`,
  an Access person is the Administrator (`ensure_owner`'s account); a
  service token is `service_for(client_id, model)`, the model being its
  ticket's class.
- **Shadow.** It decides nothing. Each call whose answer differs from
  today's is logged as `member.shadow` with the operation and the Service
  user, or `unbound` and the client ID's `token_id`, never the ID. It
  decides only when the users ADR's step 9 amendment says.
- **Step 5.** Each `who.human` gate becomes an Administrator check: an
  Access person on `/mcp`, or a device whose account is the
  Administrator's. A Service user never passes it.
