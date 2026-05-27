# ---------------------------------------------------------------------------
# Download BirdNET+ V3.0 model from Zenodo
# ---------------------------------------------------------------------------

download_model() {
    local model_dest="${MODEL_DIR}/${MODEL_FILE}"
    local labels_dest="${MODEL_DIR}/${LABELS_FILE}"

    # Skip if already present (re-running installer).
    if [ -f "${model_dest}" ] && [ -f "${labels_dest}" ]; then
        success "Model already downloaded at ${MODEL_DIR} — skipping."
        return
    fi

    info "Downloading BirdNET+ V3.0 model (~541 MB FP32 ONNX) from Zenodo…"
    info "  This may take a few minutes on a slow connection."

    install -d -m 0750 -o "${SERVICE_USER}" -g "${SERVICE_USER}" "${MODEL_DIR}"

    # Model (Zenodo API /content endpoint handles + in filenames correctly).
    # Uses download_large so a dropped connection picks up where it left off
    # on the next run instead of restarting from 0 MB.
    if [ ! -f "${model_dest}" ]; then
        if ! download_large "${ZENODO_API}/${MODEL_FILE}/content" "${model_dest}" "BirdNET+ V3.0 model (~541 MB)"; then
            warn "Model download was interrupted; the partial file is kept at:"
            warn "  ${model_dest}"
            warn "Re-run this installer to resume from where it stopped."
            warn "Common causes: no internet connection, Zenodo temporarily down, or disk full."
            fatal "Model download failed. Check the cause above and retry."
        fi
        chown "${SERVICE_USER}:${SERVICE_USER}" "${model_dest}"
        success "Model downloaded to ${model_dest}"
    fi

    # Labels (small file — no resume needed, but keep the consistent helper).
    if [ ! -f "${labels_dest}" ]; then
        if ! download "${ZENODO_API}/${LABELS_FILE}/content" "${labels_dest}"; then
            fatal "Labels download failed. Check your internet connection and retry."
        fi
        chown "${SERVICE_USER}:${SERVICE_USER}" "${labels_dest}"
        success "Labels downloaded to ${labels_dest}"
    fi
}
