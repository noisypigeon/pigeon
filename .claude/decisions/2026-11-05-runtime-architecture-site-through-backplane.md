## The architecture site on docs.pigeon.dev, built through backplane

* Date: 2026-11-05
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: runtime

## Context

pigeon's architecture is spread across its ADRs, and nothing draws it. The site should answer three questions:

- what the parts are: the engines, the control plane, the units and the providers;
- how they connect;
- where data flows: plans, state, credentials, signed requests and sites.

It must stay true as the code changes. The picture should come mostly from the code, not be drawn by hand.

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
  - `site-template-blog` keeps `template.yaml`'s `version: 1.0.1`, so its first release is `1.0.2`.
  - Past `static-site-template/v*` tags stay.
- **A site names its template.**
  - `static_site.template` becomes `{ name, version, features }`, and `name` is one of the templates.
  - Until the blog re-pins, a template without `name` resolves to the old `static-site-template/v<version>` tag. That fallback is removed once nothing uses it.
- **Sites outside `workloads/`.**
  - A unit may hold sites in `<unit>/sites/<name>/`: a `config.yaml` with only `static_site`, and `site/`. This mirrors `workers/<name>/`.
  - `sites build` and `publish` find them there.
  - Their bucket and domain belong to the enclosing unit's `config.yaml`.

### 2. The architecture as data: `architecture.describe`

- **The operation.** Backplane's catalog gains `architecture.describe`, with engine `backplane` and effect `read`. Its `result` is a graph, built only from loaded files and the engines' own functions.
- **Nodes.** Each is `{ id, kind, role, label, source }`, where `source` is the repository path it came from:

  | Kind | From |
  | --- | --- |
  | `engine` | `backplane::engines()`, with each engine's role |
  | `operation` | `backplane::catalog()`, with each operation's effect |
  | `unit` | `cobot::validate`: each unit's path, provider and modules |
  | `worker` | each `workers/*/worker.yaml`: classes, containers, crons and binding names |
  | `resource` | each binding's kind (`d1`, `r2`, `secrets`, `durable_object`, `container`), named by the binding, never by its value |
  | `capability` | each `catalog/*/capabilities.yaml` entry |
  | `policy` | each `.config/policies/*.yaml`, with its target |

- **Edges.** Each is `{ from, to, kind }`, where `kind` is one of:
  - `runs` (operation → engine);
  - `binds` (worker → resource);
  - `provisions` (unit → provider);
  - `requests` (unit → capability);
  - `governs` (policy → target).
- **Hand-written flows.** Flows the code cannot derive are written by hand. Examples: a pull request's plan and apply; plan, approve, apply; minting; drift; and this site's own build.
  - Each flow is `{ id, title, steps: [{ from, to, carries, note }] }`.
  - `carries` is one of `request`, `signed-request`, `plan`, `state`, `credential`, `artifact`, `audit` or `read`.
  - Flows live in the site's `static_site.site.flows`, and the architecture template's `schema.json` checks their shape.
  - Every `from` and `to` must be a node `describe` returns. `sites build` fails on an unknown one and names it.
- **What it never holds:** an ID, an account, a bucket name, a secret name's value or any `$output` value. Only binding and output names appear.
- **Stability.** The output is sorted, and is byte-stable for a given tree.
- **The CLI.** `pigeon backplane run architecture.describe --json` prints it. This is how CI and a Runner Container get it.

### 3. The architecture template

- **Data in.** `sites build` writes `describe`'s result to the assembled project's `data/architecture.json`, which is never committed. The template reads it with Zola's `load_data`.
- **Diagrams.**
  - They are inline SVG, laid out at build time by Tera macros: fixed lanes by kind, and nodes placed in their lane's order.
  - No client JavaScript is needed to read the site.
  - Every diagram has a text equivalent: a table of nodes and a numbered list of steps.
- **Pages:**

  | Page | Shows |
  | --- | --- |
  | `/` | One system diagram: the repository, CI, the control plane, Runner Containers, providers and sites. Below it, a card per engine. |
  | `/engines/<name>/` | The engine's role, its operations and effects, and the policies that govern what it produces. |
  | `/control-plane/` | The `pigeon.dev` Worker: Access, the API and MCP, the `Instance` Durable Object, D1's tables, R2, the Secrets Store and the Runner. |
  | `/units/` | Each unit, its provider, its modules and the capabilities it requests. |
  | `/flows/<id>/` | One flow as a pipeline diagram, with its numbered steps beside it. |
  | `/decisions/` | The ADR index, linking each ADR on GitHub, grouped by category. |

- **Features**, with their defaults:
  - `fonts` (on);
  - `commit` (on): the build's commit in the footer;
  - `motion` (on): animated edges;
  - `decisions` (on): the ADR index.
- **The design tokens** are CSS custom properties on `:root`, and every color comes from them:
  - **canvas:** near-black `#0b0d10` with a faint 24px grid, and a light theme (`#f7f7f5`) under `prefers-color-scheme` and `data-theme`;
  - **accent:** one orange of its own, `#ff7a1a`. It marks the Cloudflare boundary and anything with effect `apply`, and nothing else;
  - **flow:** one cool cyan, `#38d6c4`, for data in motion;
  - **roles:** one fill each for policy, action, composition, resource and provider. They are checked for contrast in both themes;
  - **edges:** a style per `carries` (for example, `signed-request` is a double line, `credential` is dotted, and `state` is solid).
- **Type, through the `fonts` feature (Bunny Fonts):**
  - a bold geometric sans for headings;
  - a neutral sans for body text;
  - a monospace for operation names, paths and labels.
- **Layout and motion.**
  - Cards for engines and products.
  - Wide diagrams scroll inside their own frame, never the page, from 360px up.
  - Animated dashes run along edges in the direction of flow. They are off under `prefers-reduced-motion`, and off with `motion`.

### 4. The site: `docs.pigeon.dev`

- **Resources.** `infra/pigeon/config.yaml` gains, composed from the Cloudflare catalog:
  - an R2 bucket for the site;
  - its custom domain, `docs.pigeon.dev`, using `cloudflare/r2-custom-domain`;
  - the root and `www` transform rulesets that an apex renders for a static site, using `cloudflare/ruleset`.

  `infra/pigeon` stays a local unit, which its owner applies. The bucket's name is a setting in `config.yaml`, as `pigeon-state`'s is, and never appears in output.
- **The site** is `infra/pigeon/sites/docs/`:
  - its `config.yaml` pins `site-template-architecture` and holds `static_site.site`: the title, the nav, and the flows (§2);
  - its `site/pages/` holds prose for each page;
  - it has no posts.
- **Access does not cover it.** `docs.pigeon.dev` is public. Access keeps covering `pigeon.dev`'s API and `/mcp`.

### 5. The job, through the control plane

- **Operations.** Backplane's catalog gains:

  | Name | Engine | Effect | Does |
  | --- | --- | --- | --- |
  | `docs.plan` | cobot | `plan` | `architecture.describe`, then `sites build` of `infra/pigeon/sites/docs/`. It stores the built site as a plan in R2: its file digests, and a file-level diff against what is live. It expires after 24 hours. |
  | `docs.publish` | cobot | `apply` | Syncs exactly that plan's files to the site's bucket, deletes stale keys, and checks the site live. It takes a plan ID. |

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
  - The run requests Cloudflare's existing `r2.write`.
  - The capability schema gains a `bucket` scope beside `zone` and `account`, so the minted token can write only the site's bucket.
  - It is minted at dispatch, reaches only that Container as `COBOT_CREDENTIALS`, and is revoked when the run ends, per §8.
- **Roles.**
  - Backplane decides and records.
  - cobot builds and syncs.
  - `.config/policies/backplane.yaml` still forbids process and file-writing code in backplane's source. `architecture.describe` only reads.
- **Until phase 4 is deployed**, CI builds the site with no secrets, as `sites.yml` does for the blog. The owner publishes it locally with `pigeon cobot sites publish --state DIR`.

### 6. Docs

`CLAUDE.md` gains a pointer bullet:

- site templates are `src/site-templates/<name>/`;
- the architecture site is `infra/pigeon/sites/docs/` on `docs.pigeon.dev`, built from `architecture.describe` and published through the control plane.

Its static-site bullets name `src/site-templates/` once the move lands.

### 7. Execution

Each step is released before the next relies on it, and each passes `mise run ci`.

1. This ADR, the amendments it adds to the two static-site ADRs, and the `CLAUDE.md` pointer. A design mock is reviewed outside the repository.
2. `refactor(cobot)`: templates under `src/site-templates/<name>/`; `static_site.template.name`, with the unnamed fallback; the `site-template-blog` unit and tier; and sites in `<unit>/sites/<name>/`.
3. `chore(workloads)`: the blog pins `name: blog` and `site-template-blog`'s first release. Then `refactor(cobot)` drops the fallback.
4. `feat(backplane)`: `architecture.describe` (§2), with its tests.
5. `feat(site-templates)`: the architecture template (§3), released as `site-template-architecture`.
6. `feat(cobot)`: `sites build` writes `data/architecture.json` and checks the flows' node names.
7. `feat(infra)`: §4's bucket, domain and rulesets, and `infra/pigeon/sites/docs/`. The owner applies `infra/pigeon` locally, then publishes the site.
8. After phase 4's step 11:
   - `feat(catalog)`: the `bucket` scope;
   - `feat(backplane)`: `docs.plan` and `docs.publish`, and their triggers (§5).

### 8. Verification

- `architecture.describe` is byte-stable for a given tree.
- Its output, with tokens and IDs in the environment and every unit's state at hand, contains no ID, account, bucket name or secret.
- A flow naming an unknown node fails `sites build`, and the error names the node.
- The blog builds identically from `site-template-blog` and from its last `static-site-template` tag, ignoring the footer's commit.
- The site renders at 360px and 1440px, in both themes, with no horizontal page scroll. Every diagram has its text equivalent.
- With `prefers-reduced-motion`, nothing animates.
- `docs.publish` without an approved, unexpired plan is refused.
- Its minted token cannot write any other bucket, and stops working once the run ends.
- A Container stopped mid-publish is re-dispatched, and the live site ends up equal to the plan's digests.
- A plan with no changed files asks for no approval.
- `.config/policies/backplane.yaml` still holds.
