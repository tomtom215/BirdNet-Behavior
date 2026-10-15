#!/usr/bin/env python3
"""An independent reference for the BirdNET+ V3.0 path this project ships by default.

Nothing here shares code with the Rust pipeline: the audio is read with
soundfile, mixed to mono by a plain mean, resampled to 32 kHz with soxr (its
VHQ setting, a different resampler from rubato), cut into 4.5 s windows by
slicing, and run through onnxruntime. A trailing remainder under 1.5 s is dropped
and a longer one zero-padded, as the Rust chunker does. For each window it
prints the start, the top species and its probability.

    v3_reference.py MODEL.onnx LABELS.csv a.wav b.wav ...

Compare with the Rust side via tests/parity_dump.rs (its third field per species
is the probability). Needs numpy, soundfile, soxr, onnxruntime
(tools/parity/requirements-primary.txt).
"""
import sys

import numpy as np
import onnxruntime as ort
import soundfile as sf
import soxr

WINDOW = 144_000  # 4.5 s at 32 kHz
MIN_TAIL = 48_000  # 1.5 s at 32 kHz


def main() -> int:
    model, labels, *wavs = sys.argv[1:]
    rows = [line.rstrip("\n").split(";") for line in open(labels, encoding="utf-8-sig")][1:]
    sci = [r[2] for r in rows]
    sess = ort.InferenceSession(model, providers=["CPUExecutionProvider"])
    inp = sess.get_inputs()[0].name
    for wav in wavs:
        y, rate = sf.read(wav, dtype="float32", always_2d=True)
        z = soxr.resample(y.mean(axis=1), rate, 32_000, quality="VHQ").astype(np.float32)
        out, pos = [], 0
        while pos < len(z):
            chunk = z[pos : pos + WINDOW]
            if len(chunk) < WINDOW:
                chunk = np.pad(chunk, (0, WINDOW - len(chunk)))
            p = sess.run(["predictions"], {inp: chunk[None, :]})[0][0]
            k = int(np.argmax(p))
            out.append((round(pos / 32_000, 2), sci[k], round(float(p[k]), 4)))
            pos += WINDOW
            if pos >= len(z) or len(z) - pos < MIN_TAIL:
                break
        print(wav.rsplit("/", 1)[-1], out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
