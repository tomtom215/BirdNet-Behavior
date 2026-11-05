"""Run Perch v2 and BirdNET+ V3.0 over every complete 5 s window (hop 5 s) of the downloaded annotated soundscapes.

usage: python -I run_real.py [DATASET_TAG ...]    (default: all present)
Writes   work/real/<TAG>_windows.csv   one row per window (labels, level, bookkeeping)
         work/real/<TAG>_perch.npy     (n,14795) float32 Perch logits   (shares are recomputed by analyse.py)
         work/real/<TAG>_bnp.npy       (n,11560) float16 BirdNET+ probabilities, computed on the 4.5 s centred in the window
Threads: onnxruntime intra_op_num_threads=2.
"""
import sys, os, csv, json, time
import numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common as c
import events as ev
from taxon import Mapper

tags = sys.argv[1:]
perch_sci, perch_isb = c.load_perch_labels()
bnp_sci, _ = c.load_bnp_labels()
mp = Mapper(perch_sci, perch_isb, bnp_sci)
psess, bsess = c.sessions(2)
os.makedirs(c.WORK + "/real", exist_ok=True)
files = [f for f in ev.real_files() if not tags or f[0] in tags]
bytag = {}
for t, p, e in files: bytag.setdefault(t, []).append((p, e))

HOP = N = c.N_WIN
B_OFF, B_LEN = int(0.25 * c.SR), int(4.5 * c.SR)
for tag, lst in bytag.items():
    rows, PL, BL = [], [], []
    t_start = time.time()
    for path, events in lst:
        x, sr, dur = c.load_audio_32k(path)
        nwin = len(x) // N
        base = os.path.basename(path)
        meta = []
        for k in range(nwin):
            t0 = k * 5.0
            pres, edge, unk = ev.label_window(events, t0, t0 + 5.0)
            pmap, pbasis = {}, {}
            for s in pres:
                i, b = mp.perch_idx(s); pbasis[s] = b
                if i is not None: pmap[s] = i
            emap = [mp.perch_idx(s)[0] for s in edge if mp.perch_idx(s)[0] is not None]
            seg = x[k * N:(k + 1) * N]
            meta.append(dict(
                dataset=tag, file=base, t0=t0, src_sr=sr,
                rms_dbfs=round(20 * np.log10(max(float(np.sqrt((seg.astype(np.float64) ** 2).mean())), 1e-9)), 2),
                present_names="|".join(sorted(pres)), n_present_all=len(pres),
                present_perch="|".join(str(i) for i in sorted(set(pmap.values()))),
                n_present_mapped=len(set(pmap.values())),
                n_present_unmapped=sum(1 for s in pres if s not in pmap),
                unmapped_names="|".join(sorted(s for s in pres if s not in pmap)),
                edge_perch="|".join(str(i) for i in sorted(set(emap))),
                n_edge_all=len(edge), has_unknown=int(unk),
                present_bnp="|".join(str(j) for j in sorted({mp.bnp_idx(s) for s in pres if mp.bnp_idx(s) is not None}))))
        for i in range(0, nwin, 8):
            segs = np.stack([x[k * N:(k + 1) * N] for k in range(i, min(i + 8, nwin))])
            PL.append(c.perch_logits(psess, segs).astype(np.float32))
            BL.append(c.bnp_probs(bsess, segs[:, B_OFF:B_OFF + B_LEN]).astype(np.float16))
        rows += meta
        print(f"{tag} {base}: {nwin} windows ({dur:.0f} s decoded, src {sr} Hz)  [{time.time()-t_start:.0f}s]", flush=True)
    if not rows: continue
    np.save(f"{c.WORK}/real/{tag}_perch.npy", np.concatenate(PL))
    np.save(f"{c.WORK}/real/{tag}_bnp.npy", np.concatenate(BL))
    with open(f"{c.WORK}/real/{tag}_windows.csv", "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0])); w.writeheader(); w.writerows(rows)
    print(f"== {tag}: {len(rows)} windows done in {time.time()-t_start:.0f}s", flush=True)
