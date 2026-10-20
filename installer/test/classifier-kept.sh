#!/usr/bin/env bash
# installer/test/classifier-kept.sh — does a station that already exists keep
# the classifier it has?
#
# The default became BirdNET V2.4. Every station installed before that runs
# BirdNET+ V3.0, and its database holds rows only that network scored. An
# update, a repair or a reinstall that quietly moved it to V2.4 would put two
# networks' confidences in one history and change what the station detects with
# nobody asking — the one thing an upgrade must never do. The config is pinned
# per station, so these re-run the real installer over existing stations and
# check that nothing about their model changes:
#
#   K1  a V3.0 station, updated:        its config is byte-identical, and
#                                       nothing is downloaded or installed
#                                       that it did not already have.
#   K2  ... with --classifier birdnet-v24: the same, and the flag is reported as
#                                       not applied rather than ignored.
#   K3  ... repaired with its model gone: V3.0 is what is re-fetched.
#   K4  a V3.0 config from before the geomodel shipped: the geomodel retrofit
#                                       still happens, V2.4 files do not appear.
#   K5  a config naming a model of the operator's own: untouched.
#   K6  a V2.4 station, updated, with and without --classifier birdnet-v3.
#   K7  a V2.4 station whose range filter failed the first time: the empty
#                                       placeholder is filled in, the binary is
#                                       asked for that file only, MODEL_PATH is
#                                       untouched.
#   K8  no config but the V3.0 model on disk (a restore from backup): V3.0;
#                                       and an explicit request still wins.
#   K9  a V2.4 station whose model is missing: the binary is asked for the
#                                       model, and for nothing else.
#   K10 a range filter the operator commented out is left commented.
#
# The V3.0 config is a fixture captured from the PREVIOUS installer
# (installer/test/data/), not one this branch wrote, so "untouched" is
# measured against what is really out there.
#
# Usage: installer/test/classifier-kept.sh
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

# A V3.0 station as the previous installer left it: its config, all four
# V3.0 files, and a binary (so the installer sees an existing install and
# updates, as `curl | sudo bash` does non-interactively).
seed_v3_station() {
    sed "s|@SB@|${SB}|g" "${FIXTURE}" >"$(flow_config "${SB}")"
    for f in v3-model v3-labels geo-model geo-labels; do flow_seed "${SB}" "${f}"; done
    flow_seed_binary "${SB}"
}
seed_v24_station() { # a fresh V2.4 install by this installer, then the binary
    flow_run "${SB}" >/dev/null 2>&1
    flow_seed_binary "${SB}"
    : >"${SB}/requests.log"
}

digest() { sha256sum <"$(flow_config "$1")" | awk '{print $1}'; }
models_matching() { flow_models_count "$1" "$2"; }
gets() { grep -c '^GET ' "$1/requests.log" || true; }
# Everything that would have cost a download: the installer's own fetches (V3.0)
# and the binary being asked to install a model (V2.4).
calls() { grep -cE '^(GET|INSTALL-MODEL) ' "$1/requests.log" || true; }
installs() { grep -c '^INSTALL-MODEL ' "$1/requests.log" || true; }

# ── K1 ──────────────────────────────────────────────────────────────────────
echo "=== K1. an existing V3.0 station, updated, is left exactly as it is ==="
new_sandbox
seed_v3_station
before="$(digest "${SB}")"
flow_run "${SB}"
rc=$?
[ "${rc}" -eq 0 ] && pass "the update succeeded" || { fail "the update failed (exit ${rc})"; sed 's/^/        /' "${SB}/out.log" | tail -8; }
[ "$(digest "${SB}")" = "${before}" ] && pass "the config is byte-identical" \
    || fail "the update changed the config: $(diff <(sed "s|@SB@|${SB}|g" "${FIXTURE}") "$(flow_config "${SB}")" | head -6 | tr '\n' ' ')"
[ "$(models_matching "${SB}" 'birdnet-v24')" = 0 ] && pass "no V2.4 file appeared in the models directory" \
    || fail "V2.4 files were installed beside a V3.0 station: $(ls "$(flow_models_dir "${SB}")" | tr '\n' ' ')"
[ "$(calls "${SB}")" = 0 ] && pass "nothing was downloaded or installed: the verified V3.0 files were reused" \
    || fail "$(calls "${SB}") download(s) for a station that already had everything: $(cat "${SB}/requests.log" | tr '\n' ' ')"
grep -qE 'Classifier: BirdNET\+ V3\.0 preview.*' "${SB}/out.log" && grep -q 'kept from' "${SB}/out.log" \
    && pass "the update says the classifier is kept" || fail "the update did not say which classifier it kept"
grep -qE '^ +Model: +BirdNET\+ V3\.0 preview' "${SB}/out.log" \
    && pass "the summary names the V3.0 preview" || fail "the summary does not name the V3.0 preview"

# ── K2 ──────────────────────────────────────────────────────────────────────
echo "=== K2. counterpart: --classifier birdnet-v24 over a V3.0 station is reported, not applied ==="
new_sandbox
seed_v3_station
before="$(digest "${SB}")"
flow_run "${SB}" --classifier birdnet-v24
rc=$?
[ "${rc}" -eq 0 ] && pass "the update succeeded" || fail "the update failed (exit ${rc})"
[ "$(digest "${SB}")" = "${before}" ] && pass "the config is byte-identical" || fail "the flag rewrote a running station's config"
[ "$(models_matching "${SB}" 'birdnet-v24')" = 0 ] && [ "$(calls "${SB}")" = 0 ] \
    && pass "and no V2.4 file was installed" || fail "V2.4 was installed for a V3.0 station"
if grep -q -- '--classifier birdnet-v24 was NOT applied' "${SB}/out.log"; then
    pass "the operator is told the flag was not applied, and why"
else
    fail "the flag was silently ignored:"; sed 's/^/        /' "${SB}/out.log" | grep -i classifier
fi
# The same through the environment.
new_sandbox
seed_v3_station
before="$(digest "${SB}")"
BIRDNET_CLASSIFIER=birdnet-v24 flow_run "${SB}"
[ "$(digest "${SB}")" = "${before}" ] && pass "BIRDNET_CLASSIFIER=birdnet-v24 does not rewrite it either" \
    || fail "BIRDNET_CLASSIFIER rewrote a running station's config"

# ── K3 ──────────────────────────────────────────────────────────────────────
echo "=== K3. repairing a V3.0 station whose model is gone re-fetches V3.0 ==="
new_sandbox
seed_v3_station
rm -f "$(flow_models_dir "${SB}")/${FLOW_V3_MODEL}"
before="$(digest "${SB}")"
flow_run "${SB}" repair
rc=$?
[ "${rc}" -eq 0 ] && pass "the repair succeeded" || { fail "the repair failed (exit ${rc})"; sed 's/^/        /' "${SB}/out.log" | tail -8; }
[ -s "$(flow_models_dir "${SB}")/${FLOW_V3_MODEL}" ] && pass "the V3.0 model is back" || fail "the V3.0 model was not re-fetched"
[ "$(models_matching "${SB}" 'birdnet-v24')" = 0 ] && [ "$(installs "${SB}")" = 0 ] \
    && pass "and the binary was not asked for V2.4" || fail "V2.4 was installed during a V3.0 repair"
[ "$(digest "${SB}")" = "${before}" ] && pass "the config is byte-identical" || fail "the repair changed the config"

# ── K4 ──────────────────────────────────────────────────────────────────────
echo "=== K4. a V3.0 config from before the geomodel still gets the geomodel, and only that ==="
new_sandbox
seed_v3_station
cat >"$(flow_config "${SB}")" <<EOF
# a config from before the geomodel shipped
DB_PATH=${SB}/home/BirdNet-Behavior/birds.db
MODEL_PATH=$(flow_models_dir "${SB}")/${FLOW_V3_MODEL}
LABELS_PATH=$(flow_models_dir "${SB}")/${FLOW_V3_LABELS}
LATITUDE=52.5200
EOF
rm -f "$(flow_models_dir "${SB}")/${FLOW_GEO_MODEL}" "$(flow_models_dir "${SB}")/${FLOW_GEO_LABELS}"
flow_run "${SB}"
if [ "$(flow_conf "${SB}" METADATA_MODEL_PATH)" = "$(flow_models_dir "${SB}")/${FLOW_GEO_MODEL}" ] \
    && [ "$(flow_conf "${SB}" METADATA_LABELS_PATH)" = "$(flow_models_dir "${SB}")/${FLOW_GEO_LABELS}" ]; then
    pass "the geomodel was added (the V3.0 retrofit is intact)"
else
    fail "the V3.0 retrofit no longer happens: METADATA_MODEL_PATH='$(flow_conf "${SB}" METADATA_MODEL_PATH)'"
fi
[ "$(flow_conf "${SB}" MODEL_PATH)" = "$(flow_models_dir "${SB}")/${FLOW_V3_MODEL}" ] && [ "$(flow_conf "${SB}" LATITUDE)" = "52.5200" ] \
    && pass "MODEL_PATH and the operator's own settings are untouched" || fail "the retrofit changed the model or the operator's settings"
[ "$(models_matching "${SB}" 'birdnet-v24')" = 0 ] && [ "$(installs "${SB}")" = 0 ] \
    && pass "and no V2.4 file was involved" || fail "V2.4 files were involved in a V3.0 retrofit"

# ── K5 ──────────────────────────────────────────────────────────────────────
echo "=== K5. a config naming a model of the operator's own is left alone ==="
new_sandbox
seed_v3_station
sed -i "s|^MODEL_PATH=.*|MODEL_PATH=/srv/models/my-own.onnx|" "$(flow_config "${SB}")"
before="$(digest "${SB}")"
flow_run "${SB}"
[ "$(digest "${SB}")" = "${before}" ] && pass "the config is byte-identical" || fail "the update rewrote a config naming a custom model"
[ "$(models_matching "${SB}" 'birdnet-v24')" = 0 ] && [ "$(installs "${SB}")" = 0 ] \
    && pass "no V2.4 file was installed for it" || fail "V2.4 was installed for a station on a custom model"
grep -q 'MODEL_PATH (/srv/models/my-own.onnx)' "${SB}/out.log" \
    && pass "the summary names the operator's model, not one of ours" || fail "the summary misdescribes a custom model"

# ── K6 ──────────────────────────────────────────────────────────────────────
echo "=== K6. a V2.4 station, updated, keeps V2.4 — with or without a contrary flag ==="
new_sandbox
seed_v24_station
before="$(digest "${SB}")"
flow_run "${SB}"
[ "$(digest "${SB}")" = "${before}" ] && pass "updated: the config is byte-identical" || fail "an update rewrote the V2.4 config"
[ "$(calls "${SB}")" = 0 ] && pass "updated: nothing was downloaded and the binary was not asked to install anything" \
    || fail "updated: $(calls "${SB}") call(s) for a station that had everything: $(tr '\n' ' ' <"${SB}/requests.log")"
flow_run "${SB}" --classifier birdnet-v3
[ "$(digest "${SB}")" = "${before}" ] && pass "--classifier birdnet-v3: the config is byte-identical" || fail "the flag rewrote a V2.4 station"
if [ "$(models_matching "${SB}" 'V3|Geomodel')" = 0 ] && grep -q -- '--classifier birdnet-v3 was NOT applied' "${SB}/out.log"; then
    pass "--classifier birdnet-v3: no V3.0 file fetched, and the flag is reported as not applied"
else
    fail "--classifier birdnet-v3 over a V2.4 station: V3 files $(models_matching "${SB}" 'V3|Geomodel'); reported: $(grep -c 'NOT applied' "${SB}/out.log")"
fi

# ── K7 ──────────────────────────────────────────────────────────────────────
echo "=== K7. a V2.4 station whose range filter failed gets it on the next run, and nothing else changes ==="
new_sandbox
FLOW_FAIL_RANGE=1 flow_run "${SB}" >/dev/null 2>&1
flow_seed_binary "${SB}"
: >"${SB}/requests.log"
grep -qE '^# METADATA_MODEL_PATH=$' "$(flow_config "${SB}")" && pass "precondition: the first install left the empty placeholder" \
    || fail "precondition: no placeholder after a failed range-filter fetch"
model_before="$(flow_conf "${SB}" MODEL_PATH)"
flow_run "${SB}" repair
if [ "$(flow_conf "${SB}" METADATA_MODEL_PATH)" = "$(flow_models_dir "${SB}")/${FLOW_V24_RANGE}" ] && [ -s "$(flow_models_dir "${SB}")/${FLOW_V24_RANGE}" ]; then
    pass "the placeholder is filled in with the range filter, and the file is there"
else
    fail "the repair did not enable the range filter: '$(flow_conf "${SB}" METADATA_MODEL_PATH)'"
fi
[ -z "$(flow_conf "${SB}" METADATA_LABELS_PATH)" ] && pass "and still no METADATA_LABELS_PATH" || fail "a METADATA_LABELS_PATH appeared"
[ "$(flow_conf "${SB}" MODEL_PATH)" = "${model_before}" ] && pass "MODEL_PATH is untouched" || fail "MODEL_PATH changed"
if [ "$(calls "${SB}")" = 1 ] && grep -qx "INSTALL-MODEL birdnet-v24-range-v2 dir=$(flow_models_dir "${SB}")" "${SB}/requests.log"; then
    pass "exactly one call: the binary was asked for the range filter, and for nothing else"
else
    fail "unexpected calls: $(tr '\n' ' ' <"${SB}/requests.log")"
fi

# ── K8 ──────────────────────────────────────────────────────────────────────
echo "=== K8. no config but the V3.0 model on disk: keep V3.0; an explicit request still wins ==="
new_sandbox
flow_seed "${SB}" v3-model
flow_run "${SB}"
if [ "$(flow_conf "${SB}" MODEL_PATH)" = "$(flow_models_dir "${SB}")/${FLOW_V3_MODEL}" ] \
    && grep -q 'a station restored from a backup must not change networks' "${SB}/out.log"; then
    pass "V3.0 is kept, and the operator is told why and how to choose V2.4"
else
    fail "a restored V3.0 station was moved: MODEL_PATH='$(flow_conf "${SB}" MODEL_PATH)'"
fi
new_sandbox
flow_seed "${SB}" v3-model
flow_run "${SB}" --classifier birdnet-v24
[ "$(flow_conf "${SB}" MODEL_PATH)" = "$(flow_models_dir "${SB}")/${FLOW_V24_MODEL}" ] \
    && pass "counterpart: --classifier birdnet-v24 moves it, because someone asked" \
    || fail "an explicit --classifier birdnet-v24 was overridden by the leftover V3.0 file"

# ── K9 ─────────────────────────────────────────────────────────────────────
echo "=== K9. a V2.4 station whose model is gone gets the model back, and only the model ==="
new_sandbox
seed_v24_station
rm -f "$(flow_models_dir "${SB}")/${FLOW_V24_MODEL}"
before="$(digest "${SB}")"
flow_run "${SB}" repair
rc=$?
[ "${rc}" -eq 0 ] && pass "the repair succeeded" || { fail "the repair failed (exit ${rc})"; sed 's/^/        /' "${SB}/out.log" | tail -8; }
[ -s "$(flow_models_dir "${SB}")/${FLOW_V24_MODEL}" ] && pass "the model is back" || fail "the model was not reinstalled"
if [ "$(calls "${SB}")" = 1 ] && grep -qx "INSTALL-MODEL birdnet-v24 dir=$(flow_models_dir "${SB}")" "${SB}/requests.log"; then
    pass "by one call to the binary for birdnet-v24; the range filter that was there was left alone"
else
    fail "calls: $(tr '\n' ' ' <"${SB}/requests.log")"
fi
[ "$(digest "${SB}")" = "${before}" ] && pass "the config is byte-identical" || fail "the repair changed the config"

# ── K10 ──────────────────────────────────────────────────────────────────────
echo "=== K10. a range filter the operator commented out stays commented ==="
new_sandbox
FLOW_FAIL_RANGE=1 flow_run "${SB}" >/dev/null 2>&1
flow_seed_binary "${SB}"
sed -i "s|^# METADATA_MODEL_PATH=\$|# METADATA_MODEL_PATH=/srv/models/their-own-filter.onnx|" "$(flow_config "${SB}")"
before="$(digest "${SB}")"
flow_run "${SB}" repair
[ "$(digest "${SB}")" = "${before}" ] && pass "the commented line is theirs: the config is byte-identical" \
    || fail "the repair rewrote an operator's commented-out METADATA_MODEL_PATH"

echo
if [ "${FAILED}" -ne 0 ]; then
    echo "classifier-kept.sh: FAILED"
    exit 1
fi
echo "classifier-kept.sh: all pass"
