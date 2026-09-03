# ---------------------------------------------------------------------------
# Download and install the binary
#
# Release artifacts are gzipped tarballs of the form
#   birdnet-behavior-<version>-<target>.tar.gz
# containing a single top-level directory with the stripped binary alongside
# README, LICENSE, LICENSE-UPSTREAM, CHANGELOG, this script, and (since 0.6.0)
# a help/ directory holding the rendered operator manual served at /help/*. A
# single SHA256SUMS file is attached to each GitHub Release for verification.
# ---------------------------------------------------------------------------


# Put the new binary in place without the path ever being absent or short.
#
# `install -m 0755 src dst` is not atomic and does not fsync. Traced with
# strace, it does:
#
#     unlinkat(AT_FDCWD, "dst", 0)                            = 0
#     openat(AT_FDCWD, "src", O_RDONLY)                       = 3
#     openat(AT_FDCWD, "dst", O_WRONLY|O_CREAT|O_EXCL, 0600)  = 4
#
# — the working binary is unlinked *first*, then a fresh file is created at the
# same path and filled. Writing ~100 MB to an SD card is a multi-second window,
# and an upgrade is exactly when a solar or battery-backed box browns out.
# Afterwards `ExecStartPre` and `ExecStart` both fail, `Restart=always` with
# `StartLimitIntervalSec=0` retries every five minutes for ever, there is no web
# UI left to say so, and the previous binary was deleted rather than kept.
#
# That last part also made the *documented* recovery impossible:
# `docs/book/field/deployment.md` tells operators to "keep the previous binary
# in ${INSTALL_DIR}/${BINARY_NAME}.prev so a one-line `mv` rollback is
# possible", and nothing in the product ever created that file.
#
# So: keep the old one as `.prev`, write the new one beside it, flush it,
# prove it runs, and `mv` — `rename(2)` within one filesystem is atomic, so a
# reader either sees the whole old binary or the whole new one and never a hole.
#
# The in-tree Rust updater (`crates/birdnet-integrations/src/auto_update/`)
# already does exactly this. The installer, which is the path every real
# upgrade takes, did none of it.
install_binary_atomically() {
    local src="$1"
    local dst="${INSTALL_DIR}/${BINARY_NAME}"
    local staged="${dst}.new.$$"
    local prev="${dst}.prev"

    # Keep the outgoing binary before anything touches the path. `cp` rather
    # than `mv` so the live path stays valid until the rename below.
    if [ -f "${dst}" ]; then
        if cp -f "${dst}" "${prev}" 2>/dev/null; then
            chmod 0755 "${prev}" 2>/dev/null || true
        else
            warn "Could not save the previous binary to ${prev}; rollback will not be available."
        fi
    fi

    if ! install -m 0755 "${src}" "${staged}"; then
        rm -f "${staged}"
        fatal "Could not write the new binary to ${staged}."
    fi

    # Get it onto the medium before it becomes the live path. Without this a
    # completed install can still be partially persisted when the power goes a
    # moment later, which is the same failure with a longer fuse.
    sync "${staged}" 2>/dev/null || sync || true

    # Prove it runs before it becomes the thing that has to run. Catches a
    # wrong architecture, a truncated extraction, and a missing shared library
    # — while the working binary is still one `mv` away.
    if ! "${staged}" --version >/dev/null 2>&1; then
        rm -f "${staged}"
        fatal "The new binary would not start (\`${BINARY_NAME} --version\` failed); \
keeping the existing one at ${dst}."
    fi

    if ! mv -f "${staged}" "${dst}"; then
        rm -f "${staged}"
        fatal "Could not move the new binary into place at ${dst}."
    fi

    success "Binary installed to ${dst}"
    if [ -f "${prev}" ]; then
        info "Previous binary kept at ${prev} — roll back with: sudo mv ${prev} ${dst} && sudo systemctl restart ${SERVICE_NAME}"
    fi
}

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

    # Air-gapped / offline install: BIRDNET_BINARY_TARBALL points at a release
    # tarball already on disk (downloaded on another machine, or shipped on
    # media), so the install needs no network for the binary. The operator
    # vouches for a local file they placed themselves, so we skip the
    # SHA256SUMS round-trip and verify only the archive's internal layout.
    if [ -n "${BIRDNET_BINARY_TARBALL:-}" ]; then
        if [ ! -f "${BIRDNET_BINARY_TARBALL}" ]; then
            fatal "BIRDNET_BINARY_TARBALL=${BIRDNET_BINARY_TARBALL} is not a file."
        fi
        info "Using local binary tarball ${BIRDNET_BINARY_TARBALL} (offline install)."
        cp "${BIRDNET_BINARY_TARBALL}" "${workdir}/${archive}"
    else
        info "Downloading ${archive}…"
        if ! download "${archive_url}" "${workdir}/${archive}"; then
            fatal "Archive download failed. Check that release v${version} exists for ${arch}."
        fi

        info "Downloading SHA256SUMS for verification…"
        # A failed SHA256SUMS fetch used to warn and install anyway. That put
        # the choice of whether verification happened in the hands of whoever
        # controlled the network — and anyone able to substitute a 100 MB
        # binary can certainly drop one small request for the file that would
        # expose it. "Could not check" is not a weaker form of "checked out".
        if ! download "${sums_url}" "${workdir}/SHA256SUMS"; then
            fatal "SHA256SUMS could not be downloaded from ${sums_url}, so the archive cannot be verified. Refusing to install an unverified binary. Retry, or fetch the release and its SHA256SUMS by hand, check them with 'sha256sum -c', and install with BIRDNET_BINARY_TARBALL=/path/to/${archive}."
        fi

        # Verify *this* archive, by name.
        #
        # The previous command was `sha256sum -c SHA256SUMS --ignore-missing
        # --status --strict`, which answers a different question: "did anything
        # both listed and present fail?" With GNU coreutils 9.4 that exits 0
        # when our archive was never checked at all, as long as some other
        # listed file was present and matched — so "Checksum verified" could be
        # printed without our archive being verified. Nothing in the current
        # workdir makes that reachable, but the command has to assert what the
        # message claims, not something adjacent that happens to coincide.
        #
        # Narrow to the single line naming this archive first. Any directory
        # prefix and the binary-mode '*' are stripped and the line rewritten
        # with the bare name, because `sha256sum -c` looks up the path exactly
        # as written and would otherwise report a missing file for a
        # `release/${archive}`-style entry.
        local sums_line
        sums_line="$(awk -v want="${archive}" '
            { name = $2; sub(/^\*/, "", name); sub(/^.*\//, "", name)
              if (name == want) printf "%s  %s\n", $1, name }
        ' "${workdir}/SHA256SUMS")"

        if [ -z "${sums_line}" ]; then
            fatal "The published SHA256SUMS for v${version} has no entry for ${archive}, so it cannot be verified. Refusing to install. This usually means the release is incomplete for ${arch}, or the detected architecture is wrong."
        fi
        if [ "$(printf '%s\n' "${sums_line}" | wc -l)" -ne 1 ]; then
            fatal "The published SHA256SUMS for v${version} lists ${archive} more than once. Refusing to guess which digest is authoritative."
        fi

        printf '%s\n' "${sums_line}" >"${workdir}/SHA256SUMS.archive"
        # `--strict` also rejects a malformed digest, so a truncated or
        # HTML-error-page SHA256SUMS fails here rather than appearing to pass.
        if (cd "${workdir}" && sha256sum -c SHA256SUMS.archive --status --strict); then
            success "Checksum verified against SHA256SUMS (${archive})"
        else
            fatal "Checksum mismatch for ${archive} against published SHA256SUMS. The download is corrupt or tampered. Aborting install; nothing was written to ${INSTALL_DIR}."
        fi
    fi

    info "Extracting archive…"
    if ! tar -xzf "${workdir}/${archive}" -C "${workdir}"; then
        fatal "Archive extraction failed. The downloaded file may be corrupt."
    fi

    # The archive contains a single top-level directory named
    # birdnet-behavior-<version>-<target>. Locate the binary inside it.
    local extracted_binary
    # `awk 'NR==1'`, not `head -1`: with more than one match `find` is left
    # writing into a pipe `head` has already closed, and `set -euo pipefail`
    # turns that into a silent exit 141 — the installer stops with no output.
    # Verified: deterministic with 5000 matches, clean with one.
    extracted_binary="$(find "${workdir}" -mindepth 2 -maxdepth 3 -type f -name "${BINARY_NAME}" | awk 'NR==1')"
    if [ -z "${extracted_binary}" ] || [ ! -f "${extracted_binary}" ]; then
        fatal "Could not find '${BINARY_NAME}' binary inside the downloaded archive."
    fi

    # Stop the service here and not a moment earlier. Everything above can
    # fail — an unreachable release, an unverifiable checksum, a corrupt
    # archive — and none of it is a reason to take a working station off the
    # air. From this line on we have a verified binary in hand and the only
    # remaining obstacle is ETXTBSY, which is what the stop is for.
    stop_running_service_for_swap

    install_binary_atomically "${extracted_binary}"

    # Install the bundled operator manual (mdBook) if this release ships it, so
    # the dashboard's /help/* links work fully offline. The service points
    # BNB_HELP_DIR at ${HELP_DIR} (see 65-service.sh). Older releases have no
    # help/ in the tarball — we just skip, and /help 404s as it did before.
    local extracted_help
    extracted_help="$(find "${workdir}" -mindepth 2 -maxdepth 3 -type d -name help | awk 'NR==1')"
    if [ -n "${extracted_help}" ] && [ -d "${extracted_help}" ]; then
        rm -rf "${HELP_DIR}"
        install -d -m 0755 "$(dirname "${HELP_DIR}")"
        cp -a "${extracted_help}" "${HELP_DIR}"
        chmod -R a+rX "${HELP_DIR}"
        success "Operator manual installed to ${HELP_DIR} (served at /help)"
    else
        info "This release has no bundled manual; /help will be unavailable until you upgrade."
    fi
}
