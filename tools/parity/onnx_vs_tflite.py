#!/usr/bin/env python3
"""Is birdnet-go's V2.4 ONNX a faithful stand-in for BirdNET-Pi's V2.4 FP16 TFLite model?

    python -I onnx_vs_tflite.py --onnx MODEL.onnx --onnx-labels labels.txt \
        --pi-labels BirdNET_GLOBAL_6K_V2.4_Model_FP16_Labels.txt \
        --arrays OUT/arrays [--tflite-model BirdNET_GLOBAL_6K_V2.4_Model_FP16.tflite] --out report.json

``--arrays`` is the directory run_matrix.py wrote: ``<clip>.chunks.npy`` is the
exact float32 input BirdNET-Pi fed its interpreter, ``<clip>.logits.npy`` the raw
TFLite output.  The ONNX model is run on those very chunks, one at a time
(batch 1), and the raw logits are compared.  Nothing here is rounded: the
report carries the numbers as measured.

With ``--tflite-model`` it also runs a *sensitivity probe* on the TFLite model
itself: perturb each chunk by (a) a relative 6e-8 (float32 rounding) and (b)
uniform noise of +-0.5 LSB (1/65536 full scale), the size of the disagreement two
correct 16-bit pipelines can have, and report how far the logits move, per clip.
That is the floor for any cross-implementation tolerance.
"""

from __future__ import annotations

import argparse
import glob
import json
import os
import sys

import numpy as np


def sigmoid(x, slope):
    return 1.0 / (1.0 + np.exp(-slope * x.astype(np.float64)))


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--onnx", required=True)
    ap.add_argument("--onnx-labels", required=True)
    ap.add_argument("--pi-labels", required=True)
    ap.add_argument("--arrays", required=True)
    ap.add_argument("--tflite-model")
    ap.add_argument("--slope", type=float, default=0.75, help="sigmoid slope for score-level metrics (SENSITIVITY=1.25 -> 0.75)")
    ap.add_argument("--threshold", type=float, default=0.7)
    ap.add_argument("--out", required=True)
    a = ap.parse_args(argv)

    import hashlib
    import onnxruntime as ort

    with open(a.onnx_labels, encoding="utf-8") as f:
        onnx_sci = [l.rstrip("\n").split("_")[0] for l in f]
    with open(a.pi_labels, encoding="utf-8") as f:
        pi_sci = [l.strip().split("_")[0] for l in f]
    labels_same = onnx_sci == pi_sci

    so = ort.SessionOptions()
    so.log_severity_level = 3
    sess = ort.InferenceSession(a.onnx, so, providers=["CPUExecutionProvider"])
    in_name = sess.get_inputs()[0].name
    out_name = sess.get_outputs()[0].name

    report = {
        "onnx_sha256": hashlib.sha256(open(a.onnx, "rb").read()).hexdigest(),
        "onnxruntime": ort.__version__,
        "onnx_input": [in_name, sess.get_inputs()[0].shape],
        "onnx_output": [out_name, sess.get_outputs()[0].shape],
        "labels_scientific_names_identical_and_same_order": labels_same,
        "slope_used_for_score_metrics": a.slope,
        "threshold_used": a.threshold,
        "clips": {},
    }

    tot = {"n_chunks": 0, "n_values": 0, "sum_abs": 0.0, "max_abs": 0.0, "top1_agree": 0, "thr_flips": 0, "thr_pairs": 0, "score_max": 0.0}
    for chunk_path in sorted(glob.glob(os.path.join(a.arrays, "*.chunks.npy"))):
        cid = os.path.basename(chunk_path)[: -len(".chunks.npy")]
        chunks = np.load(chunk_path)
        ref = np.load(os.path.join(a.arrays, cid + ".logits.npy"))
        if len(chunks) == 0:
            report["clips"][cid] = {"n_chunks": 0}
            continue
        got = np.concatenate([sess.run([out_name], {in_name: c[None, :].astype(np.float32)})[0] for c in chunks])
        d = np.abs(got.astype(np.float64) - ref.astype(np.float64))
        top1_ref = ref.argmax(1)
        top1_got = got.argmax(1)
        agree = int((top1_ref == top1_got).sum())
        s_ref = sigmoid(ref, a.slope)
        s_got = sigmoid(got, a.slope)
        # restrict "which detections would be reported" to each side's own top-10 per chunk
        flips = pairs = 0
        for i in range(len(chunks)):
            cand = set(np.argsort(-ref[i])[:10]) | set(np.argsort(-got[i])[:10])
            for j in cand:
                r_hit, g_hit = s_ref[i, j] >= a.threshold, s_got[i, j] >= a.threshold
                if r_hit or g_hit:
                    pairs += 1
                    flips += int(r_hit != g_hit)
        # worst class-level disagreement among each chunk's TFLite top-10
        top10_max = max(float(np.max(d[i, np.argsort(-ref[i])[:10]])) for i in range(len(chunks)))
        c = {
            "n_chunks": int(len(chunks)),
            "logit_max_abs": float(d.max()),
            "logit_mean_abs": float(d.mean()),
            "logit_p99_abs": float(np.percentile(d, 99)),
            "logit_max_abs_within_tflite_top10": top10_max,
            "score_max_abs": float(np.abs(s_ref - s_got).max()),
            "top1_agree": agree,
            "top1_total": int(len(chunks)),
            "top1_disagreements": [
                {"chunk": int(i), "tflite": pi_sci[top1_ref[i]], "onnx": pi_sci[top1_got[i]] if labels_same else int(top1_got[i])}
                for i in np.nonzero(top1_ref != top1_got)[0]
            ],
            "threshold_pairs_hit_by_either": pairs,
            "threshold_pairs_where_only_one_hits": flips,
        }
        report["clips"][cid] = c
        tot["n_chunks"] += len(chunks)
        tot["n_values"] += d.size
        tot["sum_abs"] += float(d.sum())
        tot["max_abs"] = max(tot["max_abs"], c["logit_max_abs"])
        tot["top1_agree"] += agree
        tot["thr_flips"] += flips
        tot["thr_pairs"] += pairs
        tot["score_max"] = max(tot["score_max"], c["score_max_abs"])
    report["overall"] = {
        "n_chunks": tot["n_chunks"],
        "logit_max_abs": tot["max_abs"],
        "logit_mean_abs": tot["sum_abs"] / max(tot["n_values"], 1),
        "score_max_abs": tot["score_max"],
        "top1_agree": tot["top1_agree"],
        "top1_total": tot["n_chunks"],
        "threshold_pairs_hit_by_either": tot["thr_pairs"],
        "threshold_pairs_where_only_one_hits": tot["thr_flips"],
    }

    if a.tflite_model:
        try:
            from ai_edge_litert.interpreter import Interpreter
        except ImportError:
            from tensorflow.lite import Interpreter  # type: ignore
        it = Interpreter(a.tflite_model)
        it.allocate_tensors()
        i_idx = it.get_input_details()[0]["index"]
        o_idx = it.get_output_details()[0]["index"]

        def run(x):
            it.set_tensor(i_idx, x[None, :].astype(np.float32))
            it.invoke()
            return it.get_tensor(o_idx)[0].copy()

        levels = {
            # what two correct float32 implementations of the same linear step can differ by
            "relative_6e-8": lambda ch, rs: (ch * (1.0 + (rs.random_sample(ch.shape) - 0.5) * 1.2e-7)).astype(np.float32),
            # what two correct 16-bit pipelines (different mixers/resamplers/rounding) can differ by
            "abs_half_lsb": lambda ch, rs: ch + ((rs.random_sample(ch.shape) - 0.5) / 32768.0).astype(np.float32),
        }
        probe = {
            "what": "TFLite on BirdNET-Pi's own chunks + a tiny deterministic perturbation (RandomState(1)), compared with the "
            "unperturbed TFLite logits of the same chunks. Levels: relative_6e-8 = every sample times (1 +- 6e-8); "
            "abs_half_lsb = uniform noise in +-0.5 LSB of 16-bit full scale (1/65536).",
            "levels": {},
        }
        for lname, fn in levels.items():
            rs = np.random.RandomState(1)
            per = {}
            for chunk_path in sorted(glob.glob(os.path.join(a.arrays, "*.chunks.npy"))):
                cid = os.path.basename(chunk_path)[: -len(".chunks.npy")]
                chunks = np.load(chunk_path)
                ref = np.load(os.path.join(a.arrays, cid + ".logits.npy"))
                if len(chunks) == 0:
                    continue
                maxs, means, smax, flips, pairs = [], [], [], 0, 0
                for i, ch in enumerate(chunks):
                    out = run(fn(ch, rs))
                    dd = np.abs(out.astype(np.float64) - ref[i].astype(np.float64))
                    maxs.append(float(dd.max()))
                    means.append(float(dd.mean()))
                    sr, sg = sigmoid(ref[i], a.slope), sigmoid(out, a.slope)
                    smax.append(float(np.abs(sr - sg).max()))
                    for j in set(np.argsort(-ref[i])[:10]) | set(np.argsort(-out)[:10]):
                        if sr[j] >= a.threshold or sg[j] >= a.threshold:
                            pairs += 1
                            flips += int((sr[j] >= a.threshold) != (sg[j] >= a.threshold))
                per[cid] = {
                    "n_chunks": len(maxs),
                    "logit_max_abs_worst_chunk": max(maxs),
                    "logit_mean_abs": float(np.mean(means)),
                    "score_max_abs_worst_chunk": max(smax),
                    "threshold_pairs_hit_by_either": pairs,
                    "threshold_pairs_where_only_one_hits": flips,
                }
            probe["levels"][lname] = per
        report["tflite_perturbation_probe"] = probe

    with open(a.out, "w", encoding="utf-8") as f:
        json.dump(report, f, indent=1)
        f.write("\n")
    o = report["overall"]
    print(json.dumps({"labels_same": labels_same, **o}, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main())
