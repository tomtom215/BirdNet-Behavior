#!/usr/bin/env python3
"""What BirdNET-Pi's include / exclude / whitelist lists and occurrence filter keep, scenario by scenario.

    python -I species_lists_ref.py --pi-root PI --out golden/species_lists.json

Drives BirdNET-Pi's own `analysis.run_analysis()` (88985a3) with the audio read and the model's scores replaced
by fixed ones, so that the only code between the scores and the verdict is BirdNET-Pi's:
`loadCustomSpeciesList` reading real files under a throw-away `$HOME/BirdNET-Pi/`, the three conditions on the
include list, the exclude list and the occurrence list, and the occurrence list itself, which is asked of BirdNET-Pi's
own `MDataModel2` (v2 TFLite, SF_THRESH as given) for the scenario's place and ISO week.

Every scenario hands the same window to `run_analysis`: each species of UNIVERSE at confidence 0.99, in that order.
The record says which survived.  `tests/species_lists_match_birdnet_pi.rs` asks this project's `SpeciesFilter` the same
questions (through the importer's own reading of the same file text) and compares.
"""
import argparse
import datetime as dt
import importlib.util
import json
import os
import sys
import tempfile

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
sys.dont_write_bytecode = True

PLACE = (50.0, 5.0)
WEEK = 8
# Four species the v2 range model admits at PLACE in week 8, four it does not (checked below), and one with a
# subspecies-looking two-word name that is also admitted.
UNIVERSE = [
    "Pica pica", "Parus major", "Turdus merula", "Erithacus rubecula",
    "Bias musicus", "Casuarius casuarius", "Pitta sordida", "Menura novaehollandiae",
]

# name -> (include text, exclude text, whitelist text, sf_thresh); text None = no file
SCENARIOS = {
    "no_lists": (None, None, None, 0.03),
    "include_one_sci": ("Pica pica\n", None, None, 0.03),
    "include_picker_lines": ("Pica pica_Eurasian Magpie\nParus major_Great Tit\n", None, None, 0.03),
    "include_common_name_only": ("Eurasian Magpie\n", None, None, 0.03),
    "include_typo": ("Pica pika\n", None, None, 0.03),
    "include_blank_line_only": ("\n", None, None, 0.03),
    "include_empty_file": ("", None, None, 0.03),
    "include_out_of_range_species": ("Bias musicus\nPica pica\n", None, None, 0.03),
    "include_lowercase": ("pica pica\n", None, None, 0.03),
    "include_crlf": ("Pica pica_Eurasian Magpie\r\nParus major_Great Tit\r\n", None, None, 0.03),
    "include_trailing_spaces": ("  Pica pica  \n", None, None, 0.03),
    "exclude_one_sci": (None, "Pica pica\n", None, 0.03),
    "exclude_picker_line": (None, "Pica pica_Eurasian Magpie\n", None, 0.03),
    "exclude_common_name_only": (None, "Eurasian Magpie\n", None, 0.03),
    "exclude_lowercase": (None, "pica pica\n", None, 0.03),
    "exclude_blank_line_only": (None, "\n", None, 0.03),
    "whitelist_rescues_out_of_range": (None, None, "Bias musicus\n", 0.03),
    "whitelist_in_range_species": (None, None, "Pica pica\n", 0.03),
    "whitelist_and_exclude": (None, "Bias musicus\n", "Bias musicus\n", 0.03),
    "whitelist_outside_include": ("Pica pica\n", None, "Bias musicus\n", 0.03),
    "whitelist_inside_include": ("Pica pica\nBias musicus\n", None, "Bias musicus\n", 0.03),
    "include_and_exclude_same": ("Pica pica\n", "Pica pica\n", None, 0.03),
    "include_two_exclude_one": ("Pica pica\nParus major\n", "Parus major\n", None, 0.03),
    "sf_thresh_0_all_species": (None, None, None, 0.0),
    "sf_thresh_high_few_species": (None, None, None, 0.5),
    "sf_thresh_1_empty_list_allows_all": (None, None, None, 1.0),
    "sf_thresh_1_exclude_one": (None, "Pica pica\n", None, 1.0),
}


def load_ref_pi():
    spec = importlib.util.spec_from_file_location("ref_pi", os.path.join(HERE, "ref_pi.py"))
    mod = importlib.util.module_from_spec(spec)
    sys.modules["ref_pi"] = mod
    spec.loader.exec_module(mod)
    return mod


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--pi-root", required=True)
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    rp = load_ref_pi()
    ref = rp.PiRef(a.pi_root)
    analysis = ref.analysis
    import logging

    out = {"schema": "birdnet-parity/species-lists/1", "place": PLACE, "iso_week": WEEK, "universe": UNIVERSE, "scenarios": {}}
    log = logging.getLogger(analysis.__name__)
    for name, (inc, exc, wl, thr) in SCENARIOS.items():
        ref.configure({"LATITUDE": PLACE[0], "LONGITUDE": PLACE[1], "DATA_MODEL_VERSION": 2, "SF_THRESH": thr})
        warnings = []

        class Cap(logging.Handler):
            def emit(self, record):
                if record.levelno >= logging.WARNING:
                    warnings.append(record.getMessage())

        cap = Cap()
        log.addHandler(cap)
        orig_read, orig_an = analysis.readAudioData, analysis.analyzeAudioData
        seen = {}

        def fake_read(*args, **kw):
            return [np.zeros(144000, dtype="float32")]

        def fake_analyze(chunks, overlap, lat, lon, week):
            _labeled, species = orig_an(chunks, overlap, lat, lon, week)   # the real occurrence list, from the real MData model
            seen["occurrence_count"] = len(species)
            seen["occurrence_has"] = {s: (s in species) for s in UNIVERSE}
            return {"0.0;3.0": [(s, 0.99) for s in UNIVERSE]}, species

        analysis.readAudioData, analysis.analyzeAudioData = fake_read, fake_analyze
        saved_home = os.environ.get("HOME")
        with tempfile.TemporaryDirectory(prefix="ref_pi_lists_") as home:
            os.makedirs(os.path.join(home, "BirdNET-Pi"))
            for kind, text in (("include", inc), ("exclude", exc), ("whitelist", wl)):
                if text is not None:
                    with open(os.path.join(home, "BirdNET-Pi", f"{kind}_species_list.txt"), "w", newline="") as f:
                        f.write(text)
            os.environ["HOME"] = home
            try:
                date = dt.datetime.combine(dt.date.fromisocalendar(2026, WEEK, 1), dt.time(12, 0, 0))
                fname = os.path.join(home, f"{date:%Y-%m-%d}-birdnet-{date:%H:%M:%S}.wav")
                kept = [d.scientific_name for d in analysis.run_analysis(ref.classes.ParseFileName(fname))]
            finally:
                analysis.readAudioData, analysis.analyzeAudioData = orig_read, orig_an
                if saved_home is None:
                    os.environ.pop("HOME", None)
                else:
                    os.environ["HOME"] = saved_home
                log.removeHandler(cap)
        out["scenarios"][name] = {
            "include_file": inc, "exclude_file": exc, "whitelist_file": wl, "sf_thresh": thr,
            "occurrence_count": seen["occurrence_count"], "occurrence_has": seen["occurrence_has"],
            "kept": kept, "log": warnings,
        }
        print(f"{name:40} occurrence {seen['occurrence_count']:4}  kept {kept}")
    with open(a.out, "w") as f:
        json.dump(out, f, indent=1)
        f.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
