"""Noise-only windows (ESC-50 real clips + 3 synthetic controls): how often is ANY class over each bar? -> results/noise_*.csv"""
import sys, os, csv, collections
import numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common as c
import metrics as m

sci, isb = c.load_perch_labels()
bsci, bcls = c.load_bnp_labels()
R = list(csv.DictReader(open(c.WORK + "/noise/windows.csv")))
Z = np.load(c.WORK + "/noise/perch.npy"); B = np.load(c.WORK + "/noise/bnp.npy").astype(np.float32)
n = len(R)
T = m.window_table(Z, [[] for _ in R], [[] for _ in R], isb)
TB = m.prob_table(B, [[] for _ in R], [[] for _ in R])
cat = np.array([r["category"] if r["kind"] == "esc50" else "SYNTH:" + r["category"] for r in R])
real = np.array([r["real"] == "1" for r in R])
src = np.array([r["src_file"] if r["kind"] == "esc50" else r["category"] for r in R])

rows = []
def grp(label, mk):
    if mk.sum() == 0: return
    for j, t in enumerate(m.BARS):
        row = dict(group=label, model="Perch v2 (softmax share)", bar=t, n_windows=int(mk.sum()))
        for key, nm in (("any_ge", "frac_any_class"), ("any_sp_ge", "frac_any_species_class")):
            v = T[key][mk, j].astype(float); p, lo, hi = m.wilson(int(v.sum()), len(v))
            row.update({nm: round(p, 4), nm + "_wilson_lo": round(lo, 4), nm + "_wilson_hi": round(hi, 4)})
        row["species_classes_over_bar_per_window"] = round(float(T["fr_sp"][mk, j].mean()), 4)
        rows.append(row)
    for j, t in enumerate(m.BARS_B):
        row = dict(group=label, model="BirdNET+ V3.0 (probability)", bar=t, n_windows=int(mk.sum()))
        v = TB["any_ge"][mk, j].astype(float); p, lo, hi = m.wilson(int(v.sum()), len(v))
        row.update(frac_any_class=round(p, 4), frac_any_class_wilson_lo=round(lo, 4), frac_any_class_wilson_hi=round(hi, 4),
                   species_classes_over_bar_per_window=round(float(TB["fr"][mk, j].mean()), 4))
        rows.append(row)
grp("ESC-50 real clips, all 8 categories", real)
for cname in sorted(set(cat[real])):
    grp(f"ESC-50 {cname}", cat == cname)
for cname in sorted(set(cat[~real])):
    grp(f"synthetic control: {cname[6:]}", cat == cname)
all_keys = []
for r in rows:
    for k in r:
        if k not in all_keys: all_keys.append(k)
with open(c.RES + "/noise_bar.csv", "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=all_keys); w.writeheader(); w.writerows(rows)

# what does Perch call these clips?
tops = collections.defaultdict(collections.Counter)
for k in range(n):
    tops[cat[k]][sci[int(T["top_idx"][k, 0])]] += 1
with open(c.RES + "/noise_top1_classes.csv", "w", newline="") as f:
    w = csv.writer(f); w.writerow(["category", "model", "top1_class", "windows", "mean_top1_share_for_category"])
    for cname, cnt in sorted(tops.items()):
        mk = cat == cname
        for cl_, v in cnt.most_common(4):
            w.writerow([cname, "Perch", cl_, v, round(float(T["top_sh"][mk, 0].mean()), 4)])
    topsb = collections.defaultdict(collections.Counter)
    for k in range(n): topsb[cat[k]][bsci[int(TB["top_idx"][k, 0])]] += 1
    for cname, cnt in sorted(topsb.items()):
        mk = cat == cname
        for cl_, v in cnt.most_common(4):
            w.writerow([cname, "BirdNET+", cl_, v, round(float(TB["top_sh"][mk, 0].mean()), 4)])
print(n, "noise windows;", int(real.sum()), "real")
print("max Perch top-1 share over real noise clips:", float(T["top_sh"][real, 0].max()), "| 95th pct:", float(np.percentile(T["top_sh"][real, 0], 95)))
