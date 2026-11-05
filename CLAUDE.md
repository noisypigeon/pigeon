# CLAUDE.md

## Comments are terse; ADRs hold the reasoning

A comment in code or config says what something is, in a line or two. When the
why matters, cite the ADR that holds it. Rationale, history and alternatives
belong in the ADR, never in the file.

## Pointers

- Decisions are ADRs in `.claude/decisions/`, written before the code. Their
  naming, template and amendments are in
  [`.claude/decisions/README.md`](.claude/decisions/README.md).
- The development lifecycle (ADR PR, then autonomous implementation PR, squash
  merges, PR conventions) is
  [`2026-11-02-development-adr-driven-lifecycle.md`](.claude/decisions/2026-11-02-development-adr-driven-lifecycle.md).
- The Rust workspace and its Mise-pinned toolchain are
  [`2026-11-02-development-cargo-workspace-mise-toolchain.md`](.claude/decisions/2026-11-02-development-cargo-workspace-mise-toolchain.md).
  Crates live in `src/pigeon-cli/crates/<name>/`.
- The repository's areas are `src/<project>/`, `tooling/`, `infra/` and
  `workloads/`, per
  [`2026-11-02-development-repository-area-layout.md`](.claude/decisions/2026-11-02-development-repository-area-layout.md).
- `pigeon sentinel` enforces the policies in `.config/policies/`, one YAML file
  per subject.
- Sentinel is policy and its enforcement; cobot performs the actions policies
  govern (releases, changelogs, tags, infrastructure). Sentinel's library
  writes no file and starts no process, per
  [`2026-11-02-development-sentinel-policy-cobot-actions.md`](.claude/decisions/2026-11-02-development-sentinel-policy-cobot-actions.md).
- Provider catalogs (`schema.json`, `modules/`, `presets/`, `capabilities.yaml`)
  live in `catalog/<provider>/` (today `cloudflare`, `digitalocean`, `scaleway`,
  `backblaze` and `agnostic`); a catalog declares capabilities and never mints
  keys or holds secrets; units stay in `infra/` and `workloads/`, and an
  `infra/<provider>/` that still holds its own `schema.json` wins over its
  catalog, per
  [`2026-11-04-development-platform-instance-separation.md`](.claude/decisions/2026-11-04-development-platform-instance-separation.md).
  No new secret enters the repository; `.config/policies/secrets*.yaml` pin the
  two `secrets.config.yaml` files to their current keys.
- `infra/pigeon/` bootstraps the platform's own Cloudflare account. It is a
  local unit (`x-cobot.local: true`): CI validates it but plans and applies
  only unnamed units, so its owner applies it by hand with a scoped token in
  the environment and state in the private `noisypigeon/pigeon-platform-state`,
  per its README and the separation ADR's phase 2 amendment.
- Backplane (package `pigeon-backplane`) composes the engines into one
  platform serving the instance in `.config/instance.yaml`; it judges through
  sentinel, acts through cobot, and writes and starts nothing itself, per
  [`2026-11-03-development-backplane-platform-composition-layer.md`](.claude/decisions/2026-11-03-development-backplane-platform-composition-layer.md).
- The control plane is backplane's `worker` build on `pigeon.dev`: a Durable
  Object per instance, D1, plan-then-approve, Access, MCP and drift; cobot's
  `worker` build dispatches signed runs to Containers and mints credentials;
  runs survive a stop, per
  [`2026-11-05-runtime-control-plane-worker.md`](.claude/decisions/2026-11-05-runtime-control-plane-worker.md).
- Releases and changelogs are machinery, per
  [`2026-11-02-release-crate-units-and-images.md`](.claude/decisions/2026-11-02-release-crate-units-and-images.md):
  - every `CHANGELOG.md` is generated on merge by the release bot, which runs
    `pigeon cobot release` and `pigeon cobot tags`; a PR never edits one, and
    `pull-request.yaml` fails one that does;
  - `sentinel`, `gantry`, `cobot` and `backplane` are release units in
    `.config/releases.yaml`, each with its own version and tag
    `<unit>/vX.Y.Z` and image `ghcr.io/noisypigeon/pigeon-<unit>`, built
    from the Dockerfile stage named for it; the bot bumps them, never a PR;
  - a unit releases only when its own build changed: its crate and slice of
    `pigeon` (`commands/<unit>.rs`, `cli/<unit>.rs`), shared `pigeon` code,
    its `cargo tree`, or workspace build settings;
  - `rust.yml` runs only when a Rust input changed;
  - `pigeon` compiles in only the features (subcommands) asked for;
  - a policy that needs an unreleased sentinel feature, or a unit or
    workload that needs an unreleased cobot feature, waits: merge and
    release the engine change first, then use it in a later PR, per
    [`2026-11-03-release-cobot-image-in-workflows.md`](.claude/decisions/2026-11-03-release-cobot-image-in-workflows.md);
  - the cobot image's `tofu` and `zola` versions equal `mise.toml`'s, which
    a cobot test checks.
- Infrastructure is cobot's, per
  [`2026-11-02-runtime-cobot-infrastructure-as-code.md`](.claude/decisions/2026-11-02-runtime-cobot-infrastructure-as-code.md):
  units (`unit.yaml`, schema v0) live only under `infra/` and `workloads/`;
  `cobot.yml` plans on PR and applies on merge, committing encrypted state to
  the private `noisypigeon/pigeon-cobot-state`. Never run `tofu apply` or
  `pigeon cobot apply` by hand against that repository.
- OpenTofu owns declarative resources; Workers, Containers and their images,
  migrations, crons and secrets deploy through `pigeon cobot workers` (wrangler,
  only via cobot) from `workers/<name>/worker.yaml` specs that hold no IDs, per
  [`2026-11-05-development-cobot-workers-area.md`](.claude/decisions/2026-11-05-development-cobot-workers-area.md).
- Config units (`config.yaml` validated by `schema.json`, composed from its
  `x-cobot` annotations into single-resource `modules/`) are per
  [`2026-11-02-runtime-cloudflare-provisioner-unit.md`](.claude/decisions/2026-11-02-runtime-cloudflare-provisioner-unit.md).
  A unit's `secrets.config.yaml` holds only `ENC[cobot:v1:…]` values, under
  `COBOT_SECRETS_PASSPHRASE` in `infra/` and `COBOT_WORKLOAD_SECRETS_PASSPHRASE`
  in `workloads/`. Only `infra/` keys are break-glass, and they are never handed
  to a workload, which gets only its own secrets and minted tokens. Providers
  come only from `main`'s `.config/providers.yaml` and `providers.lock.hcl`.
- Any unit composes a catalog's module as `module: <catalog>/<name>`, such as
  `cloudflare/r2-bucket` or the provider-agnostic `agnostic/random-string`, and
  `x-cobot.each` runs a block over another property's items, per
  [`2026-11-03-runtime-infra-agnostic-shared-modules.md`](.claude/decisions/2026-11-03-runtime-infra-agnostic-shared-modules.md)
  and the separation ADR's catalog-composition amendment.
- Workloads live in `workloads/<zone>/`; each zone's apex (global config and
  DNS) is its own unit, `workloads/<zone>/apex/`, which only enables what
  the Cloudflare catalog, `catalog/cloudflare/`, defines (its schema's
  `$defs.apex`, its `modules/` and `presets/`), per
  [`2026-11-02-runtime-workloads-zone-apex.md`](.claude/decisions/2026-11-02-runtime-workloads-zone-apex.md).
- A workload with only a `config.yaml` enables what `catalog/cloudflare/`
  defines (its `$defs.workload`): today `static_site`, an R2 bucket and a
  custom domain per apex host, built with a released `src/static-site-template/`
  (`static-site-template/vX.Y.Z`) and published by `cobot.yml` after apply, per
  [`2026-11-03-runtime-cloudflare-static-sites-r2.md`](.claude/decisions/2026-11-03-runtime-cloudflare-static-sites-r2.md).
- A static site's settings are `static_site.site` in its `config.yaml`, checked
  against the pinned template's `schema.json`; its `site/` holds only
  `posts/` (named `YYYY-MM-DD-<slug>.md`), `pages/` and `assets/`, per
  [`2026-11-03-runtime-static-site-config-contract.md`](.claude/decisions/2026-11-03-runtime-static-site-config-contract.md).
- The repo is public: never push secrets, credentials, key IDs or private
  bucket names, in commits, branch names, PR text or CI logs.

## Commands

`mise run ci` must pass before every PR.

- `mise run build` / `test` / `fmt` / `lint`
- `mise run rust`: fmt-check, clippy `-D warnings`, `features`, tests.
- `mise run features`: clippy with each subcommand feature alone.
- `pigeon cobot validate|render|plan`: check, render and plan units;
  `plan` needs `COBOT_STATE_PASSPHRASE` and a `--state` directory.
- `pigeon cobot secrets set <unit> <key>` (value on stdin) and
  `pigeon cobot secrets check <unit>`, with `COBOT_SECRETS_PASSPHRASE` (or
  `COBOT_WORKLOAD_SECRETS_PASSPHRASE` under `workloads/`); there is no `get`.
- `pigeon cobot lock`: rewrite `.config/providers.lock.hcl` from the allowlist.
- `pigeon cobot sites build [WORKLOAD…]`: build static sites with Zola, no
  secrets; `sites publish --state DIR` also syncs them to R2 and checks them
  live.
- `pigeon cobot release --dry-run` / `pigeon cobot tags`: preview the bot's
  release JSON.
- `pigeon backplane instance|engines|operations|run <op>` (`--json` for the
  wire form); `mise run backplane` validates `.config/instance.yaml`.
- `mise run sentinel`: `pigeon sentinel check` against `.config/policies/`.
- `PR_TITLE='…' PR_BODY='…' PR_NUMBER=N PR_FILES="$(git diff --name-only main...)" mise run sentinel-pr`:
  adds the pull-request policy.
