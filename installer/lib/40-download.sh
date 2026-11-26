# ---------------------------------------------------------------------------
# Download helpers (curl or wget) and release-version resolution
# ---------------------------------------------------------------------------

# Every origin this installer talks to is https (GitHub). Say so to curl and wget, so that a redirect cannot move a
# download onto plain http or an older TLS, where the SHA256SUMS that the
# archive is checked against would be read over a channel anyone on the path
# can rewrite.
CURL_HTTPS_ONLY=(--proto '=https' --proto-redir '=https' --tlsv1.2)

download() {
    local url="$1"
    local dest="$2"
    if command -v curl &>/dev/null; then
        curl "${CURL_HTTPS_ONLY[@]}" -fsSL -L --retry 3 --retry-delay 2 -o "${dest}" "${url}"
    elif command -v wget &>/dev/null; then
        wget --https-only -q --tries=3 -O "${dest}" "${url}"
    else
        fatal "Neither curl nor wget is available. Please install one and retry."
    fi
}

# Large-file download helper — resumes on interrupt, shows a progress bar
# so the operator sees something is happening during a large download (the
# macOS binary; every model file is installed by the binary itself).
#
#   download_large URL DEST [HUMAN_NAME]
#
# Behaviour:
#   - If DEST already exists, resume from its current byte offset (-C -).
#   - Print a progress bar to the terminal (-#).
#   - Up to 5 automatic retries with exponential backoff for transient errors.
#   - Treat HTTP errors as failures (-f).
#   - A definitive HTTP 404 fails immediately (no retries) instead of stalling
#     ~10 s on five back-off retries. We do
#     NOT pass --retry-all-errors (which would retry the 404); curl's default
#     --retry already covers the transient cases (timeouts, 5xx, 429), and
#     --retry-connrefused keeps a slow-to-wake CDN resilient.
#   - Leave the partial file in place on failure so the next run can resume.
download_large() {
    local url="$1"
    local dest="$2"
    local name="${3:-${dest##*/}}"
    info "  Fetching ${name}…"
    if command -v curl &>/dev/null; then
        # -C - : resume; -# : progress bar.
        curl "${CURL_HTTPS_ONLY[@]}" -fL -C - -# \
            --retry 5 --retry-delay 2 --retry-connrefused --retry-max-time 600 \
            --connect-timeout 30 \
            -o "${dest}" "${url}"
    elif command -v wget &>/dev/null; then
        # -c : resume; --show-progress to stderr; tolerate transient failures.
        wget --https-only -c --tries=5 --waitretry=2 --timeout=30 --show-progress -O "${dest}" "${url}"
    else
        fatal "Neither curl nor wget is available. Please install one and retry."
    fi
}

resolve_version() {
    if [ -n "${VERSION:-}" ]; then
        echo "${VERSION}"
        return
    fi
    info "Querying latest release from GitHub…"
    local api_url="https://api.github.com/repos/${REPO}/releases/latest"
    local tmp
    tmp="$(mktemp)"
    if download "${api_url}" "${tmp}" 2>/dev/null; then
        local ver
        ver="$(grep '"tag_name"' "${tmp}" | sed -E 's/.*"v?([^"]+)".*/\1/' | awk 'NR==1')"
        rm -f "${tmp}"
        if [ -n "${ver}" ]; then
            echo "${ver}"
            return
        fi
    fi
    rm -f "${tmp}"
    fatal "Could not determine latest release version. Pass --version x.y.z (or set VERSION=x.y.z) to install a specific version."
}
