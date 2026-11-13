## The architecture site on docs.pigeon.dev, built through backplane

* Date: 2026-11-05
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: runtime
* Governs: `src/site-templates/architecture/**`, `src/pigeon-cli/crates/backplane/src/architecture/**`, `src/pigeon-cli/crates/backplane/tests/architecture.rs`, `src/pigeon-cli/crates/backplane/src/cobot/sites.rs`, `config.yaml`

## Context

pigeon's architecture is spread across its ADRs, and nothing draws it. One diagram should answer three questions:

- what the parts are: the engines, the control plane, the units and the providers;
- how they connect;
- where data flows: plans, state, credentials, signed requests and sites.

**The diagram is stateless.** No node, edge, coordinate or flow is stored or drawn by hand. Each build regenerates it from the facts the engines already read, so it changes as the architecture does: a part appears when its spec lands, and disappears when it is removed.

The pieces this needs already exist or are decided:

- **Backplane composes, judges through sentinel, acts through cobot, and starts nothing.** Its operation catalog and `Response` wire form serve every surface. See [`2026-11-03-development-backplane-platform-composition-layer.md`](2026-11-03-development-backplane-platform-composition-layer.md).
- **The control plane.** Phase 4 of the separation ADR, [`2026-11-05-runtime-control-plane-worker.md`](2026-11-05-runtime-control-plane-worker.md), runs backplane's operations from a Worker on `pigeon.dev`:
  - an `Instance` Durable Object serializes runs;
  - a Runner Container executes each one over a signed link;
  - plan, approve, apply gates every `apply` effect;
  - credentials are minted per run;
  - runs survive a stop.
- **Workers and Containers deploy through `pigeon cobot workers`.** See [`2026-11-05-development-cobot-workers-area.md`](2026-11-05-development-cobot-workers-area.md).
- **Static sites are built and published to R2 by `pigeon cobot sites build|publish`.**
  - Each site uses a released Zola theme, `src/static-site-template/`, per [`2026-11-03-runtime-cloudflare-static-sites-r2.md`](2026-11-03-runtime-cloudflare-static-sites-r2.md).
  - Its settings are checked against that theme's `schema.json`, per [`2026-11-03-runtime-static-site-config-contract.md`](2026-11-03-runtime-static-site-config-contract.md).

Two things are missing.

- **There is one template, and it is a blog.** An architecture site needs a different theme with the same contract. The blog's template should become one member of a family of site templates, and this site another.
- **The data and the job.**
  - No operation describes the platform as data.
  - No site is built or published by the control plane. `sites publish` runs only in `cobot.yml`.

**Design language.** The site borrows two references' general style, not their identity:

- **Goldsky Compose:** a near-black canvas, monospace labels, and data shown as pipelines of nodes joined by moving lines.
- **cloudflare.com:** one confident orange accent, bold sans headlines, product cards, and a network seen from its edge.

No logo, name, artwork, copy or font file is taken from either.

**Out of scope:**

- a live, request-time renderer;
- hand-written flows, notes or positions;
- pages beyond the diagram;
- the console UI;
- search;
- versioned docs per release;
- moving the blog onto the control plane before phase 6.

## Implementation plan.

### 1. A family of site templates

- **Layout.**
  - `src/static-site-template/` moves to `src/site-templates/blog/`.
  - A second template is added at `src/site-templates/architecture/`.
  - A template is any directory under `src/site-templates/` with a `template.yaml`.
- **What every template shares:**
  - it is a Zola theme, pinned by `template.yaml`'s `zola`, so there is no new toolchain;
  - its `schema.json` checks `static_site.site`;
  - its `scaffold/` turns those settings into Zola's files;
  - its `features` carry defaults;
  - it builds and publishes through `pigeon cobot sites build|publish`.
- **Release units.**
  - Each template is its own release unit in `.config/releases.yaml`, named `site-template-<name>` and tagged `site-template-<name>/vX.Y.Z`, with its own `CHANGELOG.md` tier, which the bot writes.
  - `site-template-blog` keeps `template.yaml`'s version, so its first release is the next patch after the last `static-site-template` tag.
  - Past `static-site-template/v*` tags stay.
- **A site names its template.**
  - `static_site.template` becomes `{ name, version, features }`, and `name` is one of the templates.
  - Until the blog re-pins, a template without `name` resolves to the old `static-site-template/v<version>` tag. That fallback is removed once nothing uses it.
- **Sites outside `workloads/`.**
  - A unit may hold sites in `<unit>/sites/<name>/`: a `config.yaml` with only `static_site`, and `site/`. This mirrors `workers/<name>/`.
  - `sites build` and `publish` find them there.
  - Their bucket and domain belong to the enclosing unit's `config.yaml`.

### 2. The architecture, derived: `architecture.describe`

- **The operation.** Backplane's catalog gains `architecture.describe`, with engine `backplane` and effect `read`.
  - Its `result` is `{ nodes, edges, groups, layout }`.
  - It is computed from the tree on every call.
  - Nothing is cached, stored or committed.
- **Nodes.** Each is `{ id, kind, lane, label, source }`.
  - `source` is the file and key the node came from.
  - `kind` is one of `engine`, `operation`, `policy`, `unit`, `provider`, `capability`, `worker`, `route`, `resource`, `container`, `store` or `site`, and `lane` follows from it (see the layout).
- **Edges.** Each is `{ from, to, carries, source }`. Every edge comes from one fact the engines already read:

  | Edge | `carries` | Derived from |
  | --- | --- | --- |
  | operation → engine | `runs` | `backplane::catalog()` |
  | policy → target | `governs` | each `.config/policies/*.yaml` `target`: a unit when the target is inside one, the pull request for `pull-request`, otherwise the repository |
  | route → worker | `request` | a `worker.yaml`'s `routes`; it passes through Access when the enclosing unit composes an Access application |
  | worker → resource | `binds` | a `worker.yaml`'s `bindings`, `durable_objects` and `crons` |
  | worker → container | `signed-request` | a `worker.yaml`'s `containers` |
  | container → engine | `runs` | the released image's unit in `.config/releases.yaml` |
  | unit → provider | `apply` | `cobot::validate`: each unit's providers |
  | unit → capability | `credential` | the `capabilities.yaml` entries a unit requests |
  | engine → store | `state` | cobot's state backend: the state repository, or R2 under the control plane |
  | store → site | `artifact` | each `static_site`, and each `<unit>/sites/<name>/`, with its custom domain |

- **Groups.** These are the boxes drawn around nodes, and they are derived too:
  - the Cloudflare account a worker spec deploys to;
  - each container class;
  - each local unit (`x-cobot.local`).
- **Only what is derived appears.** A part with no row in the table above does not appear. No setting adds, removes, renames or annotates a node or an edge.
- **What it never holds:** an ID, an account, a bucket name, a secret's value or any `$output` value. Only binding and output names appear.
- **The CLI.** `pigeon backplane run architecture.describe --json` prints it. This is how CI and a Runner Container get it.

#### The layout

`backplane::architecture::layout(&Graph) -> Layout` is pure. It does no I/O, and `describe` returns its result as `layout`.

- **Lanes.** Each node kind has a lane, and the lanes are in a fixed order, left to right:

  | Lane | Holds |
  | --- | --- |
  | `policy` | policies |
  | `source` | the repository and pull requests |
  | `edge` | routes and Access |
  | `control` | workers, Durable Objects and crons |
  | `data` | D1, R2, secrets and stores |
  | `operations` | backplane's operations |
  | `run` | containers and engines |
  | `units` | units |
  | `providers` | providers |
  | `sites` | sites |

  An empty lane takes no width.
- **Long edges.** An edge that crosses lanes gets a waypoint in each lane it crosses. A waypoint takes a narrow row and holds no label, so the edge passes between nodes, never through one.
- **Order within a lane.**
  - Nodes and waypoints are sorted by `id`.
  - Four barycentre sweeps, each left then right, reduce crossings. Barycentres are measured by relative position within each lane.
  - Ties break by `id`.
- **Group bands.**
  - A top-level group's members come first in each lane it spans, and form one band across those lanes.
  - In those lanes, waypoints of edges that only pass through come next, inside the band.
  - The lanes' other nodes come below the band, so no group box contains a node that is not its member.
- **Size.** On an 8px grid, a node's width comes from its label's length, and its height is fixed.
- **Groups.** A group's box is the bounding box of its members, plus one grid unit of padding.
- **Edges.**
  - Each is a chain of cubic segments, from the source's right port, through its waypoints, to the target's left port. An edge that runs leftward uses the opposite sides.
  - Edges within one lane route along that lane's gutter.
  - Ports on a side are spread evenly, in the order of the nodes at the other end.
- **The output** is integer coordinates and SVG path strings, sorted. `describe` is therefore byte-stable for a given tree.

### 3. The architecture template

- **Data in.**
  - `sites build` writes `describe`'s result to the assembled project's `data/architecture.json`, which is never committed.
  - The template reads it with Zola's `load_data`.
- **It draws; it never places.** The page is one inline SVG, drawn from `layout`:
  - a rect and label per node;
  - a path per edge;
  - a box per group;
  - a style per `carries`.

  No position, label or edge is written in the template, and no client JavaScript is needed.
- **One page.**
  - `/` is the diagram and its legend.
  - Under them, a `<table>` lists every node and edge, with its `source`, for screen readers and for linking to the repository.
- **Features**, with their defaults:
  - `fonts` (on);
  - `commit` (on): the build's commit in the footer;
  - `motion` (on): animated edges.
- **The design tokens** are CSS custom properties on `:root`, and every color comes from them:
  - **canvas:** near-black `#0b0d10` with a faint 24px grid, and a light theme (`#f7f7f5`) under `prefers-color-scheme` and `data-theme`;
  - **accent:** one orange of its own, `#ff7a1a`. It marks Cloudflare groups and `apply` edges, and nothing else;
  - **flow:** one cool cyan, `#38d6c4`, for `state` and `artifact`;
  - **lanes:** one fill per lane, checked for contrast in both themes;
  - **edges:** one style per `carries`:
    - `signed-request` is a double line;
    - `credential` is dotted;
    - `state`, `apply` and `artifact` are animated dashes.
- **Type, through the `fonts` feature (Bunny Fonts):**
  - a bold geometric sans for headings;
  - a monospace for labels.
- **Motion and width.**
  - The diagram scrolls inside its own frame, never the page, from 360px up.
  - Animation is off under `prefers-reduced-motion`, and off without `motion`.

### 4. The site: `docs.pigeon.dev`

- **Resources.** `infra/pigeon/config.yaml` gains, composed from the Cloudflare catalog:
  - an R2 bucket for the site;
  - its custom domain, `docs.pigeon.dev`, using `cloudflare/r2-custom-domain`;
  - the root and `www` transform rulesets that an apex renders for a static site, using `cloudflare/ruleset`.

  `infra/pigeon` stays a local unit, which its owner applies. The bucket's name is a setting in `config.yaml`, as `pigeon-state`'s is, and never appears in output.
- **The site** is `infra/pigeon/sites/docs/`:
  - its `config.yaml` pins `site-template-architecture`, and its `static_site.site` holds only the title and description;
  - it has no `site/`. Git keeps no empty directory, and `check_layout` rejects a placeholder file. A template whose `template.yaml` sets `content: none` therefore builds without `site/`, and `sites build` fails such a site if `site/` exists. A clean-checkout test covers both cases.
- **Access does not cover it.** `docs.pigeon.dev` is public. Access keeps covering `pigeon.dev`'s API and `/mcp`.

### 5. The job, through the control plane

- **Operations.** Backplane's catalog gains:

  | Name | Engine | Effect | Does |
  | --- | --- | --- | --- |
  | `docs.plan` | cobot | `plan` | `architecture.describe`, then `sites build` of `infra/pigeon/sites/docs/`. It stores the built site as a plan in R2: its file digests, and a file-level diff against what is live. It expires after 24 hours. |
  | `docs.publish` | cobot | `apply` | Syncs exactly that plan's files to the site's bucket, deletes stale keys, and checks the site live. It takes a plan ID. |

- **The tree the run reads.**
  - The image holds no checkout, and D1's `desired_state` holds only units, so `describe` and `sites build` read a source checkout instead.
  - `docs.plan` takes `{ revision }`: the commit of `main` it describes. A trigger resolves it with `git ls-remote` on the instance's `repository`, and the plan records it.
  - Before `describe`, cobot fetches that commit in the Runner, depth 1, into a fresh directory. It also fetches the site's pinned template tag. The repository is public, so this needs no credential.
  - backplane runs `describe` on that directory, which stays stateless. Nothing is kept after the run.
  - A Container test builds the site with no host mount.
- **The path through the Worker:**
  1. The control-plane Worker's `Instance` Durable Object queues the run, per phase 4 §3.
  2. cobot `worker` starts a Runner Container and signs every request to it, per §5.
  3. The Container runs `pigeon backplane serve --network --signed`, which calls cobot to build and sync.
  4. `docs.publish` takes §6's plan, approve, apply path. It is refused for a plan that is expired, unknown, unapproved or already applied.
  5. Its sync is resumable: a re-dispatched publish uploads only the keys whose digests differ, per §7.
- **Triggers:**
  - the daily drift cron (§11);
  - each completed `infra.apply` or `workers.deploy`;
  - each `POST /v0/instances/:id/plans` naming `docs.plan`.

  A plan with no changed files is recorded and dropped, and asks for no approval.
- **Credentials.**
  - Cloudflare's `capabilities.yaml` gains `r2.bucket.write`: `Workers R2 Storage Bucket Item Write`, with `scope: bucket`. The run requests it, and never the account-wide `r2.write`.
  - The capability schema gains the `bucket` scope beside `zone` and `account`. Minting names the bucket in the token's resources, so the token can write that bucket's objects and nothing else.
  - A test mints the token and shows a write to another bucket is refused.
  - It is minted at dispatch, reaches only that Container as `COBOT_CREDENTIALS`, and is revoked when the run ends, per §8.
- **Roles.**
  - Backplane decides and records.
  - cobot builds and syncs.
  - `.config/policies/backplane.yaml` still forbids process and file-writing code in backplane's source. `architecture.describe` only reads.
- **Until phase 4 is deployed**, CI builds the site with no secrets, as `sites.yml` does for the blog. The owner publishes it locally with `pigeon cobot sites publish --state DIR`.

### 6. Docs

`CLAUDE.md` gains a pointer bullet:

- site templates are `src/site-templates/<name>/`;
- the architecture site is `infra/pigeon/sites/docs/` on `docs.pigeon.dev`: a diagram regenerated from the code by `architecture.describe`, and published through the control plane.

Its static-site bullets name `src/site-templates/` once the move lands.

### 7. Execution

Each step is released before the next relies on it, and each passes `mise run ci`.

1. This ADR, the amendments it adds to the two static-site ADRs, and the `CLAUDE.md` pointer. A design mock is reviewed outside the repository.
2. `refactor(cobot)`: templates under `src/site-templates/<name>/`; `static_site.template.name`, with the unnamed fallback; the `site-template-blog` unit and tier; sites in `<unit>/sites/<name>/`; and `content: none`.
3. `chore(workloads)`: the blog pins `name: blog` and `site-template-blog`'s first release. Then `refactor(cobot)` drops the fallback.
4. `feat(backplane)`: `architecture.describe` and its layout (§2), with their tests.
5. `feat(site-templates)`: the architecture template (§3), released as `site-template-architecture`.
6. `feat(cobot)`: `sites build` writes `data/architecture.json` from `describe`.
7. `feat(infra)`: §4's bucket, domain and rulesets, and `infra/pigeon/sites/docs/`. The owner applies `infra/pigeon` locally, then publishes the site.
8. After phase 4's step 11:
   - `feat(catalog)`: the `bucket` scope and `r2.bucket.write`;
   - `feat(cobot)`: the revision checkout in the Runner (§5);
  - `feat(backplane)`: `docs.plan` and `docs.publish`, and their triggers (§5).

### 8. Verification

- `architecture.describe`, layout included, is byte-stable for a given tree.
- Adding a binding to a `worker.yaml` fixture adds exactly one node and one edge. Removing it restores the earlier output byte for byte.
- Over generated graphs, no two node rects overlap, every edge ends on its nodes' borders, and every group box contains its members.
- No file under `src/site-templates/architecture/` holds a node id, label or coordinate.
- Its output, with tokens and IDs in the environment and every unit's state at hand, contains no ID, account, bucket name or secret.
- The blog builds identically from `site-template-blog` and from its last `static-site-template` tag, ignoring the footer's commit.
- The site renders at 360px and 1440px, in both themes, with no horizontal page scroll. Every node and edge is in the table.
- With `prefers-reduced-motion`, nothing animates.
- `docs.publish` without an approved, unexpired plan is refused.
- Its minted token cannot write any other bucket, and stops working once the run ends.
- A Container stopped mid-publish is re-dispatched, and the live site ends up equal to the plan's digests.
- A plan with no changed files asks for no approval.
- `.config/policies/backplane.yaml` still holds.

## Amendments

### 2026-11-06: The site is a v1-manifest unit, published by the control plane's pipeline on merge

- **The site is `sites/docs` in pigeon's root `config.yaml`,** `kind: cloudflare/static-site`, with `zone: pigeon.dev`, `hosts: [docs]`, the `architecture` template at 0.1.0, and only a title and description. It has no `content`. `zones/pigeon.dev` (`kind: cloudflare/zone`) looks the zone up. This follows [`2026-11-05-runtime-instance-config-kind-units.md`](2026-11-05-runtime-instance-config-kind-units.md), and replaces §4's `infra/pigeon/sites/docs/` and its bucket, domain and rulesets in `infra/pigeon/config.yaml`. The kinds compose the same catalog modules, and their bucket is named `docs-pigeon-dev`.
- **The owner applies the two units once,** with `infra.plan` and `infra.apply` against pigeon's state. `pigeon.dev` must be a zone in the pigeon account.
- **§5's `docs.plan` and `docs.publish` are replaced** by the lifecycle ADR's pipeline:
  - a merge to `main` (`change.merge`) is a push event;
  - the control plane publishes every site whose inputs changed (`sites.publish`).
  - For this site, those inputs are everything `architecture.describe` reads.
  - `r2.bucket.write` and its `bucket` scope stay deferred: the publish uses a deploy token with R2 write on the account.

### 2026-11-06: The two units applied, with a token of their own

- **The owner applied `zones/pigeon.dev` and `sites/docs`** with `pigeon cobot plan --store` and `apply --plan` against pigeon's state. That created the zone's URL-rewrite ruleset, the site's bucket and its custom domain on `docs.pigeon.dev`.
- **They take their own account API token, not `infra/pigeon`'s bootstrap one.** That token sees no zones, so the zone lookup finds nothing and the ruleset call fails as an authentication error. The units' token holds:
  - Account Settings Read and Workers R2 Storage Edit, on the pigeon account;
  - Zone Read, DNS Edit, Transform Rules Edit and Dynamic URL Redirects Edit, on `pigeon.dev` only.

  These are the capabilities the `cloudflare/zone` and `cloudflare/static-site` kinds declare, plus the account lookup.
- **The first deploys failed** with `GET the listing: status 404`: the pipeline published before the bucket existed. A deploy with no successful one before it publishes every site, so the first merge after the apply publishes `sites/docs`.

### 2026-11-11: The page is a network diagram, derived; the template is read from the tree while releases pause

The owner asked for the site to show where traffic goes, in the Cloudflare
dashboard's look: clients, then Cloudflare, then GitHub, read top to bottom.
The lane diagram (§3) is replaced.

- **`describe` gains `network`,** beside `nodes`, `edges` and `layout`:
  - **clients:** MCP (`POST /mcp`, the catalog's tools and how many only
    read), git (`/git/`) and the owner's pages, from the Worker's routes;
  - **Access:** the `pigeon/platform` unit's `domains` and its service
    tokens' names;
  - **the Worker:** its routes, queue, crons, the number of its secrets,
    its Durable Object classes and Containers,
    from `worker.yaml`;
  - **links** between the objects, Containers, D1, R2 and Artifacts, and
    out to GitHub.
- **A link is a row in backplane's table, kept only when its evidence is in
  the tree:** a file and a pattern, such as `merges.rs` sending
  `Event::Behind` through `to_change`. A test fails when a row's evidence is
  gone, so the table follows the code. Nothing else adds a link.
- **The page draws `network` only:** each object's role, each link's label
  and kind (`call`, `job`, `store`, `git`), and where each box sits. It still
  never places a part `describe` didn't return.
- **The way back from GitHub is drawn, not duplicated:** one dashed line from
  the Actions card to `Checker`, labelled with what the code does: checks wait
  and retry (`waiting: GitHub`); the owner's `ci_rerun on: cloudflare` runs
  them on `Checker`. A small static script, `/assets/js/rail.js`, routes the
  line; without it, the Actions card says the same in words.
- **No live state.** `docs.pigeon.dev` is public. The page shows the commit
  it was built from and links to `platform.pigeon.dev`, whose dashboard
  (`2026-11-10-runtime-platform-hostname-dashboard-observability.md` §3)
  shows deploys, changes and approvals behind Access.
- **The template is read from the tree while releases pause.** A site whose
  template is in its own instance's repository may pin `version: tree`: the
  sites job reads `src/site-templates/<name>/` at the commit it publishes.
  `sites/docs` uses it. When the release job runs on Cloudflare, the site
  pins a released version again, and `tree` is refused for any other site.

Steps, each its own change:

1. `describe`'s `network` and the link table, with the evidence test.
2. `version: tree` in cobot's sites, for a template in the instance's tree.
3. The template: `index.html`, `main.css` and `rail.js`.
4. `config.yaml`: `sites/docs` pins `version: tree`.

### 2026-11-13: On narrow screens the network diagram stacks top to bottom

On a phone, the connection diagram (§3's "Motion and width", and the
2026-11-11 amendment's links) scrolled inside its frame, and its lines and
the GitHub rail ran off the right edge. The page now reads top to bottom at
every width from 360px, with no sideways scroll in any frame.

- **At 640px and below** the diagram's boxes flow in one column, in
  `layout`'s order: top to bottom, then left to right.
- **Each box lists its outgoing links in words:** the kind's swatch, the
  label, and `→` the part it reaches. The SVG lines, the bus into the
  Durable Objects and `rail.js`'s line are not drawn. The Actions card still
  says the way back from GitHub in words.
- **Wider screens are unchanged:** the drawn diagram, which still scrolls in
  its frame between 641px and its own width.
- The page still never places a part or writes a label `describe` didn't
  return; the column's order comes from `layout`'s coordinates.

Steps:

1. The template: `index.html` and `main.css`.
