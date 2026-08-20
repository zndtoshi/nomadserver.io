# Multi-stage build for nomad-server.
# Produces a slim runtime image suitable for Umbrel (linux/amd64 + arm64)
# and plain Docker deployments. Build context is this directory (server/).

FROM rust:1.97-bookworm AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock* ./
COPY src ./src
RUN cargo build --release --locked 2>/dev/null || cargo build --release \
    && strip target/release/nomad-server

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/nomad-server /usr/local/bin/nomad-server

# All mutable state lives under /data (bind-mounted in deployments).
# Runs fine as an arbitrary UID (Umbrel sets user: "1000:1000").
ENV NOMAD_DATA_DIR=/data
VOLUME /data
EXPOSE 3829
ENTRYPOINT ["/usr/local/bin/nomad-server"]
