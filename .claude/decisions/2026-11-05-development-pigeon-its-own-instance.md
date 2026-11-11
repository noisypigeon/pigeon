## pigeon is its own instance; noisypigeon moves to its own repository

* Date: 2026-11-05
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development
* Governs: `.config/instance.yaml`, `config.yaml`, `src/pigeon-cli/crates/backplane/src/instance.rs`, `src/pigeon-cli/crates/cobot/src/engines.rs`, `.config/policies/secrets*.yaml`, `workloads/README.md`

## Context

[`2026-11-04-development-platform-instance-separation.md`](2026-11-04-development-platform-instance-separation.md) splits pigeon, the product, from noisypigeon, its first instance. It does so only at phase 7, after the control plane takes over every apply. Until then this repository carries both, so it holds:

- **noisypigeon's units:** `workloads/noisypigeon.com/` (its apex and blog), and `infra/cloudflare/`, which mints a scoped token for each workload;
- **their secrets:** two `secrets.config.yaml` files, frozen by `.config/policies/secrets*.yaml`;
- **a GitHub Actions control plane:** `cobot.yml` plans on a pull request and applies on merge. `sites.yml` publishes the blog. Both read the `COBOT_*` passphrases and check out the private `noisypigeon/pigeon-cobot-state`.

That makes every pigeon pull request depend on noisypigeon's credentials and state. Its checks call `sentinel` and `cobot` directly, besides backplane. The owner wants a shape that is linear and easy to maintain:

- noisypigeon moves out now, to `noisypigeon/noisypigeon`, which exists;
- `pigeon backplane` is this repository's single entry point;
- this repository is its own instance, for development and enforcement, with no credentials in its CI;
- plans and applies run only from the CLI, outside the repository, until the control plane ([`2026-11-05-runtime-control-plane-worker.md`](2026-11-05-runtime-control-plane-worker.md)) takes over.

## Implementation plan.

### 1. Two repositories

| Repository | Holds |
| --- | --- |
| `noisypigeon/pigeon` (public) | `src/`, `catalog/`, `infra/pigeon/`, `tooling/`, `.claude/` and `.config/`. Its `.config/instance.yaml` is `name: pigeon`, and its policies are its own `.config/policies/`. |
| `noisypigeon/noisypigeon` | `.config/instance.yaml` (`name: noisypigeon`), `workloads/noisypigeon.com/` (its configs and the blog's `site/`), and a README. It holds no secret, ID, bucket name, state or workflow, so it may be public. |

- **The move is one fresh commit** in `noisypigeon/noisypigeon`. The history stays readable in this repository, which then deletes the moved files.
- **Unit paths are unchanged:** `workloads/noisypigeon.com/apex` and `workloads/noisypigeon.com/blog`. Their state in `noisypigeon/pigeon-cobot-state` still matches them.
- **`infra/cloudflare/` does not move.** It is retired (see 4).

### 2. backplane is the single entry point

- **Calls.** This repository's workflows and `mise` tasks call `pigeon backplane run <operation>`, never `pigeon sentinel` or `pigeon cobot` directly.
  - `policy.check` gains the pull-request fields (`title`, `body`, `number`, `files`) as arguments.
  - `sites.build` joins the catalog, building every site through cobot with no credentials.
  - `instance.show`, `infra.validate` and `architecture.describe` are already there.
- **The engine subcommands stay** for local work and tests. Only the repository's own automation is held to backplane.

### 3. CI holds no credentials

- **What CI can do.** This repository's CI runs only what needs no secret: formatting, lints and tests, `policy.check`, `infra.validate`, `architecture.describe` and `sites.build`.
- **What goes:**
  - `cobot.yml`;
  - `sites.yml`'s publish job;
  - every `secrets.COBOT_*` reference;
  - the `pigeon-cobot-state` checkout.
- **The release bot stays.** Publishing releases and images is not state, per [`2026-11-02-release-crate-units-and-images.md`](2026-11-02-release-crate-units-and-images.md).
- **Applies run only from the CLI.** The owner runs the released CLI from a `noisypigeon/noisypigeon` checkout:

  ```
  pigeon backplane run infra.plan  --root <noisypigeon> --arg state=<pigeon-cobot-state checkout>
  pigeon backplane run infra.apply --root <noisypigeon> --arg state=<…> --arg plan=<id>
  ```

  - A scoped `CLOUDFLARE_API_TOKEN` is in the environment.
  - Publishing a site is `sites.publish`, run the same way.
  - The control plane replaces these by-hand runs. The plan-then-apply discipline is the same.

### 4. Workloads use the catalogs directly

- **Credentials and account.** `catalog/cloudflare/schema.json`'s `$defs.apex` and `$defs.workload` change in two ways, following `infra/pigeon/`:
  - they take the provider's token from the environment (`providers: { cloudflare: {} }`), or from the runner's `COBOT_CREDENTIALS`;
  - they take the account from the `cloudflare/account` module.
- **No per-workload minting.** cobot no longer reads tokens from `infra/cloudflare`'s outputs. That covers the apex's and the static site's token checks, and `sites publish`, which derives R2 S3 credentials from the run's own Cloudflare token.
- **The Proton verification values become plain config.** They are published as TXT records anyway, which is how the separation ADR §4 already classes them.
- **Nothing is secret any more.** No workload needs a `secrets.config.yaml`.
  - `infra/cloudflare/` is retired. The owner revokes its minted tokens and `provisioner_token`, then removes its state.
  - The `secrets*.yaml` policies are deleted with the files they freeze.
- **Resource addresses do not change.** The owner's first plan from `noisypigeon/noisypigeon` must show no destroy and no replace.

### 5. What this supersedes

The superseded texts stay as written:

- **In the separation ADR:**
  - the order of phases 5 to 7: the split comes first;
  - `cobot.yml` as the interim control plane;
  - `sites.yml`'s publish.
- **The per-workload minting** in [`2026-11-02-runtime-cloudflare-provisioner-unit.md`](2026-11-02-runtime-cloudflare-provisioner-unit.md).
- **The control-plane ADR's** "A run's instance root, built from D1" amendment, execution step 2. noisypigeon invokes no catalog policy for now. sentinel's invocations stay available for when it does.
- **`workloads/` leaving this repository's layout,** per [`2026-11-02-development-repository-area-layout.md`](2026-11-02-development-repository-area-layout.md).

### 6. Execution

Each step is released before the next relies on it.

1. This ADR, and its `CLAUDE.md` pointers.
2. `feat(catalog)` and `feat(cobot)`: workloads without `infra/cloudflare`, as 4 describes.
3. `feat(backplane)`: `policy.check`'s pull-request arguments, and `sites.build`. The `mise` tasks call backplane.
4. `noisypigeon/noisypigeon`'s first commit:
   - the copied workloads in the new shape, with the verification values inline and no secrets;
   - `.config/instance.yaml`;
   - a README with the owner's plan and apply commands.
5. **The owner, by hand:**
   - plan both units from that checkout; the plan shows no destroy or replace;
   - apply;
   - check the blog live;
   - revoke `infra/cloudflare`'s tokens, and remove its state.
6. `chore`: this repository becomes its own instance.
   - **Removed:** `workloads/`, `infra/cloudflare/`, `cobot.yml`, `sites.yml`'s publish and the `secrets*.yaml` policies.
   - **Changed:** `.config/instance.yaml` becomes `pigeon`, `layout.yaml` stops requiring `workloads/`, the workflows call backplane, and `CLAUDE.md` is updated.
   - **Then the owner** deletes the `COBOT_*` Actions secrets.
7. `refactor(cobot)`: the `ENC` secrets backend is removed once nothing uses it.

### 7. Verification

- No workflow in this repository references a `COBOT_*` secret or a state repository.
- On a fresh clone, `policy.check` and `infra.validate` run with no environment at all.
- `noisypigeon/noisypigeon` holds no `ENC[`, ID or bucket name.
- The first plan from `noisypigeon/noisypigeon` shows no destroy or replace, and the blog serves after its apply.

## Amendments

### 2026-11-05: The CLI runs from pigeon; `--instance` names the instance

§3 has the owner run the CLI from a `noisypigeon/noisypigeon` checkout. That checkout holds none of the platform's parts, and cobot today resolves everything from one root: units, catalogs, providers, and the site templates with their tags. The owner chose the reverse of copying those parts in: **the CLI runs from pigeon, and `--instance DIR` overrides the instance.**

#### 1. Two roots

| Root | Where | Holds |
| --- | --- | --- |
| Platform | Where the CLI runs: a pigeon checkout. | `catalog/`; `.config/providers.yaml` and `providers.lock.hcl`; `src/site-templates/` and their tags. |
| Instance | `--instance DIR`, defaulting to the platform root. | `.config/instance.yaml`; its `infra/` and `workloads/` units; each site's `site/`; `.config/moves.yaml`. |

- **pigeon is unchanged.** Without `--instance`, the two roots are the same directory, so pigeon stays its own instance with every current command.
- **State paths stay each unit's path relative to the instance root.** `workloads/noisypigeon.com/apex` and `…/blog` keep their state in `noisypigeon/pigeon-cobot-state`.

#### 2. Commands

```
cd pigeon
pigeon backplane run infra.plan  --instance ../noisypigeon --arg state=<pigeon-cobot-state checkout>
pigeon backplane run infra.apply --instance ../noisypigeon --arg state=<…> --arg plan=<id>
```

- `pigeon cobot validate|render|plan|apply|sites build|sites publish` and `pigeon backplane instance|run|serve` all take `--instance DIR`.
- A scoped `CLOUDFLARE_API_TOKEN` stays in the environment, as §3 has it.

#### 3. The instance's manifest

- **Optional paths.** `paths.providers` and `paths.releases` become optional. Without them, the platform's are used.
- **Policies.** `paths.policies` becomes optional too. An instance without it runs no policies.
- **The platform's rules stay the platform's.** With `--instance`, `policy.check` and `architecture.describe` read only the instance's own policies. This repository's `.config/policies/` are its development rules, and they never judge another instance.

#### 4. Supersedes

§3's command lines, which ran from a `noisypigeon/noisypigeon` checkout, and §6 step 4's README there. That README goes to `workloads/README.md`, because the repository's own `README.md` is the owner's GitHub profile.

#### 5. Execution

1. This amendment.
2. `feat(cobot)` and `feat(backplane)`: the two roots, and `--instance` across `pigeon cobot` and `pigeon backplane`, released before step 3.
3. §6 steps 4 to 7, run from pigeon with `--instance`.

#### 6. Verification

- Across a separate platform and instance:
  - `infra.validate`, `sites.build` and a plan with a run token hold;
  - each state path is relative to the instance.
- Without `--instance`, every existing test holds unchanged.
- `policy.check --instance` runs none of the platform's `.config/policies/`.
