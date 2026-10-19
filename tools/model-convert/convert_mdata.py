#!/usr/bin/env python3
"""Convert BirdNET V2.4 'MData' range-filter models from TFLite to ONNX and
measure how faithful the result is.

Three subcommands.  Every path is an argument; nothing is read from the
current directory or from the repository.

    convert  --tflite IN.tflite --onnx OUT.onnx [--opset 17]
        Convert one TFLite model with tf2onnx (needs the pinned
        tensorflow-cpu + tf2onnx, see README.md).

    fingerprint  --onnx FILE [FILE ...]
        Print a name-agnostic sha256 of the graph (op types, attributes,
        wiring, constant contents, input/output signature).  tf2onnx does
        not write byte-identical files from run to run (generated constant
        names change), so use this, not the file hash, to check that a
        regeneration produced the same model.

    report   --labels labels.txt
             --v1-tflite A --v1-onnx B --v2-tflite C --v2-onnx D
             [--extra-onnx NAME=PATH ...] --out-dir DIR
        Compare TFLite against ONNX on a fixed grid, tabulate the week
        semantics at one location, and compare the ISO-week and 48-week
        schemes for every day of a year.  Needs only numpy, onnxruntime
        and ai-edge-litert (no tensorflow).  Prints Markdown tables and
        writes results.json plus CSVs to --out-dir.

Everything here is deterministic: a fixed grid, single-threaded kernels,
no random numbers.

Run it as `python -I convert_mdata.py ...` from a directory you trust; the
data files it reads (models, labels) are treated as data only.
"""

from __future__ import annotations

import argparse
import csv
import datetime as dt
import hashlib
import importlib.metadata as md
import json
import sys
from pathlib import Path

# --- the grid fixed by the task ------------------------------------------------

GRID_LAT = [-90, -60, -33.9, -1, 0, 1, 19.4, 35.7, 48.86, 51.48, 60, 90]
GRID_LON = [-180, -122.4, -0.13, 2.35, 13.4, 139.7, 151.2, 180]
# Includes the out-of-range weeks on purpose: -1, 0 and 49..53.
GRID_WEEK = [-1, 0, 1, 2, 12, 24, 25, 26, 47, 48, 49, 50, 52, 53]
THRESHOLDS = [0.01, 0.03]

# Location used for the week-semantics and ISO-vs-48-week tables (London).
PROBE_LAT, PROBE_LON = 51.48, -0.13
PROBE_T = 0.03

N_SPECIES = 6522


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def version_of(pkg: str) -> str:
    try:
        return md.version(pkg)
    except md.PackageNotFoundError:
        return "not installed"


# --- convert ---------------------------------------------------------------------


def cmd_convert(a: argparse.Namespace) -> int:
    import onnx
    import tf2onnx.convert  # imports tensorflow; only `convert` needs it

    out = Path(a.onnx)
    out.parent.mkdir(parents=True, exist_ok=True)
    tf2onnx.convert.from_tflite(str(a.tflite), opset=a.opset, output_path=str(out))
    onnx.checker.check_model(str(out))
    m = onnx.load(str(out))

    def sig(v):
        t = v.type.tensor_type
        dims = [d.dim_value if d.HasField("dim_value") else d.dim_param for d in t.shape.dim]
        return f"{v.name} {onnx.TensorProto.DataType.Name(t.elem_type)} {dims}"

    print(f"wrote {out}  sha256={sha256_file(out)}  bytes={out.stat().st_size}")
    print("  opset:", [(o.domain or "ai.onnx", o.version) for o in m.opset_import])
    print("  inputs: ", [sig(v) for v in m.graph.input])
    print("  outputs:", [sig(v) for v in m.graph.output])
    return 0


def fingerprint(path: Path) -> str:
    """sha256 over the graph with every generated name replaced by position."""
    import onnx
    from onnx import numpy_helper

    m = onnx.load(str(path))
    g = m.graph
    consts = {i.name: numpy_helper.to_array(i) for i in g.initializer}

    def sig(v):
        t = v.type.tensor_type
        dims = [d.dim_value if d.HasField("dim_value") else "dyn" for d in t.shape.dim]
        return f"{onnx.TensorProto.DataType.Name(t.elem_type)}{dims}"

    h = hashlib.sha256()
    h.update(f"opsets={sorted((o.domain, o.version) for o in m.opset_import)}\n".encode())
    h.update(f"in={[sig(v) for v in g.input]} out={[sig(v) for v in g.output]}\n".encode())
    where = {v.name: f"in{k}" for k, v in enumerate(g.input) if v.name not in consts}
    for k, n in enumerate(g.node):
        parts = [n.op_type]
        parts += [f"{a.name}={onnx.helper.get_attribute_value(a)!r}" for a in sorted(n.attribute, key=lambda a: a.name)]
        for i in n.input:
            if i in consts:
                c = consts[i]
                parts.append(f"c:{c.dtype}:{c.shape}:{hashlib.sha256(c.tobytes()).hexdigest()}")
            else:
                parts.append("t:" + where[i])
        for j, o in enumerate(n.output):
            where[o] = f"n{k}.{j}"
        h.update(("|".join(parts) + "\n").encode())
    h.update(("outs=" + ",".join(where[v.name] for v in g.output) + "\n").encode())
    return h.hexdigest()


def cmd_fingerprint(a: argparse.Namespace) -> int:
    for p in a.onnx:
        print(f"{fingerprint(Path(p))}  {p}")
    return 0


# --- runners ---------------------------------------------------------------------


class Tflite:
    """One-sample-at-a-time TFLite runner (ai-edge-litert, single thread)."""

    def __init__(self, path: Path, reference_kernels: bool = False):
        from ai_edge_litert.interpreter import Interpreter, OpResolverType

        kind = OpResolverType.BUILTIN_REF if reference_kernels else OpResolverType.AUTO
        self.it = Interpreter(model_path=str(path), num_threads=1, experimental_op_resolver_type=kind)
        self.it.allocate_tensors()
        self.i = self.it.get_input_details()[0]["index"]
        self.o = self.it.get_output_details()[0]["index"]

    def __call__(self, lat, lon, week):
        import numpy as np

        self.it.set_tensor(self.i, np.array([[lat, lon, week]], dtype=np.float32))
        self.it.invoke()
        return self.it.get_tensor(self.o)[0].copy()


class Onnx:
    """One-sample-at-a-time ONNX Runtime runner (CPU EP, single thread)."""

    def __init__(self, path: Path):
        import onnxruntime as ort

        so = ort.SessionOptions()
        so.intra_op_num_threads = 1
        so.inter_op_num_threads = 1
        self.s = ort.InferenceSession(str(path), so, providers=["CPUExecutionProvider"])
        self.name = self.s.get_inputs()[0].name

    def __call__(self, lat, lon, week):
        import numpy as np

        r = self.s.run(None, {self.name: np.array([[lat, lon, week]], dtype=np.float32)})
        assert len(r) == 1, "model must have a single output"
        return r[0][0].copy()


def run_grid(model, grid):
    import numpy as np

    out = np.empty((len(grid), N_SPECIES), dtype=np.float32)
    for k, (lat, lon, week) in enumerate(grid):
        v = model(lat, lon, week)
        assert v.shape == (N_SPECIES,) and v.dtype == np.float32, (v.shape, v.dtype)
        out[k] = v
    return out


# --- fidelity (step 3) -------------------------------------------------------------


def compare(name_a, a, name_b, b, grid):
    """Compare two (calls x species) float32 arrays; `a` is the reference."""
    import numpy as np

    both = np.isfinite(a) & np.isfinite(b)
    d = np.where(both, np.abs(a.astype(np.float64) - b.astype(np.float64)), np.nan)
    flat = int(np.nanargmax(d))
    wk_i, sp_i = divmod(flat, N_SPECIES)
    res = {
        "a": name_a,
        "b": name_b,
        "calls": len(grid),
        "cells": int(a.size),
        "max_abs": float(np.nanmax(d)),
        "mean_abs": float(np.nanmean(d)),
        "max_abs_at": {"lat": grid[wk_i][0], "lon": grid[wk_i][1], "week": grid[wk_i][2], "species_idx": sp_i},
        "nan_a": int(np.isnan(a).sum()),
        "nan_b": int(np.isnan(b).sum()),
        "nonfinite_a": int((~np.isfinite(a)).sum()),
        "nonfinite_b": int((~np.isfinite(b)).sum()),
        "min_a": float(np.nanmin(a)),
        "max_a": float(np.nanmax(a)),
        "min_b": float(np.nanmin(b)),
        "max_b": float(np.nanmax(b)),
        "max_abs_by_week": {},
        "thresholds": {},
    }
    weeks = np.array([g[2] for g in grid])
    for w in sorted(set(weeks.tolist())):
        res["max_abs_by_week"][str(w)] = float(np.nanmax(d[weeks == w]))
    for t in THRESHOLDS:
        pa, pb = a >= t, b >= t  # NaN compares False
        a_only, b_only = pa & ~pb, ~pa & pb
        ca, cb = pa.sum(axis=1), pb.sum(axis=1)
        cross = np.argwhere(a_only | b_only)
        examples = [
            {
                "call": int(c),
                "lat": grid[c][0],
                "lon": grid[c][1],
                "week": grid[c][2],
                "species_idx": int(s),
                "a": float(a[c, s]),
                "b": float(b[c, s]),
            }
            for c, s in cross[:10]
        ]
        res["thresholds"][str(t)] = {
            "a_ge_T_b_lt_T": int(a_only.sum()),
            "a_lt_T_b_ge_T": int(b_only.sum()),
            "cells_total": int(a_only.sum() + b_only.sum()),
            # How many cells were close enough to T that a difference as large as the
            # observed maximum could have flipped them.  Zero crossings means little
            # when this is also zero.
            "cells_within_max_abs_of_T": int((np.abs(a.astype(np.float64) - t) <= res["max_abs"]).sum()),
            "closest_a_to_T": float(np.abs(a.astype(np.float64) - t).min()),
            "calls_with_any_crossing": int((a_only | b_only).any(axis=1).sum()),
            "count_a_mean": float(ca.mean()),
            "count_b_mean": float(cb.mean()),
            "count_a_min_max": [int(ca.min()), int(ca.max())],
            "count_b_min_max": [int(cb.min()), int(cb.max())],
            "calls_count_equal": int((ca == cb).sum()),
            "calls_count_differs": int((ca != cb).sum()),
            "max_abs_count_diff": int(np.abs(ca - cb).max()),
            "crossing_examples": examples,
        }
        res["thresholds"][str(t)]["_counts"] = (ca.tolist(), cb.tolist())
    return res


# --- week semantics (step 4) ---------------------------------------------------------


def passing_hash(labels, vec, t):
    idx = [i for i in range(N_SPECIES) if vec[i] >= t]
    names = sorted(labels[i] for i in idx)
    return len(idx), hashlib.sha256("\n".join(names).encode("utf-8")).hexdigest(), frozenset(idx)


# --- ISO vs 48-week (step 5) -------------------------------------------------------------


def week48(d: dt.date) -> int:
    return (d.month - 1) * 4 + min((d.day - 1) // 7 + 1, 4)


def md_table(header, rows):
    out = ["| " + " | ".join(header) + " |", "|" + "|".join("---" for _ in header) + "|"]
    out += ["| " + " | ".join(str(c) for c in r) + " |" for r in rows]
    return "\n".join(out)


def cmd_report(a: argparse.Namespace) -> int:
    import numpy as np

    out_dir = Path(a.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    labels = Path(a.labels).read_text(encoding="utf-8").split("\n")
    if labels and labels[-1] == "":
        labels.pop()
    assert len(labels) == N_SPECIES, f"expected {N_SPECIES} labels, got {len(labels)}"

    extras = {}
    for item in a.extra_onnx or []:
        name, _, path = item.partition("=")
        extras[name] = Path(path)

    results: dict = {
        "versions": {p: version_of(p) for p in ("numpy", "onnx", "onnxruntime", "ai-edge-litert", "tensorflow-cpu", "tf2onnx")}
        | {"python": sys.version.split()[0]},
        "files": {},
        "grid": {"lat": GRID_LAT, "lon": GRID_LON, "week": GRID_WEEK},
    }
    files = {"v1_tflite": a.v1_tflite, "v1_onnx": a.v1_onnx, "v2_tflite": a.v2_tflite, "v2_onnx": a.v2_onnx}
    files |= {f"extra_{k}": v for k, v in extras.items()}
    for k, v in files.items():
        v = Path(v)
        results["files"][k] = {"path": str(v), "sha256": sha256_file(v), "bytes": v.stat().st_size}
        if v.suffix == ".onnx":
            results["files"][k]["graph_fingerprint"] = fingerprint(v)

    grid = [(lat, lon, w) for lat in GRID_LAT for lon in GRID_LON for w in GRID_WEEK]
    assert len(grid) == 1344

    # ---- step 3 ----
    out_arrays = {}
    for tag, tfl, onx in (("v1", a.v1_tflite, a.v1_onnx), ("v2", a.v2_tflite, a.v2_onnx)):
        out_arrays[f"{tag}_tflite"] = run_grid(Tflite(Path(tfl)), grid)
        out_arrays[f"{tag}_tflite_ref"] = run_grid(Tflite(Path(tfl), reference_kernels=True), grid)
        out_arrays[f"{tag}_onnx"] = run_grid(Onnx(Path(onx)), grid)
    for k, p in extras.items():
        out_arrays[f"extra_{k}"] = run_grid(Onnx(p), grid)

    # The converted graphs declare a dynamic batch dimension; check that a batch of N
    # rows through ONNX Runtime equals N single-row calls (the app only ever uses N=1).
    results["batch_check"] = {}
    for tag, key in (("v1", "v1_onnx"), ("v2", "v2_onnx")):
        om = Onnx(Path(files[key]))
        rows = np.array(grid[::112], dtype=np.float32)  # 12 spread-out rows
        batched = om.s.run(None, {om.name: rows})[0]
        singles = np.stack([om(*map(float, r)) for r in rows])
        results["batch_check"][tag] = {"rows": len(rows), "out_shape": list(batched.shape), "max_abs_vs_single": float(np.abs(batched.astype(np.float64) - singles).max())}
    print("batch check (ONNX Runtime, N rows in one call vs N single calls):", json.dumps(results["batch_check"]))

    # Out-of-range weeks across all 96 grid locations: do 0 and 49..53 behave exactly like
    # -1 ("no seasonal information"), and do they differ from the in-range week they would
    # equal if the week index wrapped (49 -> 1, 50 -> 2)?
    results["out_of_range_weeks"] = {}
    iw = {w: k for k, w in enumerate(GRID_WEEK)}
    for key in ("v1_tflite", "v1_onnx", "v2_tflite", "v2_onnx"):
        arr = out_arrays[key].reshape(len(GRID_LAT), len(GRID_LON), len(GRID_WEEK), N_SPECIES)
        base = arr[:, :, iw[-1]]
        rec = {}
        for w in (0, 49, 50, 52, 53):
            rec[f"max_abs_week{w}_vs_week-1"] = float(np.abs(arr[:, :, iw[w]].astype(np.float64) - base).max())
        for w, wrapped in ((49, 1), (50, 2)):
            dd = np.abs(arr[:, :, iw[w]].astype(np.float64) - arr[:, :, iw[wrapped]]).max(axis=-1)
            rec[f"week{w}_vs_week{wrapped}_locations_max_abs_gt_1e-3"] = int((dd > 1e-3).sum())
        for w in (1, 2, 12, 24, 48):
            dd = np.abs(arr[:, :, iw[w]].astype(np.float64) - base).max(axis=-1)
            rec[f"week{w}_vs_week-1_locations_max_abs_gt_1e-3"] = int((dd > 1e-3).sum())
        results["out_of_range_weeks"][key] = rec
    print("## Step 3b: out-of-range weeks over the 12x8=96 grid locations (max abs over species and locations)\n")
    for key, rec in results["out_of_range_weeks"].items():
        print(f"- {key}: " + ", ".join(f"{k}={v:.2e}" if isinstance(v, float) else f"{k}={v}" for k, v in rec.items()))
    print()

    pairs = [
        ("v1 TFLite(xnnpack) vs v1 ONNX(ours)", "v1_tflite", "v1_onnx"),
        ("v2 TFLite(xnnpack) vs v2 ONNX(ours)", "v2_tflite", "v2_onnx"),
        ("v1 TFLite(xnnpack) vs v1 TFLite(ref kernels)  [noise floor]", "v1_tflite", "v1_tflite_ref"),
        ("v2 TFLite(xnnpack) vs v2 TFLite(ref kernels)  [noise floor]", "v2_tflite", "v2_tflite_ref"),
        ("v1 TFLite(ref kernels) vs v1 ONNX(ours)", "v1_tflite_ref", "v1_onnx"),
        ("v2 TFLite(ref kernels) vs v2 ONNX(ours)", "v2_tflite_ref", "v2_onnx"),
    ]
    for k in extras:
        pairs.append((f"v2 TFLite(xnnpack) vs {k} ONNX", "v2_tflite", f"extra_{k}"))
        pairs.append((f"v2 ONNX(ours) vs {k} ONNX", "v2_onnx", f"extra_{k}"))

    results["fidelity"] = []
    print("## Step 3: fidelity over 1344 calls x 6522 species\n")
    rows = []
    for title, ka, kb in pairs:
        r = compare(ka, out_arrays[ka], kb, out_arrays[kb], grid)
        r["title"] = title
        counts = {t: r["thresholds"][t].pop("_counts") for t in r["thresholds"]}
        with open(out_dir / f"grid_counts_{ka}__{kb}.csv", "w", newline="") as f:
            w = csv.writer(f)
            w.writerow(["lat", "lon", "week"] + [f"count_a_T{t}" for t in counts] + [f"count_b_T{t}" for t in counts])
            for i, g in enumerate(grid):
                w.writerow([*g] + [counts[t][0][i] for t in counts] + [counts[t][1][i] for t in counts])
        results["fidelity"].append(r)
        t1, t3 = r["thresholds"]["0.01"], r["thresholds"]["0.03"]
        rows.append(
            [
                title,
                f"{r['max_abs']:.3e}",
                f"{r['mean_abs']:.3e}",
                f"{t1['a_ge_T_b_lt_T']}/{t1['a_lt_T_b_ge_T']}",
                f"{t3['a_ge_T_b_lt_T']}/{t3['a_lt_T_b_ge_T']}",
                f"{t1['calls_count_differs']}",
                f"{t3['calls_count_differs']}",
                f"{r['nonfinite_a']}/{r['nonfinite_b']}",
            ]
        )
    print(
        md_table(
            [
                "pair (A vs B)",
                "max abs diff",
                "mean abs diff",
                "T=0.01 cells A>=T,B<T / A<T,B>=T",
                "T=0.03 cells A>=T,B<T / A<T,B>=T",
                "calls with differing count @0.01",
                "calls with differing count @0.03",
                "non-finite A/B",
            ],
            rows,
        )
    )
    print()
    for r in results["fidelity"]:
        print(f"- {r['title']}: max_abs at {r['max_abs_at']}")
        for t, v in r["thresholds"].items():
            print(
                f"    T={t}: count A mean {v['count_a_mean']:.2f} range {v['count_a_min_max']}, "
                f"B mean {v['count_b_mean']:.2f} range {v['count_b_min_max']}, "
                f"calls equal {v['calls_count_equal']}/1344, max |dcount| {v['max_abs_count_diff']}, "
                f"calls with any crossing {v['calls_with_any_crossing']}, "
                f"cells within max_abs of T {v['cells_within_max_abs_of_T']}, closest |A-T| {v['closest_a_to_T']:.2e}"
            )
            for ex in v["crossing_examples"][:3]:
                print(f"      crossing example: {ex}")
    print("\nmax abs diff by week (A=TFLite xnnpack, B=ONNX ours):")
    for r in results["fidelity"]:
        if r["a"].endswith("_tflite") and r["b"].endswith("_onnx"):
            print(f"  {r['title']}: " + ", ".join(f"{w}:{v:.2e}" for w, v in r["max_abs_by_week"].items()))

    # ---- step 4 ----
    print(f"\n## Step 4: week semantics at lat={PROBE_LAT} lon={PROBE_LON}, T={PROBE_T}\n")
    results["weeks"] = {}
    probe_weeks = [-1, 0] + list(range(1, 54))
    for tag, tfl, onx in (("v1", a.v1_tflite, a.v1_onnx), ("v2", a.v2_tflite, a.v2_onnx)):
        tf_m, on_m = Tflite(Path(tfl)), Onnx(Path(onx))
        table = []
        for w in probe_weeks:
            ct, ht, st = passing_hash(labels, tf_m(PROBE_LAT, PROBE_LON, w), PROBE_T)
            co, ho, so = passing_hash(labels, on_m(PROBE_LAT, PROBE_LON, w), PROBE_T)
            table.append({"week": w, "tflite_count": ct, "tflite_sha256": ht, "onnx_count": co, "onnx_sha256": ho, "same_list": st == so})
        # fractional / boundary probes, TFLite only
        bound = []
        for w in (0.5, 1.0, 48.0, 48.5, 48.999, 49.0, 49.001):
            c, h, _ = passing_hash(labels, tf_m(PROBE_LAT, PROBE_LON, w), PROBE_T)
            bound.append({"week": w, "count": c, "sha256": h})
        # group weeks by list hash (TFLite)
        groups: dict[str, list] = {}
        for r in table:
            groups.setdefault(r["tflite_sha256"], []).append(r["week"])
        results["weeks"][tag] = {"table": table, "boundary": bound, "groups": list(groups.values())}
        with open(out_dir / f"weeks_{tag}.csv", "w", newline="") as f:
            w = csv.writer(f)
            w.writerow(["week", "tflite_count", "tflite_sha256", "onnx_count", "onnx_sha256", "same_list"])
            for r in table:
                w.writerow([r["week"], r["tflite_count"], r["tflite_sha256"], r["onnx_count"], r["onnx_sha256"], r["same_list"]])
        print(f"### {tag} (TFLite value; ONNX column says whether the ONNX list is identical)\n")
        print(
            md_table(
                ["week", "n passing", "sha256 of sorted list (first 16)", "ONNX list identical"],
                [[r["week"], r["tflite_count"], r["tflite_sha256"][:16], r["same_list"]] for r in table],
            )
        )
        print("\nweeks sharing an identical TFLite passing list:")
        for g in groups.values():
            if len(g) > 1:
                print("  ", g)
        print(f"distinct lists: {len(groups)} over {len(probe_weeks)} week values")
        print("boundary probes (TFLite):", bound, "\n")

    # ---- step 5 ----
    print("## Step 5: ISO week vs 48-week scheme, every day of 2026\n")
    results["iso48"] = {}
    for tag, tfl in (("v1", a.v1_tflite), ("v2", a.v2_tflite)):
        tf_m = Tflite(Path(tfl))
        cache: dict = {}

        def passing(w):
            if w not in cache:
                cache[w] = passing_hash(labels, tf_m(PROBE_LAT, PROBE_LON, w), PROBE_T)[2]
            return cache[w]

        rows5 = []
        d = dt.date(2026, 1, 1)
        while d.year == 2026:
            iso, w48 = d.isocalendar()[1], week48(d)
            si, s48 = passing(iso), passing(w48)
            rows5.append({"date": d.isoformat(), "iso_week": iso, "week48": w48, "n_iso": len(si), "n_48": len(s48), "symdiff": len(si ^ s48), "only_iso": len(si - s48), "only_48": len(s48 - si)})
            d += dt.timedelta(days=1)
        assert len(rows5) == 365
        with open(out_dir / f"iso48_2026_{tag}.csv", "w", newline="") as f:
            w = csv.DictWriter(f, fieldnames=list(rows5[0]))
            w.writeheader()
            w.writerows(rows5)
        sd = np.array([r["symdiff"] for r in rows5])
        worst = sorted(rows5, key=lambda r: (-r["symdiff"], r["date"]))[:5]
        wrap = [r for r in rows5 if r["iso_week"] >= 49]
        inr = [r for r in rows5 if r["iso_week"] <= 48]
        summ = {
            "dates": 365,
            "mean_symdiff": float(sd.mean()),
            "max_symdiff": int(sd.max()),
            "dates_at_max": int((sd == sd.max()).sum()),
            "dates_nonempty": int((sd > 0).sum()),
            "median_symdiff": float(np.median(sd)),
            "dates_iso_ge_49": len(wrap),
            "mean_symdiff_iso_ge_49": float(np.mean([r["symdiff"] for r in wrap])) if wrap else None,
            "mean_symdiff_iso_le_48": float(np.mean([r["symdiff"] for r in inr])),
            "dates_iso_le_48": len(inr),
            "dates_nonempty_iso_le_48": sum(1 for r in inr if r["symdiff"] > 0),
            "max_symdiff_iso_le_48": max(r["symdiff"] for r in inr),
            "worst5_iso_le_48": sorted(inr, key=lambda r: (-r["symdiff"], r["date"]))[:5],
            "n_passing_48_mean": float(np.mean([r["n_48"] for r in rows5])),
            "worst5": worst,
        }
        results["iso48"][tag] = summ
        print(f"### {tag} TFLite, lat={PROBE_LAT} lon={PROBE_LON}, T={PROBE_T}\n")
        print(json.dumps({k: v for k, v in summ.items() if not k.startswith("worst5")}, indent=2))
        print(
            md_table(
                ["date", "ISO week", "48-week", "n(ISO)", "n(48)", "symmetric difference", "only in ISO", "only in 48"],
                [[r["date"], r["iso_week"], r["week48"], r["n_iso"], r["n_48"], r["symdiff"], r["only_iso"], r["only_48"]] for r in worst],
            )
        )
        print("\nworst 5 among dates whose ISO week is in 1..48 (i.e. excluding the >=49 'no seasonal info' dates):")
        print(
            md_table(
                ["date", "ISO week", "48-week", "n(ISO)", "n(48)", "symmetric difference", "only in ISO", "only in 48"],
                [[r["date"], r["iso_week"], r["week48"], r["n_iso"], r["n_48"], r["symdiff"], r["only_iso"], r["only_48"]] for r in summ["worst5_iso_le_48"]],
            )
        )
        print()

    (out_dir / "results.json").write_text(json.dumps(results, indent=2, default=str))
    print(f"wrote {out_dir / 'results.json'}")
    return 0


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)

    c = sub.add_parser("convert", help="TFLite -> ONNX with tf2onnx")
    c.add_argument("--tflite", required=True)
    c.add_argument("--onnx", required=True)
    c.add_argument("--opset", type=int, default=17)
    c.set_defaults(fn=cmd_convert)

    f = sub.add_parser("fingerprint", help="name-agnostic hash of an ONNX graph")
    f.add_argument("--onnx", nargs="+", required=True)
    f.set_defaults(fn=cmd_fingerprint)

    r = sub.add_parser("report", help="fidelity, week semantics, ISO vs 48-week")
    r.add_argument("--labels", required=True)
    r.add_argument("--v1-tflite", required=True)
    r.add_argument("--v1-onnx", required=True)
    r.add_argument("--v2-tflite", required=True)
    r.add_argument("--v2-onnx", required=True)
    r.add_argument("--extra-onnx", action="append", metavar="NAME=PATH", help="extra v2 ONNX to compare, e.g. birdnetgo=/path/x.onnx")
    r.add_argument("--out-dir", required=True)
    r.set_defaults(fn=cmd_report)

    a = p.parse_args()
    return a.fn(a)


if __name__ == "__main__":
    sys.exit(main())
