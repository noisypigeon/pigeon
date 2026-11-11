## The platform at platform.pigeon.dev: one hostname, a dashboard at its root, and logs, traces, events and stats over MCP

* Date: 2026-11-10
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: runtime
* Governs: `infra/pigeon/workers/control/worker.yaml`, `.mcp.json`, `src/pigeon-cli/crates/backplane/src/worker/mod.rs`, `src/pigeon-cli/crates/backplane/src/worker/passkeys.rs`, `src/pigeon-cli/crates/backplane/src/passkeys.rs`

## Context

The control plane is served at `pigeon-control.pigeon-dev.workers.dev`, the
`workers.dev` hostname of the `pigeon-control` Worker.

- **Everything uses that hostname:**
  - MCP (`/mcp`);
  - the git front door (`/git/<namespace>/<repo>.git`);
  - the approval pages (`/approve/<grant>`, `/approvals`, `/platform/<id>`,
    `/mandate/<id>`);
  - passkeys (`/passkeys`);
  - the event log (`/v0/events`);
  - the health checks (`/healthz`).
- **Several places name it:**
  - `.mcp.json`;
  - `CLAUDE.md`'s git remote;
  - `platform/blue`'s `hostname` in `config.yaml`;
  - the agent environment's proxy, which injects the agent's Access service
    token for that host only.
- **Two decisions meant to move it:**
  - [`2026-11-08-runtime-blue-green-self-standup.md`](2026-11-08-runtime-blue-green-self-standup.md)'s
    step 4c-3 planned `control.pigeon.dev`, with Access covering the hostname
    before a route serves it.
  - [`2026-11-09-security-approvals-once-passkeys.md`](2026-11-09-security-approvals-once-passkeys.md)
    §4 binds a passkey to its page's hostname, and expects passkeys to be
    registered again when the hostname moves.
- **There is nothing at `/`.** What the platform is doing is split between
  `deploy_status`, `change_list`, `digest_show` and the event log. There is no
  page that shows it.
- **Logs are already collected.** The Worker render enables Workers Logs
  (`observability: { enabled: true }`), but only the dashboard reads them.
  - Traces are off.
  - Job Containers' output reaches only a job's own log tail.
  - Build and check durations exist only inside reports: `MainRun`'s deploy
    history and each change's jobs. Nothing keeps them as data that can be
    counted.

The owner asked for:
- one hostname, `platform.pigeon.dev`, for the whole platform instance and
  every URL the Worker serves;
- passkeys on it;
- a dashboard at its root;
- MCP tools for logs, traces and events in Cloudflare, so the dashboard can
  show builds and their stats.

## Implementation plan.

### 1. One hostname: `platform.pigeon.dev`

- **`platform.pigeon.dev` replaces `control.pigeon.dev`** in the blue-green
  ADR's step 4c-3. The mechanism is the same:
  - the `pigeon/platform` kind's `domain` setting, which the Access
    application protects as a self-hosted hostname;
  - applied first, so Access covers the hostname before a route serves it;
  - then a custom domain on the Worker, from `$deployment.domain`.
- **Every path stays as it is,** under the new host:
  - `https://platform.pigeon.dev/mcp`;
  - `https://platform.pigeon.dev/git/pigeon/pigeon.git`;
  - `https://platform.pigeon.dev/approve/<grant>`, and the other pages.

  Nothing moves to its own subdomain, so Access, the page policies and the
  `same_origin` checks have one origin to reason about.
- **Each stack also has its own hostname,** `<color>.platform.pigeon.dev`,
  for example `blue.platform.pigeon.dev`.
  - It is the stack's `hostname` setting and its `PUBLIC_HOST`.
  - The deploy job's health and smoke checks use it, so they always reach the
    stack they deploy.
  - `platform.pigeon.dev` is the live stack's `domain`. The blue-green
    cutover (§6 of the blue-green ADR) moves that one custom domain from one
    stack's Worker to the other's. Every client keeps its URL.
- **`workers.dev` goes away last.** The order is:
  1. Access covers `platform.pigeon.dev` and `blue.platform.pigeon.dev`.
  2. The Worker serves both, beside `workers.dev`.
  3. Clients move: `.mcp.json`, `CLAUDE.md`'s git remote, the agent
     environment's proxy credential and the owner's bookmarks.
  4. A deploy with `workers_dev: false` turns the old hostname off.

  Until step 4, both hostnames work.
- **The agent environment's proxy credential is the owner's to move.** It
  lives in the environment's settings, not the repository. The owner adds
  `platform.pigeon.dev` before step 3. The agent asks with the environment
  tools and never handles the token.

### 2. Passkeys on `platform.pigeon.dev`

- **The relying party ID is `platform.pigeon.dev`, not the request's host.**
  WebAuthn lets an origin use any registrable suffix of its own host.
  - So a passkey registered once works on `platform.pigeon.dev` and on every
    `<color>.platform.pigeon.dev`, across cutovers.
  - It doesn't extend to `docs.pigeon.dev`, or anything else under
    `pigeon.dev`.
- **The Worker checks against that ID.** The RP ID hash in the authenticator
  data must be SHA-256 of `platform.pigeon.dev`. The origin in the client data
  must be `https://platform.pigeon.dev` or `https://<color>.platform.pigeon.dev`
  for a stack in `config.yaml`. The relying party ID is a Worker var,
  `PASSKEY_RP_ID`, set from the kind's `domain`.
- **Passkeys made on `workers.dev` stop working at step 4.**
  - `/passkeys` says so while both hostnames are served.
  - The owner registers again on `platform.pigeon.dev`, with the code, and
    removes the old ones.

### 3. The dashboard at `/`

`GET /` is the platform's dashboard, behind Access, for a person.

- **It is server-rendered,** as the approval pages are.
  - Every value is escaped.
  - Its policy is `PROOF_CSP` (`script-src 'self'`).
  - One static script, `/dashboard.js`, refreshes the panels from the same
    JSON the MCP tools return. The page reads fully without it.
- **Panels:**
  - **Now:**
    - approvals waiting, with links;
    - open changes by state, and whose session holds them;
    - the newest deploy and its stage;
    - paused mandates.
  - **Builds:**
    - the newest builds of `pigeon`: commit, duration, size, and when;
    - the deploy history: outcome, Workers and versions, published sites,
      and how long each step took.
  - **Checks:** each check's duration and failure rate over the last day and
    week, and the slowest and flakiest checks.
  - **Errors:** the last hour's error and exception logs from the Worker and
    the job Containers, scrubbed (§4).
  - **Traffic:** requests, errors and CPU time per hour, from Workers
    analytics.
- **It holds no secret and changes nothing.** Every link to act on something
  goes to the existing approval pages.

### 4. Logs, traces, events and stats, over MCP

Five read-only tools. Each is an `Instance` operation, so the dashboard and
the MCP server call the same code.

| Tool | What it answers | Source |
|---|---|---|
| `logs_query` | Log lines by Worker or job Container, level, time range, text, change, or request ID | Workers Observability API |
| `traces_query` | Traces by route, status, duration or change; spans with their timings | Workers Observability API (traces) |
| `trace_show` | One trace's spans | Workers Observability API |
| `events_query` | The platform's event log by kind, change, session or time, beside `events_wait`'s long poll | the `Instance`'s event log |
| `stats_show` | Builds, deploys and checks: counts, durations (p50 and p95), failure rates and the slowest, by day or week; and requests and errors per hour | D1's `runs` table (below), and Workers analytics |

- **Turning on what's needed:**
  - The Worker render sets `observability.traces.enabled` beside logs, and a
    `head_sampling_rate` from `worker.yaml`. The rate defaults to 1 for the
    control plane, which is low-traffic.
  - Job Containers' stdout and stderr already reach Workers Logs. Each line is
    tagged with the job ID and change number, so logs can be filtered by
    change.
- **Pigeon's own runs become data.** A D1 table, `runs`, gets one row per job,
  deploy step and build:
  - kind, name, change, commit;
  - started, finished, outcome;
  - for a build, its size.

  `MainRun` and `ChangeRun` write a row when a job finishes. That's the source
  of the build and check stats. It needs no token, and it lasts beyond Workers
  Logs' retention. It is a migration, so it deploys at the code tier.
- **One new secret, `OBSERVABILITY_TOKEN`,** an account token with only:
  - Workers Observability Read, for logs and traces;
  - Account Analytics Read, for traffic.

  Adding it is `owner` tier, per the approvals ADR §2. Without it, the logs,
  traces and traffic panels say "not configured", and the rest works.
- **Scrubbing.** Every log line, span attribute and event leaves the tools
  with these removed, as the deploy job's `secrets_of` removes them:
  - secret values;
  - the deploy outputs' IDs and bucket names;
  - email addresses.

  The tools are open to the agent's service token. What they return is the
  same as what `deploy_status` and `change_status` already show, plus logs
  with the same scrubbing.
- **Limits.** Queries cover at most 24 hours per call, and at most 500 lines
  or 50 traces. Workers Logs keeps 7 days, so `stats_show` reads D1 for
  anything older.

### Execution

Each step is its own change.

1. **This ADR,** with a `CLAUDE.md` pointer. It amends the blue-green ADR's
   step 4c-3 hostname, and the passkeys ADR's hostname note.
2. **The hostname.**
   - The kind's `domain` setting, and Access for `platform.pigeon.dev` and
     `blue.platform.pigeon.dev`. That is a platform plan and apply.
   - Then `worker.yaml`'s custom domains, and `PASSKEY_RP_ID`, with
     `workers.dev` still on.
3. **Clients move:**
   - `.mcp.json` and `CLAUDE.md`;
   - the owner moves the proxy credential and registers a passkey on
     `platform.pigeon.dev`;
   - then `workers_dev: false`.
4. **`runs` in D1:** its migration, and the rows from `MainRun` and
   `ChangeRun`.
5. **The tools:** `events_query` and `stats_show`, from the `Instance` and D1,
   needing no token.
6. **Observability:**
   - traces on;
   - job log tags;
   - `OBSERVABILITY_TOKEN`, an `owner`-tier change;
   - `logs_query`, `traces_query` and `trace_show`.
7. **The dashboard:** `/` and `/dashboard.js`.

### Verification

- **Hostnames:**
  - `https://platform.pigeon.dev/healthz` answers through Access;
  - without Access, it answers 302 to the login;
  - `blue.platform.pigeon.dev` answers the same;
  - after step 3, `workers.dev` is gone and nothing breaks: MCP, git push,
    approval links, events.
- **Passkeys:** one registered on `platform.pigeon.dev` approves a link
  opened on `blue.platform.pigeon.dev`, and one from `workers.dev` is
  refused.
- **The dashboard:** `/` shows the newest build with its duration, the last
  deploys and the failure rate of each check, matching `deploy_status` and
  `change_status`.
- **The tools:**
  - `logs_query` for a failed check's change returns that job's lines, with
    no email or ID;
  - `trace_show` for an approval page's request shows its spans.
