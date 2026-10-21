#!/usr/bin/env python3
"""Window-by-window agreement between this project's V2.4 path and BirdNET-Pi's, with intervals.

    compare_ext.py OURS.json GOLDEN.json [--confidence 0.7] [--sensitivity 1.25] [--privacy-rank 10]
                   [--thresholds 0.3,0.5,0.7,0.9] [--boot 10000] [--seed 20261021] [--out summary.json]

OURS   is the JSON `tests/parity_dump.rs` writes (per clip, per window: the model's top 10 as raw logits,
       the best human-class rank, the window's peak-to-peak and RMS).
GOLDEN is a `run_extended.py` / `run_matrix.py` output (`x0_pi_default.json`, `a1_station.json`): BirdNET-Pi's own
       code on its FP16 TFLite model, per window top 10 with logits and its privacy mask.

For every window both sides cut identically it compares

  top-1        the same label first
  decision(T)  the SET of labels whose score (sigmoid(slope x logit), BirdNET-Pi's slope for --sensitivity) reaches T
               among the window's top 10 is the same; at T = --confidence this is the stored-detection decision
               before the occurrence filter.  Reported with and without windows nobody could call a bird.
  privacy      the window is masked (a human class in the top --privacy-rank, or next to such a window) on both or neither
  |dlogit|     the raw-logit gap on labels both put in the top 10

and groups windows into strata: ordinary; padded (the last window of a recording that is not a whole number of
windows, zero-filled); constant (peak-to-peak 0: digital silence).  The model is known to disagree with itself on the
last two (README.md, "Which V2.4 build stands in for ..."), so they are reported apart.

Intervals.  A proportion has two: the Wilson score interval treating windows as independent, and the percentile
interval of a CLUSTER bootstrap that resamples whole clips (windows of one recording are not independent: the same
bird sings through all of them).  The cluster interval is the one to read.  Both are 95 %.  Quantiles of |dlogit|
carry a cluster-bootstrap interval too.  The resampling is seeded, so the same inputs give the same numbers.
"""
import argparse
import json
import math
import sys

import numpy as np


def slope(sens):
    return max(0.5, min(1.0 - (sens - 1.0), 1.5))


def wilson(k, n, z=1.959964):
    if n == 0:
        return (float("nan"), float("nan"))
    p = k / n
    d = 1 + z * z / n
    c = p + z * z / (2 * n)
    h = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n))
    return ((c - h) / d, (c + h) / d)


def cluster_boot(values_by_clip, rng, nboot, stat):
    """values_by_clip: list of 1-D arrays (one per clip). stat(concatenated) -> float. Percentile 95 % interval."""
    n = len(values_by_clip)
    if n == 0:
        return (float("nan"), float("nan"))
    out = []
    for _ in range(nboot):
        idx = rng.integers(0, n, n)
        v = np.concatenate([values_by_clip[i] for i in idx]) if n else np.array([])
        out.append(stat(v) if len(v) else float("nan"))
    out = np.array(out)
    out = out[~np.isnan(out)]
    return (float(np.percentile(out, 2.5)), float(np.percentile(out, 97.5))) if len(out) else (float("nan"), float("nan"))


def fmt(k, n, w, c):
    return f"{k}/{n} = {100*k/n:5.1f}%  Wilson [{100*w[0]:5.1f},{100*w[1]:5.1f}]  clips-bootstrap [{100*c[0]:5.1f},{100*c[1]:5.1f}]" if n else "0/0"


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("ours")
    ap.add_argument("golden")
    ap.add_argument("--confidence", type=float, default=0.7)
    ap.add_argument("--sensitivity", type=float, default=1.25)
    ap.add_argument("--privacy-rank", type=int, default=10)
    ap.add_argument("--thresholds", default="0.3,0.5,0.7,0.9")
    ap.add_argument("--boot", type=int, default=10000)
    ap.add_argument("--seed", type=int, default=20261021)
    ap.add_argument("--out")
    a = ap.parse_args()
    s = slope(a.sensitivity)
    ours = json.load(open(a.ours))
    golden = json.load(open(a.golden))
    rng = np.random.default_rng(a.seed)
    thresholds = sorted({float(x) for x in a.thresholds.split(",")} | {a.confidence})

    clips = []   # per clip: dict of per-window records
    chunking_bad = []
    for run in golden["runs"]:
        name = run["clip"]["file"].removesuffix(".wav")
        g = run["chunks"]
        o = ours.get(name)
        dur = run["clip"].get("duration_s", 0.0)
        if o is None:
            chunking_bad.append((name, "missing from ours"))
            continue
        if len(g) != len(o) or any(abs(x["start"] - y["start"]) > 1e-3 for x, y in zip(g, o)):
            chunking_bad.append((name, f"ours {len(o)} windows, BirdNET-Pi {len(g)}"))
            continue
        own = [y.get("human_rank") is not None and y["human_rank"] < a.privacy_rank for y in o]
        recs = []
        for i, (x, y) in enumerate(zip(g, o)):
            pi_logit = {l: lg for (l, _), lg in zip(x["top10"], x["top10_logits"])}
            our_logit = {l: lg for l, lg, _p in y["top"]}
            our_score = {l: 1 / (1 + math.exp(-s * lg)) for l, lg in our_logit.items()}
            pi_score = {l: sc for l, sc in x["top10"]}
            kind = "constant" if y.get("ptp", 1.0) == 0.0 else ("padded" if x["end"] > dur + 1e-6 else "ordinary")
            dec = {}
            for T in thresholds:
                ps = {l for l, sc in pi_score.items() if sc >= T}
                os_ = {l for l, sc in our_score.items() if sc >= T}
                dec[T] = (ps == os_, len(ps | os_), len(ps ^ os_))
            mask = own[i] or (i > 0 and own[i - 1]) or (i + 1 < len(own) and own[i + 1])
            recs.append({
                "kind": kind,
                "top1": x["top10"][0][0] == y["top"][0][0],
                "dec": dec,
                "priv": bool(x["privacy_masked"]) == mask,
                "dlogit": [abs(pi_logit[l] - our_logit[l]) for l in pi_logit.keys() & our_logit.keys()],
                "rms": y.get("rms"),
            })
        clips.append({"name": name, "recs": recs})

    n_clips = len(clips)
    all_recs = [r for c in clips for r in c["recs"]]
    print(f"golden clips {len(golden['runs'])}; cut identically {n_clips}; chunking differs/missing {len(chunking_bad)}")
    for nm, why in chunking_bad:
        print(f"    {nm}: {why}")
    print(f"windows compared {len(all_recs)} "
          f"(ordinary {sum(r['kind']=='ordinary' for r in all_recs)}, padded {sum(r['kind']=='padded' for r in all_recs)}, "
          f"constant {sum(r['kind']=='constant' for r in all_recs)}); slope {s}; privacy rank {a.privacy_rank}")
    summary = {"clips": n_clips, "windows": len(all_recs), "chunking_differs": chunking_bad, "slope": s, "metrics": {}}

    def prop(label, pick, kinds=None, boot=True):
        by_clip = []
        k = n = 0
        for c in clips:
            v = np.array([1.0 if pick(r) else 0.0 for r in c["recs"] if kinds is None or r["kind"] in kinds])
            by_clip.append(v)
            k += int(v.sum())
            n += len(v)
        w = wilson(k, n)
        cb = cluster_boot([v for v in by_clip if len(v)], rng, a.boot, lambda x: float(x.mean())) if boot and n else (float("nan"),) * 2
        print(f"  {label:44} {fmt(k, n, w, cb)}")
        summary["metrics"][label] = {"k": k, "n": n, "wilson95": w, "cluster_bootstrap95": cb}

    for title, kinds in (("all windows", None), ("ordinary windows", {"ordinary"}), ("padded + constant windows", {"padded", "constant"})):
        print(title)
        prop("top-1 label the same", lambda r: r["top1"], kinds)
        for T in thresholds:
            prop(f"decision set the same at {T}", lambda r, T=T: r["dec"][T][0], kinds)
        prop("privacy mask the same", lambda r: r["priv"], kinds)
        summary["metrics"][title + " (header)"] = {}

    # The same decision, counting only windows where a bird could be recorded: either side has a label at T.
    print("decision set the same, windows where either side has a label at or above T (the windows that can store a row)")
    for T in thresholds:
        by_clip, k, n = [], 0, 0
        for c in clips:
            v = np.array([1.0 if r["dec"][T][0] else 0.0 for r in c["recs"] if r["dec"][T][1] > 0])
            by_clip.append(v)
            k += int(v.sum())
            n += len(v)
        w = wilson(k, n)
        cb = cluster_boot([v for v in by_clip if len(v)], rng, a.boot, lambda x: float(x.mean())) if n else (float("nan"),) * 2
        print(f"  T={T:<4} {fmt(k, n, w, cb)}")
        summary["metrics"][f"active windows, decision the same at {T}"] = {"k": k, "n": n, "wilson95": w, "cluster_bootstrap95": cb}

    # Pair-level discordance: of the (window, label) pairs that reach T on either side, how many reach it on one side only.
    print("(window, label) pairs reaching T on either side, ordinary windows: those reaching it on one side only")
    for T in thresholds:
        u = d = 0
        for c in clips:
            for r in c["recs"]:
                if r["kind"] == "ordinary":
                    u += r["dec"][T][1]
                    d += r["dec"][T][2]
        w = wilson(d, u)
        print(f"  T={T:<4} {d}/{u} discordant = {100*d/u if u else float('nan'):5.1f}%  Wilson [{100*w[0]:.1f},{100*w[1]:.1f}]")
        summary["metrics"][f"discordant pairs at {T} (ordinary)"] = {"k": d, "n": u, "wilson95": w}

    print("|raw-logit gap| on labels in both top 10s (BirdNET-Pi FP16 TFLite vs this project's FP32 ONNX)")
    for title, kinds in (("ordinary", {"ordinary"}), ("padded", {"padded"}), ("constant", {"constant"})):
        by_clip = [np.array([x for r in c["recs"] if r["kind"] in kinds for x in r["dlogit"]]) for c in clips]
        by_clip = [v for v in by_clip if len(v)]
        allv = np.concatenate(by_clip) if by_clip else np.array([])
        if not len(allv):
            print(f"  {title:9} no windows")
            continue
        row = {}
        parts = []
        for q in (50, 90, 99):
            ci = cluster_boot(by_clip, rng, min(a.boot, 2000), lambda x, q=q: float(np.percentile(x, q)))
            row[f"p{q}"] = (float(np.percentile(allv, q)), ci)
            parts.append(f"p{q} {np.percentile(allv, q):.4f} [{ci[0]:.4f},{ci[1]:.4f}]")
        row["max"] = float(allv.max())
        row["n"] = int(len(allv))
        print(f"  {title:9} n={len(allv):5}  " + "  ".join(parts) + f"  max {allv.max():.3f}")
        summary["metrics"][f"dlogit {title}"] = row

    if a.out:
        json.dump(summary, open(a.out, "w"), indent=1, default=lambda o: list(o) if isinstance(o, tuple) else str(o))
    return 0


if __name__ == "__main__":
    sys.exit(main())
