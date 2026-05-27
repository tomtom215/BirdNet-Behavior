# ---------------------------------------------------------------------------
# Download and install the binary
#
# Release artifacts are gzipped tarballs of the form
#   birdnet-behavior-<version>-<target>.tar.gz
# containing a single top-level directory with the stripped binary alongside
# README, LICENSE, LICENSE-UPSTREAM, CHANGELOG, and this script. A single
# SHA256SUMS file is attached to each GitHub Release for verification.
# ---------------------------------------------------------------------------

install_binary() {
    local version="$1"
    local arch="$2"

    if [ -x "${INSTALL_DIR}/${BINARY_NAME}" ]; then
        local old_ver
        old_ver="$("${INSTALL_DIR}/${BINARY_NAME}" --version 2>/dev/null | awk '{print $NF}' || true)"
        if [ -n "${old_ver}" ]; then
            info "Existing install detected (v${old_ver}) — installing v${version}."
        fi
    fi

    local archive="${BINARY_NAME}-${version}-${arch}.tar.gz"
    local base_url="https://github.com/${REPO}/releases/download/v${version}"
    local archive_url="${base_url}/${archive}"
    local sums_url="${base_url}/SHA256SUMS"

    local workdir
    workdir="$(mktemp -d)"
    # shellcheck disable=SC2064
    trap "rm -rf '${workdir}'" RETURN

    info "Downloading ${archive}…"
    if ! download "${archive_url}" "${workdir}/${archive}"; then
        fatal "Archive download failed. Check that release v${version} exists for ${arch}."
    fi

    info "Downloading SHA256SUMS for verification…"
    if download "${sums_url}" "${workdir}/SHA256SUMS" 2>/dev/null; then
        # sha256sum -c expects files referenced in SHA256SUMS to be present
        # in the working directory, so verify from inside workdir.
        if (cd "${workdir}" && sha256sum -c SHA256SUMS --ignore-missing --status --strict) 2>/dev/null; then
            success "Checksum verified against SHA256SUMS"
        else
            fatal "Checksum mismatch for ${archive} against published SHA256SUMS. Aborting install."
        fi
    else
        warn "SHA256SUMS could not be downloaded — continuing without checksum verification."
    fi

    info "Extracting archive…"
    if ! tar -xzf "${workdir}/${archive}" -C "${workdir}"; then
        fatal "Archive extraction failed. The downloaded file may be corrupt."
    fi

    # The archive contains a single top-level directory named
    # birdnet-behavior-<version>-<target>. Locate the binary inside it.
    local extracted_binary
    extracted_binary="$(find "${workdir}" -mindepth 2 -maxdepth 3 -type f -name "${BINARY_NAME}" | head -1)"
    if [ -z "${extracted_binary}" ] || [ ! -f "${extracted_binary}" ]; then
        fatal "Could not find '${BINARY_NAME}' binary inside the downloaded archive."
    fi

    install -m 0755 "${extracted_binary}" "${INSTALL_DIR}/${BINARY_NAME}"
    success "Binary installed to ${INSTALL_DIR}/${BINARY_NAME}"
}
