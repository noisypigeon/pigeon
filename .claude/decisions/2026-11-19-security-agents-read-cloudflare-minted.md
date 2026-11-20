## Agents read the platform's Cloudflare state through `cloudflare.read`, on a read-only token minted per call

* Date: 2026-11-19
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: security
* Governs: `.config/grants.yaml`, `catalog/cloudflare/capabilities.yaml`, `src/pigeon-core/src/minting.rs`, `src/pigeon-core/src/ledger.rs`

## Context

The owner, 2026-11-19: "How can we make these tests and iterations go
faster?" Approved for this ADR: read-only platform visibility for agents.

### What planners waited on today

Planners waited on the owner's dashboard screenshots for four kinds of
facts:

- queue metrics;
- Worker invocations and logs;
- a Worker's variables;
- DNS records.

Each fact is read-only. Each wait cost a round trip to the owner's phone.

### What exists

- **`insights.show` and `ops.show`** read D1 rows only. An hourly cron fills
  `insights` from GraphQL Analytics with `OBSERVABILITY_TOKEN`. Neither reads
  logs, queues, a Worker's settings or DNS.
- **The observability ADR's §4 ops were never built**
  ([`2026-11-10-runtime-platform-hostname-dashboard-observability.md`](2026-11-10-runtime-platform-hostname-dashboard-observability.md)):
  `logs_query`, `traces_query`, `trace_show`, `events_query` and
  `stats_show`. There is no `logs.` or `stats.` op in `wire.rs`.
- **The broker ADR**
  ([`2026-11-16-security-provider-calls-mcp-broker.md`](2026-11-16-security-provider-calls-mcp-broker.md)):
  - rows C2 and C3 route Cloudflare REST and GraphQL reads through
    Cloudflare's MCP server, with `provider.call`;
  - its owner answer #12 mints each call a token holding only that tool's
    groups, expiring in 5 minutes and revoked after the call;
  - its step 6, which lists the servers and files the per-call grant, hasn't
    run, and `.config/providers/mcp.yaml` has `servers: {}`.
- **Minting** (`src/pigeon-agent/src/worker/minting.rs`):
  - `GRANTER` mints from the grants ledger that `main` deployed.
  - `for_object` mints at run time for a grant routed to an object, with no
    per-mint proof. The ledger change, merged on the owner's passkey, is its
    approval (the `aigateway` precedent).
  - `token_body` puts a zone-scoped permission on named zones, each resolved
    to its ID.
- **The private-zone rules**
  ([`2026-11-15-runtime-zoned-standalone-account-imports.md`](2026-11-15-runtime-zoned-standalone-account-imports.md)):
  - every zone but `pigeon.dev` is private;
  - its name never reaches git, logs or any output, only `<alias>`;
  - `zones.private` in the `Instance` maps alias to name;
  - zone and account IDs never appear in output (§5).
- **Levels**
  ([`2026-11-18-security-users-levels-capability-grants.md`](2026-11-18-security-users-levels-capability-grants.md)):
  - an `Effect::Read` op requires `read`, which every agent Service user
    holds;
  - a minted token is not a principal and never calls the control plane.

## Implementation plan.

### 1. One operation: `cloudflare.read`

- **What it is.** An `Effect::Read` operation in the control plane's catalog,
  run inside the Worker, not in a job Container. MCP can't dispatch a
  Container op today, and the token never needs to leave the Worker. Its
  tool is `cloudflare_read`, `readOnlyHint`.
- **Who can call it.** Any principal with `read`: planners, executors and the
  owner.
- **Arguments.** `what`, one of five, plus optional filters. Every filter is
  validated against a fixed grammar: names `[a-z0-9-]+`, a window of at most
  24 hours.

| `what` | Arguments | Reads | Returns |
|---|---|---|---|
| `queues` | `queue?`, `since?` | `GET /accounts/{a}/queues`; GraphQL `queueBacklogAdaptiveGroups`, `queueMessageOperationsAdaptiveGroups` | per queue: name, producers' and consumer's script names, backlog, messages and bytes, retries and DLQ counts, per hour |
| `invocations` | `script?`, `since?` | GraphQL `workersInvocationsAdaptive` | per script and status: requests, errors, subrequests, CPU and wall-time percentiles, per hour |
| `logs` | `script`, `since?`, `query?`, `limit?` (max 500) | `POST /accounts/{a}/workers/observability/telemetry/query` | timestamp, level, message, outcome, trigger, request path |
| `settings` | `script` | `GET /accounts/{a}/workers/scripts/{script}/settings`, and the script's routes | compatibility date and flags, routes, crons, bindings by name and type; a plain-text var's value; for a secret, its name only |
| `dns` | `zone` (`pigeon.dev` or an alias), `name?` | `GET /zones/{z}/dns_records` | type, name, content, proxied, TTL |

- **What it never reads.** A script's code, a secret's value, a token, an
  Access policy or a bucket's objects. No `what` has a write form, and a
  sixth `what` takes an amendment to this ADR.
- **The output is typed rows** with only the fields above. It never passes on
  an API object.
- **Names come from config.**
  - `script` is a Worker name the deployed specs declare. The op checks it
    against the architecture graph's Workers.
  - `zone` is `pigeon.dev` or a private zone's alias, resolved through
    `zones.private` inside the Worker.

### 2. The token: `cloudflare-read`, minted per call

- **The capabilities.** `catalog/cloudflare/capabilities.yaml` adds two:
  - `dns.read`: `[DNS Read]`, zone scope;
  - `queues.read`: `[Queues Read]`, account scope.
- **The ledger.** `.config/grants.yaml` adds:

  ```yaml
  cloudflare-read:
    mint: { capabilities: [observability.read, analytics.read, workers.read, zone.read, dns.read, queues.read], ttl: 5m }
    route: { object: Instance }
  ```

- **Each call:**
  1. `for_object(env, s, "cloudflare-read", "Instance", false)` mints a token
     holding only the groups its `what` needs:
     - `queues`: Queues Read, Account Analytics Read;
     - `invocations`: Account Analytics Read;
     - `logs`: Workers Observability Read;
     - `settings`: Workers Scripts Read;
     - `dns`: Zone Read, DNS Read.

     `for_object` takes a `subset`; a capability outside the grant is refused.
  2. The call runs with the token.
  3. The token is revoked when the call ends, whether it passed or failed.
     The 5-minute expiry backs up the revoke.
  4. The call records `minted{token_id, capabilities, expires}` and never the
     value. The token lives only in the call's memory.
- **This is broker answer #12's shape** (per call, only the groups needed, 5
  minutes, revoked after), applied in the Worker. A per-job token would live
  longer than the one read it serves.
- **Zone scope without zone IDs in git.**
  - `token_body` gains one resource form for a zone-scoped permission:
    every zone in the account, under the account
    (`com.cloudflare.api.account.<a>` →
    `{ "com.cloudflare.api.account.zone.*": "*" }`).
  - `cloudflare-read` uses it only for its two read groups.
  - `ledger::check` refuses that form for any grant that isn't read-only
    (§3).
  - This needs no zone name or ID in the ledger, so a private zone stays out
    of git.

### 3. Read-only, by construction

- **`ledger::check` holds an object-routed grant named `cloudflare-read` to
  a read-only set.**
  - Every permission of every capability it names must end in ` Read`.
  - None may be a token, secret, Access or Secrets Store group: no
    `API Tokens`, no `Secrets Store`, no `Access:`.
  - Its ttl is at most 5 minutes, with no rotate.
  - A test pins the set to the six capabilities above.
- **`workers.read` is Workers Scripts Read, and the op narrows it.** That
  group can also download a script. The op calls only `…/settings` and the
  route list, through a fixed path table. Its client refuses any other path.
- **Secret values can't be read.** Cloudflare returns a secret binding's name
  and type, never its value. The op also drops the value field of any binding
  whose type isn't `plain_text` or `json`.

### 4. Nothing private or identifying leaves

Every result passes through one scrub before it is returned, logged or put in
an event:

1. **Private zones.** Each private zone name in `zones.private` is replaced
   by `<alias>`. Matching is case-insensitive and longest first, as
   `platform_job::private_masks` does, and covers record names, CNAME
   targets, routes, log messages and plain-text vars. A query naming a
   private zone is answered under its alias.
2. **Accounts.** Any account name but `noisypigeon` becomes `<account>`.
3. **IDs.** Every run of 32 or more hex digits, and every UUID, becomes `…`,
   with nothing kept. That covers zone, account, record, queue, namespace
   and token IDs.
   - The typed rows already leave out every `id`, `zone_id`, `account_id`
     and `tag` field. The scrub is the backstop for IDs inside text such as
     log messages.
4. **Secrets.** The `workers::scrub` set of known secret values, as
   `Tools::scrubbed` applies it.

- **Tests**, with `example.com` and made-up IDs only:
  - a CNAME to a private zone comes back as `<alias>`;
  - a log line holding a zone ID comes back with `…`;
  - a `secret_text` binding has no value field;
  - no output holds a 32-hex run.

### 5. Tier

- **Owner, on the passkey, never covered by Auto, a window or a mandate:**
  - this ADR;
  - the capabilities;
  - the ledger entry;
  - `token_body`'s form and `ledger::check`'s rule;
  - `for_object`'s subset;
  - the op's mint and revoke.

  These are a credential grant
  ([`2026-11-17-security-auto-covers-routine-owner-asks.md`](2026-11-17-security-auto-covers-routine-owner-asks.md)).
  Each merges with `change_merge`, and the owner approves on its page.
- **The owner's approval of the ledger entry is the approval of every mint
  it allows,** as for `aigateway`. No call asks the owner. The entry, the
  read-only rule and the scrub are what bound it.
- **Routine:** the op's typed reads, the scrub and their tests.

### 6. Relation to earlier ADRs

- **The broker ADR, rows C2 and C3.** These five reads are pigeon's own
  operation, not `provider.call` to Cloudflare's MCP server:
  - typed rows make "no IDs in output" structural, where a hosted server
    returns raw API objects;
  - they need no registry entry or server pinning, which step 6 hasn't done.

  Any other Cloudflare read still goes through the broker as planned. An
  amendment there records this.
- **The observability ADR §4.** `logs` and `invocations` carry out
  `logs_query` and `stats_show` in this form. `traces_query`, `trace_show`
  and `events_query` stay planned there. An amendment there records this.

### Execution

1. **This ADR,** with the two amendments in §6. Owner tier.
2. **The planner's change, owner tier:**
   - the capabilities, the ledger entry and the `ledger::check` rule;
   - `token_body`'s all-zones-in-account form;
   - `for_object`'s subset.

   Tests in pigeon-core's minting and ledger suites.
3. **Brief b3 (sonnet), routine:**
   - the typed reads, the path table and the scrub, in a pure core module
     `src/pigeon-core/src/cloudflare_read.rs`;
   - the op's catalog entry in `wire.rs`;
   - tests with fixtures.

   No minting.
4. **The planner's change, owner tier:** the Worker side,
   `src/pigeon-agent/src/worker/cloudflare_read.rs`. It mints with the
   subset, calls through b3's client, revokes, records `minted`, then scrubs
   and returns.
5. **After step 4 deploys,** a planner calls each `what` once against the
   platform and attaches the outputs, scrubbed, to the change. Any group
   name `token_body` rejects is fixed in the catalog.
