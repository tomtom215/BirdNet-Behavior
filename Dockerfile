# syntax=docker/dockerfile:1.9
# =============================================================================
# BirdNet-Behavior — Multi-stage Docker build
#
# This Dockerfile builds a single architecture at a time.  Multi-architecture
# manifests are assembled by the CI workflow (.github/workflows/docker.yml)
# which runs this build natively on both amd64 and arm64 runners.  Building
# natively avoids QEMU emulation, which is slow (30-45 min) and unreliable
# for release builds using LTO + codegen-units=1 (emulated linker OOMs on
# standard GitHub-hosted runners).
#
# Build arguments:
#   RUST_VERSION      Rust toolchain version (default: 1.88 — MSRV)
#   DEBIAN_CODENAME   Debian base image codename (default: bookworm)
#   BUILD_FEATURES    Comma-separated Cargo features (default: "")
#                     Pass "analytics" to enable DuckDB behavioral analytics.
#                     ⚠  analytics adds ~7 min of C++ compilation on first
#                        build because libduckdb is statically bundled.
#
# Quick start:
#   docker build -t birdnet-behavior .
#   docker build -t birdnet-behavior --build-arg BUILD_FEATURES=analytics .
# =============================================================================

ARG RUST_VERSION=1.88
ARG DEBIAN_CODENAME=bookworm

# -----------------------------------------------------------------------------
# Stage 1 — cargo-chef base
#
# cargo-chef produces a deterministic "recipe" of workspace dependencies that
# can be compiled in a separate, cache-friendly layer.  Only the recipe.json
# file is copied into the cook stage, so the dependency layer is invalidated
# solely when Cargo.lock or manifest files change — not on every source edit.
# -----------------------------------------------------------------------------
FROM rust:${RUST_VERSION}-slim-${DEBIAN_CODENAME} AS chef
WORKDIR /build
# cargo-chef is pinned to the 0.1 minor series for reproducibility.  The
# --locked flag forces cargo to honour the bundled Cargo.lock from the
# cargo-chef crate, which prevents transitive-dependency drift between builds.
RUN cargo install cargo-chef --locked --version "^0.1"

# -----------------------------------------------------------------------------
# Stage 2 — planner
#
# Reads the full workspace and emits recipe.json.  This stage is cheap: it
# does not compile anything, it only walks the manifest tree.
# -----------------------------------------------------------------------------
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# -----------------------------------------------------------------------------
# Stage 3 — builder
#
# Installs native build dependencies, cooks the cargo-chef recipe (dependency
# compile, cached layer), then compiles the application binary.
#
# System build-time dependencies:
#   cmake + g++     — required by the bundled libduckdb build (analytics)
#   libasound2-dev  — ALSA headers for the audio capture pipeline
#   pkg-config      — locates system libraries during build-script execution
# -----------------------------------------------------------------------------
FROM chef AS builder
ARG BUILD_FEATURES=""

# hadolint ignore=DL3008
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    apt-get update \
    && apt-get install -y --no-install-recommends \
        cmake \
        g++ \
        libasound2-dev \
        pkg-config

# Cook dependencies — this layer is only invalidated when Cargo.lock or a
# workspace manifest changes.  Cook the entire workspace so every
# dependency is pre-compiled; the final `cargo build --bin` below will
# reuse the warm artefacts without rebuilding anything.
COPY --from=planner /build/recipe.json recipe.json
RUN set -eu; \
    if [ -n "${BUILD_FEATURES}" ]; then \
        cargo chef cook --release --recipe-path recipe.json \
            --features "${BUILD_FEATURES}"; \
    else \
        cargo chef cook --release --recipe-path recipe.json; \
    fi

# Compile the application itself.  Dependencies are already warm from the
# previous layer, so this step only compiles workspace crates + the binary.
COPY . .
RUN set -eu; \
    if [ -n "${BUILD_FEATURES}" ]; then \
        cargo build --release --bin birdnet-behavior --features "${BUILD_FEATURES}"; \
    else \
        cargo build --release --bin birdnet-behavior; \
    fi

# Locate and stage the ONNX Runtime shared library that the `ort` crate
# downloaded into the Cargo build tree (download-binaries feature).  We use
# `find ... -not -type l` to get the real file, not a symlink, and fail loudly
# if it is missing — that almost always means the ort-sys build script could
# not download binaries and silently fell back to a stub build.
# hadolint ignore=DL4006
RUN set -eu; \
    lib=$(find target/release -name 'libonnxruntime.so*' ! -type l -print -quit); \
    if [ -z "${lib}" ]; then \
        echo "FATAL: libonnxruntime.so not found in target/release" >&2; \
        echo "       The ort-sys crate likely failed to download binaries." >&2; \
        exit 1; \
    fi; \
    echo "Staging ORT library: ${lib}"; \
    install -D -m 0644 "${lib}" /staging/usr/local/lib/libonnxruntime.so; \
    install -D -m 0755 target/release/birdnet-behavior /staging/usr/local/bin/birdnet-behavior

# -----------------------------------------------------------------------------
# Stage 4 — runtime
#
# Minimal Debian image that contains only the binary, its shared library
# dependencies, and a tiny entrypoint script.  Runs as a dedicated non-root
# user with `audio` group membership for ALSA device access.
#
# Runtime dependencies:
#   libasound2       — ALSA userspace library (audio capture)
#   ca-certificates  — TLS trust store for BirdWeather, Wikipedia, etc.
#   curl             — used by the entrypoint to fetch the BirdNET+ model
#   tini             — PID 1 init, reaps zombies and forwards signals
# -----------------------------------------------------------------------------
FROM debian:${DEBIAN_CODENAME}-slim AS runtime

# Re-declare global ARGs so they are in scope for the ENV / LABEL
# instructions below.  ARGs declared before the first FROM are only
# visible inside a stage if re-declared with another ARG instruction.
ARG DEBIAN_CODENAME
ARG BUILD_FEATURES=""

# Environment knobs
ENV BIRDNET_LISTEN=0.0.0.0:8502 \
    BIRDNET_MODEL_DIR=/data/model \
    RUST_BACKTRACE=1

# Install runtime packages and create a non-root user in a single layer so
# the image stays small and layer metadata is tidy.
# hadolint ignore=DL3008
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    rm -f /etc/apt/apt.conf.d/docker-clean \
    && apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        libasound2 \
        tini \
    && groupadd --system --gid 10001 birdnet \
    && useradd  --system --uid 10001 --gid birdnet \
                --groups audio \
                --home-dir /data --no-create-home \
                --shell /usr/sbin/nologin birdnet

# Copy the built artefacts from the builder stage.
COPY --from=builder /staging/ /
RUN ldconfig

# Entrypoint (model download + exec).
COPY --chmod=0755 docker/entrypoint.sh /usr/local/bin/entrypoint.sh

# Persistent data layout under /data:
#   /data/model        — BirdNET+ ONNX model + labels (downloaded on first run)
#   /data/recordings   — audio segments captured by the detection pipeline
#   /data/cache        — Wikipedia species image cache
#   /data/birdnet.db   — SQLite detections database
#   /data/analytics.db — DuckDB behavioral analytics database (optional)
RUN mkdir -p /data/model /data/recordings /data/cache \
    && chown -R birdnet:birdnet /data

VOLUME ["/data"]
WORKDIR /data

# Web UI default port (override via BIRDNET_LISTEN).
EXPOSE 8502/tcp

# Container health check — hit the web server's health endpoint.  The binary
# exposes /api/v2/health which returns 200 OK when ready to serve requests.
HEALTHCHECK --interval=30s --timeout=5s --start-period=60s --retries=3 \
    CMD curl --fail --silent --show-error --max-time 4 \
             "http://127.0.0.1:${BIRDNET_LISTEN##*:}/api/v2/health" \
        || exit 1

USER birdnet:birdnet

# tini handles PID 1 duties (signal forwarding, zombie reaping) so the Rust
# process gets SIGTERM cleanly for graceful shutdown.
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/entrypoint.sh"]

# OCI image metadata.  CI injects build-time labels via
# docker/metadata-action, but the static ones live here so they are present
# on every build, not only CI-driven ones.
LABEL org.opencontainers.image.title="BirdNet-Behavior" \
      org.opencontainers.image.description="Real-time acoustic bird classification with DuckDB behavioral analytics" \
      org.opencontainers.image.source="https://github.com/tomtom215/BirdNet-Behavior" \
      org.opencontainers.image.licenses="CC-BY-NC-SA-4.0" \
      org.opencontainers.image.base.name="docker.io/library/debian:${DEBIAN_CODENAME}-slim" \
      io.birdnet-behavior.build-features="${BUILD_FEATURES}"
