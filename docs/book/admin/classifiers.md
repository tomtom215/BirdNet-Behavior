# Running More Than One Classifier

Your station always runs one classifier — BirdNET, the thing that makes it a BirdNET station. This page is about running a **second** one beside it, and what the station does with two opinions about the same three seconds of audio.

You do not need this. One classifier is the supported, tested, default configuration, and most stations should stay there. Read on if you are in the tropics, where Google Perch v2 is materially better than BirdNET, or if you want a second opinion recorded on every detection.

## What a second classifier buys, and what it costs

Every classifier routed to a recording runs on **every chunk of it**. Two classifiers mean two inferences per chunk, which on a Raspberry Pi is the expensive part of the whole pipeline. In exchange:

- **A species only one of the two models knows is now heard.** The results are a union, not an intersection.
- **Each detection records which classifier produced it**, and how many classifiers reported that species in that chunk.
- **Nothing is lost.** A model that reports a species alone still produces a detection.

The cost is real and worth measuring before you commit to it: check the daemon's CPU use for a day with one classifier, then compare.

## Installing one

The station carries a small catalogue of classifiers it knows how to fetch and verify. To see it:

```bash
birdnet-behavior --install-model list
```

To install one:

```bash
birdnet-behavior --install-model perch-v2
```

This runs in the foreground and exits — deliberately. It is a several-hundred-megabyte download that takes hours on the uplink a field station has, and you should be able to watch it, interrupt it, and see it fail. There is no button in the web interface for the same reason: a multi-hour download inside a request handler has nowhere to report progress, and a retry would start a second download beside the first.

Three things it does before and while it downloads:

- **Checks free space first**, with a 512 MiB margin on top of the file. Filling the card stops the station recording, which is worse than not having the classifier. A filesystem that will not say how much space it has counts as no space rather than plenty.
- **Verifies as the bytes arrive.** The sha256 of each file is compiled into the binary — not fetched alongside it — so the checksum is a promise this project makes rather than one the download site makes. The digest is computed while streaming, so a 541 MB model never has to fit in a 1 GB board's memory at once.
- **Never leaves a half-file the station would load.** Unverified bytes carry a `.part` name; the file is renamed into place only after its digest matches.

Files land in `MODEL_DIR` (`BIRDNET_MODEL_DIR`, default `/data/model`). The same catalogue is readable over the API at `GET /api/v2/models/catalog`.

> Perch v2 ships **without a labels file**. You must supply one with 14 795 rows. Labels are positional against a model's output, so the wrong file reports species under other species' names at full confidence — a failure with no symptom at all.

## Configuring it

Up to **three** classifiers total. The primary is always loaded and is whatever `BIRDNET_MODEL` / `BIRDNET_LABELS` already point at.

```ini
# Name the primary, if you want it called something other than `birdnet`
# in routes and on detections.
MODEL_ID=birdnet
MODEL_THRESHOLD=
MODEL_SAMPLE_RATE=

# The second classifier. Each needs its OWN labels file.
MODEL_2_PATH=/data/model/perch-v2.onnx
MODEL_2_LABELS=/data/model/perch_labels.csv
MODEL_2_ID=perch
MODEL_2_THRESHOLD=0.35
MODEL_2_SAMPLE_RATE=32000

# A third, if you have one.
MODEL_3_PATH=
MODEL_3_LABELS=
MODEL_3_ID=
MODEL_3_THRESHOLD=
MODEL_3_SAMPLE_RATE=
```

Every key also exists as `BIRDNET_MODEL_2_PATH` and so on in the environment; see `.env.example`.

**`MODEL_n_SAMPLE_RATE` is not optional for Perch.** Its tensor declares `[-1, 160000]`, which is 5 s at 32 kHz and equally 3⅓ s at 48 kHz — nothing in the shape distinguishes them, so it has to be told. BirdNET's own shapes are unambiguous, and the station derives those for itself.

**`MODEL_n_THRESHOLD`** lets a second classifier be held to its own confidence bar. Leave it unset to use the station's.

## Routing sources to classifiers

By default every audio source is judged by every loaded classifier. To send particular microphones to particular classifiers:

```ini
MODEL_ROUTES=pond:perch,garden:birdnet+perch
```

The source is its `audio_sources` row id — the same label `GET /api/v2/system/capture` lists. Use `+` for more than one classifier on one source, and commas between sources.

Two rules here exist because a station runs unattended for months:

- **A source you do not name gets the primary classifier**, never nothing. Silence must not be reachable by forgetting to write a line.
- **A route naming a classifier you have not configured stops the station at startup**, where the journal and `--doctor` will show it. Accepting `pond:pecrh` and discovering at runtime that the pond routes to nothing would cost months of recordings from that microphone, and the station would look identical to one having a quiet season.

## What the station does with two opinions

Each classifier is judged against its own threshold, and the results are combined per chunk:

- **Union by species.** Anything any classifier reported above its threshold becomes a detection.
- **The highest confidence wins**, and the detection records **which** classifier gave it (`model_id`).
- **`model_agreement` counts how many distinct classifiers reported that species in that chunk.**

**Agreement is never folded into the confidence.** An averaged or bonused number would have no calibration behind it while sitting in the same column as a real one, quietly changing the meaning of every threshold you have set and every historical comparison you make. The confidence you see is the winning classifier's own output, unchanged.

`model_agreement` is `NULL` on rows written before this feature existed — a third state, and not the same as `1` ("one classifier asked, one answered").

> Both columns are stored on every detection today, and nothing in the web interface or the API reads them back yet. They are there so that a station running two classifiers now is not analysing a season of data later with the corroboration missing.

## Windows: what "the same chunk" means

Classifiers do not agree on how much audio a judgement covers. BirdNET+ V3.0 wants 4.5 s; Perch v2 wants 5.0 s.

The station cuts each chunk to the **longest** window any loaded classifier wants, and advances by the **shortest**. Each classifier reads its own window from the start of the shared chunk.

The reason it is not simply "cut and step by the longest" is a gap you would never see. Cut at 5.0 s and step 5.0 s, and BirdNET hears the first 4.5 s of every chunk and never the last half-second — audio no classifier is ever given, recurring on every chunk for as long as the station runs, producing no error and no missing-data indicator. The detections that would have been there simply are not there.

Stepping by the shortest window removes it. The shortest-window classifier gets exactly the chunks it would have had running alone, and longer-window classifiers get overlapping chunks instead of gaps.

What this costs, so you can decide: a classifier whose window is longer than the shortest runs `longest ÷ shortest` more inferences — 5.0 ÷ 4.5 ≈ **1.11**, about 11 % more for Perch beside BirdNET. The daemon logs it at startup as `extra_inference_ratio` alongside both window lengths, so you do not have to work it out:

```text
classifiers want different windows: chunking to the longest and stepping by the
shortest ... longest_window_secs=5.0 shortest_window_secs=4.5
extra_inference_ratio=1.1111112
```

One consequence worth knowing when you read detection times: a detection carries the **chunk's** span, not its own classifier's. A shorter-window classifier's detection can therefore show an end time up to half a second later than the audio it actually read. The recorded span always contains that audio, so a clip cut from it contains the detection. With one classifier the two are identical.

## What the station refuses, and why

**A different sample rate.** Every classifier must want audio at the same rate. A recording is decoded and resampled once; resampling it twice would double the most expensive step in the pipeline on the boards this runs on, and there is no rate that is correct for both. The station says so at startup and names both rates rather than starting and feeding one of them audio prepared for the other. Differing *window lengths* are fine — that is what the chunking above is for.

**More classifiers than the machine can hold.** All classifiers together may use at most **half** the machine's effective memory ceiling — the cgroup limit if there is one, otherwise physical memory — counting each model's file size plus a 256 MiB working-set allowance. On a 1 GB board that budget is 512 MiB, which one BirdNET model already fills, so a second is skipped and the arithmetic goes in the journal. A machine that will not report its memory gets one classifier, because "unknown" must not be read as "plenty".

**Nothing at all.** A station with no classifier is not a degraded station; it is a station that has stopped being what it is for. Startup refuses it.

## Not yet supported

**Bat classification.** `BattyBirdNET` is not a peer classifier — it is a chained second stage that consumes BirdNET **v2.4** embeddings and needs 256 kHz audio fed through *without* resampling. That needs a capture path, a deliberate no-resample route, a real v2.4 model, and the ultrasonic validation filter, none of which ship yet.

## See also

- [Settings & Detection](./settings.md) — thresholds, sensitivity and the rest of the detection knobs
- [Tuning Detection Accuracy](../guides/tuning.md) — what to change before reaching for a second model
- [Audio & Microphones](./audio.md) — the sources `MODEL_ROUTES` names
