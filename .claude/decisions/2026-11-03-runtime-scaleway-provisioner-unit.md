## infra/scaleway: foundational projects and IAM

* Date: 2026-11-03
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: runtime

## Context

`.claude/decisions/2026-11-02-runtime-cloudflare-provisioner-unit.md` set the shape of an `infra/` child, and `.claude/decisions/2026-11-03-runtime-digitalocean-provisioner-unit.md` followed it. `infra/scaleway` is the next child, with the same structure, contracts and policies:

- config units composed by `x-cobot`;
- single-resource modules;
- secrets under `COBOT_SECRETS_PASSPHRASE`;
- providers allowlisted, pinned and taken only from `main`;
- date-keyed generations;
- every module, and the schema, a release unit.

It starts with the foundations only: projects, and the IAM that later Scaleway work grants access through. More modules follow in later amendments.

**runway as reference.** runway's `tools/terraform/scaleway/{project,iam-application,iam-policy,iam-api-key}` are the reference, adopted on purpose rather than copied:

- its `iam-api-key` carries `time_rotating` and `terraform_data` beside the key;
- here the key is one resource, and cobot's generations rotate it;
- `scaleway_iam_api_key.expires_at` makes Scaleway enforce each generation's expiry, as Cloudflare does for its tokens.

**No engine change.** Everything here uses features cobot already has:

- `x-cobot.each`, from `.claude/decisions/2026-11-03-runtime-infra-agnostic-shared-modules.md`;
- `rotate`;
- `$module.<m>[$item.<field>]`.

**Break-glass material** is one hand-made API key of an IAM application. Workloads only ever receive minted, expiring keys of applications this unit manages.

Out of scope, each for a later amendment:

- Object Storage, instances, networks and any other Scaleway service;
- SSH keys;
- handing minted keys to workloads;
- IAM groups and users.

## Implementation plan.

### 1. `config.yaml`

```yaml
region: fr-par
projects:
  - { name: pigeon, description: Resources provisioned by cobot. }
rotation: { period_days: 90, overlap_days: 7 }
applications: []
keys: []
```

- `region` configures the provider.
- `projects` lists `{ name, description }` entries. Each is created, never looked up, and none is the organization's default project.
- `rotation` means what it means in `infra/cloudflare`, and applies to `keys`.
- `applications` lists `{ name, description, rules }` entries.
  - Each rule is `{ permission_sets: [..], projects: [names] }` or `{ permission_sets: [..], organization: true }`.
  - The schema rejects `IAMManager`, `OrganizationManager` and `ProjectManager` in any rule, so only break-glass holds them.
- `keys` lists `{ name, application, project }` entries.
  - `application` is a key from `applications`, and `project` a key from `projects`. `project` is the key's default project.

`applications` and `keys` start empty. Each later ADR adds its own entries.

### 2. Composition

- **Secrets:** `access_key`, `secret_key` and `organization_id`.
- **Provider:** `scaleway = { access_key: $secret.access_key, secret_key: $secret.secret_key, organization_id: $secret.organization_id, region: $config.region }`.
- **Blocks:**

  | Block | From | Module | `for_each` | Inputs |
  | --- | --- | --- | --- | --- |
  | `projects` | property | `project` | `$item.name` | `name`, `description` |
  | `applications` | property | `iam-application` | `$item.name` | `name`, `description` |
  | `policies` | `x-cobot.each`, over `applications` | `iam-policy` | `$item.name` | `name: $item.name`, `application_id: $module.applications[$item.name].id`, `rules: $item.rules`, `project_ids: $module.projects[*].id`, `organization_id: $secret.organization_id` |
  | `keys` | property, with `rotate` from `$config.rotation` | `iam-api-key` | `$item.name` | `name: $item.name`, `application_id: $module.applications[$item.application].id`, `default_project_id: $module.projects[$item.project].id`, `not_before` and `expires_on` from `$generation` |

- **Root outputs**, all sensitive: `project_ids`, `application_ids`, `access_keys` and `secret_keys`.

### 3. Modules

Each wraps one resource, with the unified shape:

| Module | Resource | Inputs | Outputs |
| --- | --- | --- | --- |
| `project` | `scaleway_account_project` | `name`, `description` | `id`, `name` |
| `iam-application` | `scaleway_iam_application` | `name`, `description` | `id`, `name` |
| `iam-policy` | `scaleway_iam_policy`, one dynamic `rule` per entry | `name`, `application_id`, `rules`, `project_ids`, `organization_id` | `id`, `name` |
| `iam-api-key` | `scaleway_iam_api_key` | `name`, `application_id`, `default_project_id`, `not_before`, `expires_on` | `id`, and the sensitive `access_key` and `secret_key` |

- **`iam-policy`.**
  - A rule with `projects` maps each name through `project_ids`. A rule with `organization: true` takes `organization_id`.
  - A `precondition` fails the plan, naming the project, when a rule names a project missing from `projects`. This mirrors `api-token`'s check for unknown zones.
- **`iam-api-key`.**
  - `expires_at` is the generation's `expires_on`, and Scaleway refuses the key after it.
  - `description` is `<name>@<not_before date>..<expires_on date>`.
  - `expires_at` cannot change after creation, which suits keys that are minted once per generation.
- **Checked against the provider.** These resources and attributes were checked against `tofu providers schema -json` at `scaleway/scaleway` 2.87.0.

### 4. Secrets

`infra/scaleway/secrets.config.yaml` holds three keys:

- **`access_key`** and **`secret_key`**, the break-glass key.
  - It belongs to an IAM application created by hand in the console, holding `IAMManager` and `ProjectManager` on the organization, plus every permission set any minted rule grants.
  - It has no expiry. Rotating it is a manual, documented act.
- **`organization_id`**, an identifier kept out of the repository, as Cloudflare's `account_id` is.

### 5. Provider

`.config/providers.yaml` gains `scaleway/scaleway: 2.87.0`, the newest version at least two weeks old today. `pigeon cobot lock` regenerates the lockfile.

That lands in its own PR, together with `infra/backblaze`'s provider, before either unit.

### 6. Versions

- `.config/releases.yaml` gains `infra-scaleway-<module>` for each module, and `infra-scaleway-schema`, whose paths are `schema.json` and `modules/`.
- Each starts at `0.1.0`, with a changelog tier that `.config/policies/release-changelog.yaml` lists.

### 7. Docs

`CLAUDE.md` gains a pointer to this ADR.

### 8. Execution

1. **PR A** holds this ADR and `.claude/decisions/2026-11-03-runtime-backblaze-provisioner-unit.md`.
2. **PR B**, `build(cobot): allow the scaleway and b2 providers`.
3. **By hand:** the owner creates the break-glass application and its API key.
4. **PR C**, `feat(infra): scaleway provisioner unit`, holds §1 to §3, §6 and §7.
   - The owner runs `pigeon cobot secrets set infra/scaleway <key>` for each of the three keys, and commits `secrets.config.yaml`.
   - Its plan shows one `scaleway_account_project` to create.

### 9. Verification

- `pigeon cobot validate`, `tofu fmt -check` and `pigeon sentinel check` pass.
- **Locally, with placeholder secrets that are never committed,** `pigeon cobot plan`:
  - inits from a lockfile holding only `scaleway`;
  - plans one project for the empty config;
  - with a sample application, rule and key, plans the application, its policy, and `module.keys["<name>@<from>..<to>"]`.
- **`validate` rejects** `IAMManager` in a rule.
- **`plan` fails:**
  - naming the project, when a rule names an unknown project;
  - on an invalid index, when a key names an unknown application or project.
- **After PR C merges,** the apply commits encrypted state, and the bot tags `infra-scaleway-*/v0.1.0`.

## Amendments

### 2026-11-04: Capabilities, not secrets

[`2026-11-04-development-platform-instance-separation.md`](2026-11-04-development-platform-instance-separation.md) supersedes this unit's secrets. The unit becomes `catalog/scaleway/`. It declares capabilities in `capabilities.yaml` and defines no `secrets.config.yaml`. Its credentials are minted per run from the instance's Scaleway grant, which the pigeon account's Secrets Store holds. The work lands in that ADR's phase 1b, replacing the open implementation PR. The text above stays as written.
