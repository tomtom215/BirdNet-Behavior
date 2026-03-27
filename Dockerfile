# syntax=docker/dockerfile:1
# =============================================================================
# BirdNet-Behavior — Multi-stage Docker build
#
# Supported platforms (via docker buildx):
#   linux/amd64   — x86_64 servers and desktops
#   linux/arm64   — Raspberry Pi 4 / 5 (64-bit OS)
#   linux/arm/v7  — Raspberry Pi 3 / 32-bit OS  ⚠ see note below
#
# ⚠  linux/arm/v7: ONNX Runtime does not ship prebuilt armv7 binaries.
#    Building for armv7 requires ORT_STRATEGY=compile (adds ~30 min).
#    For Raspberry Pi 3, prefer the native binary from GitHub Releases.
#
# Build arguments:
#   RUST_VERSION      Rust toolchain version (default: 1.88)
#   DEBIAN_CODENAME   Base image codename (default: bookworm)
#   BUILD_FEATURES    Comma-separated Cargo features (default: "")
#                     Pass "analytics" to enable DuckDB behavioral analytics
#                     ⚠ analytics adds ~7 min C++ compile for bundled libduckdb
#
# Quick start:
#   docker build -t birdnet-behavior .
#   docker build -t birdnet-behavior --build-arg BUILD_FEATURES=analytics .
# =============================================================================

ARG RUST_VERSION=1.88
ARG DEBIAN_CODENAME=bookworm

# -----------------------------------------------------------------------------
# Stage 1 — cargo-chef base
# Installs the cargo-chef tool used for deterministic dependency caching.
# -----------------------------------------------------------------------------
FROM rust:${RUST_VERSION}-slim-${DEBIAN_CODENAME} AS chef
RUN cargo install cargo-chef --locked
WORKDIR /build

# -----------------------------------------------------------------------------
# Stage 2 — dependency planner
# Reads the workspace Cargo.toml/Cargo.lock and produces a recipe.json that
# fingerprints the exact dependency set.  Only re-runs when Cargo files change.
# -----------------------------------------------------------------------------
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# -----------------------------------------------------------------------------
# Stage 3 — builder
# Compiles dependencies first (cached layer), then the application binary.
#
# System build-time dependencies:
#   cmake + g++      — bundled libduckdb C++ compilation (analytics feature)
#   libasound2-dev   — ALSA headers for audio capture
#   pkg-config       — helps locate system libraries during build
# -----------------------------------------------------------------------------
FROM chef AS builder
ARG BUILD_FEATURES=""

RUN apt-get update && apt-get install -y --no-install-recommends \
        cmake \
        g++ \
        libasound2-dev \
        pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Cook dependencies only — this layer is invalidated only when Cargo.lock changes
COPY --from=planner /build/recipe.json recipe.json
RUN if [ -n "${BUILD_FEATURES}" ]; then \
        cargo chef cook --release --recipe-path recipe.json --features "${BUILD_FEATURES}"; \
    else \
        cargo chef cook --release --recipe-path recipe.json; \
    fi

# Build the application
COPY . .
RUN if [ -n "${BUILD_FEATURES}" ]; then \
        cargo build --release --bin birdnet-behavior --features "${BUILD_FEATURES}"; \
    else \
        cargo build --release --bin birdnet-behavior; \
    fi

# Locate and stage the ONNX Runtime shared library.
# The `ort` crate (download-binaries feature) downloads a prebuilt libonnxruntime.so
# into the Cargo build tree; we copy it to a predictable path for the next stage.
RUN set -e; \
    lib=$(find /build/target -name "libonnxruntime.so" ! -type l 2>/dev/null | head -1); \
    if [ -z "$lib" ]; then \
        echo "ERROR: libonnxruntime.so not found — ort download may have failed" >&2; \
        exit 1; \
    fi; \
    echo "Found ORT library: $lib"; \
    cp "$lib" /libonnxruntime.so

# -----------------------------------------------------------------------------
# Stage 4 — runtime image
# Minimal Debian image containing only what is needed to run the binary.
#
# Runtime dependencies:
#   libasound2      — ALSA userspace library for audio capture
#   ca-certificates — TLS roots for BirdWeather / Wikipedia / HTTPS integrations
#   curl            — used by entrypoint to download the BirdNET+ model
# -----------------------------------------------------------------------------
FROM debian:${DEBIAN_CODENAME}-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
        libasound2 \
        ca-certificates \
        curl \
    && rm -rf /var/lib/apt/lists/*

# Dedicated non-root user; member of `audio` for ALSA device access
RUN groupadd --system birdnet \
    && useradd --system --gid birdnet --groups audio \
       --home-dir /data --no-create-home birdnet

# Application binary
COPY --from=builder /build/target/release/birdnet-behavior /usr/local/bin/birdnet-behavior

# ONNX Runtime shared library
COPY --from=builder /libonnxruntime.so /usr/local/lib/libonnxruntime.so
RUN ldconfig

# Entrypoint script (model download + exec)
COPY docker/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh

# Persistent data layout under /data:
#   /data/model       — BirdNET+ ONNX model + labels (auto-downloaded on first run)
#   /data/recordings  — audio segments from the capture pipeline
#   /data/cache       — Wikipedia species image cache
#   /data/birdnet.db  — SQLite detections database
#   /data/analytics.db — DuckDB behavioral analytics (optional)
RUN mkdir -p /data/model /data/recordings /data/cache \
    && chown -R birdnet:birdnet /data

VOLUME ["/data"]
WORKDIR /data

# Web UI default port (override with BIRDNET_LISTEN env var)
EXPOSE 8502

USER birdnet
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
