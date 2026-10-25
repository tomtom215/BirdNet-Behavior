#!/usr/bin/env python3
"""How much does the choice of calendar and of range-filter model change the species list?

    python -I range_calendar.py --pi-root PI --onnx birdnet-v24-range-v2.onnx --out golden/range_calendar.json [YEAR ...]

For eight stations on five continents and every day of each YEAR (default 2024 2026 2028: a leap
year, a year whose ISO calendar has a week 53, a leap year) it asks BirdNET-Pi's own range models
(`BirdNET_GLOBAL_6K_V2.4_MData_Model{,_V2}_FP16.tflite`, run by `ai-edge-litert`) and this project's
ONNX conversion of the v2 model for the species they admit at SF_THRESH 0.03, under three set-ups:

  pi_default   MData v1 and the raw ISO week 1-53 of the date          (BirdNET-Pi as installed)
  pi_v2_iso    MData v2 and the raw ISO week                            (DATA_MODEL_VERSION=2)
  ours         range v2 ONNX and the model's own 48-week-year week      (a new install here)

and records, per station, on how many days two set-ups' lists differ and by how many species.
It also asserts, for every station-day, that the ONNX conversion returns exactly BirdNET-Pi's own
TFLite v2 list at the 48-week week (the two are compared as sets of label indices).
"""
import argparse
import datetime as dt
import json
import os

import numpy as np

STATIONS = {
    "London": (51.48, -0.13), "Ithaca": (42.48, -76.45), "Helsinki": (60.2, 24.9), "Sydney": (-33.87, 151.21),
    "Sao_Paulo": (-23.55, -46.63), "Nairobi": (-1.29, 36.82), "Hokkaido": (43.06, 141.35), "Austin": (30.27, -97.74),
}
THR = 0.03


def birdnet_week(m: int, d: int) -> int:
    return (m - 1) * 4 + min((d - 1) // 7 + 1, 4)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--pi-root", required=True)
    ap.add_argument("--onnx", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("years", nargs="*", type=int, default=[2024, 2026, 2028])
    a = ap.parse_args()

    import onnxruntime as ort
    from ai_edge_litert.interpreter import Interpreter

    model_dir = os.path.join(a.pi_root, "model")

    def tfl(name):
        it = Interpreter(os.path.join(model_dir, name))
        it.allocate_tensors()
        return it

    m1 = tfl("BirdNET_GLOBAL_6K_V2.4_MData_Model_FP16.tflite")
    m2 = tfl("BirdNET_GLOBAL_6K_V2.4_MData_Model_V2_FP16.tflite")
    ox = ort.InferenceSession(a.onnx)
    ox_in = ox.get_inputs()[0].name

    def run_tfl(it, lat, lon, wk):
        i = it.get_input_details()[0]["index"]
        o = it.get_output_details()[0]["index"]
        it.set_tensor(i, np.array([[lat, lon, wk]], dtype="float32"))
        it.invoke()
        return it.get_tensor(o)[0].copy()

    cache = {}

    def species(kind, lat, lon, wk):
        k = (kind, lat, lon, wk)
        if k not in cache:
            v = {"v1": lambda: run_tfl(m1, lat, lon, wk), "v2": lambda: run_tfl(m2, lat, lon, wk),
                 "ox": lambda: ox.run(None, {ox_in: np.array([[lat, lon, wk]], dtype="float32")})[0][0]}[kind]()
            cache[k] = frozenset(np.nonzero(v >= THR)[0].tolist())
        return cache[k]

    def summary(xs):
        xs = np.array(xs)
        return {"days_differ": int((xs > 0).sum()), "mean_species": round(float(xs.mean()), 2), "max_species": int(xs.max())}

    result = {"sf_thresh": THR, "stations": STATIONS, "years": {}}
    for y in a.years:
        days = [dt.date(y, 1, 1) + dt.timedelta(d) for d in range((dt.date(y + 1, 1, 1) - dt.date(y, 1, 1)).days)]
        per = {}
        for name, (lat, lon) in STATIONS.items():
            cal, ver, tot, pi_only, ours_only, sizes_pi, sizes_ours = [], [], [], [], [], [], []
            for d in days:
                wi, wb = d.isocalendar()[1], birdnet_week(d.month, d.day)
                pi, pi2 = species("v1", lat, lon, wi), species("v2", lat, lon, wi)
                ours, ours_tfl = species("ox", lat, lon, wb), species("v2", lat, lon, wb)
                assert ours == ours_tfl, f"ONNX range v2 and BirdNET-Pi's TFLite v2 disagree at {name} {d}"
                cal.append(len(pi2 ^ ours)); ver.append(len(pi ^ pi2)); tot.append(len(pi ^ ours))
                pi_only.append(len(pi - ours)); ours_only.append(len(ours - pi))
                sizes_pi.append(len(pi)); sizes_ours.append(len(ours))
            per[name] = {
                "days": len(days),
                "calendar_only__pi_v2_iso_vs_ours": summary(cal),
                "model_version_only__pi_v1_vs_pi_v2_at_iso_week": summary(ver),
                "pi_default_vs_ours__symmetric_difference": summary(tot),
                "pi_default_vs_ours__admitted_by_pi_only": summary(pi_only),
                "pi_default_vs_ours__admitted_by_ours_only": summary(ours_only),
                "list_size_pi_default": [min(sizes_pi), max(sizes_pi)],
                "list_size_ours": [min(sizes_ours), max(sizes_ours)],
            }
        result["years"][str(y)] = per
        print(y, "done", flush=True)
    with open(a.out, "w") as f:
        json.dump(result, f, indent=1)
        f.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
