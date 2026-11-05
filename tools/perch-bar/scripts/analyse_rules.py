"""(d) Reporting rules compared on the same windows: fixed bar on the share, top-k above a floor, share relative to the top class,
bar on the raw logit.  -> results/rules_*.csv

For every rule and every evaluation set: recall over annotated/known-present species (cluster bootstrap CI), reported classes that are
not present (species classes and FSD classes, per window), and 'exact' = fraction of windows whose report equals the present set.
Evaluation sets: real strict windows (n annotated species = 0 [no annotation], 1, 2, 3+) and the synthetic mixtures (n = 1..4).
"""
import sys, os, csv, collections
import numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common as c
import metrics as m
import realdata as rd

sci, isb = c.load_perch_labels()

def rules():
    R = []
    for t in m.BARS: R.append(("bar", f"share >= {t:.2f}", t, None))
    for k in (1, 2, 3, 5):
        for f in (0.02, 0.05, 0.10): R.append(("topk", f"top-{k} with share >= {f:.2f}", k, f))
    for r in (0.10, 0.25, 0.50):
        for f in (0.02, 0.05, 0.10): R.append(("rel", f"share >= {r:.2f} x top share and >= {f:.2f}", r, f))
    for z in (6, 8, 10, 12): R.append(("logit", f"logit >= {z}", z, None))
    return R

def report(rule, T, Z, w):
    kind, _, a, b = rule
    idx, sh = T["top_idx"][w], T["top_sh"][w]
    if kind == "bar": return idx[sh >= a]
    if kind == "topk": return idx[:a][sh[:a] >= b]
    if kind == "rel": return idx[sh >= max(b, a * sh[0])]
    if kind == "logit": return np.nonzero(Z[w] >= a)[0]

def evaluate(rule, T, Z, present, edge, wins, cluster):
    tp = []; tot = []; frs = []; frf = []; exact = []; nrep = []
    for w in wins:
        rep = report(rule, T, Z, w)
        ps, es = set(present[w]), set(edge[w])
        got = [int(x) for x in rep]
        hit = sum(1 for x in got if x in ps)
        bad = [x for x in got if x not in ps and x not in es]
        tp.append(hit); tot.append(len(ps)); frs.append(sum(1 for x in bad if isb[x])); frf.append(sum(1 for x in bad if not isb[x]))
        exact.append(int(hit == len(ps) and len(bad) == 0)); nrep.append(len(got))
    tp, tot, frs, frf, exact, nrep = map(lambda a: np.array(a, float), (tp, tot, frs, frf, exact, nrep))
    cl = np.asarray(cluster)
    out = {}
    if tot.sum() > 0:
        r, lo, hi = m.cluster_ratio_ci(tp, tot, cl, seed=3)
        out.update(recall=round(r, 4), recall_lo=round(lo, 4), recall_hi=round(hi, 4))
    r, lo, hi = m.cluster_ratio_ci(frs, np.ones(len(frs)), cl, seed=5)
    out.update(fr_species_per_window=round(r, 4), fr_lo=round(lo, 4), fr_hi=round(hi, 4))
    out.update(fr_fsd_per_window=round(float(frf.mean()), 4), exact_window_frac=round(float(exact.mean()), 4), reported_per_window=round(float(nrep.mean()), 3),
               precision_hits_over_reported=round(float(tp.sum() / nrep.sum()), 4) if nrep.sum() > 0 else "")   # 'edge' classes count as non-hits here
    return out

rows = []
# ---- real
D = rd.load_all()
W, Z = D["W"], D["Z"]
N = len(W)
cl = np.array([r["cluster"] for r in W])
n_all = np.array([int(r["n_present_all"]) for r in W]); n_unm = np.array([int(r["n_present_unmapped"]) for r in W])
n_edge = np.array([int(r["n_edge_all"]) for r in W]); unk = np.array([int(r["has_unknown"]) for r in W])
strict = (n_unm == 0) & (unk == 0); empty = (n_all == 0) & (n_edge == 0) & (unk == 0)
T = m.window_table(Z, D["present"], D["edge"], isb)
sets = {"real, no annotation": np.nonzero(empty)[0], "real, 1 species": np.nonzero(strict & (n_all == 1))[0],
        "real, 2 species": np.nonzero(strict & (n_all == 2))[0], "real, 3+ species": np.nonzero(strict & (n_all >= 3))[0],
        "real, all strict windows": np.nonzero(strict & ((n_all >= 1) | empty))[0]}
for R in rules():
    for sname, wins in sets.items():
        if len(wins) == 0: continue
        e = evaluate(R, T, Z, D["present"], D["edge"], wins, cl[wins])
        rows.append(dict(eval_set=sname, family=R[0], rule=R[1], n_windows=len(wins), **e))
# ---- synthetic
sc = list(csv.DictReader(open(c.WORK + "/syn/mix_windows.csv"))); SZ = np.load(c.WORK + "/syn/mix_perch.npy")
A = list(csv.DictReader(open(c.WORK + "/syn/alone_windows.csv"))); AZ = np.load(c.WORK + "/syn/alone_perch.npy")
a_idx = [k for k, r in enumerate(A) if float(r["level_dbfs"]) == -30.0 and not int(r["clipped"])]
SZ_all = np.concatenate([AZ[a_idx], SZ])
pres = [[int(A[k]["perch_idx"])] for k in a_idx] + [m.parse_ids(r["perch_idx"]) for r in sc]
nsp = np.array([1] * len(a_idx) + [int(r["n_species"]) for r in sc])
cfgs = np.array(["single"] * len(a_idx) + [r["cfg"] for r in sc])
clip = np.array([0] * len(a_idx) + [int(r["clipped"]) for r in sc])
TS = m.window_table(SZ_all, pres, [[] for _ in pres], isb)
for R in rules():
    for n in (1, 2, 3, 4):
        wins = np.nonzero((nsp == n) & (clip == 0))[0]
        e = evaluate(R, TS, SZ_all, pres, [[] for _ in pres], wins, np.arange(len(wins)))
        rows.append(dict(eval_set=f"synthetic, {n} species (all configs)", family=R[0], rule=R[1], n_windows=len(wins), **e))
    wins = np.nonzero((nsp >= 2) & (cfgs == "eq") & (clip == 0))[0]
    e = evaluate(R, TS, SZ_all, pres, [[] for _ in pres], wins, np.arange(len(wins)))
    rows.append(dict(eval_set="synthetic, 2-4 species, equal level", family=R[0], rule=R[1], n_windows=len(wins), **e))
    wins = np.nonzero((nsp >= 2) & (cfgs == "d20") & (clip == 0))[0]
    e = evaluate(R, TS, SZ_all, pres, [[] for _ in pres], wins, np.arange(len(wins)))
    rows.append(dict(eval_set="synthetic, 2-4 species, one 20 dB louder", family=R[0], rule=R[1], n_windows=len(wins), **e))
all_keys = []
for r in rows:
    for k in r:
        if k not in all_keys: all_keys.append(k)
with open(c.RES + "/rules_eval.csv", "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=all_keys); w.writeheader(); w.writerows(rows)
print(len(rows), "rows")
