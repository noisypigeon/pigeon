## pigeon cobot workers: Workers and Containers through wrangler

* Date: 2026-11-05
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development

## Context

pigeon runs two kinds of change on Cloudflare, and needs one tool for each.

- **Infrastructure**: D1, R2, the Secrets Store, DNS and Access, and every other cloud an instance uses. OpenTofu plans and applies it through cobot, per [`2026-11-02-runtime-cobot-infrastructure-as-code.md`](2026-11-02-runtime-cobot-infrastructure-as-code.md).
- **Workers and Containers**: Worker scripts, Container applications and their images, Durable Object migrations, Cron Triggers and Worker secrets. wrangler deploys them.

The separation ADR's phase 3 spike ([`2026-11-04-development-platform-instance-separation.md`](2026-11-04-development-platform-instance-separation.md), "Phase 3 spike report") deployed a Worker and a Container by hand with wrangler. It showed what a CLI should take over:

- **A hand-edited `wrangler.toml`.** It needed `SET_LOCALLY` IDs that must never reach the repository, and a `-c` flag on every command.
- **Token permissions found one error at a time.** The working set was Workers Admin (Editor cannot upload a script); Read and Write on Workers Containers, Secrets Store, D1, Workers R2 Storage, and SSL and Certificates; Account Settings Read; and Workers KV Storage Read.
- **A billing trap.** `wrangler delete` leaves the Container application running unless the operator says otherwise.
- **The image push and Durable Object migrations.** OpenTofu's Worker script resource does neither, so amendment 1 §2 of the separation ADR cannot deploy the control plane as written.

The platform's API, MCP server and UI will offer the same outcomes as the CLI. One set of outcome-level operations should serve them all.

## Implementation plan.

### 1. Roles

- **OpenTofu** owns declarative resources, on Cloudflare and every other cloud. That covers the platform's bootstrap and every instance's units.
- **wrangler** owns what it deploys: Worker scripts, Container applications and their images, Durable Object migrations, Cron Triggers and Worker secrets.
- **Both are cobot's actions**, per [`2026-11-02-development-sentinel-policy-cobot-actions.md`](2026-11-02-development-sentinel-policy-cobot-actions.md). Nothing else starts wrangler. sentinel and backplane start nothing.
- **This supersedes amendment 1 §2** of the separation ADR, which deployed the control plane with OpenTofu's Worker script resource. That text stays as written.

### 2. The Worker spec

A Worker is declared in `workers/<name>/worker.yaml`, inside the unit whose resources it binds, for example `infra/pigeon/workers/control/worker.yaml`. A JSON schema in cobot checks it, and `pigeon cobot validate` covers it.

```yaml
description: The platform's control plane.
name: pigeon-control
entry: { release: backplane, artifact: worker }   # or { crate: <path> }
compatibility_date: "2026-10-01"
routes: [ { custom_domain: pigeon.dev } ]
crons: ["17 3 * * *"]
durable_objects:
  classes: [Instance]
  migrations: [ { tag: v1, new_sqlite_classes: [Instance] } ]
containers:
  - class: Runner
    image: { release: backplane }               # a released image, by digest
    instance_type: standard-1
    max_instances: 4
bindings:
  d1:      { DB: $output.infra/pigeon.database_id }
  r2:      { STATE: $output.infra/pigeon.state_bucket }
  secrets: { BACKPLANE_TOKEN: $output.infra/pigeon.backplane_token_secret }
secrets: [GRANT_KEY]                            # names only; values at deploy
```

- **References, not IDs.** Each `$output.<unit>.<name>` resolves from that unit's OpenTofu state outputs at render time.
- **Never in a spec:** an ID, an account, a secret value or a bucket name.

### 3. Commands

Every command is outcome-level and idempotent. Each prints a human summary, or `--json` in backplane's wire form.

| Command | Outcome |
| --- | --- |
| `pigeon cobot workers validate [NAME…]` | Every spec is valid, and every reference resolves. |
| `pigeon cobot workers render [NAME…] --state DIR` | `wrangler.jsonc` is written to `.cobot/workers/<name>/`, which is gitignored and never committed. |
| `pigeon cobot workers plan [NAME…] --state DIR` | A dry run, diffed against what is deployed, and stored for 24 hours, like a stored infra plan. |
| `pigeon cobot workers deploy --plan ID --state DIR` | In order: D1 migrations; the image pushed by digest; the Worker deployed; its secrets set. Only a stored, unexpired plan deploys. |
| `pigeon cobot workers status [NAME…]` | The deployed version and URL, and the Container application with its instances. |
| `pigeon cobot workers secrets put NAME KEY` | A Worker secret is set from stdin. It is never printed. |
| `pigeon cobot workers teardown NAME` | The Worker **and** its Container application are deleted. Nothing OpenTofu owns is touched. |
| `pigeon cobot workers permissions [NAME…]` | The exact token permissions the specs need, derived from their bindings. |

### 4. Credentials and output

- **The token** is `CLOUDFLARE_API_TOKEN`, or the run's minted credential from `COBOT_CREDENTIALS`, through the resolver phase 3 added.
- **The account** is read from the token, as the `cloudflare/account` module does. A token that sees more than one account is refused.
- **Output is scrubbed** of the token, account IDs and resource IDs before it is printed. That makes it safe for public CI logs.

### 5. Toolchain

- **`mise.toml`** pins `node` and `npm:wrangler`.
- **The cobot image** gains the same versions. The cobot test that checks `tofu` and `zola` against `mise.toml` checks them too.
- **No build at deploy.** Images are built and released by the release pipeline, per [`2026-11-02-release-crate-units-and-images.md`](2026-11-02-release-crate-units-and-images.md). `deploy` only pushes a released image by digest, so Docker is needed only where `deploy` runs.

### 6. Platform operations

- **Backplane's catalog** gains these operations, so the API, MCP server and UI call the same outcomes:
  - `workers.plan`, effect `plan`;
  - `workers.deploy`, effect `apply`, taking a plan ID;
  - `workers.status`, effect `read`;
  - `workers.teardown`, effect `apply`.
- **Cloudflare's `capabilities.yaml`** gains `workers.deploy`, holding §3's permissions at account scope, so phase 4 can mint a deploy token per run.

### 7. Execution

Each step is released before the next relies on it, per [`2026-11-03-release-cobot-image-in-workflows.md`](2026-11-03-release-cobot-image-in-workflows.md).

1. This ADR, and its `CLAUDE.md` pointer.
2. `chore`: pin `node` and `wrangler` in `mise.toml` and the cobot image, with the version test.
3. `feat(cobot)`: the spec, its schema, `validate` and `render`.
4. `feat(cobot)`: `plan`, `deploy`, `status`, `secrets`, `teardown` and `permissions`.
5. `feat(catalog)`: Cloudflare's `workers.deploy` capability.
6. `feat(backplane)`: the `workers.*` operations.

### 8. Verification

- `render` is byte-stable for a given spec and state, and its output holds no ID.
- `teardown` removes the Container application along with the Worker.
- `permissions` for the control-plane spec equals the spike's working set.
- `deploy` without a stored, unexpired plan is refused.
- The version test covers `node` and `wrangler`.
- Every command's output, with a token and IDs in the environment, contains neither.

## Amendments

### 2026-11-05: Images are built at deploy, the spec gains queues and Secrets Store bindings, and node and wrangler stay off the cobot image for now

Built as phase E1 of [`2026-11-05-development-changes-through-pigeon-mcp.md`](2026-11-05-development-changes-through-pigeon-mcp.md).

- **Images are built at deploy, from a Dockerfile stage.** A container's `image` is `{ dockerfile, target }`. `deploy` runs `docker build --target` into a local tag named after the plan, and `render` writes a one-line `<Class>.Dockerfile` (`FROM` that tag) that wrangler builds and pushes. §5's "no build at deploy" assumed releases on GitHub, which are leaving it, and wrangler can't select a stage itself. A released image (`{ release }`) returns with the release job (hosting ADR phase 4).
- **`entry` is `{ crate, features }` only**, built by `worker-build` with default features off. `{ release, artifact }` waits for the same release job.
- **Binding shapes, as built:**
  - `d1: { NAME: { name, id: $output…, migrations? } }`: wrangler needs the database's name (already public in `infra/pigeon/config.yaml`) beside its ID;
  - `r2: { NAME: $output… }`, with `[n]` to index a list output such as `bucket_names`;
  - `secrets_store: { NAME: { store: $output…, secret } }`, replacing `secrets:` under bindings;
  - `queues: { producers: { NAME: queue }, consumers: [{ queue, max_batch_size? }] }`;
  - `vars`, which refuses a value that looks like an ID.
  Each Durable Object binding is its class's name in `SCREAMING_SNAKE_CASE`, and every container class must also be a Durable Object class.
- **The spec is checked by serde** (`deny_unknown_fields`) and `workers::validate`, not a JSON Schema file: every rule needs code anyway (existing paths, references, class membership).
- **Plans** are stored at `<state>/workers-plans/<id>.json`, each Worker's rendered configuration's digest, and expire with infra plans (`plans::LIFETIME_SECS`). `deploy` re-renders and refuses a Worker whose digest changed since the plan.
- **`deploy` checks secrets but doesn't set them:** it names each declared secret the deployed Worker lacks, and `secrets put` sets it from stdin.
- **node and wrangler** are pinned in `mise.toml` with `worker-build`, but not added to the cobot image yet: until phase 4, deploys run on the owner's machine. They join the image, and §5's version test, with the `workers.*` operations.
- **`teardown`** runs `wrangler delete --force`, then deletes each Container application named after the Worker. wrangler's listing holds IDs, so it's read but never printed.
