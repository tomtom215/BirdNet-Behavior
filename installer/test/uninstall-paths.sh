#!/usr/bin/env bash
# installer/test/uninstall-paths.sh — does uninstall.sh find the paths the
# station actually uses, and nothing else?
#
# uninstall.sh reads the config and the unit to decide what `--remove-*`
# deletes. Three ways it got that wrong:
#
#   * read_conf kept quotes and inline comments that the station's own config
#     parser strips. `DB_PATH="/srv/bn/birds.db"` became a path starting with
#     a quote, rm_path refused it as non-absolute, and under `set -e` the
#     uninstall stopped half way through.
#   * MODEL_DIR was `<parent of DB_PATH>/models`, never MODEL_PATH, so
#     `--remove-models` could `rm -rf` a directory that was never the model's.
#   * With no DB_PATH in the config it fell back to the analytics database's
#     path, so birds.db itself was never removed.
#
# Usage: installer/test/uninstall-paths.sh

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
FAILED=0
pass() { printf '  PASS  %s\n' "$*"; }
fail() { printf '  FAIL  %s\n' "$*"; FAILED=1; }

WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT

# Run uninstall.sh's own detection against a config and a unit, and print the
# variables it settled on.
detect() { # $1=config text  $2=ExecStart line
    printf '%s\n' "$1" > "${WORK}/birdnet.conf"
    printf '[Service]\n%s\n' "$2" > "${WORK}/unit.service"
    (
        set -euo pipefail
        # shellcheck disable=SC2034  # read by the sourced functions
        CONFIG_FILE="${WORK}/birdnet.conf"
        # shellcheck disable=SC2034
        SERVICE_FILE="${WORK}/unit.service"
        # shellcheck disable=SC2034
        DATA_DIR_OVERRIDE=""
        # shellcheck disable=SC2034
        STREAM_DIR="/tmp/birdnet-stream"
        # shellcheck disable=SC1090
        source <(sed -n '/^read_conf()/,/^}/p;/^svc_flag()/,/^}/p;/^detect_paths()/,/^}/p' \
            "${REPO_ROOT}/uninstall.sh")
        detect_paths
        printf 'DB_PATH=%s\nDATA_DIR=%s\nRECS_DIR=%s\nANALYTICS_DB=%s\nIMAGE_CACHE_DIR=%s\nMODEL_DIR=%s\nMODEL_FILES=%s\n' \
            "$DB_PATH" "$DATA_DIR" "$RECS_DIR" "$ANALYTICS_DB" "$IMAGE_CACHE_DIR" "${MODEL_DIR:-}" "${MODEL_FILES[*]:-}"
    )
}

expect() { # $1=output  $2=VAR=value  $3=what
    if grep -qxF -- "$2" <<<"$1"; then pass "$3"; else fail "$3 — wanted $2, got: $(grep -- "${2%%=*}=" <<<"$1")"; fi
}

UNIT='ExecStart=/usr/local/bin/birdnet-behavior --config /etc/birdnet/birdnet.conf --analytics-db /srv/bn/analytics.db'

echo "=== quotes and inline comments are read as the station reads them ==="
out="$(detect 'DB_PATH="/srv/bn/birds.db"
RECS_DIR=/srv/bn/recs   # where clips go
MODEL_PATH='"'"'/srv/bn/models/m.onnx'"'"'' "$UNIT")"
expect "$out" "DB_PATH=/srv/bn/birds.db" "a double-quoted DB_PATH loses its quotes"
expect "$out" "DATA_DIR=/srv/bn" "the data dir is the database's parent"
expect "$out" "RECS_DIR=/srv/bn/recs" "an inline comment is not part of RECS_DIR"
expect "$out" "MODEL_DIR=/srv/bn/models" "a model inside the data dir's models/ removes that dir"

echo "=== a model outside the data dir is removed by name, never by guessing its dir ==="
out="$(detect 'DB_PATH=/srv/bn/birds.db
MODEL_PATH=/opt/shared-models/birdnet.onnx
LABELS_PATH=/opt/shared-models/labels.csv' "$UNIT")"
expect "$out" "MODEL_DIR=" "no directory is chosen for a model that lives elsewhere"
expect "$out" "MODEL_FILES=/opt/shared-models/birdnet.onnx /opt/shared-models/labels.csv" \
    "only the configured model files are removed"

echo "=== no DB_PATH: the database is birds.db, and the unit's paths are read ==="
# The unit's paths differ from anything derivable from the config, so only a
# working read of ExecStart= can produce them.
out="$(detect 'RECS_DIR=/srv/bn/recordings' \
    'ExecStart=/usr/local/bin/birdnet-behavior --analytics-db /var/lib/bn/analytics.db --image-cache-dir /var/cache/bn')"
expect "$out" "DB_PATH=/srv/bn/birds.db" "DB_PATH falls back to birds.db in the data dir"
expect "$out" "ANALYTICS_DB=/var/lib/bn/analytics.db" "the unit's --analytics-db is read"
expect "$out" "IMAGE_CACHE_DIR=/var/cache/bn" "the unit's --image-cache-dir is read"

echo "=== counterpart: a plain installer config reads unchanged ==="
out="$(detect 'DB_PATH=/home/pi/BirdNet-Behavior/birds.db
RECS_DIR=/home/pi/BirdNet-Behavior/recordings
MODEL_PATH=/home/pi/BirdNet-Behavior/models/BirdNET+_V3.0.onnx' \
    'ExecStart=/usr/local/bin/birdnet-behavior --analytics-db /home/pi/BirdNet-Behavior/analytics.db')"
expect "$out" "DB_PATH=/home/pi/BirdNet-Behavior/birds.db" "installer DB_PATH"
expect "$out" "MODEL_DIR=/home/pi/BirdNet-Behavior/models" "installer model dir"

# ── end to end: run the whole script against a sandboxed filesystem ─────────
# The detection checks above cannot see what the *execution* half does with the
# paths, so these run uninstall.sh itself. Its system paths are constants, so a
# copy has them pointed into ${WORK}; `id` and `systemctl` are stubbed so it
# runs unprivileged and touches no real unit.
E2E="${WORK}/e2e"
run_uninstall() { # $1=ExecStart line  $@(rest)=uninstall flags; prints output, returns its status
    local unit="$1"; shift
    rm -rf "${E2E}"
    mkdir -p "${E2E}/etc/birdnet" "${E2E}/systemd" "${E2E}/bin" "${E2E}/stubs" \
        "${E2E}/data/backups" "${E2E}/data/recordings" "${E2E}/stream" "${E2E}/incoming"
    printf 'DB_PATH=%s/data/birds.db\nRECS_DIR=%s/data/recordings\n' "${E2E}" "${E2E}" \
        > "${E2E}/etc/birdnet/birdnet.conf"
    printf '[Service]\n%s\n' "${unit}" > "${E2E}/systemd/birdnet-behavior.service"
    : > "${E2E}/data/birds.db"; : > "${E2E}/data/backups/birds-1.db"
    : > "${E2E}/incoming/operator-file.wav"; : > "${E2E}/bin/birdnet-behavior"
    printf '#!/bin/sh\n[ "$1" = "-u" ] && echo 0 || command id "$@"\n' > "${E2E}/stubs/id"
    printf '#!/bin/sh\nexit 1\n' > "${E2E}/stubs/systemctl"
    chmod +x "${E2E}/stubs/id" "${E2E}/stubs/systemctl"
    sed -e "s|^BIN_PATH=.*|BIN_PATH=\"${E2E}/bin/birdnet-behavior\"|" \
        -e "s|^HELP_DIR=.*|HELP_DIR=\"${E2E}/share/help\"|" \
        -e "s|^CONFIG_DIR=.*|CONFIG_DIR=\"${E2E}/etc/birdnet\"|" \
        -e "s|^SERVICE_FILE=.*|SERVICE_FILE=\"${E2E}/systemd/birdnet-behavior.service\"|" \
        -e "s|^TMPFS_UNIT_FILE=.*|TMPFS_UNIT_FILE=\"${E2E}/systemd/tmpfs.mount\"|" \
        -e "s|^ZRAM_FILE=.*|ZRAM_FILE=\"${E2E}/systemd/zram-swap.service\"|" \
        -e "s|^STREAM_DIR=.*|STREAM_DIR=\"${E2E}/stream\"|" \
        "${REPO_ROOT}/uninstall.sh" > "${E2E}/uninstall.sh"
    # Every constant must have been redirected, or this would touch the host.
    local k
    for k in BIN_PATH HELP_DIR CONFIG_DIR SERVICE_FILE TMPFS_UNIT_FILE ZRAM_FILE STREAM_DIR; do
        if ! grep -qE "^${k}=\"${E2E}/" "${E2E}/uninstall.sh"; then
            echo "sandbox rewrite missed ${k} — not running"; return 97
        fi
    done
    PATH="${E2E}/stubs:${PATH}" bash "${E2E}/uninstall.sh" "$@" 2>&1
}

echo "=== --analytics-db \"\" (analytics off, as the unit documents) does not abort --remove-db ==="
out="$(run_uninstall "ExecStart=/usr/local/bin/birdnet-behavior --watch-dir ${E2E}/stream --analytics-db \"\"" --remove-db -y)"
rc=$?
if [ "$rc" -eq 0 ]; then pass "uninstall exits 0"; else fail "uninstall exited ${rc}: $(tail -3 <<<"$out")"; fi
if grep -q "Uninstall complete" <<<"$out"; then pass "it reaches 'Uninstall complete'"; else fail "it stopped before 'Uninstall complete'"; fi
if [ ! -e "${E2E}/data/backups" ]; then pass "backups/ is removed"; else fail "backups/ was left behind"; fi
if [ ! -e "${E2E}/data/birds.db" ]; then pass "birds.db is removed"; else fail "birds.db was left behind"; fi

echo "=== counterpart: a quoted real --analytics-db path is still read and removed ==="
: # (the detection half) — quotes are part of the unit's syntax, not the path
out="$(detect 'DB_PATH=/srv/bn/birds.db' 'ExecStart=/usr/local/bin/birdnet-behavior --analytics-db "/srv/bn/a.duckdb"')"
expect "$out" "ANALYTICS_DB=/srv/bn/a.duckdb" "a quoted --analytics-db loses its quotes"

echo "=== an operator's own --watch-dir is never deleted by a plain uninstall ==="
out="$(run_uninstall "ExecStart=/usr/local/bin/birdnet-behavior --watch-dir ${E2E}/incoming --analytics-db ${E2E}/data/analytics.db" -y)"
rc=$?
if [ "$rc" -eq 0 ]; then pass "uninstall exits 0"; else fail "uninstall exited ${rc}: $(tail -3 <<<"$out")"; fi
if [ -e "${E2E}/incoming/operator-file.wav" ]; then pass "the operator's watch dir and its files are kept"; else fail "the operator's watch dir was deleted"; fi
if grep -qF "${E2E}/incoming" <<<"$out"; then pass "the kept watch dir is named in the output"; else fail "the kept watch dir is not mentioned"; fi
if [ ! -e "${E2E}/stream" ]; then pass "the installer's own stream dir is still removed"; else fail "the installer's stream dir was left behind"; fi

echo "=== counterpart: the installer's default watch dir is still removed ==="
out="$(run_uninstall "ExecStart=/usr/local/bin/birdnet-behavior --watch-dir ${E2E}/stream --analytics-db ${E2E}/data/analytics.db" -y)"
if [ ! -e "${E2E}/stream" ]; then pass "the default stream dir is removed"; else fail "the default stream dir was left behind"; fi
if [ ! -e "${E2E}/bin/birdnet-behavior" ]; then pass "the binary is removed"; else fail "the binary was left behind"; fi

if [ "$FAILED" -ne 0 ]; then
    echo "uninstall-paths: FAILED"
    exit 1
fi
echo "uninstall-paths: all pass"
