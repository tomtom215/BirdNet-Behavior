"""Render the CSVs in results/ as plain-text pipe tables (results/tables/*.txt).  Pure formatting; no new numbers."""
import sys, os, csv, collections
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common as c

OUT = c.RES + "/tables"; os.makedirs(OUT, exist_ok=True)
BARS = ["0.05", "0.1", "0.15", "0.2", "0.25", "0.3", "0.35", "0.4", "0.5", "0.7"]

def rd(name): return list(csv.DictReader(open(f"{c.RES}/{name}.csv")))
def pct(x, nd=0): return "" if x in ("", None) else f"{100*float(x):.{nd}f}"
def ci(p, lo, hi, nd=0):
    if p in ("", None): return ""
    return f"{100*float(p):.{nd}f} ({100*float(lo):.{nd}f}-{100*float(hi):.{nd}f})"
def md(header, rows, align=None):
    s = "| " + " | ".join(header) + " |\n|" + "|".join(["---"] * len(header)) + "|\n"
    for r in rows: s += "| " + " | ".join(str(x) for x in r) + " |\n"
    return s
def write(name, text): open(f"{OUT}/{name}.txt", "w").write(text)   # plain-text pipe tables (the harness forbids report .md files)

# ---- synthetic: share by condition
rows = [r for r in rd("syn_share_by_condition") if r["subset"] == "all"]
write("syn_share", md(["n species", "levels", "role", "pairs", "mean share (95% CI by segment)", "median", "10th-90th pct", "mean share / share alone", "top-1 %", "top-3 %"],
      [[r["n_species"], r["cfg"], r["role"], r["n_pairs"], f"{float(r['mean_share']):.2f} ({float(r['mean_ci_lo']):.2f}-{float(r['mean_ci_hi']):.2f})", f"{float(r['median_share']):.2f}",
        f"{float(r['p10']):.2f}-{float(r['p90']):.2f}", f"{float(r['mean_ratio_to_alone']):.2f}", pct(r["frac_rank1"]), pct(r["frac_rank_le3"])] for r in rows]))
rows = [r for r in rd("syn_share_by_condition") if r["subset"] != "all"]
write("syn_share_alone_ok", md(["n species", "levels", "role", "pairs", "mean share (95% CI by segment)", "median", "mean share / share alone", "top-1 %"],
      [[r["n_species"], r["cfg"], r["role"], r["n_pairs"], f"{float(r['mean_share']):.2f} ({float(r['mean_ci_lo']):.2f}-{float(r['mean_ci_hi']):.2f})", f"{float(r['median_share']):.2f}",
        f"{float(r['mean_ratio_to_alone']):.2f}", pct(r["frac_rank1"])] for r in rows]))

# ---- synthetic: bar sweep
sw = rd("syn_bar_sweep")
for sub in ("all", "alone_ok"):
    cells = []
    for r in sw:
        k = (r["n_species"], r["cfg"])
        if r["subset"] == sub and k not in cells: cells.append(k)
    rec_rows, fr_rows = [], []
    for k in cells:
        rs = [r for r in sw if r["subset"] == sub and (r["n_species"], r["cfg"]) == k]
        rec_rows.append([k[0], k[1], rs[0]["n_pairs"], rs[0]["n_windows"]] + [pct(r["recall"]) for r in rs])
        fr_rows.append([k[0], k[1], rs[0]["n_windows"]] + [f"{float(r['fr_species_per_window']):.2f}" for r in rs])
    write(f"syn_recall_{sub}", md(["n", "levels", "pairs", "windows"] + BARS, rec_rows))
    write(f"syn_fr_{sub}", md(["n", "levels", "windows"] + BARS, fr_rows))
    ci_rows = []
    for k in cells:
        for t in ("0.1", "0.2", "0.35", "0.5"):
            r = next(x for x in sw if x["subset"] == sub and (x["n_species"], x["cfg"]) == k and x["bar"] == t)
            ci_rows.append([k[0], k[1], t, ci(r["recall"], r["recall_boot_lo"], r["recall_boot_hi"]), f"{100*float(r['recall_wilson_lo']):.0f}-{100*float(r['recall_wilson_hi']):.0f}",
                            f"{float(r['fr_species_per_window']):.2f} ({float(r['fr_lo']):.2f}-{float(r['fr_hi']):.2f})", pct(r["frac_windows_with_false_species"])])
    write(f"syn_recall_ci_{sub}", md(["n", "levels", "bar", "recall % (bootstrap CI by segment)", "Wilson CI", "false species classes / window (CI)", "% windows with >=1 false species"], ci_rows))
write("syn_absent_fsd", md(["n", "levels", "windows", "largest absent share: median", "90th pct", "max", "FSD50K share: mean", "90th pct"],
      [[r["n_species"], r["cfg"], r["n_windows"], r["max_absent_median"], r["max_absent_p90"], r["max_absent_max"], r["fsd_share_mean"], r["fsd_share_p90"]] for r in rd("syn_absent_and_fsd")]))
write("syn_level_sweep", md(["active-RMS level (dBFS)", "windows", "clipped (excluded)", "mean share", "median", "10th pct", "recall@0.10 %", "@0.35 %", "@0.50 %"],
      [[r["level_dbfs"], r["n_windows"], r["n_clipped"], r["mean_share"], r["median_share"], r["p10"], pct(r["recall@0.1"]), pct(r["recall@0.35"]), pct(r["recall@0.5"])] for r in rd("syn_level_sweep_single")]))

# ---- real
if os.path.exists(c.RES + "/real_inventory.csv"):
    inv = rd("real_inventory")
    write("real_inventory", md(["set", "files", "hours", "windows", "no annotation", "1 sp", "2 sp", "3 sp", "4 sp", "5+ sp", "with '????' box", "with a species lacking a Perch class", "species-window pairs"],
          [[r["dataset"], r["files"], r["hours"], r["windows"], r["w_empty"], r["w_1"], r["w_2"], r["w_3"], r["w_4"], r["w_5plus"], r["w_unknown_box"], r["w_with_unmapped_species"], r["pairs"]] for r in inv]))
    sw = rd("real_bar_sweep")
    groups = []
    for r in sw:
        if r["group"] not in groups: groups.append(r["group"])
    rec_rows, fr_rows = [], []
    for g in groups:
        rs = [r for r in sw if r["group"] == g]
        rec_rows.append([g, rs[0]["n_pairs"], rs[0]["n_files"]] + [pct(r.get("recall", "")) for r in rs])
        fr_rows.append([g, rs[0]["n_windows_fr"]] + [("" if r.get("fr_species_per_window", "") == "" else f"{float(r['fr_species_per_window']):.2f}") for r in rs])
    write("real_recall", md(["group", "pairs", "files"] + BARS, rec_rows))
    write("real_fr", md(["group", "windows"] + BARS, fr_rows))
    ci_rows = []
    for g in groups:
        if not (g.startswith("all datasets")): continue
        for t in ("0.1", "0.2", "0.35", "0.5"):
            r = next(x for x in sw if x["group"] == g and x["bar"] == t)
            ci_rows.append([g, t, r["n_pairs"], ci(r.get("recall", ""), r.get("recall_boot_lo", ""), r.get("recall_boot_hi", "")),
                            "" if "fr_species_per_window" not in r or r["fr_species_per_window"] == "" else f"{float(r['fr_species_per_window']):.2f} ({float(r['fr_boot_lo']):.2f}-{float(r['fr_boot_hi']):.2f})",
                            "" if r.get("fr_regional_oracle_per_window", "") == "" else f"{float(r['fr_regional_oracle_per_window']):.2f} ({float(r['fr_reg_boot_lo']):.2f}-{float(r['fr_reg_boot_hi']):.2f})",
                            pct(r.get("frac_windows_with_false_species", ""))])
    write("real_recall_ci", md(["group", "bar", "pairs", "recall % (bootstrap CI by recording)", "false species classes / window (CI)", "same, restricted to the dataset's own species list (oracle)", "% windows with >=1 false species"], ci_rows))
    dist = rd("real_share_by_n_present")
    write("real_share", md(["annotated species in window", "pairs", "mean share (95% CI by recording)", "10th pct", "median", "90th pct", "top-1 %", "top-3 %", "top-10 %"],
          [[r["n_annotated_species"], r["n_pairs"], f"{float(r['mean_share']):.2f} ({float(r['mean_boot_lo']):.2f}-{float(r['mean_boot_hi']):.2f})", r["p10"], r["median"], r["p90"], pct(r["frac_rank1"]), pct(r["frac_rank_le3"]), pct(r["frac_rank_le10"])] for r in dist]))
    em = rd("real_empty_windows_bar")
    groups = []
    for r in em:
        if r["group"] not in groups: groups.append(r["group"])
    rows1, rows2 = [], []
    for g in groups:
        rs = [r for r in em if r["group"] == g]
        rows1.append([g, rs[0]["n_windows"], rs[0]["n_files"]] + [pct(r["frac_any_species_class_ge_bar"], 1) for r in rs])
        rows2.append([g, rs[0]["n_windows"]] + [pct(r["frac_any_class_ge_bar"], 1) for r in rs])
    write("real_empty_species", md(["group", "windows", "files"] + BARS, rows1))
    write("real_empty_any", md(["group", "windows"] + BARS, rows2))
    ci_rows = []
    for g in groups[:1]:
        for t in BARS:
            r = next(x for x in em if x["group"] == g and x["bar"] == t)
            ci_rows.append([t, ci(r["frac_any_species_class_ge_bar"], r["frac_any_species_class_ge_bar_boot_lo"], r["frac_any_species_class_ge_bar_boot_hi"], 1),
                            ci(r["frac_any_class_ge_bar"], r["frac_any_class_ge_bar_boot_lo"], r["frac_any_class_ge_bar_boot_hi"], 1),
                            ci(r["frac_windows_regional_oracle_class_ge_bar"], r["regional_boot_lo"], r["regional_boot_hi"], 1)])
    write("real_empty_ci", md(["bar", "% windows with any SPECIES class >= bar (CI by recording)", "% windows with any class >= bar (species or FSD50K)", "% windows with a class on the dataset's own species list >= bar (oracle)"], ci_rows))
if os.path.exists(c.RES + "/noise_bar.csv"):
    nb = rd("noise_bar")
    groups = []
    for r in nb:
        if r["group"] not in groups: groups.append(r["group"])
    rows = []
    for g in groups:
        rs = [r for r in nb if r["group"] == g and r["model"].startswith("Perch")]
        rows.append([g, rs[0]["n_windows"]] + [pct(r["frac_any_class"]) for r in rs])
    write("noise_any", md(["group", "windows"] + BARS, rows))
    rows = []
    for g in groups:
        rs = [r for r in nb if r["group"] == g and r["model"].startswith("Perch")]
        rows.append([g, rs[0]["n_windows"]] + [pct(r["frac_any_species_class"]) for r in rs])
    write("noise_species", md(["group", "windows"] + BARS, rows))
    g = groups[0]
    rows = []
    for t in BARS:
        r = next(x for x in nb if x["group"] == g and x["model"].startswith("Perch") and x["bar"] == t)
        rows.append([t, ci(r["frac_any_class"], r["frac_any_class_wilson_lo"], r["frac_any_class_wilson_hi"], 1), ci(r["frac_any_species_class"], r["frac_any_species_class_wilson_lo"], r["frac_any_species_class_wilson_hi"], 1)])
    write("noise_ci", md(["bar", "% windows with any class >= bar (Wilson CI)", "% windows with any species class >= bar (Wilson CI)"], rows))
print("tables written to", OUT)
