#!/usr/bin/env python3
"""BirdNET-Pi's own `splitSignal`, over every overlap the settings page can hold and every recording length.

    python -I chunking_ref.py --pi-root PI --out golden/chunking.json

`analysis.splitSignal(sig, rate, overlap, seconds=3.0, minlen=1.5)` is called as BirdNET-Pi calls it
(`analysis.readAudioData`), on a ramp so that the first sample of each returned window names where it started.
Grid: overlap 0.00 to 2.90 in steps of 0.05 (59 values; `OVERLAP` is a free decimal, 0 to 2.9), recording length 1.5 s to
40 s in quarter seconds (155 values), 48 kHz.  Recorded per overlap: the number of windows for every length, and for the
lengths where `tests/chunking_matches_birdnet_pi.rs` needs more, the start offsets.
"""
import argparse
import importlib.util
import json
import os
import sys

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
sys.dont_write_bytecode = True
RATE = 48000


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--pi-root", required=True)
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    spec = importlib.util.spec_from_file_location("ref_pi", os.path.join(HERE, "ref_pi.py"))
    rp = importlib.util.module_from_spec(spec)
    sys.modules["ref_pi"] = rp
    spec.loader.exec_module(rp)
    analysis = rp.PiRef(a.pi_root).analysis
    lengths_q = list(range(6, 161))          # quarter seconds: 1.5 s ... 40 s
    out = {"schema": "birdnet-parity/chunking/1", "rate": RATE, "seconds": 3.0, "minlen": 1.5, "lengths_quarter_seconds": [6, 160],
           "overlaps": {}}
    ramp = np.arange(1, 40 * RATE + 1, dtype="float64")
    for k in range(0, 59):
        overlap = round(k * 0.05, 2)
        counts, starts = [], {}
        for q in lengths_q:
            n = q * RATE // 4
            chunks = analysis.splitSignal(ramp[:n], RATE, overlap, seconds=3.0)
            st = [int(c[0]) - 1 for c in chunks]
            counts.append(len(st))
            starts[q] = st
        out["overlaps"][f"{overlap:.2f}"] = {"counts": counts, "step": int((3.0 - overlap) * RATE), "last_starts": {str(q): s[-1] for q, s in starts.items()}}
    with open(a.out, "w") as f:
        json.dump(out, f, indent=0)
        f.write("\n")
    print("overlaps", len(out["overlaps"]), "lengths", len(lengths_q))
    return 0


if __name__ == "__main__":
    sys.exit(main())
