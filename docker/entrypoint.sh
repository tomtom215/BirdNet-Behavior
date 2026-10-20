#!/bin/sh
# =============================================================================
# BirdNet-Behavior container entrypoint
#
# Responsibilities:
#   1. Choose the classifier (BirdNET V2.4 by default, the BirdNET+ V3.0 preview
#      on request; an existing V3.0 volume stays on V3.0) and say which.
#   2. Install its files on first run only. BirdNET V2.4 and its range filter
#      are installed by the binary itself (`birdnet-behavior --install-model`,
#      which checks every file's sha256 and size and renames it into place
#      atomically); nothing in this script knows what they must hash to. The
#      BirdNET+ V3.0 preview is downloaded here, with resume support, progress
#      logging every 15 s and sha256 verification, from the project's stable
#      GitHub models release, falling back to Zenodo (the upstream source) when
#      that asset is absent or unreachable.
#   3. Point the daemon at the species-occurrence filter that pairs with the
#      classifier (the V2.4 range filter v2, or the V3.0 geomodel).
#   4. Set container-appropriate defaults (listen on all interfaces).
#   5. Exec the birdnet-behavior binary, forwarding any extra arguments.
#
# Environment variables:
#   BIRDNET_CLASSIFIER          birdnet-v24 (BirdNET V2.4 + range filter v2) or
#                               birdnet-v3 (BirdNET+ V3.0 preview + geomodel).
#                               Unset: a new volume gets birdnet-v24; a volume
#                               that already holds the V3.0 model keeps running
#                               it, because an upgrade must never switch a
#                               station's model under its history. This script
#                               reads it and removes it before starting the
#                               daemon, which does not.
#   BIRDNET_MODEL_DIR           Model directory (default: /data/model)
#   BIRDNET_SKIP_MODEL_DOWNLOAD Set to "1" to skip auto-download (both models)
#   BIRDNET_METADATA_MODEL      Geomodel path; set it to opt out of the fetch
#   BIRDNET_GEOMODEL_BASE       Override the geomodel upstream origin (mirror)
#   BIRDNET_MODEL               Path to ONNX model file (auto-set if blank)
#   BIRDNET_LABELS              Path to labels CSV file (auto-set if blank)
#   BIRDNET_LISTEN              Web server address (default: 0.0.0.0:8502)
#   TZ                          IANA zone the station files detections under
#                               (unset: UTC, and a warning on the way in)
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
# docker/strip-blank-env.sh for the full reasoning and the two exceptions.
BNB_STRIP_LIB="${BNB_STRIP_LIB:-/usr/local/bin/strip-blank-env.sh}"
# The daemon, which is also what installs BirdNET V2.4 (`--install-model`). A
# variable so a test can put a stub there; nothing sets it in an image.
BNB_BINARY="${BNB_BINARY:-/usr/local/bin/birdnet-behavior}"
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
# Timezone (ON-7)
# ---------------------------------------------------------------------------
# Detections are filed under the container's local hours, which come from TZ:
# glibc reads it, and SQLite's localtime — the one source of the station's
# offset — with it. Unset, the container runs on UTC and every hour on the
# dashboard is a UTC hour. Set to a name the image does not know, glibc falls
# back to UTC just as silently, which is the worse failure because it looks
# configured. Either way, say so on the way in; neither stops the station.
case "${TZ:-}" in
    "")
        printf '[birdnet] WARNING: TZ is not set: the station runs on UTC and files detections under UTC hours. Set TZ=<Area/City> in .env (for example TZ=Europe/Berlin)\n' >&2
        ;;
    *)
        if [ ! -f "/usr/share/zoneinfo/$TZ" ]; then
            printf '[birdnet] WARNING: TZ=%s is not a zone this image knows, so the station runs on UTC. Use an IANA name such as Europe/Berlin\n' "$TZ" >&2
        fi
        ;;
esac

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

# The final HTTP status for a URL (after redirects), from a HEAD request;
# "000" when the host could not be reached at all.
remote_status() {
    url="$1"
    curl --silent --location --head --output /dev/null --write-out '%{http_code}' \
        --max-time 15 --retry 2 --retry-delay 3 "$url" 2>/dev/null || true
}

# ---------------------------------------------------------------------------
# Model paths, the classifier choice, and the V3.0 pins
# ---------------------------------------------------------------------------
MODEL_DIR="${BIRDNET_MODEL_DIR:-/data/model}"

# BirdNET V2.4 + range filter v2 (the default). The NAMES `birdnet-behavior
# --install-model` gives the files, so the daemon can be pointed at them.
# Nothing else about them is here — no digest, no URL, no size: the binary
# installs them and owns what they must hash to (model_catalog.rs). The labels
# are a file the binary itself carries (BirdNET-Pi's English names on V2.4's
# class order); the file birdnet-go publishes names 460 of 6 522 classes
# differently, which would split a species in two the day a BirdNET-Pi history
# is imported. installer/test/model-pins-match-catalogue.sh fails if a V2.4
# digest or URL ever appears in this file.
V24_MODEL_ID="birdnet-v24"
V24_RANGE_ID="birdnet-v24-range-v2"
V24_MODEL_FILE="birdnet-v24.onnx"
V24_LABELS_FILE="birdnet-v24_labels.txt"
V24_RANGE_FILE="birdnet-v24-range-v2.onnx"

# BirdNET+ V3.0 preview (opt-in; what every volume created before V2.4 became
# the default is running). Primary origin: the stable, arch-independent GitHub
# models release (shared by every app release). Fallback: Zenodo, the upstream
# source. Each file is verified against the pinned sha256 below before it is
# accepted, regardless of which origin served it — these hashes match
# installer/lib/10-config.sh, the Rust catalogue and the models release's
# SHA256SUMS. (Override the origins for an air-gapped mirror via
# BIRDNET_MODEL_BASE / BIRDNET_ZENODO_BASE; they apply to V3.0 only.)
V3_MODEL_FILE="BirdNET+_V3.0-preview3_Global_11K_FP32.onnx"
V3_LABELS_FILE="BirdNET+_V3.0-preview3_Global_11K_Labels.csv"
V3_MODEL_SHA256="2a0f9efba1a98e3193ad3dfcb8323116a7de88e39545f3619a7ea46e3bb7d743"
V3_LABELS_SHA256="8124b0ea2d187104c5e2cd95a0f937165647e20349c8fd34d4d5ef991821f8f0"
V3_MODEL_RELEASE_TAG="models-v3.0-preview3"
ZENODO_BASE="${BIRDNET_ZENODO_BASE:-https://zenodo.org/api/records/18247420/files}"

# ---------------------------------------------------------------------------
# Which classifier does this container run?
# ---------------------------------------------------------------------------
# In order:
#   1. BIRDNET_CLASSIFIER, when set. An unknown value stops the container with
#      the two valid ones; a typo must not quietly run the other network.
#   2. BIRDNET_MODEL, when set: pointing it at the V2.4 file this script
#      manages is V2.4; pointing it at anything else is a model of the
#      operator's own, which keeps the V3.0 defaults it always had for labels
#      and the geomodel (they are matched by name, so they suit any classifier)
#      rather than losing them on upgrade.
#   3. A volume that already holds the V3.0 model file keeps running V3.0. An
#      upgrade must never switch the model under a running station: its history
#      would be rows two different networks scored. One log line says so and
#      says how to change it.
#   4. Otherwise a new volume gets BirdNET V2.4, the network BirdNET-Pi and
#      birdnet-go run.
#
# Sets CLASSIFIER and what the rest of the script reads for it (the unprefixed
# MODEL_FILE ... names), and logs one "Classifier:" line. Needs MODEL_DIR; must
# run before BIRDNET_MODEL is defaulted below, which would hide whether the
# operator set it.
select_classifier() {
    requested="${BIRDNET_CLASSIFIER:-}"
    case "$requested" in
        "" | birdnet-v24 | birdnet-v3) ;;
        *) die "BIRDNET_CLASSIFIER=${requested} is not a classifier this image knows. Use birdnet-v24 (the default) or birdnet-v3." ;;
    esac

    if [ -n "$requested" ]; then
        CLASSIFIER="$requested"
        how="explicit"
    elif [ -n "${BIRDNET_MODEL:-}" ]; then
        if [ "${BIRDNET_MODEL}" = "${MODEL_DIR}/${V24_MODEL_FILE}" ]; then
            CLASSIFIER="birdnet-v24"
            how="model-v24"
        elif [ "${BIRDNET_MODEL}" = "${MODEL_DIR}/${V3_MODEL_FILE}" ]; then
            CLASSIFIER="birdnet-v3"
            how="model-v3"
        else
            CLASSIFIER="birdnet-v3"
            how="model-own"
        fi
    elif [ -f "${MODEL_DIR}/${V3_MODEL_FILE}" ]; then
        CLASSIFIER="birdnet-v3"
        how="volume-v3"
    else
        CLASSIFIER="birdnet-v24"
        how="default"
    fi

    case "$CLASSIFIER" in
        birdnet-v24)
            CLASSIFIER_NAME="BirdNET V2.4"
            MODEL_FILE="${V24_MODEL_FILE}"
            LABELS_FILE="${V24_LABELS_FILE}"
            ;;
        *)
            CLASSIFIER_NAME="BirdNET+ V3.0"
            MODEL_FILE="${V3_MODEL_FILE}"
            LABELS_FILE="${V3_LABELS_FILE}"
            MODEL_SHA256="${V3_MODEL_SHA256}"
            LABELS_SHA256="${V3_LABELS_SHA256}"
            MODEL_RELEASE_TAG="${V3_MODEL_RELEASE_TAG}"
            GH_BASE="${BIRDNET_MODEL_BASE:-https://github.com/tomtom215/BirdNet-Behavior/releases/download/${MODEL_RELEASE_TAG}}"
            ;;
    esac

    case "$how" in
        explicit)
            log "Classifier: ${CLASSIFIER_NAME} (BIRDNET_CLASSIFIER=${CLASSIFIER}). Set it to birdnet-v24 or birdnet-v3 to change it." ;;
        model-v24)
            log "Classifier: BirdNET V2.4 (BIRDNET_MODEL is the V2.4 file). Set BIRDNET_CLASSIFIER to be explicit." ;;
        model-v3)
            log "Classifier: BirdNET+ V3.0 preview (BIRDNET_MODEL is the V3.0 file). Set BIRDNET_CLASSIFIER to be explicit." ;;
        model-own)
            log "Classifier: your own model (BIRDNET_MODEL=${BIRDNET_MODEL}); labels and the occurrence filter default to the BirdNET+ V3.0 ones, as before. Set BIRDNET_CLASSIFIER=birdnet-v24 to use V2.4's instead." ;;
        volume-v3)
            log "Classifier: BirdNET+ V3.0 preview — this volume already holds its model, so it keeps running it (an upgrade never switches a station's model). New volumes default to BirdNET V2.4; set BIRDNET_CLASSIFIER=birdnet-v24 to move this one, or birdnet-v3 to keep V3.0 and silence this." ;;
        *)
            log "Classifier: BirdNET V2.4 (the default; the network BirdNET-Pi and birdnet-go run). Set BIRDNET_CLASSIFIER=birdnet-v3 on a new volume for the BirdNET+ V3.0 preview." ;;
    esac
}
select_classifier
# Read once, here. The daemon does not read BIRDNET_CLASSIFIER, and the binary
# reports every BIRDNET_* variable it does not read — including in the
# `--install-model` runs below, which would open the first start's log with
# "BIRDNET_CLASSIFIER is not a variable this station reads".
unset BIRDNET_CLASSIFIER

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
MODEL_DEFAULT="${MODEL_DIR}/${MODEL_FILE}"
LABELS_DEFAULT="${MODEL_DIR}/${LABELS_FILE}"
: "${BIRDNET_MODEL:=${MODEL_DEFAULT}}"
: "${BIRDNET_LABELS:=${LABELS_DEFAULT}}"
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
        # "We could not check this" must never return the same value as "this
        # checked out". It used to: a stripped image with no sha256sum turned
        # every verification in this script into a no-op that reported success,
        # so the absence of the checking tool counted as a successful check of
        # the 541 MB model. `installer/test/checksum-refusals.sh` closed the
        # three instances of this shape in the installer; this was the one left
        # in the container, and it is finding LC-15.
        #
        # The runtime image is debian-slim, which ships coreutils, so this is a
        # guard against a future stripped base rather than a live failure.
        warn "sha256sum not available — cannot verify ${file##*/} integrity, and refusing to treat that as verified."
        return 1
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
    # The timing is the ~541 MB classifier's; a 14 MB geomodel was told the
    # same. Say it only for a large (or unknown-size) file.
    if [ "${total}" -eq 0 ] || [ "${total}" -ge 100000000 ]; then
        log "  Typical download: 1–3 min on fibre, 5–15 min on home broadband."
    fi
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
        # Verify what is on the volume, not merely that something is. This
        # branch used to `return 0` on presence alone, three lines below the
        # comment above promising a sha256 check, and `$expected` was never
        # consulted — so any unverified file sitting at this path was adopted as
        # the model for the life of the volume. It does not take a partial
        # download to get one there: `fetch_one` stages at `${dest}.tmp` and
        # only moves on success, but a container killed between that move and
        # the verification below, a volume restored from a truncated snapshot,
        # or a disk that filled during the move all leave exactly this. Nothing
        # downstream catches it — `src/doctor/model.rs` accepts any file over a
        # megabyte — so the station runs a season on a broken model.
        #
        # `installer/lib/55-model.sh` fixed this on bare metal with
        # `model_file_is_verified`; this is the container half of LC-2.
        if verify_sha256 "$dest" "$expected"; then
            log "${desc}: already cached ($(human_bytes "$actual")) and sha256 verified — skipping download."
            return 0
        fi
        warn "${desc}: the cached file ($(human_bytes "$actual")) does not match its expected sha256 — discarding it and downloading again."
        # `${dest}.tmp` is deliberately left alone: it is the resume state for a
        # download still in progress across restarts, which is what makes a
        # 541 MB fetch survivable on a slow link.
        rm -f "$dest"
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
# Fetch and verify the bundled V3.0 classifier and its labels, but only the files
# this script manages: those at the default paths. A file the operator pointed
# BIRDNET_MODEL / BIRDNET_LABELS at (a BirdNET V2.4 ONNX, Perch) is theirs. It
# used to be checked against the V3.0 digest like any other, failed, was deleted
# by `ensure_model_file`, and V3.0 bytes were then downloaded under the name of
# the model the operator had chosen: their model gone, and a different one
# running under its name with labels that do not match it.
acquire_default_models() {
    if [ "${BIRDNET_SKIP_MODEL_DOWNLOAD:-}" = "1" ]; then
        log "BIRDNET_SKIP_MODEL_DOWNLOAD=1 — skipping model download."
        [ -f "${BIRDNET_MODEL}"  ] || warn "Model not found at ${BIRDNET_MODEL}"
        [ -f "${BIRDNET_LABELS}" ] || warn "Labels not found at ${BIRDNET_LABELS}"
        return 0
    fi

    mkdir -p "${MODEL_DIR}"

    # Announce the model source once, up front, so users know what's happening
    # even if the model is already cached and no download is needed.
    log "BirdNET+ V3.0 model directory: ${MODEL_DIR}"
    log "Source: GitHub models release ${MODEL_RELEASE_TAG} (sha256-verified), Zenodo fallback."

    if [ "${BIRDNET_MODEL}" = "${MODEL_DEFAULT}" ]; then
        ensure_model_file \
            "${BIRDNET_MODEL}" \
            "${GH_BASE}/${MODEL_FILE}" \
            "${ZENODO_BASE}/${MODEL_FILE}/content" \
            "${MODEL_SHA256}" \
            "BirdNET+ V3.0 model (ONNX)"
    else
        log "BIRDNET_MODEL=${BIRDNET_MODEL} is not the bundled V3.0 model — using it as supplied (not verified, never replaced)."
        [ -f "${BIRDNET_MODEL}" ] || warn "Model not found at ${BIRDNET_MODEL}"
    fi

    if [ "${BIRDNET_LABELS}" = "${LABELS_DEFAULT}" ]; then
        ensure_model_file \
            "${BIRDNET_LABELS}" \
            "${GH_BASE}/${LABELS_FILE}" \
            "${ZENODO_BASE}/${LABELS_FILE}/content" \
            "${LABELS_SHA256}" \
            "species labels CSV"
    else
        log "BIRDNET_LABELS=${BIRDNET_LABELS} is not the bundled V3.0 labels — using it as supplied (not verified, never replaced)."
        [ -f "${BIRDNET_LABELS}" ] || warn "Labels not found at ${BIRDNET_LABELS}"
    fi

    log "Model ready."
}

# Run `birdnet-behavior --install-model ID` into ${MODEL_DIR}. The binary checks
# every file's sha256 and size and renames it into place only when it matches;
# its output goes to the container log as it comes.
install_with_binary() {
    BIRDNET_MODEL_DIR="${MODEL_DIR}" "${BNB_BINARY}" --config /dev/null --install-model "$1"
}

# BirdNET V2.4's model and labels, installed by the binary — the counterpart of
# acquire_default_models, which downloads the V3.0 pair itself.
#
# Only the files at the default paths are this script's, as above; a file the
# operator pointed BIRDNET_MODEL / BIRDNET_LABELS at is theirs and is neither
# checked nor replaced. A managed file that is present and non-empty is trusted:
# the binary's install is atomic, so a file under its final name was verified
# when it was written (the V3.0 code re-hashes because it writes with a plain
# download that can leave a truncated file at the final name). A missing one
# is installed. Damage done after that — a bad card, a truncated restore — is
# not found here; `docker exec <container> birdnet-behavior --install-model
# birdnet-v24` replaces the files.
acquire_v24_models() {
    if [ "${BIRDNET_SKIP_MODEL_DOWNLOAD:-}" = "1" ]; then
        log "BIRDNET_SKIP_MODEL_DOWNLOAD=1 — skipping model download."
        [ -f "${BIRDNET_MODEL}"  ] || warn "Model not found at ${BIRDNET_MODEL}"
        [ -f "${BIRDNET_LABELS}" ] || warn "Labels not found at ${BIRDNET_LABELS}"
        return 0
    fi

    mkdir -p "${MODEL_DIR}"
    log "BirdNET V2.4 model directory: ${MODEL_DIR}"

    need=0
    if [ "${BIRDNET_MODEL}" = "${MODEL_DEFAULT}" ]; then
        [ -s "${BIRDNET_MODEL}" ] || need=1
    else
        log "BIRDNET_MODEL=${BIRDNET_MODEL} is not the BirdNET V2.4 model this image manages — using it as supplied (not verified, never replaced)."
        [ -f "${BIRDNET_MODEL}" ] || warn "Model not found at ${BIRDNET_MODEL}"
    fi
    if [ "${BIRDNET_LABELS}" = "${LABELS_DEFAULT}" ]; then
        [ -s "${BIRDNET_LABELS}" ] || need=1
    else
        log "BIRDNET_LABELS=${BIRDNET_LABELS} is not the BirdNET V2.4 labels this image manages — using it as supplied (not verified, never replaced)."
        [ -f "${BIRDNET_LABELS}" ] || warn "Labels not found at ${BIRDNET_LABELS}"
    fi

    if [ "$need" = 1 ]; then
        rule
        log "Installing BirdNET V2.4 (model ~54 MB + labels) with birdnet-behavior --install-model."
        log "  Every file is sha256-verified before it is kept. This runs only on first start;"
        log "  the files are cached in the Docker volume, so later starts are instant."
        rule
        if ! install_with_binary "${V24_MODEL_ID}"; then
            warn "Common causes:"
            warn "  • no internet in the container (check the host's DNS/firewall)"
            warn "  • huggingface.co temporarily unreachable (retry shortly)"
            warn "  • the volume has less than ~600 MB free (df -h on the host's docker root)"
            die "Failed to install BirdNET V2.4. To run the BirdNET+ V3.0 preview instead, set BIRDNET_CLASSIFIER=birdnet-v3."
        fi
        if [ "${BIRDNET_MODEL}" = "${MODEL_DEFAULT}" ] && [ ! -s "${BIRDNET_MODEL}" ]; then
            die "birdnet-behavior --install-model reported success but ${BIRDNET_MODEL} is missing; this image's entrypoint and binary disagree about the file names."
        fi
        if [ "${BIRDNET_LABELS}" = "${LABELS_DEFAULT}" ] && [ ! -s "${BIRDNET_LABELS}" ]; then
            die "birdnet-behavior --install-model reported success but ${BIRDNET_LABELS} is missing; this image's entrypoint and binary disagree about the file names."
        fi
    else
        log "BirdNET V2.4 model and labels already installed — skipping."
    fi
    log "Model ready."
}

# What to run at start: the binary's installer for V2.4, the download code above
# for V3.0 (and for a model of the operator's own, as before).
acquire_classifier_models() {
    if [ "${CLASSIFIER}" = "birdnet-v24" ]; then
        acquire_v24_models
    else
        acquire_default_models
    fi
}
acquire_classifier_models

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

    # Verify what is on the volume, not merely that something is — the
    # classifier's cache branch above had the same presence-only check and the
    # same reasons a bad file can sit here (a kill between the move and the
    # verification, a truncated snapshot, a full disk).
    if [ -f "$dest" ]; then
        if verify_sha256 "$dest" "$expected"; then
            log "${desc}: already cached and sha256 verified — skipping download."
            return 0
        fi
        warn "${desc}: the cached file does not match its expected sha256 — discarding it and downloading again."
        rm -f "$dest"
    fi

    for src in mirror upstream; do
        if [ "$src" = "mirror" ]; then
            url="${GH_BASE}/${filename}"
            origin="GitHub release ${MODEL_RELEASE_TAG}"
        else
            url="${GEOMODEL_UPSTREAM_BASE}/${filename}"
            origin="upstream birdnet-team/geomodel ${GEOMODEL_VERSION}"
        fi

        # The models release may not carry the geomodel yet (RELEASING.md), and
        # falling through to upstream is then the expected path. Ask first, so
        # that answer is one line rather than a download banner, curl's 404,
        # and two warnings. Only a definite 404 is taken as "not there": any
        # other answer goes through fetch_one and is reported if it fails.
        if [ "$src" = "mirror" ] && [ "$(remote_status "$url")" = "404" ]; then
            log "${desc}: not published in ${origin} — fetching it from upstream."
            continue
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

# Decide the geomodel: leave an operator's alone, or fetch ours and point the
# daemon at it. A function so installer/test/container-model-cache.sh can
# drive it.
setup_geomodel() {
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
            # Only a default: an operator who set the labels alone (a translated
            # or edited label file for the geomodel we fetch) keeps theirs. This
            # used to overwrite it; the station checks the pair at startup.
            BIRDNET_METADATA_LABELS="${BIRDNET_METADATA_LABELS:-${geolabels_path}}"
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
}

# The V2.4 range filter, installed by the binary and pointed at exactly as the
# geomodel is, but one file with no labels of its own. Non-fatal for the same
# reason. A function so installer/test/classifier-docker.sh can drive it.
setup_range_filter() {
    if [ "${GEOMODEL_USER_SET}" = "1" ]; then
        log "BIRDNET_METADATA_MODEL is set explicitly — leaving the occurrence filter alone."
    elif [ "${BIRDNET_SKIP_MODEL_DOWNLOAD:-}" = "1" ]; then
        log "BIRDNET_SKIP_MODEL_DOWNLOAD=1 — skipping the range filter install too."
    else
        mkdir -p "${MODEL_DIR}"
        range_path="${MODEL_DIR}/${V24_RANGE_FILE}"

        if [ -s "${range_path}" ]; then
            log "BirdNET V2.4 range filter: already installed — skipping."
        else
            log "Installing the BirdNET V2.4 range filter v2 (~30 MB) with birdnet-behavior --install-model."
            if ! install_with_binary "${V24_RANGE_ID}"; then
                rm -f "${range_path}"
            fi
        fi

        if [ -s "${range_path}" ]; then
            BIRDNET_METADATA_MODEL="${range_path}"
            export BIRDNET_METADATA_MODEL
            if [ -n "${BIRDNET_METADATA_LABELS:-}" ]; then
                warn "BIRDNET_METADATA_LABELS is set, but the V2.4 range filter is indexed like the V2.4 labels and needs none. Unset it unless you mean to override the label file."
            fi
            log "Species occurrence filtering: ON (BirdNET V2.4 range filter v2; set BIRDNET_SF_THRESH to tune; default 0.03)."
        else
            warn "Range filter unavailable — species occurrence filtering is OFF."
            warn "Every species the classifier knows stays a candidate wherever this"
            warn "station is. Restart the container to retry, then check with:"
            warn "  docker exec <container> birdnet-behavior --doctor"
        fi
    fi
}

# The filter that belongs to the classifier, and only that one: the V2.4 range
# filter for V2.4, the geomodel for everything else.
setup_occurrence_filter() {
    if [ "${CLASSIFIER}" = "birdnet-v24" ]; then
        setup_range_filter
    else
        setup_geomodel
    fi
}
setup_occurrence_filter

# ---------------------------------------------------------------------------
# Container defaults
# ---------------------------------------------------------------------------
# The binary already defaults to 0.0.0.0:8502; this pins that default here so
# the container binds on all interfaces whatever the binary's default becomes,
# unless the user has set BIRDNET_LISTEN explicitly.
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
# (BIRDNET_CLASSIFIER was removed from the environment right after it was read,
# so neither the installs above nor this daemon report it as an unknown variable.)
exec "${BNB_BINARY}" "$@"
