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

It needs three things: your coordinates, the BirdNET geomodel, and that model's own label file. The installer and the Docker entrypoint fetch the last two (~14 MB) and wire them up, so on a current install the only thing you supply is the location.

**Check it is actually on** &mdash; `SF_THRESH` does nothing by itself:

```console
$ birdnet-behavior --doctor | grep occurrence
[ PASS ] Species occurrence filter - active - .../BirdNET+_Geomodel_V3.0.2_Global_12K_FP32.onnx ...
```

A `WARN` there names whichever of the three is missing. The geomodel download is deliberately non-fatal: a station without it still detects, it just stops filtering by location, and re-running `install.sh repair` picks it up later.

| Setting | `birdnet.conf` | Environment | Flag |
|---|---|---|---|
| Geomodel | `METADATA_MODEL_PATH` | `BIRDNET_METADATA_MODEL` | `--metadata-model` |
| Its label file | `METADATA_LABELS_PATH` | `BIRDNET_METADATA_LABELS` | `--metadata-labels` |

The model takes `(latitude, longitude, week)` and returns one occurrence probability per species. **It does not score the same species list as the classifier** &mdash; the geomodel covers 12 012 species across birds, mammals, insects, amphibians and reptiles, where the V3.0 Global 11K classifier emits 11 560 &mdash; so the label file is what maps one list onto the other, matched by scientific name.

Omit the label file only for a metadata model indexed identically to the classifier (a matched BirdNET pair, e.g. a V2.4 `MData` model beside V2.4 labels). The station checks that at startup and **refuses a mismatched model** rather than reading one list's index into the other, which would report birds under other birds' names with full confidence.

Once it is running:

- Make sure your latitude/longitude are correct first — the filter is only as good as your coordinates.
- Raise `SF_THRESH` to be stricter about out-of-range species; lower it (or 0) to let everything through (useful if you *want* to catch vagrants).

### 5. Quality pre-filter

Optionally drops audio segments dominated by **rain, wind, or broadband noise** *before* they ever reach the model — cheaper and more reliable than cleaning up the resulting false positives. Enable it under Detection when your station is exposed to weather.

### 6. Rare-bird quarantine

Rather than choosing between "log everything" and "miss the rarities," send borderline rare birds to the [quarantine queue](../guide/reviews.md#reviews-vs-quarantine) for a quick human approve/reject. This lets you run a *lower* threshold for rare species without polluting your life list.

### 7. Barking dogs and other non-birds

BirdNET's label set is not birds only — it carries `Dog`, `Siren`, `Engine`,
`Fireworks`, `Power tools`, `Gun`, `Environmental` and `Noise`, because the
training data contains them. A dog barking near the microphone is broadband
enough that the classifier does not answer `Dog` and stop: it also produces
confident-looking scores for whatever species the bark most resembles. Because
the barking is regular — same dog, same garden, every evening — the phantom
accumulates until it looks like a resident.

```text
BIRDNET_NOISE_THRESHOLD=0.6      # 0 = off; typical 0.5–0.8
BIRDNET_NOISE_CLASSES=Dog        # unset = Dog; empty = watch nothing
```

When a watched class scores at or above the threshold, every detection in that
three-second chunk is discarded. Only that chunk — a bark is a few hundred
milliseconds, and the chunks overlap, so a bark on a boundary is caught in both.
Beside a road or a fire station, add `Siren` and `Engine`. Do **not** add
`Noise` or `Environmental`: they score highly on ordinary quiet recordings and
will suppress most of the night.

### 8. One song, one detection

A 15-second recording is five 3-second chunks, so a bird singing throughout is
recorded five times. Every count in the application is a row count — daily
totals, the activity heat map, the dawn-chorus curve — so a species that sings
in long phrases outscores one that calls in short bursts for no reason but
phrasing.

```text
BIRDNET_DUPLICATE_INTERVAL_SECS=30   # 0 = off
```

Each species is then admitted at most once per interval; the first chunk wins,
because that is when the bird started singing. Off by default, since turning it
on changes how many rows your station records and puts a visible step in every
chart on the day you do it.

### 9. Day birds at night

A blue tit "detected" at 02:30 is almost always the model hearing something
else. A blanket night filter would be worse than the problem — owls, nightjars,
rails and bitterns call at night on purpose and are the detections most worth
having — so the filter asks *who*, not just *when*.

```text
BIRDNET_NIGHT_FILTER=1
BIRDNET_NIGHT_MARGIN_MINS=60
BIRDNET_NIGHT_EXTRA_NOCTURNAL=Catharus,Vireo
```

Species in a genus known to call at night are exempt; everything else detected
between sunset + margin and sunrise − margin is sent to
[quarantine](../guide/reviews.md#reviews-vs-quarantine), never dropped, because
the taxonomy is genus-level and cannot be complete. It needs your station
coordinates, and it fails open: no coordinates, an unreadable timestamp or a
polar summer all mean "keep everything".

**If you record nocturnal flight calls, leave this off** — migrating thrushes
and warblers calling overhead are exactly what it would quarantine — or name
those genera in `BIRDNET_NIGHT_EXTRA_NOCTURNAL`.

## Letting the station tune itself

Two places on the web UI turn your own review history into advice. Both only
ever suggest; nothing changes until you press the button.

**Suggested thresholds** (Species page). For each species you have both
confirmed and rejected detections of, the station works out the threshold that
best separates the two, and shows what it would have cost (confirmations lost)
and caught (rejections stopped) against the reviews it came from. Suggestions
that separate nothing are not shown — if your reviews and the model's
confidence disagree at random, there is no threshold worth applying.

The more detections you review, the better this gets. Reviewing a mix of good
and bad detections for one species is worth far more than reviewing many of
either alone: with only confirmations the best answer is "admit everything",
which is not a threshold.

**Species that may not be birds** (Station → Data). Flags species by the
*shape* of their detections rather than by name: every review rejected, never
detected confidently, confidence that never varies, many detections on very few
days. Two of those signals have to agree before a species is listed, and it
needs at least 10 detections first, so a genuine scarce visitor is not flagged
on the day it arrives. The Exclude button adds it to the ordinary species
exclusion list, where you can undo it.

## A recommended starting recipe

1. Set your **location** accurately.
2. Get mic **levels** right (peaks near −6 dB).
3. Run the defaults for a few days and read the [Quality dashboard](../admin/settings.md#data-quality).
4. If false positives dominate: enable the **quality pre-filter**, then nudge the **confidence threshold** up by 0.05.
5. Add **per-species overrides** for the one or two species generating the most noise.
6. Turn on **quarantine** for rare birds so you can keep recall high without losing trust in the log.
7. Review a mixture of good and bad detections for a week, then check the **suggested thresholds** on the Species page and the **suspect species** list under Station → Data — by then both have something to say.

> Everything here is reversible and stored in the database, so experiment freely — and remember to change just one knob at a time.
