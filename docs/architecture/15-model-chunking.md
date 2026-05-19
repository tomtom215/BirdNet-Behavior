# Chunk sizing for BirdNET model families

> Why our pipeline now picks the audio chunk length based on the model,
> and the empirical evidence that drove the default.

## Context

When the project moved from BirdNET V2.4 (BirdNET-Pi's model) to the
BirdNET+ V3.0 *developer preview* model, two of the model's published
contract changes had non-obvious effects on detection confidence:

- Sample rate dropped from 48 kHz to 32 kHz (documented).
- Input length became **variable** instead of fixed 3 seconds
  (documented).

The pipeline was already auto-adjusting the sample rate to match the
model. It was **not** auto-adjusting the chunk length: it kept feeding
the V3.0 model 3.0-second chunks (96 000 samples at 32 kHz) because
that's the value V2.4 was trained on.

## Finding

Same `tests/testdata/Pica_pica_30s.wav` fixture, same model file
(`BirdNET+_V3.0-preview3_Global_11K_FP32.onnx`), Python ONNX Runtime
sweeping chunk length in 0.5 s steps:

| Chunk samples | Seconds @ 32 kHz | Best Pica pica score |
|--------------:|-----------------:|----------------------|
|        64 000 |             2.00 | 0.501                |
|        80 000 |             2.50 | 0.501                |
|        96 000 |             3.00 | 0.521                |
|       112 000 |             3.50 | 0.704                |
|       128 000 |             4.00 | 0.717                |
|     **144 000** |         **4.50** | **0.718**            |
|       160 000 |             5.00 | 0.717                |
|       176 000 |             5.50 | 0.720                |
|       192 000 |             6.00 | 0.715                |

The cliff between 3.0 s and 3.5 s is real: confidence climbs from 0.52
to 0.70 and then plateaus. **For the V3.0 preview models we now default
to 144 000 samples (= 4.5 s at 32 kHz).** That's also numerically the
same chunk size V2.4 used (at 48 kHz × 3 s); the model happens to
extract better features when it sees roughly that many samples
regardless of sample rate.

## For comparison: BirdNET V2.4

The same WAV processed through `birdnet-analyzer` (V2.4) with its
default 3.0 s chunks gives the canonical reference numbers BirdNET-Pi
users will recognise:

| Begin | End | Confidence |
|------:|----:|------------|
|   3 s | 6 s | 0.958      |
|  12 s | 15 s| 0.970      |
|  21 s | 24 s| 0.939      |

So the V3.0 preview model is **inherently less confident** than V2.4 on
common European species — its 11 560-class output spreads probability
mass across roughly twice as many possible labels, and its training
set is documented as "may not reflect final performance." This is not
a pipeline bug. Operators who need the higher V2.4 confidence numbers
can run the V2.4 model — it remains supported (fixed-shape
`[1, 144 000]`).

## Decision

1. `BirdNetModel::recommended_chunk_samples()` returns the model's
   trained length when the input shape is fixed and 144 000 when it is
   dynamic (V3.0 preview).
2. `BirdNetModel::recommended_chunk_secs()` divides that by the
   model's inferred sample rate.
3. `start_detection_daemon` adopts the model's recommendation when it
   differs from the pipeline's configured `chunk_duration_secs`,
   logging the adjustment alongside the existing sample-rate and
   raw-vs-mel auto-adjustments.
4. The chunk length is still operator-overridable: anyone needing
   bit-for-bit reproducibility against a fixed length can set
   `chunk_duration_secs` in code or via the config plumbing.

## Verification

End-to-end against the bundled WAV with the rebuilt binary:

```
$ grep "adjusting pipeline chunk" /tmp/birdnet-e2e/daemon.log
… adjusting pipeline chunk duration to match model recommendation
  configured_chunk_secs=3.0 model_chunk_secs=4.5
```

```
$ sqlite3 -header -column /tmp/birdnet-e2e/data/birds.db \
    "SELECT Sci_Name, Com_Name, ROUND(Confidence*100,1) AS conf_pct
       FROM detections WHERE Sci_Name LIKE 'Pica%' ORDER BY Confidence DESC"
Sci_Name   Com_Name         conf_pct
---------  ---------------  --------
Pica pica  Eurasian Magpie  71.5
```

vs the previous Rust pipeline (same daemon, same model, same WAV, only
chunk length changed):

```
Sci_Name          Com_Name             conf_pct
----------------  -------------------  --------
Pica serica       Oriental Magpie      61.1
Pica pica         Eurasian Magpie      52.2
Pica bottanensis  Black-rumped Magpie  50.1
```

The 19-point absolute confidence gain on the target species is the
single biggest accuracy improvement of this branch.

## Related issue: unique-key constraint loses duplicate-species chunks

The detections schema declares `UNIQUE(Date, Time, Sci_Name)`. Because
all chunks of one recording inherit the same `(Date, Time)` from the
filename, a bird that calls in multiple chunks of the same file
currently produces exactly one detection row in the database — the
highest-confidence one is the *first* one inserted, not the *best* one.
This is a separate issue from chunking and is tracked for a follow-up
schema migration (proposed key:
`UNIQUE(Date, Time, Sci_Name, File_Name, chunk_offset)`).

## See also

- [`docs/architecture/05-audio-pipeline.md`](05-audio-pipeline.md) — audio path overview
- [`docs/architecture/06-ml-inference.md`](06-ml-inference.md) — model contract
- [`crates/birdnet-core/src/inference/model.rs`](../../crates/birdnet-core/src/inference/model.rs)
  — `recommended_chunk_samples` and `recommended_chunk_secs`
- [`crates/birdnet-core/src/detection/daemon.rs`](../../crates/birdnet-core/src/detection/daemon.rs)
  — pipeline auto-adjustment
