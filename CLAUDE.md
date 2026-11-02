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
- Releases and changelogs are machinery, per
  [`2026-11-02-release-crate-units-and-images.md`](.claude/decisions/2026-11-02-release-crate-units-and-images.md):
  - every `CHANGELOG.md` is generated on merge by the release bot, which runs
    `pigeon cobot release` and `pigeon cobot tags`; a PR never edits one, and
    `pull-request.yaml` fails one that does;
  - `sentinel`, `gantry` and `cobot` are release units in
    `.config/releases.yaml`, each with its own version and tag
    `<unit>/vX.Y.Z` (the first two also an image
    `ghcr.io/noisypigeon/pigeon-<unit>`); the bot bumps them, never a PR;
  - a unit releases only when its own build changed: its crate and slice of
    `pigeon` (`commands/<unit>.rs`, `cli/<unit>.rs`), shared `pigeon` code,
    its `cargo tree`, or workspace build settings;
  - `rust.yml` runs only when a Rust input changed;
  - `pigeon` compiles in only the features (subcommands) asked for;
  - a policy that needs an unreleased sentinel feature waits: merge and
    release the engine change first, then use it in a later PR.
- Infrastructure is cobot's, per
  [`2026-11-02-runtime-cobot-infrastructure-as-code.md`](.claude/decisions/2026-11-02-runtime-cobot-infrastructure-as-code.md):
  units (`unit.yaml`, schema v0) live only under `infra/` and `workloads/`;
  `cobot.yml` plans on PR and applies on merge, committing encrypted state to
  the private `noisypigeon/pigeon-cobot-state`. Never run `tofu apply` or
  `pigeon cobot apply` by hand against that repository.
- Config units (`config.yaml` validated by `schema.json`, composed from its
  `x-cobot` annotations into single-resource `modules/`) are per
  [`2026-11-02-runtime-cloudflare-provisioner-unit.md`](.claude/decisions/2026-11-02-runtime-cloudflare-provisioner-unit.md).
  A unit's `secrets.config.yaml` holds only `ENC[cobot:v1:…]` values under
  `COBOT_SECRETS_PASSPHRASE`; only `infra/` units have one, and a break-glass
  key is never handed to a workload, which gets only minted tokens. Providers
  come only from `main`'s `.config/providers.yaml` and `providers.lock.hcl`.
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
  `pigeon cobot secrets check <unit>`, with `COBOT_SECRETS_PASSPHRASE`; there is
  no `get`.
- `pigeon cobot lock`: rewrite `.config/providers.lock.hcl` from the allowlist.
- `pigeon cobot release --dry-run` / `pigeon cobot tags`: preview the bot's
  release JSON.
- `mise run sentinel`: `pigeon sentinel check` against `.config/policies/`.
- `PR_TITLE='…' PR_BODY='…' PR_NUMBER=N PR_FILES="$(git diff --name-only main...)" mise run sentinel-pr`:
  adds the pull-request policy.
