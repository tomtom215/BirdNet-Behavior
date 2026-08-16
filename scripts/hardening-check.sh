#!/usr/bin/env bash
# Run the binary under the constraints its own systemd unit imposes.
#
# The ICU outage that emptied every analytics dashboard on 0.13.1 was not a
# subtle bug. DuckDB autoinstalled its extensions into `$HOME/.duckdb`, the
# shipped unit sets `ProtectHome=read-only`, the write failed, and every
# date-ranged query failed permanently from then on. Every test passed
# throughout, because nothing ever ran the product under its own hardening —
# tests run as a normal user with a writable home and a working network.
#
# This closes that gap. It reproduces the unit's sandbox with a mount
# namespace and drives the binary through it, so the class of bug ("assumes a
# writable $HOME", "assumes a writable /usr", "assumes the real /tmp") is
# caught here rather than on a station.
#
# Directives reproduced, from installer/lib/65-service.sh:
#
#   ProtectHome=read-only   ->  $HOME bind-mounted read-only
#   ProtectSystem=strict    ->  /usr bind-mounted read-only
#   PrivateTmp=yes          ->  a fresh, empty /tmp
#   ReadWritePaths=         ->  DATA_DIR and CONFIG_DIR stay writable
#
# Not reproduced: the kernel-facing directives (ProtectKernelLogs,
# ProtectControlGroups, ProtectClock, ...) and NoNewPrivileges. Those need real
# systemd. `systemd-run --user` applies them if you have a session bus:
#
#   systemd-run --user --pty --property=ProtectHome=read-only \
#       --property=PrivateTmp=yes birdnet-behavior --verify-extension
#
# Usage:
#   scripts/hardening-check.sh [--bin PATH] [--db PATH]
#
#   --bin PATH   binary to exercise (default: target/debug/birdnet-behavior)
#   --db  PATH   an existing SQLite database to migrate under hardening. Use a
#                COPY of a real station database: migrations run for real, and
#                this is the only way to see whether a data-rewriting migration
#                can take its backup inside ReadWritePaths.
#
# Exits non-zero if any check fails. Exits 99 if the sandbox itself could not
# be applied — a sandbox that silently failed to apply would let everything
# below pass for the wrong reason.

set -euo pipefail

# ── Re-exec inside the namespace ────────────────────────────────────────────
# `unshare -rm` gives a user + mount namespace in which the bind mounts below
# are permitted without real root. The script re-runs itself inside it rather
# than shipping a second file.
if [ "${BNB_HARDENED:-0}" != "1" ]; then
  BIN="target/debug/birdnet-behavior"
  DB=""
  while [ $# -gt 0 ]; do
    case "$1" in
      --bin) BIN="${2:-}"; shift 2 ;;
      --db)  DB="${2:-}";  shift 2 ;;
      -h|--help) sed -n '2,42p' "$0"; exit 0 ;;
      *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
  done

  [ -x "$BIN" ] || {
    echo "no binary at $BIN — build it first (cargo build) or pass --bin" >&2
    exit 2
  }
  BIN="$(cd "$(dirname "$BIN")" && pwd)/$(basename "$BIN")"
  if [ -n "$DB" ]; then
    [ -r "$DB" ] || { echo "cannot read $DB" >&2; exit 2; }
    DB="$(cd "$(dirname "$DB")" && pwd)/$(basename "$DB")"
  fi

  unshare -rm --help >/dev/null 2>&1 || {
    echo "unshare is unavailable; cannot build the sandbox" >&2
    exit 2
  }

  # Outside /tmp on purpose: PrivateTmp replaces /tmp, so a working directory
  # underneath it would vanish the moment the sandbox is applied.
  WORK="$(mktemp -d /var/tmp/bnb-hardening-XXXXXX)"
  trap 'rm -rf "$WORK"' EXIT
  mkdir -p "$WORK/data" "$WORK/config" "$WORK/tmp"
  [ -n "$DB" ] && cp "$DB" "$WORK/data/birds.db"

  export BNB_HARDENED=1 BNB_BIN="$BIN" BNB_WORK="$WORK" BNB_DB="$DB"
  exec unshare -rm "$0"
fi

# ── Inside the namespace ────────────────────────────────────────────────────
WORK="$BNB_WORK"
BIN="$BNB_BIN"
DATA_DIR="$WORK/data"
CONFIG_DIR="$WORK/config"

# Refuse to touch the host's mounts.
#
# Everything below bind-mounts over $HOME, /usr and /tmp. Run outside a private
# mount namespace — because `BNB_HARDENED=1` was set by hand, or because the
# re-exec above did not happen — it does that to the real system, as root, and
# the `rm -rf "$WORK"` on exit then deletes what /tmp is mounted from.
#
# This is not a hypothetical: it is exactly how writing this script took out a
# development container. The body was run directly to test the guard below, the
# bind mount landed on the host's /tmp, and every tool that needed a temp
# directory stopped working.
if [ "$(readlink /proc/1/ns/mnt 2>/dev/null)" = "$(readlink /proc/self/ns/mnt 2>/dev/null)" ] \
   && [ -r /proc/1/ns/mnt ]; then
  echo "refusing to run: this is the host mount namespace, not a private one." >&2
  echo "Run the script normally — it re-execs itself under 'unshare -rm'." >&2
  exit 99
fi

# Belt and braces. Even inside a namespace, a mount can propagate back to the
# parent when the parent's mounts are shared, which is the default on many
# systems. Make this tree private before touching anything.
mount --make-rprivate / 2>/dev/null || true

mount --bind "$HOME" "$HOME"
mount -o remount,ro,bind "$HOME"
mount --bind /usr /usr
mount -o remount,ro,bind /usr
mount --bind "$WORK/tmp" /tmp

# Prove the sandbox before trusting anything that runs in it.
if (echo probe > "$HOME/.bnb-hardening-probe") 2>/dev/null; then
  rm -f "$HOME/.bnb-hardening-probe"
  echo "SANDBOX NOT APPLIED: \$HOME is still writable. Every check below would" >&2
  echo "pass without testing the thing this script exists for." >&2
  exit 99
fi
if ! (echo probe > "$DATA_DIR/.probe") 2>/dev/null; then
  echo "SANDBOX BROKEN: DATA_DIR is not writable, so this is not the unit's" >&2
  echo "sandbox — ReadWritePaths= keeps DATA_DIR writable." >&2
  exit 99
fi
rm -f "$DATA_DIR/.probe"
echo "sandbox applied: \$HOME ro, /usr ro, private /tmp, DATA_DIR rw"
echo

fails=0
note() { printf '  %-7s %s\n' "$1" "$2"; }

# ── 1. Extensions load, and land somewhere writable ─────────────────────────
echo "1. DuckDB extensions under ProtectHome=read-only"
if "$BIN" --analytics-db "$DATA_DIR/analytics.duckdb" --verify-extension >"$WORK/verify.log" 2>&1; then
  note "ok" "--verify-extension exits 0"
else
  note "FAIL" "--verify-extension exits non-zero (see below)"
  sed 's/^/         /' "$WORK/verify.log" | tail -12
  # A build with nothing embedded and no network legitimately cannot load ICU.
  # That is a build-configuration fact, not a hardening failure, so it is
  # reported but does not by itself fail the run.
  if grep -q "no embedded extension was bundled" "$WORK/verify.log"; then
    note "note" "this build embeds no extensions; rerun a release build to gate offline"
  else
    fails=1
  fi
fi

# The whole point: nothing may be written to $HOME, ever.
if [ -e "$HOME/.duckdb" ]; then
  note "FAIL" "\$HOME/.duckdb was created — the 0.13.1 outage, exactly"
  fails=1
else
  note "ok" "no \$HOME/.duckdb"
fi

# ── 2. Migrations, including any pre-migration backup ───────────────────────
if [ -n "$BNB_DB" ]; then
  echo
  echo "2. Migrations against the supplied database"
  printf 'DB_PATH=%s\nRECS_DIR=%s\n' "$DATA_DIR/birds.db" "$DATA_DIR/recordings" \
    > "$CONFIG_DIR/birdnet.conf"

  before="$("$BIN" --config "$CONFIG_DIR/birdnet.conf" --migration-report 2>/dev/null || true)"
  if printf '%s' "$before" | grep -q "Schema"; then
    note "ok" "--migration-report runs read-only under hardening"
  else
    note "FAIL" "--migration-report produced nothing"
    fails=1
  fi

  # Boot far enough for migrations to run, then stop. A short timeout is
  # enough: migrations happen while the state is built, before serving.
  timeout 30 "$BIN" --config "$CONFIG_DIR/birdnet.conf" >"$WORK/boot.log" 2>&1 || true

  if grep -qiE "read-only file system|permission denied" "$WORK/boot.log"; then
    note "FAIL" "the boot hit a read-only path"
    grep -iE "read-only file system|permission denied" "$WORK/boot.log" | sed 's/^/         /' | head -5
    fails=1
  else
    note "ok" "no read-only or permission failures during boot"
  fi

  backups="$(find "$DATA_DIR" -maxdepth 1 -name '*.pre-migration-*.backup' | wc -l)"
  if [ "$backups" -gt 0 ]; then
    note "ok" "$backups pre-migration backup(s) written inside DATA_DIR"
  else
    note "note" "no pre-migration backup (none was pending for this database)"
  fi

  if [ -e "$HOME/.duckdb" ]; then
    note "FAIL" "\$HOME/.duckdb appeared during boot"
    fails=1
  else
    note "ok" "no \$HOME/.duckdb after boot"
  fi
fi

echo
if [ "$fails" -eq 0 ]; then
  echo "PASS — nothing needed a writable \$HOME."
else
  echo "FAIL — see above."
fi
exit "$fails"
