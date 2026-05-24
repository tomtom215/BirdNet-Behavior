#!/usr/bin/env bash
# Async macOS (Apple Silicon) verification runbook for BirdNet-Behavior.
#
# Run this on a real M-series Mac to confirm the parts that cannot be checked
# on the Linux CI/dev sandbox: the from-source build, the preflight doctor, a
# real --web-only boot, and the avfoundation microphone enumeration. It also
# prints the manual steps (TCC mic consent, the launchd LaunchAgent) that need
# a human in front of the machine.
#
#   curl -fsSLO https://raw.githubusercontent.com/tomtom215/BirdNet-Behavior/main/packaging/macos/verify-macos.sh
#   bash verify-macos.sh                # build + boot + report
#   SKIP_BUILD=1 bash verify-macos.sh   # reuse an existing target/release build
#
# It is read-only with respect to your system: it builds in the repo, boots a
# throwaway server against a temp DB on an unused port, and cleans up after
# itself. It installs nothing and changes no system settings.
set -euo pipefail

PORT="${PORT:-8599}"
PASS=0 FAIL=0 WARN=0
ok()   { printf '  \033[32mok\033[0m   %s\n' "$1"; PASS=$((PASS+1)); }
bad()  { printf '  \033[31mFAIL\033[0m %s\n' "$1"; FAIL=$((FAIL+1)); }
warn() { printf '  \033[33mwarn\033[0m %s\n' "$1"; WARN=$((WARN+1)); }
hdr()  { printf '\n\033[1m== %s ==\033[0m\n' "$1"; }

hdr "Platform"
OS="$(uname -s)"; ARCH="$(uname -m)"
if [ "$OS" = "Darwin" ]; then ok "macOS ($OS)"; else bad "not macOS (uname=$OS) — this script targets Apple Silicon"; fi
if [ "$ARCH" = "arm64" ]; then ok "Apple Silicon (arm64)"; else warn "arch is $ARCH, not arm64 — ort ships an aarch64-apple-darwin prebuilt; x86_64 Macs are untested"; fi

hdr "Toolchain & dependencies"
for tool in cargo rustc cmake ffmpeg; do
  if command -v "$tool" >/dev/null 2>&1; then ok "$tool: $(command -v "$tool")"; else
    bad "$tool not found"
    [ "$tool" = "cmake" ] && echo "       cmake builds the bundled libduckdb:  brew install cmake"
    [ "$tool" = "ffmpeg" ] && echo "       ffmpeg captures the mic (avfoundation): brew install ffmpeg"
  fi
done

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
echo "  repo: $ROOT"

hdr "Build (cargo build --release --features analytics)"
BIN="$ROOT/target/release/birdnet-behavior"
if [ "${SKIP_BUILD:-0}" = "1" ] && [ -x "$BIN" ]; then
  warn "SKIP_BUILD=1 — reusing existing $BIN"
else
  echo "  building (first build downloads the ONNX Runtime prebuilt + compiles libduckdb; ~10-15 min)…"
  if cargo build --release --features analytics; then ok "release build succeeded"; else bad "release build failed — see output above"; fi
fi
[ -x "$BIN" ] && ok "binary present: $BIN" || bad "binary missing: $BIN"

hdr "Preflight (--doctor)"
if [ -x "$BIN" ]; then
  set +e; "$BIN" --doctor --config /nonexistent.conf; DOC=$?; set -e
  case "$DOC" in
    0) ok "doctor exit 0 (all checks passed)";;
    1) warn "doctor exit 1 (warnings — expected with no config/audio source)";;
    2) warn "doctor exit 2 (errors — review the report above)";;
    *) bad "doctor exit $DOC (outside the documented 0/1/2 contract)";;
  esac
fi

hdr "Boot smoke (--web-only, GET /)"
if [ -x "$BIN" ]; then
  TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"; [ -n "${SRV:-}" ] && kill "$SRV" 2>/dev/null || true' EXIT
  printf 'SITENAME=macOS Verify\nLATITUDE=51.48\nLONGITUDE=-0.13\nDB_PATH=%s/birds.db\n' "$TMP" > "$TMP/birdnet.conf"
  "$BIN" --web-only --config "$TMP/birdnet.conf" --listen "127.0.0.1:$PORT" >"$TMP/server.log" 2>&1 &
  SRV=$!
  UP=0
  for _ in $(seq 1 60); do
    if ! kill -0 "$SRV" 2>/dev/null; then bad "server exited during startup — see log below"; sed 's/^/       /' "$TMP/server.log" | tail -20; break; fi
    code="$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$PORT/" 2>/dev/null || true)"
    if [ "$code" = "200" ]; then ok "GET / -> 200 (server boots and serves)"; UP=1; break; fi
    sleep 1
  done
  [ "$UP" = "1" ] || { [ -n "${SRV:-}" ] && kill -0 "$SRV" 2>/dev/null && bad "GET / never returned 200 within 60s"; }
  kill "$SRV" 2>/dev/null || true; wait "$SRV" 2>/dev/null || true; SRV=
fi

hdr "Microphone enumeration (avfoundation)"
if command -v ffmpeg >/dev/null 2>&1; then
  echo "  audio input devices (use the [n] index as the mic 'device' in your config):"
  ffmpeg -hide_banner -f avfoundation -list_devices true -i "" 2>&1 | grep -iE 'AVFoundation|audio devices|\[[0-9]+\]' | sed 's/^/    /' || warn "could not enumerate devices"
else
  warn "ffmpeg missing — cannot enumerate avfoundation devices"
fi

hdr "Manual steps (need a human at the machine)"
cat <<'EOF'
  [ ] Microphone / TCC consent: start the station with a mic source; macOS shows a
      "BirdNet-Behavior would like to access the microphone" prompt the FIRST time
      ffmpeg opens avfoundation. Approve it under
        System Settings -> Privacy & Security -> Microphone.
      A headless LaunchDaemon cannot get this consent — use the per-user LaunchAgent.

  [ ] launchd LaunchAgent:
        cp packaging/macos/com.tomtom215.birdnet-behavior.plist ~/Library/LaunchAgents/
        # edit ProgramArguments paths, BNB_STATION_LAT/LON and BNB_SHARE_SECRET, then:
        launchctl load -w ~/Library/LaunchAgents/com.tomtom215.birdnet-behavior.plist
      Confirm it auto-starts at login and survives a reboot; logs at
      /opt/homebrew/var/log/birdnet-behavior.log

  [ ] Inert Linux-only paths (informational — these are cfg-gated off, not errors):
      systemd service controls on /admin/system are disabled; the tmpfs transient-audio
      mount is not used; some /proc host metrics on the System page are blank.

  [ ] macos-14 release dry run: trigger .github/workflows/release.yml (or run the
      aarch64-apple-darwin build locally) and confirm the tarball + .sha256 are produced.
      Then fill packaging/macos/birdnet-behavior.rb's sha256 and publish the tap.
EOF

hdr "Summary"
printf '  \033[32m%d ok\033[0m, \033[33m%d warn\033[0m, \033[31m%d fail\033[0m\n' "$PASS" "$WARN" "$FAIL"
echo "  Paste this whole output back to finish the macOS sign-off."
[ "$FAIL" -eq 0 ]
