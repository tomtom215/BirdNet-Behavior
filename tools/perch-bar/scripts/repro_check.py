"""Reproducibility: rebuild some mixtures from the seeds and compare their Perch logits with the stored ones; re-run Perch on a few stored real windows."""
import sys, os, csv, collections
import numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common as c
import run_syn as rs

S = np.load(c.WORK + "/syn/segments.npy"); cat = list(csv.DictReader(open(c.WORK + "/syn/catalogue.csv")))
act = [float(r["active_rms_dbfs"]) for r in cat]
by_sp = collections.defaultdict(list)
for r in cat: by_sp[r["species"]].append(int(r["seg_id"]))
species = sorted(by_sp)
M = list(csv.DictReader(open(c.WORK + "/syn/mix_windows.csv"))); MZ = np.load(c.WORK + "/syn/mix_perch.npy")
psess, _ = c.sessions(2, bnp=False)
worst = 0.0; same_comp = True
for n, cfg in ((2, "eq"), (3, "ladder"), (4, "d20")):
    rng = rs.rng_for(n, cfg)
    batch, idx = [], []
    for d in range(8):
        sp = list(rng.choice(species, size=n, replace=False)); sids = [int(rng.choice(by_sp[s])) for s in sp]
        rel = list(rng.permutation(rs.rel_levels(n, cfg)))
        mix = np.zeros(c.N_WIN, np.float32)
        for sid, rl in zip(sids, rel): mix += rs.scaled(S[sid], act[sid], rs.REF + rl)
        row = next(r for r in M if int(r["n_species"]) == n and r["cfg"] == cfg and int(r["draw"]) == d)
        same_comp &= (row["seg_ids"] == "|".join(map(str, sids))) and (row["rel_db"] == "|".join(f"{r:g}" for r in rel))
        batch.append(np.clip(mix, -1, 1)); idx.append(int(row["win_id"]))
    z = c.perch_logits(psess, np.stack(batch))
    worst = max(worst, float(np.abs(z - MZ[idx]).max()))
print("first 8 draws of cells (2,eq), (3,ladder), (4,d20): same composition from seeds:", same_comp, "| max |logit difference| on re-running Perch:", worst)
R = list(csv.DictReader(open(c.WORK + "/real/SNE_windows.csv"))); RZ = np.load(c.WORK + "/real/SNE_perch.npy")
x, sr, dur = c.load_audio_32k(c.DATA + "/z7525805_sne_audio/" + R[0]["file"])
ks = [0, 7, 100, 119]
z = c.perch_logits(psess, np.stack([x[k * c.N_WIN:(k + 1) * c.N_WIN] for k in ks]))
print("real SNE windows", ks, "max |logit difference| after re-decoding and re-running:", float(np.abs(z - RZ[ks]).max()))
