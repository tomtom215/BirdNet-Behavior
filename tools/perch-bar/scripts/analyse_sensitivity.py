"""How much do the real-data recall / false-report numbers depend on the rule that decides which annotated species are 'present' in a 5 s window?
Windows are relabelled from the original annotation events with three rules; the Perch logits are unchanged.  -> results/real_label_sensitivity.csv
  default : present if overlap >= min(0.5*d, 1.0 s)   (d = max(event length, 0.3 s)); 'edge' = within 0.5 s of the window
  lenient : present if overlap >= 0.05 s ; edge = within 0.5 s
  strict  : present if overlap >= min(1.0*d, 2.0 s)   (the whole call, or 2 s of a long one, must lie inside the window)
"""
import sys, os, csv, collections
import numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common as c
import metrics as m
import events as ev
import realdata as rd

D = rd.load_all()
W, Z, sci, isb, mp = D["W"], D["Z"], D["sci"], D["isb"], D["mp"]
N = len(W)
evmap = {}
for tag, p, e in ev.real_files(): evmap[(tag, os.path.basename(p).replace("__", "/", 1) if tag == "CEB" else os.path.basename(p))] = e
cl = np.array([r["cluster"] for r in W])
rows = []
for name, kw in (("default", dict()), ("lenient", dict(core_frac=1.0, core_cap=0.05)), ("strict", dict(core_frac=1.0, core_cap=2.0))):
    present, edge, n_all, strict = [], [], [], []
    for r in W:
        key = (r["dataset"], r["file"].replace("__", "/", 1) if r["dataset"] == "CEB" else r["file"])
        t0 = float(r["t0"])
        pres, ed, unk = ev.label_window(evmap[key], t0, t0 + 5.0, **kw)
        ids, unm = [], 0
        for s in pres:
            i, _ = mp.perch_idx(s)
            if i is None: unm += 1
            else: ids.append(i)
        eids = [i for i in (mp.perch_idx(s)[0] for s in ed) if i is not None]
        present.append(sorted(set(ids))); edge.append(sorted(set(eids))); n_all.append(len(pres))
        strict.append(unm == 0 and not unk)
    n_all = np.array(n_all); strict = np.array(strict)
    T = m.window_table(Z, present, edge, isb)
    for g, gm in (("n=1", n_all == 1), ("n=2", n_all == 2), ("n>=3", n_all >= 3), ("n>=1", n_all >= 1)):
        pm = gm[T["pair_win"]]
        fm = gm & strict
        for j, t in enumerate(m.BARS):
            if t not in (0.10, 0.20, 0.35, 0.50): continue
            hit = (T["pair_share"][pm] >= t).astype(float)
            rec, lo, hi = m.cluster_ratio_ci(hit, np.ones(len(hit)), cl[T["pair_win"][pm]], seed=5)
            fr = T["fr_sp"][fm, j].astype(float)
            frm, flo, fhi = m.cluster_ratio_ci(fr, np.ones(len(fr)), cl[fm], seed=6)
            rows.append(dict(label_rule=name, group=g, bar=t, n_pairs=int(pm.sum()), n_windows_fr=int(fm.sum()), recall=round(rec, 4), recall_lo=round(lo, 4), recall_hi=round(hi, 4),
                             fr_species_per_window=round(frm, 4), fr_lo=round(flo, 4), fr_hi=round(fhi, 4)))
with open(c.RES + "/real_label_sensitivity.csv", "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=list(rows[0])); w.writeheader(); w.writerows(rows)
print(len(rows), "rows")
