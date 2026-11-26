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
#   * the binary is a stub that does what `birdnet-behavior --install-model`
#     does — writes the files under the names the real one gives them, adopts a
#     file under an earlier installer's name when it is the right one, refuses a
#     local file that is not — and writes each id it was asked to install, with
#     the retry schedule it was handed, to ${sb}/requests.log. Every model is
#     installed by the binary (MDL-09); the real one is exercised separately
#     (its own tests, and a real install); what these tests hold is what the
#     installer does with it;
#   * the network is a function that refuses and logs anything the installer
#     tries to fetch itself (it should fetch no model);
#   * every path points into the sandbox.
# Everything else — argument parsing, classifier resolution, write_config, the
# kept-config rules, the move off the old V3.0 names, the summary — is the code
# that ships.

FLOW_REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"

# The file names are the contract: stations in the field have them in their
# configs, and `birdnet-behavior --install-model` writes them.
FLOW_V24_MODEL="birdnet-v24.onnx"
FLOW_V24_LABELS="birdnet-v24_labels.txt"
FLOW_V24_RANGE="birdnet-v24-range-v2.onnx"
FLOW_V3_MODEL="birdnet-v3.onnx"
FLOW_V3_LABELS="birdnet-v3_labels.csv"
FLOW_GEO_MODEL="birdnet-v3-geomodel.onnx"
FLOW_GEO_LABELS="birdnet-v3-geomodel_labels.txt"
# The names the installer gave the V3.0 files before the binary installed them.
FLOW_V3_LEGACY_MODEL="BirdNET+_V3.0-preview3_Global_11K_FP32.onnx"
FLOW_V3_LEGACY_LABELS="BirdNET+_V3.0-preview3_Global_11K_Labels.csv"
FLOW_GEO_LEGACY_MODEL="BirdNET+_Geomodel_V3.0.2_Global_12K_FP32.onnx"
FLOW_GEO_LEGACY_LABELS="BirdNET+_Geomodel_V3.0.2_Global_12K_Labels.txt"

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
# v24-model v24-labels v24-range v3-model v3-labels geo-model geo-labels, or
# legacy-v3-model legacy-v3-labels legacy-geo-model legacy-geo-labels (the
# same bodies under the names an earlier installer gave them).
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
        legacy-v3-model)   printf '%s' "${FLOW_BODY_V3_MODEL}"   >"${dir}/${FLOW_V3_LEGACY_MODEL}" ;;
        legacy-v3-labels)  printf '%s' "${FLOW_BODY_V3_LABELS}"  >"${dir}/${FLOW_V3_LEGACY_LABELS}" ;;
        legacy-geo-model)  printf '%s' "${FLOW_BODY_GEO_MODEL}"  >"${dir}/${FLOW_GEO_LEGACY_MODEL}" ;;
        legacy-geo-labels) printf '%s' "${FLOW_BODY_GEO_LABELS}" >"${dir}/${FLOW_GEO_LEGACY_LABELS}" ;;
        *) echo "flow_seed: unknown file '${what}'" >&2; return 1 ;;
    esac
}

# A stand-in for the binary: `--version`, and `--install-model ID`, which does
# what the real one does with the files — writes them under the names the real
# one gives them (fixed bodies), adopts a file an earlier installer left under
# its old name when it has the right body (a hard link: the same file), and
# refuses a `--from-file PATH` whose body is not the pinned one — and logs to
# $FLOW_REQUESTS each id with the directory, the local file and the retry
# schedule it was handed, each file it had to write ("WROTE name": what the
# real one would have downloaded) and each it adopted ("ADOPTED path"). Environment switches, set by the caller of flow_run:
#   FLOW_FAIL_MODEL=1     birdnet-v24 fails as if every origin were down
#   FLOW_FAIL_RANGE=1     birdnet-v24-range-v2 fails likewise
#   FLOW_FAIL_V3_MODEL=1  birdnet-v3 fails likewise (when not adopted)
#   FLOW_FAIL_GEO=1       birdnet-v3-geomodel fails likewise (when not adopted)
#   FLOW_OLD_BINARY=1     every id is "unknown", as in a binary from before V2.4
flow_write_stub_binary() { # $1 = path
    cat >"$1" <<'STUB'
#!/bin/bash
id=""
from=""
next=""
for a in "$@"; do
    case "${next}" in
        id) id="${a}"; next=""; continue ;;
        from) from="${a}"; next=""; continue ;;
    esac
    case "${a}" in
        --version) echo "birdnet-behavior 0.0.1"; exit 0 ;;
        --install-model) next=id ;;
        --from-file) next=from ;;
    esac
done
[ -n "${id}" ] || exit 0
dir="${BIRDNET_MODEL_DIR:-}"
if [ -n "${FLOW_REQUESTS:-}" ]; then
    if [ -n "${from}" ]; then
        echo "INSTALL-MODEL ${id} dir=${dir} from=${from}" >>"${FLOW_REQUESTS}"
    else
        echo "INSTALL-MODEL ${id} dir=${dir}" >>"${FLOW_REQUESTS}"
    fi
    echo "RETRY-WAITS ${id} ${BIRDNET_MODEL_RETRY_WAITS-<unset>}" >>"${FLOW_REQUESTS}"
fi
# The real binary reports every BIRDNET_* variable it does not read; the
# installer's own settings must not reach it.
for v in BIRDNET_CLASSIFIER BIRDNET_SKIP_MODEL BIRDNET_NONINTERACTIVE BIRDNET_LOCAL_MODEL; do
    eval "[ -n \"\${${v}+set}\" ]" && echo "CHILD-SAW ${v}" >>"${FLOW_REQUESTS}"
done
if [ "${FLOW_OLD_BINARY:-0}" = 1 ]; then
    echo "Install failed: no model called ${id}" >&2
    exit 1
fi
legacy=""; legacy_labels=""; labels=""; lbody=""; fail=0
case "${id}" in
    birdnet-v24)
        model="birdnet-v24.onnx"; body="fake BirdNET V2.4 classifier"
        labels="birdnet-v24_labels.txt"; lbody="fake BirdNET V2.4 labels"; fail="${FLOW_FAIL_MODEL:-0}" ;;
    birdnet-v24-range-v2)
        model="birdnet-v24-range-v2.onnx"; body="fake BirdNET V2.4 range filter v2"; fail="${FLOW_FAIL_RANGE:-0}" ;;
    birdnet-v3)
        model="birdnet-v3.onnx"; body="fake BirdNET+ V3.0 classifier"
        labels="birdnet-v3_labels.csv"; lbody="fake BirdNET+ V3.0 labels"
        legacy="BirdNET+_V3.0-preview3_Global_11K_FP32.onnx"; legacy_labels="BirdNET+_V3.0-preview3_Global_11K_Labels.csv"
        fail="${FLOW_FAIL_V3_MODEL:-0}" ;;
    birdnet-v3-geomodel)
        model="birdnet-v3-geomodel.onnx"; body="fake BirdNET geomodel"
        labels="birdnet-v3-geomodel_labels.txt"; lbody="fake BirdNET geomodel labels"
        legacy="BirdNET+_Geomodel_V3.0.2_Global_12K_FP32.onnx"; legacy_labels="BirdNET+_Geomodel_V3.0.2_Global_12K_Labels.txt"
        fail="${FLOW_FAIL_GEO:-0}" ;;
    *)
        echo "Install failed: no model called ${id}" >&2
        exit 1 ;;
esac
# place NAME BODY LEGACY: already right; adopted from the old name; or fetched.
place() {
    if [ "$(cat "${dir}/$1" 2>/dev/null)" = "$2" ]; then return 0; fi
    if [ -n "$3" ] && [ "$(cat "${dir}/$3" 2>/dev/null)" = "$2" ]; then
        rm -f "${dir}/$1"; ln "${dir}/$3" "${dir}/$1"
        echo "ADOPTED ${dir}/$3" >>"${FLOW_REQUESTS}"
        return 0
    fi
    [ "${fail}" = 1 ] && { echo "Install failed: no origin served the verified file" >&2; return 1; }
    echo "WROTE $1" >>"${FLOW_REQUESTS}"
    printf '%s' "$2" >"${dir}/$1"
}
if [ -n "${from}" ]; then
    if [ "$(cat "${from}" 2>/dev/null)" != "${body}" ]; then
        echo "Install failed: sha256 mismatch: ${from} is not the pinned model" >&2
        exit 1
    fi
    echo "WROTE ${model} from=${from}" >>"${FLOW_REQUESTS}"
    printf '%s' "${body}" >"${dir}/${model}"
else
    place "${model}" "${body}" "${legacy}" || exit 1
fi
if [ -n "${labels}" ]; then
    place "${labels}" "${lbody}" "${legacy_labels}" || exit 1
fi
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

        # ── the network: the installer must fetch no model itself ──────────
        flow_refuse() { echo "GET ${1}" >>"${sb}/requests.log"; return 1; }
        sleep() { echo "SLEEP $*" >>"${sb}/requests.log"; }
        download_large() { flow_refuse "$1"; }
        download()       { flow_refuse "$1"; }

        main "$@"
    ) >"${sb}/out.log" 2>&1
}

# URLs requested in sandbox $1, one per line.
flow_requests() { cat "$1/requests.log"; }

# Assert helpers shared by the callers. They use the caller's pass/fail.
flow_requested() { grep -qF -- "$2" "$1/requests.log"; }
