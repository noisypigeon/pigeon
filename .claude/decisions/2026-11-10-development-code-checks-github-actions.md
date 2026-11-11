## Code checks build and test on GitHub Actions; judging, validation and the platform stay on Cloudflare

* Date: 2026-11-10
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development
* Governs: `.github/workflows/ci.yml`, `.config/policies/mirror*.yaml`, `src/pigeon-cli/crates/backplane/src/actions.rs`, `src/pigeon-cli/crates/backplane/src/worker/github.rs`, `src/pigeon-cli/crates/backplane/src/worker/change.rs`, `src/pigeon-cli/crates/backplane/tests/actions.rs`

## Context

[`2026-11-05-development-git-hosting-cloudflare-artifacts.md`](2026-11-05-development-git-hosting-cloudflare-artifacts.md)'s 2026-11-06 amendment made GitHub a source mirror only:

- its workflows were deleted;
- `.config/policies/mirror.yaml` forbids any file under `.github/workflows/`;
- every check runs in the control plane's `Checker` Containers.

[`2026-11-08-development-github-mirror-on-merge.md`](2026-11-08-development-github-mirror-on-merge.md) has the control plane push `main` and the tags to GitHub, with a GitHub App installation token minted per job. A test asserts that `change/*` never reaches the mirror.

[`2026-11-10-development-factory-crates-functional-core.md`](2026-11-10-development-factory-crates-functional-core.md) §5 decides which tests a push runs (by code path, from a coverage map), and that the merge job runs the full set once on the exact head it squashes.

The factory ADR's execution step 2, done in !137, has `bound` judge a change with `main`'s policies and approvals, never the change's own.

**The owner's direction:**

- CI builds that verify a change's code is valid, so that it compiles, lints clean and passes its tests, run on GitHub Actions instead of Cloudflare.
- Any compliance, validation or execution of the platform stays on Cloudflare.

**What GitHub Actions gives:**

- hosted runners at no cost for a public repository, with caches and parallel jobs;
- the Rust build no longer competes with the platform's own jobs for `Checker` Containers.

**What it must not take:**

- a credential;
- a decision;
- the power to move `main`.

## Implementation plan.

### 1. The split

**On GitHub Actions: code checks only.** Each one reads the tree and nothing else, and passes or fails:

- `fmt`;
- clippy, per crate and per feature set (`mise run features`);
- the wasm builds (`--target wasm32-unknown-unknown`);
- the tests that the factory ADR §5 selects for a push;
- the merge gate's full set (factory ADR §5), on the exact head the merge will squash;
- the coverage run that writes the test map (factory ADR §5).

**On Cloudflare: everything else.**

- `bound`, and every sentinel policy: change, pull-request, secrets, mirror, migrations, decisions, layout;
- `change.scope`: which checks a change needs, and which tests;
- tiers, grants, approvals and mandates;
- `infra.validate`, `cobot validate`, plans and applies;
- the merge, its queue and restacks;
- deploys, site builds and publishes, platform runs;
- the mirror, its reconcile, and the digest.

A check that needs a secret, a Cloudflare binding, or the instance's state is a platform check by definition, and stays on Cloudflare.

### 2. How a change's code reaches GitHub

- **Only after `bound` passes.** The control plane pushes the change's head to GitHub as `change/<n>-<slug>`, by the mirror job (mirror ADR §2) with its installation token. A head that `bound` refused never leaves Cloudflare, so the secrets policies always judge a commit before it is public.
- **Force-pushed on each new head,** like the branch on Artifacts.
- **Deleted on merge or close.** The mirror ADR's rule stays for `main` and tags: fast-forward and add only. `change/*` on GitHub is the control plane's alone, and only it writes there.
- **Public.** The repository is public, and the branch's commits become public before merge. CLAUDE.md's rule already holds for every commit: no secret, credential, key ID or private bucket name, in any commit or branch name.

### 3. How a check runs: `main`'s workflow, dispatched on the head

- **One workflow, `.github/workflows/ci.yml`,** triggered only by `workflow_dispatch`.
  - Its inputs:
    - `sha`, the head to check;
    - `number`;
    - `checks`, the JSON list of checks from `change.scope`;
    - `tests`, the nextest filter from the test map, or `all`.
  - It checks out `sha` and runs exactly the listed checks, through `mise`, with the toolchain `mise.toml` pins.
  - It has no `push`, `pull_request` or `pull_request_target` trigger.
- **The control plane dispatches it on `main`.** The workflow that runs is always `main`'s, never the change's. So a change that edits `ci.yml`, a test filter or its own checks can't loosen the checks it is judged by. This is the rule that !137 applied to `bound`.
- **No secrets.**
  - The workflow declares `permissions: contents: read`, and uses no repository or environment secret.
  - Third-party actions are pinned by commit SHA.
  - Caches are keyed by `Cargo.lock` and the toolchain.
- **Selection stays on Cloudflare.** `change.scope` decides the `checks` and `tests` inputs, and `bound` records them on the change. The workflow only executes them.
- **The merge gate.**
  - The merge job dispatches the full set on the exact head it will squash, after any restack, and waits for it.
  - Since `main` must be an ancestor of that head, the head's tree is the squash's tree.
  - A failure refuses the merge, as in the factory ADR §5.
  - The coverage run writes `tests.json` as a workflow artifact. The control plane copies it to `builds/<sha>/tests.json` on Cloudflare, so the map's reader stays there.

### 4. How results come back

- **The App gains Actions: read and write.** Write is needed to dispatch, and read to follow runs. It has no other new permission. It stays installed on `noisypigeon/pigeon` only.
- **The control plane follows the run.**
  - `ChangeRun` stores the run's ID from the dispatch.
  - Its alarm polls the run through the App, with the same backoff as a `Checker` job, until the run finishes.
  - It has no webhook, so it needs no inbound secret.
- **Each check is a job on the change,** as today. `fmt`, each clippy and each test set keeps its name, status and attempts. Its log tail is copied to `CI_LOGS`, so `change_status`, the dashboard and `ci_rerun` work as before. `ci_rerun` re-dispatches the one check.
- **The result is bound to the head.** A run whose `sha` input is not the change's current head is ignored, as a `Checker` result for an old head is today.

### 5. When GitHub is unavailable

- **A dispatch or poll that fails** is retried with backoff (1, 5 and 30 minutes). The change stays `pushed` and lists the check as `waiting: GitHub`. Nothing on Cloudflare is blocked: approvals, other merges and deploys go on.
- **Break-glass.**
  - The `Checker` classes and the `ci` image stay deployed.
  - The owner can run a change's code checks on Cloudflare with `ci_rerun` and `on: cloudflare`, at the owner tier.
  - A later ADR may remove that path once Actions has run for a while.

### 6. What the policies say

- **`mirror.yaml`** allows exactly one file under `.github/workflows/`: `ci.yml`. Its rules check that:
  - its only trigger is `workflow_dispatch`;
  - its `permissions` are `contents: read`;
  - it names no `secrets.`;
  - every `uses:` is pinned to a 40-character SHA.
- **`.config/approvals.yaml`** guards `.github/**`. A change to the workflow takes the owner's proof, and it is judged only once it is on `main`, since dispatch uses `main`'s workflow (§3).
- **The hosting ADR's 2026-11-06 amendment is amended:**
  - GitHub stays the source mirror, and stays behind Artifacts;
  - it also runs code checks for the control plane, through `ci.yml` alone;
  - no release, deploy or platform action runs there.

### 7. Execution

Each step is its own change.

0. **The mirror runs.** The owner:
   - creates the App (mirror ADR §3), now with Actions: read and write;
   - sets `GITHUB_APP_ID` and `GITHUB_APP_KEY`;
   - runs `pigeon cobot git sync pigeon` for the five tags GitHub holds that Artifacts lacks;
   - runs `deploy_retry`.

   `deploy_status` then shows the mirror in step.
1. **This ADR,** with a `CLAUDE.md` pointer.
2. **`feat(policies)`:** the `mirror.yaml` rules for `ci.yml`, and `.github/**` guarded in `.config/approvals.yaml`. This takes the owner's proof.
3. **`feat(ci)`:** `.github/workflows/ci.yml` (§3). It is dispatched by hand once on `main`'s head, and is used by nothing yet.
4. **`feat(backplane)`:**
   - the mirror job pushes `change/*` heads after `bound`, and deletes them on merge or close (§2);
   - the cobot mirror test is changed to match.
5. **`feat(backplane)`:**
   - code checks dispatch to Actions, and are followed through the App (§3 and §4);
   - `waiting: GitHub` and the retries (§5);
   - `ci_rerun` gains `on: cloudflare` at the owner tier;
   - `deploy_status` and the dashboard show the run's link.
6. **`feat(backplane)`:**
   - the merge gate dispatches the full set;
   - the coverage artifact is copied to `builds/<sha>/tests.json`.

   This lands with, or after, the factory ADR's §5 step.
7. **`docs(claude)`:** `CLAUDE.md`'s Commands describe where each check runs.

### 8. Verification

- **The workflow:**
  - a change that edits `ci.yml` to skip a check is still judged by `main`'s `ci.yml`;
  - a `ci.yml` with a `push` trigger, a secret, or an unpinned action fails `mirror.yaml`.
- **Order:** a head that fails `bound` never appears on GitHub. A merged or closed change's branch is gone from GitHub.
- **Results:**
  - each code check appears on the change with its status and log tail;
  - a result for an old head is ignored;
  - `ci_rerun` re-dispatches one check.
- **Outage:** with dispatch refused, the change shows `waiting: GitHub`, other merges and deploys proceed, and `ci_rerun … on: cloudflare` checks it on Cloudflare.
- **The split:**
  - `bound`, `infra.validate`, the policies, the merge and the deploy still run only on Cloudflare;
  - no workflow run holds a token that can write.
