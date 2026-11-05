"""Task 2: controlled mixtures BY CONSTRUCTION from the clean single-species segments in work/syn/ (build_catalogue.py).

Level convention: each source segment is scaled so that its ACTIVE-RMS (RMS over the time spans covered by its
annotated boxes, full band) equals the requested dBFS.  The loudest component of a mixture sits at REF (-30 dBFS);
the others sit lower by the configured difference.  Mixture = sum of the scaled 5 s segments (no other processing).

 n = 1 : every segment alone at -20,-30,-40,-50,-60 dBFS   (the 'alone' look-up table; also the level sweep)
 n = 2 : equal | d10 (one at 0, other -10 dB) | d20
 n = 3,4 : equal | d10 (one at 0, rest -10) | d20 (one at 0, rest -20) | ladder (0,-10,-20,-30)
DRAWS draws per (n, config); species drawn without replacement from the 16-species pool, then one segment of each.
Seeds: sha256 of a fixed string per cell -> numpy default_rng.  Writes work/syn/{mix_windows.csv, mix_perch.npy,
alone_windows.csv, alone_perch.npy}.  Perch logits are stored as float32.
"""
import sys, os, csv, hashlib, collections
import numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common as c

REF = -30.0
LEVELS_ALONE = [-20.0, -30.0, -40.0, -50.0, -60.0]
DRAWS = 100
CELLS = [(2, "eq"), (2, "d10"), (2, "d20"), (3, "eq"), (3, "d10"), (3, "d20"), (3, "ladder"),
         (4, "eq"), (4, "d10"), (4, "d20"), (4, "ladder")]

def rel_levels(n, cfg):
    return {"eq": [0.0] * n, "d10": [0.0] + [-10.0] * (n - 1), "d20": [0.0] + [-20.0] * (n - 1),
            "ladder": [0.0, -10.0, -20.0, -30.0][:n]}[cfg]

def rng_for(*parts):
    return np.random.default_rng(int(hashlib.sha256(("bnb-multispecies-v1|syn|" + "|".join(map(str, parts))).encode()).hexdigest()[:16], 16))

def scaled(seg, act_rms_dbfs, target_dbfs):
    return seg * np.float32(10 ** ((target_dbfs - act_rms_dbfs) / 20))

if __name__ == "__main__":
    S = np.load(c.WORK + "/syn/segments.npy")
    cat = list(csv.DictReader(open(c.WORK + "/syn/catalogue.csv")))
    act = [float(r["active_rms_dbfs"]) for r in cat]
    by_sp = collections.defaultdict(list)
    for r in cat: by_sp[r["species"]].append(int(r["seg_id"]))
    species = sorted(by_sp)
    psess, _ = c.sessions(2, bnp=False)

    # ---- alone table
    A_rows, A_batch = [], []
    for sid in range(len(cat)):
        for L in LEVELS_ALONE:
            a = scaled(S[sid], act[sid], L); pk = float(np.abs(a).max())
            A_rows.append(dict(seg_id=sid, species=cat[sid]["species"], perch_idx=cat[sid]["perch_idx"], level_dbfs=L,
                               peak=round(pk, 4), clipped=int(pk > 1.0)))
            A_batch.append(np.clip(a, -1.0, 1.0))  # a window whose peak exceeds full scale is hard-clipped and flagged
    A_logits = np.concatenate([c.perch_logits(psess, np.stack(A_batch[i:i + 8])) for i in range(0, len(A_batch), 8)])
    np.save(c.WORK + "/syn/alone_perch.npy", A_logits)
    with open(c.WORK + "/syn/alone_windows.csv", "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=list(A_rows[0])); w.writeheader(); w.writerows(A_rows)
    print("alone windows:", len(A_rows), flush=True)

    # ---- mixtures
    M_rows, M_batch = [], []
    for n, cfg in CELLS:
        rng = rng_for(n, cfg)
        for d in range(DRAWS):
            sp = list(rng.choice(species, size=n, replace=False))
            sids = [int(rng.choice(by_sp[s])) for s in sp]
            rel = list(rng.permutation(rel_levels(n, cfg)))
            mix = np.zeros(c.N_WIN, np.float32)
            for sid, rl in zip(sids, rel):
                mix += scaled(S[sid], act[sid], REF + rl)
            peak = float(np.abs(mix).max())
            M_batch.append(np.clip(mix, -1.0, 1.0))  # hard-clipped and flagged if the peak exceeds full scale
            M_rows.append(dict(win_id=len(M_rows), n_species=n, cfg=cfg, draw=d,
                               seg_ids="|".join(map(str, sids)), species="|".join(sp),
                               perch_idx="|".join(cat[s]["perch_idx"] for s in sids),
                               rel_db="|".join(f"{r:g}" for r in rel),
                               level_dbfs="|".join(f"{REF + r:g}" for r in rel),
                               rms_dbfs=round(20 * np.log10(float(np.sqrt((mix.astype(np.float64) ** 2).mean()))), 2),
                               peak=round(peak, 4), clipped=int(peak > 1.0)))
    M_logits = np.concatenate([c.perch_logits(psess, np.stack(M_batch[i:i + 8])) for i in range(0, len(M_batch), 8)])
    np.save(c.WORK + "/syn/mix_perch.npy", M_logits)
    with open(c.WORK + "/syn/mix_windows.csv", "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=list(M_rows[0])); w.writeheader(); w.writerows(M_rows)
    print("mixture windows:", len(M_rows), flush=True)
