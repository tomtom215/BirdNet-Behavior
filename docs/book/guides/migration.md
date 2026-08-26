# Migrating from BirdNET-Pi

BirdNet-Behavior imports an existing BirdNET-Pi history safely and **non-destructively** — the source database is opened read-only and never modified. It can import *your own* station's history, which is the common case and needs almost no thought, or *another* station's, which needs two answers from you before it will be right.

## The common case: your own station's history

1. Stop BirdNET-Pi so it isn't writing while you import:
   ```bash
   sudo systemctl stop birdnet_*
   ```
2. Open **Settings → Data → Import**, or go straight to `/station/data#import`.
3. Give it the file — either **Upload** it, or switch to **Server Path** and type the absolute path to `~/BirdNET-Pi/BirdDB.txt` (or `birds.db`).
4. **Review the preview** — schema, total detections, unique species, date range, duplicate count, and a checklist with ✔ / ⚠ / ✘ against each validation.
5. Click **Start Import**.
6. Verify the per-species count comparison when it finishes.

Leave the two **"Where did this recording come from?"** fields blank. Blank means "this is this station, on this clock", which is what makes the import a no-op reconciliation.

Duplicate rows are skipped, so **re-running the import is safe** — if it is interrupted, run it again.

## Importing another station's history

This is a different operation and the form treats it as one. Two facts drive everything below:

- **BirdNET-Pi stores local wall-clock time with no time zone.** A history recorded at UTC−5 and read by a UTC+1 station is six hours out, and nothing in the file says so. Every hour-of-day analytic — the dawn chorus, the heat map, sessionisation, peak windows — would average the two clocks together.
- **A different place is a different set of birds.** Sunrise, habitat and species pool all differ, so "first of year" and species richness stop meaning one thing.

### Tell it where the file came from

Fill in both fields before importing:

| Field | What to put |
|---|---|
| **Source station name** | Anything that will mean something to you later — "Hollow Oak, north transect". Stored with the batch and shown beside it. |
| **Source station's UTC offset** | The source station's *standard* offset, in seconds, east-positive. UTC−5 is `-18000`; UTC+1 is `3600`. |

Each timestamp is then converted individually — source local → the real UTC instant → *this* station's local time for that instant — so the destination half of the conversion is exactly right on both sides of every daylight-saving change **this** station observes.

> **What a single offset cannot do.** If the source station observed daylight saving, roughly half its history carries a different real offset than one number can describe, and those rows land an hour out. Recovering that needs the source's IANA time zone, which BirdNET-Pi does not record. The number you enter is stored with the batch, and the whole import can be removed, so this is recoverable — but it is not automatic.

### What the validator will tell you

The preview compares the file's coordinates against this station's and reports:

- a **⚠ warning** when the source is far enough away to be a different site — the batch is then tagged `NNN km away` in the provenance list;
- a **✘ failure**, which refuses the import, when the file *itself* contains detections from several different coordinates. Such a file is already a merge of several sites and cannot be attributed to one place.

The distance check is a warning, not a block: merging two sites is a legitimate thing to want, and only you can say whether two coordinates are one station whose GPS fix moved or two sites a county apart.

### Decide whether imported rows count

Under **Imported histories** there is one checkbox:

> **Keep imported detections out of the analytics**

- **Off (the default)** — imported detections count as this station's own everywhere: life list, first-of-year, species richness, phenology, the heat map, co-occurrence, the dawn chorus.
- **On** — they stay in the database and in the Recordings browser, but stop contributing to any of those.

Turn it **on** when you have imported another site and want your station's own numbers back. It applies to both the detection database and the DuckDB analytics copy, so the two cannot disagree.

### Undoing an import

Every imported detection is tagged with the batch that brought it in. Each batch in the **Imported histories** list has a **Remove** action that deletes exactly those rows — nothing this station heard itself is touched. Merging another site's history is a decision you can take back.

## What carries over, and what does not

**Carries over:** every detection row — date, time, scientific and common name, confidence, coordinates, and the BirdNET parameters (cutoff, week, sensitivity, overlap). BirdNET-Pi config keys (`ALSA_CARD`, `LATITUDE`/`LONGITUDE`, `OVERLAP`, `SF_THRESH`, `BIRDWEATHER_TOKEN`, …) are also understood when reading a `birdnet.conf`; see the mapping table in [Configuration](../getting-started/configuration.md).

**Does not carry over: the audio.** The importer copies rows, not files. Imported detections keep their original `File_Name`, but the WAV is not brought across, so the clip player on an imported detection's page has nothing behind it. Your counts, species lists and every analytic are unaffected — only playback is.

If you want the audio too, copy it yourself before importing, into the station's recordings directory — `~/BirdNet-Behavior/recordings` under the service user on a default `install.sh`, `/data/recordings` in Docker — keeping the original filenames.

## After the import

The import writes back-dated history straight to SQLite, so the DuckDB analytics copy is **rebuilt** afterwards — otherwise the incremental startup sync would skip every imported row as "older than the latest already synced". The progress card shows this as a second stage; on a large history it takes a while, and the behavioural and time-series pages are thin until it finishes.
