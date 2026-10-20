# ---------------------------------------------------------------------------
# Choosing the classifier
#
# Which network a station runs decides every detection it stores, so the rule
# for choosing is written down here and held by installer/test/classifier-*.sh:
#
#   1. A station that already has a config keeps the classifier that config
#      names. The config is pinned per station (write_config never rewrites an
#      existing one), and an upgrade must never swap the model under a running
#      station: its history would split into rows two different networks
#      scored. Only a config whose MODEL_PATH is the V2.4 file this installer
#      manages selects V2.4; every other existing config selects the V3.0 files,
#      because that is what every install before V2.4 became the default
#      fetched, and what a repair of one of those must keep fetching.
#   2. Otherwise an explicit request (--classifier, else BIRDNET_CLASSIFIER).
#   3. Otherwise, if the V3.0 model file is already in the models directory
#      (a station restored from a backup of its data without its config), V3.0:
#      the same "never switch under a history" rule, applied to the evidence
#      that is left.
#   4. Otherwise the default, BirdNET V2.4.
# ---------------------------------------------------------------------------

# Where an operator reads how the two classifiers differ and how to move a
# station from one to the other.
CLASSIFIER_GUIDE_URL="https://github.com/${REPO}/blob/main/docs/book/guides/parity.md"

# True when $1 is a classifier id this installer knows.
valid_classifier_id() {
    case "$1" in
        birdnet-v24 | birdnet-v3) return 0 ;;
        *) return 1 ;;
    esac
}

# Make $1 the classifier the rest of the installer works with, by copying its
# file names (and, for V3.0 only, its digests and release tag) into the
# unprefixed names the download, config and summary code read.
# Silent: it only sets variables.
select_classifier() {
    case "$1" in
        birdnet-v24)
            CLASSIFIER="birdnet-v24"
            CLASSIFIER_NAME="BirdNET V2.4"
            MODEL_FILE="${V24_MODEL_FILE}"
            LABELS_FILE="${V24_LABELS_FILE}"
            # No digests and no release tag: the binary installs these files
            # and owns what they must hash to (see 10-config.sh).
            MODEL_SHA256=""
            LABELS_SHA256=""
            MODEL_RELEASE_TAG=""
            MODEL_SIZE_HINT="~54 MB"
            # The binary will not install into less than the model's size plus
            # a 512 MiB margin, so the floor is above that, not above 85 MB.
            REQUIRED_FREE_MB=600
            ;;
        birdnet-v3)
            CLASSIFIER="birdnet-v3"
            CLASSIFIER_NAME="BirdNET+ V3.0"
            MODEL_FILE="${V3_MODEL_FILE}"
            LABELS_FILE="${V3_LABELS_FILE}"
            MODEL_SHA256="${V3_MODEL_SHA256}"
            LABELS_SHA256="${V3_LABELS_SHA256}"
            MODEL_RELEASE_TAG="${V3_MODEL_RELEASE_TAG}"
            MODEL_SIZE_HINT="~541 MB"
            REQUIRED_FREE_MB=900
            ;;
        *)
            fatal "Unknown classifier '$1'. Choose ${CLASSIFIER_V24} (the default) or ${CLASSIFIER_V3}."
            ;;
    esac
    if [ -n "${MODEL_RELEASE_TAG}" ]; then
        MODEL_GH_BASE="https://github.com/${REPO}/releases/download/${MODEL_RELEASE_TAG}"
    else
        MODEL_GH_BASE=""
    fi
}

# The value of KEY in FILE (default: the station config): the last active
# `KEY=value` line, trimmed and unquoted. Prints nothing when there is none.
config_value() {
    local key="$1" file="${2:-${CONFIG_FILE}}" line
    [ -f "${file}" ] || return 0
    line="$(grep -E "^[[:space:]]*${key}[[:space:]]*=" "${file}" 2>/dev/null | tail -n 1 || true)"
    line="${line#*=}"
    line="${line#"${line%%[![:space:]]*}"}"
    line="${line%"${line##*[![:space:]]}"}"
    case "${line}" in
        \"*\") line="${line#\"}"; line="${line%\"}" ;;
    esac
    printf '%s' "${line}"
}

# Settle CLASSIFIER for this run (see the rules at the top of this file) and
# say what was decided and why. Needs MODEL_DIR and CONFIG_FILE final, so it
# runs after require_root has derived them.
resolve_classifier() {
    local requested="${CLASSIFIER_REQUEST}"
    if [ -n "${requested}" ] && ! valid_classifier_id "${requested}"; then
        fatal "Unknown classifier '${requested}'. Choose ${CLASSIFIER_V24} (the default) or ${CLASSIFIER_V3}."
    fi

    local chosen how kept_model=""
    if [ -f "${CONFIG_FILE}" ]; then
        how="kept"
        kept_model="$(config_value MODEL_PATH)"
        if [ "${kept_model}" = "${MODEL_DIR}/${V24_MODEL_FILE}" ]; then
            chosen="${CLASSIFIER_V24}"
        else
            chosen="${CLASSIFIER_V3}"
        fi
    elif [ -n "${requested}" ]; then
        how="requested"
        chosen="${requested}"
    elif [ -f "${MODEL_DIR}/${V3_MODEL_FILE}" ]; then
        how="v3-files"
        chosen="${CLASSIFIER_V3}"
    else
        how="default"
        chosen="${DEFAULT_CLASSIFIER}"
    fi
    select_classifier "${chosen}"

    case "${how}" in
        kept)
            if [ "${chosen}" = "${CLASSIFIER_V24}" ] \
                || [ "${kept_model}" = "${MODEL_DIR}/${V3_MODEL_FILE}" ]; then
                CLASSIFIER_SUMMARY="$(classifier_summary "${chosen}")"
            elif [ -n "${kept_model}" ]; then
                CLASSIFIER_SUMMARY="the model in MODEL_PATH (${kept_model}) — this installer did not change it"
            else
                CLASSIFIER_SUMMARY="none named: ${CONFIG_FILE} has no MODEL_PATH, and this installer did not add one"
            fi
            info "Classifier: ${CLASSIFIER_SUMMARY}."
            info "  (kept from ${CONFIG_FILE} — an existing station's classifier is never changed by an install, update or repair.)"
            if [ -n "${requested}" ] && [ "${requested}" != "${chosen}" ]; then
                warn "--classifier ${requested} was NOT applied: ${CONFIG_FILE} already names this station's model,"
                warn "and changing a running station's classifier splits its detection history between two networks."
                warn "To switch on purpose, install the other classifier's files and edit MODEL_PATH, LABELS_PATH and"
                warn "METADATA_MODEL_PATH in that file — ${CLASSIFIER_GUIDE_URL} has the steps."
            fi
            ;;
        requested)
            CLASSIFIER_SUMMARY="$(classifier_summary "${chosen}")"
            info "Classifier: ${CLASSIFIER_SUMMARY} (--classifier ${chosen})."
            ;;
        v3-files)
            CLASSIFIER_SUMMARY="$(classifier_summary "${chosen}")"
            info "Classifier: ${CLASSIFIER_SUMMARY}."
            info "  The BirdNET+ V3.0 model is already in ${MODEL_DIR} and there is no ${CONFIG_FILE}, so this keeps it:"
            info "  a station restored from a backup must not change networks under its history."
            info "  Pass --classifier ${CLASSIFIER_V24} to use BirdNET V2.4 instead."
            ;;
        default)
            CLASSIFIER_SUMMARY="$(classifier_summary "${chosen}")"
            info "Classifier: ${CLASSIFIER_SUMMARY}."
            info "  (the default; pass --classifier ${CLASSIFIER_V3} on a new install for the BirdNET+ V3.0 preview.)"
            ;;
    esac
}

# The one-line description of a classifier for a station this run is setting up.
classifier_summary() {
    case "$1" in
        birdnet-v24) printf '%s' "BirdNET V2.4 + range filter v2 — the network BirdNET-Pi and birdnet-go run" ;;
        *)           printf '%s' "BirdNET+ V3.0 preview — a different network from BirdNET-Pi's; its detections differ by design" ;;
    esac
}

# Until main() knows better the default is selected, so every name the
# download and config code reads is defined whichever path reaches it first.
select_classifier "${DEFAULT_CLASSIFIER}"
