#!/usr/bin/env python3
"""Why do the ONNX builds and the TFLite builds disagree on digital silence and zero-padded tails?

    python -I silence_probe.py --arrays OUT/matrix/arrays --pi-labels LABELS.txt \
        --tflite-fp16 FP16.tflite --tflite-fp32 FP32.tflite \
        --onnx-dfttrunc A.onnx --onnx-full B.onnx --work DIR --out silence_probe.json

Four parts, all measured, none assumed:
  graph    structural facts: op counts, DFT-matrix shapes, front-end constants in each ONNX and the
           values the FP16 TFLite stores for them (and what float16 rounding does to them).
  rounded  ONNX copies of BirdNET_v2.4_fp32.onnx whose float initialisers were passed through float16
           (all of them except the DFT matrices, which have no TFLite counterpart; or only the Hann
           windows / mel matrices / scalar constants / the CNN weights).  Compared with the FP16
           TFLite on the 54 BirdNET-Pi chunks: if the FP16 TFLite is "the FP32 graph with FP16
           constants", the rounded copy must close the gap.
  inputs   a digital-silence chunk and a zero-padded tail chunk fed as is, with 1 LSB of dither, with
           a DC offset, and a constant, to every build.
  leakage  the spectrum of a windowed constant frame read from inside each model (FP16 TFLite, FP32
           TFLite, ONNX) against a float64 FFT, plus a check that the TFLite CAST is the FFT's real part.
"""

from __future__ import annotations

import argparse
import collections
import glob
import hashlib
import json
import os
import sys

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
SLOPE = 0.75


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--arrays", required=True)
    ap.add_argument("--pi-labels", required=True)
    ap.add_argument("--tflite-fp16", required=True)
    ap.add_argument("--tflite-fp32", required=True)
    ap.add_argument("--onnx-dfttrunc", required=True)
    ap.add_argument("--onnx-full", required=True)
    ap.add_argument("--work", required=True, help="where the float16-rounded ONNX copies are written")
    ap.add_argument("--out", required=True)
    a = ap.parse_args(argv)

    import onnx
    import onnxruntime as ort
    from onnx import numpy_helper

    try:
        from ai_edge_litert.interpreter import Interpreter
    except ImportError:
        from tensorflow.lite import Interpreter  # type: ignore

    os.makedirs(a.work, exist_ok=True)
    labels = [l.strip().split("_")[0] for l in open(a.pi_labels, encoding="utf-8")]
    so = ort.SessionOptions()
    so.log_severity_level = 3

    # ---- graph facts -------------------------------------------------------------------------
    graph = {}
    models = {}
    for key, path in (("dfttrunc", a.onnx_dfttrunc), ("full", a.onnx_full)):
        m = onnx.load(path)
        models[key] = m
        init = {i.name: i for i in m.graph.initializer}
        mats = {n: list(i.dims) for n, i in init.items() if "dft_matrix" in n}
        graph[key] = {
            "nodes": len(m.graph.node),
            "initializers": len(init),
            "op_counts_top": collections.Counter(n.op_type for n in m.graph.node).most_common(8),
            "dft_matrices": mats,
            "front_end_scalars": {
                n.split("MEL_SPEC")[-1]: float(numpy_helper.to_array(i))
                for n, i in init.items()
                if "MEL_SPEC" in n and not i.dims
            },
            "first_nodes": [f"{n.op_type}:{n.name.split('/')[-1]}" for n in list(m.graph.node)[:7]],
        }
    names_t = {i.name for i in models["dfttrunc"].graph.initializer}
    names_f = {i.name for i in models["full"].graph.initializer}
    graph["initializer_names_only_in_dfttrunc"] = sorted(names_t - names_f)[:20]
    graph["initializer_names_only_in_full"] = sorted(names_f - names_t)[:20]

    tf16 = Interpreter(a.tflite_fp16)
    tf16.allocate_tensors()
    det = tf16.get_tensor_details()
    f16 = [d for d in det if d["dtype"] == np.float16]
    graph["tflite_fp16"] = {
        "float16_constant_tensors": len(f16),
        "complex64_tensors": sum(1 for d in det if d["dtype"] == np.complex64),
        "scalar_constants_stored_as_float16": {
            d["name"]: {
                "stored": float(tf16.get_tensor(d["index"])),
                "float32_of_the_same_decimal_literal_in_onnx": None,
            }
            for d in f16
            if "MEL_SPEC" in d["name"] and len(d["shape"]) == 0
        },
        "mel_and_window_tensors_float16": [d["name"] for d in f16 if "MEL_SPEC" in d["name"] and len(d["shape"]) > 0],
        "op_counts_top": collections.Counter(o["op_name"] for o in tf16._get_ops_details()).most_common(8),
        "rfft_ops": sorted({o["op_name"] for o in tf16._get_ops_details() if "FFT" in o["op_name"].upper()}),
    }
    # the ONNX literal for each scalar, to show what float16 does to it
    full_init = {i.name: i for i in models["full"].graph.initializer}
    for name, rec in graph["tflite_fp16"]["scalar_constants_stored_as_float16"].items():
        base = name  # same TF names in both
        if base in full_init:
            v = float(numpy_helper.to_array(full_init[base]))
            rec["float32_of_the_same_decimal_literal_in_onnx"] = v
            rec["relative_change"] = (rec["stored"] - v) / v if v else 0.0

    # ---- float16-rounded ONNX copies ---------------------------------------------------------
    def is_dft(n):
        return "dft_matrix" in n

    def is_front(n, dims):
        return "MEL_SPEC" in n or tuple(dims) in {(513, 96), (1025, 96)}

    preds = {
        "r_all": lambda n, d: not is_dft(n),
        "r_hann": lambda n, d: "hann_window" in n,
        "r_mel_matrices": lambda n, d: tuple(d) in {(513, 96), (1025, 96)},
        "r_scalars": lambda n, d: len(d) == 0 and "MEL_SPEC" in n,
        "r_front": lambda n, d: not is_dft(n) and is_front(n, d),
        "r_cnn": lambda n, d: not is_dft(n) and not is_front(n, d),
    }
    rounded_paths = {}
    counts = {}
    for key, pred in preds.items():
        m = onnx.load(a.onnx_full)
        n_changed = 0
        for init in m.graph.initializer:
            if init.data_type == onnx.TensorProto.FLOAT and pred(init.name, tuple(init.dims)):
                arr = numpy_helper.to_array(init)
                init.CopyFrom(numpy_helper.from_array(arr.astype(np.float16).astype(np.float32), init.name))
                n_changed += 1
        p = os.path.join(a.work, f"fp32_{key}.onnx")
        onnx.save(m, p)
        rounded_paths[key] = p
        counts[key] = n_changed
    sessions = {"c": a.onnx_dfttrunc, "d": a.onnx_full, **{k: p for k, p in rounded_paths.items()}}
    sess = {k: ort.InferenceSession(p, so, providers=["CPUExecutionProvider"]) for k, p in sessions.items()}

    def onnx_run(k, x):
        return sess[k].run(["output"], {"input": x[None, :].astype(np.float32)})[0][0]

    t32 = Interpreter(a.tflite_fp32)
    t32.allocate_tensors()

    def tf_run(it, x):
        it.set_tensor(it.get_input_details()[0]["index"], x[None, :].astype(np.float32))
        it.invoke()
        return it.get_tensor(it.get_output_details()[0]["index"])[0].copy()

    def run(k, x):
        if k == "a":
            return tf_run(tf16, x)
        if k == "b":
            return tf_run(t32, x)
        return onnx_run(k, x)

    # chunk groups (same rule as variants_compare.py)
    with open(os.path.join(HERE, "fixtures.json"), encoding="utf-8") as f:
        rate_of = {c["id"]: c["rate"] for c in json.load(f)["clips"]}

    def group(chunk, rate):
        if not chunk.any():
            return "digital_silence"
        nz = np.nonzero(chunk)[0]
        if len(chunk) - 1 - nz[-1] >= 24000:
            return "zero_tail"
        if rate <= 22050:
            return "band_limited"
        if float(np.sqrt(np.mean(chunk.astype(np.float64) ** 2))) < 1e-4:
            return "near_silence"
        return "ordinary"

    rows = []
    X = []
    for cp in sorted(glob.glob(os.path.join(a.arrays, "*.chunks.npy"))):
        cid = os.path.basename(cp)[: -len(".chunks.npy")]
        ch = np.load(cp)
        for i, c in enumerate(ch):
            rows.append((cid, i, group(c, rate_of[cid])))
            X.append(c)
    ref_a = {cid: np.load(os.path.join(a.arrays, cid + ".logits.npy")) for cid in {r[0] for r in rows}}
    A = np.stack([ref_a[cid][i] for cid, i, _ in rows])
    L = {"a": A}
    L["b"] = np.stack([run("b", c) for c in X])
    for k in sessions:
        L[k] = np.stack([run(k, c) for c in X])

    def st(x, y, sel):
        d = np.abs(x[sel].astype(np.float64) - y[sel].astype(np.float64))
        return {"n": int(len(d)), "max_abs": float(d.max()), "mean_abs": float(d.mean()), "top1": int((x[sel].argmax(1) == y[sel].argmax(1)).sum())}

    groups = {
        "all": np.ones(len(rows), bool),
        "ordinary": np.array([r[2] == "ordinary" for r in rows]),
        "zero_tail+digital_silence": np.array([r[2] in ("zero_tail", "digital_silence") for r in rows]),
        "band_limited": np.array([r[2] == "band_limited" for r in rows]),
    }
    rounded = {"initializers_rounded": counts, "vs": {}}
    for ref in ("a", "b", "d"):
        for k in ["c", "d", *preds]:
            if k == ref:
                continue
            rounded["vs"][f"{ref}-{k}"] = {g: st(L[ref], L[k], sel) for g, sel in groups.items()}

    # ---- input experiments -------------------------------------------------------------------
    idx = {(cid, i): n for n, (cid, i, _) in enumerate(rows)}
    rs = np.random.RandomState(7)
    lsb = 1.0 / 32768.0
    dither = (rs.randint(-1, 2, 144000) * lsb).astype(np.float32)
    inputs = {"digital_silence: zeros": np.zeros(144000, np.float32)}
    inputs["digital_silence + constant 0.1"] = np.full(144000, 0.1, np.float32)
    inputs["digital_silence + dither (+-1 LSB)"] = dither
    for clip in ("ij_mono_48k_7s5", "ij_mono_48k_7s6"):
        base = X[idx[(clip, 2)]].astype(np.float32)
        z0 = int(np.nonzero(base)[0][-1]) + 1
        v = base.copy()
        inputs[f"{clip}[2]: zero tail as is ({(144000 - z0) / 48000:.2f} s)"] = v
        d = base.copy()
        d[z0:] = dither[z0:]
        inputs[f"{clip}[2]: tail = dither (+-1 LSB)"] = d
        c = base.copy()
        c[z0:] = lsb * 3
        inputs[f"{clip}[2]: tail = constant 3 LSB"] = c
        n = base.copy()
        n[z0:] = (rs.standard_normal(144000 - z0) * 1e-3).astype(np.float32)
        inputs[f"{clip}[2]: tail = noise sigma 1e-3"] = n
    keys = ["a", "b", "c", "d", "r_all", "r_hann", "r_scalars", "r_mel_matrices"]
    pair_list = [("a", "b"), ("a", "c"), ("b", "c"), ("a", "r_all"), ("c", "r_all"), ("b", "r_all"), ("a", "r_hann"), ("a", "r_scalars"), ("a", "r_mel_matrices")]
    exp = {}
    for name, x in inputs.items():
        out = {k: run(k, x) for k in keys}
        exp[name] = {
            "top1": {k: labels[int(out[k].argmax())] for k in keys},
            "max_abs": {f"{p}-{q}": float(np.abs(out[p].astype(np.float64) - out[q]).max()) for p, q in pair_list},
            "mean_abs": {f"{p}-{q}": float(np.abs(out[p].astype(np.float64) - out[q]).mean()) for p, q in pair_list},
        }

    # ---- leakage of a windowed constant frame, measured inside the models -----------------------
    # Digital silence is normalised to a constant (-1.0); each 2048-sample frame is that constant times
    # the Hann window, so every non-DC bin of the spectrum is pure rounding noise.  Read the tensor
    # that feeds the mel matrix in each model (TFLite: output of CAST after RFFT2D; ONNX: output of the
    # rfft MatMul) for the all-zero chunk and compare with a float64 FFT of the same frames.
    from onnx import TensorProto, helper

    zeros = np.zeros(144000, np.float32)

    def tfl_real_part(path):
        it = Interpreter(path, experimental_preserve_all_tensors=True)
        it.allocate_tensors()
        idx = {d["name"]: d["index"] for d in it.get_tensor_details()}
        it.set_tensor(it.get_input_details()[0]["index"], zeros[None, :])
        it.invoke()
        return it.get_tensor(idx["model/MEL_SPEC1/Cast"]).reshape(-1, 1025).astype(np.float64)

    mm = onnx.load(a.onnx_full)
    for n in ("model/MEL_SPEC1/stft/rfft4", "model/MEL_SPEC1/stft/mul"):
        mm.graph.output.append(helper.make_tensor_value_info(n, TensorProto.FLOAT, None))
    s_dbg = ort.InferenceSession(mm.SerializeToString(), so, providers=["CPUExecutionProvider"])
    _, r_onnx, frames = s_dbg.run(["output", "model/MEL_SPEC1/stft/rfft4", "model/MEL_SPEC1/stft/mul"], {"input": zeros[None, :]})
    exact = np.fft.rfft(frames[0].astype(np.float64), axis=-1).real

    def rel(r):
        r = r.reshape(-1, r.shape[-1]).astype(np.float64)
        return float((np.abs(r[:, 2:]).max(axis=1) / np.abs(r[:, 0])).max())

    # sanity: the TFLite CAST really is the real part of the FFT (not |FFT|, not the imaginary part)
    ch = np.load(os.path.join(a.arrays, "pica_pica_30s.chunks.npy"))[1]
    it32 = Interpreter(a.tflite_fp32, experimental_preserve_all_tensors=True)
    it32.allocate_tensors()
    idx32 = {d["name"]: d["index"] for d in it32.get_tensor_details()}
    it32.set_tensor(it32.get_input_details()[0]["index"], ch[None, :])
    it32.invoke()
    cast = it32.get_tensor(idx32["model/MEL_SPEC1/Cast"]).reshape(-1, 1025).astype(np.float64)
    fr2 = s_dbg.run(["model/MEL_SPEC1/stft/mul"], {"input": ch[None, :]})[0][0].astype(np.float64)
    ref_c = np.fft.rfft(fr2, axis=-1)
    def dft_matrix_check():
        out = {}
        w = numpy_helper.to_array(full_init["model/MEL_SPEC1/stft/hann_window/sub_2"]).astype(np.float32)
        frame = (w * np.float32(-1.0)).astype(np.float64)
        for name, n_fft in (("model/MEL_SPEC1/stft/rfft3_dft_matrix", 2048), ("model/MEL_SPEC2/stft/rfft2_dft_matrix", 1024)):
            D = numpy_helper.to_array(full_init[name]).astype(np.float64)
            exact = np.cos(2 * np.pi * np.outer(np.arange(n_fft), np.arange(D.shape[1])) / n_fft)
            e = np.abs(D - exact)
            rec = {"shape": list(D.shape), "max_abs_error": float(e.max()), "mean_abs_error": float(e.mean()),
                   "max_abs_error_of_float32_rounding_alone": float(np.abs(exact.astype(np.float32) - exact).max())}
            if n_fft == 2048:
                f = frame
                r = lambda R: float(np.abs(R[2:]).max() / abs(R[0]))
                rec["leak_of_constant_frame_float64_matmul_with_stored_matrix"] = r(f @ D)
                rec["leak_of_constant_frame_float64_matmul_with_exact_matrix"] = r(f @ exact)
            out[name] = rec
        return out

    leak = {
        "what": "max over the 511 MEL_SPEC1 frames of max_{k>=2}|Re X[k]| / |Re X[0]| for the all-zero chunk (input normalised to constant -1.0)",
        "float64_FFT_of_the_ONNX_frames": rel(exact),
        "tflite_fp32_RFFT2D": rel(tfl_real_part(a.tflite_fp32)),
        "tflite_fp16_RFFT2D_with_fp16_hann": rel(tfl_real_part(a.tflite_fp16)),
        "onnx_fp32_dense_dft_matmul": rel(r_onnx),
        "hann_window_max_abs_change_when_rounded_to_fp16": float(
            np.abs(
                numpy_helper.to_array(full_init["model/MEL_SPEC1/stft/hann_window/sub_2"]).astype(np.float16).astype(np.float32)
                - numpy_helper.to_array(full_init["model/MEL_SPEC1/stft/hann_window/sub_2"])
            ).max()
        ),
        "onnx_dft_matrix_vs_exact_cosine": dft_matrix_check(),
        "tflite_cast_check_pica_chunk1": {
            "max_abs_vs_Re_FFT": float(np.abs(cast - ref_c.real).max()),
            "max_abs_vs_Im_FFT": float(np.abs(cast - ref_c.imag).max()),
            "max_abs_value": float(np.abs(ref_c.real).max()),
        },
    }

    report = {
        "files": {
            os.path.basename(p): hashlib.sha256(open(p, "rb").read()).hexdigest()
            for p in (a.tflite_fp16, a.tflite_fp32, a.onnx_dfttrunc, a.onnx_full)
        },
        "graph": graph,
        "float16_rounded_onnx": rounded,
        "input_experiments": exp,
        "leakage": leak,
        "chunk_groups": {g: int(s.sum()) for g, s in groups.items()},
    }
    with open(a.out, "w", encoding="utf-8") as f:
        json.dump(report, f, indent=1)
        f.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
