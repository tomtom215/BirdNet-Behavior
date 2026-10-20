#!/usr/bin/env bash
# installer/test/classifier-never-mixed.sh — is a classifier ever paired with the
# other network's occurrence filter?
#
# model_catalog.rs says why not, in the notes on the V2.4 range filter: it is
# indexed like the V2.4 labels (by position), so it is meaningful only to
# BirdNET V2.4; the V3.0 classifier has its own geomodel, whose 12 012 outputs
# are matched to its 11 560 by scientific name through a label file of its own.
# Hand either one to the other network and the station reports a species list
# that is plausible and wrong — no error, no log line, just birds that were
# filtered by the wrong map.
#
# The pairing is decided in four places (the download dispatcher, the fresh
# config, and the two kept-config retrofits), so this holds the invariant at
# each of them, and then over whole runs:
#
#   P1  every config the installer writes or keeps, in every scenario below,
#       pairs V2.4 only with the range filter (and no labels file) and V3.0
#       only with the geomodel (and its labels);
#   P2  the dispatcher fetches one family and never the other;
#   P3  write_config, handed a contradictory state (the OTHER family's flag
#       set), still does not mix;
#   P4  each kept-config retrofit refuses a config of the other family.
#
# Usage: installer/test/classifier-never-mixed.sh
# Needs bash + coreutils. No root, no network.

set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=installer/test/lib/install-flow.sh
source "${HERE}/lib/install-flow.sh"

FAILED=0
pass() { printf '  PASS  %s\n' "$*"; }
fail() { printf '  FAIL  %s\n' "$*"; FAILED=1; }
SANDBOXES=()
trap 'rm -rf "${SANDBOXES[@]}"' EXIT
new_sandbox() { SB="$(flow_new)"; SANDBOXES+=("${SB}"); }

FIXTURE="${HERE}/data/config-v3-written-by-the-previous-installer.conf"

# assert_paired CONFIG LABEL — the one rule, read off a config file.
assert_paired() {
    local cfg="$1" label="$2" model filter labels
    model="$(grep -E '^MODEL_PATH=' "${cfg}" | tail -n 1 | cut -d= -f2-)"
    filter="$(grep -E '^METADATA_MODEL_PATH=' "${cfg}" | tail -n 1 | cut -d= -f2- || true)"
    labels="$(grep -E '^METADATA_LABELS_PATH=' "${cfg}" | tail -n 1 | cut -d= -f2- || true)"
    case "${model##*/}" in
        "${FLOW_V24_MODEL}")
            if [ -n "${filter}" ] && [ "${filter##*/}" != "${FLOW_V24_RANGE}" ]; then
                fail "${label}: a V2.4 config names '${filter##*/}' as its occurrence filter, not the v2 range filter"
            elif [ -n "${labels}" ]; then
                fail "${label}: a V2.4 config names a METADATA_LABELS_PATH (${labels##*/}); the range filter has none"
            else
                pass "${label}: V2.4 is paired with $([ -n "${filter}" ] && echo 'the range filter' || echo 'no filter (none installed)')"
            fi
            ;;
        "${FLOW_V3_MODEL}")
            if [ -n "${filter}" ] && [ "${filter##*/}" != "${FLOW_GEO_MODEL}" ]; then
                fail "${label}: a V3.0 config names '${filter##*/}' as its occurrence filter, not the geomodel"
            elif [ -n "${filter}" ] && [ "${labels##*/}" != "${FLOW_GEO_LABELS}" ]; then
                fail "${label}: a V3.0 config names the geomodel without its labels ('${labels##*/}')"
            else
                pass "${label}: V3.0 is paired with $([ -n "${filter}" ] && echo 'the geomodel' || echo 'no filter (none installed)')"
            fi
            ;;
        *)
            fail "${label}: MODEL_PATH is '${model}', neither classifier this test knows"
            ;;
    esac
}

# ── P1. whole runs ──────────────────────────────────────────────────────────
echo "=== P1. every config in every scenario pairs a classifier with its own filter ==="
new_sandbox; flow_run "${SB}";                         assert_paired "$(flow_config "${SB}")" "fresh, default"
new_sandbox; flow_run "${SB}" --classifier birdnet-v3; assert_paired "$(flow_config "${SB}")" "fresh, --classifier birdnet-v3"
new_sandbox; flow_run "${SB}" --classifier birdnet-v24; assert_paired "$(flow_config "${SB}")" "fresh, --classifier birdnet-v24"
new_sandbox; FLOW_FAIL_RANGE=1 flow_run "${SB}";       assert_paired "$(flow_config "${SB}")" "fresh V2.4, range filter failed"
new_sandbox
sed "s|@SB@|${SB}|g" "${FIXTURE}" >"$(flow_config "${SB}")"; flow_seed_binary "${SB}"
flow_run "${SB}";                                      assert_paired "$(flow_config "${SB}")" "V3.0 station updated"
flow_run "${SB}" --classifier birdnet-v24;             assert_paired "$(flow_config "${SB}")" "V3.0 station updated with --classifier birdnet-v24"
new_sandbox; flow_run "${SB}"; flow_seed_binary "${SB}"
flow_run "${SB}";                                      assert_paired "$(flow_config "${SB}")" "V2.4 station updated"
flow_run "${SB}" --classifier birdnet-v3;              assert_paired "$(flow_config "${SB}")" "V2.4 station updated with --classifier birdnet-v3"
new_sandbox; FLOW_FAIL_RANGE=1 flow_run "${SB}" >/dev/null 2>&1; flow_seed_binary "${SB}"
flow_run "${SB}" repair;                               assert_paired "$(flow_config "${SB}")" "V2.4 station, range filter retried by repair"

# What was fetched, as well as what was written: the files of the other family
# must not even be on the disk of a station that cannot use them.
new_sandbox; flow_run "${SB}"
if flow_models_match "${SB}" 'V3|Geomodel'; then fail "a V2.4 station has V3.0/geomodel files on disk"; else pass "a V2.4 station has no V3.0/geomodel files on disk"; fi
new_sandbox; flow_run "${SB}" --classifier birdnet-v3
if flow_models_match "${SB}" 'birdnet-v24'; then fail "a V3.0 station has V2.4 files on disk"; else pass "a V3.0 station has no V2.4 files on disk"; fi

# ── P2. the dispatcher ──────────────────────────────────────────────────────
echo "=== P2. the occurrence-filter dispatcher fetches one family and never the other ==="
dispatch() { # $1 = classifier id; prints which downloader ran
    (
        # shellcheck disable=SC1090
        source <(flow_script)
        download_range_filter() { echo range; }
        download_geomodel()     { echo geomodel; }
        select_classifier "$1"
        download_occurrence_filter
    ) 2>&1
}
[ "$(dispatch birdnet-v24)" = "range" ]    && pass "birdnet-v24 -> the range filter only" || fail "birdnet-v24 dispatched to '$(dispatch birdnet-v24)'"
[ "$(dispatch birdnet-v3)" = "geomodel" ]  && pass "birdnet-v3 -> the geomodel only"      || fail "birdnet-v3 dispatched to '$(dispatch birdnet-v3)'"

# ── P3. write_config handed a contradiction ─────────────────────────────────
echo "=== P3. write_config does not mix even when the wrong family's flag is set ==="
render() { # $1 classifier  $2 GEOMODEL_INSTALLED  $3 RANGE_FILTER_INSTALLED -> sandbox path
    local sb
    sb="$(flow_new)"; SANDBOXES+=("${sb}")
    (
        # shellcheck disable=SC1090
        source <(flow_script)
        # shellcheck disable=SC2034  # read by the sourced installer
        {
            CONFIG_DIR="${sb}/etc/birdnet"; CONFIG_FILE="${CONFIG_DIR}/birdnet.conf"
            DATA_DIR="${sb}/home/BirdNet-Behavior"; RECS_DIR="${DATA_DIR}/recordings"
            IMAGE_CACHE_DIR="${DATA_DIR}/image_cache"; MODEL_DIR="${DATA_DIR}/models"
            DB_PATH="${DATA_DIR}/birds.db"; INSTALL_DIR="${sb}/bin"; SERVICE_USER="$(id -un)"
            ALSA_CARD_VALUE=""; RTSP_URL_VALUE=""; LATITUDE_VALUE=""; LONGITUDE_VALUE=""
            CADDY_USER_VALUE=""; CADDY_PWD_VALUE=""
        }
        chown() { :; }
        info() { :; }; success() { :; }; warn() { :; }
        select_classifier "$1"
        GEOMODEL_INSTALLED="$2"
        RANGE_FILTER_INSTALLED="$3"
        write_config
    ) >/dev/null 2>&1
    printf '%s' "${sb}"
}
cfg="$(flow_config "$(render birdnet-v24 1 0)")"
if ! grep -qE '^METADATA_(MODEL|LABELS)_PATH=' "${cfg}" && ! grep -q 'Geomodel' <(grep -E '^[A-Z_]+=' "${cfg}"); then
    pass "V2.4 with only GEOMODEL_INSTALLED set: no filter is written (the geomodel is not substituted)"
else
    fail "V2.4 was given the geomodel: $(grep -E '^METADATA_' "${cfg}" | tr '\n' ' ')"
fi
sb="$(render birdnet-v24 1 1)"; cfg="$(flow_config "${sb}")"
if [ "$(grep -E '^METADATA_MODEL_PATH=' "${cfg}" | cut -d= -f2-)" = "$(flow_models_dir "${sb}")/${FLOW_V24_RANGE}" ] \
    && ! grep -qE '^METADATA_LABELS_PATH=' "${cfg}"; then
    pass "V2.4 with both flags set: the range filter, and no labels line"
else
    fail "V2.4 with both flags set wrote: $(grep -E '^METADATA_' "${cfg}" | tr '\n' ' ')"
fi
cfg="$(flow_config "$(render birdnet-v3 0 1)")"
if ! grep -qE '^METADATA_(MODEL|LABELS)_PATH=' "${cfg}"; then
    pass "V3.0 with only RANGE_FILTER_INSTALLED set: no filter is written (the range filter is not substituted)"
else
    fail "V3.0 was given the range filter: $(grep -E '^METADATA_' "${cfg}" | tr '\n' ' ')"
fi
cfg="$(flow_config "$(render birdnet-v3 1 1)")"
if grep -qE "^METADATA_MODEL_PATH=.*/${FLOW_GEO_MODEL//+/\\+}\$" "${cfg}" && grep -qE '^METADATA_LABELS_PATH=' "${cfg}" \
    && ! grep -q "${FLOW_V24_RANGE}" <(grep -E '^[A-Z_]+=' "${cfg}"); then
    pass "V3.0 with both flags set: the geomodel and its labels"
else
    fail "V3.0 with both flags set wrote: $(grep -E '^METADATA_' "${cfg}" | tr '\n' ' ')"
fi

# ── P4. the kept-config retrofits ───────────────────────────────────────────
echo "=== P4. each retrofit refuses a config of the other family ==="
v24_conf="MODEL_PATH=@MD@/${FLOW_V24_MODEL}
# METADATA_MODEL_PATH=
# METADATA_LABELS_PATH="
v3_conf="MODEL_PATH=@MD@/${FLOW_V3_MODEL}
# METADATA_MODEL_PATH=
# METADATA_LABELS_PATH="
# The retrofits compare the config's MODEL_PATH with ${MODEL_DIR}/<file>, and
# MODEL_DIR is inside a sandbox that does not exist until the helper makes it;
# so the config is a template with @MD@ where the models directory goes.
retrofit_in_place() { # $1 function  $2 config template (with @MD@)
    local sb conf
    sb="$(flow_new)"; SANDBOXES+=("${sb}")
    conf="${2//@MD@/${sb}/home/BirdNet-Behavior/models}"
    printf '%s\n' "${conf}" >"${sb}/etc/birdnet/birdnet.conf"
    (
        # shellcheck disable=SC1090
        source <(flow_script)
        # shellcheck disable=SC2034
        {
            CONFIG_FILE="${sb}/etc/birdnet/birdnet.conf"
            MODEL_DIR="${sb}/home/BirdNet-Behavior/models"
            INSTALL_DIR="${sb}/bin"
        }
        info() { :; }; success() { :; }; warn() { :; }
        # shellcheck disable=SC2034  # read by the sourced retrofit functions
        GEOMODEL_INSTALLED=1
        # shellcheck disable=SC2034
        RANGE_FILTER_INSTALLED=1
        "$1"
    ) >/dev/null 2>&1
    printf '%s' "${sb}"
}
sb="$(retrofit_in_place enable_geomodel_in_kept_config "${v24_conf}")"
if [ "$(cat "${sb}/etc/birdnet/birdnet.conf")" = "${v24_conf//@MD@/${sb}/home/BirdNet-Behavior/models}" ]; then
    pass "the geomodel retrofit leaves a V2.4 config alone"
else
    fail "the geomodel retrofit wrote into a V2.4 config: $(grep METADATA "${sb}/etc/birdnet/birdnet.conf" | tr '\n' ' ')"
fi
sb="$(retrofit_in_place enable_range_filter_in_kept_config "${v3_conf}")"
if [ "$(cat "${sb}/etc/birdnet/birdnet.conf")" = "${v3_conf//@MD@/${sb}/home/BirdNet-Behavior/models}" ]; then
    pass "the range-filter retrofit leaves a V3.0 config alone"
else
    fail "the range-filter retrofit wrote into a V3.0 config: $(grep METADATA "${sb}/etc/birdnet/birdnet.conf" | tr '\n' ' ')"
fi
# Counterparts: each retrofit does its own family's work, so the refusals above
# are not just "never writes anything".
sb="$(retrofit_in_place enable_range_filter_in_kept_config "${v24_conf}")"
if grep -qE "^METADATA_MODEL_PATH=.*/${FLOW_V24_RANGE}\$" "${sb}/etc/birdnet/birdnet.conf"; then
    pass "counterpart: the range-filter retrofit fills a V2.4 config's placeholder"
else
    fail "counterpart: the range-filter retrofit did not fill a V2.4 placeholder: $(cat "${sb}/etc/birdnet/birdnet.conf")"
fi
sb="$(retrofit_in_place enable_geomodel_in_kept_config "${v3_conf}")"
if grep -qE "^METADATA_MODEL_PATH=.*/Geomodel|^METADATA_MODEL_PATH=.*BirdNET\\+_Geomodel" "${sb}/etc/birdnet/birdnet.conf"; then
    pass "counterpart: the geomodel retrofit fills a V3.0 config's placeholder"
else
    fail "counterpart: the geomodel retrofit did not fill a V3.0 placeholder: $(cat "${sb}/etc/birdnet/birdnet.conf")"
fi

echo
if [ "${FAILED}" -ne 0 ]; then
    echo "classifier-never-mixed.sh: FAILED"
    exit 1
fi
echo "classifier-never-mixed.sh: all pass"
