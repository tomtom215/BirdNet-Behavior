"""What kind of class is a false report?  For bars 0.10, 0.20, 0.35: congener of a present species (same genus), other species class,
or FSD50K sound class.  Synthetic mixtures and real strict windows with >= 1 annotated species. -> results/false_report_types.csv"""
import sys, os, csv, collections
import numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common as c
import metrics as m
import realdata as rd

sci, isb = c.load_perch_labels()
genus = [s.split()[0] if isb[i] else None for i, s in enumerate(sci)]

def count(T, present, edge, wins, bars=(0.10, 0.20, 0.35)):
    out = {}
    for t in bars:
        cc = collections.Counter(); nw = 0
        for w in wins:
            nw += 1
            pset, eset = set(present[w]), set(edge[w])
            pg = {genus[i] for i in pset}
            for ci, sh in zip(T["top_idx"][w], T["top_sh"][w]):
                ci = int(ci)
                if sh < t: break
                if ci in pset or ci in eset: continue
                cc["fsd_sound_class" if not isb[ci] else ("congener_of_present" if genus[ci] in pg else "other_species_class")] += 1
        out[t] = (nw, cc)
    return out

rows = []
sc = list(csv.DictReader(open(c.WORK + "/syn/mix_windows.csv"))); SZ = np.load(c.WORK + "/syn/mix_perch.npy")
pres = [m.parse_ids(r["perch_idx"]) for r in sc]
T = m.window_table(SZ, pres, [[] for _ in pres], isb)
for n in (2, 3, 4):
    wins = [k for k, r in enumerate(sc) if int(r["n_species"]) == n]
    for t, (nw, cc) in count(T, pres, [[] for _ in pres], wins).items():
        rows.append(dict(set=f"synthetic, {n} species", bar=t, windows=nw, congener_of_present=cc["congener_of_present"], other_species_class=cc["other_species_class"], fsd_sound_class=cc["fsd_sound_class"]))
D = rd.load_all()
W = D["W"]
n_all = np.array([int(r["n_present_all"]) for r in W]); n_unm = np.array([int(r["n_present_unmapped"]) for r in W]); unk = np.array([int(r["has_unknown"]) for r in W])
n_edge = np.array([int(r["n_edge_all"]) for r in W])
strict = (n_unm == 0) & (unk == 0); empty = (n_all == 0) & (n_edge == 0) & (unk == 0)
TR = m.window_table(D["Z"], D["present"], D["edge"], isb)
for lab, mk in (("real, strict windows with >=1 annotated species", strict & (n_all >= 1)), ("real, windows with no annotation", empty)):
    wins = list(np.nonzero(mk)[0])
    for t, (nw, cc) in count(TR, D["present"], D["edge"], wins).items():
        rows.append(dict(set=lab, bar=t, windows=nw, congener_of_present=cc["congener_of_present"], other_species_class=cc["other_species_class"], fsd_sound_class=cc["fsd_sound_class"]))
with open(c.RES + "/false_report_types.csv", "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=list(rows[0])); w.writeheader(); w.writerows(rows)
for r in rows: print(r)
