#!/usr/bin/env python3
"""Golden for `tests/resampler_matches_resampy_parity.rs`: what BirdNET-Pi's resampler does to a known signal.

    python -I resampler_golden.py OUT.json

BirdNET-Pi reads audio with ``librosa.load(sr=48000, res_type='kaiser_fast')``, which for any
rate other than 48 kHz calls ``resampy.resample(y, sr, 48000, filter='kaiser_fast')``.  This
runs exactly that on a deterministic sum of sines (frequencies below the lowest source
Nyquist used, so there is nothing for a resampler to remove) and records 256 consecutive
output samples from the middle of the clip, together with the analytic value the ideal
band-limited resampler would give (the same sines evaluated at the output times).  The Rust
test regenerates the input from the recorded recipe, resamples it with the shipping
`birdnet_core::audio::resample::resample` and compares the two, so it needs neither Python
nor resampy.

Pinned: resampy 0.4.3, numpy 2.5.4 (tools/parity/requirements-primary.txt).
"""
import json
import sys

import numpy as np
import resampy

TONES = [(440.0, 0.30), (1800.0, 0.20), (3500.0, 0.15), (6100.0, 0.10), (9000.0, 0.05)]
SECONDS = 2.0
WINDOW_START = 40000   # output sample index, well away from both edges
WINDOW_LEN = 256
RATES = [44100, 32000, 22050]   # all have a source Nyquist (>= 11025 Hz) above the highest tone
TARGET = 48000


def signal(rate, n):
    t = np.arange(n, dtype=np.float64) / rate
    return sum(a * np.sin(2 * np.pi * f * t) for f, a in TONES)


def main(out):
    runs = []
    for rate in RATES:
        n = int(rate * SECONDS)
        x = signal(rate, n).astype(np.float32)
        y = resampy.resample(x, rate, TARGET, filter="kaiser_fast")
        k = np.arange(WINDOW_START, WINDOW_START + WINDOW_LEN)
        ideal = signal(TARGET, int(TARGET * SECONDS))[k]
        w = y[WINDOW_START:WINDOW_START + WINDOW_LEN].astype(np.float64)
        runs.append({
            "from_rate": rate,
            "input_len": n,
            "output_len": int(len(y)),
            "resampy_window": [float(v) for v in w],
            "resampy_max_abs_error_vs_ideal": float(np.abs(w - ideal).max()),
        })
    doc = {
        "schema": "birdnet-parity/resampler/1",
        "reference": "resampy %s kaiser_fast (librosa.load res_type, BirdNET-Pi analysis.py)" % resampy.__version__,
        "tones_hz_amplitude": TONES,
        "seconds": SECONDS,
        "to_rate": TARGET,
        "window_start": WINDOW_START,
        "window_len": WINDOW_LEN,
        "runs": runs,
    }
    with open(out, "w") as f:
        json.dump(doc, f, indent=1)
        f.write("\n")
    for r in runs:
        print(r["from_rate"], "resampy vs ideal max abs error %.2e" % r["resampy_max_abs_error_vs_ideal"])


if __name__ == "__main__":
    main(sys.argv[1])
