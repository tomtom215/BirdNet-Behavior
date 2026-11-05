"""Catalogue of clean single-species 5 s source segments for the controlled mixtures (Task 2).

Source: CEB 'train_xenocanto' (Xeno-canto focal recordings re-annotated with bounding boxes; Zenodo 21821133).
A file is a *single-species file* when the species column has exactly one distinct value over all of the file's boxes.
Candidate windows: 5 s windows on a 1 s grid, fully inside the audio, whose boxes cover >= 1.0 s (union) of the window.
Per species, windows are ranked by sha256(seed|file|t0) and at most 4 per file / no two windows closer than 5 s are kept.
Outputs work/syn/segments.npy (n,160000 float32), work/syn/catalogue.csv.
"""
import sys, os, csv, glob, hashlib, collections
import numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common as c
from taxon import Mapper

SEED = "bnb-multispecies-v1|syn"
rows = list(csv.DictReader(open(f"{c.DATA}/z21821133_ceb_trainxc_csv/train_xenocanto.csv", encoding="utf-8")))
byf = collections.defaultdict(list)
for r in rows: byf[r["filepath"]].append(r)
perch_sci, perch_isb = c.load_perch_labels(); bnp_sci, _ = c.load_bnp_labels()
mp = Mapper(perch_sci, perch_isb, bnp_sci)
os.makedirs(c.WORK + "/syn", exist_ok=True)

def union_len(iv, w0, w1):
    iv = sorted((max(a, w0), min(b, w1)) for a, b in iv if min(b, w1) > max(a, w0))
    tot, cur0, cur1 = 0.0, None, None
    for a, b in iv:
        if cur1 is None or a > cur1:
            if cur1 is not None: tot += cur1 - cur0
            cur0, cur1 = a, b
        else: cur1 = max(cur1, b)
    return tot + (cur1 - cur0 if cur1 is not None else 0.0)

segs, cat = [], []
for path in sorted(glob.glob(c.DATA + "/ceb_trainxc_flac/*.flac")):
    key = os.path.basename(path).replace("__", "/", 1)
    ev = byf.get(key, [])
    sp = {r["scientific_name"] for r in ev}
    if len(sp) != 1: continue
    name = next(iter(sp))
    pidx, basis = mp.perch_idx(name)
    if pidx is None: continue
    if any(r["label_status"] != "valid" for r in ev): continue
    iv = [(float(r["start_time"]), float(r["end_time"])) for r in ev]
    x, sr, dur = c.load_audio_32k(path)
    cands = []
    for t0 in np.arange(0, len(x) / c.SR - 5.0 + 1e-9, 1.0):
        act = union_len(iv, t0, t0 + 5.0)
        if act >= 1.0: cands.append((hashlib.sha256(f"{SEED}|{key}|{t0}".encode()).hexdigest(), float(t0), act))
    cands.sort()
    kept = []
    for h, t0, act in cands:
        if all(abs(t0 - k[1]) >= 5.0 for k in kept): kept.append((h, t0, act))
        if len(kept) == 4: break
    for h, t0, act in kept:
        seg = x[int(t0 * c.SR): int(t0 * c.SR) + c.N_WIN]
        mask = np.zeros(c.N_WIN, bool)
        for a, b in iv:
            a0, b0 = int((max(a, t0) - t0) * c.SR), int((min(b, t0 + 5) - t0) * c.SR)
            if b0 > a0: mask[a0:b0] = True
        act_rms = float(np.sqrt((seg[mask].astype(np.float64) ** 2).mean()))
        oth = seg[~mask]
        oth_rms = float(np.sqrt((oth.astype(np.float64) ** 2).mean())) if len(oth) > c.SR else float("nan")
        segs.append(seg.astype(np.float32))
        cat.append(dict(seg_id=len(cat), species=name, perch_idx=pidx, bnp_idx=mp.bnp_idx(name) if mp.bnp_idx(name) is not None else "",
                        file=os.path.basename(path), t0=t0, active_s=round(act, 2), active_rms=act_rms,
                        active_rms_dbfs=round(20 * np.log10(max(act_rms, 1e-9)), 2),
                        event_vs_rest_db=round(20 * np.log10(max(act_rms, 1e-9) / max(oth_rms, 1e-9)), 2) if oth_rms == oth_rms else "",
                        license=ev[0]["license"], xc_id=ev[0]["xc_id"]))
np.save(c.WORK + "/syn/segments.npy", np.stack(segs))
with open(c.WORK + "/syn/catalogue.csv", "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=list(cat[0])); w.writeheader(); w.writerows(cat)
cnt = collections.Counter(r["species"] for r in cat)
print(len(cat), "segments;", len(cnt), "species")
print(sorted(cnt.items(), key=lambda kv: -kv[1]))
print(collections.Counter(r["license"] for r in cat))
