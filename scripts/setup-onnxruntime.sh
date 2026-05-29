#!/usr/bin/env bash
#
# setup-onnxruntime.sh — seed the ONNX Runtime prebuilt static library into the
# cache that `ort-sys` checks *before* it tries to download, so `cargo build`
# works in environments where ort-sys's bundled TLS client cannot reach the
# pyke CDN.
#
# Why this is needed: `ort` is configured with `download-binaries` + a bundled
# rustls client whose root store does NOT include a TLS-intercepting sandbox
# proxy's CA, so a cold build fails with:
#     ort-sys failed to download prebuilt binaries ...: invalid peer certificate: UnknownIssuer
# `curl` uses the system CA store, which DOES trust the proxy, so we fetch the
# exact same artifact (verified by sha256) and unpack it where ort-sys looks.
#
# ort-sys skips the network entirely when
#   ${ORT_CACHE_DIR:-$HOME/.cache/ort.pyke.io}/dfbin/<target>/<sha256>/
# already exists (see ort-sys build/main.rs). The URL + sha256 + the raw-LZMA2
# tar format all come from ort-sys's own `build/download/dist.txt`, which we
# parse so this keeps working across `ort` version bumps.
#
# Idempotent: re-running is a no-op once the cache is populated.
#
# Usage:
#   scripts/setup-onnxruntime.sh [TARGET]
# TARGET defaults to the host triple (`rustc -vV`); pass e.g.
# aarch64-unknown-linux-gnu when cross-compiling for a Raspberry Pi.

set -euo pipefail

log() { printf '[setup-onnxruntime] %s\n' "$*" >&2; }
die() { printf '[setup-onnxruntime] ERROR: %s\n' "$*" >&2; exit 1; }

TARGET="${1:-$(rustc -vV 2>/dev/null | sed -n 's/^host: //p')}"
[ -n "$TARGET" ] || die "could not determine target triple; pass one explicitly"

# Feature set this repo builds ort with (no cuda/webgpu/rocm/etc.) → "none".
FEATURE_SET="none"

# --- read ort-sys's dist table (source of truth for url + hash + format) ---
# Prefer the extracted registry source; fall back to the downloaded .crate
# tarball. `cargo fetch` populates registry/cache/*.crate but only extracts to
# registry/src lazily at build time, so on a fresh checkout the .crate is the
# only available copy.
CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
read_dist_table() {
  local f cr d dt
  # `|| true` on each glob: an empty glob makes `ls` exit non-zero, which would
  # otherwise trip `set -e` depending on the call site.
  f="$(ls -d "$CARGO_HOME"/registry/src/*/ort-sys-*/build/download/dist.txt 2>/dev/null \
        | sort -V | tail -1 || true)"
  if [ -n "$f" ] && [ -f "$f" ]; then
    cat "$f"; return 0
  fi
  cr="$(ls "$CARGO_HOME"/registry/cache/*/ort-sys-*.crate 2>/dev/null | sort -V | tail -1 || true)"
  [ -n "$cr" ] && [ -f "$cr" ] || return 1
  d="$(mktemp -d)"
  tar -xzf "$cr" -C "$d" 2>/dev/null || { rm -rf "$d"; return 1; }
  dt="$(ls -d "$d"/ort-sys-*/build/download/dist.txt 2>/dev/null | head -1 || true)"
  if [ -n "$dt" ] && [ -f "$dt" ]; then cat "$dt"; rm -rf "$d"; return 0; fi
  rm -rf "$d"; return 1
}

DIST_TABLE="$(read_dist_table)" \
  || die "ort-sys not found under $CARGO_HOME/registry — run 'cargo fetch' first"

# Row format: <feature_set>\t<target>\t<url>\t<sha256>
ROW="$(printf '%s\n' "$DIST_TABLE" | awk -F'\t' -v fs="$FEATURE_SET" -v t="$TARGET" \
  '$1==fs && $2==t {print; exit}')"
[ -n "$ROW" ] || die "no prebuilt ONNX Runtime in dist.txt for '$FEATURE_SET' + '$TARGET'"
URL="$(printf '%s' "$ROW" | cut -f3)"
HASH="$(printf '%s' "$ROW" | cut -f4)"
[ -n "$URL" ] && [ -n "$HASH" ] || die "could not parse url/hash from dist row: $ROW"

CACHE_ROOT="${ORT_CACHE_DIR:-$HOME/.cache/ort.pyke.io}"
DEST="$CACHE_ROOT/dfbin/$TARGET/$HASH"

if [ -e "$DEST/libonnxruntime.a" ] || [ -n "$(ls -A "$DEST" 2>/dev/null || true)" ]; then
  log "already seeded: $DEST — nothing to do"
  exit 0
fi

command -v curl   >/dev/null || die "curl not found"
command -v python3 >/dev/null || die "python3 not found (needed to decompress the raw-LZMA2 tar)"
command -v tar    >/dev/null || die "tar not found"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

log "downloading $URL"
curl -fsSL --retry 3 --max-time 300 -o "$TMP/ort.tar.lzma2" "$URL" \
  || die "download failed (check network/proxy)"

log "verifying sha256 ($HASH)"
echo "$HASH  $TMP/ort.tar.lzma2" | sha256sum -c - >/dev/null \
  || die "sha256 mismatch — refusing to seed a corrupt/tampered artifact"

log "decompressing raw LZMA2 → tar"
python3 - "$TMP/ort.tar.lzma2" "$TMP/ort.tar" <<'PY' || die "LZMA2 decompression failed"
import lzma, sys
src, dst = sys.argv[1], sys.argv[2]
# ort-sys reads the stream with Lzma2Reader::new(.., 1 << 26, None): raw LZMA2,
# 64 MiB dict, no props byte. Mirror that here.
dec = lzma.LZMADecompressor(format=lzma.FORMAT_RAW,
                            filters=[{"id": lzma.FILTER_LZMA2, "dict_size": 1 << 26}])
with open(src, "rb") as f:
    data = dec.decompress(f.read())
with open(dst, "wb") as f:
    f.write(data)
PY

log "unpacking into $DEST"
mkdir -p "$DEST"
tar -xf "$TMP/ort.tar" -C "$DEST"

[ -e "$DEST/libonnxruntime.a" ] || \
  die "unpacked archive did not contain libonnxruntime.a (got: $(ls -A "$DEST" 2>/dev/null))"

log "done — ort-sys will now link the cached ONNX Runtime without hitting the network"
