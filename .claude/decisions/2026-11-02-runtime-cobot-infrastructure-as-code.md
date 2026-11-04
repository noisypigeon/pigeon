## Cobot: infrastructure as code with state in git

* Date: 2026-11-02
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: runtime

## Context

runway's Terraform works, and running it costs a great deal:

- **Configuration.** A 2,030-line generated `root.hcl` and a 988-line reusable run workflow.
- **State.** State lives in per-zone Scaleway buckets, each with its own deployer key. Locks are released by hand, from a runbook.
- **Applies.** An apply is a PR comment, can take several stages, and then needs a bot merge.
- **Churn.** Generated stubs were committed and then ignored, the layout was migrated several times, and every leaf move forced a manual state move.
- **Availability.** `init` fails whenever garden is down.

What worked, and is kept:

- declarative unit files, rendered into Terraform by a tool rather than written by hand per leaf;
- a plan on every PR;
- strict discipline about secrets.

pigeon replaces the rest:

- **Engine.** OpenTofu, because its native client-side state encryption lets state be committed without exposing what it holds.
- **State.** One private repository, `noisypigeon/pigeon-cobot-state`. It mirrors pigeon's directory structure, so a unit's path is its state key. git provides history, backup and review, with no buckets and no per-unit credentials.
- **Apply.** A plan on every PR, and an apply on merge. What `main` says is what gets applied, with no apply comments and no bot merge.
- **Placement.** Only `infra/` and `workloads/` may hold Terraform. That matches the purposes `.claude/decisions/2026-11-02-development-repository-area-layout.md` gave them: break-glass access and bootstrapping in `infra/`, and workloads in `workloads/`.
- **Schema.** The concept is kept, but the schema starts from scratch as v0, and later amendments iterate on it. Nothing of runway's `workload.yaml` or `zone.yaml` is carried over.
- **Code.** A new crate, `cobot`, invoked as `pigeon cobot`. It is a release unit like `sentinel` and `gantry`.
- **Enforcement.** New sentinel policies make the placement rules, and the rule that only cobot's workflow acts on infrastructure, checkable.

This is part one. It proves the pipeline end to end with a unit that provisions nothing real. Out of scope, each for a later amendment:

- real providers and their credentials, and secrets inside units;
- module sources beyond a unit's own directory, such as `tooling/`;
- moving or renaming a unit, and migrating its state;
- a cobot image that carries `tofu`;
- drift detection;
- dependencies between units.

## Implementation plan.

### 1. Units and schema v0

- A unit is a directory under `infra/` or `workloads/` that holds a `unit.yaml`.
- Its path from the repository root is its identity, and its state key.
- Units do not nest.

`unit.yaml` at schema v0. Unknown keys are rejected:

```yaml
schema: 0
name: bootstrap
kind: infra
modules:
  - name: proof
    source: ./modules/proof
    inputs: { message: hello }
```

- `name` and each module `name` are lowercase words joined by dashes. Module names are unique within the unit.
- `kind` is `infra` or `workload`, and must match the unit's top-level directory.
- `source` is a relative path that resolves to a directory inside the unit.
- `inputs` maps to the module's variables, and its values are passed through as JSON.
- Hand-written HCL exists only as a unit's local modules.

The proof unit is `infra/bootstrap/`:

- `unit.yaml`, as above;
- `modules/proof/main.tf`, holding one `variable "message"` and one `terraform_data` resource that records it.

It needs no credentials and costs nothing.

### 2. The `cobot` crate

- It lives at `src/pigeon-cli/crates/cobot`, behind the `pigeon` feature `cobot`, which is on by default.
- Its commands:
  - **`pigeon cobot validate`** finds every unit under `infra/` and `workloads/` and checks it against §1. It exits `1` on any problem, listed as `path: message`.
  - **`pigeon cobot render [UNIT…]`** writes each unit's `main.tf.json` into the gitignored work directory `.cobot/<unit path>/`. The file holds:
    - one `module` block per module, with `source` as an absolute path and the `inputs`;
    - `terraform { required_version, backend "local" { path } }`.
  - **`pigeon cobot plan [UNIT…] --state DIR [--summary FILE]`**:
    1. renders;
    2. runs `tofu init -input=false`;
    3. runs `tofu plan -detailed-exitcode -out=<work>/plan`;
    4. writes a Markdown summary, one section per unit: no changes, or the `tofu show` text of its plan.
  - **`pigeon cobot apply [UNIT…] --state DIR`** plans, then runs `tofu apply` on the saved plan, but only for units with changes.
- With no `UNIT`, a command covers every unit.
- State lives at `DIR/<unit path>/terraform.tfstate`.
- **Encryption:**
  - cobot sets `TF_ENCRYPTION` for every `tofu` call. It uses the `pbkdf2` key provider, with the passphrase taken from `COBOT_STATE_PASSPHRASE`, and the `aes_gcm` method.
  - Encryption is `enforced` for both state and plan files.
  - Without the variable, cobot exits `2` before running anything, so plaintext state is never written.
- `tofu` runs through `std::process::Command`, and its output streams through.
- **Tests:**
  - validation cases, both passing and failing;
  - render output;
  - an end-to-end `plan` and `apply` of a fixture unit against a temporary state directory, which asserts the written state is encrypted. It runs whenever `tofu` is on `PATH`, which in CI it is, through Mise.

### 3. Toolchain

- `mise.toml` pins `opentofu` to its newest release that is at least two weeks old.
- `.gitignore` gains `/.cobot/`.
- `.config/policies/layout.yaml` ignores `.cobot` at the root.

### 4. The state repository

- `noisypigeon/pigeon-cobot-state` is private, and is created by hand.
- `istx25` is a collaborator, so the existing `BOT_GITHUB_TOKEN` reaches it.
- It mirrors pigeon: `infra/bootstrap/terraform.tfstate`, and so on. It holds only OpenTofu's encrypted state.
- Each apply run is one commit, `cobot: apply <units> from noisypigeon/pigeon@<sha>`.

### 5. The workflow: `.github/workflows/cobot.yml`

- **Job `plan`**, on `pull_request`:
  1. checks out pigeon, and checks out the state repository read-only with `BOT_GITHUB_TOKEN`;
  2. sets up Mise;
  3. runs `pigeon cobot validate`, then `pigeon cobot plan --summary`;
  4. posts the summary as the job summary, and as one sticky PR comment marked `<!-- cobot-plan -->` that is updated in place (`pull-requests: write`).
- **Job `apply`**, on `push` to `main`:
  1. runs in `concurrency: { group: cobot-apply, cancel-in-progress: false }`;
  2. checks out the state repository writable;
  3. runs `pigeon cobot apply`;
  4. commits and pushes the state repository.

  A rejected push fails the run without retrying, because the apply has already happened and needs a person.
- **Secrets:** `COBOT_STATE_PASSPHRASE` (new) and `BOT_GITHUB_TOKEN`.
- cobot runs from source here, through Mise and Cargo, until its image carries `tofu`.

### 6. Release unit

- `cobot` sets its own `version = "0.1.0"`, and joins `.config/releases.yaml` with a changelog tier.
- `image` becomes optional in the release config, and `cobot` has none for now:
  - `pigeon sentinel tags` reports `image: null`;
  - the release workflow's `image` job skips the Docker steps when there is no image, and still creates the GitHub Release.

### 7. Sentinel policies

The engine gains a `tree` rule kind, `find`:

- `match` is a regex applied to each path relative to the root, found by walking the repository recursively.
- `allow_under` lists the path prefixes where a match is allowed. An empty list allows it nowhere.
- `forbid_content`, if set, is checked against the contents of each matched file that is not allowed. With it, the rule fails only on files whose content matches.
- `ignore` lists regexes for paths that are never walked: `.git`, `target` and `.cobot`.

New policies:

- **`.config/policies/terraform.yaml`**: "Terraform lives only in infra/ and workloads/, and state never lives in this repository."
  - `.tf`, `.tf.json`, `.tfvars`, `.terraform.lock.hcl` and `unit.yaml` files are allowed only under `infra/` and `workloads/`;
  - `*.tfstate*` and anything under a `.terraform/` directory are allowed nowhere.
- **`.config/policies/terraform-actions.yaml`**: "Only the cobot workflow acts on infrastructure."
  - no file under `.github/workflows/` other than `cobot.yml` may mention `tofu`, `terraform` or `cobot apply`.

Each new rule gets fixtures in the sentinel tests, and each fixture is checked to fail for its intended reason.

### 8. Docs

`CLAUDE.md`:

- points to this ADR;
- lists `pigeon cobot validate|render|plan`;
- says never to run `tofu apply`, or `pigeon cobot apply`, by hand against the state repository.

### 9. Execution

1. **PR A** holds this ADR.
2. **By hand, before PR B:**
   - create the private `noisypigeon/pigeon-cobot-state` with an initial commit;
   - invite `istx25` to it;
   - add the `COBOT_STATE_PASSPHRASE` Actions secret to pigeon.
3. **PR B**, titled `feat(cobot): plan on PR, apply on merge, encrypted state in git`, holds steps 1–8.
   - Its `plan` job comments one resource to add for `infra/bootstrap`.
   - On merge, `apply` commits encrypted state to `pigeon-cobot-state`, and the release bot cuts `cobot/v0.1.0`.

### 10. Verification

- `mise run ci` passes, including the end-to-end `tofu` test.
- `pigeon sentinel check` fails when:
  - a `.tf` file sits under `src/`;
  - a `.tfstate` file sits anywhere;
  - `rust.yml` mentions `tofu`.
- PR B's plan comment shows `terraform_data` to be created in `infra/bootstrap`.
- After merge:
  - `pigeon-cobot-state` holds `infra/bootstrap/terraform.tfstate`, and its content is an encrypted envelope, not readable state;
  - the next PR plans "No changes" for the unit.

## Amendments

### 2026-11-02: Engine first, no fork plans, and no PR code near the state secrets

A review of #17 found three defects in this ADR's plan, and a review of this amendment found three more:

- **The release-first loop blocks PR B.** The `lifecycle` check runs the released `pigeon-sentinel` image, which cannot parse a policy using the new `find` rule. A PR adding both could never pass, so §9 has to land in more than one PR.
- **Fork PRs get no secrets, and no write token.** GitHub withholds repository secrets from `pull_request` runs triggered from forks, and gives them a read-only `GITHUB_TOKEN`. A fork PR could neither be planned nor be told why. pigeon takes no external contributions, so forks are excluded rather than accommodated.
- **PR code runs beside the secrets.** As written, `plan` builds cobot from the PR and initializes the PR's HCL, while `BOT_GITHUB_TOKEN` and `COBOT_STATE_PASSPHRASE` are present. A changed `build.rs`, test or module could read both, and decrypt every state file.
- **PR configuration can read the runner.** Even without a provider, HCL functions such as `file("/proc/self/environ")` read the environment and the filesystem during `plan`, and whatever a plan shows is posted publicly.
- **The first cobot PR could not plan.** A plan built from `main` has no `pigeon cobot` until cobot itself has merged.

This supersedes §5's `plan` job and §9's execution. Both texts stay as written.

#### 1. Execution, in three PRs

1. **`feat(sentinel): find rule and optional release images`**:
   - the `find` rule kind and its tests, with no policy using it;
   - optional `image` in the release configuration.

   The release bot then cuts and publishes the new `sentinel` image.
2. **`feat(cobot): the cobot crate and pigeon cobot`**:
   - the crate, its CLI and tests (§1's schema, §2, §3 and §6);
   - no workflow, no unit under `infra/`, and no new policy.
3. **`feat(cobot): plan on PR, apply on merge`**:
   - `cobot.yml`, the proof unit, `terraform.yaml`, `terraform-actions.yaml` and the `CLAUDE.md` changes.

   Its plan runs on the `main` that now holds PR 2's cobot.

#### 2. Plans never run PR-built code with secrets

- **The binary comes from `main`.** The `plan` job builds `pigeon` from a checkout of `main`, in its own directory, and never from the PR's Rust. It then checks out the PR into a second directory, and reads only its `infra/` and `workloads/` as data.
- **Providers are allowlisted by construction.** cobot runs `tofu init` with `-plugin-dir` pointing at an empty directory, so only the built-in `terraform` provider is available. A unit that needs any other provider fails `init` before any secret-bearing step. Later amendments that add providers do so through an explicit, pinned plugin mirror, not network installation.
- **Provisioners are rejected.** `pigeon cobot validate` rejects any `provisioner` block in a unit's modules.
- **Functions that read the runner are rejected.** `validate` rejects any call, in a unit's `.tf` files, to:
  - `file`, `fileexists`, `fileset`;
  - `filebase64`, `filebase64sha256`, `filebase64sha512`, `filemd5`, `filesha1`, `filesha256`, `filesha512`;
  - `templatefile`, `abspath` or `pathexpand`.

  Later amendments may allow a vetted subset.
- **Plans publish structure, never values.**
  - cobot captures `tofu`'s output instead of echoing it.
  - The summary is built from `tofu show -json`: each resource's address and action only (create, update, replace, delete, or no-op), plus the counts.
  - A failed `tofu` call reports its exit code and the first line of each diagnostic summary, never its detail.
  - Before printing anything, cobot replaces every occurrence of the passphrase with `***`.
- **Secrets are scoped to the step that needs them.**
  - `COBOT_STATE_PASSPHRASE` is set only on the `plan` step.
  - The state repository is checked out read-only, with `persist-credentials: false`, before the PR checkout. The bot token appears only in that step.
- **Trust boundary.** Only someone with write access can open a same-repository PR, and those people are trusted with this repository. These measures keep them from leaking the secrets by accident or through a careless dependency, not from acting in bad faith.

#### 3. No external contributions

- pigeon takes no external contributions. Its interaction limits are set to collaborators only, by hand in the repository settings, so fork PRs, issues and comments from others are not expected.
- As a guard, the `plan` job still runs only when `github.event.pull_request.head.repo.full_name == github.repository`. A fork PR that slips through is skipped, and nothing is posted.
- No `pull_request_target` workflow is added.

### 2026-11-03: Moving a unit's state

Context's out-of-scope list left "moving or renaming a unit, and migrating its state" to a later amendment. The workloads ADR's amendment of this date moves `workloads/cloudflare/apex` to `workloads/noisypigeon.com/apex`. That unit's nine DNS records are live, and they must not be destroyed and recreated.

A move changes only where the state lives. The addresses inside the state must already match, as they do here, because the per-zone apex composes to the same addresses. Renaming addresses within a unit stays out of scope.

#### 1. `.config/moves.yaml`

```yaml
description: Units whose state moves to a new path, once.
moves:
  - { from: workloads/cloudflare/apex, to: workloads/noisypigeon.com/apex }
```

`validate` rejects:

- a move between areas;
- a `to` that is not a discovered unit;
- a `from` that still is one;
- a path in two moves.

#### 2. Plan and apply

- **Plan.** While `to` has no state and `from` does, cobot plans `to` against a copy of `from`'s state. The summary notes "state moves from `<from>`".
- **Apply.**
  1. While `to` has no state, cobot copies `from`'s state file to `to`, still encrypted, byte for byte.
  2. Once `to` has state, it is never overwritten: it may hold what a failed apply already changed.
  3. cobot applies `to`.
  4. After any successful apply of `to`, including one with no changes, it deletes `from`'s file.

  The workflow's `git add -A` commits the result in the run's single state commit. If the apply fails, `to` keeps whatever partial state it wrote, `from` stays, and the next run retries against `to` alone.
- **Spent entries.** Once `from` has no state, the entry does nothing, and a later PR removes it.
- **Address moves.** A resource change with a `previous_address` is listed as `move <from> → <to>`, never hidden as "No changes".

#### 3. Verification

An end-to-end test with built-in fixtures:

1. applies a unit;
2. moves it with an entry;
3. plans the note with no changes;
4. applies, checking that the new state file exists and the old one is gone;
5. replans "No changes".

A second end-to-end test fails an apply after the copy, and checks that the retry plans against `to`'s partial state, never `from`'s.

Unit tests cover each `validate` rejection.

### 2026-11-04: Runs move to the platform

[`2026-11-04-development-platform-instance-separation.md`](2026-11-04-development-platform-instance-separation.md) moves plan and apply off GitHub Actions. A Worker on `pigeon.dev` runs them in Cloudflare Containers, and an approved plan, not a merge, gates an apply. Encrypted state moves from the state repository to R2. The CI plan and apply in this ADR, and the state repository, stay until that ADR's phase 6 cut-over. The text above stays as written.
