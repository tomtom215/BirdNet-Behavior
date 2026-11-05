"""BirdNET+ V3.0 vs Perch v2 on the same real windows -> results/bnp_vs_perch.csv

Pairs: annotated species that have a class in BOTH models.  False reports: classes that are not annotated-present and not 'edge',
counted (a) in the model's own class space and (b) restricted to the bird classes the two models share (BirdNET+ class 'Aves' and a
Perch class of the same Latin name).  Windows for false reports: 'strict' (no unmapped species, no unknown box).
BirdNET+ runs on the 4.5 s centred inside each 5 s window, Perch on the whole 5 s window.
"""
import sys, os, csv
import numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common as c
import metrics as m
import realdata as rd

D = rd.load_all()
W, Z, B, sci, isb, bsci, bcls, mp = D["W"], D["Z"], D["B"].astype(np.float32), D["sci"], D["isb"], D["bsci"], D["bcls"], D["mp"]
N = len(W)
ds = np.array([r["dataset"] for r in W]); cl = np.array([r["cluster"] for r in W])
n_all = np.array([int(r["n_present_all"]) for r in W]); n_unm = np.array([int(r["n_present_unmapped"]) for r in W])
n_edge = np.array([int(r["n_edge_all"]) for r in W]); unk = np.array([int(r["has_unknown"]) for r in W])
strict = (n_unm == 0) & (unk == 0); empty = (n_all == 0) & (n_edge == 0) & (unk == 0)

# class correspondence
p2b = {}
for i, s in enumerate(sci):
    if isb[i] and s in mp.bnp: p2b[i] = mp.bnp[s]
common_bird_p = np.zeros(len(sci), bool); common_bird_b = np.zeros(len(bsci), bool)
for i, j in p2b.items():
    if bcls[j] == "Aves": common_bird_p[i] = True; common_bird_b[j] = True
print("classes with the same Latin name in both models:", len(p2b), "| of which BirdNET+ 'Aves':", int(common_bird_p.sum()))

# common present species per window
pres_p, pres_b, edge_p, edge_b = [], [], [], []
for k in range(N):
    pp, bb = [], []
    for ci in D["present"][k]:
        j = p2b.get(ci)
        if j is None:
            j2 = mp.bnp_idx(sci[ci])
            j = j2
        if j is not None: pp.append(ci); bb.append(j)
    pres_p.append(pp); pres_b.append(bb)
    ep = list(D["edge"][k]); edge_p.append(ep)
    edge_b.append([j for j in (mp.bnp_idx(sci[i]) for i in ep) if j is not None])
T = m.window_table(Z, D["present"], D["edge"], isb)
TB = m.prob_table(B, pres_b, edge_b)
P = T["P"]

rows = []
BARS_ALL = m.BARS_B
def fr_common(top_idx, top_sh, k, t, mask, pset, eset):
    cnt = 0
    for ci, sh in zip(top_idx[k], top_sh[k]):
        ci = int(ci)
        if sh < t: break
        if mask[ci] and ci not in pset and ci not in eset: cnt += 1
    return cnt

def group(label, rec_mask, fr_mask):
    pm_w = np.nonzero(rec_mask)[0]
    pair_cl, sp_perch, sp_bnp = [], [], []
    for k in pm_w:
        for ci, bj in zip(pres_p[k], pres_b[k]):
            pair_cl.append(cl[k]); sp_perch.append(float(P[k, ci])); sp_bnp.append(float(B[k, bj]))
    sp_perch = np.array(sp_perch); sp_bnp = np.array(sp_bnp)
    fk = np.nonzero(fr_mask)[0]
    for t in BARS_ALL:
        for model in ("Perch v2", "BirdNET+ V3.0"):
            vals = sp_perch if model == "Perch v2" else sp_bnp
            row = dict(group=label, model=model, bar=t, n_pairs=len(vals), n_windows_fr=len(fk))
            if len(vals):
                hit = (vals >= t).astype(float)
                rec, lo, hi = m.cluster_ratio_ci(hit, np.ones(len(hit)), np.array(pair_cl), seed=17)
                row.update(recall=round(rec, 4), recall_boot_lo=round(lo, 4), recall_boot_hi=round(hi, 4))
            own, comm = [], []
            for k in fk:
                if model == "Perch v2":
                    ps, es = set(D["present"][k]), set(D["edge"][k])
                    own.append(sum(1 for ci, sh in zip(T["top_idx"][k], T["top_sh"][k]) if sh >= t and isb[int(ci)] and int(ci) not in ps and int(ci) not in es))
                    comm.append(fr_common(T["top_idx"], T["top_sh"], k, t, common_bird_p, ps, es))
                else:
                    ps, es = set(pres_b[k]), set(edge_b[k])
                    # BirdNET+ present set = all annotated species with a BirdNET+ class
                    ps = set(D["present_b"][k]) | ps
                    own.append(sum(1 for ci, sh in zip(TB["top_idx"][k], TB["top_sh"][k]) if sh >= t and int(ci) not in ps and int(ci) not in es))
                    comm.append(fr_common(TB["top_idx"], TB["top_sh"], k, t, common_bird_b, ps, es))
            if len(fk):
                o = np.array(own, float); cm = np.array(comm, float)
                p, lo, hi = m.cluster_ratio_ci(o, np.ones(len(o)), cl[fk], seed=19)
                row.update(fr_own_classes_per_window=round(p, 4), fr_own_boot_lo=round(lo, 4), fr_own_boot_hi=round(hi, 4))
                p, lo, hi = m.cluster_ratio_ci(cm, np.ones(len(cm)), cl[fk], seed=23)
                row.update(fr_common_bird_per_window=round(p, 4), fr_common_boot_lo=round(lo, 4), fr_common_boot_hi=round(hi, 4))
            rows.append(row)

for lab, g in (("n annotated = 1", n_all == 1), ("n annotated = 2", n_all == 2), ("n annotated >= 3", n_all >= 3), ("n annotated >= 1", n_all >= 1)):
    group(lab, g & (np.array([len(x) for x in pres_p]) >= 1), g & strict)
group("no annotation (recall n/a)", np.zeros(N, bool), empty)
all_keys = []
for r in rows:
    for k in r:
        if k not in all_keys: all_keys.append(k)
with open(c.RES + "/bnp_vs_perch.csv", "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=all_keys); w.writeheader(); w.writerows(rows)
# empty windows: any class above bar, both models
emp = []
for t in BARS_ALL:
    for model, anyg, j in (("Perch v2", None, None), ("BirdNET+ V3.0", None, None)):
        if model == "Perch v2":
            v = np.array([T["top_sh"][k, 0] >= t for k in np.nonzero(empty)[0]], float)
        else:
            v = np.array([TB["top_sh"][k, 0] >= t for k in np.nonzero(empty)[0]], float)
        p, lo, hi = m.cluster_ratio_ci(v, np.ones(len(v)), cl[empty], seed=29)
        wp, wlo, whi = m.wilson(int(v.sum()), len(v))
        emp.append(dict(model=model, bar=t, n_windows=len(v), frac_any_class_ge_bar=round(p, 4), boot_lo=round(lo, 4), boot_hi=round(hi, 4),
                        wilson_lo=round(wlo, 4), wilson_hi=round(whi, 4)))
with open(c.RES + "/bnp_vs_perch_empty_windows.csv", "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=list(emp[0])); w.writeheader(); w.writerows(emp)
print("done", N)

# ---- top-1 agreement with the single annotated species (windows with exactly one annotated species, which has a class in both models)
rows1 = []
tags = np.array([r["dataset"] for r in W])
for lab, tmask in [("ALL", np.ones(N, bool))] + [(t, tags == t) for t in rd.TAG_ORDER]:
    ks = [k for k in np.nonzero(tmask & (n_all == 1) & (n_unm == 0))[0] if len(pres_p[k]) == 1]
    if not ks: continue
    okp = sum(1 for k in ks if int(T["top_idx"][k, 0]) == pres_p[k][0])
    okb = sum(1 for k in ks if int(TB["top_idx"][k, 0]) == pres_b[k][0])
    pp_, plo, phi = m.wilson(okp, len(ks)); pb_, blo, bhi = m.wilson(okb, len(ks))
    rows1.append(dict(set=lab, windows=len(ks), perch_top1_is_annotated=okp, perch_frac=round(pp_, 3), perch_wilson_lo=round(plo, 3), perch_wilson_hi=round(phi, 3),
                      bnp_top1_is_annotated=okb, bnp_frac=round(pb_, 3), bnp_wilson_lo=round(blo, 3), bnp_wilson_hi=round(bhi, 3)))
with open(c.RES + "/bnp_vs_perch_top1.csv", "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=list(rows1[0])); w.writeheader(); w.writerows(rows1)
for r in rows1: print(r)
