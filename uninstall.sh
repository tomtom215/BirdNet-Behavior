#!/usr/bin/env bash
# uninstall.sh — safe, idempotent uninstaller for BirdNet-Behavior.
#
# Reverses what install.sh sets up on a bare-metal systemd host (and handles the
# macOS launchd LaunchAgent). It is:
#   • SAFE by default      — removes only the *software* (service, binary, tmpfs
#                            unit); your database, recordings, settings and the
#                            downloaded model are KEPT unless you opt in.
#   • IDEMPOTENT           — re-running is harmless; already-gone items are skipped.
#   • DETERMINISTIC        — real paths are read from the installed config/service,
#                            not re-guessed, so it removes exactly what was installed.
#   • FOOL-PROOF           — refuses to delete protected/again-ambiguous paths,
#                            shows a plan, and asks before destroying data.
#
# Usage:
#   sudo ./uninstall.sh                 # remove software, KEEP all data
#   sudo ./uninstall.sh --dry-run       # show the plan, change nothing
#   sudo ./uninstall.sh --purge         # remove EVERYTHING (data, config, model)
#   sudo ./uninstall.sh --remove-db --remove-recordings   # pick what to delete
#
# Data flags (opt-in; default keeps each):
#   --remove-db            delete the database (birds.db + analytics + backups)
#   --remove-recordings    delete the saved birdsong audio (recordings/)
#   --remove-config        delete /etc/birdnet (your settings)
#   --remove-models        delete the downloaded BirdNET model (~541 MB)
#   --remove-image-cache   delete cached species images
#   --remove-zram          remove the zram-swap service install.sh may have added
#   --purge                shorthand for all of the above + logs
#
# Other flags:
#   --keep-binary          leave /usr/local/bin/birdnet-behavior in place
#   --data-dir DIR         override the data directory (else auto-detected)
#   --config-file FILE     override the config path (default /etc/birdnet/birdnet.conf)
#   -y, --yes              don't prompt for confirmation (for automation)
#   --dry-run              print the plan only
#   -h, --help             this help
set -euo pipefail

# ── Defaults (mirror install.sh) ────────────────────────────────────────────
BINARY_NAME="birdnet-behavior"
BIN_PATH="/usr/local/bin/${BINARY_NAME}"
HELP_DIR="/usr/local/share/birdnet-behavior/help"
CONFIG_DIR="/etc/birdnet"
CONFIG_FILE="${CONFIG_DIR}/birdnet.conf"
SERVICE_FILE="/etc/systemd/system/birdnet-behavior.service"
SERVICE_UNIT="birdnet-behavior.service"
TMPFS_UNIT='tmp-birdnet\x2dstream.mount'
TMPFS_UNIT_FILE='/etc/systemd/system/tmp-birdnet\x2dstream.mount'
ZRAM_UNIT="zram-swap.service"
ZRAM_FILE="/etc/systemd/system/zram-swap.service"
STREAM_DIR="/tmp/birdnet-stream"
MAC_LABEL="com.tomtom215.birdnet-behavior"

PURGE=0; REMOVE_DB=0; REMOVE_RECS=0; REMOVE_CONFIG=0; REMOVE_MODELS=0
REMOVE_IMAGE_CACHE=0; REMOVE_ZRAM=0; KEEP_BINARY=0; DRY_RUN=0; ASSUME_YES=0
DATA_DIR_OVERRIDE=""

# ── Output helpers ──────────────────────────────────────────────────────────
# ANSI-C quoting ($'…') stores real ESC bytes, so the colours render whether
# they land in a printf format or a %s argument (plain '\033…' only renders in
# the format position, which printed literal escape codes in the plan output).
if [ -t 1 ]; then R=$'\033[0;31m'; G=$'\033[0;32m'; Y=$'\033[1;33m'; B=$'\033[1m'; Z=$'\033[0m'
else R=''; G=''; Y=''; B=''; Z=''; fi
info()    { printf "${B}[*]${Z} %s\n" "$*"; }
ok()      { printf "${G}[ok]${Z} %s\n" "$*"; }
warn()    { printf "${Y}[warn]${Z} %s\n" "$*"; }
err()     { printf "${R}[error]${Z} %s\n" "$*" >&2; }
die()     { err "$*"; exit 1; }

usage() { sed -n '2,40p' "$0" | sed 's/^# \{0,1\}//'; exit 0; }

# ── Parse args ──────────────────────────────────────────────────────────────
while [ $# -gt 0 ]; do
  case "$1" in
    --purge) PURGE=1 ;;
    --remove-db) REMOVE_DB=1 ;;
    --remove-recordings) REMOVE_RECS=1 ;;
    --remove-config) REMOVE_CONFIG=1 ;;
    --remove-models) REMOVE_MODELS=1 ;;
    --remove-image-cache) REMOVE_IMAGE_CACHE=1 ;;
    --remove-zram) REMOVE_ZRAM=1 ;;
    --keep-binary) KEEP_BINARY=1 ;;
    --data-dir) DATA_DIR_OVERRIDE="${2:-}"; shift ;;
    --config-file) CONFIG_FILE="${2:-}"; shift ;;
    -y|--yes) ASSUME_YES=1 ;;
    --dry-run) DRY_RUN=1 ;;
    -h|--help) usage ;;
    *) die "unknown option: $1 (try --help)" ;;
  esac
  shift
done
if [ "$PURGE" = 1 ]; then
  REMOVE_DB=1; REMOVE_RECS=1; REMOVE_CONFIG=1; REMOVE_MODELS=1
  REMOVE_IMAGE_CACHE=1; REMOVE_ZRAM=1
fi

# ── Safe removal: guard catastrophic targets, idempotent, dry-run aware ──────
PROTECTED_RE='^/$|^/bin$|^/sbin$|^/etc$|^/home$|^/root$|^/usr$|^/usr/local$|^/usr/local/bin$|^/var$|^/var/log$|^/tmp$|^/opt$|^/Users$|^/Library$|^/Applications$|^/System$|^/private$'
rm_path() { # $1=path  $2=label
  local orig="$1" p label
  [ -n "$orig" ] || return 0        # genuinely-unset path → nothing to do
  p="${orig%/}"; label="${2:-$orig}"
  # `/` becomes "" after stripping the trailing slash — treat that, $HOME, and
  # the system-directory denylist as hard refusals.
  # A here-string, not `printf | grep -q`: this is the guard standing between
  # this script and `rm -rf` on a system directory, and a pipeline whose
  # consumer quits early can report failure under `set -o pipefail` even when it
  # matched. Not measurable for a producer this small (0 failures in 3000 runs),
  # but a delete guard is not where a theoretical race is worth keeping.
  if [ -z "$p" ] || [ "$p" = "${HOME%/}" ] || grep -qE "$PROTECTED_RE" <<<"$p"; then
    err "refusing to remove protected path: ${orig}"; return 1
  fi
  case "$p" in /*) ;; *) err "refusing non-absolute path: ${orig}"; return 1 ;; esac
  if [ ! -e "$p" ]; then info "already gone: ${label}"; return 0; fi
  if [ "$DRY_RUN" = 1 ]; then echo "  would remove  ${label}  ->  $p"; return 0; fi
  rm -rf -- "$p" && ok "removed ${label} ($p)"
}

confirm() { # $1=prompt
  [ "$ASSUME_YES" = 1 ] && return 0
  [ "$DRY_RUN" = 1 ] && return 0
  printf "${Y}%s${Z} [y/N] " "$1"
  local a=""; read -r a </dev/tty 2>/dev/null || a=""
  case "$a" in y|Y|yes|YES) return 0 ;; *) return 1 ;; esac
}

# `DRY_RUN` is "0" or "1" — both non-empty, so `${DRY_RUN:+…}` would always
# expand. Use an explicit label instead.
DRY_LABEL=""; [ "$DRY_RUN" = 1 ] && DRY_LABEL="  (dry-run — nothing will change)"

# ── macOS branch (launchd) ──────────────────────────────────────────────────
if [ "$(uname -s)" = "Darwin" ]; then
  info "macOS detected — handling the launchd LaunchAgent.${DRY_LABEL}"
  PLIST="$HOME/Library/LaunchAgents/${MAC_LABEL}.plist"
  MAC_DATA="$HOME/Library/Application Support/birdnet-behavior"
  if [ -f "$PLIST" ]; then
    if [ "$DRY_RUN" = 1 ]; then echo "  would unload + remove the LaunchAgent ($PLIST)"
    else
      launchctl unload -w "$PLIST" 2>/dev/null || true
      rm -f "$PLIST" && ok "removed LaunchAgent ($PLIST)"
    fi
  else
    info "no LaunchAgent loaded ($PLIST) — nothing to unload"
  fi

  if [ "$REMOVE_DB" = 1 ] || [ "$REMOVE_RECS" = 1 ] || [ "$REMOVE_CONFIG" = 1 ] || [ "$PURGE" = 1 ]; then
    if [ -e "$MAC_DATA" ] || [ -e "$HOME/Library/Logs/birdnet-behavior.log" ]; then
      if confirm "Remove $MAC_DATA and logs?"; then
        rm_path "$MAC_DATA" "macOS data dir"
        rm_path "$HOME/Library/Logs/birdnet-behavior.log" "log"
        rm_path "$HOME/Library/Logs/birdnet-behavior.err.log" "error log"
      else info "kept data (declined)."; fi
    else
      info "no data directory at $MAC_DATA — nothing to remove"
    fi
  elif [ -e "$MAC_DATA" ]; then
    echo "  Data kept at: $MAC_DATA"
    echo "  Remove later with: ./uninstall.sh --purge"
  fi

  if command -v brew >/dev/null 2>&1 && brew list birdnet-behavior >/dev/null 2>&1; then
    warn "Installed via Homebrew — also run: brew uninstall birdnet-behavior"
  fi
  SCRIPT_DIR="$(cd "$(dirname "$0")" 2>/dev/null && pwd || echo .)"
  if [ -x "${SCRIPT_DIR}/target/release/birdnet-behavior" ]; then
    info "Note: your from-source binary (target/release/birdnet-behavior) is left in place — 'cargo clean' or delete the repo to remove it."
  fi
  echo
  if [ "$DRY_RUN" = 1 ]; then info "Dry run only — nothing was changed."
  else ok "macOS uninstall complete."; fi
  exit 0
fi

# ── Linux: require root for system paths ────────────────────────────────────
if [ "$(id -u)" -ne 0 ]; then
  die "needs root for /etc, /usr/local and systemd — re-run with: sudo $0 $*"
fi

# ── Deterministic path detection (read the real install) ────────────────────
# Both helpers print the value or nothing, and always succeed: a missing key
# makes grep exit non-zero, which under `set -e -o pipefail` would otherwise
# kill the script mid-detection. The trailing `|| true` keeps them quiet.
read_conf() { # $1=key  -> value or empty
  [ -f "$CONFIG_FILE" ] || return 0
  { grep -E "^[[:space:]]*$1[[:space:]]*=" "$CONFIG_FILE" 2>/dev/null \
      | tail -1 | cut -d= -f2- | sed 's/^[[:space:]]*//;s/[[:space:]]*$//'; } || true
}
svc_flag() { # $1=flag  -> value or empty (from ExecStart=)
  [ -f "$SERVICE_FILE" ] || return 0
  { grep -E '^ExecStart=' "$SERVICE_FILE" 2>/dev/null | tail -1 \
      | grep -oE "$1[ =][^ ]+" | awk 'NR==1' | sed -E "s/^$1[ =]//"; } || true
}

DB_PATH="$(read_conf DB_PATH)"
RECS_DIR="$(read_conf RECS_DIR)"
[ -z "$DB_PATH" ]  && DB_PATH="$(svc_flag --analytics-db)"   # analytics db sits in DATA_DIR too
IMAGE_CACHE_DIR="$(svc_flag --image-cache-dir)"
ANALYTICS_DB="$(svc_flag --analytics-db)"
WATCH_DIR="$(svc_flag --watch-dir)"; [ -n "$WATCH_DIR" ] && STREAM_DIR="$WATCH_DIR"

# DATA_DIR: override > parent of DB_PATH > parent of recordings > best-effort default
DATA_DIR="$DATA_DIR_OVERRIDE"
if [ -z "$DATA_DIR" ] && [ -n "$DB_PATH" ];   then DATA_DIR="$(dirname "$DB_PATH")"; fi
if [ -z "$DATA_DIR" ] && [ -n "$RECS_DIR" ];  then DATA_DIR="$(dirname "$RECS_DIR")"; fi
if [ -z "$DATA_DIR" ]; then
  _home="$(getent passwd "${SUDO_USER:-root}" 2>/dev/null | cut -d: -f6 || true)"; _home="${_home:-$HOME}"
  DATA_DIR="${_home}/BirdNet-Behavior"
  DATA_DIR_GUESSED=1
fi
[ -z "$RECS_DIR" ]        && RECS_DIR="${DATA_DIR}/recordings"
[ -z "$IMAGE_CACHE_DIR" ] && IMAGE_CACHE_DIR="${DATA_DIR}/image_cache"
[ -z "$ANALYTICS_DB" ]    && ANALYTICS_DB="${DATA_DIR}/analytics.db"
MODEL_DIR="${DATA_DIR}/models"
BACKUPS_DIR="${DATA_DIR}/backups"
DB_PATH="${DB_PATH:-${DATA_DIR}/birds.db}"

# Record whether a native (systemd) install is present, to advise Docker users.
HAD_NATIVE=0
if [ -f "$SERVICE_FILE" ] || [ -e "$BIN_PATH" ]; then HAD_NATIVE=1; fi

# ── Plan ─────────────────────────────────────────────────────────────────────
echo
info "${B}BirdNet-Behavior uninstaller${Z}${DRY_LABEL}"
echo "  Software (always removed): systemd service, tmpfs mount unit, ${STREAM_DIR}$([ "$KEEP_BINARY" = 1 ] && echo "" || echo ", binary, operator manual")"
echo "  Detected data dir:         ${DATA_DIR}$([ "${DATA_DIR_GUESSED:-0}" = 1 ] && echo "  (guessed — config already gone)")"
plan_line() { printf "    %-18s %s\n" "$1" "$2"; }
plan_line "database"      "$([ "$REMOVE_DB" = 1 ] && echo REMOVE || echo keep)   (${DB_PATH}, ${ANALYTICS_DB}, ${BACKUPS_DIR})"
plan_line "recordings"    "$([ "$REMOVE_RECS" = 1 ] && echo REMOVE || echo keep)   (${RECS_DIR})"
plan_line "settings"      "$([ "$REMOVE_CONFIG" = 1 ] && echo REMOVE || echo keep)   (${CONFIG_DIR})"
plan_line "model (~541MB)" "$([ "$REMOVE_MODELS" = 1 ] && echo REMOVE || echo keep)   (${MODEL_DIR})"
plan_line "image cache"   "$([ "$REMOVE_IMAGE_CACHE" = 1 ] && echo REMOVE || echo keep)   (${IMAGE_CACHE_DIR})"
plan_line "zram-swap"     "$([ "$REMOVE_ZRAM" = 1 ] && echo REMOVE || echo keep)   (${ZRAM_FILE})"
echo
if [ "$REMOVE_DB" = 1 ] || [ "$REMOVE_RECS" = 1 ] || [ "$REMOVE_CONFIG" = 1 ] || [ "$REMOVE_MODELS" = 1 ]; then
  if [ "${DATA_DIR_GUESSED:-0}" = 1 ] && [ -z "$DATA_DIR_OVERRIDE" ]; then
    die "config + service are already gone, so the data directory had to be guessed ($DATA_DIR) — refusing to delete data from a guessed path. Re-run the same command with the path made explicit (add -y to skip the prompt):  --data-dir $DATA_DIR"
  fi
  confirm "This will permanently DELETE the data marked REMOVE above. Continue?" \
    || die "aborted — nothing changed."
fi

# ── Execute (idempotent) ─────────────────────────────────────────────────────
if command -v systemctl >/dev/null 2>&1; then
  for unit in "$SERVICE_UNIT" "$TMPFS_UNIT"; do
    if systemctl is-active --quiet "$unit" 2>/dev/null; then
      [ "$DRY_RUN" = 1 ] && echo "  would stop ${unit}" || { systemctl stop "$unit" 2>/dev/null || true; ok "stopped ${unit}"; }
    fi
    if systemctl is-enabled --quiet "$unit" 2>/dev/null; then
      [ "$DRY_RUN" = 1 ] && echo "  would disable ${unit}" || { systemctl disable "$unit" 2>/dev/null || true; }
    fi
  done
  if [ "$REMOVE_ZRAM" = 1 ]; then
    systemctl is-enabled --quiet "$ZRAM_UNIT" 2>/dev/null && { [ "$DRY_RUN" = 1 ] && echo "  would disable ${ZRAM_UNIT}" || systemctl disable --now "$ZRAM_UNIT" 2>/dev/null || true; }
  fi
fi

rm_path "$SERVICE_FILE" "systemd service unit"
rm_path "$TMPFS_UNIT_FILE" "tmpfs mount unit"
[ "$REMOVE_ZRAM" = 1 ] && rm_path "$ZRAM_FILE" "zram-swap unit"
rm_path "$STREAM_DIR" "tmpfs stream dir"
[ "$KEEP_BINARY" = 1 ] || rm_path "$BIN_PATH" "binary"
# The bundled operator manual is software, removed with the binary. Tidy the
# now-empty parent dir too (best-effort; ignored if other files live there).
[ "$KEEP_BINARY" = 1 ] || { rm_path "$HELP_DIR" "operator manual"; [ "$DRY_RUN" = 1 ] || rmdir "$(dirname "$HELP_DIR")" 2>/dev/null || true; }

if command -v systemctl >/dev/null 2>&1 && [ "$DRY_RUN" != 1 ]; then
  systemctl daemon-reload 2>/dev/null || true
fi

[ "$REMOVE_CONFIG" = 1 ]      && rm_path "$CONFIG_DIR" "settings"
[ "$REMOVE_RECS" = 1 ]        && rm_path "$RECS_DIR" "recordings"
[ "$REMOVE_IMAGE_CACHE" = 1 ] && rm_path "$IMAGE_CACHE_DIR" "image cache"
[ "$REMOVE_MODELS" = 1 ]      && rm_path "$MODEL_DIR" "model"
if [ "$REMOVE_DB" = 1 ]; then
  for f in "$DB_PATH" "${DB_PATH}-wal" "${DB_PATH}-shm" "$ANALYTICS_DB" "${ANALYTICS_DB}-wal" "${ANALYTICS_DB}.wal"; do
    rm_path "$f" "$(basename "$f")"
  done
  rm_path "$BACKUPS_DIR" "backups"
fi
# Remove the (now-empty) data dir only if we removed everything in it.
if [ "$REMOVE_DB" = 1 ] && [ "$REMOVE_RECS" = 1 ] && [ "$REMOVE_MODELS" = 1 ] && [ "$REMOVE_IMAGE_CACHE" = 1 ] \
   && [ "$DRY_RUN" != 1 ] && [ -d "$DATA_DIR" ] && [ -z "$(ls -A "$DATA_DIR" 2>/dev/null)" ]; then
  rm_path "$DATA_DIR" "data dir (now empty)"
fi

echo
if [ "$DRY_RUN" = 1 ]; then
  info "Dry run only — nothing was changed. Re-run without --dry-run to apply."
else
  ok "Uninstall complete."
fi
if [ "$HAD_NATIVE" = 0 ]; then
  warn "No systemd service or binary found at the standard paths."
  echo "  If you run BirdNet-Behavior in Docker, tear it down with Compose instead"
  echo "  (from the directory with your docker-compose.yml):"
  echo "    docker compose down       # keep named volumes"
  echo "    docker compose down -v    # also remove the database volume"
fi
# What was kept, with the one-liner to finish the job.
KEPT=()
[ "$REMOVE_DB" = 0 ]          && KEPT+=("database ($DATA_DIR)")
[ "$REMOVE_RECS" = 0 ]       && KEPT+=("recordings ($RECS_DIR)")
[ "$REMOVE_CONFIG" = 0 ] && [ -d "$CONFIG_DIR" ] && KEPT+=("settings ($CONFIG_DIR)")
[ "$REMOVE_MODELS" = 0 ] && [ -d "$MODEL_DIR" ] && KEPT+=("model ($MODEL_DIR)")
if [ "${#KEPT[@]}" -gt 0 ]; then
  echo "  Kept (reinstall reuses these):"
  for k in "${KEPT[@]}"; do echo "    • $k"; done
  echo "  To remove everything: sudo $0 --purge"
fi
