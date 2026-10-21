#!/usr/bin/env python3
"""Every detection-affecting default, for a fresh install of BirdNET-Pi, birdnet-go and this project, with file:line.

    python3 defaults_audit.py --pi PI_CHECKOUT --go BIRDNET_GO_CHECKOUT [--repo .] [--markdown OUT.md]

Each cell is a pair (file, regular expression).  The script finds the first line of the file that matches, prints
`value  (file:line)`, and exits 1 if any pattern matches nothing, so a cell cannot go on citing a line that has moved
or a value that has changed.  What a cell *says* is written next to the pattern and is the reader's to judge; what
this checks is that the cited line holds the cited text.  Pinned upstream commits: BirdNET-Pi 88985a3, birdnet-go
30d163ded.
"""
import argparse
import os
import re
import sys

# (setting, {project: (text, file, regex)})  project in pi, go, us
ROWS = [
    ("Classifier a fresh install runs",
     {"pi": ("BirdNET V2.4 FP16 TFLite", "scripts/install_config.sh", r"^MODEL=BirdNET_GLOBAL_6K_V2\.4_Model_FP16"),
      "go": ("BirdNET V2.4 (embedded), `models.enabled: [birdnet]`", "internal/conf/defaults.go", r'"models\.enabled", \[\]string\{"birdnet"\}'),
      "us": ("BirdNET V2.4 FP32 ONNX (`DEFAULT_CLASSIFIER`)", "installer/lib/10-config.sh", r"^DEFAULT_CLASSIFIER=.*CLASSIFIER_V24")}),
    ("CONFIDENCE (threshold)",
     {"pi": ("0.7", "scripts/install_config.sh", r"^CONFIDENCE=0\.7"),
      "go": ("0.8 in code, 0.7 in the shipped config.yaml", "internal/conf/defaults.go", r'"birdnet\.threshold", 0\.8'),
      "us": ("0.70", "crates/birdnet-core/src/config.rs", r"pub const DEFAULT_CONFIDENCE_THRESHOLD: f32 = 0\.70")}),
    ("Threshold comparison",
     {"pi": ("`>=` on the float32 score", "scripts/utils/analysis.py", r"if confidence >= conf\.getfloat\('CONFIDENCE'\)"),
      "go": ("`<=` rejects, so `>` keeps", "internal/analysis/processor/processor.go", r"if result\.Confidence <= confidenceThreshold"),
      "us": ("`>=` on the float32 score", "crates/birdnet-core/src/inference/model.rs", r"if confidence >= self\.config\.confidence_threshold")}),
    ("SENSITIVITY (default)",
     {"pi": ("1.25", "scripts/install_config.sh", r"^SENSITIVITY=1\.25"),
      "go": ("1.0", "internal/conf/defaults.go", r'"birdnet\.sensitivity", 1\.0'),
      "us": ("1.25", "crates/birdnet-core/src/config.rs", r"pub const DEFAULT_SENSITIVITY: f32 = 1\.25")}),
    ("SENSITIVITY (meaning)",
     {"pi": ("sigmoid slope = clamp(2 - S, 0.5, 1.5)", "scripts/utils/models.py", r"self\._sensitivity = max\(0\.5, min\(1\.0 - \(sens - 1\.0\), 1\.5\)\)"),
      "go": ("sigmoid slope = S", "internal/classifier/analyze.go", r"return 1\.0 / \(1\.0 \+ math\.Exp\(-sensitivity\*x\)\)"),
      "us": ("slope = clamp(2 - S, 0.5, 1.5), V2.4 only", "crates/birdnet-core/src/inference/model.rs", r"pub const fn logit_slope")}),
    ("Score mapping",
     {"pi": ("sigmoid(slope x logit) per class", "scripts/utils/models.py", r"return 1 / \(1\.0 \+ np\.exp\(-self\._sensitivity \* logits\)\)"),
      "go": ("sigmoid(S x logit) per class", "internal/classifier/analyze.go", r"confidence := applySigmoidToPredictionsReuse"),
      "us": ("sigmoid(slope x logit); V3.0 emits probabilities and skips it", "crates/birdnet-core/src/inference/model.rs", r"is_probability_output")}),
    ("OVERLAP (seconds)",
     {"pi": ("0.0", "scripts/install_config.sh", r"^OVERLAP=0\.0"),
      "go": ("0.0 in code, 1.5 in the shipped config.yaml", "internal/conf/defaults.go", r'"birdnet\.overlap", 0\.0'),
      "us": ("0.0", "src/daemon/mod.rs", r"cli\.overlap,")}),
    ("Analysis window",
     {"pi": ("3 s at 48 kHz, tail >= 1.5 s kept and zero-padded", "scripts/utils/analysis.py", r"def splitSignal\(sig, rate, overlap, seconds=3\.0, minlen=1\.5\)"),
      "go": ("3 s (the model's input)", "internal/classifier/analyze.go", r"predictions, err := bn\.classifier\.Predict\(sample\[0\]\)"),
      "us": ("from the model's input shape: 3 s for V2.4, 4.5 s for V3.0; tail kept and padded", "crates/birdnet-core/src/detection/pipeline.rs", r"chunk_duration_secs")}),
    ("Recording (segment) length",
     {"pi": ("15 s", "scripts/install_config.sh", r"^RECORDING_LENGTH=15"),
      "go": ("capture buffer analysed every 3 s window; `realtime.interval` 15 s is the merge window, not a recording length", "internal/conf/defaults.go", r'"realtime\.interval", 15'),
      "us": ("15 s", "src/cli.rs", r'default_value = "15", env = "BIRDNET_SEGMENT_DURATION"')}),
    ("SF_THRESH",
     {"pi": ("0.03", "scripts/install_config.sh", r"^SF_THRESH=0\.03"),
      "go": ("0.01", "internal/conf/defaults.go", r'"birdnet\.rangefilter\.threshold", 0\.01'),
      "us": ("0.03", "src/cli.rs", r'default_value = "0\.03", env = "BIRDNET_SF_THRESH"')}),
    ("Range model",
     {"pi": ("DATA_MODEL_VERSION=1 (MData v1)", "scripts/install_config.sh", r"^DATA_MODEL_VERSION=1"),
      "go": ("`latest` range filter model", "internal/conf/defaults.go", r'"birdnet\.rangefilter\.model", RangeFilterModelLatest'),
      "us": ("range v2 ONNX (v1 is not hosted as ONNX)", "installer/lib/62-config-file.sh", r'geo_model_line="METADATA_MODEL_PATH=\$\{MODEL_DIR\}/\$\{V24_RANGE_FILE\}"')}),
    ("Week handed to the range model",
     {"pi": ("ISO week 1-53, raw", "scripts/utils/classes.py", r"week = self\.file_date\.isocalendar\(\)\[1\]"),
      "go": ("48-week year: 4 per month, days 29-31 in week 4", "internal/inference/onnx/rangefilter.go", r"func CalculateWeek\(month, day int\) float32"),
      "us": ("48-week year, the same arithmetic", "crates/birdnet-core/src/civil.rs", r"pub const fn birdnet_week\(month: u32, day: u32\) -> u32")}),
    ("Week stored with a detection",
     {"pi": ("ISO week of the detection's date", "scripts/utils/classes.py", r"self\.week = self\.datetime\.isocalendar\(\)\[1\]"),
      "go": ("none: no week field in `detection.Result` (absence)", None, None),
      "us": ("the 48-week-year week", "crates/birdnet-core/src/detection/daemon/process.rs", r"fn geomodel_week")}),
    ("No location configured",
     {"pi": ("passes the installer's 0.0, 0.0 to the model", "scripts/install_config.sh", r"LATITUDE=0\.0000"),
      "go": ("range filter skipped unless `LocationConfigured`", "internal/analysis/processor/processor.go", r"!settings\.BirdNET\.LocationConfigured"),
      "us": ("occurrence filter off; lists still apply", "crates/birdnet-core/src/detection/daemon/process.rs", r"Always consulted, even with no coordinates")}),
    ("Human-sound rule",
     {"pi": ("any `Human` class in the model's top max(10, 60 x PRIVACY_THRESHOLD) masks the window and its neighbours", "scripts/utils/analysis.py", r"human_cutoff = max\(10, int\(6000 \* priv_thresh / 100\.0\)\)"),
      "go": ("a human class above the threshold drops that detection only; filter on by default", "internal/analysis/processor/processor.go", r"if isHumanSoundLabel\(result\.Species\) && result\.Confidence > baseThreshold"),
      "us": ("Pi's rank rule, rank 10; `PRIVACY_RANK=0` is off", "crates/birdnet-core/src/config.rs", r"pub const DEFAULT_PRIVACY_RANK: usize = 10")}),
    ("PRIVACY_THRESHOLD default",
     {"pi": ("0 (rank rule at 10 still applies)", "scripts/install_config.sh", r"^PRIVACY_THRESHOLD=0"),
      "go": ("`privacyfilter.enabled` true, confidence 0.05", "internal/conf/defaults.go", r'"realtime\.privacyfilter\.confidence", 0\.05'),
      "us": ("0 (off); the rank rule is separate", "src/daemon/mod.rs", r"cli\.privacy_threshold,")}),
    ("Species include list",
     {"pi": ("non-empty list rejects everything not in it; exact scientific name", "scripts/utils/analysis.py", r"if sci_name not in include_list and len\(include_list\) != 0"),
      "go": ("`species.include` means \"always include despite the range filter\": BirdNET-Pi's whitelist, not its include list", "internal/conf/config.go", r"Include \[\]string .*// Always include these species"),
      "us": ("common or scientific name, case-insensitive; a list that matches no species is ignored", "crates/birdnet-core/src/inference/species_filter.rs", r"species include list matches no known species")}),
    ("Species exclude list",
     {"pi": ("exact scientific name; beats include and whitelist", "scripts/utils/analysis.py", r"elif sci_name in exclude_list and len\(exclude_list\) != 0"),
      "go": ("common or scientific name, case-insensitive", "internal/analysis/processor/processor.go", r"if strings\.EqualFold\(commonName, excluded\)"),
      "us": ("common or scientific name, case-insensitive; beats include and whitelist", "crates/birdnet-core/src/inference/species_filter.rs", r"let exclude = resolve_to_scientific\(self\.config\.exclude_list")}),
    ("Whitelist",
     {"pi": ("exempts a species from the occurrence list only", "scripts/utils/analysis.py", r"sci_name not in whitelist_list"),
      "go": ("`species.include` (see the include row): the same word, the opposite meaning", "internal/classifier/range_filter.go", r"len\(settings\.Realtime\.Species\.Include\)"),
      "us": ("exempts from the occurrence threshold only", "crates/birdnet-core/src/inference/species_filter.rs", r"passing\.extend\(resolve_to_scientific\(self\.config\.whitelist")}),
    ("Rows per bird per window / repeat suppression",
     {"pi": ("every window that clears the bar is a row", "scripts/utils/analysis.py", r"for sci_name, confidence in entries:"),
      "go": ("detections merged over `realtime.interval` = 15 s, best kept", "internal/conf/defaults.go", r'"realtime\.interval", 15'),
      "us": ("every window; `DUPLICATE_INTERVAL_SECS` 0 = off", "src/daemon/mod.rs", r"config\.and_then\(\|c\| c\.get_parsed::<i64>\(\"DUPLICATE_INTERVAL_SECS\"\)")}),
    ("Minimum corroboration (detections per species)",
     {"pi": ("none (absence; every window above the bar is a row)", None, None),
      "go": ("`falsepositivefilter.level` 0 = off", "internal/conf/defaults.go", r'"realtime\.falsepositivefilter\.level", 0'),
      "us": ("`CONFIRMATION_LEVEL` off", "src/cli.rs", r'default_value = "off", env = "BIRDNET_CONFIRMATION_LEVEL"')}),
    ("Dynamic (adaptive) threshold",
     {"pi": ("none (absence)", None, None),
      "go": ("ON: trigger 0.90, floor 0.20", "internal/conf/defaults.go", r'"realtime\.dynamicthreshold\.enabled", true'),
      "us": ("off (`BIRDNET_DYNAMIC_THRESHOLD`)", "src/daemon/config.rs", r'enabled: flag\("BIRDNET_DYNAMIC_THRESHOLD"\)\.unwrap_or\(defaults\.enabled\)')}),
    ("Resampler for audio not at the model's rate",
     {"pi": ("librosa `kaiser_fast` (resampy)", "scripts/utils/analysis.py", r"res_type='kaiser_fast'"),
      "go": ("ffmpeg/sox in the capture path; the classifier takes the buffer as is", "internal/classifier/analyze.go", r"sample\[0\]"),
      "us": ("rubato sinc, delay made whole (commit 280f7fc)", "crates/birdnet-core/src/audio/resample.rs", r"Async::<f32>::new_sinc")}),
    ("Date and time of a row",
     {"pi": ("file name's naive local time + window start", "scripts/utils/classes.py", r"self\.datetime = file_date \+ datetime\.timedelta\(seconds=self\.start\)"),
      "go": ("wall clock when the result is made, minus a fixed offset (not the file's name)", "internal/detection/factory.go", r"detectionTime := time\.Now\(\)\.Add\(-DetectionTimeOffset\)"),
      "us": ("file name's local time + window start; UTC instant derived with the local offset", "crates/birdnet-core/src/civil.rs", r"pub fn shift_datetime")}),
    ("Stored confidence",
     {"pi": ("rounded to 4 decimal places", "scripts/utils/classes.py", r"self\.confidence = round\(float\(confidence\), 4\)"),
      "go": ("rounded to 2 decimal places", "internal/detection/factory.go", r"roundedConfidence := math\.Round\(p\.Confidence\*100\) / 100"),
      "us": ("float32 as scored, not rounded (`Confidence` column)", "crates/birdnet-core/src/inference/model.rs", r"confidence >= self\.config\.confidence_threshold")}),
]


def find(root, rel, pattern):
    path = os.path.join(root, rel)
    rx = re.compile(pattern, re.MULTILINE)
    with open(path, encoding="utf-8", errors="replace") as f:
        text = f.read()
    m = rx.search(text)
    if not m:
        return None
    return text.count("\n", 0, m.start()) + 1


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--pi", required=True)
    ap.add_argument("--go", required=True)
    ap.add_argument("--repo", default=".")
    ap.add_argument("--markdown")
    a = ap.parse_args()
    roots = {"pi": a.pi, "go": a.go, "us": a.repo}
    bad = 0
    lines = ["| Setting | BirdNET-Pi `88985a3` | birdnet-go `30d163ded` | This project |", "|---|---|---|---|"]
    for setting, cells in ROWS:
        out = []
        for proj in ("pi", "go", "us"):
            text, rel, pat = cells[proj]
            if rel is None:   # an absence: nothing to cite, and said so
                out.append(text)
                continue
            ln = find(roots[proj], rel, pat)
            if ln is None:
                bad += 1
                print(f"NOT FOUND  {setting} / {proj}: {rel} ~ {pat}", file=sys.stderr)
                out.append(f"{text} (**{rel}: pattern not found**)")
            else:
                out.append(f"{text} (`{rel}:{ln}`)")
        lines.append("| " + setting + " | " + " | ".join(out) + " |")
    md = "\n".join(lines) + "\n"
    if a.markdown:
        open(a.markdown, "w").write(md)
    else:
        print(md)
    print(f"{len(ROWS)} settings, {3 * len(ROWS)} cells, {bad} not found", file=sys.stderr)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
