#!/usr/bin/env python3
"""Check that the comparison in convert_mdata.py can actually fail.

A fidelity report that has only ever printed "no differences" proves nothing
about what it would catch.  This script breaks a known-good ONNX on purpose,
runs the same grid through TFLite and the broken copy, and prints what the
comparison saw:

  * week upper bound 49 -> 50   (week 49 must now differ from TFLite)
  * the 6522-wide output bias shifted by +1e-3, +1e-4, +1e-5, +1e-6

Usage:
    python -I sensitivity_check.py --tflite A.tflite --onnx GOOD.onnx --mutant-dir DIR

DIR must be an empty scratch directory with room for five copies of the model
(use a tmpfs if the disk is tight); the mutants are left there for inspection.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))  # our own sibling module

import numpy as np  # noqa: E402
import onnx  # noqa: E402
from onnx import numpy_helper  # noqa: E402

import convert_mdata as C  # noqa: E402


def set_scalar(model, old: float, new: float) -> None:
    hit = 0
    for init in model.graph.initializer:
        a = numpy_helper.to_array(init)
        if a.shape == () and abs(float(a) - old) < 1e-9:
            init.CopyFrom(numpy_helper.from_array(np.array(new, dtype=a.dtype), init.name))
            hit += 1
    assert hit >= 1, f"no scalar initializer equal to {old}"


def shift_output_bias(model, delta: float) -> None:
    for init in model.graph.initializer:
        a = numpy_helper.to_array(init)
        if a.shape == (C.N_SPECIES,):
            init.CopyFrom(numpy_helper.from_array((a + np.float32(delta)).astype(np.float32), init.name))
            return
    raise SystemExit("no 6522-wide initializer (output bias) found")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--tflite", required=True)
    ap.add_argument("--onnx", required=True, help="the known-good ONNX made from --tflite")
    ap.add_argument("--mutant-dir", required=True)
    a = ap.parse_args()

    out = Path(a.mutant_dir)
    out.mkdir(parents=True, exist_ok=True)
    grid = [(la, lo, w) for la in C.GRID_LAT for lo in C.GRID_LON for w in C.GRID_WEEK]
    ref = C.run_grid(C.Tflite(Path(a.tflite)), grid)

    cases = [("mutant_week_upper_bound_49_to_50", lambda m: set_scalar(m, 49.0, 50.0))]
    for d in (1e-3, 1e-4, 1e-5, 1e-6):
        cases.append((f"mutant_bias_plus_{d:g}", lambda m, d=d: shift_output_bias(m, d)))

    for name, mutate in cases:
        m = onnx.load(a.onnx)
        mutate(m)
        p = out / f"{name}.onnx"
        onnx.save(m, str(p))
        r = C.compare("tflite", ref, name, C.run_grid(C.Onnx(p), grid), grid)
        t1, t3 = r["thresholds"]["0.01"], r["thresholds"]["0.03"]
        worst_weeks = {w: f"{v:.1e}" for w, v in r["max_abs_by_week"].items() if v > 1e-4}
        print(
            f"{name}: max_abs={r['max_abs']:.2e} mean_abs={r['mean_abs']:.2e} "
            f"crossing_cells@0.01={t1['cells_total']} @0.03={t3['cells_total']} "
            f"calls_with_different_count@0.03={t3['calls_count_differs']} "
            f"weeks_with_max_abs>1e-4={worst_weeks}"
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
