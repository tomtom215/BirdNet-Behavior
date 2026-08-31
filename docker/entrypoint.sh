#!/bin/sh
# =============================================================================
# BirdNet-Behavior container entrypoint
#
# Responsibilities:
#   1. Auto-download the BirdNET+ V3.0 model on first run only, with resume
#      support, progress logging every 15 s, sha256 verification, and clear
#      failure diagnostics. The model is pulled from the project's stable GitHub
#      models release (sha256-verified), falling back to Zenodo (the upstream
#      source) when that asset is absent or unreachable.
#   2. Set container-appropriate defaults (listen on all interfaces).
#   3. Exec the birdnet-behavior binary, forwarding any extra arguments.
#
# Environment variables:
#   BIRDNET_MODEL_DIR           Model directory (default: /data/model)
#   BIRDNET_SKIP_MODEL_DOWNLOAD Set to "1" to skip auto-download (both models)
#   BIRDNET_METADATA_MODEL      Geomodel path; set it to opt out of the fetch
#   BIRDNET_GEOMODEL_BASE       Override the geomodel upstream origin (mirror)
#   BIRDNET_MODEL               Path to ONNX model file (auto-set if blank)
#   BIRDNET_LABELS              Path to labels CSV file (auto-set if blank)
#   BIRDNET_LISTEN              Web server address (default: 0.0.0.0:8502)
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
# docker/strip-blank-env.sh for the full reasoning and the one exception.
BNB_STRIP_LIB="${BNB_STRIP_LIB:-/usr/local/bin/strip-blank-env.sh}"
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
# Helpers
# ---------------------------------------------------------------------------
log()   { printf '[birdnet] %s\n' "$*"; }
warn()  { printf '[birdnet] WARNING: %s\n' "$*" >&2; }
die()   { printf '[birdnet] ERROR: %s\n' "$*" >&2; exit 1; }
rule()  { log '----------------------------------------------------------------'; }

# Format a byte count as a short, human-readable string (541 MB, 809 KB, …).
# Falls back gracefully if numfmt is missing.
human_bytes() {
    b="${1:-0}"
    if command -v numfmt >/dev/null 2>&1; then
        numfmt --to=si --suffix=B --format='%.1f' -- "$b" 2>/dev/null \
            || printf '%s B\n' "$b"
    else
        awk -v b="$b" 'BEGIN {
            split("B KB MB GB TB", u, " "); i=1;
            while (b >= 1000 && i < 5) { b = b / 1000; i++ }
            if (i == 1) { printf "%d %s\n", b, u[i] }
            else        { printf "%.1f %s\n", b, u[i] }
        }'
    fi
}

# Fetch the Content-Length header for a URL; prints "0" on failure.
remote_size() {
    url="$1"
    curl --silent --location --head --max-time 15 --retry 2 --retry-delay 3 "$url" 2>/dev/null \
        | awk 'BEGIN{IGNORECASE=1; v=0}
               /^content-length:/ {gsub(/\r/,""); v=$2}
               END{print v+0}'
}

# ---------------------------------------------------------------------------
# Model paths
# ---------------------------------------------------------------------------
MODEL_DIR="${BIRDNET_MODEL_DIR:-/data/model}"
MODEL_FILE="BirdNET+_V3.0-preview3_Global_11K_FP32.onnx"
LABELS_FILE="BirdNET+_V3.0-preview3_Global_11K_Labels.csv"

# Primary origin: the stable, arch-independent GitHub models release (shared by
# every app release). Fallback: Zenodo, the upstream source. Each file is
# verified against the pinned sha256 below before it is accepted, regardless of
# which origin served it — these hashes match installer/lib/10-config.sh and the
# models release's SHA256SUMS. (Override the origins for an air-gapped mirror via
# BIRDNET_MODEL_BASE / BIRDNET_ZENODO_BASE.)
MODEL_RELEASE_TAG="models-v3.0-preview3"
GH_BASE="${BIRDNET_MODEL_BASE:-https://github.com/tomtom215/BirdNet-Behavior/releases/download/${MODEL_RELEASE_TAG}}"
ZENODO_BASE="${BIRDNET_ZENODO_BASE:-https://zenodo.org/api/records/18247420/files}"
MODEL_SHA256="2a0f9efba1a98e3193ad3dfcb8323116a7de88e39545f3619a7ea46e3bb7d743"
LABELS_SHA256="8124b0ea2d187104c5e2cd95a0f937165647e20349c8fd34d4d5ef991821f8f0"

# ---------------------------------------------------------------------------
# Geomodel (species occurrence filter) — optional
# ---------------------------------------------------------------------------
# Separate from the classifier and versioned separately: the classifier says
# what it heard, the geomodel says which species plausibly occur at this
# latitude/longitude in this week of the year. Without it every one of the
# classifier's species stays a candidate wherever the container runs.
#
# The two do not score the same species list (12 012 against 11 560), so the
# geomodel's own label file ships beside it and is what maps one onto the
# other. Both are required or neither is used.
#
# Origins mirror the classifier's: our models release first, then the upstream
# birdnet-team release it is mirrored from. Pins match installer/lib/10-config.sh.
GEOMODEL_VERSION="v3.0.2"
GEOMODEL_FILE="BirdNET+_Geomodel_V3.0.2_Global_12K_FP32.onnx"
GEOMODEL_LABELS_FILE="BirdNET+_Geomodel_V3.0.2_Global_12K_Labels.txt"
GEOMODEL_SHA256="b151f680a47de5371f39b3df129aea5946ac6baa039582274f833b42eaf992ea"
GEOMODEL_LABELS_SHA256="c15818db07e55978d909a9bcd916cd0615b0183f789227d9516059151787c784"
GEOMODEL_UPSTREAM_BASE="${BIRDNET_GEOMODEL_BASE:-https://github.com/birdnet-team/geomodel/releases/download/${GEOMODEL_VERSION}}"

# Respect explicit overrides; otherwise use the default paths under MODEL_DIR.
: "${BIRDNET_MODEL:=${MODEL_DIR}/${MODEL_FILE}}"
: "${BIRDNET_LABELS:=${MODEL_DIR}/${LABELS_FILE}}"
export BIRDNET_MODEL BIRDNET_LABELS

# Deliberately NOT defaulted here. An operator who set these keeps them; one who
# did not gets them only once both files are on disk, because a path pointing at
# a file that never downloaded makes the daemon refuse the model on every start
# and --doctor report FAIL on a container that merely declined an optional 14 MB
# fetch.
GEOMODEL_USER_SET=0
[ -n "${BIRDNET_METADATA_MODEL:-}" ] && GEOMODEL_USER_SET=1

# ---------------------------------------------------------------------------
# sha256 verification
# ---------------------------------------------------------------------------
# Return 0 when FILE matches EXPECTED, 1 otherwise. If sha256sum is somehow
# missing (it ships in coreutils) we warn and treat the file as unverifiable
# rather than blocking startup.
verify_sha256() {
    file="$1"
    expected="$2"
    if ! command -v sha256sum >/dev/null 2>&1; then
        warn "sha256sum not available — cannot verify ${file##*/} integrity."
        return 0
    fi
    actual="$(sha256sum "$file" | awk '{print $1}')"
    [ "$actual" = "$expected" ]
}

# ---------------------------------------------------------------------------
# Single-URL download with resume + periodic progress logging
# ---------------------------------------------------------------------------
#
# curl runs in the background so a companion loop can print a single log line
# every 15 s (readable in `docker compose logs`, unlike the raw \r-based
# progress meter which is unreadable in non-TTY log streams).
#
# Partial downloads are resumed via `--continue-at -` so that a lost connection
# on a 500 MB fetch does not force the user to start over. On success the file
# is moved into place and 0 returned; on failure the partial is kept (for the
# next attempt to resume) and curl's exit code returned — the caller decides
# whether to fall back to another origin.
# ---------------------------------------------------------------------------
fetch_one() {
    dest="$1"
    url="$2"
    desc="$3"

    # Pre-flight: how big is the file on the server?
    total="$(remote_size "$url")"
    if [ "$total" -gt 0 ] 2>/dev/null; then
        total_h="$(human_bytes "$total")"
    else
        total=0
        total_h="unknown size"
    fi

    rule
    log "Downloading ${desc} (${total_h})"
    log "  from:  ${url}"
    log "  to:    ${dest}"
    log "  This runs only on first start. The model is cached in the"
    log "  Docker volume so subsequent container starts are instant."
    log "  Typical download: 1–3 min on fibre, 5–15 min on home broadband."
    rule

    tmpfile="${dest}.tmp"
    start_ts="$(date +%s)"

    # Background curl with resume support.
    curl --fail --location --silent --show-error \
         --retry 5 --retry-delay 10 --retry-max-time 900 \
         --connect-timeout 30 \
         --continue-at - \
         --output "${tmpfile}" \
         "${url}" &
    curl_pid=$!

    # Progress watcher — logs one line every 15 s while curl is still running.
    (
        while kill -0 "${curl_pid}" 2>/dev/null; do
            sleep 15
            kill -0 "${curl_pid}" 2>/dev/null || break
            got=0
            [ -f "${tmpfile}" ] && got="$(wc -c < "${tmpfile}" 2>/dev/null || echo 0)"
            elapsed=$(( $(date +%s) - start_ts ))
            if [ "${total}" -gt 0 ] 2>/dev/null && [ "${got}" -gt 0 ] 2>/dev/null; then
                pct=$(( got * 100 / total ))
                # crude speed average
                if [ "${elapsed}" -gt 0 ] 2>/dev/null; then
                    speed=$(( got / elapsed ))
                    log "  …${desc}: ${pct}%  ($(human_bytes "${got}") / ${total_h}, $(human_bytes "${speed}")/s, ${elapsed}s elapsed)"
                else
                    log "  …${desc}: ${pct}%  ($(human_bytes "${got}") / ${total_h})"
                fi
            else
                log "  …${desc}: $(human_bytes "${got}") so far (${elapsed}s elapsed)"
            fi
        done
    ) &
    watcher_pid=$!

    # Wait for curl; preserve its exit code.
    set +e
    wait "${curl_pid}"
    rc=$?
    set -e
    kill "${watcher_pid}" 2>/dev/null || true
    wait "${watcher_pid}" 2>/dev/null || true

    elapsed=$(( $(date +%s) - start_ts ))

    if [ "${rc}" -eq 0 ]; then
        mv "${tmpfile}" "${dest}"
        final="$(wc -c < "${dest}" 2>/dev/null || echo 0)"
        log "  done: ${desc} saved ($(human_bytes "${final}") in ${elapsed}s)"
        return 0
    fi

    # Failure path — keep the partial file so the next attempt can resume.
    partial=0
    [ -f "${tmpfile}" ] && partial="$(wc -c < "${tmpfile}" 2>/dev/null || echo 0)"
    warn "${desc}: curl exited ${rc} after ${elapsed}s"
    if [ "${partial}" -gt 0 ] 2>/dev/null; then
        warn "Partial file ($(human_bytes "${partial}")) kept at ${tmpfile}."
        warn "The next attempt will resume from where it left off."
    fi
    return "${rc}"
}

# ---------------------------------------------------------------------------
# Multi-origin, verified model fetch
# ---------------------------------------------------------------------------
#
# Fetch DEST trying GitHub (the stable models release) first, then Zenodo (the
# upstream source), and accept the result only once it matches EXPECTED sha256.
# A mismatch discards the file and falls through to the next origin. Both hosts
# serve the byte-identical file, so a partial left by a failed GitHub attempt is
# safely resumed against Zenodo, and the final sha256 check is the backstop.
# ---------------------------------------------------------------------------
ensure_model_file() {
    dest="$1"
    gh_url="$2"
    zenodo_url="$3"
    expected="$4"
    desc="$5"

    if [ -f "$dest" ]; then
        actual="$(wc -c < "$dest" 2>/dev/null || echo 0)"
        log "${desc}: already cached ($(human_bytes "$actual")) — skipping download."
        return 0
    fi

    for src in github zenodo; do
        if [ "$src" = "github" ]; then
            url="$gh_url"
            origin="GitHub release ${MODEL_RELEASE_TAG}"
        else
            url="$zenodo_url"
            origin="Zenodo"
        fi

        log "Fetching ${desc} from ${origin}…"
        if ! fetch_one "$dest" "$url" "$desc"; then
            warn "${desc}: ${origin} download failed — trying the next source."
            continue
        fi

        if verify_sha256 "$dest" "$expected"; then
            log "  ${desc}: sha256 verified (${origin})."
            return 0
        fi

        warn "${desc}: sha256 mismatch from ${origin} — discarding and trying the next source."
        rm -f "$dest" "${dest}.tmp"
    done

    warn "Common causes:"
    warn "  • no internet in the container (check the host's DNS/firewall)"
    warn "  • GitHub and Zenodo both temporarily unreachable (retry shortly)"
    warn "  • the volume is out of disk (df -h on the host's docker root)"
    die "Failed to download a verified ${desc} from GitHub or Zenodo."
}

# ---------------------------------------------------------------------------
# Model auto-download driver
# ---------------------------------------------------------------------------
if [ "${BIRDNET_SKIP_MODEL_DOWNLOAD:-}" = "1" ]; then
    log "BIRDNET_SKIP_MODEL_DOWNLOAD=1 — skipping model download."
    [ -f "${BIRDNET_MODEL}"  ] || warn "Model not found at ${BIRDNET_MODEL}"
    [ -f "${BIRDNET_LABELS}" ] || warn "Labels not found at ${BIRDNET_LABELS}"
else
    mkdir -p "${MODEL_DIR}"

    # Announce the model source once, up front, so users know what's happening
    # even if the model is already cached and no download is needed.
    log "BirdNET+ V3.0 model directory: ${MODEL_DIR}"
    log "Source: GitHub models release ${MODEL_RELEASE_TAG} (sha256-verified), Zenodo fallback."

    ensure_model_file \
        "${BIRDNET_MODEL}" \
        "${GH_BASE}/${MODEL_FILE}" \
        "${ZENODO_BASE}/${MODEL_FILE}/content" \
        "${MODEL_SHA256}" \
        "BirdNET+ V3.0 model (ONNX)"

    ensure_model_file \
        "${BIRDNET_LABELS}" \
        "${GH_BASE}/${LABELS_FILE}" \
        "${ZENODO_BASE}/${LABELS_FILE}/content" \
        "${LABELS_SHA256}" \
        "species labels CSV"

    log "Model ready."
fi

# ---------------------------------------------------------------------------
# Geomodel auto-download driver (non-fatal)
# ---------------------------------------------------------------------------
# Unlike the classifier, a missing geomodel is not fatal: the container starts
# and detects, it simply does not filter by location — which is how every
# release before this one behaved. Refusing to start over an optional 14 MB
# download would be the worse failure.

# Fetch one geomodel file, our release first then upstream. Returns non-zero
# instead of dying, so the caller can degrade rather than abort.
ensure_geomodel_file() {
    dest="$1"
    filename="$2"
    expected="$3"
    desc="$4"

    if [ -f "$dest" ]; then
        log "${desc}: already cached — skipping download."
        return 0
    fi

    for src in mirror upstream; do
        if [ "$src" = "mirror" ]; then
            url="${GH_BASE}/${filename}"
            origin="GitHub release ${MODEL_RELEASE_TAG}"
        else
            url="${GEOMODEL_UPSTREAM_BASE}/${filename}"
            origin="upstream birdnet-team/geomodel ${GEOMODEL_VERSION}"
        fi

        log "Fetching ${desc} from ${origin}…"
        if ! fetch_one "$dest" "$url" "$desc"; then
            warn "${desc}: ${origin} download failed — trying the next source."
            continue
        fi

        if verify_sha256 "$dest" "$expected"; then
            log "  ${desc}: sha256 verified (${origin})."
            return 0
        fi

        warn "${desc}: sha256 mismatch from ${origin} — discarding and trying the next source."
        rm -f "$dest" "${dest}.tmp"
    done

    rm -f "$dest" "${dest}.tmp"
    return 1
}

if [ "${GEOMODEL_USER_SET}" = "1" ]; then
    log "BIRDNET_METADATA_MODEL is set explicitly — leaving the geomodel alone."
elif [ "${BIRDNET_SKIP_MODEL_DOWNLOAD:-}" = "1" ]; then
    log "BIRDNET_SKIP_MODEL_DOWNLOAD=1 — skipping the geomodel download too."
else
    mkdir -p "${MODEL_DIR}"
    geomodel_path="${MODEL_DIR}/${GEOMODEL_FILE}"
    geolabels_path="${MODEL_DIR}/${GEOMODEL_LABELS_FILE}"

    if ensure_geomodel_file "${geomodel_path}" "${GEOMODEL_FILE}" \
        "${GEOMODEL_SHA256}" "geomodel (~14 MB)" &&
        ensure_geomodel_file "${geolabels_path}" "${GEOMODEL_LABELS_FILE}" \
            "${GEOMODEL_LABELS_SHA256}" "geomodel labels"; then
        BIRDNET_METADATA_MODEL="${geomodel_path}"
        BIRDNET_METADATA_LABELS="${geolabels_path}"
        export BIRDNET_METADATA_MODEL BIRDNET_METADATA_LABELS
        log "Species occurrence filtering: ON (set BIRDNET_SF_THRESH to tune; default 0.03)."
    else
        # The model alone is unusable — the station refuses a geomodel it cannot
        # align — so neither is left behind to be half-configured next start.
        rm -f "${geomodel_path}" "${geolabels_path}"
        warn "Geomodel unavailable — species occurrence filtering is OFF."
        warn "Every species the classifier knows stays a candidate wherever this"
        warn "station is. Restart the container to retry, then check with:"
        warn "  docker exec <container> birdnet-behavior --doctor"
    fi
fi

# ---------------------------------------------------------------------------
# Container defaults
# ---------------------------------------------------------------------------
# The binary defaults to 127.0.0.1:8502 (loopback only), which is unreachable
# from outside the container.  Override to bind on all interfaces unless the
# user has already set BIRDNET_LISTEN explicitly.
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
rule
exec /usr/local/bin/birdnet-behavior "$@"
