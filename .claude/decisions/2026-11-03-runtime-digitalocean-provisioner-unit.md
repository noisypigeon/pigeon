## infra/digitalocean: the second provisioner unit

* Date: 2026-11-03
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: runtime

## Context

`.claude/decisions/2026-11-02-runtime-cloudflare-provisioner-unit.md` made `infra/cloudflare` the first child of `infra/`, and set the shape every later child follows:

```
infra/<provider>/
  config.yaml            non-secret options, validated by schema.json
  schema.json            JSON Schema plus x-cobot composition; versioned
  secrets.config.yaml    keys in clear, every value encrypted
  modules/<name>/        one resource each; individually versioned
```

The second child is `infra/digitalocean`. It bootstraps a DigitalOcean account, with these pieces:

- one project that every resource joins;
- private Spaces buckets;
- Spaces keys scoped to those buckets and rotated;
- SSH keys registered for later nodes.

It follows that ADR's contracts and policies unchanged:

- config units and `x-cobot` composition;
- single-resource modules with the unified shape;
- secrets under `COBOT_SECRETS_PASSPHRASE`;
- providers allowlisted, pinned and taken only from `main`;
- rotation in date-keyed generations;
- every module, and the schema, a release unit of its own.

Nothing in that ADR is superseded.

**No engine change.** cobot's config-unit engine is provider-agnostic. That covers `providers`, `secrets`, `singletons`, `for_each`, `rotate` and `$generation`, and `outputs`. Only the apex and workload handoff is bound to `infra/cloudflare`, and this unit uses neither. So no `cobot` release has to come first, and the release-first rule of `.claude/decisions/2026-11-03-release-cobot-image-in-workflows.md` is met as things stand.

**Break-glass material.** The unit holds two hand-made credentials:

- a personal access token;
- a full-access Spaces key.

DigitalOcean's API cannot mint personal access tokens. What this unit can mint is bucket-scoped Spaces keys, so those are the only DigitalOcean credentials a workload could ever be handed.

**Weaker expiry.** DigitalOcean does not expire Spaces keys on its own, unlike Cloudflare tokens. A rotated generation is revoked by the daily scheduled apply destroying it, so it outlives its dates if applies stop running.

Out of scope, each for a later amendment:

- handing Spaces keys or bucket names to workloads;
- DNS, which stays in Cloudflare;
- droplets, VPCs, Kubernetes, databases and a container registry;
- any bucket or key in the initial config;
- any workload that uses DigitalOcean.

## Implementation plan.

### 1. `config.yaml`

```yaml
region: tor1
project:
  name: pigeon
  description: Resources provisioned by cobot.
  purpose: Operational / Developer tooling
  environment: Production
rotation: { period_days: 90, overlap_days: 7 }
ssh_keys: []
buckets: []
keys: []
```

- `region` is the region for every bucket.
- `project` describes the one project. It is never the account's default project.
- `rotation` means what it means in `infra/cloudflare`, and applies to `keys`.
- `ssh_keys` lists `{ name, public_key }` entries.
  - `public_key` is an OpenSSH public key, `ssh-ed25519 <base64>`, with no comment, so no address lands in the repository.
  - The schema rejects any other key type.
- `buckets` lists `{ name, versioning }` entries. `name` is a key of lowercase words joined by dashes, and `versioning` defaults to `true`.
- `keys` lists `{ name, grants: [{ bucket, permission }] }` entries.
  - `bucket` is a key from `buckets`.
  - `permission` is `read` or `readwrite`.
  - The schema rejects `fullaccess`, so only the break-glass key holds it.

Each list starts empty. Each later ADR adds its own entries.

### 2. `schema.json` and its `x-cobot`

**Top level:**

```json
"x-cobot": {
  "version": "0.1.0",
  "secrets": ["provisioner_token", "spaces_access_id", "spaces_secret_key", "bucket_prefix"],
  "providers": {
    "digitalocean": {
      "token": "$secret.provisioner_token",
      "spaces_access_id": "$secret.spaces_access_id",
      "spaces_secret_key": "$secret.spaces_secret_key"
    }
  },
  "singletons": {
    "project": {
      "module": "project",
      "inputs": {
        "name": "$config.project.name",
        "description": "$config.project.description",
        "purpose": "$config.project.purpose",
        "environment": "$config.project.environment"
      }
    },
    "project_resources": {
      "module": "project-resources",
      "inputs": { "project_id": "$module.project.id", "urns": "$module.buckets[*].urn" }
    }
  },
  "outputs": {
    "project_id": "$module.project.id",
    "bucket_names": "$module.buckets[*].name",
    "spaces_access_keys": "$module.keys[*].access_key",
    "spaces_secret_keys": "$module.keys[*].secret_key",
    "ssh_key_ids": "$module.ssh_keys[*].id"
  }
}
```

**Properties:**

| Property | Module | `for_each` | Inputs |
| --- | --- | --- | --- |
| `ssh_keys` | `ssh-key` | `$item.name` | `name: $item.name`, `public_key: $item.public_key` |
| `buckets` | `spaces-bucket` | `$item.name` | `prefix: $secret.bucket_prefix`, `key: $item.name`, `region: $config.region`, `versioning: $item.versioning` |
| `keys` | `spaces-key` | `$item.name` | `name: $item.name`, `grants: $item.grants`, `bucket_names: $module.buckets[*].name`, `not_before: $generation.not_before`, `expires_on: $generation.expires_on` |

- `keys` carries `rotate`, fed `$config.rotation.period_days` and `$config.rotation.overlap_days`, exactly as `infra/cloudflare`'s `tokens` does.
- Root outputs are sensitive and live only in encrypted state, as in `infra/cloudflare`.

### 3. Modules

Each wraps one resource. The table lists the attributes that matter:

| Module | Resource | Inputs | Outputs |
| --- | --- | --- | --- |
| `project` | `digitalocean_project`, `is_default = false` | `name`, `description`, `purpose`, `environment` | `id`, `name` |
| `project-resources` | `digitalocean_project_resources` | `project_id`, `urns` (a map) | `id` |
| `spaces-bucket` | `digitalocean_spaces_bucket`, `acl = "private"`, `force_destroy = false` | `prefix`, `key`, `region`, `versioning` | `id`, `name` (sensitive), `urn` |
| `spaces-key` | `digitalocean_spaces_key` | `name`, `grants`, `bucket_names`, `not_before`, `expires_on` | `id`, `name`, and the sensitive `access_key` and `secret_key` |
| `ssh-key` | `digitalocean_ssh_key` | `name`, `public_key` | `id`, `name`, `fingerprint` |

- **`project-resources`** sets `count = length(var.urns) > 0 ? 1 : 0`. With no buckets it plans nothing, and its `id` output is `null`.
- **Bucket names.**
  - `spaces-bucket` names its bucket `<prefix>-<key>`.
  - `bucket_prefix` is 12 random lowercase alphanumerics, held as a secret. So private bucket names never land in the repository, and the prefix keeps names globally unique.
  - The `name` output is sensitive. Plan summaries print addresses only, and those carry the key, not the name.
- **Spaces keys.**
  - Each generation is its own instance, keyed `<name>@<not_before>..<expires_on>` by cobot.
  - Its DigitalOcean name is the same string, built in `locals` from the dates.
  - One `grant` is rendered per entry, with `bucket` mapped through `bucket_names`.
  - A `precondition` fails the plan, naming the grant, when its `bucket` is not a key of `bucket_names`. This mirrors `api-token`'s check for unknown zones.
- **Pinning.** `required_providers` pins `digitalocean/digitalocean` exactly as in the allowlist.

**Check before writing.** The Cloudflare ADR's first amendment found modules naming the wrong API. So before PR C, the implementing session:

1. runs `tofu providers schema -json` at the pinned version;
2. checks every resource, attribute and output above against it;
3. checks that Spaces serves `tor1`.

Any mismatch lands as an amendment to this ADR first.

### 4. Secrets

`infra/digitalocean/secrets.config.yaml` holds four keys, in the existing format, under `COBOT_SECRETS_PASSPHRASE`:

- **`provisioner_token`**, the break-glass key. It is created by hand in the control panel, as a custom-scoped personal access token. Its scopes are read, create, update and delete on `project`, `ssh_key` and `spaces_key`, plus whatever the provider needs to read Spaces and projects. It has no expiry.
- **`spaces_access_id`** and **`spaces_secret_key`**, a full-access Spaces key created by hand. Only the provider uses it, to manage buckets.
- **`bucket_prefix`**, generated once, for example with `openssl rand -hex 6`. It never changes, since renaming a bucket means replacing it.

Rotating `provisioner_token` or the Spaces key is a manual, documented act, as for Cloudflare's `provisioner_token`. Each is rotated on any suspected leak of `COBOT_SECRETS_PASSPHRASE`.

`validate`'s existing guards then reject these four keys in any workload's secrets.

### 5. Provider

`.config/providers.yaml` gains:

```yaml
  digitalocean/digitalocean: <newest 2.x at least two weeks old>
```

- `pigeon cobot lock` regenerates `.config/providers.lock.hcl`.
- Both land in their own PR, before any unit uses the provider, as the Cloudflare ADR's §5 requires.

### 6. Versions

**Release units.** `.config/releases.yaml` gains one unit per module, and one for the schema, each with no image:

| Unit | Manifest | Paths |
| --- | --- | --- |
| `infra-digitalocean-<module>` | `infra/digitalocean/modules/<module>/module.yaml` | its own directory |
| `infra-digitalocean-schema` | `infra/digitalocean/schema.json` | `schema.json`, `modules/` |

- There are five module units: `project`, `project-resources`, `spaces-bucket`, `spaces-key` and `ssh-key`.
- Every module and the schema start at `0.1.0`.

**Changelogs.**

- Each unit gets a changelog tier beside its files.
- `.config/policies/release-changelog.yaml` lists the new paths.
- The release bot writes them. A PR never does.

### 7. Docs

`CLAUDE.md`'s pointers gain a line: `infra/digitalocean` is the second provisioner unit, per this ADR.

### 8. Execution

1. **PR A**, `docs(adr): infra/digitalocean provisioner unit`, holds this ADR.
2. **PR B**, `build(cobot): allow the digitalocean provider`, holds only `.config/providers.yaml` and `.config/providers.lock.hcl`.
3. **By hand, before PR C:**
   - the owner creates the personal access token and the full-access Spaces key, as §4 describes;
   - the owner generates `bucket_prefix`.
4. **PR C**, `feat(infra): digitalocean provisioner unit`, holds the modules, `schema.json`, `config.yaml`, the release units, the policy change and the docs.
   - It follows §3's check against the provider's schema first.
   - The owner runs `pigeon cobot secrets set infra/digitalocean <key>` for each of the four keys locally, and commits `secrets.config.yaml` to the PR branch.
   - Its plan uses `main`'s providers, and shows only `digitalocean_project` to create.

### 9. Verification

- `mise run ci` passes.
- `pigeon cobot validate` passes.
- `pigeon cobot render infra/digitalocean` writes a `main.tf.json` holding:
  - the `digitalocean` provider fed from `var.secrets`;
  - the `project` and `project_resources` modules;
  - empty `ssh_keys`, `buckets` and `keys` blocks.
- `mise run sentinel` and `sentinel-pr` pass.
- `validate` fails when a scratch config grants `fullaccess`, or gives an SSH key a comment.
- `plan` fails, naming the grant, when a scratch config names a grant's bucket outside `buckets`.
- **PR C's plan** shows `digitalocean_project` ×1 to create, and nothing else.
- **After merge:**
  - the apply commits encrypted state for `infra/digitalocean`;
  - the bot tags and publishes `infra-digitalocean-*/v0.1.0`;
  - the next PR plans "No changes" for the unit;
  - the project appears in the control panel, and is not the default project.
