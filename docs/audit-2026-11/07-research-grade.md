# 07 — Research-grade readiness (PAM reviewer's audit)

Audit date 2026-11-22. Read-only: no code changed and no `cargo` run (another agent held the
build). Every repo claim below cites a file and line that was opened. Claims about **field
standards** (Raven, Darwin Core / Audubon Core, eBird, GBIF, GUANO/BWF, occupancy modelling,
Pérez-Granados 2023, Wood & Kahl 2024) are marked **FOLKLORE**: they come from general
knowledge, and nothing in this repo verifies them. The only standards text in the tree is the
register's Darwin Core row (`docs/UNATTENDED_DEPLOYMENT_AUDIT.md:823`) and the Raven header,
which a commit message says was checked against BirdNET-Analyzer v2.0.0
(`crates/birdnet-web/src/routes/export/raven.rs:5-9`). The session that implements any of
this must check each standard against its primary source.

The lens: **could a reviewer accept a paper whose data came from this device?** Three
questions decide it:

1. Can each record be traced to the audio, the model, the settings and the clock that made it?
2. Can absence be told apart from not listening?
3. Are the derived statistics what they say they are?

The answers, in short: (1) mostly, for the primary classifier, with gaps listed below. (2) Only
roughly, at a resolution of one day. The effort ledger also counts audio the station skipped.
(3) Several analytics are web-analytics measures applied to detections, and none of the
time-series ones look at effort.

Register IDs (`R-`, `FR-`, `UP-`, `S-`, `A-`, `G-`) are from
`docs/UNATTENDED_DEPLOYMENT_AUDIT.md` and `docs/HANDOFF.md` §5.

---

## 1. Schema summary (cited)

### 1.1 What a detection row carries

`detections`, built up by migrations (`crates/birdnet-db/src/migration.rs`):

| Column | Migration (line) | Meaning / caveat |
|---|---|---|
| `Date`, `Time` | 1 (`:63-64`) | **Local wall clock, no offset**, TEXT; `Time` holds whole seconds (`:644-645`) |
| `Sci_Name`, `Com_Name` | 1 (`:65-66`) | The label-file strings. No taxon ID and no taxonomy edition |
| `Confidence` | 1 (`:67`) | The **sensitivity-transformed** sigmoid score. Not raw; not calibrated (R-10) |
| `Lat`, `Lon`, `Cutoff`, `Week`, `Sens`, `Overlap` | 1 (`:68-73`) | Written per row since `e3f9b80` (UP-1, R-2). `Cutoff` is the threshold the row actually cleared |
| `File_Name` | 1 (`:74`) | The clip's name. Kept after the audio is pruned (`:543-549`) |
| `is_locked` | 7 (`:133`) | Protects the clip from purges |
| `chunk_offset_secs` | 11 (`:219`) | Start of the window inside the **source segment**, which is deleted after about 600 s. Not exported |
| `correlation_id` | 12 (`:250`) | Links the row to log lines |
| `Source` | 18 (`:463`) | Stream label (`local`, `RTSP_n`, `src_seed_n`) |
| `Duration_Secs` | 20 (`:506`) | Clip length |
| `Clip_Pruned_At` | 22 (`:562`) | Unix time the audio was removed. **Not projected** by any reader (`sqlite/types.rs:368`) |
| `import_batch_id` | 25 (`:700`) | → `import_batches` (source coordinates, UTC offset, applied shift; `:684-699`) |
| `review_verdict` | 26 (`:754`) | `confirmed` / `rejected` / NULL. Mirrors `detection_reviews` |
| `detected_at_utc` | 32/44 (`:1151`, `:1945-1957`) | Integer **seconds**. Live rows are stamped by the daemon (`src/daemon/processor.rs:1033-1038`); backfilled rows use the host tz database; a non-existent local time gets NULL |
| `run_id` | 43 (`:1902`) | → `analysis_runs` |
| `birdweather_soundscape_id` | 45 (`:1976`) | |
| `clip_offset_secs`, `detection_secs` | 47 (`:2011-2012`) | Where the window sits inside the saved clip (used by Raven) |
| `model_id`, `model_agreement` | 50 (`:2136-2137`) | Which classifier won; how many cleared their own bar |

Linked tables:

- `analysis_runs` (`:1882-1900`) holds, per daemon start: `app_version`, model name, path,
  SHA-256 and byte length, labels path, SHA-256 and count, `geomodel_sha256`, `confidence`,
  `sensitivity`, `overlap`, `sf_thresh`, `lat`, `lon`, `started_at`. There is **no end time**
  and **no record of secondary classifiers** (`src/daemon/run.rs:101-117`).
- `detection_model_scores` (`:2442-2456`) holds each classifier's score, bar, state and window
  for every detection that was written.
- `quarantine` (`:174-191`, plus `cutoff/sens/overlap` at `:1990-1992` and `run_id`) holds rows
  below a per-species threshold.
- `detection_reviews` (`:2553-2565`) holds **one** live verdict per detection (`:2590-2591`),
  with `notes`, `reviewed_at` and `detached`. **It has no reviewer column.**
- `detection_comments` (`:2092-2109`) is append-only, with `author` and `user_id`.

**Not stored anywhere:** scores below the threshold (`src/daemon/disposition.rs:142-154`, "drop
silently", "the model already applied the same gate"); top-k alternatives; the raw logit; a clip
checksum (no hits for `sha256` under `crates/birdnet-core/src/audio/extraction`); a stable row id
(R-3; migration 59 says so at `:2501-2506`); a station id (`docs/design/multi-station.md:31-34`);
a per-row clock-trust flag.

### 1.2 Effort and context tables

| Table | Key | What it measures | Writer | Caveat |
|---|---|---|---|---|
| `recording_effort` (`migration.rs:796-801`) | (local `date`, `source`) | Seconds a source's capture gauge read "up" | `src/integrations/effort.rs:56-91`: one 300 s credit per poll, per source with `source_up == Some(true)` | Measures **capture uptime, not analysis**. It does not subtract shed or dropped segments (RES-01). Daily only (RES-02). Local date (`:37-38`) |
| `audio_levels` (`:1054-1065`) | (local date, local hour, source) | Noise floor, SNR, flatness, rain, from **one segment per 5 min** | `src/integrations/acoustic_health.rs:24-36,79` | `samples` is a count of 5-minute probes, not of analysed segments |
| `sound_levels`, `sound_level_broadband` (`:1476-1499`) | (date, hour, source[, band]) | Third-octave and A/Z levels; `calibration_db` on the broadband row only | the same probe (`acoustic_health.rs:193-227`) | See RES-20 |
| `weather` (`:425-436`) | `at` (UTC) | Open-Meteo, MET Norway, Wunderground or PirateWeather | `crates/birdnet-integrations/src/weather.rs:343-366` | No provider column. Forecast endpoint. Off by default (RES-21) |
| `maintenance_runs` (`:531-534`), `audit_log` (`:326-333`) | | Job runs; who did what | | `audit_log` is pruned at **180 days** (`sqlite/queries/maintenance.rs:61`, `accounts/audit.rs:144`) |
| In memory only | | Half-hour uptime ring, 24 h (`src/capture/uptime.rs:1-8`); `birdnet_segments_shed_total`, `segment_dropped`, `files_analysed_total` (`crates/birdnet-web/src/metrics.rs:218,401`; `src/daemon/mod.rs:125,135-137`) | | **Lost on restart; never persisted.** These are the data effort needs |

---

## 2. Export formats

| Format | Route | Source rows | Columns / content | Missing for a methods section |
|---|---|---|---|---|
| Detections CSV | `GET /api/v2/detections/export` (`export/mod.rs:36`) | `detections_analytic` (rejected rows and excluded imports removed; `export/csv.rs:49-54`, `read.rs:341`), ordered by local `Date`, `Time` | The 12 BirdNET-Pi columns, plus `Event_Date` (RFC 3339 with offset), `Detected_At_UTC`, `Run_Id`, `Model_Name`, `Model_SHA256`, `Model_Id`, `Model_Agreement` (`csv.rs:245-248`) | `Source`, `review_verdict`, `clip_offset_secs`, `detection_secs`, `chunk_offset_secs`, `Duration_Secs`, `Clip_Pruned_At`, `import_batch_id`, labels SHA, sensitivity, run settings. No schema version, licence or citation header. Rejected rows cannot be exported; there is no confirmed-only filter |
| Detections JSON | same, `?format=json` | same | Every `DetectionRow` field (`sqlite/types.rs:124-202`, so **includes** `source`, `review_verdict`, the clip offsets), plus `event_date`, `model_name`, `model_sha256` | Same gaps, except the CSV-only ones |
| Species CSV/JSON | `/species/export` (`csv.rs:123-181`) | `top_species` (aggregate) | `Com_Name, Sci_Name, Count, Avg_Confidence` | No effort, no period, no verdict counts |
| eBird Record Format | `/detections/export/ebird` (`export/ebird.rs:1-31`) | analytic view, confidence ≥ 0.75 | One record per species per hour, `Number=X`, protocol and observers from the caller | Claims presence from **unreviewed** detections; `Duration` is always 60 (FR-5); no confirmed-only option (RES-27) |
| BirdDB.txt | `/detections/export/birddb` | analytic view | The 12 legacy fields | Legacy by design |
| Raven selection table | `/detections/export/raven`, `/recordings/{clip}/raven.txt` (`raven.rs:11-18`) | analytic view | BirdNET-Analyzer layout. Band fixed at 0–15 000 Hz (`raven.rs:43-48`) | Includes clips that have been **pruned** (RES-09). The band is not measured |
| Audacity labels | `/recordings/{clip}/labels.txt` | one clip | begin, end, label | — |
| Model scores | `/detections/export/model-scores` (`export/mod.rs:41-44`) | per classifier | score, bar, state | Exists only for rows that were written; no sub-threshold windows |
| Analysis runs | `GET /api/v2/analysis-runs[/{id}]` (`routes/analysis_runs.rs:19-23`) | | JSON | Not in any file export |
| Effort-corrected abundance | `GET /api/v2/analytics/abundance` (openapi) | DuckDB | Per species-week rate and `effort_hours` | The only route that exposes effort, and it is aggregated (RES-03, RES-15) |
| **Absent** | — | — | — | Darwin Core Archive or GBIF (R-DwC); Audubon Core media; Kaleidoscope; Parquet; effort export (FR-5); export of reviews, comments, quarantine, levels or weather; a deployment record (FR-3); a "methods block" (RES-24) |

---

## 3. Analytics validity

The verdict scale used below:

- **SOUND**: a defensible statistic.
- **SOUND-IF**: sound only with caveats the UI does not state.
- **METAPHOR**: a web-analytics construct that can mislead an ecologist.
- **BIASED**: a systematic error, demonstrable from the code.

| Analysis | Query (file:line) | Verdict | Why |
|---|---|---|---|
| Base view (`detections_ts`) | `birdnet-behavioral/src/queries.rs:90-98` | SOUND | Rejected rows are removed with null-safe logic. Instant and wall clock are kept apart. No confidence floor and **no effort join** |
| Sessionization ("bursts") | `queries.rs:234-265` | SOUND-IF | Gaps are measured on the UTC instant. Partitioned by `Sci_Name` only, so two microphones merge into one session but double the counts. A session is a vocal bout, not a visit or an individual |
| Retention / residency | `queries.rs:294-355` | METAPHOR | "Station weeks" is `COUNT(DISTINCT week)` of **any detection** (`:341-343`), not weeks with effort. A dead week where nothing was heard is treated as not existing. "Retention" of a species is not a demographic quantity |
| Dawn-chorus funnel / assembly | `queries.rs:367-388`, `:458-486` | METAPHOR / SOUND-IF | The hour window is fixed local clock time (`:381`), not relative to sunrise or civil twilight. It only means something with a sunrise-relative axis |
| Sequence match / count | `queries.rs:526-573` | METAPHOR | No null model. Common species match by chance |
| Next / previous species | `queries.rs:658-700` | METAPHOR | The denominator is stated (`HANDOFF` 32), but there is no expected value under independence |
| "Who answers whom" | `queries.rs:752-792`; `pages/behavioral_answers.rs:26,53-79` | METAPHOR, misleading name | Counts of A→B within 1 min across up to 56 ordered pairs. The floor is 3 and "one-sided" means ≥3:1. No chance baseline, no correction for multiple comparisons. Abundance and diel overlap produce "answers" |
| Co-occurrence | `sqlite/queries/correlation.rs:72-80,121` | SOUND-IF | Jaccard on 5-minute local blocks. Better than shared days (ANA7), but no expectation from diel overlap, and repeated autumn hours merge two real blocks |
| Companion species | `correlation.rs:143-168` | METAPHOR | Shared days. Confounded by how common each species is |
| Daily richness / Shannon H′ / Pielou | `birdnet-timeseries/src/queries/diversity.rs:25-86` | BIASED | Uses detections as if they were individuals. Effort-blind. No rarefaction. H′ over vocal activity is not community diversity |
| Accumulation curve | `diversity.rs:107-138` | SOUND-IF | Plotted against calendar date, not effort. One false positive adds a species |
| Moving average | `timeseries/src/queries/trend.rs:28-133` | BIASED | Downtime is zero-filled. The doc at `:11-14` claims days the station was not running are NULL; the code checks only the spine (`:60-63`) |
| Year over year | `trend.rs:135-232` | BIASED | Raw counts, with no correction for effort or recording window |
| Anomaly z-score | `trend.rs:253-330` | BIASED | Gaussian z on overdispersed counts. Downtime days are zeros. At z = 2 over a 180-day lookback, chance flags are expected (about 4.6 % two-sided under normality — FOLKLORE arithmetic). No multiplicity control |
| Quiet days / gaps | `timeseries/src/queries/gap.rs:14-112` | BIASED | Its doc says "useful for identifying equipment outages" (`:69`), but it cannot tell an outage from silence: there are **0 hits for `effort` in `crates/birdnet-timeseries/src`** |
| Effort-corrected abundance | `phenology/abundance.rs:252-362` | BIASED (RES-03) | The numerator counts every source; the denominator is the single largest source (`:322`) |
| Relative abundance | `abundance.rs:105-160` | SOUND-IF, mis-documented | Normalising to the peak week does **not** correct for effort, which the doc claims (`:9-13`) |
| Phenology first/last date | `phenology/timing.rs:95-130` | BIASED | Uses MIN and MAX of the detection date. Depends on effort and on sample size. One early false positive sets the arrival date |
| First of year / season, returning | `birdnet-db/src/species_tracking.rs:12-24` | BIASED | Effort-blind (no `effort` hits). Downtime reads as an absence, and the return after it as an arrival |
| Dawn-chorus rings | `pages/dawn_chorus.rs:69,146,215-230` | SOUND-IF | 60 days bucketed by **local clock hour**, overlaid with **today's** sunrise |
| Phantom heuristics | `birdnet-db/src/phantoms.rs:1-30` | SOUND (as an advisory) | It reports and never acts |
| Threshold suggestion (Youden's J) | `birdnet-db/src/thresholds.rs:18-31` | SOUND-IF | Labels come from non-random reviews of **already-admitted** detections, so the sample is truncated and biased (RES-07) |

DST and clock handling are the strong point: instants for ordering and wall clock for grouping
(`queries.rs:55-89`), non-existent local times get NULL (migration 44). The weak point is that
every **bucket** is local: effort (`effort.rs:37`), levels and analytics. On the day the clocks
change, a day has 23 or 25 hours and the repeated hour is merged. No analytic normalises for this.

---

## 4. Findings

### RES-01: The effort ledger counts listening the station did not analyse
- Severity: **high** — Kind: bug — **NEW** (it sharpens FR-5 / WE-2)
- Evidence: `src/integrations/effort.rs:69-91` credits 300 s to every source whose gauge is up.
  The gauge is set by the supervisor (`src/capture/supervisor.rs:520,549,557`) and also by
  **each detection** (`src/daemon/processor.rs:1063`, "a detection here proves the source
  produced a file"). Load shedding analyses "one segment in two" whenever the queue holds more
  than 40 or the CPU is at or above 80 °C (`docs/book/field/deployment.md:119-126`;
  `src/daemon/mod.rs:168-179`). It is counted only in an in-memory counter
  (`src/daemon/mod.rs:135-137` → `metrics.rs:401`), as are dropped segments (`:125`).
  Effort is not debited for privacy-suppressed windows (`birdnet-core/src/detection/mod.rs:56`),
  failed classifier windows (HANDOFF 39) or digital-silence skips either. A related claim is
  not enforced: the stream drain's assumption that "an unprocessed segment is never removed" is
  stated, not checked (`birdnet-core/src/audio/capture/disk/manager.rs:494-498`). Only the file
  in flight is leased.
  Status: VERIFIED-BY-READING. Whether a size purge drains queued, unanalysed segments is
  CONJECTURED; the probe would fill the watch dir past `STREAM_MAX_MB` while inference is paused
  and count `files_analysed_total` against segments written.
- Why a reviewer cares: thermal shedding happens on summer afternoons, on the hardware this
  project targets. That puts a seasonal and diel bias in the denominator, and the station
  reports the hours as full listening.
- Proposed change: add a table `analysis_effort(utc_hour INTEGER, source TEXT, run_id
  INTEGER, segments_captured, segments_analysed, seconds_analysed, shed_backlog,
  shed_thermal, dropped, privacy_windows_suppressed, failed_windows, PRIMARY KEY(utc_hour,
  source, run_id))`. Fold it at the point where each segment is decided. Make
  `recording_effort` a view over it, or keep it as the capture-uptime ledger with that name.
- Acceptance gate: force the shed policy (a thermal closure returning true) over 20 segments.
  Assert `seconds_analysed` equals 10 × the segment length while capture uptime equals 20 ×.
  Watch the gate fail against the current ledger.
- Effort M. Depends on nothing. Feeds RES-02, RES-03 and RES-14.

### RES-02: Effort has a resolution of one day, so detection/non-detection histories cannot be built
- Severity: **high** — Kind: gap — **NEW** (export side: ALREADY-TRACKED FR-5)
- Evidence: `recording_effort` is keyed `(date, source)` (`migration.rs:796-801`). Half-hour
  uptime exists only in a 24 h in-memory ring (`src/capture/uptime.rs:1-8`). The "listened"
  test is a day with ≥ 60 s (`pages/listening.rs:26`), and effort is "only ever used to add a
  statement … never to scale" (`:12-16`). No route lists effort; the only one that returns it is
  `/analytics/abundance`.
  Status: VERIFIED-BY-READING.
- Why: occupancy models need survey occasions with known effort (FOLKLORE). So do hour-of-day
  rates and the dawn chorus. Daily totals cannot say whether 06:00–07:00 was recorded, and a
  day with 61 s of recording is drawn as a full day.
- Proposed change: persist the uptime ring as `capture_intervals(source, start_utc, end_utc,
  state)`, or use RES-01's hourly table. Add `GET /api/v2/effort?from&to&grain=hour|day` and a
  CSV. Add a **detection-history export**: species × occasion (hour, day or sunrise-relative
  window) with 1 / 0 / NA, where NA means no effort.
- Acceptance gate: a fixture with the source down 05:00–07:00 exports NA for those occasions and
  0 for listened hours with no detection. It must fail against a daily-only ledger.
- Effort M–L.

### RES-03: Effort-corrected abundance divides detections from every source by one source's hours
- Severity: **high** — Kind: bug — **NEW** (the fix for R-15 inverted the bias)
- Evidence: `phenology/abundance.rs:276` says "Detections are not per source". The effort CTE
  takes `MAX(seconds)` per day (`:322`), and the numerator is `COUNT(*)` over all rows
  (`:340-341`). But detections **are** per source: "the same bird heard by two streams is
  recorded as two detections" (`migration.rs:455-457`), and no cross-stream collapse exists
  (`sqlite/types.rs:56`, "later, opt-in").
  Status: VERIFIED-BY-READING. Probe: two sources, the same bird in both, 1 h each. Expect a
  rate of 1/h; the current SQL gives 2/h.
- Why: a two-microphone station reports twice the rate of a one-microphone station for the same
  birds. Comparisons between stations or years that span a microphone change are wrong.
- Proposed change: compute the rate per source (detections of source s over hours of source s)
  and report each, or collapse duplicates across sources first (an `event_id` per window) and
  divide by the union of covered time. Document which one is used.
- Acceptance gate: the probe above, red on today's SQL. Its counterpart, one source, stays at 1/h.
- Effort S–M.

### RES-04: No record of windows that were analysed and found nothing, and no scores below the threshold
- Severity: **high** — Kind: gap — **NEW**
- Evidence: `src/daemon/disposition.rs:142-154` (`DropBelowGlobal`). Only species above the
  threshold become rows; `detection_model_scores` exists only for rows that were written
  (`migration.rs:2442-2456`).
  Status: VERIFIED-BY-READING.
- Why: estimating per-species precision, recall or a score-to-probability calibration needs
  scores **across the range**, including below the deployment threshold. Wood & Kahl 2024
  recommend logistic calibration of score against verified outcome (FOLKLORE). Re-thresholding
  a season after the fact is impossible without raw audio (R-5). Occupancy also needs
  "analysed, not detected".
- Proposed change: add an opt-in `window_scores` store: per analysed window, the top-k (for
  example 5) species and scores, or the per-species maximum per segment, in a compact table
  partitioned by month, with an explicit retention. Add `segments_analysed` to the effort table
  (RES-01).
- Acceptance gate: with the feature on, a window whose best score is 0.4 against a 0.75 cutoff
  appears in `window_scores` and not in `detections`.
- Effort L (storage budget on an SD card needs measuring first).

### RES-05: No re-analysis or batch path, and SD-card recordings cannot be ingested
- Severity: **high** — Kind: gap — **ALREADY-TRACKED (R-5, FR-2)**, with a **NEW** detail
- Evidence: register R-5 (`UNATTENDED_DEPLOYMENT_AUDIT.md:816`) is still open: the only hits for
  "re-analy" are comments (`src/daemon/run.rs:226`, `src/cli.rs:997`). Raw audio is kept only if
  `RAW_AUDIO_KEEP_EVERY` > 0, and the default is 0 (`docs/book/getting-started/configuration.md:88`).
  **NEW:** `RecordingFile::parse` accepts only `YYYY-MM-DD-birdnet-…-HH:MM:SS`
  (`birdnet-core/src/detection/types.rs:212-239`). AudioMoth or Song Meter names (UTC
  `YYYYMMDD_HHMMSS`, FOLKLORE) are refused, so autonomous-recorder deployments cannot use the
  station as an analyser.
  Status: VERIFIED-BY-READING.
- Proposed change: add `birdnet-behavior analyze --input DIR --name-pattern {audiomoth|songmeter|birdnet|regex}
  --tz UTC --deployment ID --run-label …`. It would write a new `analysis_runs` row
  (`kind='batch'`), a parent `deployments` row and, from RES-01, effort with the file's
  duration as the effort. Add `--reanalyze --from --to --model …` over `<recordings>/raw`.
- Acceptance gate: two runs over the same fixture folder with different thresholds give two
  `run_id`s with disjoint rows. Running twice with the same settings gives identical rows
  (the determinism probe, RES-31).
- Effort L.

### RES-06: The reviewer's identity is lost after 180 days, and one verdict overwrites the last
- Severity: **high** — Kind: gap — **NEW**
- Evidence: `detection_reviews` has no user column (`migration.rs:2553-2565`). The live verdict
  is unique per detection (`:2590-2591`). The reviewer is recorded only through
  `crate::audit::audit(…Some(&request_user)…)` (`pages/detection_reviews.rs:531-545`), and
  `audit_log` is deleted after `AUDIT_RETENTION_DAYS = 180` (`sqlite/queries/maintenance.rs:61`,
  `accounts/audit.rs:144`). Only two states exist: `confirmed` and `rejected` (`:2560`).
  Status: VERIFIED-BY-READING.
- Why: a methods section has to say who verified the data and how disagreement was resolved.
  After six months the station cannot say who reviewed what, and a second reviewer silently
  replaces the first.
- Proposed change: make `review_events` append-only (`detection key, reviewer_user_id,
  reviewer_name, verdict ∈ {confirmed, rejected, uncertain, wrong_species}, proposed_sci_name,
  call_type, blind BOOLEAN, protocol_id, at_utc`) and derive `review_verdict` from a declared
  rule (latest, or consensus). Exempt `detection.review` audit rows from pruning until then.
- Acceptance gate: two reviewers disagree, `audit_log` is pruned, and both events survive with
  their names. The derived verdict follows the declared rule.
- Effort M.

### RES-07: No validation protocol: reviewers see confidence, the queue is not random, and there are no precision estimates or intervals
- Severity: **high** — Kind: gap — **NEW**
- Evidence: the queue is newest first (`sqlite/queries/detection_reviews.rs:526`). Each queue
  row draws a confidence pill (`pages/detection_reviews.rs:314`). No `RANDOM()` or sampling
  exists anywhere in the review code. The threshold suggestion fits on whatever was reviewed
  (`thresholds.rs:18-31`), which is only detections already admitted above the cutoff.
  Exports cannot include rejected rows and cannot filter to confirmed rows (`csv.rs:14-22,49`).
  Status: VERIFIED-BY-READING.
- Why: a per-species precision estimate (Pérez-Granados 2023 reports that BirdNET evaluations
  vary widely; FOLKLORE) needs a random or score-stratified sample, reviewed blind, with a
  binomial interval.
- Proposed change: add a "Validation study" mode. The operator picks species, a period, n per
  score bin and a seed. The server draws a stratified random sample, hides confidence and
  model, records `protocol_id`, and reports precision per bin and species with Wilson or
  Clopper–Pearson intervals and a logistic score→P(correct) fit. Export the sample and verdicts
  as CSV. Add `?verdict=confirmed|rejected|any|unreviewed` to every export.
- Acceptance gate: with seed s, the same sample is drawn twice. The blind page contains no
  confidence value in the rendered region. On a fixture with known truth, the computed interval
  contains it.
- Effort M–L.

### RES-08: The CSV export leaves out the columns a methods section needs, and carries no schema version or licence
- Severity: medium — Kind: gap — **NEW** (Darwin Core: ALREADY-TRACKED R-DwC; effort: FR-5)
- Evidence: header at `export/csv.rs:245-248`. `Source` and `review_verdict` are in JSON only.
  `chunk_offset_secs`, `Clip_Pruned_At`, `import_batch_id` and `birdweather_soundscape_id` are
  in no reader (`sqlite/types.rs:368`). There are no exports of reviews, comments, quarantine,
  effort, levels or weather (`export/mod.rs:34-47`). Rows are ordered by local `Date`, `Time`
  (`read.rs:342`), not by instant.
  Status: VERIFIED-BY-READING.
- Proposed change: add `/api/v2/export/dataset.zip`: `detections.csv` with every column,
  `runs.csv`, `effort.csv`, `reviews.csv`, `comments.csv`, `levels.csv`, `weather.csv`,
  `deployment.json`, a `datapackage.json` (Frictionless; FOLKLORE format) with column units and
  types, `schema_version`, a licence and a citation. Darwin Core Archive later, as R-DwC.
- Acceptance gate: a round-trip test reads every table back and matches it against SQLite. A
  column-coverage gate fails if a `detections` column appears in no export.
- Effort M.

### RES-09: The Raven export points at clips that have been deleted
- Severity: medium — Kind: bug — **NEW**
- Evidence: `selection_window` needs only `file_name` and an offset or duration
  (`raven.rs:52-59`). `analytic_detections` does not filter `Clip_Pruned_At` (`read.rs:340-343`),
  and `DETECTION_COLS` does not project it (`sqlite/types.rs:368`).
  Status: VERIFIED-BY-READING. Runtime is CONJECTURED. Probe: seed a row with `Clip_Pruned_At`
  set and call `/detections/export/raven`.
- Why: Raven opens a multi-file table and fails, or the analyst silently loses selections.
- Proposed change: project `Clip_Pruned_At`. Leave pruned rows out of selection tables, or
  write them with an empty `Begin Path` plus a comment column. Count the omissions in a header
  comment or a response header.
- Acceptance gate: the probe returns no selection for the pruned clip.
- Effort S.

### RES-10: Retention deletes the audio behind reviewed and commented records
- Severity: medium — Kind: footgun — **NEW**
- Evidence: the clip-retention and species-cap SQL spare only `is_locked`
  (`src/maintenance.rs:1061-1072`, `:1149-1165`). The disk-full purge does the same
  (`deployment.md:104-105`).
  Status: VERIFIED-BY-READING.
- Why: a verified record without its audio cannot be re-verified by a reviewer, and audio is the
  evidence (associatedMedia, FOLKLORE).
- Proposed change: exempt rows with a `review_verdict` or a comment by default. Add a setting
  `RETAIN_REVIEWED_AUDIO` (default on). Show "evidence at risk" counts.
- Acceptance gate: a confirmed row older than the cap keeps its clip, and an unreviewed sibling
  is pruned (the counterpart).
- Effort S.

### RES-11: `analysis_runs` records less than the configuration that decided what was recorded
- Severity: medium — Kind: gap — **NEW** (it extends R-1)
- Evidence: `RunManifest` and `NewAnalysisRun` record only the primary classifier and five
  numbers (`src/daemon/run.rs:18-42,101-117`). These are not recorded:
  - secondary classifiers' files and hashes, and their thresholds (`model_thresholds` is held
    in memory, `run.rs:40-42`);
  - privacy threshold and rank (`src/daemon/mod.rs:312-322`);
  - the noise and corroboration filters;
  - species include and exclude lists, which are **re-read live** (`admin/settings/applies.rs:7-8`)
    without starting a new run;
  - per-species thresholds and intervals (`species_intervals`, migration 56);
  - dynamic-threshold configuration (off by default, `detection/dynamic_threshold/mod.rs:133-136`);
  - segment length, clip LUFS normalisation, and the source's signal chain (gain, the 120 Hz
    high-pass at `capture/types.rs:843`, AGC).
  There is no `ended_at`.
  Status: VERIFIED-BY-READING.
- Proposed change: add `analysis_runs.config_json` (canonical, sorted keys) with
  `config_sha256`, plus `run_models(run_id, model_id, path, sha256, labels_sha256, threshold)`.
  A live settings change that affects admission closes the run and opens a new one. Set
  `ended_at` on clean shutdown.
- Acceptance gate: changing the exclude list while running produces a new `run_id` on the next
  row. Two classifiers produce two `run_models` rows.
- Effort M.

### RES-12: Clock trust is never written down, and capture records under an untrusted clock
- Severity: medium — Kind: gap — **NEW** (TD-8 covers the alert wording)
- Evidence: the NTP state goes only to a gauge and an alert (`src/integrations/station_health.rs:493-503`).
  Recording fails **open** on an implausible clock (`src/capture/runloop.rs:110-111`).
  Timestamps come from the filename's local time (`src/doctor/clock.rs:6-8`). There is no table
  of clock steps.
  Status: VERIFIED-BY-READING.
- Why: a Pi has no RTC. Rows written before NTP syncs, or during months of free-running, are
  misdated with nothing to flag them.
- Proposed change: add `clock_events(at_utc, kind ∈ {synced, unsynced, step}, delta_secs)` from a
  monotonic-vs-realtime comparison each poll. Add a nullable `clock_synced` per hour in the
  effort table, and a `Clock_Trusted` export column derived by joining them.
- Acceptance gate: a simulated step of +3600 s writes a `step` event. Rows in the unsynced
  interval export `Clock_Trusted=false`.
- Effort M.

### RES-13: Time resolution, and local-time buckets in the levels and effort tables
- Severity: medium — Kind: gap — partly **ALREADY-TRACKED (R-8, fixed for exports)**, rest **NEW**
- Evidence:
  - `detected_at_utc` is integer seconds (`migration.rs:1151`), and `Time` is truncated
    (`:644-645`).
  - The window start inside the segment (`chunk_offset_secs`, `processor.rs:1001`) is not
    exported.
  - `audio_levels`, `sound_levels` and `recording_effort` key on the **local** date and hour
    (`audio_levels.rs:54-57`, `effort.rs:37-38`), so the repeated autumn hour merges two hours.
  Status: VERIFIED-BY-READING.
- Proposed change: key the new effort and level tables on UTC hours, keep the local values as
  derived columns, and export a `Begin_UTC` with milliseconds (instant plus window offset).
- Acceptance gate: an `Europe/Berlin` fall-back fixture produces two level rows for 02:00–03:00.
- Effort S–M.

### RES-14: The time-series analytics ignore effort entirely
- Severity: **high** — Kind: bug — **NEW**
- Evidence: 0 hits for `effort` under `crates/birdnet-timeseries/src`. The moving average
  zero-fills from the first detection (`trend.rs:21,60-63`) while its doc says days the station
  was off are NULL (`:11-14`). YoY uses raw counts (`:135-232`). The anomaly z-score treats
  downtime as zeros (`:253-330`). `QuietDays` claims to find outages (`gap.rs:69`). Richness and
  Shannon (`diversity.rs:25-86`).
  Status: VERIFIED-BY-READING.
- Why: a week of downtime reads as a collapse and an anomaly. A longer solar window in June
  reads as an increase.
- Proposed change: join the effort table in each builder. Return `effort_hours` and a rate.
  Make a day NULL below an effort floor (for example 50 % of the scheduled window). Mark
  anomalies only where effort is normal. Correct the moving-average doc.
- Acceptance gate: a fixture with 7 days of effort 0 in the middle gives NULL moving averages and
  no "low" anomalies there, and fails against today's SQL.
- Effort M. Depends on RES-01 and RES-02.

### RES-15: "Abundance" is a vocal-activity rate, its doc claims an effort correction it does not make, and absences are dropped
- Severity: medium — Kind: footgun — **NEW**
- Evidence: `abundance.rs:9-13` says relative abundance "corrects for this [effort]" by dividing
  by the peak week. The effort-corrected `weekly` CTE is built from detections only
  (`:337-346`), so a species-week that was listened to with zero detections is **absent**, not
  a rate of 0. There is no interval.
  Status: VERIFIED-BY-READING.
- Proposed change: rename it in the UI and API docs to "detection rate (detections per listened
  hour)". Emit zero rows over the effort spine. Add a Poisson or negative-binomial CI per week.
  Fix the module doc.
- Acceptance gate: a listened week with no detections of species X returns `rate=0`, with an
  upper CI bound > 0.
- Effort S–M.

### RES-16: The behavioural "patterns" are web-analytics measures with no null model
- Severity: medium — Kind: footgun — **NEW**
- Evidence:
  - "Who answers whom": `answers_sql` (`queries.rs:752-792`), a floor of 3 and a "one-sided"
    rule (`behavioral_answers.rs:26,53-79`).
  - Co-occurrence: `correlation.rs:72-80,121`.
  - Retention: weeks counted from detections (`queries.rs:341-343`).
  - Next/previous species: `queries.rs:658-700`.
  Status: VERIFIED-BY-READING.
- Why: "answers" implies interaction. With 8 species there are 56 ordered pairs, tested without
  correction, and the counts are driven by abundance and shared diel timing. A reviewer would
  require a permutation or randomisation baseline (FOLKLORE: circular time-shift null models are
  standard in acoustic co-occurrence work).
- Proposed change: compute an expected count under a circular time-shift permutation (per day,
  per species) and report observed/expected with a permutation p and an FDR correction. Rename
  "answers" to "heard within 1 min after". Retention should use effort weeks.
- Acceptance gate: two independent Poisson species with equal rates give an observed/expected
  ratio of about 1 and no "one-sided" headline. The counterpart, a planted lagged follower, is
  flagged.
- Effort M.

### RES-17: Dawn analyses use clock hours and today's sun across a 60-day window
- Severity: medium — Kind: footgun — **NEW**
- Evidence: the rings take 60 days of local-hour buckets (`dawn_chorus.rs:69,146`) and draw
  **today's** sunrise and dusk (`:215-230`), while the page claims to show species singing
  "relative to sunrise" (`:405`). The funnel and assembly windows use a fixed
  `hour_start..hour_end` (`queries.rs:381,479`).
  Status: VERIFIED-BY-READING.
- Why: across 60 days near an equinox at mid-latitude, sunrise moves by more than an hour, and
  a DST change adds another. Onset-time statistics then mix season with behaviour (FOLKLORE
  magnitude).
- Proposed change: add a `minutes_from_sunrise` column (computed per row from the row's date and
  station coordinates, using `birdnet-scheduler`) to `detections_ts`. Offer a sunrise-relative
  axis and default the dawn analyses to it.
- Acceptance gate: a fixture where a species always sings at sunrise − 10 min over 60 days gives
  a single-bucket peak on the relative axis and a smeared peak on the clock axis.
- Effort M.

### RES-18: Pseudo-replication and double counting across sources
- Severity: medium — Kind: footgun — **NEW** (UP-6 and HANDOFF 40 are adjacent)
- Evidence: no cross-stream collapse exists (`sqlite/types.rs:56`; `migration.rs:455-460`). Every
  aggregate counts rows, so N microphones give N× the count. Shannon uses detection counts
  (`diversity.rs:59-86`). Overlapping windows (`Overlap`) also produce several rows per
  vocalisation.
  Status: VERIFIED-BY-READING.
- Proposed change: add an `event_id` that groups (window instant ± overlap, species) across
  sources and overlapping windows. Offer "events", "detection-minutes" and "rows" as units, and
  say which one every chart uses.
- Acceptance gate: two sources hearing the same call produce 1 event and 2 rows.
- Effort M.

### RES-19: Phenology dates and first-of-season flags are effort-blind and sensitive to single false positives
- Severity: medium — Kind: footgun — **NEW**
- Evidence: `MIN` and `MAX` of the date (`phenology/timing.rs:116-119`). `species_tracking.rs:12-24`
  has no effort input. The only guard is `min_detections`.
  Status: VERIFIED-BY-READING.
- Proposed change: use quantile dates (for example the 5th and 95th percentile of detection
  days), require confirmed detections or k days within w, and report whether the station had
  effort in the 14 days before. Flag "first of season after an outage" as uncertain.
- Acceptance gate: one detection on 2 Jan plus a run from 1 Apr gives an arrival in April.
  An outage from 1 Mar to 1 Apr flags the April arrival as "effort gap before".
- Effort S–M.

### RES-20: Sound levels are labelled dB SPL from one unverified offset and a ~5 % sample
- Severity: medium — Kind: footgun — **NEW** (FR-6 tracks "referenced to the ADC")
- Evidence:
  - One environment variable sets the offset for **all** sources (`acoustic_health.rs:236-245`).
  - Any non-zero offset makes the unit "dB SPL" (`routes/system.rs:90`).
  - Band rows have no calibration column; the broadband row's offset is overwritten mid-hour
    (`sound_levels.rs:178-182`).
  - Levels are measured on the newest segment once every 5 min (`acoustic_health.rs:24-36,79`).
  - The 120 Hz high-pass and optional AGC are applied at capture (`capture/types.rs:843`;
    `migration.rs:379-381`) and are not recorded beside the levels.
  Status: VERIFIED-BY-READING.
- Proposed change: give each source its own `calibration(db_offset, method, calibrator,
  reference_db, frequency_hz, date, operator)` record. Label the unit SPL only when such a record
  exists. Store the HP/AGC/gain state per hour. Describe the measurement as "LAeq of a 15 s
  sample every 5 min", not hourly.
- Acceptance gate: an offset without a calibration record reports "dBFS + offset (unverified)".
  A station with AGC on refuses to report SPL.
- Effort M.

### RES-21: Weather comes from a forecast model, from mixed providers, with no provenance
- Severity: medium — Kind: gap — **NEW** (A-6 tracks weather on a detection)
- Evidence: Open-Meteo `/v1/forecast?…&past_days=1&forecast_days=2` (`birdnet-integrations/src/weather.rs:357-366`),
  upserted on `at` (`birdnet-db/src/weather.rs:110-114`). The four providers
  (`weather.rs:343-354`) share a table with no provider or model column (`migration.rs:425-436`).
  The licence and attribution of the data are not stored. It is off by default (`:420-421`).
  Status: VERIFIED-BY-READING.
- Why: a covariate has to say whether it was observed or modelled, by whom, and at what
  resolution. A personal weather station (Wunderground) mixed with model data is a different
  variable.
- Proposed change: add `provider`, `product ∈ {observation, analysis, forecast}`, `lead_hours`
  and `licence` columns. Prefer an archive or reanalysis endpoint for past hours. Export it with
  the dataset.
- Effort S–M.

### RES-22: There is no deployment record or station identity
- Severity: medium — Kind: gap — **ALREADY-TRACKED (FR-3, UP-6)**
- Evidence: `audio_sources` holds only the signal chain (`migration.rs:361-386`). Gain edits are
  not versioned (only `updated_at`). `multi-station.md:31-34` says "nothing mints or persists an
  id".
  Status: VERIFIED-BY-READING.
- Proposed change (concrete): add `stations(station_uuid)` and `deployments(id, station_uuid,
  source_id, start_utc, end_utc, lat, lon, coord_uncertainty_m, elevation_m, mic_make,
  mic_model, mic_sensitivity_dbv, height_m, orientation_deg, enclosure, windscreen, habitat,
  notes)`. Make `detections` reach a deployment through (source, instant).
- Effort M.

### RES-23: Clips carry thin metadata and no checksum
- Severity: low — Kind: gap — **NEW**
- Evidence: RIFF INFO holds only the name, the scientific name, "Confidence: 87% [local time]"
  and the date (`audio/extraction/metadata.rs:11-17`). There are no coordinates, offset, model
  or run, no GUANO (FOLKLORE format), and no stored hash.
  Status: VERIFIED-BY-READING.
- Proposed change: write GUANO or iXML with UTC time, location, run, model SHA and station. Store
  `clip_sha256` on the row and verify it on export.
- Effort S.

### RES-24: No researcher chapter and no citable methods block
- Severity: low — Kind: polish/opportunity — **NEW**
- Evidence: no book chapter covers effort, verification or validity. A grep for
  `occupancy|Darwin|methods section` under `docs/book` finds nothing. `CITATION.cff:24` licenses
  the **software** (CC BY-NC-SA). No export states a data licence.
  Status: VERIFIED-BY-READING.
- Proposed change: add `GET /api/v2/methods` that renders the active runs, model hashes,
  thresholds, filters, effort totals, review protocol and software version as a paragraph and as
  JSON. Add a "Using the data in research" chapter. Add a data-licence setting carried into
  every export.
- Effort S.

### RES-25: No stable detection identifier
- Severity: medium — Kind: gap — **ALREADY-TRACKED (R-3)**
- Evidence: `migration.rs:2501-2506`.
- Note: RES-06, RES-08 and RES-18 all key more cleanly on an id. Consider a UUIDv7 column
  written at insert and backfilled once, beside the existing `DetectionKey`.

### RES-26: A "listened" day needs only 60 s of recording
- Severity: low — Kind: footgun — **NEW**
- Evidence: `pages/listening.rs:26`, used by the heat map, terrain, charts, year-in-review and
  species pages (the callers grepped in `pages/*.rs`).
  Status: VERIFIED-BY-READING.
- Proposed change: shade days by the fraction of the scheduled window they covered. Treat a day
  under 50 % as "partial".
- Effort S.

### RES-27: The eBird export claims presence from unreviewed detections
- Severity: medium — Kind: footgun — **NEW**
- Evidence: the floor is 0.75 on unreviewed rows (`export/ebird.rs:14-16,48`). There is no
  verdict filter. `Stationary` with one observer is the default for an unattended recorder
  (`:23-27`).
  Status: VERIFIED-BY-READING. eBird's policy on automated or recorder-only detections is
  FOLKLORE; check it against eBird's current rules before deciding the default.
- Proposed change: default to `verdict=confirmed`, require an explicit flag for unreviewed rows,
  put "automated acoustic detection, BirdNET <model sha>" in the checklist comments, and set
  `Duration` from effort.
- Effort S.

### RES-28: Kept raw audio bypasses the privacy filter, and suppressions are invisible
- Severity: low (higher for public-facing stations) — Kind: gap — **NEW** for raw keep;
  suppression counts ALREADY-TRACKED (S-5, G-13)
- Evidence: raw keep copies aged segments verbatim (`deployment.md:111-117`). There are no
  privacy, speech or voice hits in `birdnet-core/src/audio/capture/disk/*.rs`.
  Status: VERIFIED-BY-READING. That a kept segment can contain speech is CONJECTURED (likely).
- Proposed change: skip or mute kept segments that contain windows the privacy filter flagged.
  Log `privacy_suppressed` per hour (RES-01) and per kept segment.
- Effort S–M.

### RES-29: Offsite backup covers the database, not the audio evidence
- Severity: low — Kind: gap — **NEW**
- Evidence: the offsite flow encrypts and uploads `backups/birds.db.backup.*`
  (`birdnet-integrations/src/offsite/mod.rs:12-17`).
  Status: VERIFIED-BY-READING for the database. Whether the local full backup or tar includes
  clips was not read (CONJECTURED).
- Proposed change: add an optional offsite sync of `recordings/` (locked, reviewed and raw
  audio) with a manifest of SHA-256s.
- Effort M.

### RES-30: Confidence is presented as a probability
- Severity: low — Kind: polish — **ALREADY-TRACKED (R-10)**
- Note: RES-07's logistic calibration is the constructive answer to R-10.

### RES-31: Reproducibility of outputs is unmeasured
- Severity: low — Kind: gap — **NEW**
- Evidence: inference runs on 2 threads (HANDOFF item 4). The behavioural results depend on a
  community DuckDB extension whose semantics changed between versions (`queries.rs:210-224`).
  Pages show the extension version (`pages/behavioral.rs:137` etc.); whether the JSON routes
  carry it was not checked.
  Status: CONJECTURED. Probe: analyse the same WAV twice and on x86 and aarch64, then compare
  scores bit-for-bit.
- Proposed change: add a determinism gate (same file → identical rows on one platform; a
  tolerance across platforms), and put the extension version and `app_version` into every
  analytics JSON response.
- Effort S.

### RES-32: The taxonomy edition is not recorded
- Severity: low–medium — Kind: gap — **NEW** (HANDOFF 40 tracks merging names across classifiers)
- Evidence: rows carry free-text `Sci_Name` and `Com_Name` (`migration.rs:65-66`).
  `labels_sha256` identifies the label file, not the taxonomy it follows
  (`analysis_runs.rs:30-33`).
  Status: VERIFIED-BY-READING.
- Proposed change: record `taxonomy ∈ {eBird/Clements YYYY, IOC x.y}` per run, and export
  `taxonID` or `scientificNameID` where it is known.
- Effort S.

---

## Not examined

- `crates/birdnet-behavioral/src/connection/*` (sync and live): whether `recording_effort` and
  the DuckDB copy agree, beyond the mirror call at `effort.rs:99-104`.
- `birdnet-timeseries` `activity.rs` and `peak.rs`, and the executors, beyond the module index.
- `pages/occurrence_page.rs` and the geomodel filter's week logic. `quarantine.rs` page
  behaviour. `model_scores` export columns.
- `routes/openapi.rs` beyond counting the research-relevant paths in `openapi.json`.
- `birdnet-migrate/src/provenance.rs` past its header; the importer's handling of effort for
  imported history.
- `docs/architecture/08-behavioral-analytics.md` past its headings.
- The admin backup/restore archive contents (RES-29).
- Whether `/api/v2/analytics/*` JSON carries the extension version (RES-31).
- Every standards claim marked FOLKLORE. None was checked against primary sources, and the
  implementing session must do that.
- No code was run. Every CONJECTURED item names the probe that would settle it.
