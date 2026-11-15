## The architecture, complete, read by agents and published on every deploy

* Date: 2026-11-15
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: runtime
* Governs: `src/pigeon-cli/crates/backplane/src/architecture/**`, `src/pigeon-cli/crates/backplane/tests/architecture.rs`, `src/pigeon-cli/crates/backplane/src/catalog.rs`, `src/pigeon-cli/crates/backplane/src/cobot/scope.rs`, `src/pigeon-cli/crates/backplane/src/jobs.rs`, `src/pigeon-cli/crates/backplane/src/wire.rs`, `src/pigeon-cli/crates/backplane/src/worker/mod.rs`, `src/site-templates/architecture/templates/index.html`, `src/site-templates/architecture/static/assets/css/main.css`, `CLAUDE.md`

## Context

[`2026-11-05-runtime-architecture-site-through-backplane.md`](2026-11-05-runtime-architecture-site-through-backplane.md)
derives the Network Diagram from the code with `architecture.describe`. Its
2026-11-11 amendment draws `network`: clients from the Worker's route tables,
Access, the Worker's spec, and links kept only while their evidence is in the
tree. [`2026-11-11-runtime-docs-under-platform-passkey.md`](2026-11-11-runtime-docs-under-platform-passkey.md)
§2 serves the site at `/docs/`, and says agents read the same graph with
`architecture_describe`.

Three things have drifted.

- **Agents can't read it.** `architecture_describe` is an MCP tool, but the
  Worker refuses every operation outside the control plane: "runs in an
  instance's Container, which the control plane doesn't dispatch yet". It
  takes no arguments and has no text or SVG form. `CLAUDE.md` doesn't name it.
- **It isn't rebuilt on every merge.** `cobot::scope::deploys` republishes the
  architecture site when `config.yaml`, `infra/`, `catalog/`, `workloads/` or
  `.config/` change. `network` also reads backplane's sources (routes,
  objects, links), so a Worker-only merge deploys new code and leaves the
  diagram behind. The footer shows the commit the site was built from, which
  is then older than the one deployed.
- **Parts of the code are missing, and nothing notices.** Only routes in
  `SURFACES`' tables become clients. `fetch` serves some paths before the
  router: `/builds/`, the Buildkite release route, Apple's association file,
  the app's `/v0/app/*` API and `/healthz`. The spec's Secrets Store binding,
  its own routes, and every outbound service (Cloudflare's API, APNs, App
  Store Connect, Bitrise, Buildkite) have no node. No test fails when a route,
  binding or outbound host is added without one.

## Implementation plan.

### 1. Coverage: every route, binding and outbound service, derived

All of it comes from the tree, as the 2026-11-11 amendment's links do.
Nothing is placed by hand.

- **Routes.** `network` reads every route the Worker serves:
  - each surface's `ROUTES` table, as now;
  - the paths `fetch` serves before the router: each string literal and each
    `<module>::<CONST>` naming a `&str` path, in `fetch`'s body in
    `worker/mod.rs`;
  - the app's API: each `"/v0/app/…"` literal and constant in the Worker's
    sources, under the `devices::API` prefix.
- **Clients**, by the route's surface:
  - `mcp`, `git` and `owner`, as now;
  - `app`, "iOS app": `/v0/app/*` and the association file, each signed by its
    device;
  - `ci`, "CI": the Buildkite release route and `/builds/`, the paths a
    pipeline or job Container calls;
  - `health`: `/healthz` and `/healthz/smoke`, under `owner`.
- **The Worker's hostnames.** Each of the spec's `routes`, bound, becomes an
  entry in `access.hosts`, its kind beside it:
  - a `custom_domain` on `pigeon.dev` is shown as is;
  - a route that serves every subdomain of a zone (the standalone hostnames
    ADR's `subdomains_of`) is shown as `*.pigeon.dev`, "standalone instance
    (own account)";
  - any other hostname is never shown; it reads "standalone instance (own
    account)".
- **Bindings.** `Worker` gains `secrets_store`: its Secrets Store bindings'
  names, such as `GRANTER`. A part `Secrets Store`, kind `store`, appears when
  there is one.
- **Outbound services.** A table, `SERVICES`, beside `LINKS`: each row is a
  service's id, its name, the host the code calls, and what for. A row is kept
  only while its host is in a non-test string literal of the Worker's
  sources. Each becomes a part, kind `service`, and a link from `Worker`. The
  host is evidence only; the diagram shows the name. Rows:
  - Cloudflare API: token minting through `GRANTER`, and the insights
    GraphQL query under `OBSERVABILITY_TOKEN`;
  - APNs: pushes to the owner's iPhone;
  - App Store Connect: TestFlight uploads for `ios.release`;
  - Bitrise and Buildkite: macOS builds, the failover providers.
  - GitHub's API is the existing `Actions` and `Mirror` parts; its host maps
    to them.
- **Links without a host.** `LINKS` gains evidenced rows for:
  - `Worker → Secrets Store`: "granter: mints scoped tokens", from
    `minting.rs`'s `secret_store(GRANTER)`;
  - `Worker → Web Push`: "approvals, digest", from `push.rs`. Web Push's
    endpoints come from each subscription, so the part names no host.
- **macOS providers.** `network` gains `providers`: the providers
  `core/macos` declares, by name, each with whether it's GitHub's
  (`macos-26`) or a failover.
- **Not drawn:** the Auto window, holds and the breaker. They are states the
  control plane keeps, not parts or traffic; `deploy_status`,
  `session_status` and the dashboard show them.

### 2. The coverage check

`tests/architecture.rs` gains a test against the platform's own tree that
fails, naming each gap, when:

- a route the Worker serves (§1's three sources) is in no client's items,
  directly or under a listed prefix;
- a field of the bound spec's `bindings`, `routes`, `durable_objects`,
  `containers`, `crons` or `secrets` holds something `network` doesn't
  represent, or `Bindings` gains a field the test doesn't know;
- a host in a non-test string literal of the Worker's sources maps to no
  `SERVICES` row and isn't `pigeon.dev`'s, Cloudflare's own internal ones, or
  a test's `.example`;
- a `LINKS` or `SERVICES` row's ends aren't parts.

The evidence test stays: a row whose evidence is gone fails.

**Privacy**, in the same file: `describe`'s JSON holds no domain but
`pigeon.dev`'s and file paths, no 32-hex ID, no bucket name from any unit's
settings, and no secret's name beyond the count. A standalone's hostname
never appears.

### 3. Rebuilt on every deploy, showing the deployed SHA

- **`cobot::scope::deploys` always includes a site on the `architecture`
  template** when it has any input set. Its sync puts only the files that
  changed: the page, whose SHA changed, and `architecture.json`.
- **`sites.publish` takes `deployed`,** the SHA the deploy job runs for. The
  job passes its spec's `sha`. It is added to the architecture data as
  `deployed`.
- **The page shows "deployed `<sha>`"** when `deployed` is set, and "built
  from" otherwise, as for a local build.

### 4. Agents read it: `architecture_describe`

- **The site carries its data.** `sites build` writes the architecture data
  to the site's `static/architecture.json` as well as `data/`, for the
  `architecture` template only. It lands in the `DOCS` bucket beside the page.
- **The Worker serves the operation itself.** `architecture.describe` is a
  control-plane read: the Worker reads `architecture.json` from `DOCS` and
  answers it. A service token may call it; `/docs/`'s passkey gate is for
  people's pages, and this is the same data, as that ADR §2 intends.
- **It takes `format`:**
  - `json` (the default): `{ nodes, edges, groups, layout, network,
    deployed }`, each node, edge, part and link with its `source`;
  - `text`: the network as an indented outline, one line per client route,
    host, Worker binding, part, link (with its evidence) and service;
  - `svg`: a standalone SVG drawn from `network.diagram`, with clients, Access,
    the Worker and the services as labelled boxes.
- **Both renderings are pure functions** of the JSON, in
  `architecture::render`, built without `engines` so the Worker uses them.
  `pigeon backplane run architecture.describe` takes the same `format`.
- **When `DOCS` holds no `architecture.json`** yet, the tool answers a
  violation naming the next deploy.

### 5. Docs

`CLAUDE.md`'s command list gains `architecture_describe` (`format`
`json|text|svg`): the platform's graph as of the deployed SHA, and that the
coverage test fails on a part of the code the diagram lacks.

### 6. Execution

1. This ADR.
2. `feat(backplane)`: §1 to §5, with the coverage and privacy tests.

The standalone hostnames planner's `subdomains_of` route and the noisypigeon
instance's parts land through §1's derivations; the coverage test tells
either change what it lacks.
