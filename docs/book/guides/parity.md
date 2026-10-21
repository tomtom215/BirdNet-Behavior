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

BirdNET+ V3.0 preview, which releases up to v0.17.1 installed for everyone, is a different network: it listens to 4.5 s windows (V2.4: 3 s), reports calibrated probabilities (V2.4: raw scores that `SENSITIVITY` reshapes), and names about 11 500 species (V2.4: 6 522). Its detections are not BirdNET-Pi's and are not meant to be. Choose it with `--classifier birdnet-v3` (see [Installation](../getting-started/installation.md#which-classifier-a-new-install-runs)) if you want the newer model. Two defaults here are tuned to V2.4 and say less on V3.0: `SENSITIVITY` is ignored (V3.0 reports calibrated probabilities), and the human-sound rule hardly fires (measured below: 0 of 31 windows of read speech).

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

**Migrating a `PRIVACY_THRESHOLD`.** BirdNET-Pi reads the key as a percentage of the species list: a window is human if a human class ranks within the top `max(10, 60 × value)` classes. Here `PRIVACY_THRESHOLD` is the model's own confidence for a human sound, and the rank rule has a key of its own. For a BirdNET-Pi value `p`, set `PRIVACY_RANK=max(10, 60 × p)` and `PRIVACY_THRESHOLD=0` (so `p=0` gives 10, `p=1` gives 60 and `p=10` gives 600). A BirdNET-Pi `1` left in `PRIVACY_THRESHOLD` is in range here, validates clean, and then masks nothing short of certainty; `--doctor` warns about any value above 0.5 and says this. `PRIVACY_RANK` accepts 0 (off) to 12 000, as `--privacy-rank` and `BIRDNET_PRIVACY_RANK` as well as the config key. It is the *Privacy Rank* field in the Detection section of the Settings page (`/admin/settings`; blank = the default, 10), and it can still be set in the file, the environment or on the command line. Either rule suppresses a window: `PRIVACY_THRESHOLD` (0 = off, typically 0.01–0.03 when used) is a second, independent rule on top of the rank rule.

The commit that added `PRIVACY_RANK` (`6888399`) reports its decision agreeing window for window with BirdNET-Pi's own masks on 13 of the 14 parity clips with the real V2.4 model, including all four speech windows; the fourteenth is the synthetic clip's digital-silence tail, which is already listed below as disagreeing between every pair of model builds. That comparison is `tools/parity/compare_rust.py` (its `privacy` column); it was re-run for this page and gives the same 13 of 14 clips (52 of 54 windows).

## What was measured

Everything below was produced by a command in [`tools/parity`](https://github.com/tomtom215/BirdNet-Behavior/tree/main/tools/parity) and re-run for this page; `tools/parity/README.md` lists the commands. The reference is BirdNET-Pi's own, unmodified code (`88985a3`) running its V2.4 FP16 TFLite model. Two sets of recordings went through it and through this project:

- **The first set, 14 clips** (54 analysis windows): a magpie, a binaural dawn chorus in several formats and lengths, a tawny owl, a crow, a spoken-word recording, synthetic silence and noise.
- **The second set, 106 clips** (900 windows, 30 s each except 20 five-second clips): 49 geotagged field recordings picked from the archive.org *radio aporee* collection by title keyword only (dawn chorus, rain, wind, night, frogs and insects, crowds and speech, traffic, water, waterbirds, quiet), 29 more picked the same way by bird name, 20 sound-effect classes from ESC-50 (birds, other animals, weather, machines, human sounds), 4 LibriVox speech excerpts and 4 generated clips (pink noise, a 1 kHz tone, a clipped square wave, digital silence). 40 of the 106 are 44.1 kHz and had to be resampled. BirdNET-Pi stores 79 rows on them, on 18 clips.

Nothing in either set was chosen for what BirdNET hears in it; the selection is in `search_extended.py`. The audio is not in the repository; `fetch_fixtures.py` rebuilds every clip and checks its sha256.

### What a station stores: the real service, end to end

A fresh-install configuration (the file `installer/lib/62-config-file.sh` writes, with the station's coordinates, and the two model files `--install-model` fetched and verified) in an empty directory, the real binary started the way the systemd unit starts it, each clip dropped into the watch directory under a `YYYY-MM-DD-birdnet-HH:MM:SS.wav` name, and the rows read back from SQLite:

| | BirdNET-Pi stores | This project stores | Same row (species and window) |
|---|---|---|---|
| First set (14 clips) | 12 | 12 | **12** |
| Second set (106 clips) | 79 | 80 | **79** |

The one extra row is a robin in a 44.1 kHz recording: BirdNET-Pi's score for that window is 0.6931 (below 0.7), this project's 0.7025. Running BirdNET-Pi's FP16 model on this project's own input gives 0.6926 and this project's FP32 model on BirdNET-Pi's input gives 0.7024: it is the FP16 weights, not the pipeline, and a window that close to the bar can go either way. On the 91 rows both stored, the confidences differ by at most 0.0067 (the 22.05 kHz clip; 0.0040 otherwise), mean 0.0007. The species name, the common name, the date and time (file time plus the window's start) and the clip's file name come out the same. With `OVERLAP=1.0` and `2.0` the magpie recording stores 5 and 11 rows, the same windows as BirdNET-Pi (confidences within 0.0007); overlapping windows are not merged here, as they are not there.

Before the resampler fix described below, the first set stored 11 of those 12: a borderline great tit in the 44.1 kHz clip (BirdNET-Pi 0.7113, here 0.6564) was missing.

**What is stored differently, whatever the audio.** The `Confidence` column holds the score as computed (`0.9122954607`); BirdNET-Pi rounds to four places (`0.912`, in its own test). The `Week` column holds this project's 48-week-year week (7 where BirdNET-Pi writes the ISO week 8, 18 where it writes 20, 12 where it writes 13), because that is the week the range filter is asked about; it is not an ISO week, so a query that groups imported and native rows by week mixes two calendars.

### Stage by stage

| Stage | Result |
|---|---|
| Decoding and mixing to mono, 48 kHz recordings | **Bit-identical** to BirdNET-Pi's samples: first three windows of each of 76 clips (10 + 66; 15 mono and 183 stereo windows in the second set), largest difference 0.0 |
| Windows, default `OVERLAP=0` | Same count and same starts on all 13 clips of the first set that BirdNET-Pi cuts and all 106 of the second; against BirdNET-Pi's own `splitSignal`, same count for every recording length from 1.5 s to 40 s |
| Windows, other overlaps | 9 097 of 9 145 (overlap, length) combinations give the same count as `splitSignal`; 48 give one window fewer here (see below) |
| Resampled recordings (44.1 kHz) | Input to the model within 0.16 % RMS of BirdNET-Pi's (median over 100 windows from 40 clips), at most 2.8 % (a recording of crickets, nearly all of its energy near the filters' edge); before the fix it was 28 % on the first set's clip. See below |
| Raw scores, all labels both put in the top 10 | 8 584 pairs in ordinary windows: gap in raw score (logit) median **0.009**, 90th percentile 0.040, 99th percentile 0.37 (clip-bootstrap 95 % interval 0.14 to 0.67), largest 1.34; in the 20 zero-padded windows (156 pairs) median 0.09, largest 0.77 |
| Top label of a window | The same in 874 of 900 windows (97.1 %, clip-bootstrap 95 % interval 94.4 to 98.9). The 26 others are windows where nothing scores above 0.29 on either side and two labels are close |
| The decision that stores a row (`CONFIDENCE` 0.7), before the occurrence filter | The same set of labels at or above 0.7 in 899 of 900 windows. Of the 115 windows where either side has a label at 0.7 or more, **114 agree (99.1 %; Wilson 95 % interval 95.2 to 99.8)**; 114 (window, label) pairs reach 0.7 on at least one side in ordinary windows and 1 of them does so on one side only (the robin above). At 0.5: 187 of 189 windows; at 0.3: 305 of 314 |
| Human-sound mask (`PRIVACY_RANK` 10 against BirdNET-Pi's `PRIVACY_THRESHOLD` 0) | The same windows masked in 886 of 900 (98.4 %). BirdNET-Pi masks 292 of the 900 (32 %): this rule is far from rare in noisy recordings. The 14 differences: 10 are one digital-silence clip (below), 4 are windows where the best human class sits at rank 8 against 10, 34 against 15, 10 against 4, 70 against 44 |
| Species-occurrence filter, the model | This project's ONNX conversion of the v2 model returns exactly BirdNET-Pi's own TFLite v2 list for all 265 (place, week 1 to 53) questions, and for eight stations on five continents on every day of 2024, 2026 and 2028 (8 776 station-days; sets of label indices compared) |
| Species lists | 21 of 27 scenarios run through BirdNET-Pi's `run_analysis` keep the same species; the 6 that differ are listed below |

### Every remaining difference, with its cause

The decomposition below runs BirdNET-Pi's FP16 TFLite model and this project's FP32 ONNX model on the same chunk, then on each side's own chunk (`tools/parity` scratch script; the numbers are logits).

- **Digital silence and zero-padded tails: inherent to the two builds of the model.** On a recording's zero-padded last window the two get identical input (difference 0.0) and still differ by 0.79 and 0.62 in raw score (the 7.5 s and 7.6 s clips); on the all-zero window of the synthetic clip by 0.37, and the human class ranks 3 here and 591 there. The V2.4 graph rescales every window by its own range, so a constant window becomes rounding noise and the two builds' rounding differs. Real recordings contain neither, but a microphone that drops out does: **a digital-silence window is masked here (its human class ranks 3rd) and so are its neighbours**, and BirdNET-Pi's is not (rank 591), so a bird next to a dropout can be lost here and kept there. 10 windows in the second set, one clip.
- **Atypical non-bird sounds: inherent.** The largest gaps in ordinary windows (1.34 on a cough, 1.16 on every window of a pure 1 kHz tone, label *Siren*) are the same on identical input: the two builds differ by that much on content a bird model never sees. None is near a threshold.
- **Weights: inherent.** On identical input the FP32 and FP16 builds differ by a median 0.009 and, at the 99th percentile, 0.37 in raw score. That is what moves the robin from 0.6931 to 0.7025.
- **A quiet, band-limited recording that had to be resampled: inherent sensitivity, triggered by the resampler.** The 22.05 kHz version of the first set's dawn-chorus clip has one window (3 to 6 s) whose raw score for one label differs by 1.55. Its resampled input differs from BirdNET-Pi's by 0.96 % RMS (at most 1.8 least-significant bits of 16); on identical input the two models differ by 0.006, and feeding either model the other resampler's input moves it by 1.54. BirdNET-Pi's own model moves that much for an input change that small on a very quiet window. No stored row changes.
- **The resampler (fixed).** The earlier version of this page called the 28 % input difference on the 44.1 kHz clip "the model's sensitivity to a nearly silent input, not a fault in either resampler". It was a fault in this project's: the resampling library delays its output by `128 x ratio - 1` samples, the code removed only the whole-sample part, and every recording whose rate is not 48 kHz arrived early by 0.69 sample at 44.1 kHz (0.38 at 22.05 kHz, 0.67 at 48 to 32 kHz, 1.0 at 32 to 48 kHz). Shifting this project's chunk by the measured 0.689 sample moved the raw score from 0.863 back to 1.201 (BirdNET-Pi: 1.202). The delay is now made a whole number and removed; the remaining 0.005 to 0.017 sample is the library's own interpolation grid. BirdNET-Pi's recorders write 48 kHz, so most stations never resample; RTSP streams, uploaded files and 32 kHz (V3.0) analysis do.
- **Fractional `OVERLAP` (not fixed, tiny).** BirdNET-Pi computes the step as `int((3 - overlap) x 48000)` in 64-bit floats; this project as `3 x 48000 - int(overlap x 48000)` in 32-bit. For 14 of 59 overlaps in steps of 0.05 the step differs by one or two samples (0.7: 110 399 against 110 400), which cannot matter, except that when the last window would hold exactly the 1.5 s minimum the extra sample decides whether it exists: 48 of 9 145 combinations, never at `OVERLAP=0`, and for a 15 s segment only at 2.10.

## The default V3.0 path, against an independent reference

There is no BirdNET-Pi to compare V3.0 with, so `tools/parity/v3_reference.py` is a reference that shares no code with the pipeline: soundfile, a plain-mean mono mix, **soxr** (not rubato) for the resample to 32 kHz, slicing into 4.5 s windows, onnxruntime. It was run on all 120 recordings of both sets and compared with `tests/parity_dump.rs` on the V3.0 model: 119 recordings give a comparison (the digital-silence clip makes the model output NaN in the reference; this project treats a non-finite score as 0), 655 windows, the same window count on every recording. **The top species is the same in 650 of 655 windows**; the five that are not are every window of the pure 1 kHz tone, where two species tie at 0.026. On the 650 windows with the same top species the probabilities differ by a median 0.00003, a 99th percentile 0.0003 and at most 0.0040; the 200 of them that score 0.7 or more differ by at most 0.0040. This checks the front end (decode, resample, window) and the confidence mapping of the default model; it does not say V3.0 identifies birds better or worse than V2.4.

### What the human-sound rule does on V3.0

The rule is BirdNET-Pi's, written for V2.4, where a window of speech puts a human class among the model's ten best. On the same 91 clips the rule masks **246 of 665 windows (37.0 %) on V2.4 and 18 of 459 (3.9 %) on V3.0**. On the five LibriVox speech clips it masks 26 of 44 windows on V2.4 and **0 of 31 on V3.0**, on three ESC-50 laughing, coughing and crying clips 6 of 6 and 0 of 3, on five crowd recordings 41 of 50 and 7 of 35. On one of those speech clips V3.0 gives its human classes a probability of 0.002 or less in all seven windows (rank 12 to 55 of 11 560 classes), so a confidence threshold would not catch it either (`PRIVACY_THRESHOLD` 0.01 to 0.03 is below that). **On a V3.0 station the default does not keep conversation out of the table, and a station that opted into V3.0 should not rely on it for privacy.** Nothing in this project measured a setting that does.

## What is not the same, whatever you do

With the defaults above, a default station still differs from BirdNET-Pi's in the ways below. Each was measured this session; `tools/parity/defaults_audit.py` prints the full table of defaults with the file and line of every cell (it refuses to run if a cited line no longer holds the cited text).

| | BirdNET-Pi | birdnet-go (from its source, not run) | This project |
|---|---|---|---|
| **Default model** | V2.4 | V2.4 | V2.4 for a new install; BirdNET+ V3.0 preview is a choice (`--classifier birdnet-v3`) and an older station keeps the model it has |
| **`SENSITIVITY`** | sigmoid slope `clamp(2 - S, 0.5, 1.5)`; default 1.25 gives 0.75 | the slope itself; default 1.0 | BirdNET-Pi's, on V2.4; ignored by V3.0 |
| **Confidence threshold** | 0.7, `>=` | 0.8 in code (0.7 in its shipped config file), strictly `>` | 0.70, `>=` |
| **Overlap** | 0.0 | 0.0 in code, 1.5 in its shipped config file | 0.0 |
| **Range filter** | MData v1 (v2 optional) | its own latest model, threshold 0.01 | v2 with V2.4 (v1 is not hosted as ONNX); V3.0 has its own geomodel; threshold 0.03 |
| **Week asked of the range filter** | ISO week 1 to 53, raw | 48-week year (four per month) | 48-week year |
| **No location set** | passes 0.0, 0.0 (or -1, -1) to the model | filter skipped | filter off; every species passes. An explicit 0.0, 0.0 is used as given: the magpie recording stores nothing, as in BirdNET-Pi |
| **Human sounds** | any human class in the top 10 masks the window and its neighbours | the human detection alone is dropped; filter on | BirdNET-Pi's rule, on by default (`PRIVACY_RANK` 10) |
| **Rows** | every window that clears the bar | merged over 15 s, adaptive threshold on by default | every window that clears the bar; the duplicate, corroboration and adaptive-threshold features are off by default |
| **Stored confidence / week** | 4 decimals / ISO week | 2 decimals / none | full precision / 48-week-year week |
| **Time of a row** | the file name's time plus the window's start | the wall clock when the result is made | as BirdNET-Pi |

birdnet-go was not run. Applying its rules (score `sigmoid(logit)`, strictly above 0.8 or 0.7, one row per species per 15 s, human classes dropped, no range filter, no adaptive threshold) to this project's model outputs on the 77 recordings of the second set that both sides cut would store 18 rows (threshold 0.8) or 24 rows (0.7) where BirdNET-Pi stores 27 (`tools/parity/birdnet_go_estimate.py`). It is an estimate of what the rules do to the same scores, not a measurement of birdnet-go, and the two are not interchangeable in either direction.

**The range filter: lists differ on almost every day.** Three things differ at once between BirdNET-Pi as installed and a new install here: the model (MData v1, v2), the week it is asked about (ISO, 48-week), and nothing else. Measured on eight stations (London, Ithaca, Helsinki, Sydney, São Paulo, Nairobi, Hokkaido, Austin) for every day of 2024, 2026 and 2028 with BirdNET-Pi's own range models (`tools/parity/golden/range_calendar.json`, SF_THRESH 0.03):

- *The calendar alone* (BirdNET-Pi with v2 and its ISO week against this project): the lists differ on 219 to 341 days of the year depending on the station and the year (2026 has an ISO week 53), by a mean of 5 species (Sydney) to 33 (Ithaca), at most 185 species on one day. This is not only a December effect: an ISO week number between 1 and 48 is read by the model as the same-numbered week of a 48-week year, which is up to four calendar weeks away. Weeks 49 to 53 give the model no season, and it answers with one year-round list about twice as long as any real week's.
- *The model alone* (v1 against v2, same ISO week): different on every day, by a mean of 17 species (Helsinki) to 141 (São Paulo), because v2 admits many more species in the tropics.
- *Both together*, BirdNET-Pi as installed against a new install here, symmetric difference a mean of 29 (Helsinki) to 142 (São Paulo) species; admitted by BirdNET-Pi only, a mean of 1.5 (Sydney) to 35 (Hokkaido) species; by this project only, 10 (Helsinki) to 132 (São Paulo). The ranges cover all three years.

What that does to *stored rows* was not separable in the recordings tested: BirdNET-Pi's rows are the same under v1/ISO, v2/ISO and v2/48-week on all 120 clips (79 rows on the second set, 12 on the first), because the birds in those recordings are common ones all three lists admit. The 120 clips do not contain a species that falls between the lists, so the effect on a station's rows for a rare or out-of-season bird is unmeasured; the Species, *Why isn't a bird heard here?* page shows it for any bird.

**No location set.** BirdNET-Pi's installer asks `ip-api.com` for a location and falls back to 0.0, 0.0 when that fails (`tools/parity/README.md`). At 0.0, 0.0 with the v1 filter and `SF_THRESH` 0.03 the filter admitted 468 species at ISO week 8, and BirdNET-Pi's own Pica pica test recording produced no detections (`tools/parity/golden/matrix/a0_shipped_literal.json`: `detections: []`, `mdata.count: 468`). Here a station with no coordinates stores all three magpie rows and a station with 0.0, 0.0 typed in stores none (run for this page on the real binary). No installer, quick-start script or default here writes 0.0, 0.0.

**Species lists** (27 scenarios through BirdNET-Pi's own `run_analysis`, same files read by the importer): include and exclude by scientific name, the `Scientific_Common` lines the species picker writes, CRLF and padded lines, a whitelist that rescues a species the occurrence list rejects, the whitelist that loses to an exclude or an include list, an exclude that beats an include, and `SF_THRESH` 0 and 0.5 all agree (21 scenarios). They differ where BirdNET-Pi's include list is a wall that a bad entry turns into "record nothing": an include list holding only a typo, only a common name, only a lower-case name, or only a blank line makes BirdNET-Pi record nothing at all, and makes this station ignore the list and record everything (4 scenarios); and `SF_THRESH` 1.0, where the model admits no species at all, which BirdNET-Pi reads as "filter off" and this project as an empty allow-list (2 scenarios).

**File names.** Of 33 names put to BirdNET-Pi's `ParseFileName` and to this project's parser, the date, time and RTSP id agree on every name both read. BirdNET-Pi also reads `2026-2-16-birdnet-…`, a time without a leading zero, no or differently-cased `birdnet`, and no extension (this project refuses them). And this project reads six names that BirdNET-Pi rejects with an error and skips: a date or time that is not real (`2026-02-30`, `2025-02-29`, month 13, `25:00:00`, minute 60, second 60). It then stores their rows under the name's date and time as written (`2026-02-30` as 2026-03-02, `25:00:00` as the time `25:00:00`); only a hand-named file can have such a name. At the year boundary BirdNET-Pi calls 31 December 2026 and 1 January 2027 both ISO week 53 and this project calls them weeks 48 and 1 of its 48-week year.

**Other things that can keep a row out of the table here:** a recording dated before 2024 or after 2063 is quarantined rather than stored; when more than 40 segments are waiting, the CPU passes 80 °C, or a Pi reports throttling, every other segment is skipped and counted; and a file that is not named `YYYY-MM-DD-birdnet[-ID]-HH:MM:SS.wav` is refused. BirdNET-Pi does none of these.

## Not measured

BirdNET-Pi's FP16 model on a real Raspberry Pi; birdnet-go itself (only its source and an estimate above); any model other than V2.4 against a reference except V3.0's front end; Perch and a second classifier; the effect on a station's rows of the three range filters on a rare bird; recordings longer than 30 s; any rate other than 44.1 and 48 kHz on real recordings (22.05 kHz only on one clip); the numpy 1.x comparison (`np.float32(0.7) >= 0.7` is false there, true under numpy 2, and no window sat on it).

## Seeing what the range filter does

BirdNET-Pi has no page that says why a bird is missing; this station does. **Species → Why isn't a bird heard here?** (`/species/occurrence`, and `GET /api/v2/species/occurrence` for scripts) asks the same range filter the detection loop uses, for the station's location and any week of the model's 48-week year, and says for each species whether it is admitted and why. It is the quickest way to see the December effect above: ask about the same bird in week 47 and in week 48. See [Why isn't a bird heard here?](../guide/species.md#why-isnt-a-bird-heard-here).

## Checking it yourself

```bash
# the reference side: BirdNET-Pi's code and model, goldens in tools/parity/golden
tools/parity/regenerate.sh
# the first set, Rust side, against the first set's golden
PARITY_MODEL=… PARITY_LABELS=… PARITY_WAVS=a.wav:b.wav PARITY_OUT=out \
  cargo test --all-features --test parity_dump -- --nocapture
python3 tools/parity/compare_rust.py out/ours.json tools/parity/golden/matrix/a1_station.json
# the second set (106 clips): fetch, run BirdNET-Pi on it, run this project on it, compare with intervals
python3 -I tools/parity/fetch_fixtures.py DIR --manifest tools/parity/fixtures_extended.json --strict
python3 -I tools/parity/run_extended.py --pi-root PI --fixtures DIR --out OUT --as-year 2026
python3 tools/parity/compare_ext.py out/ours.json OUT/x0_pi_default.json
# every default, with its file and line, in all three projects
python3 tools/parity/defaults_audit.py --pi PI --go BIRDNET_GO --repo .
```

CI fetches the V2.4 model and range filter, checks their sha256, and runs the V2.4 tests on every change to the inference path: `v24_matches_birdnet_pi`, `v24_range_filter_matches_birdnet_pi`, `v24_range_filter_every_week_matches_birdnet_pi`, `species_lists_match_birdnet_pi`, `chunking_matches_birdnet_pi`, `file_names_match_birdnet_pi` and `resampler_matches_resampy_parity`.
