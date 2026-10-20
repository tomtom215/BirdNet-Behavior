# Same audio, same detections

If you ran BirdNET-Pi for years and trust what it told you, you should be able to
move here and see the same birds for the same sounds. This page says what that
takes, what was measured, and where the three projects differ whatever you do.
BirdNET-Pi here means [Nachtzuster's fork](https://github.com/Nachtzuster/BirdNET-Pi)
at `88985a3`; birdnet-go means [tphakala/birdnet-go](https://github.com/tphakala/birdnet-go)
at `f4f019a`.

**Same model does not mean same detections, even between BirdNET-Pi and
birdnet-go.** They run the same BirdNET V2.4 network and differ in how a score
becomes a stored row (the table below). This project is held to BirdNET-Pi,
because that is where its users and its settings file come from.

## What you get out of the box

A new install runs **BirdNET V2.4** with its **range (occurrence) filter v2**, and the three settings that decide what becomes a stored row default to BirdNET-Pi's own values:

| | BirdNET-Pi | A new install here |
|---|---|---|
| Classifier | BirdNET V2.4 | BirdNET V2.4 (`birdnet-v24.onnx`, FP32 where BirdNET-Pi runs FP16) |
| Occurrence filter | MData v1 by default | MData v2 (`birdnet-v24-range-v2.onnx`); v1 is not hosted as ONNX |
| `CONFIDENCE` | 0.70 | 0.70 (it was 0.75 up to v0.17.1) |
| `SENSITIVITY` | 1.25 | 1.25, with BirdNET-Pi's meaning |
| Human-sound rule | on: a human class in the top 10 masks the window | on: `PRIVACY_RANK` 10, the same rule |

BirdNET+ V3.0 preview, which releases up to v0.17.1 installed for everyone, is a different network: it listens to 4.5 s windows (V2.4: 3 s), reports calibrated probabilities (V2.4: raw scores that `SENSITIVITY` reshapes), and names about 11 500 species (V2.4: 6 522). Its detections are not BirdNET-Pi's and are not meant to be. Choose it with `--classifier birdnet-v3` (see [Installation](../getting-started/installation.md#which-classifier-a-new-install-runs)) if you want the newer model.

Two things to know if you are upgrading rather than installing:

- **The classifier is not changed.** A station that has a configuration keeps the classifier that configuration names, and a Docker volume that holds the V3.0 model keeps V3.0.
- **Two defaults move with the binary.** A station that never set `CONFIDENCE` (not in `birdnet.conf`, not saved on the Settings page or in the onboarding wizard) now records at 0.70, not 0.75. A station that sets neither `PRIVACY_RANK` nor `PRIVACY_THRESHOLD` now has the human-sound rule **on**, where before nothing was suppressed: a window in which a human class ranks in the model's top 10 classes, and the windows either side, are not recorded. Set `PRIVACY_RANK=0` to switch it off.

## Moving an existing station to BirdNET-Pi's results

Install the two files (Docker: `docker compose exec birdnet birdnet-behavior --install-model …`; on bare metal name the model folder, as [Installing one](../admin/classifiers.md#installing-one) shows):

```bash
birdnet-behavior --install-model birdnet-v24            # classifier + labels
birdnet-behavior --install-model birdnet-v24-range-v2   # species-occurrence filter
```

Then, in `birdnet.conf` (or the `BIRDNET_*` variables), and restart. `<model dir>` is `/data/model` in Docker and `<data dir>/models` on bare metal:

```ini
MODEL_PATH=<model dir>/birdnet-v24.onnx
LABELS_PATH=<model dir>/birdnet-v24_labels.txt
METADATA_MODEL_PATH=<model dir>/birdnet-v24-range-v2.onnx
LATITUDE=…            # needed by the range filter, as in BirdNET-Pi
LONGITUDE=…
CONFIDENCE=0.70       # BirdNET-Pi's default, and this project's since v0.17.1
SENSITIVITY=1.25      # the same number, with the same meaning, as BirdNET-Pi's
SF_THRESH=0.03        # the same
OVERLAP=0.0           # the same
PRIVACY_RANK=10       # the same rule; see below
```

With those, a V2.4 station stores BirdNET-Pi's species at BirdNET-Pi's confidences for the same audio, to the tolerances below. The last four lines are the defaults; they are written out so that a value you changed earlier is visibly put back. The one setting that does not carry across unchanged is BirdNET-Pi's `PRIVACY_THRESHOLD`.

**Migrating a `PRIVACY_THRESHOLD`.** BirdNET-Pi reads the key as a percentage of the species list: a window is human if a human class ranks within the top `max(10, 60 × value)` classes. Here `PRIVACY_THRESHOLD` is the model's own confidence for a human sound, and the rank rule has a key of its own. For a BirdNET-Pi value `p`, set `PRIVACY_RANK=max(10, 60 × p)` and `PRIVACY_THRESHOLD=0` (so `p=0` gives 10, `p=1` gives 60 and `p=10` gives 600). A BirdNET-Pi `1` left in `PRIVACY_THRESHOLD` is in range here, validates clean, and then masks nothing short of certainty; `--doctor` warns about any value above 0.5 and says this. `PRIVACY_RANK` accepts 0 (off) to 12 000, as `--privacy-rank` and `BIRDNET_PRIVACY_RANK` as well as the config key. It is not yet a field on the Settings page (one is planned), so it is set in the file, the environment or on the command line. Either rule suppresses a window: `PRIVACY_THRESHOLD` (0 = off, typically 0.01–0.03 when used) is a second, independent rule on top of the rank rule.

The commit that added `PRIVACY_RANK` (`6888399`) reports its decision agreeing window for window with BirdNET-Pi's own masks on 13 of the 14 parity clips with the real V2.4 model, including all four speech windows; the fourteenth is the synthetic clip's digital-silence tail, which is already listed below as disagreeing between every pair of model builds. That comparison is `tools/parity/compare_rust.py` (its `privacy` column); it was not re-run for this page.

## What was measured

Against BirdNET-Pi's own, unmodified code running its V2.4 FP16 TFLite model on 14
clips (a magpie, a binaural dawn chorus in several formats, a tawny owl, a crow, a
speech recording, synthetic silence and noise). Every number below was produced by
a command in [`tools/parity`](https://github.com/tomtom215/BirdNet-Behavior/tree/main/tools/parity)
and is regenerated by `tools/parity/regenerate.sh`; the Rust side is
`tests/parity_dump.rs`, `tests/v24_matches_birdnet_pi.rs` and
`tests/v24_range_filter_matches_birdnet_pi.rs`.

| Stage | Result |
|---|---|
| Decoding and mixing to mono, native 48 kHz | **Bit-identical** to BirdNET-Pi's samples on all six such clips (mono, stereo, 16- and 32-bit; first three chunks each, max difference 0.0) |
| Chunk boundaries | Same count and start on 13 of 13 clips BirdNET-Pi gives chunks for; and equal to a transcription of its `splitSignal` for six overlaps × 155 recording lengths |
| Confidence | The magpie recording's three detections store 0.9123 / 0.9318 / 0.8864; BirdNET-Pi's own test asserts 0.912 / 0.9316 / 0.8857 |
| What is stored, at `CONFIDENCE=0.7` | The same species in the same chunks on 12 of 13 clips; the exception is one borderline call in a very quiet clip that had to be resampled from 44.1 kHz |
| Raw model scores, native 48 kHz clips | Within 0.02–0.08 of BirdNET-Pi's. That gap is the weights: BirdNET-Pi runs the FP16 build and the ONNX used here is the FP32 build of the same network |
| Species-occurrence filter | The ONNX range filter returns the same species list as BirdNET-Pi's TFLite one for all 12 questions asked (two places × six weeks, including weeks 49–53 and the "unset" coordinates): same count, same hash of the sorted names |

### The default V3.0 path, against an independent reference

There is no BirdNET-Pi to compare V3.0 with, so `tools/parity/v3_reference.py` is a
reference that shares no code with the pipeline: soundfile, a plain-mean mono mix,
**soxr** (not rubato) for 48 → 32 kHz, slicing into 4.5 s windows, onnxruntime.
Over four clips (17 windows: a magpie, a chickadee soundscape, a crow, a song thrush)
both name the same top species in every window, and every window that scores 0.7 or
more agrees to within 0.0015 (for instance the magpie at 0.9256 here and 0.9249 there,
a chickadee at 0.7464 and 0.7463). The weakest windows differ most in absolute terms
(0.103 against 0.080 for a magpie window that scores far below any threshold). This
checks the front end (decode, resample, window) and the confidence mapping of the
default model; it does not say V3.0 identifies birds better or worse than V2.4.

### What is not the same, measured

- **Digital silence and zero-padded tails.** The V2.4 graph normalises each window
  by its own range, so a window that is constant becomes noise. Every build of it
  disagrees with every other on such windows by up to 6 raw-score units (top species
  differ). Real recordings contain neither, and BirdNET-Pi's last, padded window is
  affected the same way.
- **Audio that had to be resampled.** Recorders that write 48 kHz involve no
  resampling here or in BirdNET-Pi. For other rates the two resamplers agree to
  under one sample in position and differ in filter shape. On two very quiet clips
  (signal RMS about 20 LSB) our raw scores differ from BirdNET-Pi's by up to 1.75;
  BirdNET-Pi's own model moves by 1.8 on average on one of them when its input is
  changed by ±½ LSB. That is the model's sensitivity to a nearly silent input, not
  a fault in either resampler.
- **A recording shorter than 1.5 s** gets one padded chunk here and none in
  BirdNET-Pi.

Not measured: BirdNET-Pi's FP16 model on a real Raspberry Pi, any model other than
V2.4, and the whole path with BirdNET-Pi's default range filter (version 1), which
this project's catalogue does not carry because no host serves it as ONNX.

## Where they differ whatever you do

With the defaults above, five things still make a default station differ from BirdNET-Pi; each is explained below the table. (1) The range filter is v2, not v1. (2) The week is the model's 48-week year, not the ISO week, which shows in December. (3) With no location set this station runs without the filter and BirdNET-Pi does not. (4) A recording dated outside 2024–2063 is quarantined. (5) Under load, segments are shed and counted.

| | BirdNET-Pi | birdnet-go | This project |
|---|---|---|---|
| **Default model** | V2.4 | V2.4 | V2.4 for a new install; BirdNET+ V3.0 preview is a choice (`--classifier birdnet-v3`) and an older station keeps the model it has |
| **`SENSITIVITY`** | sigmoid slope `clamp(2 − S, 0.5, 1.5)`; default 1.25 → 0.75 | the slope itself; default 1.0 | BirdNET-Pi's, on V2.4; ignored by V3.0 |
| **Confidence threshold** | 0.7, `>=` | 0.7 first run (0.8 in code), `>` | 0.70, `>=` |
| **Overlap** | 0.0 | 1.5 s first run | 0.0 |
| **Range filter** | MData v1 (v2 optional) | MData v2 | v2 with V2.4 (v1 is not hosted as ONNX, so it is not offered); V3.0 has its own geomodel |
| **Range-filter week** | ISO week, 1–53, passed raw | 48-week year (4 per month) | 48-week year |
| **Range-filter threshold** | 0.03 | 0.01 | 0.03 |
| **No location set** | passes 0,0 or −1,−1 to the model, which reads it as the Gulf of Guinea and drops temperate species | filter skipped | filter off; every species passes |
| **Human-sound privacy** | rank rule: any human class in the top 10 masks the window and its neighbours, even at `PRIVACY_THRESHOLD=0` | on, 0.05 | the same rank rule, on by default (`PRIVACY_RANK` 10; 0 = off). `PRIVACY_THRESHOLD`, a confidence (typically 0.01–0.03; 0 = off, the default), is a second rule beside it; either suppresses |
| **Stored rows** | every window that clears the bar | merged over 12 s, 15 s per-species cooldown, dynamic threshold on | every window that clears the bar; duplicate and corroboration filters off by default |

**No location set.** BirdNET-Pi's installer asks `ip-api.com` for a location and falls back to 0.0, 0.0 when that fails (`tools/parity/README.md`). At 0.0, 0.0 with the v1 filter and `SF_THRESH` 0.03 the filter admitted 468 species at ISO week 8, and BirdNET-Pi's own Pica pica test recording produced no detections: `tools/parity/golden/matrix/a0_shipped_literal.json` holds `detections: []` and `mdata.count: 468`, and its `excluded_log` names "Eurasian Magpie" as excluded as below the occurrence threshold. Here a station with no coordinates configured runs without the occurrence filter, so nothing is dropped for its location (that recording was not run through this project at 0.0, 0.0 for this page). Coordinates you do set are used as given, and the filter needs both of them.

**The week matters in December.** BirdNET-Pi passes the ISO week to a model that
expects weeks 1–48 and treats anything above 48 as "no seasonal information".
From 30 November to 31 December it therefore applies a year-round list about twice
as long as any real week's. Across 2026 the species lists differ between the two
calendars on 323 of 365 days (mean 14 species, at most 99). This project, like
birdnet-go, uses the model's own 48-week year, so in December it filters out
species BirdNET-Pi lets through.

**Other things that can keep a row out of the table here:** a recording dated before
2024 or after 2063 is quarantined rather than stored (a clock that has lost its
battery must not write years of 1970s rows); when more than 40 segments are
waiting, the CPU passes 80 °C, or a Pi reports throttling, every other segment is
skipped and counted; and a file that is not named
`YYYY-MM-DD-birdnet[-ID]-HH:MM:SS.wav` is refused. BirdNET-Pi does none of these.

## Checking it yourself

```bash
# the reference side: BirdNET-Pi's code and model, goldens in tools/parity/golden
tools/parity/regenerate.sh
# the Rust side, over the same clips
PARITY_MODEL=… PARITY_LABELS=… PARITY_WAVS=a.wav:b.wav PARITY_OUT=out \
  cargo test --features analytics --test parity_dump
python3 tools/parity/compare_rust.py out/ours.json tools/parity/golden/matrix/a1_station.json
```

CI fetches the V2.4 model and range filter, checks their sha256, and runs the two
V2.4 tests above on every change to the inference path.
