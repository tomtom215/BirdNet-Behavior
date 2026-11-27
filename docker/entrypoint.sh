#!/bin/sh
# =============================================================================
# BirdNet-Behavior container entrypoint
#
# Responsibilities:
#   1. Choose the classifier (BirdNET V2.4 by default, the BirdNET+ V3.0 preview
#      on request; an existing V3.0 volume stays on V3.0) and say which.
#   2. Install its files, and its species-occurrence filter's, by asking the
#      binary: `birdnet-behavior --install-model <id>` for every model (MDL-09).
#      The binary fetches each file from its origins in order (this project's
#      release first, then the publisher's), resumes a transfer that dropped,
#      gives up on one that stalls, retries a bounded number of times, checks the
#      sha256 and size it pins, and renames each file into place only when it
#      matches. On every start: a file already on the volume is re-hashed, not
#      trusted for being there, and a V3.0 file an earlier image left under its
#      publisher's name is adopted in place rather than downloaded again.
#      Nothing in this script knows what any model must hash to or where it
#      comes from.
#   3. Point the daemon at the classifier and at the occurrence filter that
#      pairs with it (the V2.4 range filter v2, or the V3.0 geomodel), whenever
#      the filter is on the volume and verifies.
#   4. Set container-appropriate defaults (listen on all interfaces).
#   5. Exec the birdnet-behavior binary, forwarding any extra arguments.
#
# Environment variables:
#   BIRDNET_CLASSIFIER          birdnet-v24 (BirdNET V2.4 + range filter v2) or
#                               birdnet-v3 (BirdNET+ V3.0 preview + geomodel).
#                               Unset: a new volume gets birdnet-v24; a volume
#                               that already holds the V3.0 model keeps running
#                               it, because an upgrade must never switch a
#                               station's model under its history. This script
#                               reads it and removes it before starting the
#                               daemon, which does not.
#   BIRDNET_MODEL_DIR           Model directory (default: /data/model)
#   BIRDNET_SKIP_MODEL_DOWNLOAD Set to "1" to skip the install (every model)
#   BIRDNET_LOCAL_MODEL         Install the classifier from this file (mounted
#                               into the container) instead of downloading it;
#                               checked against the same pinned sha256 and size
#                               as a download, refused if not
#   BIRDNET_MODEL_RETRY_WAITS   Seconds between rounds of a model's origins
#                               (default "30 90": three rounds); read by the binary
#   BIRDNET_MODEL_MIRROR        A mirror tried before every origin (air-gapped
#                               sites); read by the binary
#   BIRDNET_METADATA_MODEL      Occurrence filter path; set it to opt out of ours
#   BIRDNET_MODEL               Path to ONNX model file (auto-set if blank)
#   BIRDNET_LABELS              Path to labels CSV file (auto-set if blank)
#   BIRDNET_LISTEN              Web server address (default: 0.0.0.0:8502)
#   TZ                          IANA zone the station files detections under
#                               (unset: UTC, and a warning on the way in)
#
# All other BIRDNET_* variables are passed through to the binary unchanged.
# =============================================================================
set -eu

# ---------------------------------------------------------------------------
# Blank settings are unset settings
# ---------------------------------------------------------------------------
# Must run before anything below reads a BIRDNET_* variable. `docker compose`
# materialises every optional `${VAR:-}` as an empty string, and clap reads an
# empty environment variable as a *supplied* value — so `BIRDNET_LATITUDE=`
# exits 2 during argument parsing rather than meaning "no latitude". See
# docker/strip-blank-env.sh for the full reasoning and the two exceptions.
BNB_STRIP_LIB="${BNB_STRIP_LIB:-/usr/local/bin/strip-blank-env.sh}"
# The daemon, which is also what installs BirdNET V2.4 (`--install-model`). A
# variable so a test can put a stub there; nothing sets it in an image.
BNB_BINARY="${BNB_BINARY:-/usr/local/bin/birdnet-behavior}"
if [ ! -r "$BNB_STRIP_LIB" ]; then
    printf '[birdnet] ERROR: %s is missing from the image\n' "$BNB_STRIP_LIB" >&2
    exit 1
fi
# The path is a variable so a test can point at the repo copy, which SC1090
# flags because it cannot be resolved statically. The directive below names the
# real location; with `-x` (set in CI) the linter follows it and checks the
# sourced file's interaction with this one, which a bare `disable=SC1090` would
# have thrown away.
#
# Note for the next editor: a comment line beginning with the linter's own name
# is parsed as a directive, so this paragraph deliberately avoids starting one
# that way. Doing it accidentally is how this comment got written twice.
# shellcheck source=docker/strip-blank-env.sh
. "$BNB_STRIP_LIB"
strip_blank_birdnet_env

# ---------------------------------------------------------------------------
# Timezone (ON-7)
# ---------------------------------------------------------------------------
# Detections are filed under the container's local hours, which come from TZ:
# glibc reads it, and SQLite's localtime — the one source of the station's
# offset — with it. Unset, the container runs on UTC and every hour on the
# dashboard is a UTC hour. Set to a name the image does not know, glibc falls
# back to UTC just as silently, which is the worse failure because it looks
# configured. Either way, say so on the way in; neither stops the station.
case "${TZ:-}" in
    "")
        printf '[birdnet] WARNING: TZ is not set: the station runs on UTC and files detections under UTC hours. Set TZ=<Area/City> in .env (for example TZ=Europe/Berlin)\n' >&2
        ;;
    *)
        if [ ! -f "/usr/share/zoneinfo/$TZ" ]; then
            printf '[birdnet] WARNING: TZ=%s is not a zone this image knows, so the station runs on UTC. Use an IANA name such as Europe/Berlin\n' "$TZ" >&2
        fi
        ;;
esac

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
log()   { printf '[birdnet] %s\n' "$*"; }
warn()  { printf '[birdnet] WARNING: %s\n' "$*" >&2; }
die()   { printf '[birdnet] ERROR: %s\n' "$*" >&2; exit 1; }
rule()  { log '----------------------------------------------------------------'; }

# ---------------------------------------------------------------------------
# Model names and the classifier choice
# ---------------------------------------------------------------------------
MODEL_DIR="${BIRDNET_MODEL_DIR:-/data/model}"

# The catalogue ids this script asks the binary to install, and the NAMES the
# binary gives the files, so the daemon can be pointed at them. Nothing else
# about them is here — no digest, no URL, no size: the binary installs them and
# owns what they must hash to (model_catalog.rs).
# installer/test/model-pins-match-catalogue.sh holds these names to the
# catalogue and fails if a digest or a URL of any model appears in this file.
V24_MODEL_ID="birdnet-v24"
V24_RANGE_ID="birdnet-v24-range-v2"
V24_MODEL_FILE="birdnet-v24.onnx"
V24_LABELS_FILE="birdnet-v24_labels.txt"
V24_RANGE_FILE="birdnet-v24-range-v2.onnx"
V3_MODEL_ID="birdnet-v3"
V3_GEOMODEL_ID="birdnet-v3-geomodel"
V3_MODEL_FILE="birdnet-v3.onnx"
V3_LABELS_FILE="birdnet-v3_labels.csv"
GEOMODEL_FILE="birdnet-v3-geomodel.onnx"
GEOMODEL_LABELS_FILE="birdnet-v3-geomodel_labels.txt"
# The names this script gave the V3.0 files before the binary installed them
# (their publishers' names). The binary adopts a verified file under one of
# them in place; the old name is then removed below.
V3_LEGACY_MODEL_FILE="BirdNET+_V3.0-preview3_Global_11K_FP32.onnx"
V3_LEGACY_LABELS_FILE="BirdNET+_V3.0-preview3_Global_11K_Labels.csv"
GEOMODEL_LEGACY_FILE="BirdNET+_Geomodel_V3.0.2_Global_12K_FP32.onnx"
GEOMODEL_LEGACY_LABELS_FILE="BirdNET+_Geomodel_V3.0.2_Global_12K_Labels.txt"

# A variable pointed at an old V3.0 name this script used to manage now names
# the file under its new one: the binary adopts the file, and the old name is
# removed once it has. $1 is the variable's name.
follow_legacy_name() {
    fl_value=""
    eval "fl_value=\${$1:-}"
    for fl_pair in "${V3_LEGACY_MODEL_FILE}:${V3_MODEL_FILE}" \
                   "${V3_LEGACY_LABELS_FILE}:${V3_LABELS_FILE}" \
                   "${GEOMODEL_LEGACY_FILE}:${GEOMODEL_FILE}" \
                   "${GEOMODEL_LEGACY_LABELS_FILE}:${GEOMODEL_LABELS_FILE}"; do
        if [ "${fl_value}" = "${MODEL_DIR}/${fl_pair%%:*}" ]; then
            eval "$1=\"\${MODEL_DIR}/${fl_pair#*:}\""
            log "$1 named ${fl_pair%%:*}, the name an earlier image gave that file; it is ${fl_pair#*:} now."
        fi
    done
}
for fl_var in BIRDNET_MODEL BIRDNET_LABELS BIRDNET_METADATA_MODEL BIRDNET_METADATA_LABELS; do
    follow_legacy_name "${fl_var}"
done

# Mirrors of the old V3.0 download code; the binary takes BIRDNET_MODEL_MIRROR.
for old_base in BIRDNET_MODEL_BASE BIRDNET_ZENODO_BASE BIRDNET_GEOMODEL_BASE; do
    eval "old_set=\${${old_base}:+set}"
    if [ -n "${old_set}" ]; then
        warn "${old_base} is no longer read: every model is installed by the binary, which tries BIRDNET_MODEL_MIRROR first. Point that at a directory holding the files --install-model writes."
        unset "${old_base}"
    fi
done

# ---------------------------------------------------------------------------
# Which classifier does this container run?
# ---------------------------------------------------------------------------
# In order:
#   1. BIRDNET_CLASSIFIER, when set. An unknown value stops the container with
#      the two valid ones; a typo must not quietly run the other network.
#   2. BIRDNET_MODEL, when set: pointing it at the V2.4 file this script
#      manages is V2.4; pointing it at anything else is a model of the
#      operator's own, which keeps the V3.0 defaults it always had for labels
#      and the geomodel (they are matched by name, so they suit any classifier)
#      rather than losing them on upgrade.
#   3. A volume that already holds the V3.0 model file (under either name)
#      keeps running V3.0. An upgrade must never switch the model under a
#      running station: its history would be rows two different networks
#      scored. One log line says so and says how to change it.
#   4. Otherwise a new volume gets BirdNET V2.4, the network BirdNET-Pi and
#      birdnet-go run.
#
# Sets CLASSIFIER and what the rest of the script reads for it (MODEL_FILE,
# LABELS_FILE, FILTER_ID), and logs one "Classifier:" line. Needs MODEL_DIR;
# must run before BIRDNET_MODEL is defaulted below, which would hide whether the
# operator set it.
select_classifier() {
    requested="${BIRDNET_CLASSIFIER:-}"
    case "$requested" in
        "" | birdnet-v24 | birdnet-v3) ;;
        *) die "BIRDNET_CLASSIFIER=${requested} is not a classifier this image knows. Use birdnet-v24 (the default) or birdnet-v3." ;;
    esac

    if [ -n "$requested" ]; then
        CLASSIFIER="$requested"
        how="explicit"
    elif [ -n "${BIRDNET_MODEL:-}" ]; then
        if [ "${BIRDNET_MODEL}" = "${MODEL_DIR}/${V24_MODEL_FILE}" ]; then
            CLASSIFIER="birdnet-v24"
            how="model-v24"
        elif [ "${BIRDNET_MODEL}" = "${MODEL_DIR}/${V3_MODEL_FILE}" ]; then
            CLASSIFIER="birdnet-v3"
            how="model-v3"
        else
            CLASSIFIER="birdnet-v3"
            how="model-own"
        fi
    elif [ -f "${MODEL_DIR}/${V3_MODEL_FILE}" ] || [ -f "${MODEL_DIR}/${V3_LEGACY_MODEL_FILE}" ]; then
        CLASSIFIER="birdnet-v3"
        how="volume-v3"
    else
        CLASSIFIER="birdnet-v24"
        how="default"
    fi

    case "$CLASSIFIER" in
        "${V24_MODEL_ID}")
            CLASSIFIER_NAME="BirdNET V2.4"
            MODEL_FILE="${V24_MODEL_FILE}"
            LABELS_FILE="${V24_LABELS_FILE}"
            FILTER_ID="${V24_RANGE_ID}"
            ;;
        "${V3_MODEL_ID}")
            CLASSIFIER_NAME="BirdNET+ V3.0"
            MODEL_FILE="${V3_MODEL_FILE}"
            LABELS_FILE="${V3_LABELS_FILE}"
            FILTER_ID="${V3_GEOMODEL_ID}"
            ;;
    esac

    case "$how" in
        explicit)
            log "Classifier: ${CLASSIFIER_NAME} (BIRDNET_CLASSIFIER=${CLASSIFIER}). Set it to birdnet-v24 or birdnet-v3 to change it." ;;
        model-v24)
            log "Classifier: BirdNET V2.4 (BIRDNET_MODEL is the V2.4 file). Set BIRDNET_CLASSIFIER to be explicit." ;;
        model-v3)
            log "Classifier: BirdNET+ V3.0 preview (BIRDNET_MODEL is the V3.0 file). Set BIRDNET_CLASSIFIER to be explicit." ;;
        model-own)
            log "Classifier: your own model (BIRDNET_MODEL=${BIRDNET_MODEL}); labels and the occurrence filter default to the BirdNET+ V3.0 ones, as before. Set BIRDNET_CLASSIFIER=birdnet-v24 to use V2.4's instead." ;;
        volume-v3)
            log "Classifier: BirdNET+ V3.0 preview — this volume already holds its model, so it keeps running it (an upgrade never switches a station's model). New volumes default to BirdNET V2.4; set BIRDNET_CLASSIFIER=birdnet-v24 to move this one, or birdnet-v3 to keep V3.0 and silence this." ;;
        *)
            log "Classifier: BirdNET V2.4 (the default; the network BirdNET-Pi and birdnet-go run). Set BIRDNET_CLASSIFIER=birdnet-v3 on a new volume for the BirdNET+ V3.0 preview." ;;
    esac
}
select_classifier
# Read once, here. The daemon does not read BIRDNET_CLASSIFIER, and the binary
# reports every BIRDNET_* variable it does not read — including in the
# `--install-model` runs below, which would open the first start's log with
# "BIRDNET_CLASSIFIER is not a variable this station reads".
unset BIRDNET_CLASSIFIER

# ---------------------------------------------------------------------------
# When the classifier cannot be installed (OOB-20; decision 7 of 2026-11-25)
# ---------------------------------------------------------------------------
# The same rule as the installer: the binary retries a bounded number of times
# (BIRDNET_MODEL_RETRY_WAITS, which it reads), then the container STARTS ANYWAY
# and says loudly that nothing is being identified. A container that exits at
# its first start is restarted by Compose into the same failure, with no
# dashboard to say why. Started, the dashboard is up and every later start
# tries again. Integrity is unchanged: a local file passes the same check a
# download does, and is refused otherwise. BIRDNET_LOCAL_MODEL is read once and
# removed: the daemon does not read it.
LOCAL_MODEL="${BIRDNET_LOCAL_MODEL:-}"
unset BIRDNET_LOCAL_MODEL
CLASSIFIER_MISSING=0

# Say, loudly, that the classifier is missing; the container carries on.
classifier_not_installed() {
    CLASSIFIER_MISSING=1
    warn "================================================================"
    warn "THIS STATION IS NOT DETECTING BIRDS: $1."
    warn "The container starts anyway, so the dashboard is up, but nothing is"
    warn "identified until the classifier is installed. To install it: restart the"
    warn "container once it can reach the internet (every start tries again), or"
    warn "mount a copy of the model file and set BIRDNET_LOCAL_MODEL to its path"
    warn "(it must match the same pinned sha256 as a download, or it is refused)."
    warn "================================================================"
}

# Respect explicit overrides; otherwise use the default paths under MODEL_DIR.
MODEL_DEFAULT="${MODEL_DIR}/${MODEL_FILE}"
LABELS_DEFAULT="${MODEL_DIR}/${LABELS_FILE}"
: "${BIRDNET_MODEL:=${MODEL_DEFAULT}}"
: "${BIRDNET_LABELS:=${LABELS_DEFAULT}}"
export BIRDNET_MODEL BIRDNET_LABELS

# Deliberately NOT defaulted here. An operator who set this keeps it; one who
# did not gets it only once the filter is on the volume and verifies, because a
# path pointing at a file that never arrived makes the daemon refuse the model
# on every start and --doctor report FAIL on a container that merely declined
# an optional download.
GEOMODEL_USER_SET=0
[ -n "${BIRDNET_METADATA_MODEL:-}" ] && GEOMODEL_USER_SET=1

# Run `birdnet-behavior --install-model ID` into ${MODEL_DIR}. Its output goes
# to the container log as it comes: what it fetches, from where, how far it has
# got (a line every ten seconds), and why when it refuses. Any further
# arguments are the binary's (`--from-file PATH`).
install_with_binary() {
    BIRDNET_MODEL_DIR="${MODEL_DIR}" "${BNB_BINARY}" --config /dev/null --install-model "$@"
}

# Remove the old V3.0 names once the binary has the files under the new ones
# and nothing this container passes the daemon names the old ones.
drop_legacy_names() {
    for dl_pair in "${V3_LEGACY_MODEL_FILE}:${V3_MODEL_FILE}" \
                   "${V3_LEGACY_LABELS_FILE}:${V3_LABELS_FILE}" \
                   "${GEOMODEL_LEGACY_FILE}:${GEOMODEL_FILE}" \
                   "${GEOMODEL_LEGACY_LABELS_FILE}:${GEOMODEL_LABELS_FILE}"; do
        dl_old="${MODEL_DIR}/${dl_pair%%:*}"
        [ -e "${dl_old}" ] && [ -e "${MODEL_DIR}/${dl_pair#*:}" ] || continue
        case " ${BIRDNET_MODEL:-} ${BIRDNET_LABELS:-} ${BIRDNET_METADATA_MODEL:-} ${BIRDNET_METADATA_LABELS:-} " in
            *" ${dl_old} "*) continue ;;
        esac
        rm -f "${dl_old}" "${dl_old}.verified"
    done
}

# ---------------------------------------------------------------------------
# The classifier
# ---------------------------------------------------------------------------
# Only the files at the default paths are this script's. A model the operator
# pointed BIRDNET_MODEL at is theirs and is neither checked nor replaced; with
# it, the default labels are used as found (as supplied), since the binary
# installs a model and its labels together.
acquire_classifier_models() {
    if [ "${BIRDNET_SKIP_MODEL_DOWNLOAD:-}" = "1" ]; then
        log "BIRDNET_SKIP_MODEL_DOWNLOAD=1 — skipping the model install."
        # MDL-27: the station itself fetches a configured catalogue file that is
        # missing (at start, then hourly), so say so rather than imply it never will.
        log "The station fetches missing model files itself once it starts; set BIRDNET_OFFLINE=1 to stop it fetching anything on its own (BIRDNET_MODEL_MIRROR is still used)."
        [ -f "${BIRDNET_MODEL}"  ] || warn "Model not found at ${BIRDNET_MODEL}"
        [ -f "${BIRDNET_LABELS}" ] || warn "Labels not found at ${BIRDNET_LABELS}"
        return 0
    fi
    mkdir -p "${MODEL_DIR}"

    if [ "${BIRDNET_MODEL}" != "${MODEL_DEFAULT}" ]; then
        log "BIRDNET_MODEL=${BIRDNET_MODEL} is not the ${CLASSIFIER_NAME} model this image manages — using it as supplied (not verified, never replaced)."
        [ -f "${BIRDNET_MODEL}" ] || warn "Model not found at ${BIRDNET_MODEL}"
        if [ "${BIRDNET_LABELS}" = "${LABELS_DEFAULT}" ] && [ ! -f "${BIRDNET_LABELS}" ] \
            && [ -f "${MODEL_DIR}/${V3_LEGACY_LABELS_FILE}" ]; then
            BIRDNET_LABELS="${MODEL_DIR}/${V3_LEGACY_LABELS_FILE}"
            export BIRDNET_LABELS
        fi
        [ -f "${BIRDNET_LABELS}" ] || warn "Labels not found at ${BIRDNET_LABELS}; set BIRDNET_LABELS to your model's labels."
        return 0
    fi
    if [ "${BIRDNET_LABELS}" != "${LABELS_DEFAULT}" ]; then
        log "BIRDNET_LABELS=${BIRDNET_LABELS} is not the ${CLASSIFIER_NAME} labels this image manages — using it as supplied (not verified, never replaced)."
    fi

    rule
    log "Installing ${CLASSIFIER_NAME} with birdnet-behavior --install-model ${CLASSIFIER}."
    log "  Every file is sha256-verified before it is kept; a file already on the volume"
    log "  is re-hashed. The first start downloads it; later starts reuse the volume."
    rule
    if [ -n "${LOCAL_MODEL}" ]; then
        # The binary checks the file against the sha256 AND size it pins, as it
        # checks a download. A file that does not match will not match again.
        log "  From ${LOCAL_MODEL} (BIRDNET_LOCAL_MODEL), checked like a download."
        if ! install_with_binary "${CLASSIFIER}" --from-file "${LOCAL_MODEL}"; then
            classifier_not_installed "${LOCAL_MODEL} (BIRDNET_LOCAL_MODEL) was refused: it is not the ${CLASSIFIER_NAME} model this image pins"
            return 0
        fi
    elif ! install_with_binary "${CLASSIFIER}"; then
        warn "Common causes:"
        warn "  • no internet in the container (check the host's DNS/firewall)"
        warn "  • every origin of the model unreachable (retry shortly)"
        warn "  • less free space on the volume than the model plus 512 MiB (df -h on the host's docker root)"
        classifier_not_installed "${CLASSIFIER_NAME} could not be downloaded and verified"
        return 0
    fi
    log "Model ready."
}
acquire_classifier_models

# ---------------------------------------------------------------------------
# The species-occurrence filter (non-fatal)
# ---------------------------------------------------------------------------
# Unlike the classifier, a missing filter is not fatal: the container starts and
# detects, it simply does not filter by location. Refusing to start over an
# optional 14–30 MB download would be the worse failure.
#
# The filter that belongs to the classifier, and only that one: the V2.4 range
# filter for V2.4 (one file, indexed like the V2.4 labels), the geomodel and its
# own labels for everything else. On every start it is installed (a re-hash when
# it is there), and the daemon is pointed at it whenever it verifies — so a
# filter installed from the dashboard is switched on by the next container
# start (MDL-14). A function so installer/test/classifier-docker.sh can drive it.
setup_occurrence_filter() {
    if [ "${GEOMODEL_USER_SET}" = "1" ]; then
        log "BIRDNET_METADATA_MODEL is set explicitly — leaving the occurrence filter alone."
        return 0
    fi
    if [ "${BIRDNET_SKIP_MODEL_DOWNLOAD:-}" = "1" ]; then
        log "BIRDNET_SKIP_MODEL_DOWNLOAD=1 — skipping the occurrence filter install too."
        return 0
    fi
    mkdir -p "${MODEL_DIR}"
    log "Installing the species-occurrence filter with birdnet-behavior --install-model ${FILTER_ID}."
    if ! install_with_binary "${FILTER_ID}"; then
        warn "Occurrence filter unavailable — species occurrence filtering is OFF."
        warn "Every species the classifier knows stays a candidate wherever this"
        warn "station is. Restart the container to retry, then check with:"
        warn "  docker exec <container> birdnet-behavior --doctor"
        return 0
    fi
    if [ "${FILTER_ID}" = "${V24_RANGE_ID}" ]; then
        BIRDNET_METADATA_MODEL="${MODEL_DIR}/${V24_RANGE_FILE}"
        export BIRDNET_METADATA_MODEL
        if [ -n "${BIRDNET_METADATA_LABELS:-}" ]; then
            warn "BIRDNET_METADATA_LABELS is set, but the V2.4 range filter is indexed like the V2.4 labels and needs none. Unset it unless you mean to override the label file."
        fi
        log "Species occurrence filtering: ON (BirdNET V2.4 range filter v2; set BIRDNET_SF_THRESH to tune; default 0.03)."
    else
        BIRDNET_METADATA_MODEL="${MODEL_DIR}/${GEOMODEL_FILE}"
        # Only a default: an operator who set the labels alone (a translated or
        # edited label file for the geomodel we install) keeps theirs.
        BIRDNET_METADATA_LABELS="${BIRDNET_METADATA_LABELS:-${MODEL_DIR}/${GEOMODEL_LABELS_FILE}}"
        export BIRDNET_METADATA_MODEL BIRDNET_METADATA_LABELS
        log "Species occurrence filtering: ON (set BIRDNET_SF_THRESH to tune; default 0.03)."
    fi
}
setup_occurrence_filter
drop_legacy_names

# ---------------------------------------------------------------------------
# Container defaults
# ---------------------------------------------------------------------------
# The binary already defaults to 0.0.0.0:8502; this pins that default here so
# the container binds on all interfaces whatever the binary's default becomes,
# unless the user has set BIRDNET_LISTEN explicitly.
: "${BIRDNET_LISTEN:=0.0.0.0:8502}"
export BIRDNET_LISTEN

# ---------------------------------------------------------------------------
# Audio source check (advisory only — does not prevent startup)
# ---------------------------------------------------------------------------
if [ -z "${BIRDNET_ALSA_DEVICE:-}" ] \
    && [ -z "${BIRDNET_PIPEWIRE_DEVICE:-}" ] \
    && [ -z "${BIRDNET_RTSP_URL:-}" ] \
    && [ -z "${BIRDNET_RTSP_URLS:-}" ]; then
    warn "No audio source configured."
    warn "Set one of: BIRDNET_ALSA_DEVICE, BIRDNET_PIPEWIRE_DEVICE,"
    warn "            BIRDNET_RTSP_URL, or BIRDNET_RTSP_URLS."
    warn "The web UI will start but no detections will be produced."
    warn "File-watch mode: drop WAV files into \$BIRDNET_WATCH_DIR instead."
fi

# ---------------------------------------------------------------------------
# Launch
# ---------------------------------------------------------------------------
rule
log "Starting birdnet-behavior  (listen: ${BIRDNET_LISTEN})"
if [ "${CLASSIFIER_MISSING}" = 1 ]; then
    warn "Starting WITHOUT a classifier: the dashboard is up, but no bird will be identified (see above)."
fi
rule
# (BIRDNET_CLASSIFIER was removed from the environment right after it was read,
# so neither the installs above nor this daemon report it as an unknown variable.)
exec "${BNB_BINARY}" "$@"
