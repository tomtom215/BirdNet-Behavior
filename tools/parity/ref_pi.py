#!/usr/bin/env python3
"""Golden-output generator that runs BirdNET-Pi's OWN analysis code.

This is the *reference side* of a detection-parity check.  It does not
re-implement anything: it imports ``scripts.utils.{helpers,models,analysis,
classes}`` from a BirdNET-Pi checkout (passed with ``--pi-root``), points the
checkout's own settings loader at a generated ``birdnet.conf``, and calls
``utils.analysis.run_analysis`` unmodified.  Everything extra (per-chunk
top-10, raw logits, the MData species list) is captured by *observing* that
run through call-through wrappers installed from outside; the clone is never
edited.

Outside-in adjustments (all of them):

* ``tflite_runtime.interpreter`` is aliased to ``ai_edge_litert.interpreter``
  in ``sys.modules`` when the real ``tflite_runtime`` is not installed.
  (``models.py`` itself falls back to ``tensorflow.lite``, which needs no
  alias.)  The backend actually used is recorded in the output.
* ``helpers._load_settings`` is called with a generated conf file and
  ``force_reload=True``; that is the function's own public parameter.
* ``analysis.MODEL`` (the module-level cache) is reset to ``None`` when a
  setting that is baked in at model construction changes (what a service
  restart does).
* ``$HOME`` is pointed at a temp dir for the duration of a call so that
  ``os.path.expanduser("~/BirdNET-Pi/include_species_list.txt")`` etc. find
  the lists given on the command line.
* ``$TZ`` defaults to ``UTC`` (``Detection`` calls ``tzlocal`` for a field
  this tool does not emit).
* Call-through spies on ``librosa.load``, ``analysis.readAudioData``,
  ``analysis.analyzeAudioData``, ``analysis.filter_humans`` and the model's
  ``predict`` record inputs/outputs; each calls the original and returns its
  result untouched.

Sub-commands: ``analyze``, ``species-list``, ``selfcheck``.  Run with
``python -I`` so the interpreter does not import from the current directory.
"""

from __future__ import annotations

import argparse
import contextlib
import datetime as dt
import hashlib
import importlib
import json
import logging
import os
import sys
import tempfile
import types

import numpy as np

SCHEMA = "birdnet-parity/pi-ref/1"
MODEL_NAME = "BirdNET_GLOBAL_6K_V2.4_Model_FP16"

# scripts/install_config.sh @ 88985a3.  LATITUDE/LONGITUDE there are filled in
# from an ip-api.com lookup at install time; 0.0000 is only the fallback used
# when that lookup fails, and it is the only fixed value in the file.
SHIPPED_DEFAULTS = {
    "MODEL": MODEL_NAME,
    "SENSITIVITY": 1.25,
    "CONFIDENCE": 0.7,
    "OVERLAP": 0.0,
    "SF_THRESH": 0.03,
    "DATA_MODEL_VERSION": 1,
    "PRIVACY_THRESHOLD": 0,
    "LATITUDE": 0.0,
    "LONGITUDE": 0.0,
    "EXTRACTION_LENGTH": "",
    "DATABASE_LANG": "en",
}

# tests/helpers.py::Settings.with_defaults() in the same commit: what the
# project's own unit test (and, by its name, the expected values in
# analysis.py's __main__ block) runs under.  Note SF_THRESH is 0.003 there,
# not the 0.03 that install_config.sh ships.
PI_TEST_SETTINGS = dict(
    SHIPPED_DEFAULTS, LATITUDE=50.0, LONGITUDE=5.0, SF_THRESH=0.003, EXTRACTION_LENGTH=6
)

EXPECTED_PICA = [0.912, 0.9316, 0.8857]  # analysis.py __main__, FP16 model
# analysis.py __main__ names this file; tests/test_analysis.py creates it as a
# symlink to "Pica pica_30s.wav".  The date gives ISO week 8.
SELFCHECK_FILE_DATE = dt.datetime(2024, 2, 24, 16, 19, 37)


def sha256_file(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def sha256_names(names) -> str:
    """sha256 of the sorted names joined with LF, no trailing LF, UTF-8."""
    return hashlib.sha256("\n".join(sorted(names)).encode("utf-8")).hexdigest()


def install_tflite_shim() -> str:
    """Make BirdNET-Pi's ``import tflite_runtime.interpreter`` resolve.

    Returns a string naming the backend that will actually run the model.
    """
    from importlib import metadata

    try:
        import tflite_runtime.interpreter  # noqa: F401  (real package)

        return "tflite_runtime " + metadata.version("tflite-runtime")
    except ImportError:
        pass
    try:
        from ai_edge_litert import interpreter as litert

        pkg = types.ModuleType("tflite_runtime")
        pkg.__path__ = []  # mark as a package
        pkg.interpreter = litert
        sys.modules["tflite_runtime"] = pkg
        sys.modules["tflite_runtime.interpreter"] = litert
        return "ai-edge-litert " + metadata.version("ai-edge-litert") + " (aliased as tflite_runtime.interpreter)"
    except ImportError:
        pass
    # models.py falls back to `from tensorflow import lite` by itself.
    return "tensorflow.lite " + metadata.version(
        "tensorflow-cpu" if _has_dist("tensorflow-cpu") else "tensorflow"
    )


def _has_dist(name: str) -> bool:
    from importlib import metadata

    try:
        metadata.version(name)
        return True
    except metadata.PackageNotFoundError:
        return False


def render_conf(cfg: dict) -> str:
    """birdnet.conf text in the dialect helpers.PHPConfigParser reads."""
    lines = []
    for key, value in cfg.items():
        if isinstance(value, float):
            value = repr(value)
        lines.append(f"{key}={value}")
    return "\n".join(lines) + "\n"


def _jsonable(x):
    if isinstance(x, (np.floating,)):
        return float(x)
    if isinstance(x, (np.integer,)):
        return int(x)
    raise TypeError(f"not JSON serialisable: {type(x)}")


def dump_json(obj: dict, path: str) -> None:
    """Diff-friendly JSON: one key per line down to run level, one chunk/detection per line."""

    def comp(v):
        return json.dumps(v, separators=(",", ":"), default=_jsonable, ensure_ascii=False)

    line_lists = {"chunks", "detections", "entries"}

    def render(v, depth, key=None):
        pad = " " * depth
        if isinstance(v, dict) and depth < 2 and v:
            items = [f"{pad} {json.dumps(k)}: {render(x, depth + 1, k)}" for k, x in v.items()]
            return "{\n" + ",\n".join(items) + "\n" + pad + "}"
        if isinstance(v, list) and v and key in line_lists | {"runs"}:
            sub = [pad + " " + (render(x, depth + 1) if key == "runs" else comp(x)) for x in v]
            return "[\n" + ",\n".join(sub) + "\n" + pad + "]"
        return comp(v)

    with open(path, "w", encoding="utf-8") as f:
        f.write(render(obj, 0) + "\n")


class PiRef:
    """Thin driver around an unmodified BirdNET-Pi checkout."""

    def __init__(self, pi_root: str):
        self.pi_root = os.path.abspath(pi_root)
        if not os.path.isfile(os.path.join(self.pi_root, "scripts", "utils", "analysis.py")):
            raise SystemExit(f"--pi-root {pi_root!r} does not look like a BirdNET-Pi checkout")
        os.environ.setdefault("TZ", "UTC")
        self.backend = install_tflite_shim()
        if self.pi_root not in sys.path:
            sys.path.insert(0, self.pi_root)
        imp = importlib.import_module
        self.helpers = imp("scripts.utils.helpers")
        self.models = imp("scripts.utils.models")
        self.classes = imp("scripts.utils.classes")
        self.analysis = imp("scripts.utils.analysis")
        self._model_key = None
        self._tmp = tempfile.TemporaryDirectory(prefix="ref_pi_")
        self._conf_n = 0

    # -- settings ---------------------------------------------------------
    def configure(self, cfg: dict) -> dict:
        """Load ``cfg`` through BirdNET-Pi's own settings loader."""
        full = dict(SHIPPED_DEFAULTS)
        full.update(cfg)
        self._conf_n += 1
        path = os.path.join(self._tmp.name, f"birdnet-{self._conf_n}.conf")
        with open(path, "w") as f:
            f.write(render_conf(full))
        self.helpers._load_settings(path, force_reload=True)
        key = (full["MODEL"], float(full["SENSITIVITY"]), float(full["SF_THRESH"]), int(full["DATA_MODEL_VERSION"]))
        if key != self._model_key:
            self.analysis.MODEL = None  # what a service restart does
            self._model_key = key
        return full

    def labels(self):
        return self.helpers.get_model_labels(MODEL_NAME)

    # -- one clip ---------------------------------------------------------
    def analyze(
        self,
        clip: str,
        cfg: dict,
        *,
        week: int | None = None,
        year: int = 2020,
        file_date: dt.datetime | None = None,
        include=(),
        exclude=(),
        whitelist=(),
        keep_arrays: bool = False,
    ) -> dict:
        full = self.configure(cfg)
        if file_date is None:
            if week is not None:
                file_date = dt.datetime.combine(dt.date.fromisocalendar(year, week, 1), dt.time(12, 0, 0))
            else:
                file_date = SELFCHECK_FILE_DATE
        analysis = self.analysis
        log = logging.getLogger(analysis.__name__)
        warnings: list[str] = []

        class Cap(logging.Handler):
            def emit(self, record):
                if record.levelno >= logging.WARNING:
                    warnings.append(record.getMessage())

        cap = Cap()
        log.addHandler(cap)

        rec: dict = {"loaded": None, "chunks": None, "predict": [], "labeled": None, "species": None, "filtered": None}
        restore = []

        def patch(obj, name, new):
            old = getattr(obj, name)
            setattr(obj, name, new)
            restore.append((obj, name, old))

        import librosa

        orig_load = librosa.load

        def spy_load(*a, **k):
            sig, rate = orig_load(*a, **k)
            rec["loaded"] = {"n_samples": int(len(sig)), "rate": int(rate), "dtype": str(sig.dtype)}
            return sig, rate

        orig_read = analysis.readAudioData

        def spy_read(*a, **k):
            chunks = orig_read(*a, **k)
            rec["chunks"] = chunks
            return chunks

        orig_analyze = analysis.analyzeAudioData

        def spy_analyze(*a, **k):
            labeled, species = orig_analyze(*a, **k)
            rec["labeled"] = labeled
            rec["species"] = list(species)
            rec["model"] = analysis.MODEL
            return labeled, species

        orig_filter = analysis.filter_humans

        def spy_filter(preds):
            out = orig_filter(preds)
            rec["filtered"] = out
            return out

        patch(librosa, "load", spy_load)
        patch(analysis, "readAudioData", spy_read)
        patch(analysis, "analyzeAudioData", spy_analyze)
        patch(analysis, "filter_humans", spy_filter)

        model = None
        saved_home = os.environ.get("HOME")
        with tempfile.TemporaryDirectory(prefix="ref_pi_run_", dir=self._tmp.name) as run_dir:
            home = os.path.join(run_dir, "home")
            os.makedirs(os.path.join(home, "BirdNET-Pi"))
            for name, items in (("include", include), ("exclude", exclude), ("whitelist", whitelist)):
                if items:
                    with open(os.path.join(home, "BirdNET-Pi", f"{name}_species_list.txt"), "w") as f:
                        f.write("\n".join(items) + "\n")
            link = os.path.join(run_dir, f"{file_date:%Y-%m-%d}-birdnet-{file_date:%H:%M:%S}.wav")
            os.symlink(os.path.abspath(clip), link)
            os.environ["HOME"] = home
            try:
                # Wrap the model's predict() so every call's raw TFLite output is kept.
                model = analysis.load_global_model()
                orig_predict = model.predict

                def spy_predict(chunk):
                    out = orig_predict(chunk)
                    logits = model.interpreter.get_tensor(model._output_layer_idx)[0].copy()
                    rec["predict"].append((out, logits))
                    return out

                model.predict = spy_predict  # instance attribute shadows the method
                pi_file = self.classes.ParseFileName(link)
                detections = analysis.run_analysis(pi_file)
                error = None
            except Exception as e:  # BirdNET-Pi's process_file() also swallows errors per file
                detections, error = [], f"{type(e).__name__}: {e}"
            finally:
                if saved_home is None:
                    os.environ.pop("HOME", None)
                else:
                    os.environ["HOME"] = saved_home
                if model is not None and "predict" in vars(model):
                    del model.predict
                for obj, name, old in reversed(restore):
                    setattr(obj, name, old)
                log.removeHandler(cap)

        labels = self.labels()
        index = {l: i for i, l in enumerate(labels)}
        result = {
            "schema": SCHEMA,
            "clip": clip_info(clip),
            "config": {k: full[k] for k in sorted(full)},
            "run": {
                "file_date": file_date.isoformat(),
                "iso_week_passed_to_mdata": pi_file.week if error is None or rec["labeled"] is not None else None,
                "effective_sigmoid_slope": float(model._sensitivity) if model is not None else None,
                "n_labels": len(labels),
                "labels_sha256": hashlib.sha256("\n".join(labels).encode()).hexdigest(),
                "backend": self.backend,
            },
            "error": error,
        }
        if rec["loaded"]:
            result["audio_48k_mono"] = rec["loaded"]
        if rec["chunks"] is None:
            result.update(mdata=None, chunks=[], detections=[], excluded_log=warnings)
            return result

        chunks = rec["chunks"]
        result["audio_48k_mono"]["n_chunks"] = len(chunks)
        result["audio_48k_mono"]["chunk_dtypes"] = sorted({str(c.dtype) for c in chunks})
        out_chunks = []
        logit_rows = []
        keys = list((rec["labeled"] or {}).keys())
        filtered = rec["filtered"] or []
        for i, (sorted_scores, logits) in enumerate(rec["predict"]):
            top = sorted_scores[:10]
            idx = [index[l] for l, _ in top]
            slot = keys[i] if i < len(keys) else None
            s, e = (slot.split(";") if slot else (None, None))
            masked = bool(filtered and i < len(filtered) and filtered[i] == [("Human_Human", 0.0)])
            # 0-based rank of the best-ranked label whose name contains "Human"
            # (what filter_humans() tests within its first `human_cutoff` entries).
            hr = next((r for r, (l, _) in enumerate(sorted_scores) if "Human" in l), None)
            out_chunks.append(
                {
                    "i": i,
                    "slot": slot,
                    "start": float(s) if s is not None else None,
                    "end": float(e) if e is not None else None,
                    "top10": [[l, float(sc)] for l, sc in top],
                    "top10_logits": [float(logits[j]) for j in idx],
                    "first5_labels": labels[:5],
                    "first5_logits": [float(x) for x in logits[:5]],
                    "human_best_rank": hr,
                    "human_best": [sorted_scores[hr][0], float(sorted_scores[hr][1])] if hr is not None else None,
                    "privacy_masked": masked,
                }
            )
            logit_rows.append(logits)
        result["chunks"] = out_chunks

        sp = rec["species"]
        details = rec["model"]._mdata_model.get_species_list_details(labels) if sp else []
        result["mdata"] = {
            "version": int(full["DATA_MODEL_VERSION"]),
            "sf_thresh": float(full["SF_THRESH"]),
            "lat": float(full["LATITUDE"]),
            "lon": float(full["LONGITUDE"]),
            "week": pi_file.week,
            "count": len(sp),
            "empty_means_allow_all": len(sp) == 0,
            "first5": [[n, float(sc)] for sc, n in details[:5]],
            "last5": [[n, float(sc)] for sc, n in details[-5:]],
            "sorted_names_sha256": sha256_names(sp),
        }
        result["detections"] = [
            {
                "start": d.start,
                "stop": d.stop,
                "date": d.date,
                "time": d.time,
                "week": d.week,
                "scientific_name": d.scientific_name,
                "common_name": d.common_name,
                "confidence": d.confidence,
            }
            for d in detections
        ]
        result["excluded_log"] = warnings
        if keep_arrays:
            result["_arrays"] = {
                "chunks": np.stack([np.asarray(c, dtype=np.float32) for c in chunks]) if chunks else np.zeros((0, 144000), np.float32),
                "logits": np.stack(logit_rows).astype(np.float32) if logit_rows else np.zeros((0, len(labels)), np.float32),
            }
        return result

    # -- MData only -------------------------------------------------------
    def species_list(self, lat: float, lon: float, week: int, version: int, sf_thresh: float, want_raw: bool = False):
        full = self.configure({"DATA_MODEL_VERSION": version, "SF_THRESH": sf_thresh, "LATITUDE": lat, "LONGITUDE": lon})
        labels = self.labels()
        m = self.models.get_meta_model(MODEL_NAME, version)
        m.set_meta_data(lat, lon, week)
        details = m.get_species_list_details(labels)
        names = m.get_species_list(labels)
        out = {
            "version": version,
            "sf_thresh": sf_thresh,
            "lat": lat,
            "lon": lon,
            "week": week,
            "count": len(names),
            "first5": [[n, float(sc)] for sc, n in details[:5]],
            "last5": [[n, float(sc)] for sc, n in details[-5:]],
            "sorted_names_sha256": sha256_names(names),
        }
        raw = None
        if want_raw:
            sample = np.expand_dims(np.array([lat, lon, week], dtype="float32"), 0)
            m.interpreter.set_tensor(m._input_layer_idx, sample)
            m.interpreter.invoke()
            raw = m.interpreter.get_tensor(m._output_layer_idx)[0].copy()
        return out, names, raw


def clip_info(path: str) -> dict:
    info = {"file": os.path.basename(path), "sha256": sha256_file(path), "bytes": os.path.getsize(path)}
    try:
        import soundfile as sf

        i = sf.info(path)
        info.update(rate=i.samplerate, channels=i.channels, frames=i.frames, duration_s=i.frames / i.samplerate, subtype=i.subtype)
    except Exception as e:  # pragma: no cover - soundfile ships with librosa
        info["info_error"] = str(e)
    return info


# -- CLI ------------------------------------------------------------------
def _csv(s):
    return [x.strip() for x in s.split(",") if x.strip()] if s else []


def add_config_args(p):
    d = SHIPPED_DEFAULTS
    p.add_argument("--lat", type=float, default=d["LATITUDE"])
    p.add_argument("--lon", type=float, default=d["LONGITUDE"])
    p.add_argument("--sensitivity", type=float, default=d["SENSITIVITY"])
    p.add_argument("--confidence", type=float, default=d["CONFIDENCE"])
    p.add_argument("--overlap", type=float, default=d["OVERLAP"])
    p.add_argument("--sf-thresh", type=float, default=d["SF_THRESH"])
    p.add_argument("--data-model-version", type=int, choices=(1, 2), default=d["DATA_MODEL_VERSION"])
    p.add_argument("--privacy-threshold", type=float, default=d["PRIVACY_THRESHOLD"])


def cfg_from_args(a) -> dict:
    return {
        "LATITUDE": a.lat,
        "LONGITUDE": a.lon,
        "SENSITIVITY": a.sensitivity,
        "CONFIDENCE": a.confidence,
        "OVERLAP": a.overlap,
        "SF_THRESH": a.sf_thresh,
        "DATA_MODEL_VERSION": a.data_model_version,
        "PRIVACY_THRESHOLD": a.privacy_threshold,
    }


def cmd_analyze(a) -> int:
    ref = PiRef(a.pi_root)
    fd = dt.datetime.fromisoformat(a.file_date) if a.file_date else None
    runs = []
    for clip in a.clips:
        res = ref.analyze(
            clip,
            cfg_from_args(a),
            week=a.week,
            year=a.year,
            file_date=fd,
            include=_csv(a.include),
            exclude=_csv(a.exclude),
            whitelist=_csv(a.whitelist),
            keep_arrays=bool(a.arrays_dir),
        )
        arrs = res.pop("_arrays", None)
        if arrs is not None:
            os.makedirs(a.arrays_dir, exist_ok=True)
            stem = os.path.splitext(os.path.basename(clip))[0]
            np.save(os.path.join(a.arrays_dir, f"{stem}.chunks.npy"), arrs["chunks"])
            np.save(os.path.join(a.arrays_dir, f"{stem}.logits.npy"), arrs["logits"])
        runs.append(res)
    if len(runs) == 1:
        dump_json(runs[0], a.out)
    else:
        dump_json({"schema": SCHEMA, "runs": runs}, a.out)
    return 0


def cmd_species_list(a) -> int:
    ref = PiRef(a.pi_root)
    entries = []
    for v in _csv(a.versions) or ["1", "2"]:
        for lat, lon in a.locations:
            for wk in [int(w) for w in _csv(a.weeks)]:
                out, _, raw = ref.species_list(lat, lon, wk, int(v), a.sf_thresh, want_raw=bool(a.raw_dir))
                if a.compact:
                    out.pop("first5"), out.pop("last5")
                entries.append(out)
                if raw is not None:
                    os.makedirs(a.raw_dir, exist_ok=True)
                    np.save(os.path.join(a.raw_dir, f"mdata_v{v}_{lat}_{lon}_w{wk}.npy"), raw)
    dump_json({"schema": SCHEMA, "backend": ref.backend, "entries": entries}, a.out)
    return 0


def cmd_selfcheck(a) -> int:
    """Reproduce the expected confidences embedded in analysis.py's __main__."""
    ref = PiRef(a.pi_root)
    clip = os.path.join(ref.pi_root, "tests", "testdata", "Pica pica_30s.wav")
    report = {"schema": SCHEMA, "expected": EXPECTED_PICA, "backend": ref.backend, "variants": {}}
    for name, cfg in (("pi_unit_test_settings", PI_TEST_SETTINGS), ("install_config_defaults", SHIPPED_DEFAULTS)):
        res = ref.analyze(clip, cfg, file_date=SELFCHECK_FILE_DATE)
        got = [d["confidence"] for d in res["detections"]]
        species = [d["scientific_name"] for d in res["detections"]]
        exact = got == EXPECTED_PICA and all(s == "Pica pica" for s in species)
        delta = (
            max(abs(g - e) for g, e in zip(got, EXPECTED_PICA)) if len(got) == len(EXPECTED_PICA) else None
        )
        report["variants"][name] = {
            "config": res["config"],
            "got": got,
            "species": species,
            "starts": [d["start"] for d in res["detections"]],
            "exact_match_as_in___main__": exact,
            "max_abs_delta": delta,
            "within_1e-4_as_in_unit_test": delta is not None and delta <= 1e-4 and len(got) == 3,
            "mdata_count": res["mdata"]["count"] if res.get("mdata") else None,
            "error": res["error"],
        }
    dump_json(report, a.out)
    print(json.dumps({k: {kk: vv for kk, vv in v.items() if kk != "config"} for k, v in report["variants"].items()}, indent=1))
    return 0


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--pi-root", required=True, help="path to a BirdNET-Pi checkout (commit 88985a3 was used)")
    sub = ap.add_subparsers(dest="cmd", required=True)

    pa = sub.add_parser("analyze", help="run run_analysis() on clips and emit JSON")
    pa.add_argument("clips", nargs="+")
    pa.add_argument("--out", required=True)
    pa.add_argument("--week", type=int, help="ISO week 1-53 to put in the (synthetic) file name; default: self-check date, week 8")
    pa.add_argument("--year", type=int, default=2020, help="ISO year used with --week (2020 has 53 ISO weeks)")
    pa.add_argument("--file-date", help="YYYY-MM-DDTHH:MM:SS for the file name; overrides --week")
    pa.add_argument("--include", default="", help="comma-separated scientific names (include_species_list.txt)")
    pa.add_argument("--exclude", default="", help="comma-separated scientific names (exclude_species_list.txt)")
    pa.add_argument("--whitelist", default="", help="comma-separated scientific names (whitelist_species_list.txt)")
    pa.add_argument("--arrays-dir", help="also save <stem>.chunks.npy (float32 [n,144000]) and <stem>.logits.npy ([n,6522]) here")
    add_config_args(pa)
    pa.set_defaults(fn=cmd_analyze)

    ps = sub.add_parser("species-list", help="MData species list for (lat,lon,week) grids; no audio")
    ps.add_argument("--out", required=True)
    ps.add_argument("--versions", default="1,2")
    ps.add_argument("--weeks", default="1,26,48,49,52,53")
    ps.add_argument("--sf-thresh", type=float, default=SHIPPED_DEFAULTS["SF_THRESH"])
    ps.add_argument("--location", action="append", default=[], metavar="LAT,LON", help="repeatable")
    ps.add_argument("--raw-dir", help="save the raw MData output vector per entry as .npy")
    ps.add_argument("--compact", action="store_true", help="omit first5/last5 (keeps count + sorted-name sha256)")
    ps.set_defaults(fn=cmd_species_list)

    pc = sub.add_parser("selfcheck", help="reproduce analysis.py's embedded expected confidences")
    pc.add_argument("--out", required=True)
    pc.set_defaults(fn=cmd_selfcheck)

    a = ap.parse_args(argv)
    if a.cmd == "species-list":
        a.locations = [tuple(float(x) for x in s.split(",")) for s in a.location] or [(-1.0, -1.0), (51.48, -0.13)]
    return a.fn(a)


if __name__ == "__main__":
    sys.exit(main())
