## One MCP entry point: agents, the app and the owner's pages call the same operations, and an owner's passkey travels in a call's `_meta`

* Date: 2026-11-15
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: runtime
* Governs: `src/pigeon-cli/crates/backplane/src/mcp.rs`, `src/pigeon-cli/crates/backplane/src/wire.rs`, `src/pigeon-cli/crates/backplane/src/routes.rs`, `src/pigeon-cli/crates/backplane/src/worker/api.rs`, `src/pigeon-cli/crates/backplane/src/worker/app.rs`, `src/pigeon-cli/crates/backplane/src/worker/devices.rs`, `src/pigeon-cli/crates/backplane/src/worker/mod.rs`, `src/pigeon-ios/Pigeon/API.swift`

## Context

The owner's goal: one MCP-based entry point that the web, the iOS app and
agents all use. The control Worker today serves 40 routes in five groups:
MCP clients 4 (`/mcp`, `/v0/operations`, `/v0/events`, `/v0/run`), git 1,
the iOS app 20 (`/v0/app/*`, AASA), the owner's browser 13 (`/approve/`,
`/approvals*`, `/ci-window/`, `/app/link`, `/devices`, `/docs/`,
`/mandate/`, `/grants/`, `/passkeys*`, `/platform/`, `/sw.js`, `/healthz*`)
and CI 2 (`/builds/`, `/v0/apple/release`).

What the code does now:

- `/mcp` (`worker/api.rs`, `mcp.rs`) is POST only, answers `initialize`,
  `ping`, `tools/list`, `tools/call`, and exposes `wire::CATALOG`'s 58
  operations. `Mcp-Session-Id` is minted and never checked; there is no
  GET stream, no resources. `/v0/run` calls the same `run()`;
  `/v0/events` calls `events::wait` outside `run()`, unaudited.
- Owner operations are a fixed list (`core/approval/grants.rs::OPERATIONS`),
  not a field of the registry. `run()` checks `Effect::Apply` needs a
  human; the members ADR's §8 order (`core/member::check`) is unwired.
- Of the 20 app routes only `POST /v0/app/approvals/<id>` runs an
  operation; the rest call Durable Objects directly. Its passkey proof is
  unbound (purpose `grant:<key>` only), weaker than the page's
  `pigeon-proof-v1` challenge, which binds op, target and head.
- Each page re-implements its grant, challenge and proof plumbing.

It builds on
[`2026-11-14-security-passkey-only-owner-proofs.md`](2026-11-14-security-passkey-only-owner-proofs.md)
(one proof function, bound challenges),
[`2026-11-14-security-instance-members-roles-invitations.md`](2026-11-14-security-instance-members-roles-invitations.md)
§8 (the check order),
[`2026-11-15-security-owner-approvals-always-listed.md`](2026-11-15-security-owner-approvals-always-listed.md),
[`2026-11-15-security-standing-auto-owner-flags.md`](2026-11-15-security-standing-auto-owner-flags.md),
[`2026-11-12-development-grants-applies-rescue-yield.md`](2026-11-12-development-grants-applies-rescue-yield.md),
[`2026-11-12-runtime-ios-app-approvals-insights.md`](2026-11-12-runtime-ios-app-approvals-insights.md)
(device session, three tabs) and
[`2026-11-15-runtime-standalone-hostnames-platform-zone.md`](2026-11-15-runtime-standalone-hostnames-platform-zone.md).
MCP references are to the 2025-11-25 specification (streamable HTTP,
`_meta`, authorization), which `mcp.rs` already negotiates.

## Implementation plan.

### 1. Every read and action is an operation

- `wire::Operation` answers `asks` (`none`: runs on the caller's
  capability; `grant`: a mandate, window, policy or change grant answers,
  else `pending` for the owner; `page`: the owner proves on its page each
  time), `capability` (the members ADR's list; what the caller needs to
  ask) and `doors` (§2). `grants::OPERATIONS` becomes the `grant` set.
  Tiers stay the control plane's, per change.
- Each `/v0/app/*` route and page action becomes an operation, its pure
  part in the core: `home.show`, `tracker.show`, `ops.show`,
  `insights.show`, `approval.list`, `approval.show`, `approval.decide`,
  `auto.start`, `auto.end`, `owner.answer`, `whoami`, `profile.show`,
  `profile.update`, `avatar.put` (base64, at most 512 KiB), `avatar.remove`,
  `device.push`, `device.link`, `device.redeem`, `device.unlink`,
  `passkey.remove`, `members.list`, `members.invite`, `members.admit`,
  `join.*`, `instances.*`, `learner.*`, `push.subscribe`, `docs.view`,
  `proof.challenge`; the existing `mandate.*`, `grant.*`, `ci.window`,
  `platform.*` pages' decisions become `<op>.decide`.
- `run()` is the one authorizer, in the §8 order: who, membership,
  capability, ticket, tier (mandate, window, policy, change grant), proof,
  separation, record. `events.wait` goes through it too.
- MCP resources serve read-only documents only (`pigeon://docs/…`, an ADR,
  a change's plan); everything the app or pages call is a tool, with one
  input schema.

### 2. One handler, two doors

`mcp::serve` is reachable at:

| Door | Edge | Callers | Credential |
|---|---|---|---|
| `/mcp` | Access | agents, the owner's browser | Access service token; or Access plus the `sign-in` cookie |
| `/v0/app/mcp` | Access bypassed, as `/v0/app/*` is | the app; a standalone's only path | `X-Pigeon-Device` signature over `POST /v0/app/mcp` and the body |

- Each caller resolves to an account before any operation (members §8.1);
  an Access email names no one. `device.redeem` and `join.*` alone accept a
  key the request itself presents, as `/v0/app/session` does now.
- An operation names its doors; a tool absent from a door is absent from
  that door's `tools/list`.
- `Mcp-Session-Id` is an HMAC over the account, door and capabilities,
  checked on every later request; a mismatch is 404, per the spec.
- Agents keep `ticket`. The `sign-in` cookie needs a same-origin `Origin`
  and `Content-Type: application/json` on every POST.
- OAuth 2.1 authorization is not adopted: Access, the device key and the
  passkey already authenticate, and a token issuer adds a secret to guard.
  Revisit when a third-party MCP client needs `/mcp`.

### 3. An owner's proof travels in `_meta`

1. The client calls the owner-tier tool; with no proof, the answer is
   `pending` with the listed grant, as now (always-listed §3).
2. The client calls `proof.challenge {grant}`; the Worker mints the
   `pigeon-proof-v1` challenge from the grant's op, target and head,
   never from the request.
3. The owner asserts with their passkey (native on the app, `passkey.js`
   in the browser; RP `platform.pigeon.dev`).
4. The client repeats the call with `_meta["dev.pigeon/proof"]`:
   credential ID, client data, authenticator data, signature.
5. `run()` passes it to the one proof function (`proof::on_page`, renamed
   `proof::verify`), which recomputes the challenge, then redeems the
   grant and records `by`, `via` (`app`, `page`) and `proof`.

The proof is never an argument, so no schema, model or log carries it. A
device signature, Access or a cookie alone proves nothing. The app's
proofs become bound, closing today's gap. Windows, mandates and policy
still answer before a proof is asked for; the owner tier, guarded paths,
asks and credential grants are never covered.

### 4. Events stream on `GET` of either door

- `GET /mcp` with `Accept: text/event-stream` opens a stream served by the
  `Instance`. Each SSE `id` is the event number; `Last-Event-ID` resumes.
  The stream closes after 5 minutes and the client reconnects.
- `events.wait` stays a tool for long-poll clients. The follower moves to
  it; `/v0/events`, `/v0/run` and `/v0/operations` become aliases of
  `events.wait`, `tools/call` and `tools/list`, then go (§7).

### 5. What stays outside `/mcp`

| Route | Why |
|---|---|
| `/git/…` | git smart HTTP |
| `/.well-known/apple-app-site-association` | Apple fetches it |
| `/v0/apple/release` (and `/archive`) | Buildkite OIDC callback |
| `/builds/…` | job Containers fetch binaries by SHA-256 |
| `/healthz`, `/healthz/smoke` | the deploy job's probe |
| `/connect/callback` | a provider's OAuth redirect URI |
| the web shell `/*` | HTML, `/sw.js` at the root scope, `/passkey.js`, the manifest |

The shell holds no data and no route logic beyond serving assets: every
page, including `/approve/<grant>` links already sent, renders from MCP
calls. WebAuthn needs no route of its own. 40 routes become 9.

### 6. The app's on-device coordinator

The app may use Apple's Foundation Models framework on the phone to plan
calls to the instance's own `/v0/app/mcp`, its only connector.

- The app wraps `tools/list` entries as `Tool`s. Each turn the model sees
  a short catalogue (name, category, read-only, tier) and the schemas of
  at most 5 tools it picks.
- It runs with the device's account and no more. Read-only tools run;
  any other tool waits for the person's tap. Owner-tier tools are only
  staged: the app opens the approval card, and §3's proof is the person's,
  never the model's. `_meta` is set by the app, after the card.
- Tool results are data, never instruction; prompts and transcripts stay
  on the phone.
- It is on only where `SystemLanguageModel` is available; otherwise the
  app works as without it.

### 7. Migration

The app sends `X-Pigeon-App: <build>`; builds before it send nothing.

1. `routes::inventory()` counts every route, Access-bypassed ones
   included; a test pins 40. `architecture/network.rs` reads it instead
   of scanning sources; `tests/architecture.rs` keeps checking it against
   the source, so the coverage test can still fail.
2. The registry gains `asks`, `capability`, `doors`; `run()` becomes the
   §8 authorizer; `grants::OPERATIONS` reads the registry.
3. `mcp.rs`: session check, `GET` stream, resources, `_meta` proofs,
   `proof.challenge`.
4. §1's app operations; each `/v0/app/*` handler becomes a shim calling
   `run()` with the same answer shape, and counts calls by build.
5. `/v0/app/mcp`; the hostnames proxy forwards it under `/v0/app/`.
6. The app's `MCPClient` in PigeonCore; `Store` calls tools; TestFlight.
7. §6's coordinator, after 6.
8. The web shell; each page moves to MCP calls; old page paths serve the
   shell.
9. The CLI and follower call `/mcp`; the `/v0/run`, `/v0/operations` and
   `/v0/events` aliases go.
10. `MIN_APP_BUILD` is set to step 6's build once every linked device has
    called `/v0/app/mcp` and none a shim for 14 days; shims then answer
    426 with "update the app", and go a release later.
11. The inventory test pins 9; the architecture diagram's Clients row
    shows the two doors and §5's list: `/v0/app/mcp` under the app,
    `/connect/callback` under the owner, the shell as one `/` prefix.

A route added before step 6 is a shim that calls `run()` with an
operation and has no logic of its own; the Now tab's
`/v0/app/changes/<n>` is `change.status`, which gains `position`,
`model` and `approval`.

Steps 2 to 5 and 8 to 9 are briefs; 6, 7 and 10 need the owner's phone
and TestFlight.
