# Measuring the Station

This page describes the instruments that measure the station against ground truth, and how to use them. **None of the measurements below has been made.** The repository holds the harness, the formats, the arithmetic and their tests. It does not hold annotated station recordings, timings from a Raspberry Pi, or a model record made on aarch64. Until someone runs the protocol and publishes the result, the station's accuracy is not known.

"Parity" elsewhere in this manual ([Same audio, same detections](./parity.md)) means agreement with BirdNET-Pi's code on the same audio. It is not accuracy: two programs can agree and both be wrong.

## The instruments

All of them are in the `birdnet-core` crate. The command-line front end is the `model_check` example:

```text
cargo run --release -p birdnet-core --example model_check -- <command> ...
```

| Command | What it does | Audit row |
|---|---|---|
| `predict` | Runs every recording in a directory through the station's own decode, resample and chunking, then through the classifier, and writes every window's score for every species above `--floor` as a predictions TSV | ACC-03, ACC-20, MMP-15 |
| `score` | Scores a predictions TSV against Raven selection tables. It reports per-species and pooled precision and recall at each threshold, with Wilson and recording-cluster bootstrap intervals | ACC-03, ACC-20, MMP-15 |
| `record` | Writes a classifier's raw output on one recording, bit for bit, with the digests of the model, the labels and the input | ACC-12, RES-31 |
| `compare` | Diffs two records. It exits 1 when they differ, either bit for bit or beyond `--max-abs-diff` | ACC-12, RES-31 |
| `bench` | Times one window's inference, many times, and reports the median, p10, p90, p99, IQR, minimum and maximum | MMP-13 |

`predict` runs the classifier the way the station feeds it. It skips the station's filters (privacy, noise, occurrence, confirmation, duplicate interval), so it measures the classifier on the station's input, not the stored rows. To score the stored rows, write them in the predictions format below. The predictions are capped at `--floor` (default 0.01), so a threshold sweep can only reach down to the floor.

### Formats

**Annotations** are Raven selection tables. A table is tab-separated, with columns `Begin Time (s)`, `End Time (s)` and a species column (`Species` by default; set another with `--species-column`). A `Begin File` or `Begin Path` column says which recording each selection belongs to. Without one, the table describes the recording its file is named after (`<recording>.Table.1.selections.txt`). Raven writes a selection once per view, and the repeats are read once. A selection with a blank species is skipped. A selection whose times do not parse is an error, because dropping it silently would raise recall.

**Predictions** are a TSV with the header `recording  begin_secs  end_secs  species  score`. The recording is the file stem, so `site1_0600.wav` is `site1_0600`.

### The scoring rule

- **Precision counts detections.** A detection at or above the threshold is a true positive when it overlaps an annotation of the same species in the same recording by more than `--min-overlap` seconds (default: any overlap of positive length).
- **Recall counts annotations.** An annotation is recalled when at least one detection at or above the threshold overlaps it.
- **Only annotated recordings are scored.** A recording the annotators went through and marked nothing in still counts, and every detection in it is a false positive. A recording nobody annotated counts for nothing.

### The intervals

- **Wilson score interval (95%)** on each proportion. It treats detections, or annotations, as independent. Overlapping windows of one recording are not independent, so this interval is too narrow when they cluster.
- **Recording-cluster bootstrap (95%, percentile method)** on the pooled rows. Recordings are resampled with replacement (`--replicates`, default 1000) from a fixed `--seed`, so a report can be reproduced. Quote this interval when the two disagree. Neither interval says anything about recordings unlike the ones sampled: another site, season, microphone or year.

## Protocol: precision and recall of the default configuration (ACC-03)

1. **Sample before you look.** Draw recordings from the station's own archive at random. Stratify by site, month and hour, and write the design down before any recording is opened. Hand-picked recordings measure the picker.
2. **Annotate blind.** Annotators mark every sound of every target species in each recording, without seeing the station's output. Annotate whole recordings, not detections. Recall cannot be measured from detections alone.
3. **Measure the annotation.** Have a second annotator do a random subset, and report their agreement. The score is only as good as the annotation.
4. **Hold out.** Set any threshold on one part of the sample, and report precision and recall on the other part.
5. **Run.** Use `predict` with the station's model files on the held-out recordings, then `score` with the thresholds the station uses. Publish the table, the sampling design, the model digests (`record` prints them) and the seed.

The result is agreement with those annotators on those recordings. It does not mean a detection is correct with that probability at another station. A recall below 1 also does not show which birds were absent.

## Protocol: the V3.0 window length and privacy default (ACC-20)

- **Window length.** Run `predict` on one annotated set twice, with `--window-secs 3.0` and with `--window-secs 4.5` (the length the daemon picks for V3.0). Score both with one plan and one seed, and compare the precision–recall curves per species with their intervals. One clip is not evidence. The daemon's choice of 4.5 s rests on one recording: the magpie's confidence was 0.52 with a 3 s window and 0.72 with 4.5 s (the comment in `fit_pipeline_to_classifiers`).
- **Privacy.** This needs recordings of speech made with consent, annotated as speech, and annotated recordings with no speech. The human classes' predictions are scored as a species against those annotations. Masking recall (speech windows masked) and the cost (bird windows masked with no speech) both need reporting. No such recordings are in this repository, and none have been collected for it.

## Protocol: Perch's bar on station audio (MMP-15)

`tools/perch-bar` measured Perch v2 on public annotated datasets. That is a measurement of the model on those recordings, not of a station. To validate the bar on a station, annotate that station's recordings as in the ACC-03 protocol, then run `predict` with the Perch model and its labels and `score` with `--thresholds` around the bar. Report the curve, not one point, together with the bar the station uses.

## A model record on aarch64 (ACC-12) and reproducibility (RES-31)

`crates/birdnet-core/tests/model_output_is_reproducible.rs` runs the bundled magpie clip through the pipeline and the 11-class test model twice, and requires the output to be bit-identical. It then holds the output to `crates/birdnet-core/tests/data/golden/tiny_v24_magpie.json`, which was made on x86-64 Linux. On the same platform the match must be bit for bit. On another platform the test prints the comparison and requires every raw output within 1e-5, no change of top class, and no value that is finite on one side only. The test model checks the pipeline and the ONNX runtime. It does not check BirdNET's weights.

**The CI job that would close ACC-12** (not added; workflows are the maintainer's) runs on an aarch64 runner, for example GitHub's `ubuntu-24.04-arm`:

1. Check out, install the pinned toolchain, and seed the ONNX Runtime cache as the x86 jobs do.
2. `cargo test -p birdnet-core --all-features --test model_output_is_reproducible`
3. With the real model fetched as the installer fetches it:
   `model_check record --model <V2.4 onnx> --labels <labels> --input tests/testdata/Pica_pica_30s.wav --out aarch64.json`, then
   `model_check compare x86_64.json aarch64.json --max-abs-diff <bound>`, where `x86_64.json` was recorded the same way on x86-64 and committed. Choose the bound from the first comparison, publish it, and keep the printed comparison (top-1 changes and threshold crossings) as the job's artefact.

Bit-identical output across architectures is not expected, because SIMD kernels sum in a different order. The comparison reports by how much the outputs differ. It does not decide whether that difference is acceptable.

## Inference time on a Pi (MMP-13)

`model_check bench --model <onnx> --labels <labels> --threads <n> --runs 200 --warmup 10` reports one window's inference time with its spread, plus the architecture and thread count. A run on a development machine says nothing about a Pi. On the Pi:

- Run it three times, cold and after ten minutes of load. Record the SoC temperature, any throttling (`vcgencmd get_throttled`) and the cooling.
- Run it at `--threads 1`, 2, 3 and 4. The station uses 2.
- A thread setting must also survive the service unit's sandbox: 0.17.0's unit killed ONNX Runtime's thread pool (`SystemCallFilter`, `sched_setaffinity`). A bench run by hand does not show that. Check it under the unit.

No Pi timing has been made with this.
