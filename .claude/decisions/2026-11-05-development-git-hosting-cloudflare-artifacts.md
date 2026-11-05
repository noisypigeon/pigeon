## Git is hosted on Cloudflare Artifacts; GitHub is a mirror

* Date: 2026-11-05
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development

## Context

GitHub carries this repository's git, its reviews, its CI and its releases. Its outages now stall the work. On a single day:

- Actions jobs died on 504s from release downloads;
- a push failed with "fatal error in commit_refs_rules";
- a pull request kept a stale head after a push.

The owner wants GitHub out of the critical path, kept only as a backup mirror.

Where GitHub is load-bearing today:

- **Actions runs everything:**
  - `cobot.yml`: plan and apply, PR comments, and the state-repository checkout;
  - `lifecycle.yml`: sentinel and backplane checks;
  - `release.yml`: `cobot release`, tags, GitHub Releases and ghcr images;
  - `rust.yml`;
  - `sites.yml`.
- **Pull requests feed the pull-request policy** (`.config/policies/pull-request.yaml`) and carry the plan comments.
- **Releases are marked done by a GitHub Release,** per [`2026-11-02-release-crate-units-and-images.md`](2026-11-02-release-crate-units-and-images.md). The control plane's Worker bundle is attached to one, per [`2026-11-04-development-platform-instance-separation.md`](2026-11-04-development-platform-instance-separation.md).
- **The engines need only local git.** They use `git archive` and `git tag --list`, so they carry over unchanged.

**Cloudflare Artifacts** is git-compatible storage, in open beta on Workers Paid:

- **API.** A REST API (`/accounts/<account>/artifacts/namespaces/…`) creates namespaces and repositories, imports from a public HTTPS remote and mints repository-scoped tokens. A Workers binding does the same from a Worker.
- **Git.** Clients speak smart HTTP, authenticated by a bearer token.
- **Limits.** 1 GB per new repository, 32 MB per file, and 2,000 git requests per 10 seconds per repository.
- **What it lacks.** It has no reviews, CI or mirroring of its own.

These facts were gathered when this ADR was written, and are checked against Cloudflare's documentation on the first real run.

[`2026-11-05-runtime-control-plane-worker.md`](2026-11-05-runtime-control-plane-worker.md) already places plans, approvals and runs on Cloudflare. This ADR moves git, review, CI and releases there too.

## Implementation plan.

### 1. Repositories

- **Each instance has an Artifacts namespace named after it,** in **that instance's own Cloudflare account**:
  - `pigeon`'s is in the platform's account, the one `infra/pigeon` bootstraps;
  - `noisypigeon`'s is in noisypigeon's own account.
- **The run's token picks the account.** It must see exactly one account (`cloudflare::Api::account`), and no account ID is ever written. Before acting, the CLI prints the account's last four characters and the namespace, so a token for the wrong account shows.
- **A repository is a unit of kind `cloudflare/git-repo`** in its instance's `config.yaml`, per [`2026-11-05-runtime-instance-config-kind-units.md`](2026-11-05-runtime-instance-config-kind-units.md):

  ```yaml
  units:
    repos/pigeon:
      kind: cloudflare/git-repo
      mirror: https://github.com/noisypigeon/pigeon
  ```

  - The repository's name is the unit's last segment.
  - `mirror` names its GitHub backup.
  - `default_branch` defaults to `main`.
- **An API kind.** The kind declares `x-cobot.api: artifacts`. cobot applies it through the Cloudflare API, not OpenTofu, and it never appears in a plan.
- **Its credential.** It requests `artifacts.write` on the account, which a run fills from its own token.
- **Creation.** `pigeon cobot git apply` creates the namespace and the repository. It imports the repository once from `mirror` while the repository is empty, and is idempotent after that.
- **Artifacts is the source of truth.** No recurring sync ever flows from GitHub back to it.
- **The state repositories stay out of scope.** They move to R2 under the separation ADR's phase 5.

### 2. Access

- **No token is stored.** `pigeon cobot git push|fetch|clone <repo> [git args…]` runs the user's `git` with a repository-scoped token minted for that call: read for fetch and clone, write for push. The token reaches git only through `GIT_CONFIG_COUNT`/`KEY`/`VALUE` (`http.<remote>.extraHeader`), never through argv or `.git/config`.
- **A front door, from phase 3.** backplane's Worker, in the platform's account, serves `git.pigeon.dev/<repo>`, with Access in front, and proxies smart HTTP with a token minted per request.
  - **Other accounts.** Reaching another instance's account needs a credential for it, which waits for the control plane to mint per-instance credentials (control-plane ADR §8). Until then, such an instance is reached through `pigeon cobot git … --instance DIR`, with its own token.
  - Pushes are accepted only to `change/*`.
  - `main` and the tags are written only by the control plane.

### 3. Changes replace pull requests

These take effect from phase 3.

- **Opening one.** A push to `change/<name>` opens or updates a change row in D1, holding its number, title, body, head, base and files.
- **Checks** run in Containers, as cobot's `worker` build dispatches runs. Results are D1 rows.
- **Review and approval** go through the CLI and MCP, with the owner signing in through Access, as plan-then-approve does.
- **Merging.** The control plane squash-merges with the subject `<title> (!N)`. The lifecycle ADR's title and squash rules carry over.
- **The pull-request policy** becomes `target: change`, with the same fields. `pull-request` stays as an alias until the cutover.

### 4. CI leaves Actions

- **The checks move:**
  - `rust.yml`, `lifecycle.yml` and `sites.yml` become control-plane checks on `change/*` and `main`;
  - `cobot.yml`'s plan and apply move to the control plane, as already decided.
- **A check's Container fetches its tree** with a read token minted per run. This amends the control-plane ADR's "a run never clones" for checks, which hold no instance credential.

### 5. Releases

- **The release job** runs `cobot release` and `cobot tags` on merge, and pushes its commit and tags itself.
- **Images** go to `registry.cloudflare.com`, which Containers pull from.
- **Worker bundles and release notes** go to R2 at `releases/<unit>/vX.Y.Z/`, with a D1 row. That row, not a GitHub Release, marks a tag done.
- **`.config/releases.yaml`** gains `repository` and `registry` for Cloudflare. Changelog links point at change pages, and existing `(#N)` subjects still parse.

### 6. GitHub is a mirror

- **What is mirrored.** `main` and the tags only, never `change/*`, after every merge and by a daily reconcile that compares ref SHAs.
  - **Until phase 5,** the owner runs `pigeon cobot git mirror` locally, with their own GitHub credentials.
  - **After that,** the control plane mirrors, with a fine-grained token in the Secrets Store.
- **Failures.** A failed mirror is retried and reported as drift; it never blocks a merge.
- **The mirror is read-only by convention.** Pull requests are disabled, and each README names the source. noisypigeon's mirror keeps the owner's GitHub profile current.
- **Failback.** If Artifacts fails, GitHub becomes primary again. Point `origin` at it, and set `repository` and the front door's upstream to it.

### 7. Execution

1. **This change, the last one merged through GitHub:**
   - this ADR;
   - the `cloudflare/git-repo` kind;
   - API units in the manifest;
   - the Artifacts client;
   - `pigeon cobot git apply|status|push|fetch|clone|mirror`.

   pigeon's own root `config.yaml` waits for step 2. Until a cobot release knows API kinds, the released image in CI's `plan` and `apply` would treat `repos/pigeon` as an OpenTofu unit, per [`2026-11-03-release-cobot-image-in-workflows.md`](2026-11-03-release-cobot-image-in-workflows.md).
2. **The owner, locally, from a pigeon checkout,** with `CLOUDFLARE_API_TOKEN` scoped to the platform's account and holding Artifacts Write. First, write the root `config.yaml`:

   ```yaml
   # pigeon as an instance: its units by kind; see .claude/decisions/2026-11-05-runtime-instance-config-kind-units.md.
   schema: v1
   name: pigeon
   description: The pigeon platform, as its own instance.
   repository: https://github.com/noisypigeon/pigeon
   units:
     repos/pigeon:
       kind: cloudflare/git-repo
       mirror: https://github.com/noisypigeon/pigeon
   ```

   Then:

   ```
   cargo run -q --manifest-path src/pigeon-cli/Cargo.toml -p pigeon -- cobot git apply     # namespace pigeon, repo pigeon, imported from GitHub
   cargo run -q --manifest-path src/pigeon-cli/Cargo.toml -p pigeon -- cobot git status    # Artifacts main equals GitHub main
   git add config.yaml && git commit -m "chore: pigeon declares its repository"
   cargo run -q --manifest-path src/pigeon-cli/Cargo.toml -p pigeon -- cobot git push pigeon HEAD:main
   cargo run -q --manifest-path src/pigeon-cli/Cargo.toml -p pigeon -- cobot git mirror
   ```

   That commit is the first to reach `main` through Artifacts, not GitHub. Run step 2 only after the release bot has released cobot from step 1: the mirror's push to GitHub still triggers its `cobot.yml` until the cutover, and that needs a cobot that knows API kinds.

   From then on, every change follows the same routine:
   1. Commit locally.
   2. `mise run ci`.
   3. `pigeon cobot git push pigeon HEAD:main`.
   4. `pigeon cobot git mirror`.

   GitHub pull requests and Actions are no longer used for the work.
3. **The front door, push events, and changes with checks in Containers.**
4. **The release job,** the Cloudflare registry, and R2 releases.
5. **The cutover:**
   - the control plane mirrors;
   - `.github/workflows/` and `BOT_GITHUB_TOKEN` are removed;
   - Devin's GitHub integration is retired;
   - `.config/policies/sentinel-actions.yaml` is retargeted at the job specifications.
6. **noisypigeon** declares `repos/noisypigeon` in its `config.yaml`. It then follows step 2 from pigeon, with `--instance ../noisypigeon` and a token scoped to noisypigeon's own account.

### 8. Supersedes

The superseded texts stay as written:

- **"GitHub Actions is the control plane"** and the bundle on a GitHub Release, in the separation ADR.
- **The GitHub Release that marks a tag done,** in the release ADR.
- **The pull-request mechanics** of [`2026-11-02-development-adr-driven-lifecycle.md`](2026-11-02-development-adr-driven-lifecycle.md), from phase 3.

### 9. Risks

- **Beta.** The API and its limits may change. The mirror and the failback cover an outage.
- **Size.** 1 GB per repository and 32 MB per file. `sentinel` gains a size policy before either is approached.
- **Agent sessions.** Sessions that clone from GitHub read the mirror until the front door exists. Their network allowlist then needs `git.pigeon.dev`.

### 10. Verification

- `tests/git.rs`, against a fake API and local remotes:
  - `apply` creates, imports, and is idempotent;
  - an API unit never plans;
  - the token appears in no argv, file or output;
  - `mirror` moves `main` and the tags but never `change/*`, and overwrites a mirror that diverged.
- After step 2, `pigeon cobot git status` shows Artifacts and GitHub at the same `main`.
- From phase 3, a change is merged while GitHub is unreachable.
- A failback drill on a scratch repository succeeds.
