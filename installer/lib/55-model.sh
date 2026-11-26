# ---------------------------------------------------------------------------
# Installing the models: the binary does it (MDL-09).
#
# Every model file a station runs — BirdNET V2.4 and its range filter, the
# BirdNET+ V3.0 preview and its geomodel — is installed by
# `birdnet-behavior --install-model <id>`. The binary fetches each file from its
# origins in order (this project's release first, then the publisher's), resumes
# a transfer that dropped, gives up on one that stalls, tries the origins again a
# bounded number of times (BIRDNET_MODEL_RETRY_WAITS), checks the sha256 and the
# size it pins, and renames the file into place only when it matches; a file
# already there is re-hashed rather than trusted for being there, and one an
# earlier version of this script left under the publisher's name is adopted in
# place when it verifies, not downloaded again. This script asks for the files
# and reads the binary's exit status; what they must hash to, and where they
# come from, appears nowhere here.
#
# The same call on every run, install, update and repair alike, so a model that
# a bad card damaged after it was installed is found and replaced by the next
# `install.sh repair`, and a filter whose download failed at install is tried
# again by the next update — the filters used to be fetched once with no retry.
# ---------------------------------------------------------------------------

# Run `birdnet-behavior --install-model ID` into ${MODEL_DIR}. Output is the
# binary's own, shown as it comes: it says what it is installing, how far it
# has got, and why when it refuses. The binary is the installed one, or
# MODEL_BINARY when that is set (the macOS path, whose binary lives elsewhere).
install_model_with_binary() {
    local id="$1" bin="${MODEL_BINARY:-${INSTALL_DIR}/${BINARY_NAME}}"
    shift
    if [ ! -x "${bin}" ]; then
        warn "${bin} is not installed, so ${id} cannot be installed with it."
        return 127
    fi
    # The child gets the model directory and the retry schedule, and nothing
    # else of the installer's own settings: the binary reports every BIRDNET_*
    # variable it does not read, so `BIRDNET_CLASSIFIER=… sudo -E bash
    # install.sh` would open the install with "BIRDNET_CLASSIFIER is not a
    # variable this station reads". `--config /dev/null` is an empty config:
    # there is no config yet on a fresh install, and without one the binary
    # warns that it found none. Any further arguments are the binary's
    # (`--from-file PATH`).
    env -u BIRDNET_CLASSIFIER -u BIRDNET_SKIP_MODEL -u BIRDNET_NONINTERACTIVE \
        -u BIRDNET_BINARY_TARBALL -u BIRDNET_SKIP_GLIBC_CHECK -u BIRDNET_LOCAL_MODEL \
        BIRDNET_MODEL_RETRY_WAITS="${MODEL_RETRY_WAITS}" \
        BIRDNET_MODEL_DIR="${MODEL_DIR}" "${bin}" --config /dev/null --install-model "${id}" "$@"
}

# How many rounds the binary makes of a model's origins under
# MODEL_RETRY_WAITS, for the messages.
model_attempts() {
    local -a waits=()
    read -r -a waits <<<"${MODEL_RETRY_WAITS}"
    echo $(( ${#waits[@]} + 1 ))
}

# Record that the classifier is not installed, and say so now. The install
# carries on; print_summary repeats it last, and the run exits
# EXIT_NO_CLASSIFIER (10-config.sh). $1 is why, as a clause.
classifier_not_installed() {
    CLASSIFIER_MISSING=1
    CLASSIFIER_MISSING_WHY="$1"
    loud_warn "THE CLASSIFIER IS NOT INSTALLED: $1." \
              "The install carries on, so the configuration, the service and the dashboard" \
              "exist — but the station will detect NOTHING until the model is installed." \
              "How to install it is at the end of this run."
}

# Give the service user what the binary wrote into ${MODEL_DIR}: the files and
# their verification records.
own_model_dir() {
    chown -R "${SERVICE_USER}:${SERVICE_USER}" "${MODEL_DIR}" 2>/dev/null || true
}

# The selected classifier, from its origins or from --local-model.
download_classifier() {
    if [ "${BIRDNET_SKIP_MODEL:-0}" = "1" ]; then
        install -d -m 0750 -o "${SERVICE_USER}" -g "${SERVICE_USER}" "${MODEL_DIR}"
        local filter_files="${V24_RANGE_FILE}"
        [ "${FILTER_ID}" = "${V3_GEOMODEL_ID}" ] && filter_files="${GEOMODEL_FILE} and ${GEOMODEL_LABELS_FILE}"
        loud_warn "BIRDNET_SKIP_MODEL=1 — the ML model was NOT installed." \
                  "The service will start but will detect NOTHING until you stage it:" \
                  "  put ${MODEL_FILE} and ${LABELS_FILE} (and ${filter_files} to filter by location)" \
                  "  in ${MODEL_DIR}, then restart the service. They are what" \
                  "  ${BINARY_NAME} --install-model ${CLASSIFIER} and ${FILTER_ID} write."
        return
    fi

    info "Installing the ${CLASSIFIER_NAME} classifier (${MODEL_SIZE_HINT}) + labels with ${BINARY_NAME} --install-model ${CLASSIFIER}…"
    info "  The binary checks each file's sha256 and size before it keeps it, resumes a"
    info "  download that drops, and tries its origins $(model_attempts) time(s) before giving up."
    case "${CLASSIFIER}" in
        "${CLASSIFIER_V24}") info "  BirdNET is licensed CC BY-NC-SA 4.0: free to use, but not for commercial purposes." ;;
        *)                   info "  BirdNET+ V3.0 is licensed CC BY-SA 4.0." ;;
    esac

    install -d -m 0750 -o "${SERVICE_USER}" -g "${SERVICE_USER}" "${MODEL_DIR}"

    if [ -n "${LOCAL_MODEL}" ]; then
        # The binary checks the file against the sha256 AND size it pins — the
        # same check a download passes — and copies it into place only when
        # both match. A file that does not match will not match on a retry.
        info "  From ${LOCAL_MODEL}, checked against the same sha256 and size as a download."
        if ! install_model_with_binary "${CLASSIFIER}" --from-file "${LOCAL_MODEL}"; then
            own_model_dir
            classifier_not_installed "${LOCAL_MODEL} was refused: it is not the ${CLASSIFIER_NAME} model this release pins (the reason is above)"
            return 0
        fi
    elif ! install_model_with_binary "${CLASSIFIER}"; then
        own_model_dir
        warn "${CLASSIFIER_NAME} could not be installed (the reason is above)."
        warn "Common causes: no internet connection, every origin of the model unreachable,"
        warn "less free space in ${MODEL_DIR} than the model plus 512 MiB, or a ${BINARY_NAME}"
        warn "older than this installer (it answers that the model is unknown) — install a current release."
        classifier_not_installed "${CLASSIFIER_NAME} could not be downloaded and verified ($(model_attempts) attempts)"
        return 0
    fi
    own_model_dir
    success "${CLASSIFIER_NAME} model and labels installed and verified in ${MODEL_DIR}"
}

# The occurrence filter that belongs to the selected classifier, and only that
# one: the V2.4 range filter for V2.4, the geomodel for V3.0.
#
# Deliberately NON-FATAL, unlike the classifier. A station without the
# classifier detects nothing and must stop; a station without the filter
# detects everything and merely stops filtering by location, so aborting an
# otherwise good install over an optional 14–30 MB download would be the worse
# failure. It warns, leaves METADATA_MODEL_PATH commented out, and `--doctor`
# reports the filter as off with the command that fixes it.
#
# Sets RANGE_FILTER_INSTALLED=1 or GEOMODEL_INSTALLED=1 once the binary has the
# files in place and verified, which is what 62-config-file.sh keys the
# METADATA_* settings on.
RANGE_FILTER_INSTALLED=0
GEOMODEL_INSTALLED=0

download_occurrence_filter() {
    if [ "${BIRDNET_SKIP_MODEL:-0}" = "1" ]; then
        info "BIRDNET_SKIP_MODEL=1 — skipping the occurrence filter install too."
        return 0
    fi
    local what="the BirdNET V2.4 range filter v2 (~30 MB)"
    [ "${FILTER_ID}" = "${V3_GEOMODEL_ID}" ] && what="the BirdNET geomodel (~14 MB) + labels"

    info "Installing ${what} with ${BINARY_NAME} --install-model ${FILTER_ID}…"
    info "  This is the species occurrence filter: it drops birds that do not"
    info "  occur near this station at this time of year (it needs LATITUDE and LONGITUDE)."

    install -d -m 0750 -o "${SERVICE_USER}" -g "${SERVICE_USER}" "${MODEL_DIR}"

    if ! install_model_with_binary "${FILTER_ID}"; then
        own_model_dir
        warn "The occurrence filter could not be installed (the reason is above)."
        warn "The station will run WITHOUT species occurrence filtering: every"
        warn "species the classifier knows stays a candidate wherever it is."
        warn "Re-run this installer to retry (sudo bash install.sh repair), then check with:"
        warn "  birdnet-behavior --doctor"
        return 0
    fi
    own_model_dir
    if [ "${FILTER_ID}" = "${V3_GEOMODEL_ID}" ]; then
        GEOMODEL_INSTALLED=1
    else
        RANGE_FILTER_INSTALLED=1
    fi
    success "Occurrence filter installed and verified in ${MODEL_DIR}"
    # Whether filtering is on depends on the config naming it, which
    # write_config settles and reports.
    return 0
}

# The V3.0 files under the names this installer gave them before the binary
# installed them, as config keys and file names: KEY LEGACY_NAME NEW_NAME.
legacy_model_names() {
    printf '%s %s %s\n' \
        MODEL_PATH "${V3_LEGACY_MODEL_FILE}" "${V3_MODEL_FILE}" \
        LABELS_PATH "${V3_LEGACY_LABELS_FILE}" "${V3_LABELS_FILE}" \
        METADATA_MODEL_PATH "${GEOMODEL_LEGACY_FILE}" "${GEOMODEL_FILE}" \
        METADATA_LABELS_PATH "${GEOMODEL_LEGACY_LABELS_FILE}" "${GEOMODEL_LABELS_FILE}"
}

# Move a kept config off the old V3.0 file names, and then remove the old files.
#
# The binary has, by now, adopted each old file that verified under its new
# name — a hard link, so no second copy and no download — and a config line that
# names the old path exactly is the one this installer wrote, so it is pointed at
# the new name. Only a line whose new file is really there is moved: a station
# whose model could not be installed keeps the path it had. An old file is
# removed only once no active line of the config names it (and only when the
# new one exists), so nothing the station loads disappears under it; a file an
# operator pointed somewhere else is never touched.
move_config_off_legacy_names() {
    [ -f "${CONFIG_FILE}" ] || return 0
    local key old new moved=0 active
    while read -r key old new; do
        [ -e "${MODEL_DIR}/${new}" ] || continue
        if [ "$(config_value "${key}")" = "${MODEL_DIR}/${old}" ]; then
            sed -i "s|^\\([[:space:]]*${key}[[:space:]]*=\\).*\$|\\1${MODEL_DIR}/${new}|" "${CONFIG_FILE}"
            moved=1
        fi
        active="$(grep -E '^[[:space:]]*[A-Z_0-9]+[[:space:]]*=' "${CONFIG_FILE}" || true)"
        if [ -e "${MODEL_DIR}/${old}" ] && ! grep -qF -- "${MODEL_DIR}/${old}" <<<"${active}"; then
            rm -f "${MODEL_DIR}/${old}" "${MODEL_DIR}/${old}.verified"
        fi
    done < <(legacy_model_names)
    if [ "${moved}" = 1 ]; then
        success "Pointed ${CONFIG_FILE} at the model files under the names ${BINARY_NAME} gives them (the same files, verified)."
    fi
}
