#!/usr/bin/env bash
# hardware-test.sh — on-device acceptance harness for a real Raspberry Pi station.
#
# Closes the gaps `docs/RELEASE_PLAN.md` § 5 ("Still not established") leaves open:
# throughput, thermals and live capture from a physical microphone were never
# measured on the board, and the fault-injection recovery paths were only ever
# exercised against fakes in `cargo test`. This script runs them against the
# installed systemd service on the hardware itself.
#
# Usage:
#   ./hardware-test.sh                 # run every phase in order
#   ./hardware-test.sh --resume        # continue after the reboot phase
#   ./hardware-test.sh --phase perf    # run one phase (repeatable)
#   ./hardware-test.sh --skip install  # run everything EXCEPT this (repeatable)
#   ./hardware-test.sh --list          # show phase ids
#   ./hardware-test.sh --safe          # skip the destructive phases
#   ./hardware-test.sh --yes           # never prompt (skips interactive unplug)
#
# `--skip install` is the one to know about when testing a binary you put on
# the box yourself: the install phase fetches the PUBLISHED release and would
# overwrite it. Skipped phases are recorded as skipped, so the `--resume` after
# the reboot does not quietly run them either.
#
# Run it as the ordinary login user (NOT root) — the station's data directory is
# derived from $HOME, and that is the account the installer configures. sudo is
# invoked for the individual steps that need it.
#
# Results land in ./birdnet-hwtest-<timestamp>/ as report.md + results.jsonl.

set -uo pipefail   # deliberately not -e: a failing check is data, not an abort

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

readonly REPO="tomtom215/BirdNet-Behavior"
readonly INSTALL_URL="https://raw.githubusercontent.com/${REPO}/main/install.sh"
readonly SERVICE="birdnet-behavior.service"
readonly BINARY="/usr/local/bin/birdnet-behavior"
readonly CONFIG_FILE="/etc/birdnet/birdnet.conf"
readonly STREAM_DIR="/tmp/birdnet-stream"
readonly PORT="${BIRDNET_PORT:-8502}"
readonly BASE="http://127.0.0.1:${PORT}"
# Prometheus text lives under the versioned API prefix, not at the site root —
# `health::router()` is one of the routers nested under /api/v2.
readonly METRICS_URL="${BASE}/api/v2/metrics"

# Phase list, in execution order.
readonly PHASES=(
  env install verify capture detect pipeline perf web
  watchdog unplug netloss diskfull dbcorrupt duckdb reboot report
)
# Phases that deliberately break the station.
readonly DESTRUCTIVE=" watchdog unplug netloss diskfull dbcorrupt duckdb reboot "

# ---------------------------------------------------------------------------
# Options
# ---------------------------------------------------------------------------

RESUME=0
ASSUME_YES=0
SAFE_ONLY=0
SELECTED=()
SKIPPED=()

# Print the header comment block (line 2 up to the first non-comment line).
usage() { awk 'NR>1 && /^#/ { sub(/^# ?/, ""); print; next } NR>1 { exit }' "$0"; exit 0; }

while [ $# -gt 0 ]; do
  case "$1" in
    --resume)  RESUME=1 ;;
    --yes|-y)  ASSUME_YES=1 ;;
    --safe)    SAFE_ONLY=1 ;;
    --phase)   shift; SELECTED+=("${1:-}") ;;
    --skip)    shift; SKIPPED+=("${1:-}") ;;
    --list)    printf '%s\n' "${PHASES[@]}"; exit 0 ;;
    -h|--help) usage ;;
    *) echo "unknown option: $1 (try --help)" >&2; exit 64 ;;
  esac
  shift
done

# ---------------------------------------------------------------------------
# Preconditions — checked before anything is written to disk, so a refused run
# leaves no stray artifacts behind.
# ---------------------------------------------------------------------------

if [ "$(id -u)" = "0" ]; then
  echo "Run this as your normal login user, not root — the station's data directory" >&2
  echo "is derived from \$HOME, and root's \$HOME is not where the installer puts it." >&2
  exit 1
fi
for _c in curl systemctl journalctl awk sed df; do
  command -v "$_c" >/dev/null 2>&1 || { echo "missing required command: $_c" >&2; exit 1; }
done
unset _c

# ---------------------------------------------------------------------------
# Output directory + state (survives the reboot phase)
# ---------------------------------------------------------------------------

STATE_LINK="${HOME}/.birdnet-hwtest-current"
if [ "$RESUME" = "1" ] && [ -L "$STATE_LINK" ]; then
  OUT="$(readlink -f "$STATE_LINK")"
else
  OUT="${PWD}/birdnet-hwtest-$(date +%Y%m%d-%H%M%S)"
  mkdir -p "$OUT"
  ln -sfn "$OUT" "$STATE_LINK"
fi
readonly OUT
readonly STATE="${OUT}/state.env"
readonly RESULTS="${OUT}/results.jsonl"
readonly LOGFILE="${OUT}/run.log"
touch "$STATE" "$RESULTS" "$LOGFILE"

# ---------------------------------------------------------------------------
# Presentation + recording
# ---------------------------------------------------------------------------

if [ -t 1 ]; then
  C_RED=$'\033[31m'; C_GRN=$'\033[32m'; C_YEL=$'\033[33m'
  C_BLU=$'\033[34m'; C_DIM=$'\033[2m'; C_BLD=$'\033[1m'; C_OFF=$'\033[0m'
else
  C_RED=''; C_GRN=''; C_YEL=''; C_BLU=''; C_DIM=''; C_BLD=''; C_OFF=''
fi

PASS_N=0; FAIL_N=0; WARN_N=0; SKIP_N=0
CURRENT_PHASE="-"

log()  { printf '%s\n' "$*" | tee -a "$LOGFILE" >/dev/null; }
say()  { printf '%s\n' "$*"; log "$*"; }
head1() {
  CURRENT_PHASE="$1"
  say ""
  say "${C_BLD}${C_BLU}=== [$1] $2 ===${C_OFF}"
}
info() { say "${C_DIM}    $*${C_OFF}"; }

# json_escape <string>
json_escape() {
  local s=${1//\\/\\\\}; s=${s//\"/\\\"}; s=${s//$'\n'/\\n}; s=${s//$'\t'/\\t}
  printf '%s' "$s"
}

# record <PASS|FAIL|WARN|SKIP|INFO> <id> <description> [detail]
record() {
  local status="$1" id="$2" desc="$3" detail="${4:-}"
  case "$status" in
    PASS) PASS_N=$((PASS_N+1)); say "${C_GRN}[PASS]${C_OFF} ${id} — ${desc}" ;;
    FAIL) FAIL_N=$((FAIL_N+1)); say "${C_RED}[FAIL]${C_OFF} ${id} — ${desc}" ;;
    WARN) WARN_N=$((WARN_N+1)); say "${C_YEL}[WARN]${C_OFF} ${id} — ${desc}" ;;
    SKIP) SKIP_N=$((SKIP_N+1)); say "${C_DIM}[SKIP]${C_OFF} ${id} — ${desc}" ;;
    INFO) say "${C_DIM}[info]${C_OFF} ${id} — ${desc}" ;;
  esac
  [ -n "$detail" ] && info "$detail"
  printf '{"ts":"%s","phase":"%s","status":"%s","id":"%s","desc":"%s","detail":"%s"}\n' \
    "$(date -Is)" "$(json_escape "$CURRENT_PHASE")" "$status" \
    "$(json_escape "$id")" "$(json_escape "$desc")" "$(json_escape "$detail")" \
    >> "$RESULTS"
}

# check <id> <description> <command...> — PASS on exit 0, FAIL otherwise.
check() {
  local id="$1" desc="$2"; shift 2
  local out rc
  out="$("$@" 2>&1)"; rc=$?
  if [ $rc -eq 0 ]; then
    record PASS "$id" "$desc" "${out:0:400}"
  else
    record FAIL "$id" "$desc" "exit ${rc}: ${out:0:400}"
  fi
  return $rc
}

state_set() { local k="$1" v="$2"; sed -i "/^${k}=/d" "$STATE" 2>/dev/null; printf '%s=%s\n' "$k" "$v" >> "$STATE"; }
state_get() { local k="$1"; sed -n "s/^${k}=//p" "$STATE" | tail -1; }
phase_done() { [ "$(state_get "PHASE_$1")" = "done" ]; }

confirm() {
  [ "$ASSUME_YES" = "1" ] && return 0
  local reply
  printf '%s' "${C_YEL}$1 [y/N] ${C_OFF}"
  read -r reply </dev/tty || return 1
  [[ "$reply" =~ ^[Yy] ]]
}

# ---------------------------------------------------------------------------
# Small helpers
# ---------------------------------------------------------------------------

have() { command -v "$1" >/dev/null 2>&1; }
svc_active() { systemctl is-active --quiet "$SERVICE"; }
metrics() { curl -sf --max-time 10 "$METRICS_URL" 2>/dev/null; }

# Current value of the audio-source gauge (first source), or "" if absent.
source_up_value() { metrics | awk '/^birdnet_audio_source_up\{/ { print $2; exit }'; }

# Total detections as the API reports them (Stats.total_detections).
stats_total() {
  curl -sf --max-time 10 "${BASE}/api/v2/stats" 2>/dev/null \
    | grep -o '"total_detections"[[:space:]]*:[[:space:]]*[0-9]*' \
    | grep -o '[0-9]*$' | head -1
}

# metric_value <exact series name, labels included>
metric_value() {
  metrics | awk -v k="$1" '$1 == k { print $2; found=1 } END { if (!found) print "" }' | tail -1
}

soc_temp_c() {
  if have vcgencmd; then
    vcgencmd measure_temp 2>/dev/null | sed 's/temp=//; s/'"'"'C//'
  elif [ -r /sys/class/thermal/thermal_zone0/temp ]; then
    awk '{ printf "%.1f", $1/1000 }' /sys/class/thermal/thermal_zone0/temp
  fi
}

throttled_hex() { have vcgencmd && vcgencmd get_throttled 2>/dev/null | sed 's/throttled=//'; }

# Whole-machine CPU utilisation as an integer percentage, measured over
# `$1` seconds (default 1) from /proc/stat.
#
# The harness recorded loadavg and never a utilisation figure, so "how much CPU
# headroom does this board have while running a station?" — and "is the CPU
# number the dashboard shows even real?" — were both unanswerable from a run.
# Loadavg is not a substitute: it counts runnable tasks, not busy time, and on
# a 4-core Pi a load of 1.0 could be 25 % busy or one task blocked on I/O.
cpu_util_pct() {
  local secs="${1:-1}" a b at bt ai bi
  # shellcheck disable=SC2034  # fields are positional; only the sums are used
  read -r _ u1 n1 s1 i1 w1 q1 sq1 st1 _ < /proc/stat
  at=$(( u1 + n1 + s1 + i1 + w1 + q1 + sq1 + st1 )); ai=$(( i1 + w1 ))
  sleep "$secs"
  read -r _ u2 n2 s2 i2 w2 q2 sq2 st2 _ < /proc/stat
  bt=$(( u2 + n2 + s2 + i2 + w2 + q2 + sq2 + st2 )); bi=$(( i2 + w2 ))
  a=$(( bt - at )); b=$(( bi - ai ))
  [ "$a" -gt 0 ] && echo $(( (100 * (a - b)) / a )) || echo 0
}

# The CPU percentage the dashboard's Vitals panel is showing, or empty when the
# page cannot be read. Parsed from the Station page's CPU vital.
ui_cpu_pct() {
  curl -sf --max-time 10 "${BASE}/station" 2>/dev/null \
    | tr '\n' ' ' \
    | sed -n 's/.*>CPU<\/div>[^0-9]*<div class="v">\([0-9]\+\)%.*/\1/p' \
    | head -1
}

daemon_pid() { systemctl show -p MainPID --value "$SERVICE" 2>/dev/null; }

daemon_rss_kb() {
  local p; p="$(daemon_pid)"
  [ -n "$p" ] && [ "$p" != "0" ] && awk '/^VmRSS:/ { print $2 }' "/proc/$p/status" 2>/dev/null
}

# Run a command inside the running service's mount namespace.
#
# The unit sets PrivateTmp=yes, so the daemon's /tmp is a private tmpfs and the
# host's ${STREAM_DIR} is a *different directory* that the daemon never reads or
# writes. Any check or injection involving the watch dir has to cross into that
# namespace or it is measuring the wrong filesystem.
in_service_ns() {
  local pid; pid="$(daemon_pid)"
  [ -n "$pid" ] && [ "$pid" != "0" ] || return 1
  have nsenter || return 1
  sudo nsenter -t "$pid" -m -- "$@"
}

# The device string the daemon is configured to open.
configured_alsa_device() {
  sudo sed -n 's/^[[:space:]]*ALSA_CARD[[:space:]]*=[[:space:]]*//p' "$CONFIG_FILE" 2>/dev/null \
    | tr -d '"' | tail -1
}

# wait_for <seconds> <command...> — poll until the command succeeds.
wait_for() {
  local deadline=$(( SECONDS + $1 )); shift
  while [ $SECONDS -lt $deadline ]; do
    "$@" >/dev/null 2>&1 && return 0
    sleep 2
  done
  return 1
}

snapshot_journal() {
  journalctl -u "$SERVICE" --no-pager -n "${2:-300}" > "${OUT}/journal-$1.log" 2>&1 || true
}

# Discover the live paths from the installed unit rather than assuming them.
discover_paths() {
  local exec_line
  exec_line="$(systemctl show -p ExecStart --value "$SERVICE" 2>/dev/null)"
  ANALYTICS_DB="$(sed -n 's/.*--analytics-db \([^ ]*\).*/\1/p' <<<"$exec_line" | head -1)"
  DATA_DIR="${HOME}/BirdNet-Behavior"
  [ -n "$ANALYTICS_DB" ] && DATA_DIR="$(dirname "$ANALYTICS_DB")"
  DB_PATH="$(sed -n 's/^[[:space:]]*DB_PATH=//p' "$CONFIG_FILE" 2>/dev/null | tr -d '"' | tail -1)"
  [ -z "$DB_PATH" ] && DB_PATH="${DATA_DIR}/birds.db"
  RECS_DIR="$(sed -n 's/^[[:space:]]*RECS_DIR=//p' "$CONFIG_FILE" 2>/dev/null | tr -d '"' | tail -1)"
  [ -z "$RECS_DIR" ] && RECS_DIR="${DATA_DIR}/recordings"
  [ -z "$ANALYTICS_DB" ] && ANALYTICS_DB="${DATA_DIR}/analytics.db"
}

# ---------------------------------------------------------------------------
# Safety net — every destructive phase arms its own undo before it breaks
# anything, so a Ctrl-C or a dead SSH session cannot strand the station.
# ---------------------------------------------------------------------------

BALLAST=""
STOPPED_PID=""

cleanup() {
  local rc=$?
  # Idempotent: on_signal calls this and then exits, which fires it again via
  # the EXIT trap. Clearing the globals makes the second pass a no-op instead
  # of a second `rm` and a duplicate line in the log.
  if [ -n "$BALLAST" ] && [ -f "$BALLAST" ]; then
    sudo rm -f "$BALLAST"
    echo "cleanup: removed ballast $BALLAST"
  fi
  BALLAST=""
  if [ -n "$STOPPED_PID" ] && kill -0 "$STOPPED_PID" 2>/dev/null; then
    sudo kill -CONT "$STOPPED_PID" 2>/dev/null && echo "cleanup: resumed SIGSTOPped pid $STOPPED_PID"
  fi
  STOPPED_PID=""
  return $rc
}

# Signals need their own handler, and it has to exit.
#
# A bash trap whose handler merely RETURNS does not stop the script: execution
# resumes where the signal interrupted it. With one `trap cleanup EXIT INT TERM`
# and a cleanup that returns, Ctrl-C during the destructive suite freed the
# ballast and then carried straight on into the next fault injection — with
# BALLAST already cleared, so the phase went on measuring a filesystem whose
# ballast had just been pulled. An operator reaching for Ctrl-C wants the run
# to stop; the docs promised as much. 130 is the conventional SIGINT status.
on_signal() {
  say ""
  say "${C_YEL}interrupted — cleaning up and stopping.${C_OFF}"
  cleanup
  exit 130
}
trap cleanup EXIT
trap on_signal INT TERM

# ===========================================================================
# Phases
# ===========================================================================

phase_env() {
  head1 env "Board, OS and peripheral inventory"

  {
    echo "== date ==";        date -Is
    echo "== model ==";       cat /proc/device-tree/model 2>/dev/null; echo
    echo "== os-release =="; cat /etc/os-release
    echo "== kernel ==";      uname -a
    echo "== glibc ==";       getconf GNU_LIBC_VERSION 2>/dev/null || ldd --version | head -1
    echo "== memory ==";      free -h
    echo "== disk ==";        df -h
    echo "== cpu ==";         lscpu 2>/dev/null | head -20
    echo "== usb ==";         lsusb 2>/dev/null
    echo "== alsa ==";        arecord -l 2>/dev/null
    echo "== network ==";     ip -br addr
    echo "== time ==";        timedatectl status 2>/dev/null
    echo "== throttled ==";   throttled_hex
  } > "${OUT}/env.txt" 2>&1

  local model; model="$(tr -d '\0' < /proc/device-tree/model 2>/dev/null)"
  record INFO env.model "${model:-unknown board}"

  # Architecture must be aarch64 — there is no armv7 build.
  if [ "$(uname -m)" = "aarch64" ]; then
    record PASS env.arch "64-bit kernel (aarch64)"
  else
    record FAIL env.arch "expected aarch64, found $(uname -m)" \
      "The project ships no 32-bit binary. Reflash with the 64-bit Pi OS image."
  fi

  # glibc floor: the release links against 2.39.
  local glibc; glibc="$(getconf GNU_LIBC_VERSION 2>/dev/null | awk '{print $2}')"
  [ -z "$glibc" ] && glibc="$(ldd --version 2>/dev/null | head -1 | grep -oE '[0-9]+\.[0-9]+' | tail -1)"
  if [ -n "$glibc" ] && [ "$(printf '2.39\n%s\n' "$glibc" | sort -V | head -1)" = "2.39" ]; then
    record PASS env.glibc "glibc ${glibc} >= 2.39 — native binary supported"
  else
    record FAIL env.glibc "glibc ${glibc:-unknown} < 2.39" \
      "The native binary cannot run here; this box needs the Docker path instead."
  fi

  # A capture device must exist, or every audio phase below is meaningless.
  if arecord -l 2>/dev/null | grep -q '^card'; then
    local cards; cards="$(arecord -l | grep '^card' | sed 's/,.*//' | paste -sd'; ')"
    record PASS env.mic "capture device present" "$cards"
    state_set HAS_MIC 1
  else
    record FAIL env.mic "no ALSA capture device found" \
      "Plug in the USB microphone. Audio phases will be skipped without it."
    state_set HAS_MIC 0
  fi

  # Undervoltage / thermal throttling flags — the classic Pi field failure.
  local thr; thr="$(throttled_hex)"
  if [ -z "$thr" ]; then
    record SKIP env.throttle "vcgencmd unavailable — cannot read throttle flags"
  elif [ "$thr" = "0x0" ]; then
    record PASS env.throttle "no throttling or undervoltage recorded (0x0)"
  else
    record WARN env.throttle "throttle register is ${thr}, not 0x0" \
      "Bit 0=undervoltage now, 16=undervoltage has occurred. Suspect the PSU or cable."
  fi

  # Clock sync — detection timestamps come from filenames in OS-local time.
  if timedatectl show -p NTPSynchronized --value 2>/dev/null | grep -q yes; then
    record PASS env.clock "system clock is NTP-synchronised"
  else
    record WARN env.clock "clock not NTP-synchronised" \
      "Timestamps and solar recording windows will be wrong until this settles."
  fi

  local free_gb; free_gb="$(df -BG --output=avail / | tail -1 | tr -dc '0-9')"
  if [ "${free_gb:-0}" -ge 6 ]; then
    record PASS env.disk "${free_gb} GiB free on / (need ~6: 34 MB binary + 541 MB model + recordings)"
  else
    record FAIL env.disk "only ${free_gb} GiB free on /" "The model alone needs ~541 MB plus room to unpack."
  fi

  record INFO env.temp "SoC temperature at rest: $(soc_temp_c) °C"
  state_set TEMP_IDLE "$(soc_temp_c)"
}

phase_install() {
  head1 install "One-line installer, end to end"

  if [ -x "$BINARY" ]; then
    record INFO install.existing "binary already present — re-running the installer is idempotent"
  fi

  local t0 rc
  t0=$SECONDS
  say "${C_DIM}    running: curl -fsSL ${INSTALL_URL} | sudo BIRDNET_NONINTERACTIVE=1 bash${C_OFF}"
  say "${C_DIM}    (this pulls a 34 MB binary and a ~541 MB model — several minutes on wifi)${C_OFF}"

  # The pipe form is deliberate: `sudo bash <(curl ...)` hands sudo a file
  # descriptor owned by the calling user, which it closes crossing to root.
  #
  # shellcheck disable=SC2024  # the redirect is intentionally performed by this
  # shell, not by root: ${OUT} belongs to the invoking user, so the log stays
  # readable without sudo and root never creates a file we cannot clean up.
  curl -fsSL "$INSTALL_URL" \
    | sudo BIRDNET_NONINTERACTIVE=1 bash > "${OUT}/install.log" 2>&1
  rc=$?
  local elapsed=$(( SECONDS - t0 ))

  if [ $rc -eq 0 ]; then
    record PASS install.run "installer exited 0 in ${elapsed}s" "full log: ${OUT}/install.log"
  else
    record FAIL install.run "installer exited ${rc} after ${elapsed}s" \
      "$(tail -20 "${OUT}/install.log" | tr '\n' ' ')"
    return 1
  fi
  state_set INSTALL_SECS "$elapsed"

  check install.binary   "binary is installed and executable" test -x "$BINARY"
  check install.config   "config file was written"            test -f "$CONFIG_FILE"
  check install.unit     "systemd unit was installed"         test -f "/etc/systemd/system/${SERVICE}"

  discover_paths
  check install.model "model landed in ${DATA_DIR}/models" \
    bash -c "ls '${DATA_DIR}/models/'*.onnx >/dev/null 2>&1"

  local sz
  sz="$(du -sh "${DATA_DIR}/models" 2>/dev/null | cut -f1)"
  record INFO install.modelsize "model directory is ${sz:-unknown}"
}

phase_verify() {
  head1 verify "Post-install verification"
  discover_paths

  local ver
  ver="$("$BINARY" --version 2>&1 | head -1)"
  if [ -n "$ver" ]; then
    record PASS verify.version "binary runs: ${ver}" \
      "This alone disproves nothing about glibc — it is the linker's verdict, and it passed."
    state_set VERSION "$ver"
  else
    record FAIL verify.version "binary would not report a version"
  fi

  # Doctor: 0 = clean, 1 = warnings, 2 = errors that block startup.
  local doc_out doc_rc
  doc_out="$("$BINARY" --config "$CONFIG_FILE" --doctor 2>&1)"; doc_rc=$?
  printf '%s\n' "$doc_out" > "${OUT}/doctor.txt"
  case $doc_rc in
    0) record PASS verify.doctor "doctor clean (exit 0)" ;;
    1) record PASS verify.doctor "doctor passed with warnings (exit 1 — startup allowed)" \
         "$(grep -i 'warn' <<<"$doc_out" | head -3 | tr '\n' ' ')" ;;
    *) record FAIL verify.doctor "doctor reported errors (exit ${doc_rc})" \
         "$(grep -iE 'fail|error' <<<"$doc_out" | head -3 | tr '\n' ' ')" ;;
  esac

  check verify.active "service is active" systemctl is-active --quiet "$SERVICE"

  if wait_for 90 curl -sf --max-time 5 "${BASE}/api/v2/health"; then
    local status
    status="$(curl -sf "${BASE}/api/v2/health" | sed -n 's/.*"status":"\([^"]*\)".*/\1/p')"
    record PASS verify.health "health endpoint responds (status=${status:-?})"
  else
    record FAIL verify.health "no response from ${BASE}/api/v2/health within 90s"
    snapshot_journal verify-health
  fi

  # The offline extension guarantee — the v0.11.0 headline fix. With no network
  # neither the DuckDB extension cache nor the community registry can satisfy
  # the load, so success proves the *embedded* copy is the one being used.
  if have unshare; then
    local ext_out ext_rc
    ext_out="$(sudo unshare -rn "$BINARY" --config "$CONFIG_FILE" --verify-extension 2>&1)"; ext_rc=$?
    printf '%s\n' "$ext_out" > "${OUT}/verify-extension.txt"
    if [ $ext_rc -eq 0 ]; then
      record PASS verify.extension "behavioral extension loads with networking disabled" \
        "$(grep -iE 'version|extension' <<<"$ext_out" | head -3 | tr '\n' ' ')"
    else
      record FAIL verify.extension "--verify-extension failed offline (exit ${ext_rc})" \
        "$(tail -5 <<<"$ext_out" | tr '\n' ' ')"
    fi
  else
    record SKIP verify.extension "unshare(1) unavailable — cannot isolate the network"
  fi

  # systemd hardening actually applied, not merely written into the unit.
  local hardening=0
  for prop in "Restart=always" "WatchdogUSec=" "ProtectSystem=strict" "OOMPolicy=stop"; do
    local key="${prop%%=*}" want="${prop#*=}" got
    got="$(systemctl show -p "$key" --value "$SERVICE" 2>/dev/null)"
    if [ -n "$want" ]; then
      [ "$got" = "$want" ] || { record WARN "verify.hardening.${key}" "${key} is '${got}', expected '${want}'"; hardening=1; }
    else
      [ -n "$got" ] && [ "$got" != "0" ] || { record WARN "verify.hardening.${key}" "${key} is unset"; hardening=1; }
    fi
  done
  [ $hardening -eq 0 ] && record PASS verify.hardening "Restart/Watchdog/ProtectSystem/OOMPolicy all applied by systemd"

  systemctl show "$SERVICE" > "${OUT}/unit-properties.txt" 2>&1
}

phase_capture() {
  head1 capture "Live capture from the physical microphone"
  discover_paths

  if [ "$(state_get HAS_MIC)" = "0" ]; then
    record SKIP capture.all "no capture device — plug in the mic and re-run --phase capture"
    return 0
  fi

  # At unity gain a local mic records via arecord; a non-zero gain routes it
  # through ffmpeg instead. Either is a live capture subprocess.
  local sub=""
  if pgrep -a arecord >/dev/null 2>&1; then
    sub="arecord: $(pgrep -a arecord | head -1)"
  elif pgrep -a ffmpeg >/dev/null 2>&1; then
    sub="ffmpeg: $(pgrep -a ffmpeg | head -1)"
  fi
  if [ -n "$sub" ]; then
    record PASS capture.subprocess "capture subprocess is running" "$sub"
  else
    record FAIL capture.subprocess "neither arecord nor ffmpeg is running" \
      "The daemon is up but nothing is pulling audio. Check Admin → Audio."
    snapshot_journal capture
  fi

  # The gauge is the daemon's own opinion of the source.
  local up; up="$(source_up_value)"
  if [ "$up" = "1" ]; then
    record PASS capture.gauge "birdnet_audio_source_up = 1"
  else
    record FAIL capture.gauge "birdnet_audio_source_up = ${up:-<absent>}"

    # The supervisor logs "capture (re)start issued" forever, but arecord's own
    # stderr — the line that says *why* it failed — is emitted at debug level
    # (`drain_capture_stderr`), so at the default log level the operator gets an
    # infinite restart loop with no diagnosis. Reproduce the exact invocation
    # here and put the real ALSA error in the report.
    local dev; dev="$(configured_alsa_device)"
    if [ -n "$dev" ]; then
      local err
      err="$(sudo timeout 10 arecord -D "$dev" -f S16_LE -r 48000 -c 1 -d 3 /dev/null 2>&1 | tr '\n' ' ')"
      if [ -n "$err" ]; then
        record FAIL capture.alsa_error "arecord on ${dev} reports: ${err:0:300}" \
          "This is the exact device/format the daemon opens (S16_LE, 48000 Hz, 1 ch). The daemon logs this at debug only."
      else
        record WARN capture.alsa_error "arecord on ${dev} succeeded when run directly" \
          "The device works standalone, so the failure is specific to the service context (sandboxing, group membership, or contention with the restart loop)."
      fi
      in_service_ns arecord -l > "${OUT}/arecord-in-namespace.txt" 2>&1 \
        && record INFO capture.ns_devices "captured 'arecord -l' as the service sees it → arecord-in-namespace.txt"
    fi
  fi

  # Segments must actually appear in the tmpfs watch directory — as seen from
  # inside the service's PrivateTmp namespace. Counting the host's copy of
  # ${STREAM_DIR} would report zero on a perfectly healthy station.
  local before after ns_ok=1
  before="$(in_service_ns find "$STREAM_DIR" -type f 2>/dev/null | wc -l)" || ns_ok=0
  if [ "$ns_ok" = "0" ] || [ -z "$before" ]; then
    record SKIP capture.segments "cannot enter the service mount namespace (nsenter/MainPID unavailable)" \
      "PrivateTmp=yes means the host's ${STREAM_DIR} is a different directory; checking it would be meaningless."
  else
    info "watching ${STREAM_DIR} inside the service namespace for new segments (60s)…"
    sleep 60
    after="$(in_service_ns find "$STREAM_DIR" -type f 2>/dev/null | wc -l)"
    if [ "${after:-0}" -gt "$before" ] || [ "${after:-0}" -gt 0 ]; then
      record PASS capture.segments "audio segments present in ${STREAM_DIR} (${before} → ${after})"
    else
      record FAIL capture.segments "no segments appeared in ${STREAM_DIR} over 60s (checked inside the namespace)"
    fi
  fi

  # And the recording that survives — proof the segment was written through.
  local recs; recs="$(find "$RECS_DIR" -name '*.wav' -o -name '*.mp3' -o -name '*.flac' 2>/dev/null | wc -l)"
  if [ "$recs" -gt 0 ]; then
    record PASS capture.recordings "${recs} recording(s) on disk in ${RECS_DIR}"
  else
    record WARN capture.recordings "no recordings yet in ${RECS_DIR}" \
      "Expected if nothing has crossed the confidence threshold — detect phase settles this."
  fi

  # Prove the mic itself is not delivering digital silence.
  #
  # Address it as plughw:, not hw: — raw hw: demands the device natively accept
  # S16_LE/48000/mono and many USB mics do not, so hw: fails on a perfectly good
  # microphone. plughw: inserts ALSA's conversion plugin, which is exactly why
  # the installer's detect_first_audio_device() emits plughw: too.
  local card wav
  card="$(arecord -l | sed -n 's/^card \([0-9]*\).*device \([0-9]*\).*/plughw:\1,\2/p' | head -1)"
  wav="${OUT}/mic-sample.wav"
  if [ -n "$card" ] && sudo timeout 8 arecord -D "$card" -d 5 -f S16_LE -r 48000 -c 1 "$wav" >/dev/null 2>&1; then
    local peak
    # sox is not a dependency; fall back to reporting that bytes were captured.
    if have sox; then
      peak="$(sox "$wav" -n stat 2>&1 | awk '/Maximum amplitude/ { print $3 }')"
      if awk "BEGIN { exit !($peak > 0.001) }"; then
        record PASS capture.signal "microphone delivers real signal (peak amplitude ${peak})"
      else
        record FAIL capture.signal "microphone output is silent (peak ${peak})" \
          "Check gain/mute: 'alsamixer -c ${card#hw:}' and unmute the capture channel."
      fi
    else
      record WARN capture.signal "captured $(stat -c%s "$wav") bytes but sox is absent" \
        "Install sox for an amplitude check, or listen to ${wav}."
    fi
  elif [ -n "$sub" ]; then
    record WARN capture.signal "could not open ${card:-the capture device} directly" \
      "Expected — the daemon holds it exclusively while capturing."
  else
    # Nothing is capturing, so the device is free. Failing to open it now is a
    # real finding, not contention: it means the daemon could not have opened it
    # either, which explains a source stuck down.
    record FAIL capture.signal "no capture process is running AND ${card:-the device} cannot be opened" \
      "$(sudo timeout 8 arecord -D "${card:-null}" -d 1 -f S16_LE -r 48000 -c 1 /dev/null 2>&1 | tail -2 | tr '\n' ' ')"
  fi
}

phase_detect() {
  head1 detect "End-to-end detection pipeline on real audio"
  discover_paths

  local before after
  before="$(stats_total)"
  before="${before:-0}"
  record INFO detect.before "detections in the database at start: ${before}"

  # Rather than wait on wild birds, push the bundled reference recording through
  # the watch directory — the same path the capture subprocess writes to.
  local ref="${OUT}/Pica_pica_30s.wav"
  if [ ! -f "$ref" ]; then
    curl -fsSL --max-time 120 \
      "https://raw.githubusercontent.com/${REPO}/main/tests/testdata/Pica_pica_30s.wav" \
      -o "$ref" 2>/dev/null
  fi

  if [ -s "$ref" ]; then
    # Inject *inside* the service's mount namespace. The unit runs with
    # PrivateTmp=yes, so a file dropped into the host's ${STREAM_DIR} is written
    # to a directory the daemon cannot see — the watcher would never fire and
    # the phase would blame the model for a delivery failure.
    local stamp target
    stamp="$(date +%Y-%m-%d-birdnet-%H:%M:%S)"
    target="${STREAM_DIR}/${stamp}.wav"
    if in_service_ns cp "$ref" "$target" 2>/dev/null; then
      record INFO detect.inject "injected the reference recording as ${target##*/} inside the service namespace"
    else
      record SKIP detect.inject "could not enter the service mount namespace to deliver the recording" \
        "PrivateTmp=yes; without nsenter the daemon cannot see anything written to the host's ${STREAM_DIR}."
      return 0
    fi

    # Wait for a NEW stored detection, not merely for the species to appear.
    #
    # This used to grep /api/v2/detections for the string "Pica pica" and pass
    # on a hit. Any station that had ever detected a magpie — including one that
    # ran this very phase yesterday — satisfied that instantly, whether or not
    # the file just injected classified as anything. Measured on a Pi 4: the
    # check passed while the stored-detection count sat unchanged at 151, and
    # the contradiction was reported as a PASS beside a WARN rather than as the
    # failure it was.
    #
    # The count rising is the load-bearing assertion: it can only happen if the
    # daemon picked the file up, decoded it, ran the model, cleared the
    # confidence gate and wrote a row. The species check then confirms it was
    # OUR file rather than a bird that happened to call during the window.
    # Same extraction as stats_total() — the field is total_detections, and a
    # near-miss like "total" silently yields an empty string, which would make
    # the guard below always false and turn this into a 180 s wait that always
    # fails. Kept literal here because wait_for spawns a fresh bash that has
    # none of this script's functions.
    if wait_for 180 bash -c "
      n=\$(curl -sf '${BASE}/api/v2/stats' 2>/dev/null \
           | grep -o '\"total_detections\"[[:space:]]*:[[:space:]]*[0-9]*' \
           | grep -o '[0-9]*\$' | head -1)
      [ -n \"\$n\" ] && [ \"\$n\" -gt ${before} ]"; then
      after="$(stats_total)"; after="${after:-0}"
      # Newest first (ORDER BY Date DESC, Time DESC), so the rows that could
      # correspond to the injection are at the head of the list.
      if curl -sf "${BASE}/api/v2/detections?limit=5" 2>/dev/null | grep -q 'Pica pica'; then
        record PASS detect.inference \
          "Eurasian Magpie (Pica pica) stored from the injected recording (${before} → ${after})" \
          "Real inference ran on this board against the 11k-species model, and the row reached SQLite."
      else
        record FAIL detect.inference \
          "detections rose ${before} → ${after} but no Pica pica among the newest rows" \
          "Something was stored and it was not the reference recording — check the confidence gate and the species filter."
      fi
    else
      after="$(stats_total)"; after="${after:-0}"
      record FAIL detect.inference \
        "the injected recording produced no new stored detection within 180s (count still ${after})" \
        "A pre-existing Pica pica on /api/v2/detections is NOT evidence: this asserts a new row, not the presence of the species."
      snapshot_journal detect
    fi
  else
    record SKIP detect.inject "could not fetch the reference recording (offline?)"
  fi

  local infcount
  infcount="$(metric_value birdnet_inference_duration_seconds_count)"
  if [ -n "$infcount" ] && [ "${infcount%.*}" -gt 0 ]; then
    record PASS detect.metric "inference histogram has ${infcount} observations"
  else
    record WARN detect.metric "birdnet_inference_duration_seconds_count is ${infcount:-absent}"
  fi
}

phase_pipeline() {
  head1 pipeline "Where captured audio is lost between microphone and model"
  discover_paths

  # A station that records continuously but classifies 6% of it looks perfectly
  # healthy: the watch dir is drained by *age*, so unread segments vanish with
  # no metric, no log line and no gap in the detection record. This phase
  # accounts for every segment between capture and inference and names the
  # stage that drops them.
  #
  # Everything it reads is already at the default log level — `run.rs` logs
  # "begin processing file" at INFO per file and "failed to process file" at
  # WARN — so no drop-in and no restart is needed, and the station stays in
  # exactly the configuration being judged.

  if ! svc_active; then
    record SKIP pipeline.all "service is not active"; return 0
  fi

  # Segment length straight off the live capture command, not assumed.
  local seg_secs argv
  argv="$(pgrep -a arecord 2>/dev/null | head -1)"
  [ -z "$argv" ] && argv="$(pgrep -a ffmpeg 2>/dev/null | head -1)"
  seg_secs="$(sed -n 's/.*--max-file-time \([0-9]*\).*/\1/p' <<<"$argv")"
  [ -z "$seg_secs" ] && seg_secs="$(sed -n 's/.*-segment_time \([0-9]*\).*/\1/p' <<<"$argv")"
  if [ -z "$seg_secs" ] || [ "$seg_secs" -le 0 ] 2>/dev/null; then
    record SKIP pipeline.all "no capture subprocess with a readable segment length" \
      "Run --phase capture first; this phase measures a running station."
    return 0
  fi
  record INFO pipeline.segment "capture writes ${seg_secs}s segments (read from the live command line)"

  local mins="${BIRDNET_PIPELINE_MINUTES:-10}"
  local t0 inf_c0
  t0="$(date +%s)"
  inf_c0="$(metric_value birdnet_inference_duration_seconds_count)"; inf_c0="${inf_c0:-0}"

  info "accounting for every segment over ${mins} minutes…"
  sleep $(( mins * 60 ))

  local inf_c1 elapsed
  inf_c1="$(metric_value birdnet_inference_duration_seconds_count)"; inf_c1="${inf_c1:-0}"
  elapsed=$(( $(date +%s) - t0 ))

  journalctl -u "$SERVICE" --since "@${t0}" --no-pager > "${OUT}/pipeline-journal.log" 2>&1 || true
  local began failed
  # `grep -c` already prints 0 when nothing matches; it just exits 1 doing so.
  # An `|| echo 0` here appended a *second* line and every later integer test
  # failed with "0\n0: integer expression expected".
  began="$(grep -c 'begin processing file' "${OUT}/pipeline-journal.log" 2>/dev/null)"
  failed="$(grep -c 'failed to process file' "${OUT}/pipeline-journal.log" 2>/dev/null)"
  began="${began:-0}"; failed="${failed:-0}"

  local expected chunks
  expected=$(( elapsed / seg_secs ))
  chunks=$(( ${inf_c1%.*} - ${inf_c0%.*} ))

  record INFO pipeline.counts \
    "segments written ~${expected} · files the daemon opened ${began} · decode failures ${failed} · detections stored ${chunks}"

  # ── Stage 1: did every recorded segment reach the daemon at all? ──────────
  local pickup
  pickup="$(awk "BEGIN { printf \"%.1f\", (${began} / ${expected}) * 100 }" 2>/dev/null)"
  if [ "$expected" -le 0 ]; then
    record SKIP pipeline.pickup "window too short to expect a segment"
  elif awk "BEGIN { exit !(${pickup} < 90) }"; then
    record FAIL pipeline.pickup \
      "the daemon opened only ${began} of ~${expected} segments (${pickup}%)" \
      "Segments are reaching disk but not the pipeline. Suspects: the notify watcher missing creation events, or the loop being blocked in inference while the FILE_SETTLE debounce (2 s) expires unnoticed. The unopened segments are deleted by the age drain, unread."
  else
    record PASS pipeline.pickup "the daemon opened ${began} of ~${expected} segments (${pickup}%)"
  fi

  # ── Stage 2: did the files it opened decode? ─────────────────────────────
  if [ "$failed" -gt 0 ]; then
    record FAIL pipeline.decode "${failed} file(s) failed to process" \
      "$(grep 'failed to process file' "${OUT}/pipeline-journal.log" | tail -2 | tr '\n' ' ')"
  elif [ "$began" -gt 0 ]; then
    record PASS pipeline.decode "every opened file decoded without error"
  fi

  # ── What the station actually detected over the window ──────────────────
  #
  # Deliberately NOT a verdict. `birdnet_inference_duration_seconds` is observed
  # in `daemon/processor.rs` inside the `DispositionDecision::Accept` arm, after
  # `insert_detection` — i.e. once per **stored detection**, not once per audio
  # chunk fed to the model. Its HELP text says "per-chunk", which is what misled
  # an earlier version of this phase into reporting a quiet room as 94% audio
  # loss. There is no exported per-chunk counter, so coverage cannot be computed
  # from the metrics endpoint at all; pickup above is the honest proxy.
  record INFO pipeline.detections \
    "${chunks} detection(s) stored during the window" \
    "Depends on how many birds called, not on pipeline health — zero is normal indoors or in a quiet hour."
}

phase_perf() {
  head1 perf "Throughput, thermals and memory under sustained load"
  discover_paths

  local t_start rss_start inf_c0 inf_s0
  t_start="$(soc_temp_c)"
  rss_start="$(daemon_rss_kb)"
  inf_c0="$(metric_value birdnet_inference_duration_seconds_count)"; inf_c0="${inf_c0:-0}"
  inf_s0="$(metric_value birdnet_inference_duration_seconds_sum)";   inf_s0="${inf_s0:-0}"

  record INFO perf.start "temp ${t_start} °C · RSS $(( ${rss_start:-0} / 1024 )) MiB · ${inf_c0%.*} inferences so far"

  # Backlog in the watch directory is the *direct* answer to "can this board keep
  # up?" — it needs no assumption about chunk length or model variant. Capture
  # writes segments at a fixed rate; if the detector drains them at least as
  # fast, the count stays flat. A monotonically growing queue is the station
  # falling behind, in the only units that matter.
  local backlog_start; backlog_start="$(in_service_ns find "$STREAM_DIR" -type f 2>/dev/null | wc -l)"
  backlog_start="${backlog_start:-0}"

  local mins="${BIRDNET_PERF_MINUTES:-10}"
  info "sampling for ${mins} minutes of live capture…"
  local samples="${OUT}/perf-samples.csv"
  echo "elapsed_s,temp_c,rss_kb,inference_count,load1,stream_backlog,cpu_pct" > "$samples"

  local deadline=$(( SECONDS + mins * 60 )) peak_temp=0 backlog_peak="$backlog_start"
  local cpu_peak=0 cpu_sum=0 cpu_n=0
  while [ $SECONDS -lt $deadline ]; do
    local t r c l b cpu
    t="$(soc_temp_c)"; r="$(daemon_rss_kb)"
    c="$(metric_value birdnet_inference_duration_seconds_count)"
    l="$(awk '{print $1}' /proc/loadavg)"
    b="$(in_service_ns find "$STREAM_DIR" -type f 2>/dev/null | wc -l)"; b="${b:-0}"
    cpu="$(cpu_util_pct 1)"
    echo "${SECONDS},${t},${r:-},${c%.*},${l},${b},${cpu}" >> "$samples"
    awk "BEGIN { exit !(${t:-0} > ${peak_temp}) }" && peak_temp="$t"
    [ "$b" -gt "$backlog_peak" ] && backlog_peak="$b"
    [ "$cpu" -gt "$cpu_peak" ] && cpu_peak="$cpu"
    cpu_sum=$(( cpu_sum + cpu )); cpu_n=$(( cpu_n + 1 ))
    sleep 29
  done

  # CPU headroom. The station keeping up (the backlog check below) says the
  # board is fast enough *today*; this says by how much, which is what decides
  # whether a warmer day or a busier dawn chorus will tip it over.
  if [ "$cpu_n" -gt 0 ]; then
    local cpu_mean=$(( cpu_sum / cpu_n ))
    record INFO perf.cpu "CPU ${cpu_mean}% mean · ${cpu_peak}% peak over ${cpu_n} samples"
    if [ "$cpu_peak" -ge 95 ]; then
      record WARN perf.cpu_headroom "CPU peaked at ${cpu_peak}% — effectively no headroom" \
        "The board is saturated. Expect the backlog to grow on a busy morning."
    else
      record PASS perf.cpu_headroom "CPU peak ${cpu_peak}% leaves headroom"
    fi
  fi

  # Is the CPU figure the dashboard shows actually real? Reported as looking
  # wrong on a Pi; it could not be reproduced off-hardware, where the reading
  # agrees with /proc/stat exactly. This settles it on the board itself rather
  # than by argument. Both numbers are whole-machine percentages sampled
  # seconds apart, so they track but will not match to the digit — the check is
  # deliberately loose, and only fails when the UI is plainly not measuring
  # (flat zero while the machine is busy, or wildly out).
  local ui_cpu truth_cpu
  truth_cpu="$(cpu_util_pct 1)"
  ui_cpu="$(ui_cpu_pct)"
  if [ -z "$ui_cpu" ]; then
    record SKIP perf.cpu_ui "could not read the CPU vital from ${BASE}/station"
  else
    local delta=$(( ui_cpu - truth_cpu )); [ "$delta" -lt 0 ] && delta=$(( -delta ))
    if [ "$ui_cpu" -eq 0 ] && [ "$truth_cpu" -ge 15 ]; then
      record FAIL perf.cpu_ui "dashboard shows CPU 0% while /proc/stat says ${truth_cpu}%" \
        "The CPU vital is not measuring anything on this hardware."
    elif [ "$delta" -le 35 ]; then
      record PASS perf.cpu_ui "dashboard CPU ${ui_cpu}% tracks /proc/stat ${truth_cpu}%"
    else
      record WARN perf.cpu_ui "dashboard CPU ${ui_cpu}% vs /proc/stat ${truth_cpu}%" \
        "Sampled seconds apart, so some drift is expected; a persistent gap this large is not."
    fi
  fi
  local backlog_end; backlog_end="$(in_service_ns find "$STREAM_DIR" -type f 2>/dev/null | wc -l)"
  backlog_end="${backlog_end:-0}"

  local t_end rss_end inf_c1 inf_s1
  t_end="$(soc_temp_c)"; rss_end="$(daemon_rss_kb)"
  inf_c1="$(metric_value birdnet_inference_duration_seconds_count)"; inf_c1="${inf_c1:-0}"
  inf_s1="$(metric_value birdnet_inference_duration_seconds_sum)";   inf_s1="${inf_s1:-0}"

  # Mean inference latency over the window — the number the plan calls "unmeasured".
  local dc ds mean
  dc=$(( ${inf_c1%.*} - ${inf_c0%.*} ))
  ds="$(awk "BEGIN { printf \"%.4f\", ${inf_s1} - ${inf_s0} }")"
  if [ "$dc" -gt 0 ]; then
    mean="$(awk "BEGIN { printf \"%.1f\", (${ds} / ${dc}) * 1000 }")"
    # A mean over a handful of samples is not a throughput figure, and the
    # denominator is not fixed either: BirdNET+ V3.0 dynamic takes 144 000
    # samples at 32 kHz — a 4.5 s chunk — while the fixed variant takes 3 s. So
    # report the latency as a measurement and let the backlog below decide
    # whether the board keeps up.
    record INFO perf.latency "mean decode-to-prediction latency ${mean} ms over ${dc} stored detection(s)" \
      "Observed once per *stored detection*, not per chunk — so the count tracks bird activity, not throughput."
    if [ "$dc" -lt 30 ]; then
      record WARN perf.latency_sample "only ${dc} detection(s) in ${mins} minutes — too few for a stable mean" \
        "A quiet site produces few detections however fast the board is. Use --phase pipeline to judge whether the station keeps up."
    fi
    state_set MEAN_LATENCY_MS "$mean"
  else
    record WARN perf.latency "no inferences completed during the ${mins}-minute window" \
      "Quiet site, or the confidence gate filtered everything before the histogram."
  fi

  # The watch-dir count is NOT a backlog and must never be used as a verdict.
  # `start_disk_manager` drains that directory purely by age and size — the
  # "locked" set it honours comes from `locked_file_names(conn)`, i.e. detection
  # clips an *operator* locked in the UI, not segments the pipeline has yet to
  # read. So the count parks at (retention ÷ segment length) whether the
  # pipeline analysed every segment or none of them, and a check that reads it
  # as a queue reports success from a number that cannot move.
  record INFO perf.backlog \
    "watch-dir holds ${backlog_start} → ${backlog_end} segments (peak ${backlog_peak})" \
    "Retention-bounded, not a queue: STREAM_RETENTION_SECS ÷ segment length. Informational only."

  # No real-time verdict here, deliberately. The only counter this phase can
  # reach is per stored detection, and the watch-dir count above is bounded by
  # the age drain — neither can tell a board that keeps up from one that does
  # not. `--phase pipeline` answers it properly, by counting the files the
  # daemon actually opened against the segments capture wrote.
  record INFO perf.realtime "throughput is not judged here — run --phase pipeline for the keep-up verdict"


  # Thermals. The Pi 4 begins soft-throttling at 80 °C, hard at 85 °C.
  record INFO perf.temp "temperature ${t_start:-?} → ${t_end:-?} °C (peak ${peak_temp} °C)"
  if awk "BEGIN { exit !(${peak_temp:-0} <= 0) }"; then
    # No reading at all. Reporting "peak 0 °C, below the threshold" would be a
    # pass that measured nothing — say so instead.
    record SKIP perf.thermal "no temperature reading available on this board" \
      "Neither vcgencmd nor /sys/class/thermal/thermal_zone0/temp returned a value."
  elif awk "BEGIN { exit !(${peak_temp} >= 80) }"; then
    record WARN perf.thermal "peak ${peak_temp} °C reached the throttle threshold (80 °C)" \
      "Add a heatsink or fan before a summer field deployment."
  else
    record PASS perf.thermal "peak ${peak_temp} °C stays below the 80 °C throttle point"
  fi

  local thr; thr="$(throttled_hex)"
  if [ -n "$thr" ] && [ "$thr" != "0x0" ]; then
    record WARN perf.throttled "throttle register ${thr} after load" "Bit 2 = currently throttled, bit 18 = has throttled."
  elif [ -n "$thr" ]; then
    record PASS perf.throttled "no throttling recorded under load (0x0)"
  fi

  # Memory drift against the unit's own MemoryHigh=768M / MemoryMax=1G.
  if [ -n "$rss_start" ] && [ -n "$rss_end" ]; then
    local drift_mb; drift_mb=$(( (rss_end - rss_start) / 1024 ))
    record INFO perf.rss "RSS $(( rss_start / 1024 )) → $(( rss_end / 1024 )) MiB (drift ${drift_mb} MiB)"
    if [ "$rss_end" -gt 786432 ]; then
      record WARN perf.memory "RSS $(( rss_end / 1024 )) MiB is above MemoryHigh=768M — systemd will start reclaiming"
    else
      record PASS perf.memory "RSS $(( rss_end / 1024 )) MiB stays under the 768 MiB MemoryHigh ceiling"
    fi
  fi
}

phase_web() {
  head1 web "HTTP surface from the network"

  local endpoints=(
    /api/v2/health /api/v2/stats /api/v2/detections /api/v2/species/top
    /api/v2/metrics /api/v2/analytics/status / /admin/doctor /admin/settings
  )
  local bad=0
  for ep in "${endpoints[@]}"; do
    local code
    code="$(curl -so /dev/null -w '%{http_code}' --max-time 10 "${BASE}${ep}" 2>/dev/null)"
    case "$code" in
      200|303|401|302) info "${ep} → ${code}" ;;
      *) record FAIL "web${ep//\//.}" "${ep} returned ${code}"; bad=1 ;;
    esac
  done
  [ $bad -eq 0 ] && record PASS web.endpoints "all ${#endpoints[@]} endpoints answered (200/302/303/401)"

  # The offline operator manual ships in the release tarball and is served from
  # a ServeDir, so a 404 means the docs tree is missing rather than broken.
  local help_code
  help_code="$(curl -so /dev/null -w '%{http_code}' --max-time 10 "${BASE}/help/" 2>/dev/null)"
  case "$help_code" in
    200|301|302) record PASS web.help "embedded operator manual is served at /help/ (${help_code})" ;;
    404) record WARN web.help "/help/ returns 404 — the rendered docs tree is absent from this install" ;;
    *)   record WARN web.help "/help/ returned ${help_code}" ;;
  esac

  # Reachability from the LAN — a station bound to loopback is invisible in the field.
  local lan_ip
  lan_ip="$(ip -4 -br addr show scope global | awk '{print $3}' | cut -d/ -f1 | head -1)"
  if [ -n "$lan_ip" ]; then
    local code
    code="$(curl -so /dev/null -w '%{http_code}' --max-time 10 "http://${lan_ip}:${PORT}/api/v2/health" 2>/dev/null)"
    if [ "$code" = "200" ]; then
      record PASS web.lan "dashboard reachable on the LAN at http://${lan_ip}:${PORT}"
    else
      record WARN web.lan "LAN address ${lan_ip}:${PORT} returned ${code}"
    fi
  fi

  # An open /admin on a non-loopback bind is the S-10 finding. Measuring the
  # HTTP behaviour alone is not enough: it has to be reconciled against what the
  # config says and what doctor claims, because the interesting failure is the
  # three disagreeing rather than any one of them being unset.
  local admin_code pwd_in_config=0 doctor_verdict
  admin_code="$(curl -so /dev/null -w '%{http_code}' --max-time 10 "${BASE}/admin/settings" 2>/dev/null)"
  sudo grep -qE '^[[:space:]]*CADDY_PWD[[:space:]]*=[[:space:]]*[^[:space:]#]' "$CONFIG_FILE" 2>/dev/null \
    && pwd_in_config=1
  doctor_verdict="$(grep -i 'admin authentication' "${OUT}/doctor.txt" 2>/dev/null | head -1)"

  if [ "$admin_code" != "200" ]; then
    record PASS web.adminauth "/admin/settings is gated (${admin_code})"
  elif [ "$pwd_in_config" = "1" ]; then
    # The installer generated a password and wrote it to the config, the panel
    # serves anyway, and doctor read the same config and called it protected.
    record FAIL web.adminopen \
      "CADDY_PWD IS set in ${CONFIG_FILE}, yet /admin/settings serves 200 unauthenticated" \
      "doctor said: ${doctor_verdict:-<no admin line>} — the config password is not reaching the auth path"
  else
    record WARN web.adminopen "/admin/settings answered 200 and no CADDY_PWD is set in the config" \
      "Open by design without a password; doctor should be warning about it too."
  fi
}

phase_watchdog() {
  head1 watchdog "systemd watchdog kills and restarts a hung daemon"

  local pid pings_before
  pid="$(daemon_pid)"
  if [ -z "$pid" ] || [ "$pid" = "0" ]; then
    record FAIL watchdog.pid "service has no main PID — cannot run the test"; return 1
  fi
  pings_before="$(metric_value birdnet_watchdog_pings_total)"
  record INFO watchdog.pings "watchdog pings sent so far: ${pings_before:-0}"

  local wsec
  wsec="$(systemctl show -p WatchdogUSec --value "$SERVICE" 2>/dev/null)"
  record INFO watchdog.config "WatchdogUSec = ${wsec}"

  say "${C_YEL}    SIGSTOPping pid ${pid} to simulate a livelock — systemd should kill it within ~2 min${C_OFF}"
  STOPPED_PID="$pid"          # the EXIT trap will SIGCONT it if we die here
  sudo kill -STOP "$pid"

  # Wait for systemd to notice the missing pings and cycle the unit.
  local deadline=$(( SECONDS + 200 )) restarted=0
  while [ $SECONDS -lt $deadline ]; do
    local now; now="$(daemon_pid)"
    if [ -n "$now" ] && [ "$now" != "0" ] && [ "$now" != "$pid" ]; then restarted=1; break; fi
    sleep 5
  done

  if [ $restarted -eq 1 ]; then
    STOPPED_PID=""            # the old process is gone; nothing to resume
    record PASS watchdog.restart "systemd killed the hung process and restarted it (pid ${pid} → $(daemon_pid))"
    # systemd's own wording varies by version ("Watchdog timeout (limit 2min)!",
    # "watchdog: ...", or just the SIGABRT it sends), and the unit may have been
    # cycling for longer than the SIGSTOP wait, so search a wider window and a
    # wider vocabulary before concluding the restart had another cause.
    if journalctl -u "$SERVICE" --since '-10 min' --no-pager 2>/dev/null \
        | grep -qiE 'watchdog|timeout \(limit|SIGABRT|Killing process'; then
      record PASS watchdog.journal "journal records the watchdog timeout / forced kill" \
        "$(journalctl -u "$SERVICE" --since '-10 min' --no-pager | grep -iE 'watchdog|timeout \(limit|SIGABRT|Killing process' | tail -2 | tr '\n' ' ')"
    else
      record WARN watchdog.journal "no watchdog/kill wording in the journal — restart may have another cause" \
        "The restart itself is proven by the PID change; only the attribution is unconfirmed."
    fi
  else
    sudo kill -CONT "$pid" 2>/dev/null
    STOPPED_PID=""
    record FAIL watchdog.restart "process still stopped after 200s — the watchdog did not fire" \
      "WatchdogSec may be unset, or sd_notify pings are not reaching systemd."
  fi

  wait_for 120 curl -sf --max-time 5 "${BASE}/api/v2/health" \
    && record PASS watchdog.recovered "station is serving again after the forced restart" \
    || record FAIL watchdog.recovered "no health response 120s after the restart"

  snapshot_journal watchdog
}

phase_unplug() {
  head1 unplug "Microphone hot-unplug and reconnect"

  if [ "$(state_get HAS_MIC)" = "0" ]; then
    record SKIP unplug.all "no capture device to unplug"; return 0
  fi
  if [ "$ASSUME_YES" = "1" ]; then
    record SKIP unplug.all "--yes given; this phase needs someone at the board"; return 0
  fi

  # If the gauge is not 1 to begin with, "dropped to 0" proves nothing — it was
  # already 0. Asserting a transition without checking the starting state is the
  # same trap as a model-gated test that reports `ok` whether or not it ran, so
  # refuse to claim the pass (and do not make the operator unplug for nothing).
  local pre; pre="$(source_up_value)"
  if [ "$pre" != "1" ]; then
    record SKIP unplug.all \
      "source gauge is already ${pre:-<absent>} before the unplug — no up→down transition to observe" \
      "Fix capture first, then re-run: ./hardware-test.sh --phase unplug"
    return 0
  fi
  record PASS unplug.precondition "source gauge is 1 before the unplug — a real transition can be observed"

  say ""
  say "${C_YEL}    ACTION REQUIRED: physically unplug the USB microphone now.${C_OFF}"
  confirm "    Unplugged?" || { record SKIP unplug.all "operator declined"; return 0; }

  # The gauge should drop and the supervisor should keep retrying, not give up.
  if wait_for 90 bash -c "[ \"\$(curl -sf '${METRICS_URL}' | awk '/^birdnet_audio_source_up\{/ { print \$2 }' | head -1)\" = '0' ]"; then
    record PASS unplug.gauge "birdnet_audio_source_up dropped to 0"
  else
    record FAIL unplug.gauge "gauge did not drop to 0 within 90s of the unplug"
  fi

  if svc_active; then
    record PASS unplug.survives "daemon stayed up with the device gone (degrades, does not crash)"
  else
    record FAIL unplug.survives "service died when the microphone was removed"
  fi

  if journalctl -u "$SERVICE" --since '-3 min' --no-pager 2>/dev/null | grep -qiE 'audio source (down|DOWN)|still trying'; then
    record PASS unplug.journal "journal shows the source marked down with retries continuing"
  else
    record WARN unplug.journal "no 'source down / still trying' line found in the journal"
  fi

  say ""
  say "${C_YEL}    ACTION REQUIRED: plug the microphone back in.${C_OFF}"
  confirm "    Plugged back in?" || { record SKIP unplug.recover "operator declined"; return 0; }

  # Capped backoff runs 2s → 60s, so allow a couple of cycles.
  if wait_for 180 bash -c "[ \"\$(curl -sf '${METRICS_URL}' | awk '/^birdnet_audio_source_up\{/ { print \$2 }' | head -1)\" = '1' ]"; then
    record PASS unplug.recover "source came back on its own — gauge returned to 1, no operator action"
  else
    record FAIL unplug.recover "source did not recover within 180s of replugging" \
      "Backoff caps at 60s, so three minutes should have been enough."
  fi

  snapshot_journal unplug
}

phase_netloss() {
  head1 netloss "Network loss degrades gracefully and self-recovers"

  local iface
  iface="$(ip -4 route show default | awk '{print $5}' | head -1)"
  if [ -z "$iface" ]; then
    record SKIP netloss.all "no default route interface found"; return 0
  fi
  record INFO netloss.iface "dropping ${iface} for 60s"

  # Pick the outage mechanism. On a NetworkManager-managed box (the Pi OS
  # default) `ip link set … down` is immediately undone by NM, which would make
  # this phase a no-op that looks like a pass. Ask NM to stand down instead.
  local down_cmd up_cmd how
  if have nmcli && systemctl is-active --quiet NetworkManager 2>/dev/null; then
    how="nmcli"; down_cmd="nmcli networking off"; up_cmd="nmcli networking on"
  else
    how="iplink"; down_cmd="ip link set ${iface} down"; up_cmd="ip link set ${iface} up"
  fi
  record INFO netloss.method "using ${how}: ${down_cmd}"

  # If this SSH session rides the interface we are about to drop, arm the
  # recovery in systemd FIRST — that way the link comes back even if this
  # script dies with the connection.
  local armed=0
  sudo systemctl stop birdnet-hwtest-netrestore.timer >/dev/null 2>&1
  sudo systemctl reset-failed birdnet-hwtest-netrestore.service >/dev/null 2>&1
  if have systemd-run; then
    sudo systemd-run --on-active=75 --timer-property=AccuracySec=1s \
      --unit=birdnet-hwtest-netrestore \
      /bin/sh -c "${up_cmd}; sleep 5; ip link set ${iface} up 2>/dev/null || true" \
      >/dev/null 2>&1 && armed=1
  fi
  if [ $armed -eq 1 ]; then
    record PASS netloss.armed "recovery armed via systemd — ${iface} comes back in 75s even if this session dies"
  else
    record WARN netloss.armed "could not arm a systemd recovery timer" \
      "If you are on SSH over ${iface}, abort now: a dropped link may not come back by itself."
    confirm "    Continue anyway?" || { record SKIP netloss.all "operator declined"; return 0; }
  fi

  # shellcheck disable=SC2086  # down_cmd is a fixed command we built above
  sudo $down_cmd
  sleep 60

  # The daemon must survive; only network integrations should complain.
  if svc_active; then
    record PASS netloss.survives "daemon stayed active through 60s with no network"
  else
    record FAIL netloss.survives "service went inactive when the network dropped"
  fi

  local health
  health="$(curl -so /dev/null -w '%{http_code}' --max-time 5 "${BASE}/api/v2/health" 2>/dev/null)"
  if [ "$health" = "200" ]; then
    record PASS netloss.local "dashboard still served on loopback with the link down"
  else
    record FAIL netloss.local "loopback health returned ${health} with the link down"
  fi

  # shellcheck disable=SC2086  # up_cmd is a fixed command we built above
  sudo $up_cmd 2>/dev/null
  sudo ip link set "$iface" up 2>/dev/null
  sudo systemctl stop birdnet-hwtest-netrestore.timer 2>/dev/null
  sudo systemctl reset-failed birdnet-hwtest-netrestore.service 2>/dev/null

  # ICMP is not always permitted; fall back to a TCP round trip before calling it.
  if wait_for 120 bash -c 'ping -c1 -W2 1.1.1.1 >/dev/null 2>&1 || curl -sf --max-time 5 -o /dev/null https://github.com'; then
    record PASS netloss.restored "network is back after ${iface} was brought up"
  else
    record WARN netloss.restored "no external connectivity 120s after bringing ${iface} up" \
      "The armed timer may still be pending; check 'ip link' and 'nmcli networking'."
  fi

  # Nothing should have panicked — a stack trace here is a real bug.
  if journalctl -u "$SERVICE" --since '-4 min' --no-pager 2>/dev/null | grep -qiE 'panicked at|thread .* panicked'; then
    record FAIL netloss.panic "the daemon PANICKED during the network outage"
    snapshot_journal netloss-panic
  else
    record PASS netloss.panic "no panic in the journal across the outage"
  fi

  snapshot_journal netloss
}

phase_diskfull() {
  head1 diskfull "Disk-full purge keeps the station alive"
  discover_paths

  local fs avail_kb total_kb used_pct
  fs="$(df -P "$RECS_DIR" 2>/dev/null | tail -1 | awk '{print $6}')"
  [ -z "$fs" ] && { record SKIP diskfull.all "cannot resolve the filesystem for ${RECS_DIR}"; return 0; }
  read -r total_kb avail_kb used_pct <<<"$(df -P "$fs" | tail -1 | awk '{print $2, $4, $5}' | tr -d '%')"
  record INFO diskfull.fs "${fs} — ${used_pct}% used, $(( avail_kb / 1024 )) MiB free"

  # Two different thresholds have to be crossed for this phase to prove
  # anything, and on most cards they are not the same number:
  #
  #   * the purge fires on a PERCENTAGE (DISK_PURGE_THRESHOLD, 95 by default);
  #   * doctor's disk check grades in ABSOLUTE bytes, and its lowest branch —
  #     the one that used to exit 2 and block startup — needs < 1 GiB free.
  #
  # Filling to 96% alone leaves ~1.3 GiB free on a 32 GB card, which never
  # reaches that branch: the phase would report success without having
  # exercised the startup fix at all. Take whichever target leaves less free.
  local target_pct=96
  local pct_free_kb cap_free_kb want_free_kb ballast_kb
  pct_free_kb=$(( total_kb - (total_kb * target_pct / 100) ))
  cap_free_kb=$(( 900 * 1024 ))            # 900 MiB — under doctor's 1 GiB floor
  want_free_kb=$pct_free_kb
  [ "$cap_free_kb" -lt "$want_free_kb" ] && want_free_kb=$cap_free_kb
  # Never consume the last 200 MiB: a truly full root can wedge journald and sshd.
  [ "$want_free_kb" -lt 204800 ] && want_free_kb=204800
  ballast_kb=$(( avail_kb - want_free_kb ))
  if [ "$ballast_kb" -le 0 ]; then
    record SKIP diskfull.all \
      "only $(( avail_kb / 1024 )) MiB free — already at or past the target, refusing to add more"
    return 0
  fi

  say "${C_YEL}    allocating $(( ballast_kb / 1024 )) MiB of ballast — leaving $(( want_free_kb / 1024 )) MiB free (≥${target_pct}% used, under doctor's 1 GiB floor)${C_OFF}"
  confirm "    Proceed? (the ballast is removed automatically, even on Ctrl-C)" \
    || { record SKIP diskfull.all "operator declined"; return 0; }

  BALLAST="${fs%/}/birdnet-hwtest-ballast.bin"
  if ! sudo fallocate -l "${ballast_kb}K" "$BALLAST" 2>/dev/null; then
    sudo dd if=/dev/zero of="$BALLAST" bs=1M count=$(( ballast_kb / 1024 )) status=none 2>/dev/null
  fi
  local now_pct; now_pct="$(df -P "$fs" | tail -1 | awk '{print $5}' | tr -d '%')"
  record INFO diskfull.filled "${fs} is now ${now_pct}% used"

  if [ "${now_pct:-0}" -lt 95 ]; then
    record SKIP diskfull.trigger "could only reach ${now_pct}% — below the 95% purge threshold"
    sudo rm -f "$BALLAST"; BALLAST=""
    return 0
  fi

  local now_free_mb
  now_free_mb="$(df -P "$fs" | tail -1 | awk '{print int($4/1024)}')"
  record INFO diskfull.free "${now_free_mb} MiB free — doctor's low-space branch needs < 1024"

  # The STARTUP path, not just the running daemon. The defect this phase exists
  # to catch lived on restart: under 1 GiB free, `--doctor` exited 2, the unit's
  #   ExecStartPre=... --doctor ... || [ $? -le 1 ]
  # gate therefore failed, and systemd refused to start the daemon that owns
  # `start_disk_manager` — the purge that would have reclaimed the space. A
  # daemon that is already running sails through all of that untouched, so
  # checking only `svc_active` after filling the disk cannot see the bug.
  local doc_rc
  "$BINARY" --config "$CONFIG_FILE" --doctor > "${OUT}/doctor-diskfull.txt" 2>&1; doc_rc=$?
  if [ "$doc_rc" -le 1 ]; then
    record PASS diskfull.doctor \
      "doctor exits ${doc_rc} with ${now_free_mb} MiB free — the ExecStartPre gate lets startup through" \
      "$(grep -i 'disk space' "${OUT}/doctor-diskfull.txt" | head -1)"
  else
    record FAIL diskfull.doctor \
      "doctor exits ${doc_rc} with ${now_free_mb} MiB free — ExecStartPre will refuse to start the daemon" \
      "Exit 2 means 'errors that will prevent operation'. The purge that fixes a full disk runs inside the process this refuses to start."
  fi

  sudo systemctl restart "$SERVICE"
  if wait_for 120 curl -sf --max-time 5 "${BASE}/api/v2/health"; then
    record PASS diskfull.restart "station restarts and serves with only ${now_free_mb} MiB free"
  else
    record FAIL diskfull.restart "station did not serve within 120s of restarting on a nearly full disk"
    snapshot_journal diskfull-restart
    if systemctl is-failed --quiet "$SERVICE" 2>/dev/null \
       || journalctl -u "$SERVICE" --since '-6 min' --no-pager 2>/dev/null \
          | grep -qi 'start request repeated too quickly'; then
      record FAIL diskfull.startlimit \
        "the unit burned StartLimitBurst and is parked in 'failed'" \
        "An unattended station stays dead until someone runs 'systemctl reset-failed' on site — for the one failure mode a 24/7 recorder is guaranteed to reach."
    fi
  fi

  info "waiting up to 5 minutes for the disk manager to notice and purge…"
  local deadline=$(( SECONDS + 300 )) purged=0
  while [ $SECONDS -lt $deadline ]; do
    if journalctl -u "$SERVICE" --since '-6 min' --no-pager 2>/dev/null \
        | grep -qiE 'purg|disk (full|pressure)|reclaim'; then purged=1; break; fi
    sleep 10
  done

  if [ $purged -eq 1 ]; then
    record PASS diskfull.purge "disk manager reacted to the pressure" \
      "$(journalctl -u "$SERVICE" --since '-6 min' --no-pager | grep -iE 'purg|disk (full|pressure)' | tail -2 | tr '\n' ' ')"
  else
    record WARN diskfull.purge "no purge line in the journal within 5 min" \
      "The maintenance tick may run on a longer interval, or there was nothing old enough to purge."
  fi

  if svc_active; then
    record PASS diskfull.survives "daemon stayed active at ${now_pct}% disk"
  else
    record FAIL diskfull.survives "service died under disk pressure"
  fi

  if journalctl -u "$SERVICE" --since '-6 min' --no-pager 2>/dev/null | grep -qiE 'panicked at'; then
    record FAIL diskfull.panic "the daemon PANICKED under disk pressure"
  else
    record PASS diskfull.panic "no panic under disk pressure"
  fi

  sudo rm -f "$BALLAST"; BALLAST=""
  record INFO diskfull.cleanup "ballast removed — $(df -P "$fs" | tail -1 | awk '{print $5}') used"

  # Never hand the station back worse than we found it. If the restart above
  # parked the unit, the space is back now, so clear the rate limit and start
  # it — the same two commands an operator would otherwise have to know.
  if ! svc_active; then
    sudo systemctl reset-failed "$SERVICE" 2>/dev/null
    sudo systemctl start "$SERVICE"
    if wait_for 120 curl -sf --max-time 5 "${BASE}/api/v2/health"; then
      record PASS diskfull.recovered "station serving again once the space was returned"
    else
      record FAIL diskfull.recovered "station still down after the ballast was removed" \
        "$(systemctl status "$SERVICE" --no-pager -n 5 2>&1 | tail -4 | tr '\n' ' ')"
    fi
  fi

  snapshot_journal diskfull
}

phase_dbcorrupt() {
  head1 dbcorrupt "SQLite corruption is quarantined and recovered"
  discover_paths

  [ -f "$DB_PATH" ] || { record SKIP dbcorrupt.all "no database at ${DB_PATH}"; return 0; }

  # Take our own copy first. The station has its own backups; this is the
  # harness's guarantee that a failed test cannot cost the operator data.
  #
  # Record the original ownership before touching anything. `cp` onto an
  # existing file writes through the inode and keeps its owner, so the restore
  # below is already safe today — this is belt-and-braces for the case where the
  # database has been removed entirely and `cp` would create it root-owned,
  # which the unprivileged `User=` could then not open.
  local safety="${OUT}/birds.db.safety" db_owner
  db_owner="$(stat -c '%U:%G' "$DB_PATH" 2>/dev/null)"
  record INFO dbcorrupt.owner "database is owned by ${db_owner:-unknown}; restores will preserve that"
  sudo systemctl stop "$SERVICE"
  sudo cp "$DB_PATH" "$safety" && sudo chown "$(id -u):$(id -g)" "$safety"
  record INFO dbcorrupt.safety "safety copy at ${safety} ($(du -h "$safety" | cut -f1))"

  # Ask the binary to take a proper backup too — that is the path recovery uses.
  "$BINARY" --config "$CONFIG_FILE" --backup-db > "${OUT}/backup-db.txt" 2>&1 \
    && record PASS dbcorrupt.backup "--backup-db produced a backup before the corruption" \
    || record WARN dbcorrupt.backup "--backup-db exited non-zero (see backup-db.txt)"

  # Preserve the WAL/SHM sidecars too, then remove them: a valid write-ahead log
  # can replay over a damaged main file, which would quietly undo the injection
  # and turn this into a test that always passes for the wrong reason.
  local sidecar
  for sidecar in "${DB_PATH}-wal" "${DB_PATH}-shm"; do
    [ -f "$sidecar" ] || continue
    sudo cp "$sidecar" "${OUT}/$(basename "$sidecar").safety" 2>/dev/null
    sudo rm -f "$sidecar"
    record INFO dbcorrupt.sidecar "removed $(basename "$sidecar") (copy kept in ${OUT})"
  done

  # Scribble over the header — the classic "SD card went bad" signature.
  sudo dd if=/dev/urandom of="$DB_PATH" bs=1024 count=8 conv=notrunc status=none
  record INFO dbcorrupt.inject "overwrote the first 8 KiB of ${DB_PATH} with random bytes"

  local chk_rc
  "$BINARY" --config "$CONFIG_FILE" --check-db > "${OUT}/check-db.txt" 2>&1; chk_rc=$?
  if [ $chk_rc -ne 0 ]; then
    record PASS dbcorrupt.detect "--check-db detects the corruption (exit ${chk_rc})"
  else
    record FAIL dbcorrupt.detect "--check-db reported a healthy database after corruption"
  fi

  sudo systemctl start "$SERVICE"

  if wait_for 180 curl -sf --max-time 5 "${BASE}/api/v2/health"; then
    record PASS dbcorrupt.recover "station came back up on its own after the corruption" \
      "This is the field case that matters: an SD card degrades and nobody is on site."
  else
    record FAIL dbcorrupt.recover "station did not serve within 180s of restarting on a corrupt DB"
    snapshot_journal dbcorrupt-fail

    # Distinguish "the recovery ran and failed" from "the recovery never ran".
    # The unit gates startup on `--doctor ... || [ $? -le 1 ]`, and a corrupt
    # database makes doctor exit 2 — so ExecStartPre fails and the daemon that
    # owns check_and_recover() is never executed. Naming that is the whole value
    # of running this on a real systemd station instead of in a unit test.
    local result execpre
    result="$(systemctl show -p Result --value "$SERVICE" 2>/dev/null)"
    execpre="$(journalctl -u "$SERVICE" --since '-5 min' --no-pager 2>/dev/null \
      | grep -iE 'control process exited|ExecStartPre|start-pre' | tail -1)"
    if [ "$result" = "exit-code" ] || [ -n "$execpre" ]; then
      record FAIL dbcorrupt.gate \
        "startup was blocked by the ExecStartPre doctor gate — the recovery path never ran" \
        "Result=${result}; ${execpre:-no ExecStartPre line}. A corrupt DB makes --doctor exit 2, so systemd refuses to start the daemon, so check_and_recover() cannot execute."
    fi

    # RestartSec=10 against StartLimitBurst=5 means the five permitted restarts
    # are spent in under a minute, after which systemd parks the unit and
    # refuses every later `systemctl start` — including the one that would have
    # worked once the cause was fixed. Naming it matters: an unattended station
    # stays dead until someone runs `reset-failed` on site.
    if systemctl is-failed --quiet "$SERVICE" 2>/dev/null \
       || journalctl -u "$SERVICE" --since '-6 min' --no-pager 2>/dev/null | grep -qi 'start request repeated too quickly'; then
      record FAIL dbcorrupt.startlimit \
        "the unit burned StartLimitBurst=5 and is parked in 'failed'" \
        "Even after the database is repaired, systemd refuses to start it until 'systemctl reset-failed' is run by hand."
    fi
  fi

  if journalctl -u "$SERVICE" --since '-5 min' --no-pager 2>/dev/null \
      | grep -qiE 'quarantin|corrupt|restore|restoring|recover'; then
    record PASS dbcorrupt.journal "journal shows the quarantine/restore path running" \
      "$(journalctl -u "$SERVICE" --since '-5 min' --no-pager | grep -iE 'quarantin|corrupt|restore|restoring' | tail -2 | tr '\n' ' ')"
  else
    record WARN dbcorrupt.journal "no quarantine/restore wording found in the journal"
  fi

  # Whatever happened, the station must end this phase with a working database.
  local final_rc
  "$BINARY" --config "$CONFIG_FILE" --check-db > "${OUT}/check-db-after.txt" 2>&1; final_rc=$?
  if [ $final_rc -eq 0 ]; then
    record PASS dbcorrupt.healthy "database passes its integrity check again"
  else
    record FAIL dbcorrupt.healthy "database is still unhealthy — restoring the harness safety copy"
    sudo systemctl stop "$SERVICE"
    sudo systemctl reset-failed "$SERVICE" 2>/dev/null
    sudo cp "$safety" "$DB_PATH"
    # Restore the original ownership. `cp` as root would otherwise leave the
    # file root-owned and the unprivileged service could not open it.
    [ -n "$db_owner" ] && sudo chown "$db_owner" "$DB_PATH"
    sudo rm -f "${DB_PATH}-wal" "${DB_PATH}-shm"
    sudo systemctl start "$SERVICE"
    if wait_for 120 curl -sf --max-time 5 "${BASE}/api/v2/health"; then
      record PASS dbcorrupt.restored "safety copy restored (owner ${db_owner:-unchanged}) and the station is serving again"
    else
      record FAIL dbcorrupt.restored "station still will not start after restoring the safety copy" \
        "$(systemctl status "$SERVICE" --no-pager -n 5 2>&1 | tail -4 | tr '\n' ' ')"
    fi
  fi

  snapshot_journal dbcorrupt
}

phase_duckdb() {
  head1 duckdb "Analytics store rebuilds itself after corruption"
  discover_paths

  [ -f "$ANALYTICS_DB" ] || { record SKIP duckdb.all "no analytics database at ${ANALYTICS_DB}"; return 0; }

  local before_size; before_size="$(stat -c%s "$ANALYTICS_DB")"
  sudo systemctl stop "$SERVICE"
  sudo dd if=/dev/urandom of="$ANALYTICS_DB" bs=1024 count=16 conv=notrunc status=none
  record INFO duckdb.inject "corrupted the first 16 KiB of ${ANALYTICS_DB} (was ${before_size} bytes)"
  sudo systemctl start "$SERVICE"

  if wait_for 180 curl -sf --max-time 5 "${BASE}/api/v2/health"; then
    record PASS duckdb.recover "station started on a corrupt analytics store" \
      "The analytics DB is derived from SQLite, so the correct behaviour is rebuild, not refuse."
  else
    record FAIL duckdb.recover "station did not come up within 180s on a corrupt analytics store"
    snapshot_journal duckdb-fail

    # The analytics store is derived from SQLite, so refusing to start over it is
    # strictly worse than discarding it. Prove that is the actual blocker by
    # removing the file and restarting — and leave the station working either
    # way, rather than handing the operator a dead box.
    local execpre
    execpre="$(journalctl -u "$SERVICE" --since '-5 min' --no-pager 2>/dev/null \
      | grep -iE 'control process exited|ExecStartPre|start-pre' | tail -1)"
    [ -n "$execpre" ] && record FAIL duckdb.gate \
      "the ExecStartPre doctor gate blocked startup over a *regenerable* store" "${execpre}"

    sudo systemctl stop "$SERVICE"
    sudo systemctl reset-failed "$SERVICE" 2>/dev/null
    sudo rm -f "$ANALYTICS_DB"
    sudo systemctl start "$SERVICE"
    if wait_for 180 curl -sf --max-time 5 "${BASE}/api/v2/health"; then
      record FAIL duckdb.rebuild_manual \
        "station starts once the corrupt analytics store is DELETED, but not while it exists" \
        "The file is regenerable from SQLite, so the product should discard and rebuild it itself instead of requiring an operator on site."
    else
      record FAIL duckdb.rebuild_manual "station will not start even with ${ANALYTICS_DB} removed" \
        "$(systemctl status "$SERVICE" --no-pager -n 5 2>&1 | tail -4 | tr '\n' ' ')"
    fi
  fi

  # Analytics pages must actually work again, not just fail quietly.
  local code
  code="$(curl -so /dev/null -w '%{http_code}' --max-time 20 "${BASE}/api/v2/analytics/status" 2>/dev/null)"
  case "$code" in
    200) record PASS duckdb.analytics "analytics endpoint answers 200 after the rebuild" ;;
    404) record SKIP duckdb.analytics "analytics endpoint not present at that path (404)" ;;
    *)   record WARN duckdb.analytics "analytics endpoint returned ${code} after the rebuild" ;;
  esac

  if journalctl -u "$SERVICE" --since '-5 min' --no-pager 2>/dev/null \
      | grep -qiE 'rebuild(ing)? (the )?analytics|analytics .*(rebuilt|recreated|quarantin)|discarding .*analytics'; then
    record PASS duckdb.journal "journal shows the analytics store actually being rebuilt"
  else
    record WARN duckdb.journal "no analytics-rebuild wording in the journal" \
      "Matching a bare 'analytics' would pass on any ordinary log line, so this asks for the rebuild itself"
  fi

  snapshot_journal duckdb
}

phase_reboot() {
  head1 reboot "Cold reboot brings the station back unattended"

  if [ "$(state_get REBOOT_ARMED)" = "1" ]; then
    # We are the post-reboot half of this phase.
    local up; up="$(awk '{print int($1)}' /proc/uptime)"
    record INFO reboot.uptime "uptime is ${up}s — this is the run after the reboot"
    if [ "$up" -lt 3600 ]; then
      record PASS reboot.happened "the board did reboot"
    else
      record WARN reboot.happened "uptime ${up}s looks too long for a fresh boot"
    fi

    if wait_for 240 systemctl is-active --quiet "$SERVICE"; then
      record PASS reboot.autostart "service came back by itself after the reboot"
    else
      record FAIL reboot.autostart "service is not active 240s after boot" \
        "Check 'systemctl is-enabled ${SERVICE}'."
    fi

    if wait_for 240 curl -sf --max-time 5 "${BASE}/api/v2/health"; then
      record PASS reboot.serving "dashboard is serving again after the reboot"
    else
      record FAIL reboot.serving "no health response 240s after boot"
    fi

    if [ "$(state_get HAS_MIC)" = "1" ]; then
      if wait_for 180 bash -c "[ \"\$(curl -sf '${METRICS_URL}' | awk '/^birdnet_audio_source_up\{/ { print \$2 }' | head -1)\" = '1' ]"; then
        record PASS reboot.capture "capture resumed after the reboot (gauge back to 1)"
      else
        record FAIL reboot.capture "capture did not resume within 180s of boot"
      fi
    fi

    state_set REBOOT_ARMED 0
    snapshot_journal reboot
    return 0
  fi

  check reboot.enabled "service is enabled for boot" systemctl is-enabled --quiet "$SERVICE"

  say ""
  say "${C_YEL}    The board will now reboot. When it comes back, reconnect and run:${C_OFF}"
  say "${C_BLD}        cd $(dirname "$OUT") && $(readlink -f "$0") --resume${C_OFF}"
  say ""
  if ! confirm "    Reboot now?"; then
    record SKIP reboot.all "operator declined the reboot"
    return 0
  fi

  state_set REBOOT_ARMED 1
  state_set REBOOT_AT "$(date +%s)"
  say "${C_YEL}    rebooting…${C_OFF}"
  sync
  sudo systemctl reboot
  exit 0
}

phase_report() {
  head1 report "Summary"
  discover_paths

  local report="${OUT}/report.md"
  local model; model="$(tr -d '\0' < /proc/device-tree/model 2>/dev/null)"

  {
    echo "# BirdNet-Behavior hardware acceptance report"
    echo
    echo "- **Board:** ${model:-unknown}"
    echo "- **OS:** $(. /etc/os-release && echo "$PRETTY_NAME") · kernel $(uname -r) · $(uname -m)"
    echo "- **glibc:** $(getconf GNU_LIBC_VERSION 2>/dev/null)"
    echo "- **Version under test:** $(state_get VERSION)"
    echo "- **Run:** $(date -Is)"
    echo "- **Artifacts:** \`${OUT}\`"
    echo
    echo "## Result"
    echo
    echo "| Outcome | Count |"
    echo "|---|---|"
    echo "| PASS | ${PASS_N} |"
    echo "| FAIL | ${FAIL_N} |"
    echo "| WARN | ${WARN_N} |"
    echo "| SKIP | ${SKIP_N} |"
    echo
    echo "## Measurements"
    echo
    echo "| Metric | Value |"
    echo "|---|---|"
    echo "| Install wall time | $(state_get INSTALL_SECS) s |"
    echo "| Mean inference latency | $(state_get MEAN_LATENCY_MS) ms per 3 s chunk |"
    echo "| Idle SoC temperature | $(state_get TEMP_IDLE) °C |"
    echo "| Peak SoC temperature | see \`perf-samples.csv\` |"
    echo "| Throttle register | $(throttled_hex) |"
    echo "| Daemon RSS now | $(( $(daemon_rss_kb 2>/dev/null || echo 0) / 1024 )) MiB |"
    echo
    echo "## Findings by phase"
    echo
    local ph
    for ph in "${PHASES[@]}"; do
      grep -F "\"phase\":\"${ph}\"" "$RESULTS" >/dev/null 2>&1 || continue
      echo "### ${ph}"
      echo
      # shellcheck disable=SC2016
      sed -n "s/.*\"phase\":\"${ph}\",\"status\":\"\([A-Z]*\)\",\"id\":\"\([^\"]*\)\",\"desc\":\"\([^\"]*\)\".*/- **\1** \`\2\` — \3/p" "$RESULTS"
      echo
    done
    echo "## Failures to triage"
    echo
    if [ "$FAIL_N" -eq 0 ]; then
      echo "None."
    else
      sed -n 's/.*"status":"FAIL","id":"\([^"]*\)","desc":"\([^"]*\)","detail":"\([^"]*\)".*/- `\1` — \2  \n  <br>\3/p' "$RESULTS"
    fi
    echo
    echo "---"
    echo
    echo "Generated by \`scripts/hardware-test.sh\`."
  } > "$report"

  say ""
  say "${C_BLD}  PASS ${PASS_N} · FAIL ${FAIL_N} · WARN ${WARN_N} · SKIP ${SKIP_N}${C_OFF}"
  say ""
  say "  Report:  ${report}"
  say "  Raw:     ${RESULTS}"
  say "  Journals + env + samples: ${OUT}/"
  say ""
  if [ "$FAIL_N" -gt 0 ]; then
    say "${C_RED}  ${FAIL_N} check(s) failed — see 'Failures to triage' in the report.${C_OFF}"
  else
    say "${C_GRN}  No failures.${C_OFF}"
  fi
}

# ===========================================================================
# Driver
# ===========================================================================

main() {
  sudo -v || { echo "sudo access is required" >&2; exit 1; }

  say "${C_BLD}BirdNet-Behavior hardware acceptance harness${C_OFF}"
  say "${C_DIM}artifacts: ${OUT}${C_OFF}"

  # The netloss phase drops the network and the reboot phase reboots the board.
  # Over SSH that kills this shell mid-run unless it is detached from the
  # session, so say so once, up front, while it can still be acted on.
  if [ -n "${SSH_CONNECTION:-}" ] && [ -z "${TMUX:-}" ] && [ -z "${STY:-}" ] \
     && { [ ${#SELECTED[@]} -eq 0 ] || [[ " ${SELECTED[*]} " == *" netloss "* ]]; } \
     && [ "$SAFE_ONLY" != "1" ]; then
    say ""
    say "${C_YEL}You are on SSH and not inside tmux or screen.${C_OFF}"
    say "${C_YEL}The netloss phase drops this box's network — that will kill this session${C_OFF}"
    say "${C_YEL}and the run with it. Strongly consider:  tmux new -s hwtest${C_OFF}"
    say "${C_DIM}(the network itself always recovers: a systemd timer is armed first)${C_OFF}"
    confirm "Continue without tmux?" || exit 0
  fi

  # Record skips up front, before anything runs. Writing them into the state
  # file is what makes the post-reboot `--resume` honour them: resume replays
  # every phase not marked done, so a phase merely absent from this invocation
  # would come back after the reboot. For `install` that would fetch the
  # published release over the binary under test, and every later result would
  # silently describe a different build.
  local sp
  for sp in ${SKIPPED[@]+"${SKIPPED[@]}"}; do
    if ! declare -F "phase_${sp}" >/dev/null; then
      say "${C_RED}unknown phase in --skip: ${sp} (try --list)${C_OFF}"
      exit 64
    fi
    CURRENT_PHASE="$sp"
    record SKIP "${sp}.skipped" "phase skipped by --skip"
    state_set "PHASE_${sp}" "done"
  done
  CURRENT_PHASE="-"

  local to_run=()
  if [ ${#SELECTED[@]} -gt 0 ]; then
    to_run=("${SELECTED[@]}")
  else
    for p in "${PHASES[@]}"; do
      # Checked before the resume logic below, which force-replays reboot and
      # report: an explicit --skip outranks that.
      [[ " ${SKIPPED[*]-} " == *" $p "* ]] && continue
      [ "$SAFE_ONLY" = "1" ] && [[ "$DESTRUCTIVE" == *" $p "* ]] && continue
      # On --resume, skip what already completed, but always re-enter reboot
      # (its post-boot half) and report.
      if [ "$RESUME" = "1" ] && phase_done "$p" && [ "$p" != "reboot" ] && [ "$p" != "report" ]; then
        continue
      fi
      to_run+=("$p")
    done
  fi

  for p in "${to_run[@]}"; do
    if ! declare -F "phase_${p}" >/dev/null; then
      echo "unknown phase: ${p}" >&2; continue
    fi
    "phase_${p}"
    local prc=$?
    state_set "PHASE_${p}" "done"
    # Everything downstream reads the installed station. If the install itself
    # never completed, the rest is noise rather than evidence — stop and report.
    if [ "$p" = "install" ] && [ $prc -ne 0 ]; then
      say ""
      say "${C_RED}Install failed — skipping the remaining phases (they would all fail for the same reason).${C_OFF}"
      say "${C_DIM}See ${OUT}/install.log, fix the cause, then re-run.${C_OFF}"
      phase_report
      exit 1
    fi
  done

  [ "$FAIL_N" -gt 0 ] && exit 1
  exit 0
}

main "$@"
