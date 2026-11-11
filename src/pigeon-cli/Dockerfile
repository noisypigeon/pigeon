# Builds `pigeon` with only FEATURES; build with `--target "$FEATURES"`.
# See .claude/decisions/2026-11-02-release-crate-units-and-images.md,
# .claude/decisions/2026-11-03-release-cobot-image-in-workflows.md
# and .claude/decisions/2026-11-03-development-backplane-platform-composition-layer.md.
FROM docker.io/library/rust:1.99.1-slim-bookworm@sha256:24707e70a4f1e23d9a7b25202ec43126d28a52294b96aad5c670df1fd1f1d400 AS build
ARG FEATURES
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p pigeon --no-default-features --features "$FEATURES"

FROM gcr.io/distroless/cc-debian12:nonroot@sha256:9dac0a79194e45a7da0158a9c6da57b217585af0786db3845d1f0ec1a0dd182f AS runtime
COPY --from=build /src/target/release/pigeon /usr/local/bin/pigeon
ENTRYPOINT ["/usr/local/bin/pigeon"]

FROM runtime AS sentinel

# tofu and zola, checksum-pinned; versions equal mise.toml's.
FROM docker.io/library/debian:bookworm-20261005-slim@sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587 AS tools
ARG TOFU_VERSION=1.13.1
ARG TOFU_SHA256=378ada19d4bc70c43732004e8159be771b23b9a5afdf059e5f8a2b3fa2c70a69
ARG ZOLA_VERSION=0.23.6
ARG ZOLA_SHA256=8f5132b3522412d04e395e0b25f6d68613ad272a873e54a2b3ebf664873024a4
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /dl
RUN curl -fsSLo tofu.tar.gz "https://github.com/opentofu/opentofu/releases/download/v${TOFU_VERSION}/tofu_${TOFU_VERSION}_linux_amd64.tar.gz" \
 && curl -fsSLo zola.tar.gz "https://github.com/getzola/zola/releases/download/v${ZOLA_VERSION}/zola-v${ZOLA_VERSION}-x86_64-unknown-linux-gnu.tar.gz" \
 && printf '%s  tofu.tar.gz\n%s  zola.tar.gz\n' "$TOFU_SHA256" "$ZOLA_SHA256" | sha256sum -c - \
 && mkdir /out \
 && tar -xzf tofu.tar.gz -C /out tofu \
 && tar -xzf zola.tar.gz -C /out zola

FROM docker.io/library/debian:bookworm-20261005-slim@sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587 AS backplane
ARG TOFU_VERSION=1.13.1
ARG ZOLA_VERSION=0.23.6
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates git \
 && rm -rf /var/lib/apt/lists/*
COPY --from=tools /out/ /usr/local/bin/
COPY --from=build /src/target/release/pigeon /usr/local/bin/pigeon
RUN tofu version | grep -qF "OpenTofu v${TOFU_VERSION}" \
 && zola --version | grep -qF "zola ${ZOLA_VERSION}"
ENV HOME=/tmp
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/pigeon"]

# The workspace's manifests and lockfile with empty sources: the input of the ci stage's warm
# layer, unchanged unless a manifest or Cargo.lock is
# (.claude/decisions/2026-11-07-development-job-images-on-cloudflare.md §2).
FROM docker.io/library/debian:bookworm-20261005-slim@sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587 AS recipe
WORKDIR /src
COPY . .
RUN mkdir /recipe \
 && cp Cargo.toml Cargo.lock /recipe/ \
 && for m in crates/*/Cargo.toml; do \
      d="$(dirname "$m")"; mkdir -p "/recipe/$d/src"; cp "$m" "/recipe/$d/"; \
      if [ -f "$d/src/lib.rs" ]; then : > "/recipe/$d/src/lib.rs"; fi; \
      if [ -f "$d/src/main.rs" ]; then echo 'fn main() {}' > "/recipe/$d/src/main.rs"; fi; \
    done

# A change's jobs run here (lifecycle ADR §4): the toolchain mise.toml pins, git, backplane's
# tools, and the workspace's dependencies already compiled, so a job builds only the crates
# it touches. Build with `--build-arg FEATURES=backplane`.
FROM docker.io/library/rust:1.99.1-slim-bookworm@sha256:24707e70a4f1e23d9a7b25202ec43126d28a52294b96aad5c670df1fd1f1d400 AS ci
# Node, wrangler and worker-build deploy Workers from the pipeline; versions equal mise.toml's
# (.claude/decisions/2026-11-07-development-control-plane-deploys-itself.md §2).
ARG NODE_VERSION=24.22.0
ARG NODE_SHA256=cc2f601cee82d9a5a233302fe307abc600727035fd7d2c3a3571755989e95008
ARG WRANGLER_VERSION=4.151.3
ARG WORKER_BUILD_VERSION=0.8.7
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl git libssl-dev pkg-config xz-utils \
 && rm -rf /var/lib/apt/lists/* \
 && rustup component add clippy rustfmt \
 && rustup target add wasm32-unknown-unknown \
 && curl -fsSLo /tmp/node.tar.xz "https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-x64.tar.xz" \
 && printf '%s  /tmp/node.tar.xz\n' "$NODE_SHA256" | sha256sum -c - \
 && tar -xJf /tmp/node.tar.xz -C /usr/local --strip-components=1 --exclude='*.md' --exclude=LICENSE \
 && rm /tmp/node.tar.xz \
 && npm install -g "wrangler@${WRANGLER_VERSION}" \
 && cargo install --locked --root /usr/local "worker-build@${WORKER_BUILD_VERSION}"
COPY --from=tools /out/ /usr/local/bin/
COPY --from=build /src/target/release/pigeon /usr/local/bin/pigeon
# No debug info: a job needs pass or fail, not a debugger, and debug info makes linking the
# test binaries need far more memory and the cache far more space.
ENV CARGO_HOME=/cache/cargo CARGO_TARGET_DIR=/cache/target HOME=/tmp \
    CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
# How many crates the warm-up compiles at once; lower it if a builder runs out of memory.
ARG WARM_JOBS=2
COPY pigeon-start /usr/local/bin/pigeon-start
# Only third-party dependencies are warmed, from the recipe, so this layer changes only with
# them; the stubs' own artifacts are removed (job images ADR §2).
WORKDIR /warm
COPY --from=recipe /recipe src/pigeon-cli
RUN export CARGO_BUILD_JOBS="$WARM_JOBS" \
 && cd src/pigeon-cli \
 && members="$(awk '/^\[package\]/{p=1;next} /^\[/{p=0} p && /^name = /{gsub(/"/,"",$3); printf "-p %s ", $3}' crates/*/Cargo.toml)" \
 && cargo fetch --locked \
 && cargo build --workspace --all-targets --locked \
 && cargo clippy --workspace --all-targets --locked \
 && cargo build --release --locked -p pigeon --no-default-features --features backplane \
 && cargo build --release --locked -p pigeon-backplane --lib --no-default-features --features worker --target wasm32-unknown-unknown \
 && cargo clean $members \
 && cargo clean --release $members \
 && cargo clean --release --target wasm32-unknown-unknown $members \
 && cd / && rm -rf /warm \
 && mkdir -p /tmp/.cache \
 && chown -R 65532:65532 /cache /tmp/.cache
# Rootless BuildKit for image jobs: the job user by name, setuid newuidmap and newgidmap, and its
# subordinate IDs (job images ADR §4, spike amendment). After the warm-up, so that layer stays.
RUN apt-get update \
 && apt-get install -y --no-install-recommends uidmap \
 && rm -rf /var/lib/apt/lists/* \
 && echo 'nonroot:x:65532:65532:nonroot:/tmp:/usr/sbin/nologin' >> /etc/passwd \
 && echo 'nonroot:x:65532:' >> /etc/group \
 && echo 'nonroot:100000:65536' >> /etc/subuid \
 && echo 'nonroot:100000:65536' >> /etc/subgid
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/pigeon"]
