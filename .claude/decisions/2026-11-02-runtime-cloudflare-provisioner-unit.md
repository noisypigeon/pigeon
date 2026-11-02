## infra/cloudflare: the provisioner unit

* Date: 2026-11-02
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: runtime

## Context

Part one of cobot, `.claude/decisions/2026-11-02-runtime-cobot-infrastructure-as-code.md` and its amendment, proved the pipeline with a unit that provisions nothing. Two of its v0 limits stand between it and real infrastructure:

- a unit is a hand-written `unit.yaml`;
- only OpenTofu's built-in provider can load.

The first real unit is `infra/cloudflare`, and it is the first child of `infra/`. It is break-glass by design. It holds one provisioner key, and from it mints scoped, rotating API tokens for workloads. Workloads never hold, or even see, the provisioner key.

It establishes the shape every later `infra/` child follows:

```
infra/cloudflare/
  config.yaml            non-secret options, validated by schema.json
  schema.json            JSON Schema plus x-cobot composition; versioned
  secrets.config.yaml    keys in clear, every value encrypted
  modules/<name>/        one resource each; individually versioned
```

The decisions:

- **Single-resource modules with a unified shape.** runway's modules are the inspiration. Nothing is copied, and every module here starts from scratch.
- **Versioning.** Each module, and the schema, is versioned on its own by the existing release bot. A schema version is tied to the module versions it was built with.
- **Composition is generated.** cobot builds the unit's root module from annotations in `schema.json`. There is no hand-written root HCL.
- **Secrets live in this public repository, encrypted under a passphrase of their own.**
  - `secrets.config.yaml` is encrypted value by value with `COBOT_SECRETS_PASSPHRASE`, a second secret, separate from `COBOT_STATE_PASSPHRASE`.
  - The ciphertext is public, so whoever holds the secrets passphrase holds the break-glass key. That is why it is not the state passphrase: leaking the state passphrase alone exposes no break-glass material, and leaking the secrets passphrase alone exposes no state.
  - Minted token values live in state, so they stay behind the state passphrase plus access to the state repository, as before.
  - Both passphrases live only as Actions secrets and in the owner's password manager. Rotating `provisioner_token` is the response to any suspected leak of the secrets passphrase.
- **Providers are allowlisted, pinned and taken only from `main`.** That is the "explicit, pinned" path the cobot amendment deferred. Hashes are verified by one committed lockfile. A pull request cannot choose which provider code runs beside the passphrases.
- **Rotation keeps two generations.** A rotated token's predecessor lives until its overlap ends, so workloads have time to switch.

This supersedes two parts of the cobot ADR, both left as written:

- §1's `unit.yaml` as the only kind of unit;
- the amendment's empty `-plugin-dir`.

Out of scope, each for a later amendment:

- handing minted tokens to workloads;
- creating zones rather than looking them up;
- DNS records and other Cloudflare resources;
- rotating `COBOT_STATE_PASSPHRASE` itself;
- a `pigeon cobot` image.

## Implementation plan.

### 1. Config units

A unit is a directory under `infra/` or `workloads/` that holds either:

- a `unit.yaml` (schema v0, unchanged); or
- a `config.yaml` and a `schema.json`, which this ADR adds.

How the files are checked:

- `config.yaml` is validated against `schema.json` (JSON Schema 2020-12) with the `jsonschema` crate.
- Both files are non-secret, and cobot rejects any value in either that looks like a credential.
- Account identifiers belong in secrets.

`infra/cloudflare/config.yaml` starts with:

```yaml
zones: [noisypigeon.com, pigeon.dev, willowgraysen.com, noisypigeon.garden]
rotation: { period_days: 90, overlap_days: 7 }
tokens: []
```

- `zones` lists existing zones this account may use. They are looked up, never created.
- `rotation` sets how long each token generation lasts, and how long a generation overlaps its successor.
- `tokens` lists the tokens to mint. Each is `{ name, capabilities: [{ permission, zones | account: true }] }`, where `permission` is a Cloudflare permission group name. The list starts empty, and each workload ADR adds its own entries.

### 2. Generated composition: `x-cobot`

`schema.json` is a JSON Schema whose annotations say how config becomes modules.

- **Top-level `x-cobot`:**
  - `version`: the schema's own version, `X.Y.Z`;
  - `providers`: each allowlisted provider's configuration, as expressions, e.g. `{ "cloudflare": { "api_token": "$secret.provisioner_token" } }`;
  - `secrets`: the keys `secrets.config.yaml` must hold.
- **On each top-level property, an `x-cobot`:**
  - `module`: a directory name under `modules/`;
  - `for_each`, for a list: the item field used as the instance key;
  - `inputs`: a map from the module's variable names to expressions.

The expressions:

| Expression | Meaning |
| --- | --- |
| `$item.<field>` | a field of the current list item; for a list of strings, `$item` itself |
| `$config.<path>` | a value in `config.yaml` |
| `$secret.<key>` | a value from the decrypted secrets |
| `$module.<m>.<output>` | an output of a single-instance module |
| `$module.<m>[$item.<field>].<output>` | one instance's output |
| `$module.<m>[*].<output>` | a map from instance key to output, across every instance |

**What cobot renders.** `.cobot/<unit>/main.tf.json` holds:

- `terraform.required_providers`, taken from the allowlist;
- one provider block per `x-cobot.providers` entry;
- `variable "secrets" { type = map(string), sensitive = true }`;
- `variable "config"`, holding the parsed config;
- one `module` block per annotated property, with `source` set to the module's absolute path, `for_each` over the keyed list, and each input rendered as an interpolation (`${each.value.name}`, `${var.secrets["provisioner_token"]}`, `${module.zone[each.value.zone].id}`, and so on).

**How values reach `tofu`.**

- Secrets go in through `TF_VAR_secrets` in the `tofu` process's environment, and are never written to disk.
- Config goes in through `TF_VAR_config`.
- Plans and state stay encrypted and enforced, as cobot part one decided.

**Rejected at validate time:**

- an unknown expression form;
- a reference to a module missing from `modules/`;
- a reference to an output the module doesn't declare;
- a `$secret` key missing from `x-cobot.secrets`;
- a module input the module doesn't declare as a variable.

### 3. Single-resource modules

- **One resource.** Each module wraps exactly one provider resource or data source. It may add one built-in `terraform_data`, used only as a `replace_triggered_by` trigger.
- **A unified shape:**
  - `module.yaml` holds `version: X.Y.Z` and a one-line `description`;
  - `main.tf` holds the resource and its `required_providers`, pinned exactly as in the allowlist;
  - `variables.tf` and `outputs.tf`: every module outputs `id`, and `name` where the resource has one, plus resource-specific outputs. Every sensitive output is marked `sensitive`.

The first four modules:

| Module | Resource | Inputs | Outputs |
| --- | --- | --- | --- |
| `zone` | `data.cloudflare_zone` | `name`, `account_id` | `id`, `name` |
| `permission-groups` | `data.cloudflare_api_token_permission_groups_list` | none | `id`, plus `ids`, a map from name to id |
| `api-token` | `cloudflare_api_token` | `name`, `capabilities`, `zone_ids`, `permission_ids`, `account_id`, `not_before`, `expires_on` | `id`, `name`, and a sensitive `value` |

`api-token` builds its policies, in `locals`, from `capabilities` and the id maps.

**Rotation in generations.** Each token is minted once per generation, as a separate instance, rather than replaced in place. Replacing in place would revoke the old token in the same apply that creates the new one.

- **Keys describe their own validity.** An instance's key is `<name>@<not_before>..<expires_on>`, in UTC dates, e.g. `blog-deploy@2026-11-02..2027-02-07`. Its `not_before` and `expires_on` are taken from the key.
- **What cobot renders, per configured token:**
  - **Current.** One instance for the current generation, which starts on day `floor(days since 1970-01-01 UTC / period_days) × period_days`.
    - If state already holds a live instance of this name with that `not_before`, whose `expires_on` is no earlier than the next generation's start (`not_before + period_days`), that instance is the current one. Its key is kept unchanged.
    - Otherwise cobot renders a new key, expiring `period_days + overlap_days` after the start. A same-start instance that would lapse too early is retained beside it until its own expiry.
    - So a generation is minted once unless a longer period demands a longer-lived token. Coverage never has a gap.
  - **Retained.** Every instance already in the unit's state with the same `<name>` whose `expires_on` is still in the future. Before rendering, cobot reads them by running `tofu show -json` on the current state.
  - **Dropped.** Instances whose `expires_on` has passed. They are destroyed on the next apply.
- **A token removed from `config.yaml`** renders no instances at all, so its live instances are destroyed, which revokes them deliberately.
- **Changing `period_days` or `overlap_days`** shapes only new generations. Live instances keep their own dates, retained from state, and expire as minted. A configuration change never destroys a valid token early, and never mints a second token for a generation that already has one.
- **Cloudflare enforces expiry.** Each token carries its own `expires_on`, so Cloudflare revokes it on time even if no apply runs.
- **Failed applies are safe.** Keys come from dates and existing state, never from a recomputed index. An instance that failed to create is planned again, and an expired one is destroyed by the next successful apply.
- **Scheduled apply.** Rotation needs applies even when nothing merges, so `cobot.yml`'s `apply` job also runs on a daily `schedule`, at 04:17 UTC. A day with no generation boundary plans "No changes".

Minted values live only in encrypted state.

### 4. Secrets

**File format.** `secrets.config.yaml` is YAML with keys in clear, so a review shows which secret changed:

```yaml
account_id: ENC[cobot:v1:...]
provisioner_token: ENC[cobot:v1:...]
```

- Each value is the base64 of an age file encrypted to an scrypt recipient, with `COBOT_SECRETS_PASSPHRASE` as the passphrase.
- `secrets set` and `secrets check`, and any `plan` or `apply` of a unit with secrets, refuse (exit `2`) unless `COBOT_SECRETS_PASSPHRASE` is set, with the same 16-character minimum as the state passphrase.
- In `cobot.yml`, `COBOT_SECRETS_PASSPHRASE` is set only on the `plan` and `apply` steps, beside `COBOT_STATE_PASSPHRASE`.

**Commands:**

- **`pigeon cobot secrets set <unit> <key>`** reads the value from stdin, without echo, then encrypts it and writes the file. Only a human runs it.
- **`pigeon cobot secrets check <unit>`** lists the keys and verifies that each one decrypts. It never prints a value.
- There is no `get`. A decrypted value only ever exists in the `tofu` process's environment.

**`infra/cloudflare`'s keys:**

- `account_id`.
- `provisioner_token`, the break-glass key.
  - It is created by hand in the Cloudflare dashboard, as an account token with Account API Tokens Write, Zone Read, Account Settings Read, and every permission any minted token will be granted.
  - Rotating it is a manual, documented act.

**Guards:**

- `pigeon cobot validate` rejects:
  - any secrets value that is not `ENC[cobot:v1:…]`;
  - a secrets file whose keys differ from `x-cobot.secrets`;
  - any `secrets.config.yaml` or `$secret` expression under `workloads/`.
- `.config/policies/terraform.yaml` gains a `find` rule that allows `secrets.config.yaml` only under `infra/`.

So a workload can never hold, or reference, break-glass material. It gets only the tokens minted for it.

### 5. Providers

**Allowlist.** `.config/providers.yaml` lists every provider cobot may load, each pinned to an exact version:

```yaml
description: Providers cobot may load, each pinned to an exact version.
providers:
  cloudflare/cloudflare: <newest 5.x at least two weeks old>
```

**Lockfile.** `.config/providers.lock.hcl` is one repository-wide lockfile.

- `pigeon cobot lock` writes it by running `tofu providers lock` for `linux_amd64`, `linux_arm64` and `darwin_arm64`, against a scratch configuration requiring exactly the allowlist.
- It is committed.

**Checks.** `validate` rejects:

- any provider a module's `required_providers` declares that is missing from the allowlist, or pinned there to a different version;
- a lockfile that disagrees with the allowlist.

**`init`.** cobot copies the lockfile into the work directory and runs `tofu init -input=false -lockfile=readonly`.

- Providers install from the registry, and their hashes must match the lockfile.
- `-plugin-dir` is no longer used.

**Providers come only from `main`.**

- The `plan` job passes `--providers` pointing at `main`'s `.config/`, so a plan uses `main`'s allowlist and lockfile, never the PR's.
- If the PR changes either file, `plan` stops before `init` and posts "Provider changes are planned after they merge; land them in their own PR first."
- A PR therefore cannot pick the provider code that runs beside `COBOT_STATE_PASSPHRASE`.
- Adding a provider is its own PR. It changes only the allowlist and the lockfile, it merges, and only then can units use it.

**Unchanged from part one:**

- plans build `pigeon` from `main`;
- plans read the PR only as data;
- plans are skipped for forks;
- summaries print only addresses and actions.

### 6. Versions

**Version lines.** `pigeon sentinel release` reads and bumps any of three version-line forms:

- TOML: `version = "X.Y.Z"`;
- YAML: `version: X.Y.Z`;
- JSON: `"version": "X.Y.Z"`.

The first match is used.

**Modules.** Each module is a release unit in `.config/releases.yaml`:

- named `infra-cloudflare-<module>`;
- its manifest is `modules/<module>/module.yaml`;
- its paths are its own directory;
- it has no image;
- it is tagged `infra-cloudflare-<module>/vX.Y.Z`.

**The schema** is a release unit too:

- named `infra-cloudflare-schema`;
- its manifest is `schema.json`;
- its paths are `schema.json` and `modules/`.

So the schema bumps whenever any module does. The schema's tag pins the exact module versions that sit beside it, and its changelog lists the module changes that caused each version.

**Changelogs.** Each module and the schema get a release changelog tier, beside their files, and `release-changelog.yaml` covers them.

**Publishing.**

- Each row from `pigeon sentinel tags` gains a `changelog` field: the path of its unit's release tier, taken from `.config/releases.yaml`.
- The release workflow reads notes from that path, never from a fixed `src/pigeon-cli/crates/<unit>/` path. Publishing an infra unit then works exactly as publishing a crate does.

### 7. Docs

`CLAUDE.md` gains:

- config units;
- the secrets commands;
- the rule that a break-glass key is never handed to a workload, which gets only minted tokens.

### 8. Execution, release-first

1. **PR A** holds this ADR.
2. **PR B**, `feat(cobot): config units, secrets and pinned providers`, holds the engine of §1, §2, §4, §5 and §6.
   - That includes date-keyed generations retained from state, the scheduled apply, `COBOT_SECRETS_PASSPHRASE`, `--providers`, and the `changelog` field with the release workflow's use of it.
   - Its tests use a fixture config unit built only from built-in resources.
   - It holds no `infra/cloudflare` and no provider yet.
   - Its merge releases `cobot` and `sentinel`.
3. **PR C**, `build(cobot): allow the cloudflare provider`, holds only `.config/providers.yaml` and `.config/providers.lock.hcl`.
4. **By hand, before PR D:** the owner generates `COBOT_SECRETS_PASSPHRASE` (for example with `openssl rand -base64 32`), adds it as an Actions secret, and keeps a copy in a password manager.
5. **PR D**, `feat(infra): cloudflare provisioner unit`, holds:
   - the modules, `schema.json`, `config.yaml`, the release units, the policy and the docs.
   - `secrets.config.yaml`, which the owner writes locally by running `pigeon cobot secrets set` for `account_id` and `provisioner_token`, and commits to the PR branch. The values are never seen by anyone else.
   - Its plan uses `main`'s providers, and should show four zone lookups and the permission-group lookup, and no tokens.

### 9. Verification

- **Unit tests:**
  - schema validation passes and fails as expected;
  - `x-cobot` renders to the expected `main.tf.json`;
  - each bad expression is rejected for its own reason;
  - secrets round-trip, a wrong passphrase fails, and a plaintext value is rejected;
  - an unlisted or mispinned provider is rejected;
  - YAML and JSON version lines bump.
- **End to end:** a built-in-only config unit plans, applies and replans to "No changes", with state and plan encrypted.
- **Rotation:** render tests with a fixed clock and a fixture state check that:
  - a fresh unit renders only the current instance;
  - inside the overlap, the previous instance is retained beside the current one;
  - after the overlap, only the current instance remains;
  - changing `period_days` keeps every live instance and adds the new current one;
  - changing `overlap_days` mid-generation keeps the current instance's key, and adds nothing;
  - increasing `period_days` without moving the generation start mints a longer-lived instance only when the existing one would lapse before the next generation;
  - removing a token from config drops all of its instances;
  - every key's dates match its `not_before` and `expires_on`.
- **Providers:** a test checks that `plan --providers` ignores a PR-local allowlist, and refuses when the PR changes it.
- **PR D:** its plan comment shows `data.cloudflare_zone` ×4 and `data.cloudflare_api_token_permission_groups_list`. After merge, the apply commits encrypted state, and the bot tags and publishes every module and the schema, with notes read from each unit's own changelog.

## Amendments

### 2026-11-02: Account tokens, and permission groups by scope

Before writing PR D, §3's modules were checked against the provider's schema at 5.28.0 (`tofu providers schema -json`). Two of them name the wrong Cloudflare API:

- **`cloudflare_api_token` is a user token.** It has no `account_id`, and it is created through `/user/tokens`, which the break-glass key cannot reach: §4 makes it an account token, with Account API Tokens Write. Tokens an account token can mint are `cloudflare_account_token`.
- **`cloudflare_api_token_permission_groups_list` is the user-scoped list.** The account-scoped one is `cloudflare_account_api_token_permission_groups_list`, which takes `account_id`.
- **Permission group names are not unique.** One name can appear once per scope, so §3's `ids`, a map from name to id, could collide.

§1, §2, §4 and the break-glass permissions are unchanged.

#### 1. `zone`

- Wraps `data.cloudflare_zone` with `filter = { name, match = "all", account = { id = account_id } }`.
- Inputs and outputs are unchanged: `name` and `account_id` in; `id` and `name` out.

#### 2. `permission-groups`

- Wraps `data.cloudflare_account_api_token_permission_groups_list`, with one input, `account_id`.
- Outputs:
  - `groups`, a map from each group name to the list of `{ id, scopes }` carrying that name. This replaces `ids`.
  - `id`, a digest of the sorted group ids, since the list has no id of its own.

#### 3. `api-token`

- Wraps `cloudflare_account_token`.
- Inputs: `name`, `capabilities`, `zone_ids` (a map from zone name to id), `groups` (the output above), `account_id`, `not_before` and `expires_on`.
- Outputs: `id`, `name` and a sensitive `value`.
- One `allow` policy per capability, built in `locals`:
  - its permission group is the one with that name whose `scopes` contain `com.cloudflare.api.account.zone` for a `zones` capability, or `com.cloudflare.api.account` for an `account: true` one;
  - its `resources` are `com.cloudflare.api.account.zone.<id>` for each listed zone, or `com.cloudflare.api.account.<account_id>`, each mapped to `*` and JSON-encoded, as the provider takes them;
  - a name with no group in that scope, or more than one, fails the plan through a `precondition`, naming the capability.

#### 4. Schema wiring

- `permission_groups` is a singleton block over `permission-groups`, fed `$secret.account_id`.
- `zones` feeds `zone` with `name: $item` and `account_id: $secret.account_id`.
- `tokens` feeds `api-token` with `zone_ids: $module.zones[*].id`, `groups: $module.permission_groups.groups`, `account_id: $secret.account_id`, and its generation's dates.

#### 5. Execution

1. This amendment lands first.
2. PR D then follows §8, with these modules. It also deletes `.config/changelog-corrections.yaml` and the code that reads it, as the release ADR's amendment of the same date directs.
3. Its plan comment shows `data.cloudflare_zone` ×4 and `data.cloudflare_account_api_token_permission_groups_list`, and no tokens.
