#!/usr/bin/env python3
"""What birdnet-go's *rules* would store, applied to the model outputs this project dumped.  An estimate, not a run.

    birdnet_go_estimate.py OURS.json [OURS2.json ...] --pi-golden x0_pi_default.json [--pi-golden-2 a1_station.json]

birdnet-go itself was not executed (it needs a Go build and its own runtime libraries); this applies the rules its source
shows (`internal/classifier/analyze.go`, `internal/analysis/processor/processor.go`, commit `30d163ded`) to the same
per-window top-10 raw logits `tests/parity_dump.rs` writes (FP32 ONNX, the model birdnet-go also runs):

  score      sigmoid(sensitivity x logit), sensitivity 1.0           (analyze.go: customSigmoid)
  keep       score > threshold, strict                                (processor.go: `Confidence <= threshold` filters)
  threshold  0.8 (code default) or 0.7 (the shipped config.yaml)
  merge      one row per species per 15 s of a recording, the best score of the 15 s    (`realtime.interval`)
  human      a human class above the threshold drops that detection only; other classes of the window stay

It does not model the dynamic threshold (on by default there), the range filter (skipped without a location), the daylight
filter or any per-species setting, so it is the rules' effect on the scores and nothing else.  Compared against
BirdNET-Pi's stored rows for the same clips.
"""
import argparse
import json
import math
import re
import sys


def sig(x, s=1.0):
    return 1.0 / (1.0 + math.exp(-s * x))


def go_rows(windows, threshold, strict=True, interval=15.0):
    rows = {}
    for w in windows:
        for label, logit, _p in w["top"]:
            if "human" in label.lower() or "homo sapiens" in label.lower():
                continue
            c = sig(logit)
            if (c > threshold) if strict else (c >= threshold):
                key = (label, int(w["start"] // interval))
                if key not in rows or c > rows[key][1]:
                    rows[key] = (w["start"], c)
    return rows


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("ours", nargs="+")
    ap.add_argument("--pi-golden", required=True)
    a = ap.parse_args()
    ours = {}
    for p in a.ours:
        ours.update(json.loads(re.sub(r"(?<![\w.])-?inf\b", "-1e9", open(p).read())))
    pi = {r["clip"]["file"].removesuffix(".wav"): r for r in json.load(open(a.pi_golden))["runs"]}
    names = sorted(set(ours) & set(pi))
    pi_rows = {n: {(d["scientific_name"], d["start"]) for d in pi[n]["detections"]} for n in names}
    n_pi = sum(len(v) for v in pi_rows.values())
    print(f"{len(names)} clips; BirdNET-Pi stores {n_pi} rows on them")
    for thr in (0.8, 0.7):
        n_go = clips_with = same_species_clips = 0
        both_species = 0
        go_species_total = pi_species_total = 0
        for n in names:
            rows = go_rows(ours[n], thr)
            go_sp = {k[0] for k in rows}
            pi_sp = {s for s, _ in pi_rows[n]}
            n_go += len(rows)
            clips_with += bool(rows)
            go_species_total += len(go_sp)
            pi_species_total += len(pi_sp)
            both_species += len(go_sp & pi_sp)
        print(f"birdnet-go rules, threshold {thr}: {n_go} rows on {clips_with} clips; species per clip: {go_species_total} (BirdNET-Pi {pi_species_total}), shared {both_species}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
