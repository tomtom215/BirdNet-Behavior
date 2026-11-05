"""Shared helpers: paths, label tables, Perch / BirdNET+ sessions, audio loading, windowing.

Run with the venv python:  SP/e2e/venv/bin/python -I script.py
"""
import os, re, csv, math, hashlib, json
import numpy as np

# Paths. PERCH_BAR_ROOT is a directory of your choice that will hold data/, work/, results/ and meta/;
# PERCH_BAR_MODELS holds perch-v2.onnx and the BirdNET+ V3.0 files named below (see README.md).
MS = os.environ["PERCH_BAR_ROOT"]
MODELS = os.environ["PERCH_BAR_MODELS"]
DATA = MS + "/data"
WORK = MS + "/work"
RES = MS + "/results"
META = MS + "/meta"
PERCH_ONNX = MODELS + "/perch-v2.onnx"
BNP_ONNX = MODELS + "/BirdNET+_V3.0-preview3_Global_11K_FP32.onnx"
BNP_LABELS = MODELS + "/BirdNET+_V3.0-preview3_Global_11K_Labels.csv"
PERCH_LABELS = os.path.join(os.path.dirname(os.path.abspath(__file__)), "../../../crates/birdnet-core/data/labels/perch-v2.csv")
SR = 32000
WIN = 5.0
N_WIN = int(WIN * SR)  # 160000


def load_perch_labels():
    rows = list(csv.DictReader(open(PERCH_LABELS, encoding="utf-8")))
    assert len(rows) == 14795
    sci = [r["sci_name"] for r in rows]
    rx = re.compile(r'[A-Z][a-z]+ [a-z\-]+')
    is_species = np.array([bool(rx.fullmatch(s)) for s in sci])
    assert is_species.sum() == 14595, is_species.sum()
    return sci, is_species


def load_bnp_labels():
    rows = list(csv.DictReader(open(BNP_LABELS, encoding="utf-8-sig"), delimiter=";"))
    assert len(rows) == 11560, len(rows)
    return [r["sci_name"] for r in rows], [r["class"] for r in rows]


def sessions(threads=2, bnp=True):
    import onnxruntime as ort
    so = ort.SessionOptions()
    so.intra_op_num_threads = threads
    so.inter_op_num_threads = 1
    so.log_severity_level = 3
    p = ort.InferenceSession(PERCH_ONNX, so, providers=["CPUExecutionProvider"])
    b = ort.InferenceSession(BNP_ONNX, so, providers=["CPUExecutionProvider"]) if bnp else None
    return p, b


def perch_logits(sess, batch):
    """batch: (n,160000) float32 -> (n,14795) float32 logits."""
    return sess.run(["label"], {"inputs": np.ascontiguousarray(batch, dtype=np.float32)})[0]


def bnp_probs(sess, batch):
    """batch: (n,samples) -> (n,11560) probabilities (model output is already a probability)."""
    return sess.run(["predictions"], {"input": np.ascontiguousarray(batch, dtype=np.float32)})[0]


def softmax(z):
    z = z.astype(np.float64)
    z = z - z.max(axis=-1, keepdims=True)
    e = np.exp(z)
    return (e / e.sum(axis=-1, keepdims=True)).astype(np.float32)


def _decode_ffmpeg_s16(path):
    """Fallback decoder (ffmpeg, mono, native rate, signed 16-bit -> float32 in [-1,1)) for files libsndfile cannot open
    (the Kenya FLAC files: libsndfile error 161 on the first read).  A truncated byte-prefix yields the decodable part."""
    import subprocess
    p = subprocess.run(["ffmpeg", "-v", "error", "-i", path, "-f", "s16le", "-ac", "1", "-"], capture_output=True)
    raw = p.stdout[: len(p.stdout) // 2 * 2]
    return np.frombuffer(raw, dtype="<i2").astype(np.float32) / 32768.0


def load_audio_32k(path, max_seconds=None):
    """Decode (FLAC/WAV), mono-mix, resample to 32 kHz with scipy resample_poly (as perch_probe.py does).
    A truncated FLAC prefix is decoded as far as it goes.  Returns (float32 array at 32 kHz, source_sr, decoded_seconds).
    A file that yields no audio at all raises (a silent empty result is how one dataset was lost once)."""
    import soundfile as sf
    from scipy.signal import resample_poly
    chunks = []
    with sf.SoundFile(path) as f:
        sr = f.samplerate
        while True:
            try:
                b = f.read(sr * 2, dtype="float32", always_2d=True)
            except Exception:
                break  # truncated final frame of a byte-prefix
            if len(b) == 0:
                break
            chunks.append(b.mean(axis=1))
    if chunks:
        x = np.concatenate(chunks)
    else:
        x = _decode_ffmpeg_s16(path)
        if len(x) == 0:
            raise RuntimeError(f"no audio could be decoded from {path}")
    dur = len(x) / sr
    if sr != SR:
        g = math.gcd(SR, sr)
        x = resample_poly(x, SR // g, sr // g).astype(np.float32)
    return x.astype(np.float32), sr, dur


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest()
