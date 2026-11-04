## The pigeon platform and its instances

* Date: 2026-11-04
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development

## Context

[`2026-11-03-development-backplane-platform-composition-layer.md`](2026-11-03-development-backplane-platform-composition-layer.md) named the split between the product and one instance of it, but left both in one repository. They are still tangled in four ways.

- **One repository holds both.** The CLI, images and provider modules (product) sit beside `.config/`, `infra/` and `workloads/`, which are the noisypigeon instance.
- **Credentials live in the repository.** They are `ENC[cobot:v1:…]` values in `secrets.config.yaml`, under `COBOT_SECRETS_PASSPHRASE` and `COBOT_WORKLOAD_SECRETS_PASSPHRASE`:
  - `infra/cloudflare` holds its `provisioner_token` and `account_id`;
  - the noisypigeon.com apex holds its Proton verification values;
  - the DigitalOcean, Scaleway and Backblaze ADRs each plan two to four more keys.
- **GitHub Actions is the control plane.** `cobot.yml` plans on a pull request and applies on merge, and encrypted state lives in a private state repository, per [`2026-11-02-runtime-cobot-infrastructure-as-code.md`](2026-11-02-runtime-cobot-infrastructure-as-code.md).
- **One Cloudflare account serves everything.** `infra/cloudflare` serves every zone, `pigeon.dev` included. It mints workload tokens that reach workloads through state, per [`2026-11-02-runtime-cloudflare-provisioner-unit.md`](2026-11-02-runtime-cloudflare-provisioner-unit.md) and [`2026-11-02-runtime-workloads-zone-apex.md`](2026-11-02-runtime-workloads-zone-apex.md).

`pigeon.dev` now lives in its own Cloudflare account, **pigeon**. The platform's resources live there.

This ADR decides the separation and the full migration to it:

1. **This repository is the platform.** Each instance gets a repository of its own; noisypigeon's is the first.
2. **The platform holds each instance's desired state.** It changes through the platform's API, an MCP server or a UI. A git merge no longer gates an infrastructure change; an approved plan does.
3. **A Worker on `pigeon.dev` coordinates runs.** OpenTofu cannot run in a Worker, so each plan and apply runs the released cobot image in a Cloudflare Container in the pigeon account.
4. **Credentials live only in the pigeon account's Cloudflare Secrets Store.** None are in any repository.
5. **Grant once, mint per use.** For each provider, an instance gives the platform one standing grant. The platform mints short-lived credentials from it for each run, scoped to that run's capabilities.
6. **Provider catalogs declare capabilities, not secrets.**

**Terms.**

- **Platform**: pigeon, meaning this repository plus the pigeon Cloudflare account.
- **Instance**: one tenant of the platform. noisypigeon is the first and, for now, the only one.
- **Catalog**: what the platform can provision for one provider, meaning its schema, modules, presets and capabilities.
- **Capability**: a named, provider-scoped permission set, such as `cloudflare:dns.write`.
- **Grant**: an instance's standing credential for one provider, held by the platform.
- **Minted credential**: a short-lived credential, scoped to one run's capabilities.
- **Run**: one plan or one apply of one instance's units.

**Constraints.**

- The repository is public. No account ID, zone ID, credential or private resource name appears in it, in its history or in CI logs.
- Cloudflare Containers are a young product. Phase 3 starts with a spike that confirms the limits this design relies on, before anything depends on them:
  - an image the size of cobot's, with `tofu` and `zola`;
  - a run long enough for an apply;
  - outbound HTTPS to each provider;
  - Secrets Store bindings.
- Until phase 6's cut-over, the current CI-driven flow keeps working unchanged.

**Out of scope, each for a later ADR:**

- more than one instance in practice: sign-up, billing, quotas. The model is multi-tenant, but only noisypigeon is onboarded;
- a full UI. Phase 6 ships the API and MCP server, and only stubs the UI;
- any secret store other than the Cloudflare Secrets Store;
- gantry's scheduling and data operations;
- migrating providers beyond Cloudflare, DigitalOcean, Scaleway and Backblaze.

## Implementation plan.

### 1. Ownership

| Where | Holds |
| --- | --- |
| This repository (`noisypigeon/pigeon`, public) | `src/` (CLI, images, control plane), `catalog/` (provider catalogs), `infra/` (the platform's own bootstrap), ADRs and policies. |
| The pigeon Cloudflare account | The control-plane Worker, a Durable Object per instance, D1, R2, the Secrets Store, Containers and Cloudflare Access, as set out in §6. |
| An instance repository (noisypigeon's is new and private) | Static site content (`posts/`, `pages/`, `assets/`), and a read-only export of desired state that the platform commits after each apply. |
| The instance's own accounts | noisypigeon's are its Cloudflare account (`noisypigeon.com`, `willowgraysen.com`, `noisypigeon.garden`), DigitalOcean, Scaleway and Backblaze. The platform reaches them only through grants. |

`pigeon.dev` belongs to the platform, not to any instance.

### 2. Repository areas after the split

This supersedes parts of [`2026-11-02-development-repository-area-layout.md`](2026-11-02-development-repository-area-layout.md).

- **`catalog/<provider>/`** is product. It holds what `infra/<provider>/` holds today, minus instance data:
  - `schema.json`, `modules/` and `presets/`;
  - a new `capabilities.yaml` (§3).

  The catalog's release units keep their `<unit>/vX.Y.Z` tags. `infra/agnostic/` moves to `catalog/agnostic/`.
- **cobot learns the new location first.** It resolves schemas, modules and presets from `catalog/<provider>/`, falling back to `infra/<provider>/`. That support is released before any file moves, because `cobot.yml` runs the released image until phase 6. Every move PR must show a clean PR plan, and its merge a green apply.
- **`infra/`** holds only the platform's own bootstrap: the pigeon account's D1, R2, Durable Object namespaces, Secrets Store, Access applications and DNS for `pigeon.dev`.
  - It is applied locally, as break-glass, until phase 6. After that the platform applies its own bootstrap as a reserved instance named `pigeon`.
- **`workloads/`**, every instance `config.yaml` and `.config/instance.yaml` leave this repository in phase 7.
- `.config/policies/layout.yaml` follows each move in the PR that makes it.

### 3. Capabilities

Each catalog's `capabilities.yaml` maps a capability name to the provider permissions it needs and the scope it is minted at:

```yaml
description: Cloudflare capabilities.
capabilities:
  dns.write:        { permissions: [DNS Write], scope: zone }
  zone.read:        { permissions: [Zone Read], scope: zone }
  transform.write:  { permissions: [Zone Transform Rules Write], scope: zone }
  redirects.write:  { permissions: [Dynamic URL Redirects Write], scope: zone }
  r2.bucket.write:  { permissions: [Workers R2 Storage Bucket Item Write], scope: bucket }
```

- **What a unit names.** A unit's desired state names the capabilities it needs and the scope it needs them on, for example `dns.write` on `noisypigeon.com`. It never names a secret.
- **How cobot checks them.** `pigeon cobot capabilities` validates each catalog's file, and checks that every capability a unit requests exists and is minted at the scope the unit asks for.
- **Where today's secret values go:**

  | Today | After |
  | --- | --- |
  | `infra/cloudflare` `provisioner_token` | the instance's Cloudflare grant |
  | `infra/cloudflare` `account_id` | instance metadata in D1; not a secret, but never published |
  | apex Proton verification | ordinary desired state, because the values are published as TXT records anyway |
  | the planned DigitalOcean, Scaleway and Backblaze keys | each provider's grant |
  | workload tokens minted by `infra/cloudflare` | credentials minted per run (§4) |

### 4. Grants and minting

| Provider | Grant (one per instance) | Minted per run | Expiry |
| --- | --- | --- | --- |
| Cloudflare | an account token with Account API Tokens Write and the permissions it may delegate | an account token holding the run's permissions, scoped to the run's zones | `expires_on`, set to the run plus 1 hour |
| Scaleway | an IAM application key holding the policies it may delegate | an IAM API key on a per-run application | `expires_at`, set to the run plus 1 hour |
| Backblaze | an application key with `writeKeys` | an application key with the run's capabilities and buckets | `validDurationInSeconds`, set to the run plus 1 hour |
| DigitalOcean | a scoped personal access token | Spaces keys scoped to the run's buckets | none; the runner destroys them when the run ends, and a sweeper deletes any older than 1 day |

- **Grants.** They are entered once, by the instance's owner, into the Secrets Store through a platform endpoint. They are never read back, and only the minting code is bound to them.
- **Minted credentials:**
  - they live only for the run;
  - they are injected only into that run's Container environment;
  - they are never written to D1, R2, state, logs or any response.
- **Audit.** Every mint and every revocation is recorded in the audit table (§6): the capability names, the scopes and the minted ID, never the value.
- **Revoking a grant** stops all future runs of that instance on that provider at once.

### 5. Engine changes

These land in this repository as released engine features, release first, per [`2026-11-03-release-cobot-image-in-workflows.md`](2026-11-03-release-cobot-image-in-workflows.md).

- **cobot:**
  - A `Credentials` resolver feeds provider blocks from credentials the runner supplies, one per capability. It replaces `TF_VAR_secrets` and `TF_VAR_handoff`.
    - The existing `ENC` file backend stays behind it until phase 7, so today's flow keeps working.
  - OpenTofu state moves to R2, encrypted with a key from the Secrets Store. The git state directory becomes the transitional backend.
  - `pigeon cobot capabilities` lists and validates catalogs (§3).
- **backplane:**
  - `plan` and `apply` operations join the catalog, with the effects `plan` and `apply`.
  - `apply` takes the ID of a stored, unexpired plan, and refuses anything else.
  - `pigeon backplane serve` answers the control plane inside the Container, in the existing JSON wire form.
- **sentinel:** a new policy forbids adding a `secrets.config.yaml` or a key to one. It lands once its rule support is released. Phase 7 tightens it to forbid any `ENC[` in the repository.

### 6. The control plane

`src/pigeon-control/` is a new project and release unit. It is a TypeScript Worker on `pigeon.dev`. Backplane stays Rust, in the Container.

- **Durable Object, one per instance.** It serializes runs: one at a time per instance, with a queue behind it.
- **D1** holds:
  - instances;
  - desired state, versioned, one row per unit per version;
  - plans: their ID, the desired-state version, a summary, an expiry, and who requested them;
  - runs;
  - an audit table.
- **R2** holds encrypted OpenTofu state, and stored plan files.
- **Containers.** Each run starts the released `pigeon-backplane` image, with only that run's minted credentials and the state key.
- **API.**
  - Its routes mirror backplane's operations: `GET /v0/instances/:id`, `POST /v0/instances/:id/plans`, `POST /v0/instances/:id/applies { plan }`, and so on.
  - It speaks backplane's `Response` form.
- **Plan, then approve.** A desired-state change produces a stored plan. An apply must name that plan's ID before it expires, after 24 hours. This replaces pull-request review as the gate.
- **MCP server.**
  - It exposes the same operations under the same names.
  - Reading and planning are open to any authenticated client. Applying requires an approval through the API.
- **Access.** Cloudflare Access protects the API and the MCP server, and every call writes an audit row.
- **Drift.** A daily scheduled plan per instance replaces `cobot.yml`'s daily apply. A plan with changes is a deviation, recorded as an alert on the instance.

### 7. Migration phases

Each phase is one or more PRs. A phase starts only once the one before it is merged and released.

| Phase | Work | Needs the owner to |
| --- | --- | --- |
| 0 | This ADR, and the amendments it adds to the ADRs it supersedes. | — |
| 1a | Freeze and catalog support, as released engine changes:<br>• the no-new-secrets policy;<br>• `capabilities.yaml` and `pigeon cobot capabilities`;<br>• cobot's `catalog/` resolution with its `infra/` fallback (§2). | close #69, #72 and #73 |
| 1b | Once 1a is released:<br>• `infra/<provider>/` catalog files move to `catalog/`;<br>• `pigeon.dev` leaves `infra/cloudflare`'s zones;<br>• the DigitalOcean, Scaleway and Backblaze units are redone as catalogs that declare capabilities and no secrets, replacing #69, #72 and #73.<br>Each move is shown by an unchanged PR plan and a green apply on merge. | — |
| 2 | The pigeon account's bootstrap units in `infra/`, applied locally. | create a bootstrap token for the pigeon account and apply locally |
| 3 | Runner spike (Context), then: cobot's `Credentials` resolver and R2 state; `backplane serve`, `plan` and `apply`. | — |
| 4 | `src/pigeon-control/`: Durable Objects, D1 schema, API, plan-then-approve, grant entry and minting for each provider in §4. | enter each provider's grant |
| 5 | Migrate noisypigeon:<br>• import each unit's `config.yaml` into D1;<br>• import state from the state repository into R2;<br>• run a platform plan, which must show no changes. | — |
| 6 | Cut over:<br>• the platform applies;<br>• `cobot.yml`'s apply job and daily apply are removed;<br>• the MCP server ships;<br>• the UI is a stub.<br>Every credential that ever lived in the repository is rotated and revoked: `provisioner_token`, the Proton verification values (re-verified), the three passphrases and the `COBOT_*` Actions secrets. | revoke the old tokens in each provider's console |
| 7 | Split:<br>• create noisypigeon's instance repository;<br>• move `workloads/` site content there;<br>• remove the `ENC` backend, `secrets.config.yaml` files and `sites.yml`'s publish, because the platform builds sites from the instance repository;<br>• archive the state repository. | create the instance repository, or allow it |

### 8. Verification

- Every phase passes `mise run ci`, and follows the release-first loop for each engine feature it needs.
- **Phase 1a:** a PR that adds a `secrets.config.yaml` key fails `sentinel`.
- **Phase 1b:** every catalog move leaves `cobot.yml`'s PR plan unchanged, and its merge applies green.
- **Phase 3:** the spike's report shows each limit in Context met, or this ADR is amended before phase 4 begins.
- **Phase 4:** an apply naming an expired or unknown plan is refused, and a minted credential works only on its run's scopes.
- **Phase 5:** the platform's plan is empty, the same as `cobot.yml`'s last plan.
- **Phase 6:** every old credential is revoked, and a plan that uses one fails.
- **Phase 7:** no `ENC[` remains anywhere in the repository, and sentinel enforces that.

## Amendments

### 2026-11-04: The control plane is Rust, built from backplane

The control plane is written in Rust and is part of backplane, not a separate TypeScript project. This supersedes:

- §5's "The Worker's language is TypeScript. Backplane stays Rust inside the Container.";
- §6's first paragraph, which made `src/pigeon-control/` a TypeScript Worker and its own release unit;
- `src/pigeon-control/` in phase 4.

That text stays as written.

#### 1. One crate, two builds

`src/pigeon-cli/crates/backplane/` (`pigeon-backplane`) builds two ways, chosen by cargo feature:

- **`worker`**: built for `wasm32-unknown-unknown` with `workers-rs` (the `worker` crate). It is §6's control plane:
  - the HTTP API;
  - the MCP server, which speaks JSON-RPC over streamable HTTP and maps each tool onto a catalog operation;
  - the per-instance Durable Object;
  - D1, R2 and Secrets Store access;
  - the Cron Trigger for the daily drift plan.

  It coordinates, and acts only through cobot. It calls cobot's `worker` build (below) to start a run's Container and to mint that run's credentials.
- **`engines`** (default, native): today's composition of `sentinel`, `cobot` and `gantry`. It runs in the Container as `pigeon backplane serve`.
- **Shared, with no feature and wasm-safe:**
  - the operation catalog (`Operation` and `Effect`);
  - the wire form (`Request` and `Response`);
  - the instance and plan types;
  - capability checks.

  The Worker and the Container compile one definition of the API from the same source.

**The actions stay cobot's**, per [`2026-11-02-development-sentinel-policy-cobot-actions.md`](2026-11-02-development-sentinel-policy-cobot-actions.md) and backplane's own ADR:

- `cobot` gains a wasm-safe `worker` feature, holding:
  - Container dispatch, which starts a run's Container through its binding with only that run's credentials and the state key;
  - grant minting and revocation, which are plain HTTPS to each provider's API (§4).
- That feature carries none of cobot's process-starting code (`tofu`, `zola`, `git`).

**Dependencies.**

- In backplane, `sentinel` and `gantry` are optional and enabled only by `engines`.
- `cobot` is required by both builds: with its default features under `engines`, and with only `worker` under backplane's `worker` build.
- `.config/policies/backplane.yaml` keeps covering all of backplane's source unchanged, so backplane still starts nothing and writes nothing itself.

#### 2. Release and deploy

- The `backplane` release unit gains a second artifact: the Worker bundle, built with `worker-build` and attached to its GitHub Release. The image is unchanged.
- Deploying is an action, so it is cobot's, per [`2026-11-02-development-sentinel-policy-cobot-actions.md`](2026-11-02-development-sentinel-policy-cobot-actions.md).
  - The platform's `infra/` bootstrap unit deploys the released bundle with the Cloudflare provider's Worker script resource.
  - It also deploys the bundle's bindings to Durable Objects, D1, R2, the Secrets Store and Containers.
  - No PR deploys the Worker by hand.

#### 3. Toolchain

- `mise.toml`'s pinned Rust adds the `wasm32-unknown-unknown` target.
- `mise run features` gains two clippy lines, each with `--no-default-features --features worker --target wasm32-unknown-unknown -- -D warnings`: one for `pigeon-backplane`, one for `cobot`.
- `rust.yml` builds the Worker bundle on every PR that changes Rust, without deploying it.

#### 4. No TypeScript unless the spike forces it

Phase 3's spike also checks that each binding the Worker needs works from `workers-rs`:

- Durable Objects;
- D1;
- R2;
- the Secrets Store;
- Containers;
- Cron Triggers.

If one doesn't work, the spike's report names it, and a further amendment adds the smallest TypeScript shim that exposes only that binding to the Rust Worker. Such a shim is its own release unit and holds no API or MCP logic.

#### 5. Phase 4, restated

Phase 4 is:

- backplane's `worker` build: Durable Objects, the D1 schema, the API, the MCP server, plan-then-approve and grant entry;
- cobot's `worker` build: Container dispatch, and minting for each provider in §4.

#### 6. Verification

- Backplane's and cobot's `worker` builds pass clippy for wasm32, and the bundle builds in CI.
- `.config/policies/backplane.yaml` still holds: no Container dispatch or minting code in backplane's source.
- A test shows that a `Response` serialized by the `engines` build deserializes in the `worker` build.
- The phase 3 spike report lists each binding in step 4 as working from Rust, or names its shim.

### 2026-11-04: Phase 2, the pigeon-account bootstrap

Phase 2 applies `infra/`'s bootstrap of the pigeon account locally, as break-glass. That runs into four constraints:

- the repository may hold no new secret, because of the freeze in `.config/policies/secrets*.yaml`;
- it may publish no account ID;
- `cobot.yml` plans and applies every unit in CI, which has no credentials for the pigeon account;
- the state repository is CI's, and is never applied against by hand.

This amendment decides how phase 2 meets them. It details phase 2's row of §7; the text above stays as written.

#### 1. The unit

- `infra/pigeon/` is a self-contained config unit with its own `schema.json` and `modules/`. It is the platform's bootstrap, not an instance's, so it uses no provider catalog. A unit's own `schema.json` already wins over a catalog.
- **Its first resources:**
  - a D1 database, for §6's desired state, plans, runs and audit;
  - two R2 buckets, one for state and one for plans. They are named `<key>-<suffix>` with `agnostic/random-string`, so no private bucket name is published.
- **Deferred to phase 4, which deploys the Worker:**
  - Durable Object namespaces, which come from the Worker's deploy;
  - Cloudflare Access, which needs the API hostname;
  - the Secrets Store, which joins once its resource is confirmed in the pinned provider.

#### 2. Credentials come from the operator's environment

- `x-cobot.providers.cloudflare` is `{}`, and `x-cobot.secrets` is `[]`. The provider reads `CLOUDFLARE_API_TOKEN` from the environment of the person running the apply.
- The account ID is looked up through that token: an `account` module reads `data "cloudflare_accounts"` and requires exactly one account. A token scoped to the pigeon account therefore names it, and the ID is never written down.
- **The bootstrap token's permissions** are Account Settings Read, D1 Edit and Workers R2 Storage Edit, on the pigeon account only. The token lives only in the operator's shell.

#### 3. Local units

- A schema may set `x-cobot.local: true`.
- A local unit is validated everywhere, CI included.
- Unit selection includes it only when it is named. So `cobot plan` and `cobot apply` with no unit named, which is how CI runs them, skip it, and `pigeon cobot plan infra/pigeon --state …` reaches it.
- This is an engine change, released before the unit lands, per the release-first loop.

#### 4. State

- **Where it lives.** The bootstrap's encrypted state lives in a new private repository, `noisypigeon/pigeon-platform-state`, cloned locally and passed as `--state`.
- **Its passphrase** is a `COBOT_STATE_PASSPHRASE` of its own, different from the instance's.
- **Later.** Phase 3 moves the state into the R2 state bucket this unit creates.
- **The rule.** `pigeon-cobot-state` is never used for it.

#### 5. Execution

1. This amendment lands first.
2. `feat(cobot): local units run only when named`, holding §3. It must be merged and released before step 3.
3. `feat(infra): the pigeon-account bootstrap unit`. It adds:
   - `infra/pigeon/` with `local: true`, and modules `account`, `d1-database` and `r2-bucket`;
   - a runbook in `infra/pigeon/README.md`;
   - release units and tiers for the new modules.

   Its CI plan must not list `infra/pigeon`.
4. **The owner:**
   - creates `noisypigeon/pigeon-platform-state` and the scoped token;
   - plans and applies `infra/pigeon` locally, following the runbook;
   - pushes the encrypted state.

#### 6. Verification

- A local unit is absent from `cobot plan` with no units named, and present when named. Tests cover both, and a non-boolean `local`.
- After step 3, the CI plan comment lists exactly the four existing units.
- `pigeon cobot render infra/pigeon` renders with no credentials.
- No account ID, token or private bucket name appears in the repository.

### 2026-11-04: The bootstrap composes the Cloudflare catalog

The phase 2 amendment gave `infra/pigeon/` its own `modules/`, because "it uses no provider catalog". That made `r2-bucket` a copy of the catalog's, and it left `account` and `d1-database` where no other unit can use them. Catalogs are the platform's product, so the platform's own bootstrap consumes them like any unit. This amendment supersedes that line of §1, and retires `infra/agnostic`.

#### 1. Catalog module references

- A block's `module` may be `<catalog>/<name>`. It resolves to `<catalog home>/modules/<name>`, where the home is `catalog/<catalog>/`, falling back to `infra/<catalog>/` as in §2.
- `agnostic/<name>` becomes one case of that form. This extends §3 of [`2026-11-03-runtime-infra-agnostic-shared-modules.md`](2026-11-03-runtime-infra-agnostic-shared-modules.md), whose out-of-scope list named "modules shared from anywhere but `infra/agnostic`".
- A bare `<name>` still names the unit's own `modules/`, or its catalog's.
- An unknown catalog or module fails validation, naming the path it looked for.

#### 2. The Cloudflare catalog

- `account` and `d1-database` move from `infra/pigeon/modules/` to `catalog/cloudflare/modules/`, as release units `infra-cloudflare-account` and `infra-cloudflare-d1-database`, starting at v0.1.0.
- `r2-bucket` gains an optional `suffix`, defaulting to `null`. With a suffix, the bucket is named `<name>-<suffix>`; without one, `<name>`, so existing buckets plan no changes.

#### 3. `infra/pigeon/`

- It holds only `schema.json`, `config.yaml` and its README.
- Its blocks name `cloudflare/account`, `cloudflare/d1-database`, `cloudflare/r2-bucket` and `agnostic/random-string`. Block names don't change, so neither do state addresses.
- Release unit `infra-pigeon-schema` follows the catalog modules it composes. `infra-pigeon-account`, `infra-pigeon-d1-database` and `infra-pigeon-r2-bucket` are retired; their tags stay.

#### 4. Retired changelogs

- A PR never edits a changelog, so a retired unit's `CHANGELOG.md` is removed by the release bot.
- `.config/releases.yaml` gains `retired`, a list of changelog paths that no tier names. The bot deletes each one it finds. A later PR drops the spent entries, as with `moves.yaml`.

#### 5. `infra/agnostic` retires

- It is a unit with no resources. `catalog/agnostic/` holds the modules, and is reached through §1.
- `infra/agnostic/` is deleted, and so is the spent `infra/bootstrap → infra/agnostic` entry in `.config/moves.yaml`.
- Its empty state stays in `pigeon-cobot-state`, unread, until phase 7 archives that repository.
- `infra/cloudflare` is unchanged. It is the instance's provisioner, and it retires at phase 6.

#### 6. Execution

1. This amendment.
2. `feat(cobot)`: §1, and §4's `retired`. Merged and released before step 4, because `cobot.yml` validates with the released image.
3. `feat(catalog)`: §2. It needs no engine change.
4. `refactor(infra)`: §3 and §5.

The owner's first apply of `infra/pigeon` waits for step 4.

#### 7. Verification

- Tests cover `<catalog>/<name>` resolution, an unknown catalog or module, and the bot deleting a `retired` changelog.
- Every PR's CI plan lists the instance's units with no resource changes, and step 4's list no longer includes `infra/agnostic`.
- After step 4, `cobot render infra/pigeon` sources every module from `catalog/`, and `infra/pigeon/modules/` is gone.

### 2026-11-04: Phase 3, the runner spike and engine changes

This details phase 3's row of §7. The phase starts with a spike, and its engine changes don't wait on the spike's result, which gates only phase 4's reliance on Containers.

#### 1. The spike kit

- It lives in `tooling/spikes/containers/`, outside the Cargo workspace, and is never a release unit.
- **The Worker** uses Rust and `workers-rs`. It exposes one route per check, and every route needs a bearer key set as a Worker secret.
- **The Container image** is built `FROM` a pinned, released `ghcr.io/noisypigeon/pigeon-cobot` image, plus a small check server.
- **`wrangler.toml`** holds no account ID. It comes from `CLOUDFLARE_ACCOUNT_ID` in the operator's shell.
- **Its README is the runbook**, teardown included. The owner deploys it with a short-lived token, and pastes each route's JSON into the report.

#### 2. What the spike checks

| Route | Confirms |
| --- | --- |
| `/image` | The Container starts the cobot-sized image and reports its `tofu` and `zola` versions. |
| `/run?minutes=N` | A run of N minutes (default 20) survives. |
| `/egress` | Outbound HTTPS reaches the Cloudflare, Scaleway, Backblaze and DigitalOcean APIs. |
| `/secrets` | A Secrets Store binding is readable. It reports presence and length only, never the value. |
| `/do`, `/d1`, `/r2` | Durable Objects, D1 and R2 work from Rust. |
| `scheduled` | A Cron Trigger fires in Rust and writes a D1 row, which `/d1` reports. |

Containers' binding from Rust is confirmed by `/image` itself. A later amendment, the spike report, records each result, and names any binding that needs amendment 1 §4's TypeScript shim.

#### 3. Credentials

- **Requesting.** `x-cobot.credentials` maps a name to `{ provider, capabilities, scope }`. Each capability must exist in that provider's `capabilities.yaml`, at the scope it is minted at (§3).
- **Using.** A provider setting references one as `$credential.<name>.<field>`. Every reference renders into one sensitive variable, `var.credentials`.
- **Sources.**
  - `runner`: JSON in `COBOT_CREDENTIALS`, which the Container supplies. Phase 4 mints the values.
  - `files`: today's `ENC` secrets and `infra/cloudflare` handoff, re-expressed as credentials.
- `$secret` and `$handoff` keep working through `files` until phase 7. Existing units render unchanged.
- Credential values only ever reach the run's environment, never a log, plan summary or response.

#### 4. State in R2

- `--state` takes a directory, as today, or `r2://<bucket>/<prefix>`. The R2 endpoint and keys come from `COBOT_STATE_R2_*`.
- cobot downloads a unit's encrypted state into its work directory, and runs tofu on the local backend unchanged. It then uploads the result with `If-Match` on the ETag it read, or `If-None-Match: *` for a first apply.
- An ETag that changed in between fails the run. The Durable Object already serializes an instance's runs, so this guards only against a second writer.
- `TF_ENCRYPTION` is unchanged. Its passphrase comes from the Secrets Store at run time.
- `s3.rs` gains binary-safe reads that return the ETag, and conditional writes. Tests use an in-process fake S3 server.

#### 5. Stored plans

- `cobot plan --store` saves the encrypted `.tfplan` under the state store's `plans/<id>/`, beside its metadata:
  - the unit;
  - a digest of the unit's desired state, meaning its `config.yaml` and `schema.json`;
  - when it was created;
  - when it expires, 24 hours later.
- `cobot apply --plan <id>` applies exactly that file. It refuses a plan that is unknown, expired, or whose digest no longer matches.

#### 6. backplane

- **Operations.** `infra.plan` (effect `plan`, optional arg `units`) and `infra.apply` (effect `apply`, arg `plan`) join the catalog, and act only through cobot.
- **`run`** accepts these operations' arguments. `pigeon backplane run` gains `--arg k=v`.
- **`pigeon backplane serve --listen <addr>`** answers:
  - `GET /healthz`;
  - `GET /v0/operations`;
  - `POST /v0/run`, which takes a `Request` and returns a `Response`.
- **The listen address is required**, with no default. In the Container, only the Worker reaches that port.
- **The server is `tiny_http`**: small and synchronous, with no async runtime. `.config/policies/backplane.yaml` still holds, because serving starts no process and writes no file.

#### 7. Execution

1. This amendment.
2. `feat(tooling)`: the spike kit (§1, §2). It also drops the spent `retired` entries from `.config/releases.yaml`.
3. The owner deploys the spike, and `docs(adr)` records its report. If a result breaks an assumption, that report amends the affected design before steps 4 to 7 rely on it.
4. `feat(cobot)`: credentials (§3).
5. `feat(cobot)`: state in R2 (§4).
6. `feat(cobot)`: stored plans (§5).
7. `feat(backplane)`: plan, apply and serve (§6).

Minting, the Worker, the D1 schema, the API and Container dispatch remain phase 4. `cobot.yml` keeps today's flow until phase 6.

#### 8. Verification

- Steps 4 to 7 each leave CI's cobot plan unchanged for the instance's units.
- Tests cover:
  - existing units rendering byte-identically;
  - an unknown capability failing;
  - runner credentials reaching only `TF_VAR_credentials`;
  - R2 state round-tripping, and an ETag conflict;
  - a plan that is expired, mismatched or unknown being refused;
  - the wire form round-tripping over HTTP.
- The spike report gives every check and binding a result from the owner's deploy.

### 2026-11-04: Worker–Container mutual TLS

This amends phase 3's §4 and §6. It records two decisions made in their PRs, and replaces `serve`'s plain HTTP with mutual TLS, for these reasons:

- `serve` carries a bearer token that can run `infra.apply`. On plain HTTP, anyone who observes the Worker–Container link could replay the token.
- `tiny_http`'s TLS is `rustls` 0.20, an old major version, while the workspace is on 0.23. It also cannot verify client certificates.

#### 1. Decisions recorded from phase 3's PRs

- **Removed state.** A removed state object is overwritten with a tombstone body through the same `If-Match` `PUT` as any update, never `DELETE`d.
  - A conditional `PUT` is atomic in R2. A read-then-`DELETE` is not, if `If-Match` on `DELETE` is ignored.
  - A pull treats a tombstone as absent.
- **`serve`.**
  - It requires a bearer token of at least 32 characters, from `BACKPLANE_TOKEN`, on every `/v0/` route.
  - It binds its `state` at startup. A request may name only that `state`.
  - It binds only loopback unless `--network` is set.
  - It answers on a fixed pool of 16 workers. A second plan or apply while one runs gets 409.

#### 2. The link

- The Worker and the Container talk over mutual TLS 1.3.
- The token stays as a second, independent check. A request needs both a valid client certificate and the token.

#### 3. Trust

- **Per instance**, one private CA issues two certificates:
  - the Container's server certificate;
  - the Worker's client certificate.
- **Each side trusts only that CA** for the other side.
- **Certificates live 90 days at most.**
  - In phase 3, the owner issues them by runbook with `openssl`, and keeps the CA key offline.
  - Phase 4 automates issuance and rotation with minting.
- **Where each piece lives.**
  - The Container's certificate, key and the client CA are in the Secrets Store. They are injected into the Container's environment as PEM, in `BACKPLANE_TLS_CERT`, `BACKPLANE_TLS_KEY` and `BACKPLANE_TLS_CLIENT_CA`.
  - The Worker's client certificate is in a Worker mTLS certificate binding.
  - None of it is ever in the repository.

#### 4. The server

- **The stack.** `serve` drops `tiny_http` for a small synchronous server, built on `std::net::TcpListener`, `rustls` 0.23 with `WebPkiClientVerifier`, and `httparse`.
- **Connections.**
  - The same 16 workers pull accepted connections.
  - Each connection has read and write timeouts, and serves one request.
  - A stalled body is released by the timeout, not held.
- **Unchanged:**
  - the routes, status codes and wire form;
  - the token check and server-owned `state`;
  - validation before the busy check.
- **TLS is required with `--network`.** It needs all three `BACKPLANE_TLS_*` variables, and refuses to start without them.
  - Loopback without TLS stays allowed for local use.
  - Backplane reads PEM from the environment, never files, so `.config/policies/backplane.yaml` still holds.

#### 5. What the spike confirms

The spike kit gains two routes:

- `/tls`: an `https://` fetch through the container binding to a probe that serves TLS with a test CA. It reports whether the Worker verified the probe's certificate.
- `/mtls`: the same fetch with an `mtls_certificates` binding. The probe reports whether it received a client certificate.

If the spike fails part of this, the fallback order is:

1. If the Worker cannot verify a private CA on the container binding, the Container still requires the client certificate, and the bearer token never travels.
   - **Requests are signed.** Each request carries an HMAC-SHA-256 under the shared `BACKPLANE_TOKEN`, over its method, path, body digest, a timestamp and a random nonce.
   - **The signature replaces the bearer check.** On `/v0/` routes, `serve` authenticates by the verified signature instead of `Authorization: Bearer`. Outside fallback 1, the bearer check stays as §2 and §4 describe.
   - **The Container checks each signature.** It refuses a timestamp more than 60 seconds old, and a nonce it has already seen within that window.
   - **Responses are signed too.** The Container signs each response over the request's nonce, the response's status code and its body's digest, and the Worker verifies it. A changed status (a 409 made 200, say) fails like a changed body.
   - **Both sides are authenticated.** An unverified server learns nothing it can replay, and cannot answer as the Container.
2. If the Worker cannot present a client certificate either, the decision returns to the owner with the spike's results. This amendment does not pre-decide it.

#### 6. Execution

1. This amendment.
2. `feat(tooling)`: §5's spike checks, and their runbook.
3. `feat(backplane)`: §4's server. It does not depend on the spike.
4. In phase 4: the Worker's mTLS binding, Secrets Store injection, and rotation through minting.

#### 7. Verification

- Tests mint a CA, a server certificate and client certificates at test time, with `rcgen` as a dev-dependency only. They cover:
  - every existing `serve` test, over TLS;
  - a missing client certificate, or one from another CA, rejected at the handshake;
  - a valid certificate without the token, which gets 401;
  - plain HTTP to the TLS port, which fails;
  - `--network` without TLS, refused;
  - a stalled body, released.
- If the spike takes fallback 1, its PR tests signed requests and responses, refusing:
  - a bad signature;
  - a stale timestamp;
  - a reused nonce;
  - a tampered response body or status;
  - a signed request that carries no bearer header, which is accepted, while an unsigned one is refused.
- The spike report gives `/tls` and `/mtls` results from the owner's deploy.
