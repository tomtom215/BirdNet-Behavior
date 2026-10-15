#!/usr/bin/env bash
# Regenerate every parity reference artefact from scratch.
#
#   PI_ROOT=/path/to/BirdNET-Pi WORK=/path/outside/the/repo tools/parity/regenerate.sh
#
# PI_ROOT   a checkout of https://github.com/Nachtzuster/BirdNET-Pi at 88985a3a6fb1931a33f26ba1c46717a2a444c5b7
#           (git clone, then `git checkout` that commit). It is only read, never modified.
# WORK      scratch directory (venv, fixtures, big .npy arrays, JSON output). Keep it OUT of the repo.
# ONNX_DIR  optional: directory holding BirdNET_v2.4_fp32_dfttrunc.onnx + labels.txt; downloaded
#           from huggingface.co/tphakala/BirdNET-v2.4 (pinned commit, sha256-checked) when unset.
# CROSSCHECK=1  also build the second stack (Python 3.11 / numpy 1.26 / librosa 0.11 / tflite-runtime 2.14.0)
#           and diff it against the first (needs a python3.11 uv can find).
#
# Needs: uv, ffmpeg, curl, outbound HTTPS to raw.githubusercontent.com, archive.org, huggingface.co.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
: "${PI_ROOT:?set PI_ROOT to the BirdNET-Pi checkout}"
: "${WORK:?set WORK to a scratch directory outside the repo}"
PI_COMMIT=88985a3a6fb1931a33f26ba1c46717a2a444c5b7
HF_COMMIT=acd49768d227c726ac94550c94f0effb3e1dd04c
ONNX_SHA256=3b72e88b3ad0c310a41adabccf8cf75b1a05daeeb40884ebd38038c91d0e423d
export PYTHONDONTWRITEBYTECODE=1

got=$(git -C "$PI_ROOT" rev-parse HEAD 2>/dev/null || echo unknown)
[ "$got" = "$PI_COMMIT" ] || echo "WARNING: PI_ROOT is at $got, goldens were made at $PI_COMMIT" >&2

mkdir -p "$WORK/out"
cd /   # never run interpreters from inside downloaded data

if [ ! -x "$WORK/venv/bin/python" ]; then
  uv venv --python 3.12 "$WORK/venv"
  uv pip install --python "$WORK/venv/bin/python" -r "$HERE/requirements-primary.txt"
fi
PY="$WORK/venv/bin/python -I"

$PY "$HERE/fetch_fixtures.py" "$WORK/fixtures" --strict
$PY "$HERE/ref_pi.py" --pi-root "$PI_ROOT" selfcheck --out "$WORK/out/selfcheck.json"
$PY "$HERE/run_matrix.py" --pi-root "$PI_ROOT" --fixtures "$WORK/fixtures" --out "$WORK/out/matrix"
$PY "$HERE/ref_pi.py" --pi-root "$PI_ROOT" species-list --out "$WORK/out/species_list_e.json" \
    --location=-1,-1 --location 51.48,-0.13 --raw-dir "$WORK/out/mdata_raw"
$PY "$HERE/ref_pi.py" --pi-root "$PI_ROOT" species-list --compact --out "$WORK/out/species_list_allweeks_compact.json" \
    --weeks "$(seq -s, 1 53)" --location=-1,-1 --location 0,0 --location 51.48,-0.13 --location 50,5 --location 42.48,-76.45

if [ -z "${ONNX_DIR:-}" ]; then
  ONNX_DIR="$WORK/onnx"; mkdir -p "$ONNX_DIR"
  base="https://huggingface.co/tphakala/BirdNET-v2.4/resolve/$HF_COMMIT"
  [ -f "$ONNX_DIR/BirdNET_v2.4_fp32_dfttrunc.onnx" ] || curl -fsSL -o "$ONNX_DIR/BirdNET_v2.4_fp32_dfttrunc.onnx" "$base/BirdNET_v2.4_fp32_dfttrunc.onnx"
  [ -f "$ONNX_DIR/labels.txt" ] || curl -fsSL -o "$ONNX_DIR/labels.txt" "$base/labels.txt"
fi
echo "$ONNX_SHA256  $ONNX_DIR/BirdNET_v2.4_fp32_dfttrunc.onnx" | sha256sum -c -

$PY "$HERE/onnx_vs_tflite.py" \
    --onnx "$ONNX_DIR/BirdNET_v2.4_fp32_dfttrunc.onnx" --onnx-labels "$ONNX_DIR/labels.txt" \
    --pi-labels "$PI_ROOT/model/BirdNET_GLOBAL_6K_V2.4_Model_FP16_Labels.txt" \
    --tflite-model "$PI_ROOT/model/BirdNET_GLOBAL_6K_V2.4_Model_FP16.tflite" \
    --arrays "$WORK/out/matrix/arrays" --out "$WORK/out/onnx_vs_tflite.json"

( cd "$WORK/out/matrix/arrays" && sha256sum *.npy > "$WORK/out/arrays.sha256" )

# --- which V2.4 build is a faithful stand-in for BirdNET-Pi's FP16 TFLite? -------------------------------
# Extra builds (sha256 from the repo's SHA256SUMS / recorded here): full ONNX, int8 ONNX, birdnet-go's FP32 TFLite.
GO_COMMIT=f4f019a3d79cb547b432bd8729fb4cf9c71aa2e7
FULL_SHA256=f49b686838f62fc8c5bcb4364cd514c64882f6fd666c204aa5d4eb80a7795264
INT8_SHA256=6a2323fe8f99c2af4bb325cf8f11b52748a86aa1f2c9e706c1ae7f907289521a
TFL32_SHA256=55f3e4055b1a13bfa9a2452731d0d34f6a02d6b775a334362665892794165e4c
X="$WORK/onnx"; mkdir -p "$X"
hf="https://huggingface.co/tphakala/BirdNET-v2.4/resolve/$HF_COMMIT"
for f in BirdNET_v2.4_fp32.onnx BirdNET_v2.4_int8_arm.onnx; do
  [ -f "$ONNX_DIR/$f" ] && cp -n "$ONNX_DIR/$f" "$X/$f" || true
  [ -f "$X/$f" ] || curl -fsSL -o "$X/$f" "$hf/$f"
done
[ -f "$X/BirdNET_GLOBAL_6K_V2.4_Model_FP32.tflite" ] || curl -fsSL -o "$X/BirdNET_GLOBAL_6K_V2.4_Model_FP32.tflite" \
  "https://raw.githubusercontent.com/tphakala/birdnet-go/$GO_COMMIT/internal/classifier/data/BirdNET_GLOBAL_6K_V2.4_Model_FP32.tflite"
( cd "$X" && printf '%s  %s\n' "$FULL_SHA256" BirdNET_v2.4_fp32.onnx "$INT8_SHA256" BirdNET_v2.4_int8_arm.onnx \
    "$TFL32_SHA256" BirdNET_GLOBAL_6K_V2.4_Model_FP32.tflite | sha256sum -c - )

$PY "$HERE/variants_compare.py" --arrays "$WORK/out/matrix/arrays" \
    --pi-labels "$PI_ROOT/model/BirdNET_GLOBAL_6K_V2.4_Model_FP16_Labels.txt" \
    --tflite-fp16 "$PI_ROOT/model/BirdNET_GLOBAL_6K_V2.4_Model_FP16.tflite" \
    --tflite-fp32 "$X/BirdNET_GLOBAL_6K_V2.4_Model_FP32.tflite" \
    --onnx-dfttrunc "$ONNX_DIR/BirdNET_v2.4_fp32_dfttrunc.onnx" --onnx-full "$X/BirdNET_v2.4_fp32.onnx" \
    --onnx-int8 "$X/BirdNET_v2.4_int8_arm.onnx" --cache "$WORK/out/variants_cache" --out "$WORK/out/variants_compare.json"
$PY "$HERE/silence_probe.py" --arrays "$WORK/out/matrix/arrays" \
    --pi-labels "$PI_ROOT/model/BirdNET_GLOBAL_6K_V2.4_Model_FP16_Labels.txt" \
    --tflite-fp16 "$PI_ROOT/model/BirdNET_GLOBAL_6K_V2.4_Model_FP16.tflite" \
    --tflite-fp32 "$X/BirdNET_GLOBAL_6K_V2.4_Model_FP32.tflite" \
    --onnx-dfttrunc "$ONNX_DIR/BirdNET_v2.4_fp32_dfttrunc.onnx" --onnx-full "$X/BirdNET_v2.4_fp32.onnx" \
    --work "$WORK/out/probe_onnx" --out "$WORK/out/silence_probe.json"

if [ "${CROSSCHECK:-0}" = 1 ]; then
  if [ ! -x "$WORK/venv-crosscheck/bin/python" ]; then
    uv venv --python 3.11 "$WORK/venv-crosscheck"
    uv pip install --python "$WORK/venv-crosscheck/bin/python" -r "$HERE/requirements-crosscheck.txt"
  fi
  X="$WORK/venv-crosscheck/bin/python -I"
  $X "$HERE/ref_pi.py" --pi-root "$PI_ROOT" selfcheck --out "$WORK/out/selfcheck_crosscheck.json"
  $X "$HERE/run_matrix.py" --pi-root "$PI_ROOT" --fixtures "$WORK/fixtures" --out "$WORK/out/matrix-crosscheck"
  $PY "$HERE/compare_runs.py" "$WORK/out/matrix" "$WORK/out/matrix-crosscheck" --out "$WORK/out/cross_runtime.json" >/dev/null
fi

echo "done: $WORK/out"
echo "to refresh the committed goldens:  cp -r \$WORK/out/{selfcheck.json,species_list_e.json,species_list_allweeks_compact.json,onnx_vs_tflite.json,variants_compare.json,silence_probe.json,arrays.sha256} $HERE/golden/ ; cp \$WORK/out/matrix/*.json $HERE/golden/matrix/"
