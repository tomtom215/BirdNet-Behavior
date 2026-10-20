# shellcheck shell=bash
# installer/test/lib/install-flow.sh — run the REAL installer's `main`, with
# the world stubbed out, and look at what it left behind.
#
# Sourced by classifier-default.sh, classifier-kept.sh and
# classifier-never-mixed.sh; not a test itself (run-ci.sh runs only the files
# directly in installer/test/).
#
# Most installer tests in this directory source one function out of a module and
# drive it. That is right for a function with one job, and it cannot say
# whether the wiring is right: whether `do_install` picks the classifier
# before it sizes the download, whether `--classifier` survives `parse_args` to
# `write_config`, whether a re-run reaches the code that leaves a config alone.
# This runs the assembled script (`installer/build.sh --stdout`, so it cannot
# drift from the modules, and needs no committed install.sh) up to but not
# including its final `main "$@"`, and then calls `main` itself.
#
# What is replaced, and why it is safe to:
#   * everything that touches the machine — packages, systemd, the unit file,
#     zram, the binary swap, ownership — is a no-op, so a mistake here cannot
#     reach the host (has_systemd is false, systemctl is a function that fails);
#   * the network is a function that serves a fixed body per file name and
#     writes every URL it was asked for to ${sb}/requests.log (this is only the
#     BirdNET+ V3.0 path: BirdNET V2.4 is installed by the binary, not fetched
#     by the installer);
#   * the binary is a stub that does what `birdnet-behavior --install-model`
#     does — writes the files under the names the real one gives them — and
#     writes each id it was asked to install to the same log. The real binary is
#     exercised separately (it needs the network); what these tests hold is what
#     the installer does with it;
#   * every path points into the sandbox.
# Everything else — argument parsing, classifier resolution, the download and
# verification code, write_config, the kept-config rules, the summary — is the
# code that ships.
#
# The V3.0 pins are changed in place: the real sha256s are of real 540 MB
# files, so each is replaced with the digest of a short fixed body. That is a
# change to data the code reads, not to the code; the real pins are held
# against the Rust catalogue by model-pins-match-catalogue.sh. (BirdNET V2.4
# has no pins in the installer to replace.)
#
# The old and new names are both set, so the same harness drives a tree from
# before the classifier choice (where MODEL_SHA256 is a top-level pin) and one
# after it (where select_classifier copies V3_* into it). That is what lets a
# new test be shown failing against the old code for the reason it exists, not
# because the harness could not start.

FLOW_REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"

# The file names are the contract: stations in the field have them in their
# configs, and `birdnet-behavior --install-model` writes the V2.4 ones.
FLOW_V24_MODEL="birdnet-v24.onnx"
FLOW_V24_LABELS="birdnet-v24_labels.txt"
FLOW_V24_RANGE="birdnet-v24-range-v2.onnx"
FLOW_V3_MODEL="BirdNET+_V3.0-preview3_Global_11K_FP32.onnx"
FLOW_V3_LABELS="BirdNET+_V3.0-preview3_Global_11K_Labels.csv"
FLOW_GEO_MODEL="BirdNET+_Geomodel_V3.0.2_Global_12K_FP32.onnx"
FLOW_GEO_LABELS="BirdNET+_Geomodel_V3.0.2_Global_12K_Labels.txt"

# Fixed bodies. Distinct, so a file in the wrong place is visible by content.
FLOW_BODY_V24_MODEL="fake BirdNET V2.4 classifier"
FLOW_BODY_V24_LABELS="fake BirdNET V2.4 labels"
FLOW_BODY_V24_RANGE="fake BirdNET V2.4 range filter v2"
FLOW_BODY_V3_MODEL="fake BirdNET+ V3.0 classifier"
FLOW_BODY_V3_LABELS="fake BirdNET+ V3.0 labels"
FLOW_BODY_GEO_MODEL="fake BirdNET geomodel"
FLOW_BODY_GEO_LABELS="fake BirdNET geomodel labels"

flow_sha() { printf '%s' "$1" | sha256sum | awk '{print $1}'; }

# The assembled installer without its final `main "$@"`. Refuses, loudly, if
# that line is not the last one: sourcing the script whole would run it.
flow_script() {
    local whole
    whole="$("${FLOW_REPO_ROOT}/installer/build.sh" --stdout)" || return 1
    if [ "$(tail -n 1 <<<"${whole}")" != 'main "$@"' ]; then
        echo 'echo "install-flow.sh: the assembled installer does not end in main \"\$@\"" >&2; exit 97'
        return 0
    fi
    sed '$d' <<<"${whole}"
}

# A fresh sandbox; prints its path.
flow_new() {
    local sb
    sb="$(mktemp -d)"
    mkdir -p "${sb}/home/BirdNet-Behavior/models" "${sb}/etc/birdnet" "${sb}/bin"
    : >"${sb}/requests.log"
    printf '%s' "${sb}"
}

flow_models_dir() { printf '%s/home/BirdNet-Behavior/models' "$1"; }

# How many files in sandbox $1's models directory have a name matching the
# extended regex $2. (A glob loop rather than `ls | grep`, which breaks on odd
# file names; these have a `+` in them.)
flow_models_count() {
    local f n=0
    for f in "$(flow_models_dir "$1")"/*; do
        [ -e "${f}" ] || continue
        [[ "${f##*/}" =~ $2 ]] && n=$((n + 1))
    done
    echo "${n}"
}
flow_models_match() { [ "$(flow_models_count "$1" "$2")" -gt 0 ]; }
flow_config()     { printf '%s/etc/birdnet/birdnet.conf' "$1"; }

# Put a verified fake file in the models directory. $2 is one of
# v24-model v24-labels v24-range v3-model v3-labels geo-model geo-labels.
flow_seed() {
    local sb="$1" what="$2" dir
    dir="$(flow_models_dir "${sb}")"
    case "${what}" in
        v24-model)  printf '%s' "${FLOW_BODY_V24_MODEL}"  >"${dir}/${FLOW_V24_MODEL}" ;;
        v24-labels) printf '%s' "${FLOW_BODY_V24_LABELS}" >"${dir}/${FLOW_V24_LABELS}" ;;
        v24-range)  printf '%s' "${FLOW_BODY_V24_RANGE}"  >"${dir}/${FLOW_V24_RANGE}" ;;
        v3-model)   printf '%s' "${FLOW_BODY_V3_MODEL}"   >"${dir}/${FLOW_V3_MODEL}" ;;
        v3-labels)  printf '%s' "${FLOW_BODY_V3_LABELS}"  >"${dir}/${FLOW_V3_LABELS}" ;;
        geo-model)  printf '%s' "${FLOW_BODY_GEO_MODEL}"  >"${dir}/${FLOW_GEO_MODEL}" ;;
        geo-labels) printf '%s' "${FLOW_BODY_GEO_LABELS}" >"${dir}/${FLOW_GEO_LABELS}" ;;
        *) echo "flow_seed: unknown file '${what}'" >&2; return 1 ;;
    esac
}

# A stand-in for the binary: `--version`, and `--install-model ID`, which writes
# the files the real one writes (the same names; fixed bodies) into
# $BIRDNET_MODEL_DIR and logs the id to $FLOW_REQUESTS. Environment switches, set
# by the caller of flow_run:
#   FLOW_FAIL_MODEL=1   birdnet-v24 fails as if the network were down
#   FLOW_FAIL_RANGE=1   birdnet-v24-range-v2 fails likewise
#   FLOW_OLD_BINARY=1   every id is "unknown", as in a binary from before V2.4
flow_write_stub_binary() { # $1 = path
    cat >"$1" <<'STUB'
#!/bin/sh
id=""
for a in "$@"; do
    if [ "${id}" = "?" ]; then id="${a}"; fi
    case "${a}" in
        --version) echo "birdnet-behavior 0.0.1"; exit 0 ;;
        --install-model) id="?" ;;
    esac
done
[ -n "${id}" ] && [ "${id}" != "?" ] || exit 0
dir="${BIRDNET_MODEL_DIR:-}"
[ -n "${FLOW_REQUESTS:-}" ] && echo "INSTALL-MODEL ${id} dir=${dir}" >>"${FLOW_REQUESTS}"
# The real binary reports every BIRDNET_* variable it does not read; the
# installer's own settings must not reach it.
for v in BIRDNET_CLASSIFIER BIRDNET_SKIP_MODEL BIRDNET_NONINTERACTIVE; do
    eval "[ -n \"\${${v}:-}\" ]" && echo "CHILD-SAW ${v}" >>"${FLOW_REQUESTS}"
done
if [ "${FLOW_OLD_BINARY:-0}" = 1 ]; then
    echo "Install failed: no classifier called ${id}" >&2
    exit 1
fi
case "${id}" in
    birdnet-v24)
        if [ "${FLOW_FAIL_MODEL:-0}" = 1 ]; then echo "Install failed: network error" >&2; exit 1; fi
        printf '%s' "fake BirdNET V2.4 classifier" >"${dir}/birdnet-v24.onnx"
        printf '%s' "fake BirdNET V2.4 labels" >"${dir}/birdnet-v24_labels.txt"
        ;;
    birdnet-v3)
        # In the real catalogue too, under these names (the Linux installer does
        # not use it: it downloads the V3.0 files itself).
        printf '%s' "fake BirdNET+ V3.0 classifier" >"${dir}/birdnet-v3.onnx"
        printf '%s' "fake BirdNET+ V3.0 labels" >"${dir}/birdnet-v3_labels.csv"
        ;;
    birdnet-v24-range-v2)
        if [ "${FLOW_FAIL_RANGE:-0}" = 1 ]; then echo "Install failed: network error" >&2; exit 1; fi
        printf '%s' "fake BirdNET V2.4 range filter v2" >"${dir}/birdnet-v24-range-v2.onnx"
        ;;
    *)
        echo "Install failed: no classifier called ${id}" >&2
        exit 1
        ;;
esac
echo "Installed and verified."
STUB
    chmod +x "$1"
}

# Mark the sandbox as an installed station: a binary the installer will find.
flow_seed_binary() {
    flow_write_stub_binary "$1/bin/birdnet-behavior"
}

# The last active value of KEY in the sandbox's config ("" when there is none).
flow_conf() {
    local line
    line="$(grep -E "^[[:space:]]*$2[[:space:]]*=" "$(flow_config "$1")" 2>/dev/null | tail -n 1 || true)"
    printf '%s' "${line#*=}"
}

# Run `main` with the given arguments in sandbox $1. The caller sets any
# environment (BIRDNET_CLASSIFIER=… flow_run …). Output: ${sb}/out.log. Exit
# status: main's.
#
flow_run() {
    local sb="$1"
    shift
    (
        export HOME="${sb}/home"
        export BIRDNET_NONINTERACTIVE=1
        export FLOW_REQUESTS="${sb}/requests.log"
        unset SUDO_USER
        # shellcheck disable=SC1090
        source <(flow_script)

        # ── paths: all inside the sandbox ──────────────────────────────────
        # shellcheck disable=SC2034  # read by the sourced installer
        {
            CONFIG_DIR="${sb}/etc/birdnet"
            CONFIG_FILE="${CONFIG_DIR}/birdnet.conf"
            INSTALL_DIR="${sb}/bin"
            DATA_DIR="${sb}/home/BirdNet-Behavior"
            RECS_DIR="${DATA_DIR}/recordings"
            IMAGE_CACHE_DIR="${DATA_DIR}/image_cache"
            MODEL_DIR="${DATA_DIR}/models"
            DB_PATH="${DATA_DIR}/birds.db"
            STREAM_DIR="${sb}/stream"
            SERVICE_FILE="${sb}/birdnet-behavior.service"
            JOURNALD_DROPIN="${sb}/journald.conf"
            HELP_DIR="${sb}/help"
            SERVICE_USER="$(id -un)"
        }

        # ── pins: the digest of each fixed body, under both generations of
        #    names (see the header) ─────────────────────────────────────────
        # shellcheck disable=SC2034
        {
            V3_MODEL_SHA256="$(flow_sha "${FLOW_BODY_V3_MODEL}")"
            V3_LABELS_SHA256="$(flow_sha "${FLOW_BODY_V3_LABELS}")"
            MODEL_SHA256="${V3_MODEL_SHA256}"
            LABELS_SHA256="${V3_LABELS_SHA256}"
            GEOMODEL_SHA256="$(flow_sha "${FLOW_BODY_GEO_MODEL}")"
            GEOMODEL_LABELS_SHA256="$(flow_sha "${FLOW_BODY_GEO_LABELS}")"
        }

        # ── the machine: nothing here may reach it ─────────────────────────
        has_systemd() { return 1; }
        systemctl()   { return 1; }
        apt-get()     { return 1; }
        modprobe()    { return 1; }
        chown()       { :; }
        require_root()           { :; }
        check_glibc()            { :; }
        check_required_tools()   { :; }
        detect_arch()            { echo x86_64-unknown-linux-gnu; }
        resolve_version()        { echo 0.0.1; }
        install_binary()         { flow_write_stub_binary "${INSTALL_DIR}/${BINARY_NAME}"; }
        setup_tmpfs_streaming()  { :; }
        prompt_station_settings(){ :; }
        resolve_listen_addr()    { :; }
        ensure_admin_password()  { :; }
        ensure_capture_backend() { :; }
        install_service()        { :; }
        maybe_setup_zram()       { :; }
        maybe_start_service()    { :; }
        validate_install()       { :; }
        create_directories() {
            mkdir -p "${DATA_DIR}" "${RECS_DIR}" "${IMAGE_CACHE_DIR}" "${MODEL_DIR}" "${CONFIG_DIR}"
        }
        # `install -d -o USER -g USER` needs that user and group to exist; the
        # ownership is not what these tests are about.
        install() {
            local args=()
            while [ "$#" -gt 0 ]; do
                case "$1" in
                    -o | -g) shift 2 ;;
                    *) args+=("$1"); shift ;;
                esac
            done
            command install "${args[@]}"
        }

        # ── the network ────────────────────────────────────────────────────
        flow_serve() { # URL DEST
            local url="$1" dest="$2" name
            echo "GET ${url}" >>"${sb}/requests.log"
            name="${url%%\?*}"
            name="${name%/content}"
            name="${name##*/}"
            name="${name//%2B/+}"
            case "${name}" in
                "${FLOW_V3_MODEL}")    printf '%s' "${FLOW_BODY_V3_MODEL}" >"${dest}" ;;
                "${FLOW_V3_LABELS}")   printf '%s' "${FLOW_BODY_V3_LABELS}" >"${dest}" ;;
                "${FLOW_GEO_MODEL}")   printf '%s' "${FLOW_BODY_GEO_MODEL}" >"${dest}" ;;
                "${FLOW_GEO_LABELS}")  printf '%s' "${FLOW_BODY_GEO_LABELS}" >"${dest}" ;;
                *)
                    echo "UNKNOWN-URL ${url}" >>"${sb}/requests.log"
                    return 1
                    ;;
            esac
        }
        download_large() { flow_serve "$1" "$2"; }
        download()       { flow_serve "$1" "$2"; }
        # The plain downloader's contract: 4 means "the origin has no such file".
        # Nothing here is ever absent; the V3.0 installer is unchanged.
        download_or_absent() { flow_serve "$1" "$2"; }

        main "$@"
    ) >"${sb}/out.log" 2>&1
}

# URLs requested in sandbox $1, one per line.
flow_requests() { cat "$1/requests.log"; }

# Assert helpers shared by the callers. They use the caller's pass/fail.
flow_requested() { grep -qF -- "$2" "$1/requests.log"; }
