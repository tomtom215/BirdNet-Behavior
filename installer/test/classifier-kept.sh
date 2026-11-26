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
# check that nothing about their model changes. Every model is installed by the
# binary (MDL-09), which re-hashes what is there and fetches only what does not
# verify, so "nothing downloaded" is read from what the stub binary had to
# write; and a V3.0 station keeps its files, moved by the binary (a hard link,
# no download) from the names the previous installer gave them onto the names
# the binary gives them, with its config pointed at those:
#
#   K1  a V3.0 station from the previous installer, updated: its config differs
#                                       only in the four file names, the same
#                                       files are adopted under them (nothing
#                                       downloaded), the old names are gone;
#                                       updated again, it is byte-identical.
#   K2  ... with --classifier birdnet-v24: the same, and the flag is reported as
#                                       not applied rather than ignored.
#   K3  ... repaired with its model gone: V3.0 is what is re-fetched.
#   K4  a V3.0 config from before the geomodel shipped: the geomodel retrofit
#                                       still happens, V2.4 files do not appear.
#   K5  a config naming a model of the operator's own: that line untouched.
#   K6  a V2.4 station, updated, with and without --classifier birdnet-v3.
#   K7  a V2.4 station whose range filter failed the first time: the empty
#                                       placeholder is filled in, only that file
#                                       is fetched, MODEL_PATH is untouched.
#   K8  no config but the V3.0 model on disk (a restore from backup): V3.0;
#                                       and an explicit request still wins.
#   K9  a V2.4 station whose model is missing: the model is fetched, and
#                                       nothing else.
#   K10 a range filter the operator commented out is left commented.
#   K11 a V3.0 station whose old model file is damaged, with the network down:
#                                       not adopted, and the config keeps the
#                                       name it had.
#
# The V3.0 config is a fixture captured from the PREVIOUS installer
# (installer/test/data/), not one this branch wrote, so "kept" is measured
# against what is really out there.
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
# V3.0 files under the names it gave them, and a binary (so the installer sees
# an existing install and updates, as `curl | sudo bash` does
# non-interactively).
seed_v3_station() {
    sed "s|@SB@|${SB}|g" "${FIXTURE}" >"$(flow_config "${SB}")"
    for f in legacy-v3-model legacy-v3-labels legacy-geo-model legacy-geo-labels; do flow_seed "${SB}" "${f}"; done
    flow_seed_binary "${SB}"
}
# That station's config once the update has moved it onto the binary's names.
moved_v3_config() {
    local dir; dir="$(flow_models_dir "${SB}")"
    sed -e "s|@SB@|${SB}|g" -e "s|${dir}/${FLOW_V3_LEGACY_MODEL}|${dir}/${FLOW_V3_MODEL}|" \
        -e "s|${dir}/${FLOW_V3_LEGACY_LABELS}|${dir}/${FLOW_V3_LABELS}|" \
        -e "s|${dir}/${FLOW_GEO_LEGACY_MODEL}|${dir}/${FLOW_GEO_MODEL}|" \
        -e "s|${dir}/${FLOW_GEO_LEGACY_LABELS}|${dir}/${FLOW_GEO_LABELS}|" "${FIXTURE}"
}
seed_v24_station() { # a fresh V2.4 install by this installer, then the binary
    flow_run "${SB}" >/dev/null 2>&1
    flow_seed_binary "${SB}"
    : >"${SB}/requests.log"
}

digest() { sha256sum <"$(flow_config "$1")" | awk '{print $1}'; }
models_matching() { flow_models_count "$1" "$2"; }
# What would have cost a download: a file the binary had to write, or anything
# the installer tried to fetch itself.
fetched() { grep -cE '^(GET|WROTE) ' "$1/requests.log" || true; }
asked() { grep '^INSTALL-MODEL ' "$1/requests.log" | awk '{print $2}' | tr '\n' ' '; }
no_legacy_left() { [ "$(models_matching "$1" '^BirdNET\+_')" = 0 ]; }

# ── K1 ──────────────────────────────────────────────────────────────────────
echo "=== K1. an existing V3.0 station, updated, keeps its files and its classifier ==="
new_sandbox
seed_v3_station
inode_before="$(stat -c %i "$(flow_models_dir "${SB}")/${FLOW_V3_LEGACY_MODEL}")"
flow_run "${SB}"
rc=$?
[ "${rc}" -eq 0 ] && pass "the update succeeded" || { fail "the update failed (exit ${rc})"; sed 's/^/        /' "${SB}/out.log" | tail -8; }
if diff -q <(moved_v3_config) "$(flow_config "${SB}")" >/dev/null; then
    pass "the config is the previous installer's but for the four file names, which are the binary's"
else
    fail "the update changed the config beyond the names: $(diff <(moved_v3_config) "$(flow_config "${SB}")" | head -6 | tr '\n' ' ')"
fi
[ "$(fetched "${SB}")" = 0 ] && pass "nothing was downloaded: the verified files were adopted" \
    || fail "$(fetched "${SB}") download(s) for a station that already had everything: $(tr '\n' ' ' <"${SB}/requests.log")"
[ "$(stat -c %i "$(flow_models_dir "${SB}")/${FLOW_V3_MODEL}" 2>/dev/null)" = "${inode_before}" ] \
    && pass "the model under its new name is the same file (no 541 MB copy)" || fail "the model was copied or replaced"
no_legacy_left "${SB}" && pass "the old names are gone" || fail "old names left: $(ls "$(flow_models_dir "${SB}")" | tr '\n' ' ')"
[ "$(asked "${SB}")" = "birdnet-v3 birdnet-v3-geomodel " ] && pass "the binary was asked for V3.0 and its geomodel, and nothing else" \
    || fail "the binary was asked for: $(asked "${SB}")"
[ "$(models_matching "${SB}" 'birdnet-v24')" = 0 ] && pass "no V2.4 file appeared in the models directory" \
    || fail "V2.4 files were installed beside a V3.0 station: $(ls "$(flow_models_dir "${SB}")" | tr '\n' ' ')"
grep -qE 'Classifier: BirdNET\+ V3\.0 preview.*' "${SB}/out.log" && grep -q 'kept from' "${SB}/out.log" \
    && pass "the update says the classifier is kept" || fail "the update did not say which classifier it kept"
grep -qE '^ +Model: +BirdNET\+ V3\.0 preview' "${SB}/out.log" \
    && pass "the summary names the V3.0 preview" || fail "the summary does not name the V3.0 preview"
# Counterpart: updated again, there is nothing left to move.
before="$(digest "${SB}")"
: >"${SB}/requests.log"
flow_run "${SB}"
[ "$(digest "${SB}")" = "${before}" ] && [ "$(fetched "${SB}")" = 0 ] \
    && pass "updated again: the config is byte-identical and nothing is fetched" \
    || fail "a second update changed the config or fetched: $(tr '\n' ' ' <"${SB}/requests.log")"

# ── K2 ──────────────────────────────────────────────────────────────────────
echo "=== K2. counterpart: --classifier birdnet-v24 over a V3.0 station is reported, not applied ==="
new_sandbox
seed_v3_station
flow_run "${SB}" --classifier birdnet-v24
rc=$?
[ "${rc}" -eq 0 ] && pass "the update succeeded" || fail "the update failed (exit ${rc})"
diff -q <(moved_v3_config) "$(flow_config "${SB}")" >/dev/null && pass "the config still runs V3.0" \
    || fail "the flag rewrote a running station's config"
[ "$(models_matching "${SB}" 'birdnet-v24')" = 0 ] && [ "$(fetched "${SB}")" = 0 ] \
    && pass "and no V2.4 file was installed" || fail "V2.4 was installed for a V3.0 station"
if grep -q -- '--classifier birdnet-v24 was NOT applied' "${SB}/out.log"; then
    pass "the operator is told the flag was not applied, and why"
else
    fail "the flag was silently ignored:"; sed 's/^/        /' "${SB}/out.log" | grep -i classifier
fi
# The same through the environment.
new_sandbox
seed_v3_station
BIRDNET_CLASSIFIER=birdnet-v24 flow_run "${SB}"
diff -q <(moved_v3_config) "$(flow_config "${SB}")" >/dev/null && pass "BIRDNET_CLASSIFIER=birdnet-v24 does not switch it either" \
    || fail "BIRDNET_CLASSIFIER rewrote a running station's config"

# ── K3 ──────────────────────────────────────────────────────────────────────
echo "=== K3. repairing a V3.0 station whose model is gone re-fetches V3.0 ==="
new_sandbox
seed_v3_station
rm -f "$(flow_models_dir "${SB}")/${FLOW_V3_LEGACY_MODEL}"
flow_run "${SB}" repair
rc=$?
[ "${rc}" -eq 0 ] && pass "the repair succeeded" || { fail "the repair failed (exit ${rc})"; sed 's/^/        /' "${SB}/out.log" | tail -8; }
[ -s "$(flow_models_dir "${SB}")/${FLOW_V3_MODEL}" ] && grep -qx "WROTE ${FLOW_V3_MODEL}" "${SB}/requests.log" \
    && pass "the V3.0 model is back, fetched" || fail "the V3.0 model was not re-fetched"
[ "$(models_matching "${SB}" 'birdnet-v24')" = 0 ] && ! grep -q 'INSTALL-MODEL birdnet-v24' "${SB}/requests.log" \
    && pass "and the binary was not asked for V2.4" || fail "V2.4 was installed during a V3.0 repair"
diff -q <(moved_v3_config) "$(flow_config "${SB}")" >/dev/null && pass "the config names the binary's files, and is otherwise unchanged" \
    || fail "the repair changed the config: $(diff <(moved_v3_config) "$(flow_config "${SB}")" | head -4 | tr '\n' ' ')"

# ── K4 ──────────────────────────────────────────────────────────────────────
echo "=== K4. a V3.0 config from before the geomodel still gets the geomodel, and only that ==="
new_sandbox
seed_v3_station
{
    echo "# a config from before the geomodel shipped"
    echo "DB_PATH=${SB}/home/BirdNet-Behavior/birds.db"
    echo "MODEL_PATH=$(flow_models_dir "${SB}")/${FLOW_V3_LEGACY_MODEL}"
    echo "LABELS_PATH=$(flow_models_dir "${SB}")/${FLOW_V3_LEGACY_LABELS}"
    echo "LATITUDE=52.5200"
} >"$(flow_config "${SB}")"
rm -f "$(flow_models_dir "${SB}")/${FLOW_GEO_LEGACY_MODEL}" "$(flow_models_dir "${SB}")/${FLOW_GEO_LEGACY_LABELS}"
flow_run "${SB}"
if [ "$(flow_conf "${SB}" METADATA_MODEL_PATH)" = "$(flow_models_dir "${SB}")/${FLOW_GEO_MODEL}" ] \
    && [ "$(flow_conf "${SB}" METADATA_LABELS_PATH)" = "$(flow_models_dir "${SB}")/${FLOW_GEO_LABELS}" ]; then
    pass "the geomodel was added (the V3.0 retrofit is intact)"
else
    fail "the V3.0 retrofit no longer happens: METADATA_MODEL_PATH='$(flow_conf "${SB}" METADATA_MODEL_PATH)'"
fi
[ "$(flow_conf "${SB}" MODEL_PATH)" = "$(flow_models_dir "${SB}")/${FLOW_V3_MODEL}" ] && [ "$(flow_conf "${SB}" LATITUDE)" = "52.5200" ] \
    && pass "MODEL_PATH names the same model under the binary's name, and the operator's own settings are untouched" \
    || fail "the retrofit changed the model or the operator's settings: MODEL_PATH='$(flow_conf "${SB}" MODEL_PATH)'"
[ "$(models_matching "${SB}" 'birdnet-v24')" = 0 ] && ! grep -q 'INSTALL-MODEL birdnet-v24' "${SB}/requests.log" \
    && pass "and no V2.4 file was involved" || fail "V2.4 files were involved in a V3.0 retrofit"

# ── K5 ──────────────────────────────────────────────────────────────────────
echo "=== K5. a config naming a model of the operator's own: that line is left alone ==="
new_sandbox
seed_v3_station
sed -i "s|^MODEL_PATH=.*|MODEL_PATH=/srv/models/my-own.onnx|" "$(flow_config "${SB}")"
flow_run "${SB}"
[ "$(flow_conf "${SB}" MODEL_PATH)" = "/srv/models/my-own.onnx" ] && pass "MODEL_PATH is the operator's still" \
    || fail "the update rewrote a config naming a custom model"
[ "$(models_matching "${SB}" 'birdnet-v24')" = 0 ] && ! grep -q 'INSTALL-MODEL birdnet-v24' "${SB}/requests.log" \
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
[ "$(fetched "${SB}")" = 0 ] && [ "$(asked "${SB}")" = "birdnet-v24 birdnet-v24-range-v2 " ] \
    && pass "updated: the binary re-checked V2.4 and its filter, and nothing was downloaded" \
    || fail "updated: asked for '$(asked "${SB}")', fetched $(fetched "${SB}"): $(tr '\n' ' ' <"${SB}/requests.log")"
flow_run "${SB}" --classifier birdnet-v3
[ "$(digest "${SB}")" = "${before}" ] && pass "--classifier birdnet-v3: the config is byte-identical" || fail "the flag rewrote a V2.4 station"
if [ "$(models_matching "${SB}" 'v3')" = 0 ] && grep -q -- '--classifier birdnet-v3 was NOT applied' "${SB}/out.log"; then
    pass "--classifier birdnet-v3: no V3.0 file fetched, and the flag is reported as not applied"
else
    fail "--classifier birdnet-v3 over a V2.4 station: V3 files $(models_matching "${SB}" 'v3'); reported: $(grep -c 'NOT applied' "${SB}/out.log")"
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
if [ "$(fetched "${SB}")" = 1 ] && grep -qx "WROTE ${FLOW_V24_RANGE}" "${SB}/requests.log"; then
    pass "exactly one file fetched: the range filter, and nothing else"
else
    fail "unexpected fetches: $(tr '\n' ' ' <"${SB}/requests.log")"
fi

# ── K8 ──────────────────────────────────────────────────────────────────────
echo "=== K8. no config but the V3.0 model on disk: keep V3.0; an explicit request still wins ==="
for seed in legacy-v3-model v3-model; do
    new_sandbox
    flow_seed "${SB}" "${seed}"
    flow_run "${SB}"
    if [ "$(flow_conf "${SB}" MODEL_PATH)" = "$(flow_models_dir "${SB}")/${FLOW_V3_MODEL}" ] \
        && grep -q 'a station restored from a backup must not change networks' "${SB}/out.log" \
        && ! grep -qx "WROTE ${FLOW_V3_MODEL}" "${SB}/requests.log" && no_legacy_left "${SB}"; then
        pass "${seed}: V3.0 is kept, its file reused, and the operator is told why and how to choose V2.4"
    else
        fail "${seed}: a restored V3.0 station was moved or refetched: MODEL_PATH='$(flow_conf "${SB}" MODEL_PATH)'; $(tr '\n' ' ' <"${SB}/requests.log")"
    fi
done
new_sandbox
flow_seed "${SB}" legacy-v3-model
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
if [ "$(fetched "${SB}")" = 1 ] && grep -qx "WROTE ${FLOW_V24_MODEL}" "${SB}/requests.log"; then
    pass "only the model was fetched; the labels and the range filter that were there were kept"
else
    fail "fetches: $(tr '\n' ' ' <"${SB}/requests.log")"
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

# ── K11 ──────────────────────────────────────────────────────────────────────
echo "=== K11. a V3.0 station whose old model file is damaged, with the network down ==="
new_sandbox
seed_v3_station
printf 'truncated' >"$(flow_models_dir "${SB}")/${FLOW_V3_LEGACY_MODEL}"
FLOW_FAIL_V3_MODEL=1 flow_run "${SB}"
rc=$?
[ "${rc}" -eq 75 ] && pass "the update says the classifier is not installed (exit 75)" || fail "exit ${rc}, not 75"
[ "$(flow_conf "${SB}" MODEL_PATH)" = "$(flow_models_dir "${SB}")/${FLOW_V3_LEGACY_MODEL}" ] \
    && pass "MODEL_PATH keeps its old name: nothing verified to move it to" \
    || fail "MODEL_PATH was moved to a file that is not there: '$(flow_conf "${SB}" MODEL_PATH)'"
[ ! -e "$(flow_models_dir "${SB}")/${FLOW_V3_MODEL}" ] && pass "and the damaged file was not adopted" || fail "the damaged file was adopted"

echo
if [ "${FAILED}" -ne 0 ]; then
    echo "classifier-kept.sh: FAILED"
    exit 1
fi
echo "classifier-kept.sh: all pass"
