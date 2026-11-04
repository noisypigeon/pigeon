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
alias pigeon='mise exec -- cargo run -q --locked --manifest-path src/pigeon-cli/Cargo.toml -p pigeon --'
export CLOUDFLARE_API_TOKEN=…            # the bootstrap token
export COBOT_STATE_PASSPHRASE=…          # this unit's passphrase, not the instance's
pigeon cobot plan infra/pigeon --root . --state ../pigeon-platform-state
pigeon cobot apply infra/pigeon --root . --state ../pigeon-platform-state
cd ../pigeon-platform-state && git add -A && git commit -m "cobot: apply infra/pigeon" && git push
unset CLOUDFLARE_API_TOKEN COBOT_STATE_PASSPHRASE
```

The unit must be named: `cobot plan` and `cobot apply` with no unit skip it.
The `account` module fails unless the token sees exactly one account.
