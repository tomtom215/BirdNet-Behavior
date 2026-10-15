#!/usr/bin/env python3
"""Which V2.4 build is a faithful stand-in for the FP16 TFLite model BirdNET-Pi runs?

    python -I variants_compare.py --arrays OUT/matrix/arrays --pi-labels LABELS.txt \
        --tflite-fp16 FP16.tflite --tflite-fp32 FP32.tflite \
        --onnx-dfttrunc A.onnx --onnx-full B.onnx --onnx-int8 C.onnx \
        [--cache DIR] --out variants_compare.json

Variants (letters used in the pair names):
  a  BirdNET-Pi's FP16 TFLite (the reference; its logits are read from <clip>.logits.npy)
  b  FP32 TFLite (birdnet-go's copy)
  c  BirdNET_v2.4_fp32_dfttrunc.onnx
  d  BirdNET_v2.4_fp32.onnx (full, non-truncated DFT)
  e  BirdNET_v2.4_int8_arm.onnx

All five see the same float32 chunks BirdNET-Pi produced (<clip>.chunks.npy), batch 1.
Per pair and per chunk group it reports max/mean |logit difference|, top-1 agreement and the
number of (chunk, label) pairs, over ALL 6522 labels, whose scaled score
sigmoid(slope * logit) is >= T on one side and < T on the other (T = 0.5, 0.7, 0.9; slope 0.75
= SENSITIVITY 1.25).  Chunk groups are assigned mechanically from the samples:
  digital_silence  all samples exactly 0
  zero_tail        >= 0.5 s of exactly-zero samples at the end (splitSignal's padding)
  band_limited     chunk of a clip whose native rate is <= 22050 Hz
  near_silence     RMS < 1e-4 (not all zero)
  ordinary         everything else
"""

from __future__ import annotations

import argparse
import glob
import json
import os
import sys

import numpy as np

SLOPE = 0.75
THRESHOLDS = (0.5, 0.7, 0.9)
PAIRS = ("a-b", "b-c", "b-d", "a-c", "a-d", "a-e", "c-d", "b-e")
GROUPS = {
    "all": None,
    "ordinary": ("ordinary",),
    "zero_tail+digital_silence": ("zero_tail", "digital_silence"),
    "band_limited": ("band_limited",),
    "near_silence": ("near_silence",),
}
HERE = os.path.dirname(os.path.abspath(__file__))


def sig(x):
    return 1.0 / (1.0 + np.exp(-SLOPE * x.astype(np.float64)))


def classify(chunk: np.ndarray, clip_rate: int) -> str:
    if not chunk.any():
        return "digital_silence"
    nz = np.nonzero(chunk)[0]
    if len(chunk) - 1 - nz[-1] >= 24000:
        return "zero_tail"
    if clip_rate <= 22050:
        return "band_limited"
    if float(np.sqrt(np.mean(chunk.astype(np.float64) ** 2))) < 1e-4:
        return "near_silence"
    return "ordinary"


def stats(x: np.ndarray, y: np.ndarray) -> dict:
    d = np.abs(x.astype(np.float64) - y.astype(np.float64))
    sx, sy = sig(x), sig(y)
    out = {
        "n_chunks": int(len(x)),
        "max_abs": float(d.max()),
        "mean_abs": float(d.mean()),
        "top1_agree": int((x.argmax(1) == y.argmax(1)).sum()),
    }
    for t in THRESHOLDS:
        out[f"cross_{t}"] = int(((sx >= t) != (sy >= t)).sum())
        out[f"either_{t}"] = int(((sx >= t) | (sy >= t)).sum())
    return out


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--arrays", required=True)
    ap.add_argument("--pi-labels", required=True)
    ap.add_argument("--tflite-fp16", required=True)
    ap.add_argument("--tflite-fp32", required=True)
    ap.add_argument("--onnx-dfttrunc", required=True)
    ap.add_argument("--onnx-full", required=True)
    ap.add_argument("--onnx-int8", required=True)
    ap.add_argument("--cache", help="directory for <clip>.<letter>.logits.npy (reused if present)")
    ap.add_argument("--out", required=True)
    a = ap.parse_args(argv)

    import hashlib

    import onnxruntime as ort
    try:
        from ai_edge_litert.interpreter import Interpreter
    except ImportError:
        from tensorflow.lite import Interpreter  # type: ignore

    with open(os.path.join(HERE, "fixtures.json"), encoding="utf-8") as f:
        rate_of = {c["id"]: c["rate"] for c in json.load(f)["clips"]}
    labels = [l.strip().split("_")[0] for l in open(a.pi_labels, encoding="utf-8")]
    pica = labels.index("Pica pica")

    files = {
        "b": a.tflite_fp32,
        "c": a.onnx_dfttrunc,
        "d": a.onnx_full,
        "e": a.onnx_int8,
        "a": a.tflite_fp16,
    }
    so = ort.SessionOptions()
    so.log_severity_level = 3
    runners = {}
    it = Interpreter(a.tflite_fp32)
    it.allocate_tensors()
    ii, oi = it.get_input_details()[0]["index"], it.get_output_details()[0]["index"]

    def run_tflite(x):
        it.set_tensor(ii, x[None, :].astype(np.float32))
        it.invoke()
        return it.get_tensor(oi)[0].copy()

    runners["b"] = run_tflite
    for k in ("c", "d", "e"):
        s = ort.InferenceSession(files[k], so, providers=["CPUExecutionProvider"])
        runners[k] = (lambda s: lambda x: s.run(["output"], {"input": x[None, :].astype(np.float32)})[0][0])(s)

    if a.cache:
        os.makedirs(a.cache, exist_ok=True)

    logits: dict[str, dict[str, np.ndarray]] = {k: {} for k in "abcde"}
    rows = []
    for cp in sorted(glob.glob(os.path.join(a.arrays, "*.chunks.npy"))):
        cid = os.path.basename(cp)[: -len(".chunks.npy")]
        chunks = np.load(cp)
        if len(chunks) == 0:
            continue
        logits["a"][cid] = np.load(os.path.join(a.arrays, cid + ".logits.npy"))
        for k in "bcde":
            cache = os.path.join(a.cache, f"{cid}.{k}.logits.npy") if a.cache else None
            if cache and os.path.exists(cache):
                logits[k][cid] = np.load(cache)
            else:
                logits[k][cid] = np.stack([runners[k](c) for c in chunks]).astype(np.float32)
                if cache:
                    np.save(cache, logits[k][cid])
        for i, c in enumerate(chunks):
            rows.append({"clip": cid, "i": i, "group": classify(c, rate_of[cid])})

    def pick(k, keep):
        parts = [logits[k][r["clip"]][r["i"]] for r in rows if keep(r)]
        return np.stack(parts)

    report = {
        "slope": SLOPE,
        "thresholds": THRESHOLDS,
        "files": {
            k: {"name": os.path.basename(p), "sha256": hashlib.sha256(open(p, "rb").read()).hexdigest()}
            for k, p in files.items()
        },
        "onnxruntime": ort.__version__,
        "n_chunks": len(rows),
        "group_sizes": {g: sum(1 for r in rows if (v is None or r["group"] in v)) for g, v in GROUPS.items()},
        "pairs": {},
    }
    for pair in PAIRS:
        x, y = pair.split("-")
        report["pairs"][pair] = {}
        for g, v in GROUPS.items():
            keep = (lambda r: True) if v is None else (lambda r, v=v: r["group"] in v)
            if not any(keep(r) for r in rows):
                continue
            report["pairs"][pair][g] = stats(pick(x, keep), pick(y, keep))

    # per-chunk max-abs for the pairs that matter
    for r in rows:
        r["max_abs"] = {
            p: float(np.abs(logits[p[0]][r["clip"]][r["i"]].astype(np.float64) - logits[p[2]][r["clip"]][r["i"]]).max())
            for p in PAIRS
        }
    report["chunks"] = rows

    # Pica pica windows t=3, 12, 21 s (chunks 1, 4, 7) at SENSITIVITY 1.25
    report["pica_windows"] = {
        "expected_pi_rounded": [0.912, 0.9316, 0.8857],
        "scaled_score_pica_pica": {
            k: [float(sig(logits[k]["pica_pica_30s"][i][pica : pica + 1])[0]) for i in (1, 4, 7)] for k in "abcde"
        },
        "logit_pica_pica": {k: [float(logits[k]["pica_pica_30s"][i][pica]) for i in (1, 4, 7)] for k in "abcde"},
    }

    with open(a.out, "w", encoding="utf-8") as f:
        # one pair per line to keep the file diffable
        f.write("{\n")
        keys = list(report)
        for n, k in enumerate(keys):
            if k == "pairs":
                f.write(' "pairs": {\n')
                for m, (pair, gs) in enumerate(report["pairs"].items()):
                    f.write(f'  "{pair}": {{\n')
                    for q, (g, st) in enumerate(gs.items()):
                        f.write(f'   "{g}": {json.dumps(st, separators=(",", ":"))}' + (",\n" if q < len(gs) - 1 else "\n"))
                    f.write("  }" + (",\n" if m < len(report["pairs"]) - 1 else "\n"))
                f.write(" }")
            elif k == "chunks":
                f.write(' "chunks": [\n' + ",\n".join("  " + json.dumps(r, separators=(",", ":")) for r in rows) + "\n ]")
            else:
                f.write(f' "{k}": {json.dumps(report[k], separators=(",", ":"))}')
            f.write(",\n" if n < len(keys) - 1 else "\n")
        f.write("}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
