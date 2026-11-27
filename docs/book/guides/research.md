# Research Use

This chapter is for someone who wants to use a station's records in a study:
what the station records, how to take it away, how to draw a validation
sample, and what none of it measures. It describes what the code does; where
the code does not do something, it says so.

## What a detection is

A **detection** is one classifier's score for one species in one window of
audio (3 s for BirdNET V2.4, 4.5 s for BirdNET+ V3.0), at or above the
threshold in force, that passed the station's filters. It is not an
individual bird, and a count of detections is not a count of birds. A long
song is many detections.

The **confidence** on a detection is the classifier's output, mapped to 0–1
(a sigmoid of the logit, reshaped by `SENSITIVITY`, for V2.4; the network's
own 0–1 output for V3.0). It is **not a probability that the species was
there**. Nothing in this project has measured how a score relates to the
chance a detection is right, per species, at this site; published evaluations
of BirdNET report that the relation varies widely between species and
places. Two classifiers' scores are on different scales.

A **non-detection is not an absence.** The station records what crossed the
threshold. Unless the opt-in window record below is on, windows that produced
nothing are counted only as seconds of effort.

An **event** (`/api/v2/events`) groups detections of one species at one site,
at most one per source, within a tolerance of one another (policy
`events-1`). It is a call heard once or by several microphones, still not a
bird.

## What a run records

Every daemon start registers an **analysis run** (`analysis_runs`): the
binary's version; the model file's path, SHA-256 and size; the labels file's
path, SHA-256 and label count; the occurrence-filter model's SHA-256 when one
filters; the confidence threshold, sensitivity, overlap, species-frequency
threshold and station position. Each classifier of the run is a row of
`analysis_run_models` with its own files' SHA-256, and every detection row
carries `run_id` and `model_id`, so an export names the file each row's
classifier loaded.

A run registered by this version also records its **deciding configuration**
in `config_json`: one JSON object, keys sorted, with its SHA-256 in
`config_sha256`. It holds, at least, the run columns above, which classifiers
ran (`classifiers`, `classifier.primary`), each classifier's own threshold
(`classifier.<id>.threshold`) and whether an occurrence model filtered. Two
runs with the same `config_sha256` were configured alike. A run registered
before this was recorded has both columns empty: **not recorded**, which is
not the same as the defaults. No secret is ever written there: only named
settings are recorded, and a key whose name marks a secret is refused.

The **taxonomy edition** a labels file follows is known only where it can be
cited (today: the Perch label set). The study bundle prints it per run;
elsewhere it reads "not recorded". Scientific names from different editions
are not equated.

## Taking the data away

| Export | What it is |
|--------|------------|
| `/api/v2/detections/export/study?from=&to=` | **The study bundle** (below). |
| `/api/v2/detections/export?from=&to=` | The detections CSV / JSON (BirdNET-Pi's columns, then this project's). Confidence to four decimals; rejected rows left out. A convenience format, lossy. |
| `/api/v2/detections/export/search?…` | Exactly the rows a `/search` query lists, every page of them, in the same columns. The search page links to it. |
| `/api/v2/detections/export/model-scores` | Each classifier's own answer per detection. |
| `/api/v2/effort/export`, `/api/v2/effort/sites` | Seconds analysed per hour and source; each site's listening as bounds. |
| `/api/v2/detections/export/ebird` | eBird Record Format. `verdict=confirmed` limits it to reviewer-confirmed rows; without it, unreviewed detections are included. Whether eBird accepts unreviewed recorder detections is the submitter's question. |

### The study bundle

`GET /api/v2/detections/export/study?from=YYYY-MM-DD&to=YYYY-MM-DD` answers a
ZIP (at most 366 days, and at most a million rows per table) holding:

* `detections.csv`: **every column** of every detection row of the local
  dates, rejected ones included (`review_verdict` says which).
* `classifier_answers.csv`: each classifier's answer on each detection's
  window (`heard`, `below_bar`, `no_label`, `failed`, with score and bar).
* `review_events.csv`: the append-only review history: every verdict, who,
  when.
* `runs.csv`, `run_classifiers.csv`: every run and classifier, with
  `config_json` and a `taxonomy_edition` column.
* `source_effort.csv`: seconds of audio analysed per local hour and source.
* `site_effort.csv`: per site and local hour, the site listened **at least**
  `lower_bound_secs` and **at most** `upper_bound_secs`; `sensor_secs` is the
  sum over sources, a per-sensor denominator. `sites=source:site,…` maps
  sources to sites; a source not named is its own site.
* `events.csv`, `event_members.csv`: events of the UTC dates, with the policy
  version and tolerance (`tolerance_secs=`, default 4 s).
* `weather.csv`: hourly weather. `provider` and `product` are empty for
  hours stored before they were recorded. Open-Meteo's hours come from a
  forecast endpoint: modelled values, not observations.
* `README.txt` and `manifest.json`: the format (`schema_version` 1), the
  generator version, the parameters, the policy versions, what the bundle does
  not claim, and the SHA-256, size, row count and columns of every member.

The bundle is **deterministic**: the same database and the same parameters
give a byte-identical archive. No member holds the time of the export, every
table is read in a total order, and a real number is written in the shortest
form that reads back as the stored value, so a score of `0.70004` is not
rounded across a `0.7` threshold. To cite a bundle, record the SHA-256 of the
archive.

An empty CSV field is `NULL`. A text value that begins with `=`, `+`, `-`,
`@`, a tab or a carriage return has an apostrophe put in front, so a
spreadsheet does not run it as a formula; strip it before analysis if a name
genuinely starts with one of these.

**Licence.** The software applies none to exported observations, and the
project's own licence does not extend to them. Choosing one is the station
operator's or the study lead's decision; `manifest.json` has `"licence":
null` until then.

## Drawing a validation sample

The review queue is newest first and shows each detection's score, so its
verdicts are neither random nor blind. A **validation sample** is drawn by the
station instead (`/admin/validation`):

1. Write the protocol **before looking at the detections**: the dates, the
   species (or all), the score bin edges (for example `0, 0.5, 0.7, 0.8, 0.9,
   1`), whether each species is its own set of strata, how many items to draw
   from each stratum, whether review is blind, and the seed. Record why that
   seed, so nobody can say it was chosen after a look.
2. `POST /admin/validation/samples` with that protocol as JSON. The station
   reads the **frame** (every detection row of the dates, rejected ones
   included), cuts it into strata, and in each stratum draws
   `n = min(per_stratum, N)` items without replacement, ordered by
   SHA-256(seed, the row's key). The same seed and the same frame always draw
   the same items. Presentation order is shuffled the same way. The protocol,
   its SHA-256, the frame's size, and for each item its stratum, the stratum's
   size `N` and its inclusion probability `n / N` are stored, append-only.
3. `GET /admin/validation/samples/{id}` lists the items. With `blind` on
   (the default) it carries no score and no model, only what is needed to find
   and play the clip.
4. Record each verdict with `POST /admin/validation/samples/{id}/verdicts`:
   `present`, `absent`, `uncertain` or `not_identifiable`. Verdicts are
   append-only; a correction is a later verdict, and the latest counts.
5. `GET /admin/validation/samples/{id}/export.csv` gives every item with its
   stratum, `N`, inclusion probability, the detection's score and model (read
   at export time), and the latest verdict.

**The station computes no accuracy figure from it.** With `w = 1 / p` per
item, a design-weighted precision for the frame is
`Σ w·present / Σ w·(present + absent)`; how to treat `uncertain` and
`not_identifiable`, and which interval to report (one that respects the
strata, and the clustering of windows from one recording), are the
analyst's choices to state. A sample of detections says nothing about
**misses**: a bird the classifier never reported cannot be drawn. Estimating
recall needs annotated time windows chosen independently of the detections,
which this station does not draw.

## The per-species threshold suggestion

Species pages suggest a threshold from the station's own reviews (Youden's J
over the reviewed scores). It is offered only with at least 20 verdicts and 5
of each kind. It is fitted to **whatever was reviewed**, which is not a random
sample, weighs the two errors equally, and can never suggest a value below the
threshold in force when the rows were recorded, because nothing below it was
stored. Treat it as a prompt to look, not an estimate.

## Opt-in: every analysed window

A station can keep a record of every analysed window, with every label scored
at or above a floor, including windows that produced no detection. The store
(`birdnet_db::window_scores`) is in its own database file, outside the backup
ring, with a retention bound; human and sound classes are never written. It is
**off**: in this version no setting turns it on and the analyser does not
call it, so it records nothing yet. What it costs: about 60 bytes
per window plus 6 bytes per label at the floor. One source at defaults
analyses 28 800 windows a day; a synthetic day with two labels per window
measured 4.5 MB on disk (the store's test
`the_cost_of_a_day_is_measured_not_guessed`): about 134 MB per source at the
default 30-day retention, 1.6 GB a year if kept. How many labels cross a
floor on real audio, and what an SD card pays in writes, have not been
measured.

## A methods paragraph

The following describes what the software does. Fill in the bracketed parts;
delete what your station did not use.

> Audio was analysed with BirdNet-Behavior [version, from `runs.csv`
> `app_version`] using [model name] (SHA-256 [from `runs.csv`]), in
> [3 / 4.5] s windows with [overlap] s overlap, at a confidence threshold of
> [value] and sensitivity [value] [and per-classifier thresholds from
> `config_json`]. [An occurrence filter (SHA-256 …) excluded species below a
> frequency threshold of …]. Confidence values are classifier output scores,
> not calibrated probabilities. Listening effort was taken from the station's
> hourly ledger of audio analysed per source; site effort is reported as
> bounds (best-covered source to the sum over sources, capped at the hour).
> [Detections were validated on a stratified random sample of [n] detections
> drawn with seed [s] (protocol SHA-256 […]), reviewed blind to score;
> precision was estimated with inclusion-probability weights.] Data and the
> configuration of every run are in the study bundle (SHA-256 […], format
> version 1).

## What is not measured

* The relation between a score and the chance a detection is right, for any
  species at any site (no calibration).
* Recall, or false alarms per hour.
* Whether two runs on different hardware give identical scores.
* Weather provenance for hours recorded before this version.
