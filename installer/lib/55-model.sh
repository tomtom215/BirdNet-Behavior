# ---------------------------------------------------------------------------
# Download the BirdNET+ V3.0 model + labels.
#
# Origin: the stable, arch-independent `models-v3.0-preview3` GitHub release
# (so the binary and the model are fetched from the *same* host and the install
# is offline-capable after that one fetch), falling back to Zenodo — the
# upstream source — when the GitHub asset is missing (e.g. installing an older
# app release whose line predates the model release) or unreachable.
#
# Whichever host serves the bytes, each file is verified against the sha256
# pinned in 10-config.sh before it is accepted: those hashes are the integrity
# root of trust (they live in version-controlled, provenance-attested source),
# so a corrupted or tampered download from either origin is rejected and the
# next source is tried. The same hashes are published as the `SHA256SUMS` asset
# of the models release.
# ---------------------------------------------------------------------------

# Verify that FILE has the expected sha256. Returns 0 on a match, 1 on a
# mismatch.
#
# A missing sha256sum is fatal, not a pass. It used to `return 0` — the same
# value a verified file returns — so the one tool that could detect a tampered
# 541 MB model being absent counted as the model being fine. preflight()
# already refuses to run without sha256sum, so this is a backstop rather than
# a path an operator reaches; a backstop that returns success is not one.
verify_model_sha256() {
    local file="$1" expected="$2"
    if ! command -v sha256sum &>/dev/null; then
        # `${file##*/}` rather than basename: the one situation this branch
        # fires in is a broken PATH, and the abort message must not itself
        # depend on an external tool to render.
        fatal "sha256sum is not available, so ${file##*/} cannot be verified. Refusing to install an unverified model. Install coreutils and re-run."
    fi
    local actual
    actual="$(sha256sum "${file}" | awk '{print $1}')"
    if [ "${actual}" = "${expected}" ]; then
        return 0
    fi
    warn "  Checksum mismatch for $(basename "${file}")"
    warn "    expected: ${expected}"
    warn "    actual:   ${actual}"
    return 1
}

# Fetch one model file from the first origin that serves bytes matching its
# pinned sha256. A file is only accepted once the checksum matches; a mismatch
# discards it and falls through to the next origin.
#
#   fetch_verified_model DEST EXPECTED_SHA HUMAN_NAME RESUMABLE LABEL URL [LABEL URL...]
#
# The origins are passed as label/URL pairs rather than derived here, because
# the two model families do not share a URL shape: Zenodo needs
# `<api>/<file>/content` while both GitHub releases take `<base>/<file>`.
# Building them at the call site keeps that knowledge next to the config that
# defines it, and lets the geomodel use its own upstream without teaching this
# function about a third source.
#
# RESUMABLE=1 routes through download_large (resume + progress bar) for the
# ~541 MB classifier; any other value uses the plain download helper.
# Returns 0 once a verified copy is in place, 1 if every origin failed.
fetch_verified_model() {
    local dest="$1" expected_sha="$2" human="$3" resumable="$4"
    shift 4

    while [ "$#" -ge 2 ]; do
        local label="$1" url="$2"
        shift 2

        info "  Fetching ${human} from ${label}…"
        if [ "${resumable}" = "1" ]; then
            if ! download_large "${url}" "${dest}" "${human}"; then
                warn "  ${human}: download from ${label} failed; trying the next source."
                continue
            fi
        else
            if ! download "${url}" "${dest}"; then
                warn "  ${human}: download from ${label} failed; trying the next source."
                continue
            fi
        fi

        if verify_model_sha256 "${dest}" "${expected_sha}"; then
            success "  ${human}: sha256 verified (${label})."
            return 0
        fi

        warn "  ${human}: discarding the file from ${label} and trying the next source."
        rm -f "${dest}"
    done

    return 1
}

# The origin label/URL pairs for one classifier file: our models release first,
# then Zenodo.
classifier_origins() {
    local filename="$1"
    printf '%s\n' \
        "GitHub release ${MODEL_RELEASE_TAG}" "${MODEL_GH_BASE}/${filename}" \
        "Zenodo" "${ZENODO_API}/${filename}/content"
}

# The origin label/URL pairs for one geomodel file: our models release first,
# then the upstream birdnet-team release it is mirrored from.
geomodel_origins() {
    local filename="$1"
    printf '%s\n' \
        "GitHub release ${MODEL_RELEASE_TAG}" "${GEOMODEL_GH_BASE}/${filename}" \
        "upstream birdnet-team/geomodel ${GEOMODEL_VERSION}" "${GEOMODEL_UPSTREAM_BASE}/${filename}"
}

download_model() {
    local model_dest="${MODEL_DIR}/${MODEL_FILE}"
    local labels_dest="${MODEL_DIR}/${LABELS_FILE}"

    # Skip if already present (re-running installer).
    if [ -f "${model_dest}" ] && [ -f "${labels_dest}" ]; then
        success "Model already downloaded at ${MODEL_DIR} — skipping."
        return
    fi

    # Explicit skip: BIRDNET_SKIP_MODEL=1 lets an air-gapped operator stage the
    # ~541 MB model out-of-band (place it at ${MODEL_DIR} later), and lets a CI
    # install smoke test exercise the full flow without the large download. The
    # daemon won't detect until the model is in place, but the install, config,
    # unit, and web UI all come up.
    if [ "${BIRDNET_SKIP_MODEL:-0}" = "1" ]; then
        install -d -m 0750 -o "${SERVICE_USER}" -g "${SERVICE_USER}" "${MODEL_DIR}"
        loud_warn "BIRDNET_SKIP_MODEL=1 — the ML model was NOT downloaded." \
                  "The service will start but will detect NOTHING until you stage it:" \
                  "  place ${MODEL_FILE} and ${LABELS_FILE} in ${MODEL_DIR}," \
                  "  then restart the service."
        return
    fi

    info "Fetching the BirdNET+ V3.0 model (~541 MB FP32 ONNX) + labels…"
    info "  Primary source: GitHub release ${MODEL_RELEASE_TAG} (sha256-verified)."
    info "  Fallback:       Zenodo. This may take a few minutes on a slow link."

    install -d -m 0750 -o "${SERVICE_USER}" -g "${SERVICE_USER}" "${MODEL_DIR}"

    # Model (~541 MB) — resumable so a dropped connection picks up where it left
    # off on the next run instead of restarting from 0 MB.
    if [ ! -f "${model_dest}" ]; then
        local model_origins
        mapfile -t model_origins < <(classifier_origins "${MODEL_FILE}")
        if ! fetch_verified_model "${model_dest}" "${MODEL_SHA256}" \
            "BirdNET+ V3.0 model (~541 MB)" 1 "${model_origins[@]}"; then
            warn "Model download failed or could not be verified from any source."
            warn "Any partial file is kept at:"
            warn "  ${model_dest}"
            warn "Re-run this installer to resume from where it stopped."
            warn "Common causes: no internet connection, GitHub/Zenodo temporarily"
            warn "down, or disk full."
            fatal "Model download failed. Check the cause above and retry."
        fi
        chown "${SERVICE_USER}:${SERVICE_USER}" "${model_dest}"
        success "Model installed to ${model_dest}"
    fi

    # Labels (small file — no resume needed).
    if [ ! -f "${labels_dest}" ]; then
        local labels_origins
        mapfile -t labels_origins < <(classifier_origins "${LABELS_FILE}")
        if ! fetch_verified_model "${labels_dest}" "${LABELS_SHA256}" \
            "species labels CSV" 0 "${labels_origins[@]}"; then
            fatal "Labels download failed or could not be verified. Check your internet connection and retry."
        fi
        chown "${SERVICE_USER}:${SERVICE_USER}" "${labels_dest}"
        success "Labels installed to ${labels_dest}"
    fi
}

# ---------------------------------------------------------------------------
# Download the BirdNET geomodel + its labels (the species occurrence filter).
#
# Deliberately NON-FATAL, unlike the classifier. A station without the
# classifier detects nothing and must stop; a station without the geomodel
# detects everything and merely stops filtering by location, which is exactly
# how every release before this one behaved. Aborting an otherwise good install
# over a 14 MB optional download would be the worse failure — so this warns,
# leaves METADATA_MODEL_PATH unset, and `--doctor` reports the filter as off
# with the command to fix it.
#
# Both files are needed or neither is used: the model's 12 012 outputs are
# meaningless without the label file that names them, and the station refuses a
# model it cannot align. A half-download therefore removes what it got rather
# than leaving a configuration that cannot start.
#
# Sets GEOMODEL_INSTALLED=1 when both files are verified and in place, which is
# what 62-config-file.sh keys the METADATA_* settings on.
# ---------------------------------------------------------------------------
GEOMODEL_INSTALLED=0

download_geomodel() {
    local model_dest="${MODEL_DIR}/${GEOMODEL_FILE}"
    local labels_dest="${MODEL_DIR}/${GEOMODEL_LABELS_FILE}"

    if [ -f "${model_dest}" ] && [ -f "${labels_dest}" ]; then
        GEOMODEL_INSTALLED=1
        success "Geomodel already present at ${MODEL_DIR} — skipping."
        return 0
    fi

    # The same escape hatch the classifier honours: an air-gapped operator
    # stages the files by hand, and the install-smoke CI job skips the fetch.
    if [ "${BIRDNET_SKIP_MODEL:-0}" = "1" ]; then
        info "BIRDNET_SKIP_MODEL=1 — skipping the geomodel download too."
        return 0
    fi

    info "Fetching the BirdNET geomodel ${GEOMODEL_VERSION} (~14 MB) + labels…"
    info "  This is the species occurrence filter: it drops birds that do not"
    info "  occur near this station at this time of year."

    install -d -m 0750 -o "${SERVICE_USER}" -g "${SERVICE_USER}" "${MODEL_DIR}"

    local model_origins labels_origins
    mapfile -t model_origins < <(geomodel_origins "${GEOMODEL_FILE}")
    mapfile -t labels_origins < <(geomodel_origins "${GEOMODEL_LABELS_FILE}")

    if [ ! -f "${model_dest}" ] &&
        ! fetch_verified_model "${model_dest}" "${GEOMODEL_SHA256}" \
            "geomodel (~14 MB)" 0 "${model_origins[@]}"; then
        rm -f "${model_dest}"
        warn "Geomodel download failed or could not be verified from any source."
        warn "The station will run WITHOUT species occurrence filtering: every"
        warn "species the classifier knows stays a candidate wherever it is."
        warn "Re-run this installer to retry, then check with:"
        warn "  birdnet-behavior --doctor"
        return 0
    fi

    if [ ! -f "${labels_dest}" ] &&
        ! fetch_verified_model "${labels_dest}" "${GEOMODEL_LABELS_SHA256}" \
            "geomodel labels" 0 "${labels_origins[@]}"; then
        # The model alone cannot be used, and a configured-but-unusable pair is
        # worse than none: the daemon would refuse it on every start. Remove
        # both so the next run is a clean retry.
        rm -f "${labels_dest}" "${model_dest}"
        warn "Geomodel labels failed to download; removing the model too, since"
        warn "the station cannot use one without the other. Occurrence filtering"
        warn "is OFF. Re-run this installer to retry."
        return 0
    fi

    chown "${SERVICE_USER}:${SERVICE_USER}" "${model_dest}" "${labels_dest}"
    GEOMODEL_INSTALLED=1
    success "Geomodel installed to ${model_dest}"
    success "Species occurrence filtering is ON (threshold SF_THRESH, default 0.03)."
    return 0
}
