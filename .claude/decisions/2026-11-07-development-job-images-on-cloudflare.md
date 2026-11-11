## Job images: a login that outlasts the push, a smaller image, `pigeon` from builds, and images built on Cloudflare

* Date: 2026-11-07
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development
* Governs: `src/pigeon-cli/Dockerfile`, `src/pigeon-cli/pigeon-start`, `src/pigeon-cli/crates/backplane/src/cobot/imagebuild.rs`, `src/pigeon-cli/crates/backplane/src/cobot/workers.rs`, `src/pigeon-cli/crates/pigeon/src/commands/workers.rs`, `src/pigeon-cli/crates/backplane/src/ship.rs`

## Context

[`2026-11-07-development-control-plane-deploys-itself.md`](2026-11-07-development-control-plane-deploys-itself.md)
made Worker code deploy itself: `!26` to `!28`, the first proven by `!28`'s
merge, which went live with no local step. The job image is still built and
pushed from the owner's machine, and its first push showed three problems.

- **The push outlives its login.**
  - `pigeon cobot workers images push` pushes with `wrangler containers push`,
    whose registry login lasts 15 minutes.
  - The `Checker` image's upload took about 35 minutes, and the registry
    answered `unauthorized` near its end.
  - The owner finished by hand: a 4-hour login from
    `wrangler containers registries credentials --push --expiration-minutes 240`,
    `docker login`, `docker tag` and `docker push`, then `DEPLOY_IMAGES`
    written with `printf`.
- **The image is several GB, and each push uploads nearly all of it again.**
  - The `ci` stage's warm-up begins with `COPY . src/pigeon-cli`, then runs
    workspace clippy, `cargo test --no-run` (about 40 linked test binaries),
    wasm clippy and a release wasm build.
  - Any source change rebuilds that whole layer, so every push uploads the
    whole cache again.
  - The same build on a development machine fills 26 GB, almost all of it
    `target/debug/deps`.
- **The deploys ADR's §5 and §6 are unbuilt.**
  - Jobs run the `pigeon` baked into the image, so a change to job-running
    code needs a new image. That's why `scope::IMAGE_BLOCKS` lists the job
    protocol's files, and why `IMAGE_LAGS` exists.
  - Images are still built on the owner's Mac, under emulation.

What the design rests on:

- **A job Container's command and environment are given when it starts.**
  `cobot::worker::start_raw` passes an `entrypoint` array and an `env` object.
- **The deploy job holds `WORKERS_DEPLOY_TOKEN`, with R2 Storage Read and
  Write.** cobot already derives S3 credentials from an account token for
  `sites publish`.
- **The registry mints push credentials with an expiry in minutes,** through
  the API that `wrangler containers registries credentials` calls.
- **`pigeon` holds no secret.** It's built from this public repository, so a
  Container may download it from a public path, provided it checks a SHA-256
  the Worker gave it.

Decided with the owner: the image keeps a warm cache of third-party
dependencies only, in a layer built from a recipe that changes only when
dependencies do.

## Implementation plan.

### 1. `images push` logs in for as long as the push takes

- **cobot mints the registry credentials** with the account token, from the
  endpoint `wrangler containers registries credentials` uses.
  - They carry push and pull and last 240 minutes, changeable with
    `--expiration-minutes`.
  - Implementation confirms the endpoint's path and fields against wrangler.
- **It then drives Docker itself:**
  - `docker login registry.cloudflare.com` with the password on stdin, never in
    argv;
  - `docker tag` and `docker push` for each class;
  - `docker logout`.

  The password is scrubbed from every output.
- **The output is the `DEPLOY_IMAGES` JSON, as now:** `source` from
  `git rev-parse HEAD`, and `images` keyed `<worker>/<Class>`.
- **It refuses a checkout with uncommitted changes.** It warns when `HEAD` is
  not an ancestor of the fetched `main`: a squash merge leaves a change
  branch's commit outside `main`'s history, and the deploy job can't diff from
  such a commit.

### 2. A smaller image: a warm layer of dependencies only

- **The `ci` stage builds a recipe in a layer of its own.** The recipe is the
  workspace's `Cargo.toml` and `Cargo.lock`, each crate's `Cargo.toml`, and
  stub `src/lib.rs` and `src/main.rs`. This is cargo-chef's idea, without the
  tool.
- **It builds:**
  - `cargo build` and `cargo clippy`, for the workspace and all targets, in
    dev;
  - `cargo build --release -p pigeon --no-default-features --features backplane`,
    for §3;
  - the Worker crates' wasm release build, for `worker-build`.
- **It keeps only the dependencies.** It removes the stubs' own artifacts
  (`cargo clean -p` for each workspace crate) and their incremental and
  fingerprint directories. It links no test binary.
- **The layer changes only with the recipe,** that is, with dependencies or
  the toolchain. A source-only image rebuild reuses it, and its push skips it.
- **Checks still compile the workspace's own crates,** but none of their 400
  or so dependencies.
- **Sizes are measured, not guessed.** The change that makes this records
  `docker history` before and after. The aim is about 1 GB for the cache
  layer.

### 3. `pigeon` from builds (replaces the deploys ADR's §5)

- **The deploy job builds and stores `pigeon`.** On each push to `main`, after
  its sites and Workers:
  - it builds
    `cargo build --release -p pigeon --no-default-features --features backplane`,
    warm from §2;
  - it uploads the binary to the CI bucket at `builds/<sha>/pigeon`, with S3
    credentials derived from `WORKERS_DEPLOY_TOKEN`;
  - it reports the binary's SHA-256. `MainRun` records it as `latest_build`,
    shown in `deploy_status`.
- **The Worker serves `GET /builds/<sha>/pigeon` from R2,** ahead of its
  Access check.
  - An Access application on `/builds/*` with a Bypass policy opens the path.
  - Responses are immutable and keyed by commit.
- **A job Container starts from the newest build.** The Worker starts it with
  `PIGEON_BUILD_URL` and `PIGEON_BUILD_SHA256` for `latest_build`, and the
  entrypoint `/usr/local/bin/pigeon-start`, a small script baked into the
  image:
  - it downloads the build, checks its SHA-256 and runs it;
  - it runs the image's `pigeon` instead when there is no build, the download
    fails or the checksum differs, and logs why.
- **So the job server, the bound job, merges and deploys all run `main`'s
  newest code.** Checks run `cargo` in their checkout as before.
- **`scope::IMAGE_BLOCKS` shrinks to the image's tools:** the Dockerfile,
  `mise.toml` and the toolchain. `IMAGE_LAGS` goes, because job code reaches
  jobs with each merge.

### 4. Images built on Cloudflare (replaces the deploys ADR's §6)

- **A spike first.**
  - **The first candidate** is rootless BuildKit with
    `--oci-worker-no-process-sandbox`, run as `buildctl-daemonless.sh`,
    checksum-pinned from BuildKit's release in a builder stage.
  - **The second** is a maintained fork of kaniko.
  - **The spike** is a one-off build operation on `Checker`. It reports
    whether the `ci` target builds and pushes, how long that takes, and the
    disk it needed.
- **If the spike works, an image job comes first when one is needed.** When
  `scope::image_inputs` blocks a deploy, `MainRun` first runs an image job on
  `Checker`:
  - it builds the `ci` target natively for `linux/amd64`;
  - it pushes with registry credentials `MainRun` mints for that job alone;
  - it reports the references.
- **Images move into `MainRun`'s storage, as `images`,** amending the deploys
  ADR's §4. The deploy job takes them from there, and their container rollout
  follows the Worker's.
  - `DEPLOY_IMAGES` stays as the bootstrap, and as the break-glass value: the
    first deploy after it changes seeds the storage.
- **If the spike fails, image builds stay the owner's.** §1 and §2 make them
  one command and an upload of about a GB.

### Credentials

- **No new long-lived secret.** S3 and registry credentials are derived or
  minted per job from `WORKERS_DEPLOY_TOKEN`, and scrubbed from every log.
- **`/builds/` is public by design.** What it serves is public code, and each
  Container checks the SHA-256 its Worker gave it.

### Execution

Each step is its own change, in this order:

1. This ADR, with a `CLAUDE.md` pointer.
2. `images push` with minted credentials and Docker, and its checks on the
   checkout.
3. The dependencies-only layer, and `pigeon-start`, in the `ci` stage. The
   owner pushes the image once with step 2's command, and the change records
   the sizes.
4. `pigeon` from builds:
   - the deploy job builds and uploads it;
   - the Worker serves `/builds/`;
   - `MainRun` keeps `latest_build`;
   - jobs start through `pigeon-start`;
   - `IMAGE_BLOCKS` shrinks.

   The owner adds the `/builds/*` Access application.
5. The BuildKit spike.
6. The image job in `MainRun`, with images in its storage, or an amendment
   recording the spike's failure.

### Verification

- **`images push` survives a long upload.** An upload longer than 15 minutes
  completes and sets `DEPLOY_IMAGES` with no manual step.
- **A source-only image rebuild reuses the dependency layer:** its push says
  "Layer already exists" for it, and takes minutes.
- **A fix to `jobs.rs` runs in the next change's jobs with no image push,**
  and `pigeon-start`'s log names the build it ran.
- **A bad build is refused safely:** with a missing or altered build, the
  Container runs the image's `pigeon`, and its log says why.
- **After the spike works:** a Dockerfile change is built, pushed and rolled
  out by the pipeline alone.

## Amendments

### 2026-11-07: Steps 2 to 4 landed; the measured sizes; how `pigeon` from builds starts

- **Sizes** (Docker Desktop's content size, compressed; `docker history` shows 0B
  for every layer on its containerd store, so the cache layer alone wasn't
  measured):

  | `Checker` image | Size |
  |---|---|
  | Before Node and wrangler (`!25`) | about 1.24 GB |
  | The whole-source warm-up (`!26` to `!30`) | 1.75 GB |
  | The dependencies-only warm-up (`!31`) | 1.28 GB |

- **`images push` retries a dropped upload** (`!32`). The first push after
  `!30` failed on a closed connection to Docker Desktop's proxy, not on its
  login. A failed `docker push` is retried after 10 s and 30 s, skipping the
  layers already pushed.
- **How §3 starts:**
  - The deploy job builds `pigeon` only when something under `src/pigeon-cli/`
    changed (`cobot::scope::builds`); otherwise jobs keep the last build.
  - The build is stored in the bucket bound as `CI_LOGS`, the one the Worker
    serves `/builds/` from, with S3 credentials derived from
    `WORKERS_DEPLOY_TOKEN`. That replaces §3's public `/builds/*` path; see
    below.
  - A Container starts through `pigeon-start` only when `MainRun` has a
    `latest_build`. Before the first build it runs the image's `pigeon`
    directly, so an image without `pigeon-start` still starts.
  - `latest_build` is kept from any deploy whose report names a build of its
    own commit, even when the rest of that deploy failed.
- **Builds are read with an Access service token, not a bypass.** Access's
  Workers application type has no path field, so a Bypass on `/builds/*` would
  open the whole Worker.
  - A dedicated service token, `BUILDS_ACCESS_ID` and
    `BUILDS_ACCESS_SECRET`, is given to every job Container with the build's
    URL. The Worker refuses it every path but `/builds/`, as it refuses the
    deployer's token every path but `/healthz`.
  - `pigeon-start` sends its headers through curl's stdin config, never argv,
    and unsets them before it runs `pigeon`.
  - Without the token, no build is named, and jobs run the image's `pigeon`.
  - It reads only builds of public code, so a check's code holding it gains
    nothing.
  - `pigeon-start` joins `IMAGE_BLOCKS`, since it lives in the image.
- **The change that lands §3 needs one last image push.** It edits `jobs.rs`
  and `control.rs`, which still block a deploy until it is deployed. Its deploy
  then runs the image's older `pigeon`, which builds nothing, so the first build
  comes with the next merge that changes `src/pigeon-cli/`.

### 2026-11-07: The BuildKit spike runs as a check

- **A spike is a check, not a new operation.** `cobot::scope` runs
  `tooling/spikes/<name>/run.sh` as the check `spike <name>` for a change
  touching it. It runs on `Checker`, from the workspace, with no credentials,
  like any check.
- **A spike change is read, then closed, never merged.** Its script prints a
  report and exits non-zero, so the report shows in `change_status`. An
  amendment records the result.
- **The first spike only builds, and doesn't push.** It answers the questions
  a push depends on:
  - whether a Container can run BuildKit at all (its user, user namespaces,
    `newuidmap`, overlay);
  - whether the `ci` target builds;
  - how long that takes, and the disk it needs.

  Pushing waits for a build that works, and then needs registry credentials
  a check never gets.
- **BuildKit and RootlessKit come from their GitHub releases,** pinned by
  version and SHA-256. RootlessKit's matches its published `SHA256SUMS`;
  BuildKit publishes none, so its pin is the checksum measured when it was
  pinned.

### 2026-11-07: R2 bindings name their jurisdiction

- **The first build was stored but never served.**
  - The deploy job uploads through the EU S3 endpoint, into the EU bucket the
    catalog creates.
  - The Worker's `CI_LOGS` binding named no jurisdiction, so it bound a
    default-jurisdiction bucket of the same name, which wrangler had created
    empty on deploy.
  - Every CI log since went there, and `/builds/` found nothing, so jobs fell
    back to the image's `pigeon`.
- **`cobot::workers` renders `"jurisdiction": "eu"` on every R2 binding**
  (`R2_JURISDICTION`), the `cloudflare/r2-bucket` module's default.
- **The auto-created twin keeps only the old logs.** Deleting it is the
  owner's call.

### 2026-11-07: A rendering fix needs the image that carries it; `deploy.retry` redeploys

- **`!38` deployed, and still rendered the old binding.** Rendering runs in
  the deploy job, in whatever `pigeon` its Container started with. Builds
  weren't served yet, so that was the image's `pigeon`, from before the fix.
  A fix that makes builds reachable can't arrive through a build.
- **The loop is broken by one image push** carrying the fix, then a deploy.
- **`deploy.retry` redeploys a passed push.** When the newest push's deploy
  passed, a retry deploys it again from nothing: every site, and every Worker
  with its current images. A deploy since itself would change nothing. A
  failed push is retried from the last deploy that passed, as before.

### 2026-11-07: The spike's first findings; the `ci` image gains rootless BuildKit's prerequisites

- **A `Checker` Container,** from change 40's runs:
  - a micro-VM (`6.18.56-cloudflare-microvm`), with 4 CPUs, 12 GB of memory
    and 15 GB of free disk, running as user 65532;
  - user namespaces allowed (`unshare -Ur` works), overlay available, no
    seccomp, `NoNewPrivs` 0, a full capability bounding set, and setuid
    honoured on the root filesystem;
  - fast pulls from Docker Hub.
- **BuildKit starts, but can't build.**
  - In a one-ID user namespace `buildkitd` answers, but unpacking a base
    image fails: `lchown … /etc/gshadow … GID 42: invalid argument`. An
    image's files belong to more than one user.
  - RootlessKit needs subordinate ID ranges, and the image has none, nor
    `newuidmap`.
- **So the `ci` stage gains, after its warm-up layer:**
  - the `uidmap` package;
  - a `nonroot` entry for 65532 in `/etc/passwd` and `/etc/group`;
  - `nonroot:100000:65536` in `/etc/subuid` and `/etc/subgid`.

  The spike then runs again under RootlessKit, for the build's time and disk.

### 2026-11-07: The spike worked; images are built in the deploy job

- **The spike built the `ci` target in a `Checker` Container,** with the
  image from `!41`:
  - `buildkitd` under RootlessKit, with the overlayfs snapshotter;
  - 497 s in all, 245 s of it the dependency warm-up;
  - BuildKit's store at 6.1 GB, leaving 7.9 GB of the 18 GB free.

  It didn't push, because a check holds no credentials.
- **The build runs inside the deploy job, not in a job of its own** (amends
  §4).
  - The deploy job already runs on `Checker` and holds `WORKERS_DEPLOY_TOKEN`.
    When `scope::image_inputs` names a change since the images' commit, it
    builds and pushes before it deploys, then deploys with the new
    references. Needs image is no longer a wait.
  - BuildKit and RootlessKit are fetched per build, pinned by SHA-256
    (`cobot::imagebuild`).
  - Each distinct image is built once and pushed under every class that
    uses it, as `<worker>-<class>:<sha, 12>`.
  - Registry credentials are minted for that build alone. They reach
    BuildKit only through a Docker config that is removed afterwards, and are
    scrubbed from every output.
- **`MainRun` keeps the newest images,** as `images`, from any deploy's
  report whose images are of its own commit, and passes them to each deploy.
  - `DEPLOY_IMAGES` stays the seed and the break-glass value. A secret that
    differs from the one last seen (`images_seed`) is the owner's push, and
    wins.
  - `deploy_status` shows the images' commit, and not their references,
    which name the account.
- **A rollout lags.** Cloudflare moves Container instances to a new image
  over several minutes, so the jobs started just after a deploy may still run
  the previous image.
