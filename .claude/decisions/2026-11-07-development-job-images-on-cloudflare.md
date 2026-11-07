## Job images: a login that outlasts the push, a smaller image, `pigeon` from builds, and images built on Cloudflare

* Date: 2026-11-07
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development

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
