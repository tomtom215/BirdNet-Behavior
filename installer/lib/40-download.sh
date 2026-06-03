# ---------------------------------------------------------------------------
# Download helpers (curl or wget) and release-version resolution
# ---------------------------------------------------------------------------

download() {
    local url="$1"
    local dest="$2"
    if command -v curl &>/dev/null; then
        curl -fsSL -L --retry 3 --retry-delay 2 -o "${dest}" "${url}"
    elif command -v wget &>/dev/null; then
        wget -q --tries=3 -O "${dest}" "${url}"
    else
        fatal "Neither curl nor wget is available. Please install one and retry."
    fi
}

# Large-file download helper — resumes on interrupt, shows a progress bar
# so the operator sees something is happening during the ~541 MB model pull.
#
#   download_large URL DEST [HUMAN_NAME]
#
# Behaviour:
#   - If DEST already exists, resume from its current byte offset (-C -).
#   - Print a progress bar to the terminal (-#).
#   - Up to 5 automatic retries with exponential backoff for transient errors.
#   - Treat HTTP errors as failures (-f).
#   - A definitive HTTP 404 fails immediately (no retries) so a missing asset
#     falls through to the next source at once instead of stalling ~10 s on
#     five back-off retries — `fetch_verified_model` then tries Zenodo. We do
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
        curl -fL -C - -# \
            --retry 5 --retry-delay 2 --retry-connrefused --retry-max-time 600 \
            --connect-timeout 30 \
            -o "${dest}" "${url}"
    elif command -v wget &>/dev/null; then
        # -c : resume; --show-progress to stderr; tolerate transient failures.
        wget -c --tries=5 --waitretry=2 --timeout=30 --show-progress -O "${dest}" "${url}"
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
        ver="$(grep '"tag_name"' "${tmp}" | sed -E 's/.*"v?([^"]+)".*/\1/' | head -1)"
        rm -f "${tmp}"
        if [ -n "${ver}" ]; then
            echo "${ver}"
            return
        fi
    fi
    rm -f "${tmp}"
    fatal "Could not determine latest release version. Pass --version x.y.z (or set VERSION=x.y.z) to install a specific version."
}
