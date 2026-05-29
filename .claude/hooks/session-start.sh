#!/bin/bash
# SessionStart hook — make the Rust workspace buildable in Claude Code on the web.
#
# `ort`/`ort-sys` (ONNX Runtime; a transitive dependency of the whole workspace
# via birdnet-core) downloads a prebuilt binary with a bundled rustls client
# that does NOT trust the web sandbox's TLS-intercepting proxy CA, so a cold
# `cargo build` / `cargo test` / `cargo clippy` fails with:
#     ort-sys failed to download prebuilt binaries ...: invalid peer certificate: UnknownIssuer
# scripts/setup-onnxruntime.sh seeds the cache ort-sys checks *before*
# downloading (using curl, which trusts the proxy via the system CA store);
# once seeded, ort-sys skips the network entirely. See docs/RELEASE_PUNCHLIST.md
# (§0 "ONNX Runtime offline note") and scripts/setup-onnxruntime.sh.
#
# Non-fatal by design: nothing here may block the session. A genuine failure is
# surfaced later by `cargo build` with a clear message. Logs go to stderr so
# stdout stays clean for the hook protocol.
set -uo pipefail

# The proxy/cert issue only exists in the remote (Claude Code on the web)
# environment; a local `cargo build` downloads ONNX Runtime normally.
if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

cd "${CLAUDE_PROJECT_DIR:-.}" || exit 0

# Populate registry/cache so the seeder can read ort-sys's dist.txt (it reads
# the extracted source if present, else falls back to the downloaded .crate).
# crates.io is reachable through the proxy, so this succeeds where ort-sys's
# own download client does not.
echo "[session-start] cargo fetch ..." >&2
cargo fetch >/dev/null 2>&1 || echo "[session-start] cargo fetch failed (continuing)" >&2

echo "[session-start] seeding ONNX Runtime for offline ort-sys build ..." >&2
if bash scripts/setup-onnxruntime.sh >&2; then
  echo "[session-start] ONNX Runtime ready." >&2
else
  echo "[session-start] ONNX Runtime seeding failed (continuing; cargo build will report details)." >&2
fi

exit 0
