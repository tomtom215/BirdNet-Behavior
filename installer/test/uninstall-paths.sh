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

if [ "$FAILED" -ne 0 ]; then
    echo "uninstall-paths: FAILED"
    exit 1
fi
echo "uninstall-paths: all pass"
