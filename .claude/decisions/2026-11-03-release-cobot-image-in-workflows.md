## A released cobot image for the cobot workflows

* Date: 2026-11-03
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: release
* Governs: `src/pigeon-cli/Dockerfile`, `src/pigeon-cli/.dockerignore`, `src/pigeon-cli/crates/cobot/tests/image_tools.rs`, `mise.toml`

## Context

[`2026-11-02-release-crate-units-and-images.md`](2026-11-02-release-crate-units-and-images.md) makes `sentinel`, `gantry` and `cobot` release units, each with its own version and `<unit>/vX.Y.Z` tag. Only `sentinel` and `gantry` set an `image`, so only they ship to GHCR, and `lifecycle.yml` runs `pigeon-sentinel:latest` instead of compiling.

`cobot` has releases but no image. Every workflow that runs it compiles it first, with Mise and `cargo build`:

- `cobot.yml`: `plan`, from a `main` checkout, and `apply`, which also runs `sites publish`;
- `sites.yml`: `build`;
- `release.yml`: `release` (`cobot release`) and `pending` (`cobot tags`).

Each run pays for a Rust toolchain and a cold build of the `cobot` feature, and the infrastructure that runs is whatever source happened to be checked out rather than a released version.

This ADR gives `cobot` an image, built and published the way the other two are, and moves those workflows onto it.

Unlike sentinel, cobot starts processes: `tofu` (`cobot/src/tofu.rs`, and `cobot lock` in `cobot/src/lib.rs`), `zola`, `git` and `tar` (`cobot/src/sites.rs`). Distroless has none of them, so cobot's image has its own runtime stage.

**The release-first loop now covers cobot.** CI plans and applies with the latest released cobot, not the PR's source. So an infrastructure change that needs an unreleased cobot feature cannot land with that feature. The engine change merges first, the bot releases it, and a later PR uses it. `rust.yml` keeps testing each PR's own cobot source.

Out of scope, for later amendments or ADRs:

- baking providers into the image. They are still downloaded each run, pinned by `main`'s `.config/providers.lock.hcl`;
- multi-architecture images, signing and provenance attestation;
- `pigeon cobot lock`, which stays a local command;
- the `release` job of `release.yml`, which keeps building from source (§4).

## Implementation plan.

### 1. `cobot` gets an image

In `.config/releases.yaml`, the `cobot` unit gains:

- `image: ghcr.io/noisypigeon/pigeon-cobot`;
- `src/pigeon-cli/Dockerfile` and `src/pigeon-cli/.dockerignore` in its `paths`, as the other imaged units have.

`image` is already a known key, so neither the config schema nor `cobot tags` changes. Its rows now carry the image, so `release.yml`'s `image` job builds and pushes it.

### 2. One Dockerfile, a stage per unit

`src/pigeon-cli/Dockerfile` keeps its `build` stage and gains named final stages, one per feature:

```dockerfile
FROM docker.io/library/rust:1.99.1-slim-bookworm@sha256:<digest> AS build
ARG FEATURES
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p pigeon --no-default-features --features "$FEATURES"

FROM gcr.io/distroless/cc-debian12:nonroot@sha256:<digest> AS runtime
COPY --from=build /src/target/release/pigeon /usr/local/bin/pigeon
ENTRYPOINT ["/usr/local/bin/pigeon"]

FROM runtime AS sentinel
FROM runtime AS gantry

FROM docker.io/library/debian:bookworm-slim@sha256:<digest> AS tools
ARG TOFU_VERSION=1.13.1
ARG TOFU_SHA256=<sha256>
ARG ZOLA_VERSION=0.23.6
ARG ZOLA_SHA256=<sha256>
# curl, fetch both release tarballs from GitHub, check each against its sha256, unpack into /out.

FROM docker.io/library/debian:bookworm-slim@sha256:<digest> AS cobot
# apt: ca-certificates and git, --no-install-recommends, lists removed.
# git config --system --add safe.directory '*'
COPY --from=tools /out/ /usr/local/bin/
COPY --from=build /src/target/release/pigeon /usr/local/bin/pigeon
# RUN tofu version and zola --version, failing unless each prints its ARG's version.
ENV HOME=/tmp
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/pigeon"]
```

- A stage's name is its feature's name, so a build selects it with `--target "$FEATURES"`. Nothing new is needed in `tags`' JSON.
- `runtime` is the image `sentinel` and `gantry` ship today. Their images do not change.
- `tools` keeps `curl` out of the shipped image.
- `safe.directory` lets `git` read a mounted checkout owned by another user.
- `HOME` is writable for `tofu`'s per-run data, under any `--user`.
- Every base is pinned by digest, each at least two weeks old. The Debian base matches the Rust image's `bookworm`.
- The Dockerfile is in every imaged unit's `paths`, so the PR that adds these stages releases all three units, once.

### 3. Tool versions match Mise

- The Dockerfile's `TOFU_VERSION` and `ZOLA_VERSION` must equal `mise.toml`'s `opentofu` and `zola`.
- A new test, `crates/cobot/tests/image_tools.rs`, reads both files and fails when they disagree. `rust.yml` already runs when `mise.toml` or anything under `src/pigeon-cli/` changes.
- Bumping a tool in Mise therefore bumps the Dockerfile too, and that releases the images. The `settings` list in `.config/releases.yaml` is unchanged.
- The check lives in a test, not a workflow, because `.config/policies/terraform-actions.yaml` keeps the word `tofu` out of every workflow but `cobot.yml`.

### 4. `release.yml`

- `image` builds with `--target "$FEATURES"`. Its smoke test, `docker run --rm "$IMAGE:$VERSION" "$FEATURES" --version`, is unchanged.
- `pending` runs `cobot tags` from `ghcr.io/noisypigeon/pigeon-cobot:latest` instead of `cargo run`. It gains `packages: read` and a `docker login ghcr.io`, and Mise leaves it.
- `release` keeps compiling from source:
  - it needs `cargo tree` and `cargo update`, which are not in the image;
  - it is the path that releases cobot itself, so a broken image can always be replaced by the next release.

### 5. `cobot.yml` and `sites.yml` run the image

Mise and `cargo build` leave both workflows. Each job that ran `src/pigeon-cli/target/debug/pigeon`:

1. adds `packages: read` to its permissions;
2. runs `docker login ghcr.io`, with the password read from stdin;
3. runs the same subcommand and flags as today, through:

   ```
   docker run --rm --user "$(id -u):$(id -g)" \
     -v "$PWD:/repo" -v "$RUNNER_TEMP:$RUNNER_TEMP" -w /repo \
     -e COBOT_STATE_PASSPHRASE -e COBOT_SECRETS_PASSPHRASE -e COBOT_WORKLOAD_SECRETS_PASSPHRASE \
     ghcr.io/noisypigeon/pigeon-cobot:latest cobot <subcommand> ...
   ```

   with the passphrases set through `env:`, and passed only to the steps that pass them today.

- **`plan`** still checks out `main` at the root, `.state` read-only and the PR as data in `.pr`. The working directory stays `main`'s, so `./.config/providers.*` are still `main`'s. The binary is no longer built from `main`; it is the released image, which the bot built from a tag on `main`.
- **`apply`** runs `cobot apply` and `cobot sites publish` from the image. `--user` leaves the state files owned by the runner, so the commit step is unchanged and runs on the runner's `git`.
- **`sites.yml`** runs `cobot sites build --root /repo`. Its checkout keeps `fetch-depth: 0` and `fetch-tags: true`, since sites are built from template tags.
- Both workflows' header comments cite this ADR.

### 6. Docs

`CLAUDE.md` says:

- `cobot` also ships an image, `ghcr.io/noisypigeon/pigeon-cobot`, run by `cobot.yml`, `sites.yml` and `release.yml`'s `pending`;
- the release-first loop covers cobot: an infrastructure or workload change that needs an unreleased cobot feature waits for that feature's release.

### 7. Execution

1. **PR A** holds this ADR, titled `docs(adr): a released cobot image for the cobot workflows`.
2. **PR B**, titled `feat(cobot): release a pigeon-cobot image`, holds §1–§3, §4's `image` change and §6.
   - `rust.yml`'s `image` matrix gains `cobot` and builds each entry with `--target`.
   - Its merge releases `sentinel`, `gantry` and `cobot`, and pushes the first `pigeon-cobot` image.
3. **PR C**, titled `ci: run cobot workflows from the pigeon-cobot image`, holds §4's `pending` change and §5. It touches only `.github/`, so its merge releases nothing.

### 8. Verification

- `mise run ci` passes, including the new tool-version test.
- PR B's `rust.yml` builds and smoke-tests all three images.
- After PR B merges, `cobot/vX.Y.Z` has a GitHub Release, and `pigeon-cobot:<version>` and `:latest` exist. In that image, `pigeon cobot --version`, `tofu version`, `zola --version` and `git --version` all run.
- On PR C, the `cobot` plan comment is posted and `sites` builds, both from the image, with no Rust toolchain installed.
- After PR C merges, `apply` is green, its state commit pushes, and the next `release.yml` run finds pending units through the image.
- `sentinel check` passes, including `terraform-actions.yaml`: no workflow but `cobot.yml` names `tofu`.

## Amendments

### 2026-11-03: No safe.directory in the cobot image

A review of #53 flagged §2's `git config --system --add safe.directory '*'`: it turns off git's ownership check for every repository the image reads. It is also not needed. §5 runs the image with `--user "$(id -u):$(id -g)"`, so a mounted checkout's owner already matches the user running git.

- The `cobot` stage sets no `safe.directory`. §2's line, and its bullet explaining it, no longer apply.
- Running the image without `--user`, against a checkout owned by another user, is unsupported: git refuses the checkout, as it should.
- Execution: this amendment lands first, then #53 without the line.
