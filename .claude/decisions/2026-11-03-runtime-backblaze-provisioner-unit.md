## infra/backblaze: private buckets and scoped keys

* Date: 2026-11-03
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: runtime

## Context

`infra/backblaze` is the next child of `infra/`. It has the shape, contracts and policies that `.claude/decisions/2026-11-02-runtime-cloudflare-provisioner-unit.md` set and `.claude/decisions/2026-11-03-runtime-digitalocean-provisioner-unit.md` followed:

- config units composed by `x-cobot`;
- single-resource modules;
- secrets under `COBOT_SECRETS_PASSPHRASE`;
- providers allowlisted, pinned and taken only from `main`;
- date-keyed generations;
- every module, and the schema, a release unit.

It manages B2 storage with two modules:

- private buckets;
- application keys scoped to those buckets and rotated.

**runway as reference.** runway's `tools/terraform/backblaze/{bucket,api-key}` are the reference, adopted on purpose:

- its bucket holds a `random_string` beside `b2_bucket`. Here the random part comes from `infra/agnostic`'s `random-string`, composed per bucket through `x-cobot.each`, exactly as `infra/digitalocean` does, per `.claude/decisions/2026-11-03-runtime-infra-agnostic-shared-modules.md`;
- its key carries `time_rotating` beside `b2_application_key`. Here the key is one resource, and cobot's generations rotate it.

**Expiry is enforced by B2.** It is counted from each key's creation.

**No engine change.** Everything here uses features cobot already has.

**Break-glass material** is one hand-made application key. Workloads only ever receive minted, expiring keys scoped to named buckets, holding file capabilities only.

Out of scope, each for a later amendment:

- lifecycle rules, file lock and CORS;
- public buckets;
- handing minted keys to workloads;
- an `exact_name` override.

## Implementation plan.

### 1. `config.yaml`

```yaml
rotation: { period_days: 90, overlap_days: 7 }
buckets: []
keys: []
```

- `rotation` means what it means in `infra/cloudflare`, and applies to `keys`.
- `buckets` lists `{ name }` entries.
  - `name` is lowercase words joined by dashes, with `maxLength: 56`.
  - Each bucket is named `<name>-<random6>`, within B2's 63-character, globally unique names.
- `keys` lists `{ name, capabilities, buckets }` entries.
  - `buckets` holds keys from `buckets`, and at least one, so every minted key is bucket-scoped.
  - `capabilities` is a non-empty subset of `listBuckets`, `readBuckets`, `listFiles`, `readFiles`, `shareFiles`, `writeFiles` and `deleteFiles`. Key, bucket and settings capabilities stay with break-glass, and the schema rejects them.

Both lists start empty. Each later ADR adds its own entries.

### 2. Composition

- **Secrets:** `application_key_id` and `application_key`.
- **Provider:** `b2 = { application_key_id: $secret.application_key_id, application_key: $secret.application_key }`.
- **Blocks:**

  | Block | From | Module | `for_each` | Inputs |
  | --- | --- | --- | --- | --- |
  | `bucket_suffixes` | `x-cobot.each`, over `buckets` | `agnostic/random-string` | `$item.name` | none |
  | `buckets` | property | `bucket` | `$item.name` | `key: $item.name`, `suffix: $module.bucket_suffixes[$item.name].result` |
  | `keys` | property, with `rotate` from `$config.rotation` | `api-key` | `$item.name` | `name: $item.name`, `capabilities: $item.capabilities`, `buckets: $item.buckets`, `bucket_ids: $module.buckets[*].id`, `period_days` and `overlap_days` from `$config.rotation`, `not_before` and `expires_on` from `$generation` |

- **Root outputs**, all sensitive: `bucket_names`, `bucket_ids`, `key_ids` and `keys`.

### 3. Modules

Each wraps one resource, with the unified shape:

| Module | Resource | Inputs | Outputs |
| --- | --- | --- | --- |
| `bucket` | `b2_bucket`, `bucket_type = "allPrivate"` | `key`, `suffix` | `id` and `name`, both sensitive |
| `api-key` | `b2_application_key` | `name`, `capabilities`, `buckets`, `bucket_ids`, `period_days`, `overlap_days`, `not_before`, `expires_on` | `id` (sensitive), `name`, and a sensitive `value` |

- **`bucket`** names its bucket `<key>-<suffix>`. Its `id` and `name` are sensitive, so private bucket names never leave encrypted state.
- **`api-key`:**
  - **Name.** `key_name` is `<name>-<not_before as YYYYMMDD>`, using only letters, digits and dashes.
  - **Scope.** `bucket_ids` maps each of `buckets` through `bucket_ids`. A `precondition` fails the plan, naming the bucket, when one is not a key of `bucket_ids`.
  - **Expiry.** `valid_duration_in_seconds` is `(period_days + overlap_days) × 86400`, a fresh generation's full length. A key is minted no earlier than its `not_before`, so B2 expires it no earlier than cobot's `expires_on`. The daily scheduled apply destroys it on that date.
- **Checked against the provider.** These resources and attributes were checked against `tofu providers schema -json` at `backblaze/b2` 0.14.0. `application_key_id` is not marked sensitive by the provider, so the module's `id` output marks it.

### 4. Secrets

`infra/backblaze/secrets.config.yaml` holds two keys:

- **`application_key_id`** and **`application_key`**, the break-glass key.
- It is created by hand in the B2 console, with:
  - `listKeys`, `writeKeys` and `deleteKeys`;
  - `listBuckets`, `readBuckets`, `writeBuckets` and `deleteBuckets`;
  - every file capability §1 allows.
- It has no expiry. Rotating it is a manual, documented act.
- The key ID is kept out of the repository, as `CLAUDE.md` requires of key IDs.

### 5. Provider

`.config/providers.yaml` gains `backblaze/b2: 0.14.0`, the newest version at least two weeks old today. `pigeon cobot lock` regenerates the lockfile.

That lands in its own PR, together with `infra/scaleway`'s provider, before either unit.

### 6. Versions

- `.config/releases.yaml` gains:
  - `infra-backblaze-bucket` and `infra-backblaze-api-key`;
  - `infra-backblaze-schema`, whose paths are `schema.json`, `modules/` and `infra/agnostic/modules/random-string/`.
- Each starts at `0.1.0`, with a changelog tier that `.config/policies/release-changelog.yaml` lists.

### 7. Docs

`CLAUDE.md` gains a pointer to this ADR.

### 8. Execution

1. **PR A** holds this ADR and `.claude/decisions/2026-11-03-runtime-scaleway-provisioner-unit.md`.
2. **PR B**, `build(cobot): allow the scaleway and b2 providers`.
3. **By hand:** the owner creates the break-glass key.
4. **PR C**, `feat(infra): backblaze provisioner unit`, holds §1 to §3, §6 and §7.
   - The owner runs `pigeon cobot secrets set infra/backblaze <key>` for both keys, and commits `secrets.config.yaml`.
   - Its plan shows no changes, since both lists are empty.

### 9. Verification

- `pigeon cobot validate`, `tofu fmt -check` and `pigeon sentinel check` pass.
- **Locally, with placeholder secrets that are never committed,** with a sample bucket and key, `pigeon cobot plan`:
  - inits from a lockfile holding exactly `b2` and `random`;
  - plans `module.bucket_suffixes["<name>"].random_string.this`, the bucket, and `module.keys["<name>@<from>..<to>"]`.
- **`validate` rejects** `writeKeys` in a key, and a key with no `buckets`.
- **`plan` fails**, naming the bucket, when a key names an unknown bucket.
- **After PR C merges,** the apply commits encrypted state, and the bot tags `infra-backblaze-*/v0.1.0`.
