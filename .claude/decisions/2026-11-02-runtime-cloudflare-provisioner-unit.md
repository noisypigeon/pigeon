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
  .terraform.lock.hcl    provider hashes
  modules/<name>/        one resource each; individually versioned
```

The decisions:

- **Single-resource modules with a unified shape.** runway's modules are the inspiration. Nothing is copied, and every module here starts from scratch.
- **Versioning.** Each module, and the schema, is versioned on its own by the existing release bot. A schema version is tied to the module versions it was built with.
- **Composition is generated.** cobot builds the unit's root module from annotations in `schema.json`. There is no hand-written root HCL.
- **Secrets live in this public repository, encrypted.** `secrets.config.yaml` is encrypted value by value with a key from `COBOT_STATE_PASSPHRASE`, the same secret that guards state.
  - That keeps one secret to manage, at the cost of one blast radius. Anyone holding the passphrase and read access to the state repository holds every minted token anyway.
  - The secrets key is derived differently from the state key, so the two are never the same key.
- **Providers are allowlisted and pinned.** That is the "explicit, pinned" path the cobot amendment deferred. Hashes are verified by a committed lockfile.

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
- `rotation` sets how long a minted token lives, and how long it overlaps its successor.
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
| `rotation` | `time_rotating` | `period_days` | `id`, `rfc3339` |
| `api-token` | `cloudflare_api_token` | `name`, `capabilities`, `zone_ids`, `permission_ids`, `account_id`, `rotation_id`, `rotation_rfc3339`, `period_days`, `overlap_days` | `id`, `name`, and a sensitive `value` |

`api-token` in more detail:

- It builds its policies, in `locals`, from `capabilities` and the id maps.
- Its `expires_on` is the rotation time plus `period_days` plus `overlap_days`.
- A `terraform_data` keyed on `rotation_id` replaces the token whenever the rotation fires, so each rotation produces a new value, not just a new expiry.

Minted values live only in encrypted state.

### 4. Secrets

**File format.** `secrets.config.yaml` is YAML with keys in clear, so a review shows which secret changed:

```yaml
account_id: ENC[cobot:v1:...]
provisioner_token: ENC[cobot:v1:...]
```

- Each value is the base64 of an age file encrypted to an scrypt recipient, with `COBOT_STATE_PASSPHRASE` as the passphrase.
- scrypt with its own random salt is a different derivation from OpenTofu's `pbkdf2`, so the secrets key and the state key are never the same key.

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
  hashicorp/time: <newest at least two weeks old>
```

**Checks.** `validate` rejects any provider that a module's `required_providers` declares when it is missing from the allowlist, or pinned there to a different version.

**Lockfile.**

- `pigeon cobot lock <unit>` writes the unit's `.terraform.lock.hcl`. It runs `tofu providers lock` for `linux_amd64`, `linux_arm64` and `darwin_arm64`.
- The lockfile is committed. `terraform.yaml` already allows it under `infra/`.

**`init`.** cobot copies the lockfile into the work directory and runs `tofu init -input=false -lockfile=readonly`.

- Providers install from the registry, and their hashes must match the lockfile.
- `-plugin-dir` is no longer used.

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

### 7. Docs

`CLAUDE.md` gains:

- config units;
- the secrets commands;
- the rule that a break-glass key is never handed to a workload, which gets only minted tokens.

### 8. Execution, release-first

1. **PR A** holds this ADR.
2. **PR B**, `feat(cobot): config units, secrets and pinned providers`, holds the engine of §1, §2, §4, §5 and §6.
   - Its tests use a fixture config unit built only from built-in resources.
   - It holds no `infra/cloudflare` yet.
   - Its merge releases `cobot` and `sentinel`.
3. **PR C**, `feat(infra): cloudflare provisioner unit`, holds:
   - the modules, `schema.json`, `config.yaml`, `.config/providers.yaml`, the lockfile, the release units, the policy and the docs.
   - `secrets.config.yaml`, which the owner writes locally by running `pigeon cobot secrets set` for `account_id` and `provisioner_token`, and commits to the PR branch. The values are never seen by anyone else.
   - Its plan should show four zone lookups, the permission-group lookup and one `time_rotating`, and no tokens.

### 9. Verification

- **Unit tests:**
  - schema validation passes and fails as expected;
  - `x-cobot` renders to the expected `main.tf.json`;
  - each bad expression is rejected for its own reason;
  - secrets round-trip, a wrong passphrase fails, and a plaintext value is rejected;
  - an unlisted or mispinned provider is rejected;
  - YAML and JSON version lines bump.
- **End to end:** a built-in-only config unit plans, applies and replans to "No changes", with state and plan encrypted.
- **PR C:** its plan comment shows `data.cloudflare_zone` ×4, `data.cloudflare_api_token_permission_groups_list` and `time_rotating`. After merge, the apply commits encrypted state, and the bot tags every module and the schema.
