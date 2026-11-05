"""Task 3 on the real annotated soundscape windows -> results/real_*.csv"""
import sys, os, csv, collections
import numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common as c
import metrics as m
import realdata as rd

BARS = m.BARS
D = rd.load_all()
W, Z, sci, isb = D["W"], D["Z"], D["sci"], D["isb"]
N = len(W)
ds = np.array([r["dataset"] for r in W]); cl = np.array([r["cluster"] for r in W])
n_all = np.array([int(r["n_present_all"]) for r in W]); n_map = np.array([int(r["n_present_mapped"]) for r in W])
n_unm = np.array([int(r["n_present_unmapped"]) for r in W]); n_edge = np.array([int(r["n_edge_all"]) for r in W])
unk = np.array([int(r["has_unknown"]) for r in W])
strict = (n_unm == 0) & (unk == 0)
empty = (n_all == 0) & (n_edge == 0) & (unk == 0)
rmasks = [D["region_masks"][t] for t in ds]
T = m.window_table(Z, D["present"], D["edge"], isb, rmasks)
np.save(c.WORK + "/real_P_top.npy", T["top_sh"])
gl = np.array([("5+" if k >= 5 else str(k)) if k > 0 else ("empty" if e else "0-other") for k, e in zip(n_all, empty)])

# ---------------- inventory
inv = []
for tag in rd.TAG_ORDER:
    sel = ds == tag
    if not sel.any(): continue
    inv.append(dict(dataset=tag, files=len(set(cl[sel])), hours=round(sel.sum() * 5 / 3600, 2), windows=int(sel.sum()),
                    w_empty=int((sel & empty).sum()), w_1=int((sel & (n_all == 1)).sum()), w_2=int((sel & (n_all == 2)).sum()),
                    w_3=int((sel & (n_all == 3)).sum()), w_4=int((sel & (n_all == 4)).sum()), w_5plus=int((sel & (n_all >= 5)).sum()),
                    w_unknown_box=int((sel & (unk == 1)).sum()), w_with_unmapped_species=int((sel & (n_unm > 0)).sum()),
                    pairs=int(sum(len(p) for p, s in zip(D["present"], sel) if s)),
                    median_window_rms_dbfs=float(np.median([float(r["rms_dbfs"]) for r, s in zip(W, sel) if s]))))
tot = dict(dataset="ALL", files=len(set(cl)), hours=round(N * 5 / 3600, 2), windows=N)
for k in ("w_empty", "w_1", "w_2", "w_3", "w_4", "w_5plus", "w_unknown_box", "w_with_unmapped_species", "pairs"):
    tot[k] = sum(r[k] for r in inv)
tot["median_window_rms_dbfs"] = float(np.median([float(r["rms_dbfs"]) for r in W]))
inv.append(tot)
with open(c.RES + "/real_inventory.csv", "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=list(inv[0])); w.writeheader(); w.writerows(inv)

# ---------------- species annotated but without a Perch class (never counted as misses)
unm = collections.Counter(); unm_w = collections.Counter()
for r in W:
    for nme in (r["unmapped_names"].split("|") if r["unmapped_names"] else []):
        unm_w[(r["dataset"], nme)] += 1
names_all = collections.Counter()
import events as ev
for tag, p, e in ev.real_files():
    for a, b, sp in e: names_all[(tag, sp)] += 1
rows = []
for (tag, sp), n_ev in sorted(names_all.items()):
    if sp == "unknown": continue
    i, basis = D["mp"].perch_idx(sp)
    if i is None:
        rows.append(dict(dataset=tag, annotated_name=sp, events_in_my_files=n_ev, windows_present_in_my_files=unm_w.get((tag, sp), 0)))
with open(c.RES + "/real_species_without_perch_class.csv", "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=["dataset", "annotated_name", "events_in_my_files", "windows_present_in_my_files"]); w.writeheader(); w.writerows(rows)
syn_used = []
for (tag, sp), n_ev in sorted(names_all.items()):
    i, basis = D["mp"].perch_idx(sp)
    if basis == "synonym":
        from taxon import SYNONYMS
        syn_used.append(dict(dataset=tag, annotated_name=sp, perch_name=SYNONYMS[sp][0], reason=SYNONYMS[sp][1], events=n_ev))
with open(c.RES + "/taxon_synonyms_used.csv", "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=["dataset", "annotated_name", "perch_name", "reason", "events"]); w.writeheader(); w.writerows(syn_used)

# ---------------- bar sweeps
pw = T["pair_win"]; psh = T["pair_share"]
sweep = []
def sweep_group(label, rec_mask, fr_mask, rows):
    pm = rec_mask[pw]
    if pm.sum() == 0 and fr_mask.sum() == 0: return
    sh = psh[pm]; pcl = cl[pw[pm]]
    for j, t in enumerate(BARS):
        row = dict(group=label, bar=t, n_pairs=int(pm.sum()), n_windows_recall=int(len(set(pw[pm]))), n_windows_fr=int(fr_mask.sum()), n_files=len(set(cl[rec_mask | fr_mask])))
        if pm.sum():
            hit = (sh >= t).astype(float)
            rec, lo, hi = m.cluster_ratio_ci(hit, np.ones(len(hit)), pcl, seed=100 + j)
            wp, wlo, whi = m.wilson(int(hit.sum()), len(hit))
            row.update(recall=round(rec, 4), recall_boot_lo=round(lo, 4), recall_boot_hi=round(hi, 4), recall_wilson_lo=round(wlo, 4), recall_wilson_hi=round(whi, 4))
        if fr_mask.sum():
            k = fr_mask
            for key, colname in (("fr_sp", "fr_species_per_window"), ("fr_reg", "fr_regional_oracle_per_window"), ("fr_fsd", "fr_fsd_per_window")):
                v = T[key][k, j].astype(float)
                p, lo, hi = m.cluster_ratio_ci(v, np.ones(len(v)), cl[k], seed=300 + j)
                row[colname] = round(p, 4)
                if key == "fr_sp": row["fr_boot_lo"] = round(lo, 4); row["fr_boot_hi"] = round(hi, 4)
                if key == "fr_reg": row["fr_reg_boot_lo"] = round(lo, 4); row["fr_reg_boot_hi"] = round(hi, 4)
            a = (T["fr_sp"][k, j] > 0).astype(float)
            ap, a_lo, a_hi = m.cluster_ratio_ci(a, np.ones(len(a)), cl[k], seed=500 + j)
            row.update(frac_windows_with_false_species=round(ap, 4), anyfr_boot_lo=round(a_lo, 4), anyfr_boot_hi=round(a_hi, 4))
        rows.append(row)

recmask_base = n_map >= 1          # recall is computed over annotated species that HAVE a Perch class, in any such window
for g in ("1", "2", "3", "4", "5+"):
    sweep_group(f"all datasets, n annotated species = {g}", (gl == g) & recmask_base, (gl == g) & strict, sweep)
sweep_group("all datasets, n annotated species >= 2", (n_all >= 2) & recmask_base, (n_all >= 2) & strict, sweep)
sweep_group("all datasets, n annotated species >= 1", (n_all >= 1) & recmask_base, (n_all >= 1) & strict, sweep)
for tag in rd.TAG_ORDER:
    sel = ds == tag
    if not sel.any(): continue
    sweep_group(f"{tag}, n >= 1", sel & (n_all >= 1) & recmask_base, sel & (n_all >= 1) & strict, sweep)
    sweep_group(f"{tag}, n = 1", sel & (n_all == 1) & recmask_base, sel & (n_all == 1) & strict, sweep)
    sweep_group(f"{tag}, n >= 2", sel & (n_all >= 2) & recmask_base, sel & (n_all >= 2) & strict, sweep)
with open(c.RES + "/real_bar_sweep.csv", "w", newline="") as f:
    all_keys = []
    for r in sweep:
        for k in r:
            if k not in all_keys: all_keys.append(k)
    w = csv.DictWriter(f, fieldnames=all_keys); w.writeheader(); w.writerows(sweep)

# ---------------- windows with NO annotation at all: how often is any class over each bar?
emp = []
def empty_group(label, mk):
    if mk.sum() == 0: return
    for j, t in enumerate(BARS):
        row = dict(group=label, bar=t, n_windows=int(mk.sum()), n_files=len(set(cl[mk])))
        for key, name in (("any_ge", "frac_any_class_ge_bar"), ("any_sp_ge", "frac_any_species_class_ge_bar")):
            v = T[key][mk, j].astype(float)
            p, lo, hi = m.cluster_ratio_ci(v, np.ones(len(v)), cl[mk], seed=700 + j)
            wp, wlo, whi = m.wilson(int(v.sum()), len(v))
            row.update({name: round(p, 4), name + "_boot_lo": round(lo, 4), name + "_boot_hi": round(hi, 4), name + "_wilson_lo": round(wlo, 4), name + "_wilson_hi": round(whi, 4)})
        v = T["fr_sp"][mk, j].astype(float); p, lo, hi = m.cluster_ratio_ci(v, np.ones(len(v)), cl[mk], seed=900 + j)
        row.update(species_classes_over_bar_per_window=round(p, 4), per_window_boot_lo=round(lo, 4), per_window_boot_hi=round(hi, 4))
        v = (T["fr_reg"][mk, j] > 0).astype(float); p, lo, hi = m.cluster_ratio_ci(v, np.ones(len(v)), cl[mk], seed=1100 + j)
        row.update(frac_windows_regional_oracle_class_ge_bar=round(p, 4), regional_boot_lo=round(lo, 4), regional_boot_hi=round(hi, 4))
        emp.append(row)
empty_group("all datasets, windows with no annotation", empty)
for tag in rd.TAG_ORDER:
    empty_group(f"{tag}, windows with no annotation", empty & (ds == tag))
with open(c.RES + "/real_empty_windows_bar.csv", "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=list(emp[0])); w.writeheader(); w.writerows(emp)

# ---------------- share of the present species by number present (real)
dist = []
for g in ("1", "2", "3", "4", "5+"):
    pm = ((gl == g) & recmask_base)[pw]
    sh = psh[pm]
    if len(sh) == 0: continue
    mean, lo, hi = m.cluster_ratio_ci(sh, np.ones(len(sh)), cl[pw[pm]], seed=1300)
    rk = T["pair_rank"][pm]
    dist.append(dict(n_annotated_species=g, n_pairs=len(sh), mean_share=round(mean, 4), mean_boot_lo=round(lo, 4), mean_boot_hi=round(hi, 4),
                     p10=round(float(np.percentile(sh, 10)), 4), median=round(float(np.median(sh)), 4), p90=round(float(np.percentile(sh, 90)), 4),
                     frac_rank1=round(float((rk == 1).mean()), 3), frac_rank_le3=round(float((rk <= 3).mean()), 3), frac_rank_le10=round(float((rk <= 10).mean()), 3)))
with open(c.RES + "/real_share_by_n_present.csv", "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=list(dist[0])); w.writeheader(); w.writerows(dist)

# ---------------- per-species recall (species with >= 15 present windows)
sp_rows = []
byc = collections.defaultdict(list)
for k in range(len(pw)):
    byc[int(T["pair_cls"][k])].append((float(psh[k]), int(T["pair_rank"][k]), W[int(pw[k])]["dataset"]))
for ci, lst in byc.items():
    if len(lst) < 15: continue
    sh = np.array([a for a, _, _ in lst]); rk = np.array([b for _, b, _ in lst])
    sp_rows.append(dict(species=sci[ci], datasets="|".join(sorted({d for _, _, d in lst})), n_windows=len(lst), median_share=round(float(np.median(sh)), 4),
                        recall_at_0_10=round(float((sh >= 0.10).mean()), 3), recall_at_0_35=round(float((sh >= 0.35).mean()), 3), frac_rank1=round(float((rk == 1).mean()), 3)))
sp_rows.sort(key=lambda r: -r["n_windows"])
with open(c.RES + "/real_species_recall.csv", "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=list(sp_rows[0])); w.writeheader(); w.writerows(sp_rows)

# ---------------- what the false reports are (strict windows, bar 0.35 / 0.10): the most frequent absent top-1 classes
fp = collections.Counter(); fpn = collections.Counter()
for k in np.nonzero(strict)[0]:
    pset, eset = set(D["present"][k]), set(D["edge"][k])
    for ci, sh in zip(T["top_idx"][k], T["top_sh"][k]):
        ci = int(ci)
        if ci in pset or ci in eset: continue
        if sh >= 0.35: fp[(sci[ci], int(isb[ci]))] += 1
        if sh >= 0.10: fpn[(sci[ci], int(isb[ci]))] += 1
with open(c.RES + "/real_false_report_classes.csv", "w", newline="") as f:
    w = csv.writer(f); w.writerow(["bar", "class", "is_species_class", "windows"])
    for (nme, s), v in fp.most_common(25): w.writerow([0.35, nme, s, v])
    for (nme, s), v in fpn.most_common(25): w.writerow([0.10, nme, s, v])
top1ok = [(T["top_idx"][k, 0] in set(D["present"][k])) for k in range(N) if n_all[k] == 1 and n_map[k] == 1]
print("windows", N, "files", len(set(cl)), "strict", int(strict.sum()), "empty", int(empty.sum()), "pairs", len(pw))
print("single-species windows: top-1 is the annotated species in", sum(top1ok), "of", len(top1ok))
