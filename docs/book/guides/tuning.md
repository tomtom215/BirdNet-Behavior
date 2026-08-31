# Tuning Detection Accuracy

Out of the box BirdNet-Behavior is tuned to be sensible. But every station is different — a noisy roadside, a quiet woodland, a single mimic that fools the model — and you can dial it in. This page explains *what each knob does* and *which way to turn it*.

> **The golden rule:** change **one** thing at a time, then watch the [Quality dashboard](../admin/settings.md#data-quality) and your [Today log](../guide/today.md) for a day before changing the next.

## The two failure modes

| Symptom | What's happening | Turn this way |
|---|---|---|
| **Too many wrong birds** (false positives) | The model is logging low-quality guesses | *Raise* confidence / sensitivity *down* / add a per-species threshold |
| **Missing birds you can hear** (false negatives) | The bar is too high, or the signal is poor | *Lower* confidence / sensitivity *up* / check mic levels first |

If you're getting both at once, it's almost always an **audio** problem (wind, clipping, a bad mic position) — fix that first on the [Audio page](../admin/audio.md) before touching detection settings.

## The knobs, in the order you should reach for them

### 1. Confidence threshold

The minimum score (0–1) a detection must clear to be logged. This is your main dial.

- **Default ~0.7.** Raise toward **0.8–0.85** to aggressively cut false positives (you'll lose some faint genuine calls).
- Lower toward **0.5–0.6** only if you're confident your audio is clean and you want maximum recall.
- Set it at `/admin/settings → Detection`.

### 2. Per-species thresholds

One noisy species shouldn't force you to raise the global bar for everyone. Override the threshold for individual species instead:

- A local mockingbird or starling logging junk? Give just that species a high threshold (e.g. 0.9).
- A rare bird you don't want to miss? You can lower its threshold — but pair that with the **quarantine** queue (below) so each one still gets a human glance.

The [Quality dashboard](../admin/settings.md#data-quality) lists your lowest-confidence species — those are the best candidates for a per-species override.

### 3. Sensitivity

The BirdNET sensitivity parameter (0.5–1.5; also `SENSITIVITY` in `birdnet.conf`). It reshapes the model's confidence curve:

- **Higher (→1.5)** makes the model more eager — more detections, more borderline calls.
- **Lower (→0.5)** makes it more conservative.
- Most stations leave this at the default and tune the confidence threshold instead. Reach for sensitivity only when the threshold alone can't find a good balance.

### 4. Species-frequency (SF) filter

Uses your **location** and the **week of the year** to drop birds that shouldn't plausibly be present (no penguins in your back garden in July). The `SF_THRESH` (default `0.03`) sets how aggressive that prior is.

> **This filter is off unless you install a metadata model.** It needs three things, and the station ships only the first: your coordinates, a metadata ("geo") ONNX model, and that model's own label file. The model download fetches the classifier and its labels only — nothing fetches a metadata model, and `SF_THRESH` does nothing on its own. Run `birdnet-behavior --doctor` and read the **Species occurrence filter** line; it names whichever of the three is missing.

To turn it on:

| Setting | `birdnet.conf` | Environment | Flag |
|---|---|---|---|
| Metadata model | `METADATA_MODEL_PATH` | `BIRDNET_METADATA_MODEL` | `--metadata-model` |
| Its label file | `METADATA_LABELS_PATH` | `BIRDNET_METADATA_LABELS` | `--metadata-labels` |

The model takes `(latitude, longitude, week)` and returns one occurrence probability per species. **It does not score the same species list as the classifier** — BirdNET Geomodel v3.0 covers 12 012 species where the V3.0 Global 11K classifier this station ships emits 11 560 — so the label file is what maps one list onto the other. Supply it and the two are matched by scientific name.

Omit the label file only for a metadata model indexed identically to the classifier (a matched BirdNET pair, e.g. a V2.4 `MData` model beside V2.4 labels). The station checks that at startup and **refuses a mismatched model** rather than reading one list's index into the other, which would report birds under other birds' names with full confidence.

Once it is running:

- Make sure your latitude/longitude are correct first — the filter is only as good as your coordinates.
- Raise `SF_THRESH` to be stricter about out-of-range species; lower it (or 0) to let everything through (useful if you *want* to catch vagrants).

### 5. Quality pre-filter

Optionally drops audio segments dominated by **rain, wind, or broadband noise** *before* they ever reach the model — cheaper and more reliable than cleaning up the resulting false positives. Enable it under Detection when your station is exposed to weather.

### 6. Rare-bird quarantine

Rather than choosing between "log everything" and "miss the rarities," send borderline rare birds to the [quarantine queue](../guide/reviews.md#reviews-vs-quarantine) for a quick human approve/reject. This lets you run a *lower* threshold for rare species without polluting your life list.

## A recommended starting recipe

1. Set your **location** accurately.
2. Get mic **levels** right (peaks near −6 dB).
3. Run the defaults for a few days and read the [Quality dashboard](../admin/settings.md#data-quality).
4. If false positives dominate: enable the **quality pre-filter**, then nudge the **confidence threshold** up by 0.05.
5. Add **per-species overrides** for the one or two species generating the most noise.
6. Turn on **quarantine** for rare birds so you can keep recall high without losing trust in the log.

> Everything here is reversible and stored in the database, so experiment freely — and remember to change just one knob at a time.
