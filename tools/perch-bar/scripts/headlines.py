"""Print the headline numbers straight from results/*.csv (so that nothing is retyped by hand)."""
import sys, os, csv
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common as c
def rd(n): return list(csv.DictReader(open(f"{c.RES}/{n}.csv")))
def ci(r, a, b, d, scale=100, nd=0):
    return f"{scale*float(r[a]):.{nd}f} ({scale*float(r[b]):.{nd}f}-{scale*float(r[d]):.{nd}f})"
print("== SYNTHETIC: mean share by condition (all pairs) | ratio to alone | top-1 %")
for r in rd("syn_share_by_condition"):
    if r["subset"] == "all" and r["cfg"] in ("single", "eq", "d10", "d20", "ladder"):
        print(f"  n={r['n_species']} {r['cfg']:6s} {r['role']:9s} pairs {r['n_pairs']:4s} mean {float(r['mean_share']):.2f} ({float(r['mean_ci_lo']):.2f}-{float(r['mean_ci_hi']):.2f}) ratio {r['mean_ratio_to_alone']} top1 {100*float(r['frac_rank1']):.0f}%")
print("== SYNTHETIC: total share of present species"); [print("  ", r["n_species"], r["cfg"], r["mean_total_share_of_present"], r["lo"], r["hi"]) for r in rd("syn_total_present_share")]
print("== SYNTHETIC: level sweep n=1"); [print("  ", r["level_dbfs"], r["mean_share"], r["recall@0.35"]) for r in rd("syn_level_sweep_single")]
print("== SYNTHETIC: quiet vs loud"); [print("  ", r) for r in rd("syn_quiet_vs_loud")]
sw = rd("syn_bar_sweep")
print("== SYNTHETIC recall % (bootstrap CI by segment) / FR species per window (CI)  [subset all | alone_ok]")
for sub in ("all", "alone_ok"):
    for key in [("1", "single"), ("2", "eq"), ("2", "d20"), ("3", "eq"), ("3", "d20"), ("4", "eq"), ("4", "d20")]:
        for t in ("0.1", "0.2", "0.35", "0.5"):
            r = next((x for x in sw if x["subset"] == sub and (x["n_species"], x["cfg"]) == key and x["bar"] == t), None)
            if r: print(f"  {sub:8s} n={key[0]} {key[1]:6s} bar {t:4s} pairs {r['n_pairs']:4s} recall {ci(r,'recall','recall_boot_lo','recall_boot_hi',100)}  FR {float(r['fr_species_per_window']):.2f} ({float(r['fr_lo']):.2f}-{float(r['fr_hi']):.2f})")
print("== REAL: inventory"); [print("  ", r) for r in rd("real_inventory")]
rs = rd("real_bar_sweep")
print("== REAL recall % (CI by recording) / FR per window (CI)")
for g in ("all datasets, n annotated species = 1", "all datasets, n annotated species = 2", "all datasets, n annotated species >= 3", "all datasets, n annotated species >= 2", "all datasets, n annotated species >= 1"):
    for t in ("0.1", "0.2", "0.35", "0.5"):
        r = next((x for x in rs if x["group"] == g and x["bar"] == t), None)
        if r and r.get("recall"): print(f"  {g[15:]:34s} bar {t:4s} pairs {r['n_pairs']:5s} recall {ci(r,'recall','recall_boot_lo','recall_boot_hi',100)} | FR {float(r['fr_species_per_window']):.3f} ({float(r['fr_boot_lo']):.3f}-{float(r['fr_boot_hi']):.3f}) windows {r['n_windows_fr']} | oracle-regional FR {float(r['fr_regional_oracle_per_window']):.3f}")
print("== REAL per set (n>=1) recall% at 0.1/0.35 and FR at 0.35")
vals = {}
for tag in ("CEB", "POW", "SNE", "SSW", "COR", "PER", "KEN", "HAW"):
    for t in ("0.1", "0.35"):
        r = next((x for x in rs if x["group"] == f"{tag}, n >= 1" and x["bar"] == t), None)
        if r and r.get("recall"):
            vals.setdefault(t, []).append(float(r["recall"]))
            print(f"  {tag} bar {t}: pairs {r['n_pairs']} files {r['n_files']} recall {100*float(r['recall']):.0f}% FR {float(r.get('fr_species_per_window','nan')):.3f}")
for t, v in vals.items(): print(f"  set-balanced mean recall at bar {t}: {100*sum(v)/len(v):.1f}% over {len(v)} sets")
em = rd("real_empty_windows_bar")
print("== REAL windows with no annotation: % with any SPECIES class >= bar (CI by recording); any class; oracle regional")
for t in ("0.1", "0.2", "0.35", "0.5", "0.7"):
    r = next(x for x in em if x["group"].startswith("all datasets") and x["bar"] == t)
    print(f"  bar {t}: n={r['n_windows']} files {r['n_files']} species {ci(r,'frac_any_species_class_ge_bar','frac_any_species_class_ge_bar_boot_lo','frac_any_species_class_ge_bar_boot_hi',100,1)} | any {ci(r,'frac_any_class_ge_bar','frac_any_class_ge_bar_boot_lo','frac_any_class_ge_bar_boot_hi',100,1)} | regional {ci(r,'frac_windows_regional_oracle_class_ge_bar','regional_boot_lo','regional_boot_hi',100,1)}")
nb = rd("noise_bar")
print("== NOISE (ESC-50 real clips, Wilson)")
for t in ("0.1", "0.2", "0.35", "0.5", "0.7"):
    r = next(x for x in nb if x["group"].startswith("ESC-50 real clips") and x["model"].startswith("Perch") and x["bar"] == t)
    print(f"  bar {t}: n={r['n_windows']} any class {ci(r,'frac_any_class','frac_any_class_wilson_lo','frac_any_class_wilson_hi',100,1)} | species class {ci(r,'frac_any_species_class','frac_any_species_class_wilson_lo','frac_any_species_class_wilson_hi',100,1)}")
print("== LABEL-RULE SENSITIVITY (n>=1)")
for r in rd("real_label_sensitivity"):
    if r["group"] == "n>=1" and r["bar"] in ("0.1", "0.35"): print(f"  {r['label_rule']:8s} bar {r['bar']}: recall {ci(r,'recall','recall_lo','recall_hi',100)} FR {float(r['fr_species_per_window']):.3f}")
print("== FALSE-REPORT TYPES"); [print("  ", r) for r in rd("false_report_types")]
print("== BirdNET+ vs Perch at MATCHED false-report rate (n annotated >= 1; own class space; lowest bar on the grid with FR <= target)")
bp = rd("bnp_vs_perch")
for g in ("n annotated >= 1", "n annotated = 1", "n annotated >= 3"):
    for target in (0.03, 0.05, 0.10, 0.25):
        out = []
        for model in ("Perch v2", "BirdNET+ V3.0"):
            rs_ = sorted([r for r in bp if r["group"] == g and r["model"] == model], key=lambda r: float(r["bar"]))
            ok = [r for r in rs_ if float(r["fr_own_classes_per_window"]) <= target]
            if ok: r = ok[0]; out.append(f"{model}: bar {r['bar']} recall {100*float(r['recall']):.1f}% ({100*float(r['recall_boot_lo']):.0f}-{100*float(r['recall_boot_hi']):.0f}) FR {float(r['fr_own_classes_per_window']):.3f}")
            else: out.append(f"{model}: no bar on the grid")
        print(f"  {g:18s} FR<={target}: " + " | ".join(out))
print("== BirdNET+ vs Perch: windows with no annotation, any class >= bar")
for r in rd("bnp_vs_perch_empty_windows"):
    if r["bar"] in ("0.1", "0.35", "0.5", "0.7", "0.9"): print("  ", r["model"], r["bar"], f"{100*float(r['frac_any_class_ge_bar']):.1f}% ({100*float(r['wilson_lo']):.1f}-{100*float(r['wilson_hi']):.1f}) n={r['n_windows']}")
