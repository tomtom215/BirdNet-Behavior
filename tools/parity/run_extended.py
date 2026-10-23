#!/usr/bin/env python3
"""BirdNET-Pi's own output for the extended clip set (`fixtures_extended.json`).

    python -I run_extended.py --pi-root PI --fixtures DIR --out DIR [--manifest FILE]

Three configurations per clip, each at the clip's own station (latitude, longitude) and recording
date from the manifest (the ISO week BirdNET-Pi derives from the file name is the recording's):

  x0_pi_default   BirdNET-Pi as installed: MData v1, SF_THRESH 0.03, CONFIDENCE 0.7, SENSITIVITY 1.25, ISO week
  x1_pi_v2_iso    the same with DATA_MODEL_VERSION=2 (the occurrence filter this project ships)
  x2_pi_v2_48wk   v2 and the model's own 48-week-year week of the date (what this project asks the filter)

`x0` carries every chunk's top 10 with raw logits (the schema `compare_rust.py` reads) and the
privacy masks; `x1` and `x2` carry detections only.  `OUT/arrays/` receives each clip's raw TFLite
logits and its first three float32 input chunks (the scratch `.npy`, not kept in the tree).
"""
from __future__ import annotations

import argparse
import datetime as dt
import importlib.util
import json
import os
import sys
import time

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
sys.dont_write_bytecode = True


def load_ref_pi():
    spec = importlib.util.spec_from_file_location("ref_pi", os.path.join(HERE, "ref_pi.py"))
    mod = importlib.util.module_from_spec(spec)
    sys.modules["ref_pi"] = mod
    spec.loader.exec_module(mod)
    return mod


def birdnet_week(month: int, day: int) -> int:
    """The model's 48-week year: four weeks a month, days 29-31 in week 4 (`civil::birdnet_week`)."""
    return (month - 1) * 4 + min((day - 1) // 7 + 1, 4)


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--pi-root", required=True)
    ap.add_argument("--fixtures", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--manifest", default=os.path.join(HERE, "fixtures_extended.json"))
    ap.add_argument("--as-year", type=int, help="pretend every recording was made in this year (month, day and time kept). "
                    "This project quarantines a recording dated before 2024, so the real-service comparison names its files "
                    "in 2026; the reference must be asked about the same dates")
    a = ap.parse_args(argv)

    rp = load_ref_pi()
    man = json.load(open(a.manifest, encoding="utf-8"))
    ref = rp.PiRef(a.pi_root)
    os.makedirs(os.path.join(a.out, "arrays"), exist_ok=True)
    t0 = time.time()
    configs = [
        ("x0_pi_default", {}, False),
        ("x1_pi_v2_iso", {"DATA_MODEL_VERSION": 2}, True),
        ("x2_pi_v2_48wk", {"DATA_MODEL_VERSION": 2}, True),
    ]
    for name, extra, slim in configs:
        runs = []
        for c in man["clips"]:
            st = c["station"]
            fdate = dt.datetime.strptime(st["date"], "%Y-%m-%d %H:%M:%S")
            if a.as_year:
                fdate = fdate.replace(year=a.as_year, day=min(fdate.day, 28) if fdate.month == 2 else fdate.day)
            cfg = {"LATITUDE": st["lat"], "LONGITUDE": st["lon"], **extra}
            kw = {"file_date": fdate}
            if name == "x2_pi_v2_48wk":
                # BirdNET-Pi reads the week from the file name; to ask its code the 48-week question,
                # name the file for the ISO week whose number equals it (any year with a week that many).
                kw = {"week": birdnet_week(fdate.month, fdate.day), "year": 2026}
            res = ref.analyze(os.path.join(a.fixtures, c["id"] + ".wav"), cfg, keep_arrays=(name == "x0_pi_default"), **kw)
            if name == "x0_pi_default":
                arrs = res.pop("_arrays")
                np.save(os.path.join(a.out, "arrays", c["id"] + ".logits.npy"), arrs["logits"])
                np.save(os.path.join(a.out, "arrays", c["id"] + ".chunks3.npy"), arrs["chunks"][:3])
            res["clip"]["id"] = c["id"]
            res["clip"]["fixture_sha256_pinned"] = c.get("sha256")
            res["run"]["station"] = st
            res["run"]["tags"] = c.get("tags", [])
            if slim:
                res.pop("chunks", None)
            runs.append(res)
        rp.dump_json({"schema": rp.SCHEMA, "config_name": name, "slim": slim, "runs": runs}, os.path.join(a.out, name + ".json"))
        print(f"{time.time() - t0:7.1f}s {name} ({len(runs)} clips)", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
