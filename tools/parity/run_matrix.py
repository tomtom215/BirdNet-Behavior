#!/usr/bin/env python3
"""Run ref_pi.py over every fixture clip for the whole configuration matrix.

    python -I run_matrix.py --pi-root PI_CHECKOUT --fixtures DIR --out DIR [--only NAME,NAME]

Writes ``OUT/<config>.json`` (one ``runs`` list, one entry per clip) and, for the
clips, ``OUT/arrays/<clip>.chunks.npy`` (BirdNET-Pi's own 48 kHz mono float32
chunks at OVERLAP=0, ``[n, 144000]``; the first three rows are the "first 3
chunks" the task asks for), ``<clip>.logits.npy`` (raw TFLite output, ``[n,
6522]``) and ``<clip>.ov1.logits.npy`` / ``<clip>.ov2.logits.npy``.

Configurations (all start from scripts/install_config.sh defaults):

  a0_shipped_literal    the defaults exactly: lat/lon 0.0 (the file's only fixed value)
  a1_station            defaults, but lat/lon/week = the clip's "station" in fixtures.json
  b_sens_{1.0,0.75,1.5} SENSITIVITY                                       (station)
  c_conf_{0.5,0.9}      CONFIDENCE                                        (station)
  d_overlap_{1.0,2.0}   OVERLAP                                           (station)
  e_{unset,london}_w{1,26,48,49,52,53}_v{1,2}
                        lat/lon (-1,-1) vs (51.48,-0.13) x ISO week x MData version;
                        "slim" (no per-chunk blocks: they equal a1's)
  f_priv_{0,1,10}       PRIVACY_THRESHOLD                                 (station)
  h_{exclude,include,whitelist}   species lists, on the Pica pica clip only
  i_sf_1.01_empty_list, i_sf_0.0_full_list   SF_THRESH extremes at lat/lon (0,0), Pica pica clip only
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
import sys
import time

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
sys.dont_write_bytecode = True  # do not litter tools/parity with __pycache__


def load_ref_pi():
    # `python -I` does not put this directory on sys.path, so load ref_pi.py by path.
    spec = importlib.util.spec_from_file_location("ref_pi", os.path.join(HERE, "ref_pi.py"))
    mod = importlib.util.module_from_spec(spec)
    sys.modules["ref_pi"] = mod
    spec.loader.exec_module(mod)
    return mod


LONDON = (51.48, -0.13)
UNSET = (-1.0, -1.0)
E_WEEKS = (1, 26, 48, 49, 52, 53)


def build_configs(clips):
    """Yield (name, per-clip function -> (cfg overrides, week, lists), slim, clip filter)."""
    st = {c["id"]: c["station"] for c in clips}

    def station(cid, **over):
        s = st[cid]
        cfg = {"LATITUDE": s["lat"], "LONGITUDE": s["lon"]}
        cfg.update(over)
        return cfg, s["week"], {}

    cfgs = []
    cfgs.append(("a0_shipped_literal", lambda cid: ({}, st[cid]["week"], {}), False, None))
    cfgs.append(("a1_station", lambda cid: station(cid), False, None))
    for v in (1.0, 0.75, 1.5):
        cfgs.append((f"b_sens_{v}", lambda cid, v=v: station(cid, SENSITIVITY=v), False, None))
    for v in (0.5, 0.9):
        cfgs.append((f"c_conf_{v}", lambda cid, v=v: station(cid, CONFIDENCE=v), False, None))
    for v in (1.0, 2.0):
        cfgs.append((f"d_overlap_{v}", lambda cid, v=v: station(cid, OVERLAP=v), False, None))
    for name, (lat, lon) in (("unset", UNSET), ("london", LONDON)):
        for wk in E_WEEKS:
            for ver in (1, 2):
                cfgs.append(
                    (
                        f"e_{name}_w{wk}_v{ver}",
                        lambda cid, lat=lat, lon=lon, wk=wk, ver=ver: (
                            {"LATITUDE": lat, "LONGITUDE": lon, "DATA_MODEL_VERSION": ver},
                            wk,
                            {},
                        ),
                        True,
                        None,
                    )
                )
    for v in (0, 1, 10):
        cfgs.append((f"f_priv_{v}", lambda cid, v=v: station(cid, PRIVACY_THRESHOLD=v), False, None))
    only_pica = {"pica_pica_30s"}
    cfgs.append(("h_exclude", lambda cid: (*station(cid)[:2], {"exclude": ["Pica pica"]}), False, only_pica))
    cfgs.append(("h_include", lambda cid: (*station(cid)[:2], {"include": ["Corvus corone", "Pica pica"]}), False, only_pica))
    # (0,0) filters Pica pica out (see a0); the whitelist rescues it.
    cfgs.append(("h_whitelist", lambda cid: ({}, st[cid]["week"], {"whitelist": ["Pica pica"]}), False, only_pica))
    # Occurrence-filter edge cases, Pica pica clip, (0,0) where the filter drops Pica pica (see a0):
    cfgs.append(("i_sf_1.01_empty_list", lambda cid: ({"SF_THRESH": 1.01}, st[cid]["week"], {}), False, only_pica))
    cfgs.append(("i_sf_0.0_full_list", lambda cid: ({"SF_THRESH": 0.0}, st[cid]["week"], {}), False, only_pica))
    return cfgs


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--pi-root", required=True)
    ap.add_argument("--fixtures", required=True, help="directory produced by fetch_fixtures.py")
    ap.add_argument("--out", required=True)
    ap.add_argument("--only", help="comma-separated config names")
    ap.add_argument("--year", type=int, default=2020, help="ISO year used to realise a week in the file name")
    a = ap.parse_args(argv)

    rp = load_ref_pi()
    with open(os.path.join(HERE, "fixtures.json"), encoding="utf-8") as f:
        man = json.load(f)
    clips = man["clips"]
    ref = rp.PiRef(a.pi_root)
    os.makedirs(os.path.join(a.out, "arrays"), exist_ok=True)

    wanted = set(a.only.split(",")) if a.only else None
    t0 = time.time()
    for name, fn, slim, restrict in build_configs(clips):
        if wanted and name not in wanted:
            continue
        runs = []
        for c in clips:
            cid = c["id"]
            if restrict and cid not in restrict:
                continue
            over, week, lists = fn(cid)
            clip_path = os.path.join(a.fixtures, cid + ".wav")
            res = ref.analyze(
                clip_path,
                over,
                week=week,
                year=a.year,
                include=lists.get("include", ()),
                exclude=lists.get("exclude", ()),
                whitelist=lists.get("whitelist", ()),
                keep_arrays=True,
            )
            arrs = res.pop("_arrays")
            ov = float(res["config"]["OVERLAP"])
            if name == "a1_station" or (ov in (1.0, 2.0) and name.startswith("d_overlap")):
                tag = "" if ov == 0.0 else f".ov{int(ov)}"
                np.save(os.path.join(a.out, "arrays", f"{cid}{tag}.logits.npy"), arrs["logits"])
                if ov == 0.0:
                    np.save(os.path.join(a.out, "arrays", f"{cid}.chunks.npy"), arrs["chunks"])
            res["clip"]["id"] = cid
            res["clip"]["fixture_sha256_pinned"] = c.get("sha256")
            res["run"]["station_basis"] = c["station"]["basis"]
            res["run"]["year_used_for_week"] = a.year
            if slim:
                res.pop("chunks", None)
            runs.append(res)
        rp.dump_json({"schema": rp.SCHEMA, "config_name": name, "slim": slim, "runs": runs}, os.path.join(a.out, name + ".json"))
        print(f"{time.time() - t0:7.1f}s {name} ({len(runs)} clips)", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
