"""Task 3 on the controlled mixtures -> results/syn_*.csv"""
import sys, os, csv, collections
import numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common as c
import metrics as m

BARS = m.BARS
sci, isb = c.load_perch_labels()
cat = list(csv.DictReader(open(c.WORK + "/syn/catalogue.csv")))
A = list(csv.DictReader(open(c.WORK + "/syn/alone_windows.csv"))); AZ = np.load(c.WORK + "/syn/alone_perch.npy")
M = list(csv.DictReader(open(c.WORK + "/syn/mix_windows.csv"))); MZ = np.load(c.WORK + "/syn/mix_perch.npy")

# ---- alone table (n=1, and the look-up for per-component baselines)
at = m.window_table(AZ, [[int(r["perch_idx"])] for r in A], [[] for _ in A], isb)
alone_share = {}
for k, r in enumerate(A):
    alone_share[(int(r["seg_id"]), float(r["level_dbfs"]))] = (float(at["P"][k, int(r["perch_idx"])]), int(r["clipped"]))

# ---- mixtures
mt = m.window_table(MZ, [m.parse_ids(r["perch_idx"]) for r in M], [[] for _ in M], isb)

def role(cfg, rel):
    return "equal" if cfg == "eq" else ("dominant" if rel == 0 else "quiet")

pairs = []
for pw, pc, sh, lg, rk in zip(mt["pair_win"], mt["pair_cls"], mt["pair_share"], mt["pair_logit"], mt["pair_rank"]):
    r = M[pw]
    sids = [int(x) for x in r["seg_ids"].split("|")]; cls = m.parse_ids(r["perch_idx"])
    rels = [float(x) for x in r["rel_db"].split("|")]; lvls = [float(x) for x in r["level_dbfs"].split("|")]
    j = cls.index(int(pc))
    a_sh, a_clip = alone_share[(sids[j], lvls[j])]
    pairs.append(dict(win_id=pw, n=int(r["n_species"]), cfg=r["cfg"], seg_id=sids[j], species=sci[int(pc)], rel_db=rels[j],
                      level_dbfs=lvls[j], role=role(r["cfg"], rels[j]), share=float(sh), logit=float(lg), rank=int(rk),
                      alone_share=a_sh, ratio=float(sh) / a_sh if a_sh > 0 else float("nan"),
                      alone_ok=int(a_sh >= 0.5), clipped_alone=a_clip, clipped_mix=int(r["clipped"])))
with open(c.RES + "/syn_pairs.csv", "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=list(pairs[0])); w.writeheader(); w.writerows(pairs)

# n=1 pairs (reference level -30 dBFS) from the alone table
single = []
for k, r in enumerate(A):
    if float(r["level_dbfs"]) == -30.0:
        single.append(dict(win_id=-1 - k, n=1, cfg="single", seg_id=int(r["seg_id"]), species=r["species"], rel_db=0.0, level_dbfs=-30.0, role="single",
                           share=float(at["P"][k, int(r["perch_idx"])]), logit=float(at["pair_logit"][list(at["pair_win"]).index(k)]),
                           rank=int(at["pair_rank"][list(at["pair_win"]).index(k)]),
                           alone_share=float(at["P"][k, int(r["perch_idx"])]), ratio=1.0, alone_ok=int(at["P"][k, int(r["perch_idx"])] >= 0.5),
                           clipped_alone=int(r["clipped"]), clipped_mix=int(r["clipped"])))

def summarize(sel, label_cols, rows_out, subset_name):
    if len(sel) == 0: return
    sh = np.array([p["share"] for p in sel]); ra = np.array([p["ratio"] for p in sel]); rk = np.array([p["rank"] for p in sel])
    cl = [p["seg_id"] for p in sel]
    mean, lo, hi = m.cluster_ratio_ci(sh, np.ones(len(sh)), cl, seed=7)
    rows_out.append(dict(**label_cols, subset=subset_name, n_pairs=len(sel), mean_share=round(mean, 4), mean_ci_lo=round(lo, 4), mean_ci_hi=round(hi, 4),
                         median_share=round(float(np.median(sh)), 4), p10=round(float(np.percentile(sh, 10)), 4), p90=round(float(np.percentile(sh, 90)), 4),
                         mean_ratio_to_alone=round(float(np.nanmean(ra)), 3), median_ratio_to_alone=round(float(np.nanmedian(ra)), 3),
                         frac_rank1=round(float((rk == 1).mean()), 3), frac_rank_le3=round(float((rk <= 3).mean()), 3)))

allpairs = single + pairs
out = []
for n, cfg, rl in sorted({(p["n"], p["cfg"], p["role"]) for p in allpairs}):
    base = [p for p in allpairs if p["n"] == n and p["cfg"] == cfg and p["role"] == rl and not p["clipped_alone"] and not p["clipped_mix"]]
    summarize(base, dict(n_species=n, cfg=cfg, role=rl), out, "all")
    summarize([p for p in base if p["alone_ok"]], dict(n_species=n, cfg=cfg, role=rl), out, "alone_share>=0.5")
with open(c.RES + "/syn_share_by_condition.csv", "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=list(out[0])); w.writeheader(); w.writerows(out)

# ---- bar sweep: recall (pairs) and false reports (windows)
winrows = []
for k, r in enumerate(M):
    winrows.append(dict(win_id=k, n=int(r["n_species"]), cfg=r["cfg"], clipped=int(r["clipped"]),
                        max_absent=float(mt["max_absent"][k]), fsd_total=float(mt["fsd_total"][k]),
                        top1=float(mt["top_sh"][k, 0]), top1_class=sci[int(mt["top_idx"][k, 0])]))
with open(c.RES + "/syn_windows.csv", "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=list(winrows[0])); w.writeheader(); w.writerows(winrows)

sweep = []
def sweep_cell(n, cfg, subset):
    if n == 1:
        sel_p = [p for p in single if not p["clipped_alone"]]
        keep = np.array([True] * len(sel_p))
        ks = [int(r["seg_id"]) for r in A if float(r["level_dbfs"]) == -30.0]
        widx = [k for k, r in enumerate(A) if float(r["level_dbfs"]) == -30.0 and not int(r["clipped"])]
        tab = at; fr_sp = at["fr_sp"][widx]; fr_fsd = at["fr_fsd"][widx]; nw = len(widx)
        wsel_alone_ok = None
    else:
        widx = [k for k, r in enumerate(M) if int(r["n_species"]) == n and r["cfg"] == cfg and not int(r["clipped"])]
        sel_p = [p for p in pairs if p["n"] == n and p["cfg"] == cfg and not p["clipped_mix"] and not p["clipped_alone"]]
        fr_sp = mt["fr_sp"][widx]; fr_fsd = mt["fr_fsd"][widx]; nw = len(widx)
    if subset == "alone_ok":
        sel_p = [p for p in sel_p if p["alone_ok"]]
        if n > 1:
            # windows in which EVERY component is recognised alone at its level
            okwin = collections.Counter(p["win_id"] for p in sel_p)
            widx2 = [k for k in widx if okwin.get(k, 0) == n]
            sel_p = [p for p in sel_p if p["win_id"] in set(widx2)]
            fr_sp = mt["fr_sp"][widx2]; fr_fsd = mt["fr_fsd"][widx2]; nw = len(widx2)
        else:
            widx2 = [k for k, r in enumerate(A) if float(r["level_dbfs"]) == -30.0 and not int(r["clipped"]) and at["P"][k, int(r["perch_idx"])] >= 0.5]
            fr_sp = at["fr_sp"][widx2]; fr_fsd = at["fr_fsd"][widx2]; nw = len(widx2)
    if not sel_p: return
    shares = np.array([p["share"] for p in sel_p]); cl = [p["seg_id"] for p in sel_p]
    for j, t in enumerate(BARS):
        hit = (shares >= t).astype(float)
        rec, rlo, rhi = m.cluster_ratio_ci(hit, np.ones(len(hit)), cl, seed=11 + j)
        wp, wlo, whi = m.wilson(int(hit.sum()), len(hit))
        fr = fr_sp[:, j].astype(float); frm, frlo, frhi = m.cluster_ratio_ci(fr, np.ones(len(fr)), np.arange(len(fr)), seed=21 + j)
        anyfr = (fr_sp[:, j] > 0).astype(float); ap, a_lo, a_hi = m.wilson(int(anyfr.sum()), len(anyfr))
        fsd = fr_fsd[:, j].astype(float)
        # a "perfect window": every present species reported and no absent species class reported
        sweep.append(dict(n_species=n, cfg=cfg, subset=subset, bar=t, n_pairs=len(sel_p), n_windows=nw,
                          recall=round(rec, 4), recall_boot_lo=round(rlo, 4), recall_boot_hi=round(rhi, 4),
                          recall_wilson_lo=round(wlo, 4), recall_wilson_hi=round(whi, 4),
                          fr_species_per_window=round(frm, 4), fr_lo=round(frlo, 4), fr_hi=round(frhi, 4),
                          frac_windows_with_false_species=round(ap, 4), anyfr_wilson_lo=round(a_lo, 4), anyfr_wilson_hi=round(a_hi, 4),
                          fr_fsd_per_window=round(float(fsd.mean()), 4)))
cells = [(1, "single")] + [(n, cfg) for n, cfg in [(2, "eq"), (2, "d10"), (2, "d20"), (3, "eq"), (3, "d10"), (3, "d20"), (3, "ladder"), (4, "eq"), (4, "d10"), (4, "d20"), (4, "ladder")]]
for n, cfg in cells:
    for subset in ("all", "alone_ok"):
        sweep_cell(n, cfg, subset)
with open(c.RES + "/syn_bar_sweep.csv", "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=list(sweep[0])); w.writeheader(); w.writerows(sweep)

# ---- largest absent class / FSD share distribution per cell
dist = []
for n, cfg in cells:
    if n == 1:
        idx = [k for k, r in enumerate(A) if float(r["level_dbfs"]) == -30.0 and not int(r["clipped"])]
        ma = at["max_absent"][idx]; fs = at["fsd_total"][idx]
    else:
        idx = [k for k, r in enumerate(M) if int(r["n_species"]) == n and r["cfg"] == cfg and not int(r["clipped"])]
        ma = mt["max_absent"][idx]; fs = mt["fsd_total"][idx]
    dist.append(dict(n_species=n, cfg=cfg, n_windows=len(idx), max_absent_median=round(float(np.median(ma)), 4), max_absent_p90=round(float(np.percentile(ma, 90)), 4),
                     max_absent_max=round(float(ma.max()), 4), fsd_share_mean=round(float(fs.mean()), 4), fsd_share_p90=round(float(np.percentile(fs, 90)), 4)))
with open(c.RES + "/syn_absent_and_fsd.csv", "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=list(dist[0])); w.writeheader(); w.writerows(dist)

# ---- level sweep of the single-species segments
lv = []
for L in sorted({float(r["level_dbfs"]) for r in A}):
    idx = [k for k, r in enumerate(A) if float(r["level_dbfs"]) == L and not int(r["clipped"])]
    sh = np.array([at["P"][k, int(A[k]["perch_idx"])] for k in idx])
    row = dict(level_dbfs=L, n_windows=len(idx), n_clipped=sum(int(r["clipped"]) for r in A if float(r["level_dbfs"]) == L),
               mean_share=round(float(sh.mean()), 4), median_share=round(float(np.median(sh)), 4), p10=round(float(np.percentile(sh, 10)), 4))
    for t in (0.1, 0.35, 0.5):
        row[f"recall@{t}"] = round(float((sh >= t).mean()), 3)
    lv.append(row)
with open(c.RES + "/syn_level_sweep_single.csv", "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=list(lv[0])); w.writeheader(); w.writerows(lv)
print("windows", len(M), "pairs", len(pairs), "clipped mix windows", sum(int(r["clipped"]) for r in M), "clipped alone", sum(int(r["clipped"]) for r in A))

# ---- where does the probability go?  total share of the present species, per cell
tot_rows = []
for n, cfg in cells:
    if n == 1:
        idx = [k for k, r in enumerate(A) if float(r["level_dbfs"]) == -30.0 and not int(r["clipped"])]
        tot = np.array([sum(at["P"][k, i] for i in [int(A[k]["perch_idx"])]) for k in idx])
    else:
        idx = [k for k, r in enumerate(M) if int(r["n_species"]) == n and r["cfg"] == cfg and not int(r["clipped"])]
        tot = np.array([sum(mt["P"][k, i] for i in m.parse_ids(M[k]["perch_idx"])) for k in idx])
    mean, lo, hi = m.cluster_ratio_ci(tot, np.ones(len(tot)), np.arange(len(tot)), seed=3)
    tot_rows.append(dict(n_species=n, cfg=cfg, n_windows=len(idx), mean_total_share_of_present=round(mean, 4), lo=round(lo, 4), hi=round(hi, 4),
                         median=round(float(np.median(tot)), 4), p10=round(float(np.percentile(tot, 10)), 4), p90=round(float(np.percentile(tot, 90)), 4)))
with open(c.RES + "/syn_total_present_share.csv", "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=list(tot_rows[0])); w.writeheader(); w.writerows(tot_rows)

# ---- two species, one louder: share of the quiet one relative to the loud one (windows where both are recognised alone, share >= 0.5)
qd = []
for cfg in ("d10", "d20"):
    by = collections.defaultdict(list)
    for p in pairs:
        if p["n"] == 2 and p["cfg"] == cfg and not p["clipped_mix"] and not p["clipped_alone"]: by[p["win_id"]].append(p)
    rat = []
    for w_, ps in by.items():
        dom = [p for p in ps if p["role"] == "dominant"][0]; q = [p for p in ps if p["role"] == "quiet"][0]
        if dom["alone_ok"] and q["alone_ok"]: rat.append(q["share"] / max(dom["share"], 1e-9))
    rat = np.array(rat); delta = int(cfg[1:])
    qd.append(dict(cfg=cfg, difference_db=delta, n_windows=len(rat), median_ratio_quiet_over_loud=round(float(np.median(rat)), 3),
                   p25=round(float(np.percentile(rat, 25)), 3), p75=round(float(np.percentile(rat, 75)), 3),
                   amplitude_ratio=round(10 ** (-delta / 20), 3), power_ratio=round(10 ** (-delta / 10), 3)))
with open(c.RES + "/syn_quiet_vs_loud.csv", "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=list(qd[0])); w.writeheader(); w.writerows(qd)
