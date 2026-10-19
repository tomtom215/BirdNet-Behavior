#!/usr/bin/env python3
"""Diff BirdNet-Behavior's per-chunk V2.4 output against BirdNET-Pi's golden.

OURS   is the JSON the Rust probe writes (one key per clip; per chunk: start,
       end, and the model's top 10 as [scientific name, raw logit, p]).
GOLDEN is a ``golden/matrix/*.json`` written by ``ref_pi.py`` (BirdNET-Pi's own
       code on the V2.4 FP16 TFLite model).

For every clip it compares the chunking (count and start offsets), the top-1
label of each chunk, the raw logits of the labels both sides report, and the
set of (chunk, species) pairs that clear ``--confidence`` once BirdNET-Pi's
sigmoid slope ``max(0.5, min(1 - (S - 1), 1.5))`` is applied to OUR logits.
That last comparison is the decision BirdNET-Pi would make before its
occurrence filter, so it isolates the model, resampler and chunker from the
range filter.

Exit status is 1 if any clip's chunking differs (other than the documented
one-chunk floor for a recording shorter than the minimum tail), or any chunk's
top-1 label differs, or the decision sets differ.  Logit differences are
reported, not gated: they are a property of the ONNX build under test, and the
README of this directory says how much each build moves.

Usage: compare_rust.py OURS.json GOLDEN.json [--confidence 0.7] [--sensitivity 1.25]
"""
import argparse
import json
import math
import sys


def slope(sensitivity: float) -> float:
    return max(0.5, min(1.0 - (sensitivity - 1.0), 1.5))


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("ours")
    ap.add_argument("golden")
    ap.add_argument("--confidence", type=float, default=0.7)
    ap.add_argument("--sensitivity", type=float, default=1.25)
    ap.add_argument("--min-tail-floor", action="store_true", default=True,
                    help="accept one chunk where BirdNET-Pi has none (recording under 1.5 s)")
    args = ap.parse_args()

    with open(args.ours) as f:
        ours = json.load(f)
    with open(args.golden) as f:
        golden = json.load(f)

    s = slope(args.sensitivity)
    bad = 0
    print(f"slope {s}, confidence >= {args.confidence}")
    print(f"{'clip':28} {'chunks':>9} {'top1':>7} {'max|dlogit|':>12} {'decisions':>10}")
    worst = 0.0
    for run in golden["runs"]:
        name = run["clip"]["file"].removesuffix(".wav")
        gch = run["chunks"]
        och = ours.get(name)
        if och is None:
            print(f"{name:28} MISSING from ours")
            bad += 1
            continue

        notes = []
        if len(gch) != len(och):
            if len(gch) == 0 and len(och) == 1 and args.min_tail_floor:
                notes.append("ours keeps the single padded chunk (documented)")
            else:
                notes.append(f"CHUNK COUNT {len(och)} vs {len(gch)}")
                bad += 1
        elif any(abs(g["start"] - o["start"]) > 1e-3 for g, o in zip(gch, och)):
            notes.append("CHUNK STARTS DIFFER")
            bad += 1

        top1_ok = top1_n = 0
        max_dlogit = 0.0
        dec_ok = dec_n = 0
        for g, o in zip(gch, och):
            # Pi's top10_logits aligns with its top10 labels.
            pi_logit = {lab: lg for (lab, _), lg in zip(g["top10"], g["top10_logits"])}
            our_logit = {lab: lg for lab, lg, _p in o["top"]}
            top1_n += 1
            top1_ok += g["top10"][0][0] == o["top"][0][0]
            for lab in pi_logit.keys() & our_logit.keys():
                max_dlogit = max(max_dlogit, abs(pi_logit[lab] - our_logit[lab]))
            # Decision sets before any occurrence filter.  BirdNET-Pi looks at
            # its own top 10 only; so does the probe.
            pi_set = {lab for lab, sc in g["top10"] if sc >= args.confidence}
            our_set = {lab for lab, lg, _p in o["top"]
                       if 1.0 / (1.0 + math.exp(-s * lg)) >= args.confidence}
            dec_n += 1
            if pi_set == our_set:
                dec_ok += 1
            else:
                notes.append(f"chunk@{g['start']}: pi-only {sorted(pi_set - our_set)} ours-only {sorted(our_set - pi_set)}")
                bad += 1
        if top1_ok != top1_n:
            bad += 1
        worst = max(worst, max_dlogit)
        print(f"{name:28} {len(och):>3}/{len(gch):<5} {top1_ok:>3}/{top1_n:<3} {max_dlogit:>12.4f} {dec_ok:>5}/{dec_n:<4}")
        for n in notes:
            print(f"    {n}")
    print(f"worst max|dlogit| over clips: {worst:.4f}")
    print("RESULT:", "DIFFERENCES FOUND" if bad else "identical decisions, chunking and top-1 on every clip")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
