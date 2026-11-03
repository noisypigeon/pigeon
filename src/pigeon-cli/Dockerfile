# Builds `pigeon` with only FEATURES; build with `--target "$FEATURES"`.
# See .claude/decisions/2026-11-02-release-crate-units-and-images.md
# and .claude/decisions/2026-11-03-release-cobot-image-in-workflows.md.
FROM docker.io/library/rust:1.99.1-slim-bookworm@sha256:24707e70a4f1e23d9a7b25202ec43126d28a52294b96aad5c670df1fd1f1d400 AS build
ARG FEATURES
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p pigeon --no-default-features --features "$FEATURES"

FROM gcr.io/distroless/cc-debian12:nonroot@sha256:9dac0a79194e45a7da0158a9c6da57b217585af0786db3845d1f0ec1a0dd182f AS runtime
COPY --from=build /src/target/release/pigeon /usr/local/bin/pigeon
ENTRYPOINT ["/usr/local/bin/pigeon"]

FROM runtime AS sentinel

FROM runtime AS gantry

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

FROM docker.io/library/debian:bookworm-20261005-slim@sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587 AS cobot
ARG TOFU_VERSION=1.13.1
ARG ZOLA_VERSION=0.23.6
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates git \
 && rm -rf /var/lib/apt/lists/* \
 && git config --system --add safe.directory '*'
COPY --from=tools /out/ /usr/local/bin/
COPY --from=build /src/target/release/pigeon /usr/local/bin/pigeon
RUN tofu version | grep -qF "OpenTofu v${TOFU_VERSION}" \
 && zola --version | grep -qF "zola ${ZOLA_VERSION}"
ENV HOME=/tmp
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/pigeon"]
