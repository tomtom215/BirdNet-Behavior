"""Known-answer checks of the measurement code (run before trusting any table).  Raises AssertionError on the first failure."""
import sys, os
import numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import metrics as m
import events as ev
import taxon

# 1. window labelling (every expectation worked out by hand) -------------------------------------------
E = [(1.0, 2.0, "A"), (4.9, 5.5, "B"), (5.2, 9.0, "C"), (10.5, 10.5, "D"), (3.0, 4.0, "unknown")]
pres, edge, unk = ev.label_window(E, 0.0, 5.0)
assert pres == {"A"}, pres          # A 1.0 s inside; B overlaps 0.1 s < min(0.3, 1.0); C starts 0.2 s after the window
assert edge == {"B", "C"}, edge     # both are within the 0.5 s margin
assert unk is True                  # the unidentifiable box [3,4] is wholly inside
pres, edge, unk = ev.label_window(E, 5.0, 10.0)
assert pres == {"B", "C"}, pres     # B overlaps 0.5 s >= 0.3 ; C overlaps 3.8 s >= 1.0
assert edge == {"D"}, edge          # point event at 10.5 padded to 0.3 s, widened by 0.5 s reaches 9.85 < 10
assert unk is False

# a species present through one event and 'edge' through another is present only
pres, edge, unk = ev.label_window([(1.0, 2.0, "A"), (5.1, 5.3, "A")], 0.0, 5.0)
assert pres == {"A"} and edge == set(), (pres, edge)
# a long event that overlaps the window by only 0.5 s is NOT present (the core overlap is capped at 1.0 s, not 0.1 s)
pres, edge, unk = ev.label_window([(4.5, 20.0, "L")], 0.0, 5.0)
assert pres == set() and edge == {"L"}, (pres, edge)
pres, edge, unk = ev.label_window([(3.5, 20.0, "L")], 0.0, 5.0)         # 1.5 s overlap >= 1.0 s -> present
assert pres == {"L"}, pres

# the 'lenient' and 'strict' rules used by analyse_sensitivity.py: an event 20 s away must never count (a zero threshold once made every event present everywhere)
E2 = [(4.9, 5.5, "B"), (20.0, 21.0, "far")]
assert ev.label_window(E2, 0.0, 5.0, core_frac=1.0, core_cap=0.05)[0] == {"B"}      # 0.1 s overlap >= 0.05 s
assert ev.label_window(E2, 0.0, 5.0)[0] == set()
assert ev.label_window(E2, 0.0, 5.0, core_frac=1.0, core_cap=2.0)[0] == set()

# 2. intervals --------------------------------------------------------------------------------------
p, lo, hi = m.wilson(0, 120); assert p == 0 and abs(hi - 0.0310) < 0.0005, (p, lo, hi)      # known: 0/120 -> upper 3.1 %
p, lo, hi = m.wilson(50, 100); assert abs(lo - 0.4038) < 0.001 and abs(hi - 0.5962) < 0.001
num = np.array([1, 0, 1, 0, 1, 1, 0, 0.]); den = np.ones(8); cl = np.arange(8)
p, lo, hi = m.cluster_ratio_ci(num, den, cl, B=4000, seed=1)
assert p == 0.5 and abs(lo - 0.125) < 1e-9 and abs(hi - 0.875) < 1e-9, (p, lo, hi)   # 8 binary clusters, 2.5th/97.5th percentiles = 1/8, 7/8
p, lo, hi = m.cluster_ratio_ci(np.array([3., 3, 3, 3]), np.array([6., 6, 6, 6]), np.arange(4), B=500)
assert p == lo == hi == 0.5          # identical clusters -> zero width

# 3. window_table on hand-built logits ---------------------------------------------------------------
C = 100
share = np.full(C, 1e-6); share[:6] = [0.52, 0.27, 0.11, 0.06, 0.02, 0.008]; share[98] = 0.004; share[99] = 0.002
share = share / share.sum()
isb = np.ones(C, bool); isb[98:] = False                                    # classes 98, 99 are 'FSD' classes
Z = np.log(share)[None, :].repeat(2, 0).astype(np.float32)
Z[1] = Z[1][::-1][::-1]
T = m.window_table(Z, [[0], [1]], [[], [2]], isb)
assert abs(T["pair_share"][0] - share[0]) < 1e-5 and T["pair_rank"][0] == 1
assert T["pair_rank"][1] == 2
B = m.BARS
j25, j10, j05 = B.index(0.25), B.index(0.10), B.index(0.05)
assert T["fr_sp"][0, j25] == 1          # only class 1 (0.27) is absent and >= 0.25
assert T["fr_sp"][0, j10] == 2          # classes 1 and 2
assert T["fr_sp"][0, j05] == 3          # classes 1, 2, 3 (0.06)
assert T["fr_sp"][1, j10] == 1          # window 1: present = {1}, edge = {2} -> only class 0 (0.52) is a false report
assert abs(T["fsd_total"][0] - (share[98] + share[99])) < 1e-6
assert T["fr_fsd"].sum() == 0           # FSD classes (0.004, 0.002) are below the lowest bar
assert abs(T["max_absent"][0] - share[1]) < 1e-5 and abs(T["max_absent"][1] - share[0]) < 1e-5
assert abs(T["max_absent_strict"][1] - share[0]) < 1e-5
assert T["any_ge"][0, B.index(0.5)] and not T["any_ge"][0, B.index(0.7)]

rm = np.zeros(C, bool); rm[1] = True                                       # 'oracle local list' contains only class 1
T2 = m.window_table(Z, [[0], [1]], [[], [2]], isb, [rm, rm])
assert T2["fr_reg"][0, j10] == 1 and T2["fr_sp"][0, j10] == 2           # class 2 is a false report overall but not on the local list
assert T2["fr_reg"][1, j10] == 0                                          # window 1 has class 1 present, so nothing on the list is absent

# 4. mapper ------------------------------------------------------------------------------------------
sci = ["Picoides arcticus", "Dryobates pubescens", "Turdus merula", "Wind"]
mp = taxon.Mapper(sci, np.array([True, True, True, False]), ["Turdus merula"])
assert mp.perch_idx("Turdus merula") == (2, "exact")
assert mp.perch_idx("Picoides pubescens") == (1, "synonym")       # hand-written synonym whose target exists
assert mp.perch_idx("Picoides villosus") == (None, "none")        # same genus as a Perch class, but a different species: never substituted
assert mp.perch_idx("unknown") == (None, "unknown")
assert mp.perch_idx("Wind") == (None, "none")                     # a sound class is not a species
mp2 = taxon.Mapper(["Picoides arcticus", "Turdus merula"], np.array([True, True]), [])
assert mp2.perch_idx("Picoides pubescens") == (None, "none")      # a synonym whose target is not in the list is not used
assert mp.bnp_idx("Turdus merula") == 0 and mp.bnp_idx("Dryobates pubescens") is None
print("selftest OK")
