## Every client on one MCP catalog: four views every tool names, one feed every reader subscribes to, and the owner's passkey signed in the client

* Date: 2026-11-17
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: runtime
* Governs: `src/**/mcp.rs`, `src/**/wire.rs`, `src/**/events.rs`, `src/**/worker/events.rs`, `src/**/push.rs`, `src/**/worker/push.rs`, `src/**/worker/apns.rs`, `src/**/worker/api.rs`, `src/**/worker/devices.rs`, `src/**/tasks.rs`

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
