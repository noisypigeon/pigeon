# The spike's Container: the released cobot image plus the probe; see ../README.md.
FROM docker.io/library/rust:1.99.1-slim-bookworm@sha256:24707e70a4f1e23d9a7b25202ec43126d28a52294b96aad5c670df1fd1f1d400 AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src src
RUN cargo build --release --locked

FROM ghcr.io/noisypigeon/pigeon-cobot:0.1.20
COPY --from=build /src/target/release/pigeon-spike-probe /usr/local/bin/pigeon-spike-probe
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/pigeon-spike-probe"]
