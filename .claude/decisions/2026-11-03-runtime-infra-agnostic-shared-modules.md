## infra/agnostic: shared modules and random resource names

* Date: 2026-11-03
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: runtime

## Context

`.claude/decisions/2026-11-03-runtime-digitalocean-provisioner-unit.md` names each Spaces bucket `<bucket_prefix>-<name>`, where `bucket_prefix` is a hand-made secret. That makes randomness a manual step, and a secret that can never change.

runway randomizes names at the resource level. Its `tools/terraform/scaleway/bucket` module creates:

- a `random_string`, 6 characters of lowercase letters and digits, from `hashicorp/random`;
- the bucket, named `{prefix}-{random}-{suffix}`.

The random part is created once, kept in state, and never needs a person. Its other buckets, volumes and instances do the same.

pigeon adopts the pattern on purpose, and makes it reusable. A module here wraps exactly one resource, so `random_string` cannot sit beside the bucket. It becomes a module of its own, which any unit composes. Later work will use it, and other modules like it, so this ADR also decides where provider-agnostic modules live and how a unit reaches them.

That takes up an item from the out-of-scope list of `.claude/decisions/2026-11-02-runtime-cobot-infrastructure-as-code.md`: "module sources beyond a unit's own directory".

**Where they live.** `infra/bootstrap` proved the pipeline, as that ADR's §1 intended, and has served its purpose.

- It becomes `infra/agnostic`, a config unit laid out exactly like `infra/cloudflare` and `infra/digitalocean`.
- It holds the modules that need no provider credentials.
- Its state moves, and its proof resource is destroyed.

This supersedes, and leaves as written:

- the cobot ADR's §1 proof unit, `infra/bootstrap`;
- the DigitalOcean ADR's `bucket_prefix` secret and its `<bucket_prefix>-<name>` names (§2, §3 and §4).

Out of scope, each for a later amendment:

- modules shared from anywhere but `infra/agnostic`;
- an `exact_name` override, as runway has, for names that must match something external;
- randomizing names anywhere but DigitalOcean buckets.

## Implementation plan.

### 1. `infra/agnostic`

```
infra/agnostic/
  config.yaml            non-secret options, validated by schema.json
  schema.json            JSON Schema plus x-cobot composition; versioned
  modules/<name>/        one resource each; individually versioned
```

- **`config.yaml`** is `{}`, with its header comment. The unit provisions nothing of its own, and its `schema.json` allows no properties.
- **`schema.json`**'s `x-cobot` holds `version`, `secrets: []` and `providers: {}`. There is no `secrets.config.yaml`, which `validate` already requires only when `x-cobot.secrets` names a key. One is added, as in the other units, once a module here needs a secret.
- **Modules** follow the unified shape of the Cloudflare ADR's §3:
  - one resource;
  - `module.yaml`, `main.tf`, `variables.tf` and `outputs.tf`;
  - `id` among the outputs;
  - `required_providers` pinned as in the allowlist.
- **The first module is `random-string`:**

  | Module | Resource | Inputs | Outputs |
  | --- | --- | --- | --- |
  | `random-string` | `random_string`, with `lower = true`, `numeric = true`, `upper = false` and `special = false` | `length` (default `6`), `keepers` (a map, default `{}`) | `id`, `result` |

  - Changing `keepers` replaces the string.
  - Its result is created once, and lives only in the encrypted state of the unit that composes it.

### 2. Moving `infra/bootstrap`

- `infra/bootstrap/` is deleted.
- `.config/moves.yaml` gains `{ from: infra/bootstrap, to: infra/agnostic }`, under the cobot ADR's amendment "Moving a unit's state".
- The first plan notes "state moves from `infra/bootstrap`", and destroys `module.proof.terraform_data.this`, which `infra/agnostic` no longer composes. The apply moves the state and deletes the old file.
- A later PR removes the spent entry.

### 3. Engine: agnostic module references

- An `x-cobot` block's `module` may be `agnostic/<name>`, which resolves to `infra/agnostic/modules/<name>/`. Any other value is a module under the unit's own `modules/`, as before.
- The reference is checked like a local one:
  - the module must exist;
  - its inputs must be variables it declares;
  - referenced outputs must be outputs it declares.
- `render` sets the block's `source` to the agnostic module's absolute path.
- `infra/agnostic`'s own blocks name their modules locally.
- **Providers.** `providers::used` also collects `required_providers` from every agnostic module a unit references. So the unit's lockfile subset holds `hashicorp/random`, and `tofu init -lockfile=readonly` succeeds.
  - A provider that needs no credentials, such as `random`, needs no `x-cobot.providers` entry.
- **Scanning.** Agnostic modules are already scanned, like any other module, because they sit inside a unit. That covers provisioners, runner-reading functions and provider pins.

### 4. Engine: blocks over another property's items

Each property's `x-cobot` drives exactly one block. A random suffix per bucket needs a second block over the same items.

- **`x-cobot.each`** is a new top-level map, `{ <block>: { module, over, for_each, inputs } }`:
  - `over` names an array property;
  - the block renders one instance per item of that property, keyed by `for_each`, with `$item` referring to the item;
  - otherwise it works like a property block.
- **`validate` rejects:**
  - an `over` that is not an array property;
  - a missing `for_each`;
  - `rotate` on an `each` block;
  - a block name that a property or singleton already uses.

### 5. DigitalOcean buckets

- **Secrets.** `infra/digitalocean`'s secrets drop `bucket_prefix`, leaving three keys: `provisioner_token`, `spaces_access_id` and `spaces_secret_key`.
- **`schema.json`:**
  - `x-cobot.each.bucket_suffixes` is `{ module: "agnostic/random-string", over: "buckets", for_each: "$item.name", inputs: {} }`;
  - `buckets` feeds `spaces-bucket` with `suffix: $module.bucket_suffixes[$item.name].result`, in place of `prefix`;
  - a bucket's `name` gains `maxLength: 56`, so `<name>-<6 characters>` stays within the 63-character limit.
- **`spaces-bucket`** takes `suffix`, and names its bucket `<name>-<suffix>`, e.g. `backups-q82q17`.
  - Its `name`, `id` and `urn` stay sensitive, so the random part lives only in encrypted state.
  - Plan summaries print addresses, which carry only `<name>`.

### 6. Provider and versions

- **Provider.** `.config/providers.yaml` gains `hashicorp/random: <newest 3.x at least two weeks old>`, and `pigeon cobot lock` regenerates the lockfile. Both land in their own PR.
- **Release units.** `.config/releases.yaml` gains:
  - `infra-agnostic-random-string`, whose manifest is `infra/agnostic/modules/random-string/module.yaml` and whose paths are its own directory;
  - `infra-agnostic-schema`, whose manifest is `infra/agnostic/schema.json` and whose paths are `schema.json` and `modules/`.
- **Pinning.** A schema's release paths also list every agnostic module it references. So `infra-digitalocean-schema` lists `infra/agnostic/modules/random-string/`, and its version pins the module it was built with.
- **Changelogs.** Each new unit gets a changelog tier beside its files, and `.config/policies/release-changelog.yaml` lists them.

### 7. Docs

- `infra/README.md` and `CLAUDE.md` name `infra/agnostic`, and point to this ADR.
- The DigitalOcean pointer in `CLAUDE.md` gives bucket names as `<name>-<random6>`.

### 8. Execution, release-first

1. **PR A** holds this ADR, and a one-line amendment to the DigitalOcean ADR that points to it.
2. **PR B**, `build(cobot): allow the random provider`, holds only `.config/providers.yaml` and `.config/providers.lock.hcl`.
3. **PR C**, `feat(cobot): agnostic modules and each blocks`, holds §3 and §4 with their tests.
   - Its fixtures use only built-in resources.
   - Its merge releases `cobot`, and the next PRs wait for that image.
4. **PR D**, `feat(infra): infra/agnostic and its random-string module`, holds §1, §2, and §6's agnostic release units, plus the docs.
5. **PR E**, `feat(infra): digitalocean provisioner unit`, is the DigitalOcean ADR's PR C with §5 applied.
   - It holds its three secrets, which the owner commits.
   - Its plan shows only `digitalocean_project` to create.

### 9. Verification

- **Unit tests:**
  - an `agnostic/` reference renders the agnostic source, and a missing one is rejected;
  - an `each` block renders `for_each` over another property's items;
  - each new `validate` rejection fails for its own reason;
  - `providers::used` includes an agnostic module's providers.
- **End to end:** a fixture unit composing an agnostic module of built-in resources plans, applies and replans to "No changes".
- **PR D's plan** shows the state move and one `terraform_data` to destroy. After merge:
  - `pigeon-cobot-state` holds `infra/agnostic/terraform.tfstate`, and no `infra/bootstrap`;
  - the bot tags `infra-agnostic-*/v0.1.0`.
- **Locally, for PR E**, with placeholder secrets, rendering and planning a sample bucket shows:
  - `module.bucket_suffixes["backups"].random_string.this`;
  - the bucket's `suffix` wired to it;
  - an empty config planning only the project.
