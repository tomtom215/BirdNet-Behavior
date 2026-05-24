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

# Offer to `brew install` a missing Homebrew-provided dependency. Honors
# ASSUME_YES=1 for unattended installs; otherwise prompts on a TTY; falls back
# to printing the command when neither applies or Homebrew is absent.
brew_install() { # $1=formula  $2=why
  local pkg="$1" why="$2" ans=""
  echo "       $why"
  if ! command -v brew >/dev/null 2>&1; then
    echo "       Homebrew not found. Install it from https://brew.sh then: brew install $pkg"
    return 1
  fi
  if [ "${ASSUME_YES:-0}" = "1" ]; then
    ans="y"
  elif [ -t 0 ]; then
    printf "       Install it now with 'brew install %s'? [y/N] " "$pkg"
    read -r ans </dev/tty 2>/dev/null || ans=""
  fi
  case "$ans" in
    y | Y | yes | YES)
      echo "       running: brew install $pkg"
      brew install "$pkg"
      ;;
    *)
      echo "       skipped — install later with: brew install $pkg"
      return 1
      ;;
  esac
}

for tool in cargo rustc cmake ffmpeg; do
  if command -v "$tool" >/dev/null 2>&1; then
    ok "$tool: $(command -v "$tool")"
    continue
  fi
  case "$tool" in
    cargo | rustc)
      bad "$tool not found — install the Rust toolchain from https://rustup.rs"
      ;;
    cmake)
      warn "cmake not found — needed to compile the bundled libduckdb"
      brew_install cmake "cmake builds the bundled libduckdb." || true
      command -v cmake >/dev/null 2>&1 && ok "cmake now: $(command -v cmake)"
      ;;
    ffmpeg)
      warn "ffmpeg not found — needed for microphone capture (avfoundation) and RTSP"
      brew_install ffmpeg "ffmpeg captures the microphone (avfoundation) and RTSP streams." || true
      command -v ffmpeg >/dev/null 2>&1 && ok "ffmpeg now: $(command -v ffmpeg)"
      ;;
  esac
done

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
echo "  repo: $ROOT"

hdr "Build (cargo build --release --features analytics)"
BIN="$ROOT/target/release/birdnet-behavior"
if [ "${SKIP_BUILD:-0}" = "1" ] && [ -x "$BIN" ]; then
  warn "SKIP_BUILD=1 — reusing existing $BIN"
else
  echo "  building (first build downloads the ONNX Runtime prebuilt + compiles libduckdb;"
  echo "  ~6 min on a 10-core M-series, longer on fewer cores)…"
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
  # `ffmpeg -list_devices true -i ""` lists devices then exits NON-ZERO by
  # design (there is no real input), so don't gate on its exit code — parse the
  # output. Surface only the *audio* inputs, which are what the mic config needs.
  devs="$(ffmpeg -hide_banner -f avfoundation -list_devices true -i "" 2>&1 || true)"
  audio="$(printf '%s\n' "$devs" | sed -n '/audio devices/,$p' \
            | grep -E '\] \[[0-9]+\]' | sed -E 's/.*\] (\[[0-9]+\] .*)/    \1/')"
  if [ -n "$audio" ]; then
    echo "  audio input devices (use the [n] index as the mic 'device' in your config):"
    printf '%s\n' "$audio"
    ok "enumerated $(printf '%s\n' "$audio" | grep -c .) audio input device(s)"
  else
    warn "no avfoundation audio inputs found — connect a microphone and grant mic permission, then retry"
  fi
else
  warn "ffmpeg missing — cannot enumerate avfoundation devices (brew install ffmpeg)"
fi

hdr "Generate a from-source LaunchAgent"
# The committed plist assumes Homebrew paths (/opt/homebrew/bin). A from-source
# run lives at target/release, so generate a ready-to-load plist pre-filled with
# the real binary path and a fresh random share secret — no hand-editing needed.
if [ -x "$BIN" ]; then
  GEN="$ROOT/target/com.tomtom215.birdnet-behavior.generated.plist"
  CONF_DIR="$HOME/Library/Application Support/birdnet-behavior"
  SECRET="$(openssl rand -base64 48 2>/dev/null | tr -d '\n' || echo 'CHANGE-ME-to-32-plus-random-bytes')"
  mkdir -p "$ROOT/target"
  cat > "$GEN" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>com.tomtom215.birdnet-behavior</string>
  <key>ProgramArguments</key>
  <array>
    <string>$BIN</string>
    <string>-c</string>
    <string>$CONF_DIR/birdnet.conf</string>
    <string>--analytics-db</string>
    <string>$CONF_DIR/analytics.duckdb</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>$HOME/Library/Logs/birdnet-behavior.log</string>
  <key>StandardErrorPath</key><string>$HOME/Library/Logs/birdnet-behavior.err.log</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>BNB_STATION_LAT</key><string>0.0</string>
    <key>BNB_STATION_LON</key><string>0.0</string>
    <key>BNB_SHARE_SECRET</key><string>$SECRET</string>
  </dict>
</dict>
</plist>
PLIST
  ok "wrote $GEN"
  echo "  Binary path and a random BNB_SHARE_SECRET are already filled in. Before loading:"
  echo "    1. mkdir -p \"$CONF_DIR\" && cp .env.example \"$CONF_DIR/birdnet.conf\"   # then set LATITUDE/LONGITUDE"
  echo "    2. edit BNB_STATION_LAT/LON in the plist to your station coordinates"
  echo "    3. cp \"$GEN\" ~/Library/LaunchAgents/com.tomtom215.birdnet-behavior.plist"
  echo "    4. launchctl load -w ~/Library/LaunchAgents/com.tomtom215.birdnet-behavior.plist"
else
  warn "no binary yet — skipping LaunchAgent generation"
fi

hdr "Manual steps (need a human at the machine)"
cat <<'EOF'
  [ ] Microphone / TCC consent: start the station with a mic source; macOS shows a
      "BirdNet-Behavior would like to access the microphone" prompt the FIRST time
      ffmpeg opens avfoundation. Approve it under
        System Settings -> Privacy & Security -> Microphone.
      A headless LaunchDaemon cannot get this consent — use the per-user LaunchAgent
      generated above. Logs land in ~/Library/Logs/birdnet-behavior.log

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
