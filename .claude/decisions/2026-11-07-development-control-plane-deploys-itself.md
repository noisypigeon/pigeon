## The control plane deploys itself: Worker upgrades on merge, with no local step

* Date: 2026-11-07
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development
* Governs: `src/pigeon-cli/crates/backplane/src/worker/main_run.rs`, `src/pigeon-cli/crates/backplane/src/ship.rs`, `src/pigeon-cli/crates/backplane/src/cobot/ship*.rs`, `src/pigeon-cli/crates/backplane/migrations/**`, `.config/policies/migrations.yaml`, `infra/pigeon/workers/control/worker.yaml`

## Context

Every Worker upgrade still runs on the owner's machine. After a merge, the
owner fetches `main` and runs `pigeon cobot workers plan --state <dir>`, then
`deploy --plan <ID>`. That run:

- reads `infra/pigeon`'s tofu outputs with the state passphrase, to resolve
  the spec's `$output.infra/pigeon.…` references;
- builds the `Checker` and `Runner` images from the Dockerfile's `ci` stage
  with local Docker, under emulation on Apple Silicon;
- applies D1 migrations;
- runs `wrangler deploy`.

On 2026-11-06 and 2026-11-07, changes 19 to 24 each needed that by hand, and
it went wrong in two ways:
- a deploy from a checkout that hadn't fetched `!21` shipped the old approval
  page;
- a machine that died mid-deploy left the owner unsure whether it had landed.

Two earlier decisions point here.
[`2026-11-06-development-control-plane-approvals-images-state.md`](2026-11-06-development-control-plane-approvals-images-state.md):
- **§2** planned a build-and-deploy job on merge, `pigeon` from `builds/`,
  and images by rootless BuildKit after a spike;
- **§3** keeps the state passphrase the owner's, never a Worker secret.

This ADR decides how Worker upgrades go out from Cloudflare alone, and
replaces §2's build-and-deploy job for Workers.

What it rests on:

- **`wrangler deploy` needs Docker only when a container's `image` is a
  Dockerfile path.**
  - With a registry reference it needs none.
  - A deploy that changes no container configuration starts no container
    rollout (Cloudflare's Containers deploy guide).
  - Most upgrades change only Worker code, and the images change only with
    the toolchain, `Cargo.lock`, `mise.toml` or the Dockerfile.
- **`MainRun` already runs a deploy job on every push to `main`.** It runs on
  a `Runner` Container, one at a time, always of the newest push, and
  publishes the sites whose inputs changed (`cobot::scope::deploys`,
  `jobs.rs`). It reports in `deploy_status`.
- **`cobot::workers` already renders, deploys and lists permissions:**
  `render`, `deploy`, `missing_secrets`, `permissions` and `resolve`.
- **The `ci` image lacks what a deploy needs.** It has Rust, the wasm target,
  git, `tofu`, `zola` and `pigeon`, but no Node, no wrangler and no
  `worker-build`.

Decided with the owner:
- **The deploy is a job in our own `MainRun` pipeline**, not Workers Builds:
  one pipeline, ordered with the sites, shown in `deploy_status`, and gated
  by our own approvals.
- **It goes out automatically after merge.** The merge, approved with the
  owner's authenticator code in the card, covers the deploy, as it already
  covers the sites.

## Implementation plan.

### 1. What deploys, and when

- **On each push to `main`, the deploy job:**
  - publishes the sites whose inputs changed, as now;
  - then deploys each Worker whose inputs changed since the last deploy that
    succeeded.
- **A Worker's inputs are:**
  - its `worker.yaml`;
  - its D1 migrations;
  - its Dockerfile stage;
  - its crate's build, by the same rule as the release units: the crate, its
    `cargo tree`, and the workspace's build settings.
- **`cobot::scope` gains `workers(dir, files)`** beside `deploys`, and
  `change.scope` names a Worker deploy so a change shows what its merge will
  roll out.
- **No second approval.** A merge approved with the owner's code deploys
  without another one.

### 2. The deploy's tools

- **Node and wrangler.** `mise.toml` pins them, and the `ci` stage installs
  them, checksum-pinned, as it does `tofu` and `zola`.
- **`worker-build`** is installed with `cargo install --locked` at a pinned
  version.
- **A cobot test** checks the image's versions against `mise.toml`, as it
  does for `tofu` and `zola`.

### 3. Outputs without the passphrase

- **The passphrase stays the owner's (§3).** A Worker spec references a few
  of `infra/pigeon`'s outputs: a D1 database ID, bucket names.
- **The owner publishes them** after each `infra/pigeon` apply, which is
  rare:

  ```
  pigeon cobot workers outputs --root . --state <dir> \
    | pigeon cobot workers secrets put --root . pigeon-control DEPLOY_OUTPUTS
  ```

  `DEPLOY_OUTPUTS` is a JSON Worker secret holding only the outputs some spec
  references.
- **The deploy job resolves `$output.…` from it.** A reference missing from
  it refuses the deploy, naming the reference and the command above.
- **No ID or bucket name enters the repository.**

### 4. Images by reference

- **Each image is rendered as a registry reference.** A container's `image`
  is its last pushed image in Cloudflare's registry, kept beside the outputs
  in `DEPLOY_IMAGES`. A Worker deploy therefore needs no Docker and, when only
  code changed, rolls no containers.
- **When an image's inputs changed, the deploy waits** as **needs image**,
  and `deploy_status` says so. The inputs are the Dockerfile, `Cargo.lock`,
  `mise.toml`, the toolchain, and, until §5, `pigeon`'s own job-running code.
  - **Until §6's image job:** the owner pushes the image once with
    `pigeon cobot workers images push`, which builds the stage, pushes it,
    and updates `DEPLOY_IMAGES`. The pipeline then goes on.
  - **After the BuildKit spike** of §2 of the approvals ADR succeeds, an
    image job on `Checker` builds and pushes it instead.

### 5. `pigeon` from `builds/`

- **The deploy job builds `pigeon`** for the merged head, in release mode,
  and uploads it to the CI bucket as `builds/<sha>/pigeon`.
- **A job Container fetches the binary** for the head it runs, before it
  starts, falling back to the binary in its image.
- **Job-running code then deploys with the Worker:** `jobs.rs`, cobot's CI
  steps and the job server stop being image inputs.

### 6. Credentials

The deploy job alone receives two more Worker secrets, as it already receives
`DEPLOY_TOKEN`:

- **`WORKERS_DEPLOY_TOKEN`:** an account token with exactly what
  `pigeon cobot workers permissions` lists for the platform's specs;
- **`DEPLOYER_ACCESS_ID` and `DEPLOYER_ACCESS_SECRET`:** a dedicated Access
  service token, allowed only `/healthz`, for the health check.

Checks never receive them. The deploy job runs only code merged with the
owner's code.

### 7. Rollout, health and rollback

1. **Migrations first.**
   - D1 migrations run before the new code, and a rollback doesn't undo them,
     so a migration must be backward-compatible: expand in one change,
     contract in a later one.
   - A sentinel rule on `migrations/` refuses a migration that drops or
     renames a table or column.
2. **A new version.** `wrangler versions upload` creates it beside the current
   version, which is recorded. Then `wrangler versions deploy <new>@100%`.
3. **Health.**
   - For up to two minutes, `GET /healthz` must answer with the new version's
     ID.
   - It is a new route behind Access, which pings D1 and a Durable Object.
4. **Rollback.**
   - If health fails, `wrangler rollback` to the recorded version.
   - The deploy reports failed with its log. The next push to `main` tries
     again.
5. **`deploy_status`** shows, per deploy:
   - the sites published;
   - each Worker's previous and new version IDs;
   - the outcome.

### 8. The control plane deploying itself

- **A code-only deploy rolls no containers,** so the `Runner` running the
  deploy is not replaced under it.
- **`MainRun`'s pipeline lives in Durable Object storage,** so it outlives
  the new code. Its alarm picks up the deploy's report as before.
- **A lost deploy job is retried,** and a repeated deploy of the same head is
  a no-op.
- **An image rollout comes last,** after the Worker's health check and report,
  so a replaced `Runner` costs nothing.

### 9. Break-glass

- **The local deploy stays** for the first deploy and for when the pipeline
  can't deploy:
  - `pigeon cobot workers plan --state <dir>`;
  - `pigeon cobot workers deploy --plan <ID>`.
- **It needs Docker only when an image changes.**

### Execution

Each step is a change through the pipeline, in this order:

1. This ADR, with a `CLAUDE.md` pointer.
2. `pigeon cobot workers outputs`, `DEPLOY_OUTPUTS` and `DEPLOY_IMAGES` in
   rendering, `pigeon cobot workers images push`, and `/healthz`.
3. Node, wrangler and `worker-build` in the `ci` stage. Its image is pushed
   once from the owner's machine, the last routine local image build.
4. Worker deploys in `MainRun`'s deploy job, with migrations, versions, health
   and rollback, and the migration rule.
5. `pigeon` from `builds/`.
6. The BuildKit spike, then image jobs.

### Verification

- **A merge that changes only Worker code** is live within minutes of
  merging, with no local step. `deploy_status` shows the new version.
- **A health check made to fail** rolls the Worker back to the recorded
  version, and the deploy reports failed.
- **A change to the Dockerfile** shows **needs image** and deploys once the
  image is pushed.
- **A `DEPLOY_OUTPUTS` missing a reference** refuses the deploy, naming it.
- **A job-runner fix,** after §5, runs in the next change's jobs without an
  image build.

## Amendments

### 2026-11-07: Steps 2 to 4 land as one change; the job image blocks a deploy only on its protocol

- **One change.** The deploy job's new code runs only in a rebuilt `ci` image,
  so the snapshot, images by reference, `/healthz`, the image's tools and the
  Worker deploy in `MainRun` land together.
- **The owner's one-time setup, from the merged `main`:**
  - set `WORKERS_DEPLOY_TOKEN`;
  - create the deployer's Access service token, add it to the Access
    application, and set `DEPLOYER_ACCESS_ID` and `DEPLOYER_ACCESS_SECRET`;
  - publish `DEPLOY_OUTPUTS` with `pigeon cobot workers outputs`;
  - push the new image with `pigeon cobot workers images push` into
    `DEPLOY_IMAGES`;
  - deploy once by hand.
- **§4's "needs image" narrows** (`cobot::scope::image_inputs`).
  - Until §5, every Rust change would leave the image behind `pigeon`, which
    would hold nearly every deploy.
  - **A deploy waits** only when a change since the images' commit touches
    the image's tools (the Dockerfile, `mise.toml`, the toolchain) or the job
    protocol the Worker and the job server share (backplane's `control.rs`,
    `jobs.rs`, `changes.rs`, `runs.rs` and `serve.rs`).
  - **Other changes deploy.** Any other `pigeon` code or `Cargo.lock` only
    leaves the image behind, and the deploy's log says so.
  - **After a new image** is pushed and `DEPLOY_IMAGES` set, `deploy.retry` (a
    build operation, open to the agent) deploys `main`'s newest push again.
- **The deploy job runs on `Checker`,** because it builds the Worker to wasm,
  amending the approvals ADR's §4 table. The `ci` image warms that release
  build.
- **The deploy uses `wrangler deploy`,** not `versions upload`. It records the
  version serving before and after from the Workers deployments API, and a
  rollback is a deployment of the recorded version at 100%.
- **Only `/healthz` for the deployer.** The deployer's Access token is refused
  every other path by the Worker itself. `/healthz` answers 503 unless D1 and
  `MainRun` answer and the version is known.
- **What remains:** §5, `pigeon` from `builds/`, then §6.

### 2026-11-13: A deployed head never deploys again unasked, and a failed deploy says why

- **On 2026-11-13 `5c5ba52` deployed and passed (event 10243).** Six minutes
  later, as !320's merge pushed `fab2ad6`, a second deploy of `5c5ba52` ran
  and failed with no reason (event 10304), which paused Auto window 1. No
  `deploy_retry` was asked.
  - `Pipeline::start` refuses the deployed head, but the alarm starts
    whatever the `starting` slot holds without asking it.
  - The likely source of the stale slot is a write across the Worker's own
    redeploy.
- **The alarm drops a `starting` head that is already `deployed`,** and starts
  the newest push instead. `deploy_retry` still redeploys, since it clears
  `deployed` first.
- **A failed deploy always records a reason:** the report's first violation,
  else its log's last non-empty line. The `deploy.failed` event shows it.
