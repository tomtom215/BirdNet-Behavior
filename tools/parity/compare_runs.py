#!/usr/bin/env python3
"""Compare two run_matrix.py output directories (e.g. made with different runtime stacks).

    python -I compare_runs.py DIR_A DIR_B [--out summary.json]

Reports, per clip, the largest |difference| between the saved 48 kHz float32 chunks and between
the raw TFLite logits, and, across every config/clip run, how many runs differ in their final
detections, MData species list (count + sha256), per-chunk top-10 label order, and the largest
per-chunk top-10 score difference.  Needs numpy only.
"""

from __future__ import annotations

import argparse
import glob
import json
import os
import sys

import numpy as np


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("a")
    ap.add_argument("b")
    ap.add_argument("--out")
    a = ap.parse_args(argv)

    arrays = {}
    for f in sorted(glob.glob(os.path.join(a.a, "arrays", "*.npy"))):
        n = os.path.basename(f)
        g = os.path.join(a.b, "arrays", n)
        if not os.path.exists(g):
            arrays[n] = "missing in B"
            continue
        x, y = np.load(f), np.load(g)
        if x.shape != y.shape:
            arrays[n] = f"shape {x.shape} vs {y.shape}"
        elif x.size == 0:
            arrays[n] = {"empty": True}
        else:
            d = np.abs(x.astype(np.float64) - y.astype(np.float64))
            arrays[n] = {"max_abs": float(d.max()), "mean_abs": float(d.mean())}

    runs = diff_det = diff_mdata = diff_order = 0
    max_score = 0.0
    details = []
    for f in sorted(glob.glob(os.path.join(a.a, "*.json"))):
        n = os.path.basename(f)
        g = os.path.join(a.b, n)
        if not os.path.exists(g):
            details.append(f"{n}: missing in B")
            continue
        ja, jb = json.load(open(f)), json.load(open(g))
        for ra, rb in zip(ja["runs"], jb["runs"]):
            runs += 1
            da = [(d["start"], d["stop"], d["scientific_name"], d["confidence"]) for d in ra["detections"]]
            db = [(d["start"], d["stop"], d["scientific_name"], d["confidence"]) for d in rb["detections"]]
            if da != db:
                diff_det += 1
                details.append(f"{n} {ra['clip']['id']}: detections differ")
            ma, mb = ra["mdata"], rb["mdata"]
            if (ma["count"], ma["sorted_names_sha256"]) != (mb["count"], mb["sorted_names_sha256"]):
                diff_mdata += 1
                details.append(f"{n} {ra['clip']['id']}: MData list differs")
            for ca, cb in zip(ra.get("chunks", []), rb.get("chunks", [])):
                if [l for l, _ in ca["top10"]] != [l for l, _ in cb["top10"]]:
                    diff_order += 1
                max_score = max(max_score, max(abs(x[1] - y[1]) for x, y in zip(ca["top10"], cb["top10"])))
    summary = {
        "a": os.path.basename(os.path.normpath(a.a)),
        "b": os.path.basename(os.path.normpath(a.b)),
        "runs_compared": runs,
        "runs_with_different_detections": diff_det,
        "runs_with_different_mdata_list": diff_mdata,
        "chunks_with_different_top10_label_order": diff_order,
        "max_abs_top10_score_difference": max_score,
        "arrays": arrays,
        "details": details,
    }
    txt = json.dumps(summary, indent=1)
    if a.out:
        with open(a.out, "w", encoding="utf-8") as f:
            f.write(txt + "\n")
    print(txt)
    return 0


if __name__ == "__main__":
    sys.exit(main())
