# infra/pigeon

The pigeon Cloudflare account's bootstrap: the platform's D1 database and its
state and plans R2 buckets, composed from `catalog/cloudflare/` modules. It is a
local unit, so CI validates it but never plans or applies it; its owner applies
it by hand, as break-glass, per
[`2026-11-04-development-platform-instance-separation.md`](../../.claude/decisions/2026-11-04-development-platform-instance-separation.md)
(amendments "Phase 2, the pigeon-account bootstrap" and "The bootstrap
composes the Cloudflare catalog").

## Once

1. Create the private repository `noisypigeon/pigeon-platform-state` and clone it
   somewhere outside this checkout.
2. In the **pigeon** Cloudflare account, create an account API token with only:
   Account Settings Read, D1 Edit, Workers R2 Storage Edit.
3. Pick a state passphrase of at least 16 characters, used only for this unit.

Neither the token nor the passphrase is ever written to a file in this
repository, a commit, a PR or a log.

## Each apply

From the root of a `main` checkout:

```sh
alias pigeon='cargo run -q --locked --manifest-path Cargo.toml -p pigeon-harness --'
export CLOUDFLARE_API_TOKEN=…            # the bootstrap token
export PIGEON_STATE_PASSPHRASE=…          # this unit's passphrase, not the instance's
pigeon infra plan infra/pigeon --root . --state ../pigeon-platform-state
pigeon infra apply infra/pigeon --root . --state ../pigeon-platform-state
cd ../pigeon-platform-state && git add -A && git commit -m "infra: apply infra/pigeon" && git push
unset CLOUDFLARE_API_TOKEN PIGEON_STATE_PASSPHRASE
```

The first run after a pull compiles `pigeon` quietly, and each plan's `tofu init`
downloads the Cloudflare provider; both can take minutes. `pigeon infra` names each
`tofu` step on stderr as it runs. To download the provider once, set
`TF_PLUGIN_CACHE_DIR` (for example `~/.terraform.d/plugin-cache`, created first).

The unit must be named: `pigeon infra plan` and `pigeon infra apply` with no unit skip it.
The `account` module fails unless the token sees exactly one account.
Every plan shows `update module.d1.cloudflare_d1_database.this (file_size,
num_tables, version)`: provider 5.28.0 marks those computed attributes unknown on
each plan. Applying it changes nothing.

## The environment's names

The names `pigeon` reads its state, secrets, tools and TLS from, without values; set only those a command needs.

```sh
export PIGEON_STATE_PASSPHRASE=
export PIGEON_SECRETS_PASSPHRASE=
export PIGEON_WORKLOAD_SECRETS_PASSPHRASE=
export PIGEON_STATE_R2_ENDPOINT=
export PIGEON_STATE_R2_ACCESS_KEY_ID=
export PIGEON_STATE_R2_SECRET_ACCESS_KEY=
export PIGEON_CREDENTIALS=
export PIGEON_DOCKER=
export PIGEON_WRANGLER=
export AGENT_TLS_CERT=
export AGENT_TLS_KEY=
export AGENT_TLS_CLIENT_CA=
```
