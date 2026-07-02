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
# mismatch. If sha256sum is somehow unavailable (it is part of coreutils, which
# preflight requires) we warn and treat the file as unverifiable rather than
# blocking the install — consistent with install_binary's checksum handling.
verify_model_sha256() {
    local file="$1" expected="$2"
    if ! command -v sha256sum &>/dev/null; then
        warn "sha256sum not found — cannot verify $(basename "${file}") integrity."
        return 0
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

# Fetch one model file, trying GitHub first then Zenodo, and verify it against
# its pinned sha256. The downloaded file is only accepted once the checksum
# matches; a mismatch discards it and falls through to the next source.
#
#   fetch_verified_model DEST FILENAME EXPECTED_SHA HUMAN_NAME RESUMABLE
#
# RESUMABLE=1 routes through download_large (resume + progress bar) for the
# ~541 MB model; any other value uses the plain download helper (small labels).
# Returns 0 once a verified copy is in place, 1 if every source failed.
fetch_verified_model() {
    local dest="$1" filename="$2" expected_sha="$3" human="$4" resumable="$5"
    local src url label

    for src in github zenodo; do
        if [ "${src}" = "github" ]; then
            url="${MODEL_GH_BASE}/${filename}"
            label="GitHub release ${MODEL_RELEASE_TAG}"
        else
            url="${ZENODO_API}/${filename}/content"
            label="Zenodo"
        fi

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
        if ! fetch_verified_model "${model_dest}" "${MODEL_FILE}" "${MODEL_SHA256}" \
            "BirdNET+ V3.0 model (~541 MB)" 1; then
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
        if ! fetch_verified_model "${labels_dest}" "${LABELS_FILE}" "${LABELS_SHA256}" \
            "species labels CSV" 0; then
            fatal "Labels download failed or could not be verified. Check your internet connection and retry."
        fi
        chown "${SERVICE_USER}:${SERVICE_USER}" "${labels_dest}"
        success "Labels installed to ${labels_dest}"
    fi
}
