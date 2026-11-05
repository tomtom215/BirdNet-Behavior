"""Non-bird / noise-only windows: (a) the ESC-50 clips fetched by fetch_esc50.py (5 s each, real recordings of rain, wind,
thunderstorm, sea waves, engine, airplane, helicopter, train), (b) three SYNTHETIC controls: digital silence, and
Gaussian white noise at -60 and -40 dBFS RMS (not real recordings).  Writes work/noise/{windows.csv,perch.npy,bnp.npy}."""
import sys, os, csv, glob
import numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common as c

meta = {r["filename"]: r for r in csv.DictReader(open(c.DATA + "/esc50_meta/esc50.csv", encoding="utf-8"))}
psess, bsess = c.sessions(2)
X, rows = [], []
for p in sorted(glob.glob(c.DATA + "/esc50_noise_clips/*.wav")):
    x, sr, dur = c.load_audio_32k(p)
    assert abs(dur - 5.0) < 1e-3, (p, dur)
    x = x[:c.N_WIN]
    if len(x) < c.N_WIN: x = np.pad(x, (0, c.N_WIN - len(x)))
    fn = os.path.basename(p); m = meta[fn]
    X.append(x); rows.append(dict(kind="esc50", category=m["category"], file=fn, src_file=m["src_file"],
                                  rms_dbfs=round(20 * np.log10(max(float(np.sqrt((x.astype(np.float64) ** 2).mean())), 1e-9)), 2), real=1))
rng = np.random.default_rng(20261105)
for name, lvl in [("silence", None), ("white_-60dBFS", -60.0), ("white_-40dBFS", -40.0)]:
    x = np.zeros(c.N_WIN, np.float32) if lvl is None else (rng.standard_normal(c.N_WIN) * 10 ** (lvl / 20)).astype(np.float32)
    X.append(x); rows.append(dict(kind="synthetic", category=name, file="", src_file="", rms_dbfs=lvl if lvl is not None else -180.0, real=0))
X = np.stack(X)
P = np.concatenate([c.perch_logits(psess, X[i:i + 8]) for i in range(0, len(X), 8)])
B = np.concatenate([c.bnp_probs(bsess, X[i:i + 8, 16000:16000 + 144000]) for i in range(0, len(X), 8)]).astype(np.float16)
os.makedirs(c.WORK + "/noise", exist_ok=True)
np.save(c.WORK + "/noise/perch.npy", P); np.save(c.WORK + "/noise/bnp.npy", B)
with open(c.WORK + "/noise/windows.csv", "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=list(rows[0])); w.writeheader(); w.writerows(rows)
print(len(rows), "noise windows")
