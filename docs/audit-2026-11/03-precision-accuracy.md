# Audit 03 — Detection precision and accuracy (single model, single source)

Read-only audit, 2026-11-22, of the signal path and decision logic from a recorded segment to a stored row. No code was
run (another agent owns the build). Every `path:line` below was opened. "FOLKLORE" marks a statement about BirdNET-Pi or
BirdNET-Analyzer that comes from general knowledge, not from this repository, and has to be checked against upstream source.

## How it works today (defaults quoted from code)

1. Decode: symphonia, all channels averaged to mono (`crates/birdnet-core/src/audio/decode.rs:241-247`); packets that fail to decode are skipped (`:228`); a truncated file keeps what was decoded (`:207-219`).
2. Resample (only when the source rate is not the model's): rubato `Async` sinc, 256 taps, BlackmanHarris2, cutoff factor 1.1 (`audio/resample.rs:94-101`); the fractional start-up delay is cancelled (`:102-126`). Stopband about -140 dB, measured in the module's tests (`:12-24`).
3. Model input is decided by the classifier's ONNX shape: V2.4 `[1,144000]` gives a 48 kHz waveform with 3.0 s windows; V3.0 gives 32 kHz probability output; a 160 000 input gives softmax (Perch) (`inference/model.rs:931-932`; `detection/daemon/run.rs:664-743`).
4. Windows: `chunk_duration_secs` 3.0 and `chunk_overlap_secs` 0.0 (`detection/pipeline.rs:147-148`); `OVERLAP` is allowed from 0 to 2.9 (`config/validate.rs:226`). A trailing remainder gets a window only if it is at least `MIN_TAIL_SECS` 1.5 s, and that window is zero-padded (`pipeline.rs:207, 281`). A window whose samples are all identical is not classified (`pipeline.rs:181-189`; `daemon/process.rs:188-191`).
5. Confidence: V2.4 uses `sigmoid(clamp(2 - SENSITIVITY, 0.5, 1.5) * logit)`. `SENSITIVITY` defaults to 1.25, a slope of 0.75 (`config.rs:68`; `model.rs:1310-1328`), and is validated to 0.5–1.5 (`validate.rs:225`). Outputs that are not finite score 0 (`model.rs:1311-1313`). The logit is not clipped (`model.rs:1423-1424`).
6. Per-window cut inside the model: the confidence must be at least the model floor. The floor is `CONFIDENCE` (0.70, `config.rs:43`), lowered to the lowest per-species threshold (`src/daemon/processor.rs:318-326`, `src/daemon/mod.rs:430-433`) or to the dynamic `min` (`src/daemon/mod.rs:280`). Then the top `top_n` = 10 classes are kept (`model.rs:79, 540`).
7. Whole-chunk filters, in order (`detection/mod.rs` `ChunkFilters::apply`):
   - privacy: on by default, `PRIVACY_RANK` 10, `PRIVACY_THRESHOLD` 0 (`src/cli.rs:733-758`). It masks a window and its neighbours (`privacy.rs:116-120, 218`).
   - noise: off by default, `NOISE_THRESHOLD` 0 with classes `["Dog"]` (`cli.rs:768`, `noise.rs` `DEFAULT_NOISE_CLASSES`).
   - corroboration: off by default (`cli.rs:837`). The levels require 20/30/50/70 % of a 6 s neighbourhood (`corroboration.rs:79, 139-147`).
8. Occurrence (range) filter: geomodel v2 with `SF_THRESH` 0.03 (`cli.rs:721`), using a 48-week year (`civil.rs:423-437`). With no coordinates it is off and every label passes (`species_filter.rs:501-503`). A species it rejects is dropped with no log line (`daemon/process.rs:534-536`).
9. Processor (`src/daemon/disposition.rs:175-207`, `processor.rs:668-930`):
   - Below a per-species threshold, the detection is quarantined.
   - Below the global threshold, it is dropped (counted as `confidence`).
   - Night filter: off by default; it quarantines at night (`daylight.rs`).
   - Duplicate interval: 0, off (`cli.rs:851`).
   - Dynamic thresholds: off by default, with trigger 0.80, min 0.25, 24 h, multipliers 1/0.75/0.5/0.25 and a 15 min cooldown (`dynamic_threshold/mod.rs:83-89, 134-139`).
10. Stored: the full-precision confidence and the window start truncated to whole seconds (`civil.rs:369`). Overlapping windows are separate rows (`docs/book/guides/parity.md:82`).
11. Parity evidence:
    - In CI: one recording, `tests/v24_matches_birdnet_pi.rs`. It compares against BirdNET-Pi's 3 asserted confidences with `TOL_CONF` 2e-3 and against all 10 windows' top-10 with `TOL_LOGIT` 0.15 (`:40-47`). The windows themselves are checked by `tests/chunking_matches_birdnet_pi.rs`.
    - The resampler is checked against a resampy golden (`tests/resampler_matches_resampy_parity.rs`).
    - The range filter is checked for every week (`tests/v24_range_filter_every_week_matches_birdnet_pi.rs`).
    - Off CI, the 120-clip study in `docs/book/guides/parity.md:75-110` agrees with BirdNET-Pi on 12/12 and 79/80 stored rows. That is agreement with BirdNET-Pi, not accuracy against ground truth.

## Findings

### ACC-01: The occurrence filter discards out-of-range birds without a trace; the "Unlikely here" quarantine is never written
- Severity: high — Kind: bug — NEW
- Evidence:
  - The detection is dropped with no log line and no row:
    `crates/birdnet-core/src/detection/daemon/process.rs:534` `if !allowed(&detection.scientific_name) { continue; }`
  - The quarantine reason exists and has a UI label: `crates/birdnet-db/src/sqlite/queries/quarantine.rs:30` `BelowSfThresh,` and `:84` `=> "Unlikely here this time of year"`.
  - A search for `BelowSfThresh` across `src` and `crates` finds only the enum, its string mapping and a round-trip test (`quarantine.rs:813`). Nothing writes it.
  - The screenshot fixture seeds `below_sf_thresh` rows (`crates/birdnet-web/examples/screenshot_server.rs`), so the UI shows a state the daemon cannot produce.
  - Status: VERIFIED-BY-READING.
- Impact:
  - Birder: a vagrant or out-of-season bird is the record most worth having, and it is exactly the bird a geomodel scores below 0.03. At any confidence (0.99 included) it disappears, with no row, no clip and no counter.
  - Researcher: there is no way to measure what the range filter removed, or to recover it.
- Proposed change: in `process_and_infer_filtered`, write the rejected detection as `QuarantineReason::BelowSfThresh` with `sf_probability` set. Thread the per-species probability from `passing_species`, so the score that rejected it is kept. Count `inc_detection_dropped("occurrence")`. Rate-limit or cap it per species per day if volume is a concern.
- Acceptance gate:
  - The fixture is a daemon run with a metadata model whose threshold excludes the species heard (`tests/species_filter_e2e.rs` already stages one). Assert that one `quarantine` row has `reason = 'below_sf_thresh'` and a non-null `sf_probability`.
  - Today the run writes zero rows, so the gate fails on today's code.
  - Counterpart: an admitted species writes no quarantine row.
- Effort M. Dependencies: ACC-02, which shares the counter.

### ACC-02: The biggest discard paths are invisible: privacy, noise, corroboration and occurrence are not counted
- Severity: high — Kind: gap — NEW
- Evidence:
  - The metric's docstring names `privacy` and `occurrence` as reasons. `crates/birdnet-web/src/metrics.rs:363` reads: "`reason` is a small closed vocabulary (`quality`, `privacy`, `occurrence`, `confidence`, `quarantine`)".
  - The only production callers are `src/daemon/processor.rs:704, 778, 838, 890, 923`, which emit `confidence`, `implausible_clock`, `implausible_hour`, `quarantine` and `duplicate`.
  - The chunk filters log only at debug level: `privacy.rs:142` "privacy filter: suppressing chunk with human voice".
  - The privacy rule is on by default and masks 32 % of windows on the 106-clip set (`docs/book/guides/parity.md:99`) and 37 % on V2.4 for the 91 clips (`:120`).
  - Status: VERIFIED-BY-READING. The masking rates are the repository's own measurement.
- Impact:
  - Birder: on a garden station near a road or a house, about a third of the analysed audio can be discarded for "human" with nothing on any page or metric.
  - Researcher: effort (minutes of audio actually judged) is unknowable, so detection rates per hour are biased by an unmeasured, time-varying mask. Speech is diurnal.
- Proposed change:
  - Return per-filter counts from `ChunkFilters::apply`: windows masked, detections removed by each filter, and the occurrence drops.
  - Export `birdnet_windows_masked_total{reason}` and `birdnet_detections_dropped_total{reason=privacy|noise|corroboration|occurrence}`.
  - Persist a daily per-reason tally, so a station page can say "N minutes masked for speech today".
- Acceptance gate:
  - Add a unit test on `ChunkFilters::apply` with one voiced chunk between two bird chunks that asserts the masked count is 3. It does not compile today, because no counts are returned.
  - Add a metrics exposition test after a daemon run with `PRIVACY_RANK` set that asserts `reason="privacy"` appears. Today it is absent.
- Effort M.

### ACC-03: No ground-truth precision or recall exists for the default configuration; "parity" is agreement with BirdNET-Pi only
- Severity: high — Kind: gap — NEW (the analogous Perch item is ALREADY-TRACKED as HANDOFF §5 item 41)
- Evidence:
  - `docs/book/guides/parity.md:75-110` compares rows with BirdNET-Pi's rows.
  - The only annotated-data study is `tools/perch-bar/README.md`, for Perch v2, run without the station's filters ("the study ran the model without the station's filters", HANDOFF §5 item 41).
  - No equivalent exists for V2.4 at `CONFIDENCE` 0.70 / `SENSITIVITY` 1.25 / `PRIVACY_RANK` 10 / `SF_THRESH` 0.03.
  - The corroboration premise ("a classifier artefact usually fires once", `corroboration.rs:14-18`) is likewise unmeasured. `crates/birdnet-core/tests/confirmation_premise.rs` checks only window arithmetic.
  - Status: VERIFIED-BY-READING (absence, by search of `tools/` and `tests/`).
- Impact:
  - Birder: no answer to "how often is a stored Blackbird wrong?".
  - Researcher: every filter's precision/recall trade-off (privacy rank, corroboration levels, dynamic thresholds, night filter) is a design argument, not a measurement.
- Proposed change:
  - Reuse the `tools/perch-bar` harness (same annotated Zenodo sets, the same whole-recording bootstrap) on V2.4 and V3.0.
  - Run each through the station's own filter chain (`ChunkFilters::apply` + occurrence filter at the recordings' coordinates and weeks).
  - Report per-level window recall and unmarked-report rate, with 95 % intervals. Include privacy on and off, corroboration off/balanced/strict, and dynamic thresholds off and on.
  - Publish the result as `tools/accuracy/` plus a manual page.
- Acceptance gate: a golden CSV in `tools/accuracy/golden/`, plus a Rust test that re-scores a small checked-in subset and matches the golden's counts. It fails today because the harness does not exist.
- Effort L. Dependencies: network access to Zenodo (the existing study fetched it).

### ACC-04: One low per-species threshold, or dynamic thresholds, changes the evidence every other species is judged on
- Severity: medium — Kind: footgun — NEW
- Evidence:
  - The model floor is the minimum over all per-species thresholds. `src/daemon/processor.rs:322-325`:
    `thresholds.values()...fold(model, |acc, &t| acc.min(...))`
  - The top-10 truncation happens inside the model, at that floor, before the occurrence filter. `model.rs:540` `detections.truncate(self.config.top_n);` runs before `process.rs:534`.
  - Corroboration counts every detection that reached it, whatever bar it will later fail. `corroboration.rs:237-244` builds `carriers` from all detections.
  - Status: VERIFIED-BY-READING (the mechanism). The size of the effect is CONJECTURED.
- Impact:
  - An operator who sets Tawny Owl to 0.30 lowers the window cut for 6 522 classes. Three things follow:
    - out-of-range or sub-threshold species compete for the 10 slots before the range filter removes them, and can push out an in-range species;
    - corroboration at `balanced`/`strict` becomes much easier for every species, because sub-threshold neighbours count;
    - with dynamic thresholds on, both happen at 0.25 for the whole station.
  - Researcher: one per-species edit changes the station-wide effective filter. Neither `analysis_runs` nor any setting records the window-cut value in use, and `Cutoff` records only the per-row bar.
- Proposed change:
  - Apply the occurrence filter (and per-species include/exclude) **before** `top_n`, by passing the allowed set into `predict_chunk`, or raise `top_n` when the floor is below the global threshold.
  - Make corroboration count only detections at or above their own effective threshold. Pass a per-detection bar, or filter carriers by `decide_disposition`.
- Acceptance gate:
  - A `process_and_infer_filtered` unit test with a stub classifier that outputs 12 out-of-range classes at 0.5 and one in-range class at 0.45, plus a per-species floor of 0.4. Assert that the in-range species is stored. Today it is truncated away.
  - A `corroborate` test where the neighbours carry the species only below the global threshold. Assert that `strict` drops it. Today it keeps it.
- Effort M.

### ACC-05: Dynamic thresholds can reinforce a systematic confuser down to 0.25 within half an hour
- Severity: medium — Kind: footgun — NEW (the feature itself is ALREADY-TRACKED as G-12 SHIPPED, `docs/FEATURE_GAP_ANALYSIS.md:312`; the runaway analysis is new)
- Evidence:
  - The first confirmation sets level 1, and each later one at least 15 minutes after the previous raises the level once (`dynamic_threshold/mod.rs:317-355`, `LEARN_COOLDOWN_MS` at `:83`).
  - The multipliers are `[1.0, 0.75, 0.50, 0.25]` with floor `min` 0.25 (`:89, :137`).
  - So three ≥0.80 detections at 0, 15 and 30 minutes take a species from 0.70 to 0.25. Every further ≥0.80 detection renews it for 24 h (`:340`).
  - The safety argument is that only detections surviving every gate confirm (`:24-31`). A repeating non-bird sound that the classifier maps to a locally plausible species (`phantoms.rs:5-11` describes exactly this) survives all of them.
  - Doc errors in the same file:
    - `:128` "this station's is 0.75" (the default is 0.70, `config.rs:43`);
    - `:130` "0.25 is the floor a 0.70 base reaches at level 3 anyway" (0.70 × 0.25 = 0.175, so the floor binds).
  - Status: VERIFIED-BY-READING.
- Impact:
  - Birder: a phantom becomes self-sustaining at 0.25, about 2.5× lower in logit than the bar it first cleared.
  - Researcher: the per-row `Cutoff` varies by hour with no record of why.
  - No precision evidence supports 0.25 for V2.4's slope-0.75 sigmoid. That bar is a logit of -1.46.
- Proposed change:
  - Raise the default `min` to at least 0.5, and require the confirmations to come from at least two distinct recordings or days.
  - Expose the level history per species (P-11 notes there is no `/events`).
  - Feed rejections from reviews back as a level reset.
  - Fix the two doc lines.
- Acceptance gate: a test in `dynamic_threshold/tests.rs` that feeds three 0.85 observations 15 minutes apart from the same `source_file` and asserts the effective threshold stays above 0.5. Today it is 0.25.
- Effort S–M.

### ACC-06: The default capture high-pass (120 Hz) and DC block change the model's input and are outside every parity measurement
- Severity: medium — Kind: gap — NEW
- Evidence:
  - The defaults are on: `crates/birdnet-core/src/audio/capture/types.rs:854-855` `high_pass: true, dc_removal: true`. The cutoffs are 120 Hz and 5 Hz (`:843, :849`).
  - The parity study bypasses capture. `docs/book/guides/parity.md:75` says "each clip dropped into the watch directory".
  - The field's doc still claims there is no cost: `types.rs:803` "Nothing `BirdNET` classifies lives down there". That is contradicted 33 lines later, at `:836-841`.
  - Status: VERIFIED-BY-READING. The effect on scores is CONJECTURED.
- Impact:
  - Birder: low-frequency vocalisers are attenuated before the model sees them: bittern boom about 150 Hz, pigeons and doves, grouse drumming, large owls. BirdNET-Pi records raw `arecord` output (FOLKLORE: no filtering in BirdNET-Pi's `record.sh`; check upstream).
  - Researcher: a station's rows are not BirdNET-Pi's for the same air, even with every parity setting matched.
- Proposed change:
  - Measure it: run the 120 parity clips through `EqProcessor` with the default chain before the pipeline, and diff the stored rows and the logits against the unfiltered run (`tools/parity/compare_rust.py` already diffs).
  - If the measurement shows a difference, either default the chain to empty for parity, or document the measured cost in `parity.md`. Fix the stale doc at `types.rs:803`.
- Acceptance gate: a Rust test that filters `tests/testdata/Pica_pica_30s.wav` through `EqChain::from_pipeline_flags(true, true)` and asserts the V2.4 top-10 per window is within `TOL_LOGIT` of the golden. If the chain matters, this fails today, which settles the question either way.
- Effort M.

### ACC-07: The "Model vs. human" panel presents a statistically invalid comparison as an accuracy signal
- Severity: medium — Kind: bug (statistical) — NEW
- Evidence:
  - The query compares `AVG(d.Confidence)` over **all** detections with `confirmed/(confirmed+rejected)` over at least 3 reviews (`crates/birdnet-db/src/sqlite/queries/analytics.rs:495-521`, `HAVING COUNT(*) >= 5 AND ...>= 3` at `:507-508`).
  - The copy draws a conclusion from it. `crates/birdnet-web/src/routes/admin/quality.rs:490-494`: "Species sorted by the gap, biggest first — these are the most likely overconfident false positives".
  - The species link goes to `/quarantine?species=` (`:822`).
  - Status: VERIFIED-BY-READING.
- Impact:
  - An uncalibrated score's mean is not comparable to a confirmation rate.
  - With 3 reviews, the 95 % Wilson interval on a rate of 2/3 is 0.21–0.94. Ranking by that gap mostly ranks noise.
  - Reviews are a non-random sample, because operators review what looks wrong.
  - An operator may exclude real species because of this panel.
- Proposed change:
  - Replace it with a per-species confirmation rate and its Wilson interval, labelled "share of *reviewed* detections confirmed (reviews are not a random sample)".
  - Hide rows with fewer than 20 verdicts, or show only the interval.
  - Drop the "overconfident false positives" sentence. Optionally add a "review a random sample" button that draws uniformly from stored rows, so the estimate becomes unbiased.
- Acceptance gate: a render test asserting that the panel prints an interval and that a species with 3 verdicts is not ranked. It fails today.
- Effort S–M.

### ACC-08: The per-species threshold suggester is too thin to trust and biased by what was reviewed
- Severity: medium — Kind: gap — NEW
- Evidence:
  - It needs only 5 verdicts and 2 per class: `crates/birdnet-db/src/thresholds.rs:85` `MIN_VERDICTS: usize = 5` and `:94` `MIN_PER_CLASS: usize = 2`.
  - It picks Youden's J with no interval (`:107-133`).
  - Every verdict that is not "confirmed" counts as a rejection: `analytics.rs:569` `confirmed: status == "confirmed"`.
  - Verdicts are binary only (`detection_reviews.rs:56-61`), and a rejection can carry the reason `'ambiguous'` (`detection_reviews.rs:845`), which then counts as a false positive.
  - The candidates come only from stored rows (at or above the bar at the time), so it can never suggest lowering below the global cut.
  - Status: VERIFIED-BY-READING.
- Impact:
  - Birder: a threshold fitted to 5 points, one click from being applied, can silence a species.
  - Researcher: the suggestion optimises a quantity the operator did not ask for, namely equal error weights on a non-random sample.
- Proposed change:
  - Require at least 20 verdicts and 5 per class.
  - Show the bootstrap interval of the suggested cut.
  - Offer a "precision at least P" target (the lowest t with a lower Wilson bound ≥ P) instead of J.
  - Add an "unsure" verdict that is excluded from both classes. Treat `ambiguous` as unsure.
- Acceptance gate: `suggest_threshold` on 3 confirmed and 2 rejected returns `None`. Today it returns `Some`, per `thresholds.rs` test `:290`.
- Effort S.

### ACC-09: An uncalibrated score is presented as certainty
- Severity: medium — Kind: polish (claims) — NEW
- Evidence:
  - The tiers: `crates/birdnet-web/src/routes/pages/confidence.rs:58` `SURE: u32 = 90` and `:92` `Self::High => "very sure"`. Its own comment (`:36-37`) admits "the model's score is not calibrated to a hit rate".
  - The onboarding card: `onboarding.rs:1762` "0.90 — only the IDs it is near-certain about. Very few false positives". Nothing measured that (see ACC-03).
  - The same presets are offered whatever the classifier. A V2.4 confidence of 0.9 at slope 0.75 (logit 2.93) and a V3.0 probability of 0.9 are different evidence (`model.rs:884-897`).
  - Status: VERIFIED-BY-READING.
- Impact: a birder reads "very sure 92 %" as a 92 % chance of being right. For BirdNET, per-species precision at a fixed score varies widely (FOLKLORE: published BirdNET evaluations report this; check before citing).
- Proposed change:
  - Word the tiers relative to the bar ("well above your cut-off"), not as certainty.
  - Drop "very few false positives" until ACC-03 measures it.
  - Show "score" rather than "%" on rows, or footnote that it is not a probability on V2.4.
- Acceptance gate: a template/string test that no user-facing surface contains "near-certain", "Very few false positives" or "very sure". It fails today.
- Effort S.

### ACC-10: Corroboration cannot see across a segment boundary, so edge windows are held to a one-sided test
- Severity: medium — Kind: gap — NEW
- Evidence:
  - The neighbourhoods are built from this file's `starts` only: `corroboration.rs:251-257` `starts.iter()...filter(|(_, s)| (**s - here).abs() <= half)`. The edge requirement is capped at the edge's size (`:265-275`).
  - So at `balanced` with overlap 0, a 15 s segment's first and last windows (2 of 5) need their single in-file neighbour.
  - A 4–6 s song that straddles a segment boundary appears once in each file and is dropped from both.
  - The CLI help is also wrong. `src/cli.rs:828-829`: "This does nothing without `--overlap`… a six-second neighbourhood holds two 3-second windows". The module says three, and that `balanced` and `strict` bite at 0 overlap (`corroboration.rs:43-55`).
  - Status: VERIFIED-BY-READING for the mechanism. The recall cost is CONJECTURED.
- Impact: when corroboration is enabled (it is off by default), recall is systematically lower in the first and last 3 s of every segment, which is 40 % of windows at 15 s segments. The rare single-call bird is lost by design.
- Proposed change:
  - Carry the previous segment's last `REFERENCE_SPAN/2` of predictions per source, keyed by `rtsp_id`, so the boundary is judged on both sides. Equivalently, defer a segment's last window until the next segment arrives.
  - Fix the CLI help.
- Acceptance gate: a `corroborate` test over two consecutive files, with a species only in file A's last window and file B's first. At `balanced`, assert both are kept. Today both are dropped.
- Effort M.

### ACC-11: The night filter's genus list omits well-known nocturnal singers
- Severity: medium — Kind: gap — NEW
- Evidence: a search of `crates/birdnet-core/src/detection/nocturnal.rs` for these genera finds 0 hits: `luscinia` (nightingales), `locustella`, `acrocephalus`, `mimus`, `coturnix`, `numenius`, `anser`, `branta`, `gavia`, `charadrius`, `tringa`, `turdus`, `catharus`. `scolopax`, `burhinus` and `botaurus` are present. The filter is off by default (`src/daemon/config.rs:97`), quarantines rather than drops (`nocturnal.rs:28-33`), and the module says nocturnal flight-call stations should not use it (`:37-42`). The doc at `src/daemon/daylight.rs:68` ("non-positive `margin_mins` disables it") contradicts `:94` (`margin_mins >= 0`). Status: VERIFIED-BY-READING.
- Impact: on a station that turns it on, a nightingale singing at 01:00 is quarantined. That is recoverable but invisible unless someone reviews the queue.
- Proposed change: add these genera with a comment per block. Better, derive "nocturnal" from a data table (for example eBird or AVONET activity fields) rather than a hand list. Fix the margin doc.
- Acceptance gate: assert that `night_verdict("Luscinia megarhynchos", &[])` is `Nocturnal`. It fails today.
- Effort S.

### ACC-12: Nothing checks the model's output on aarch64, the target platform
- Severity: medium — Kind: gap — ALREADY-TRACKED (`docs/architecture/12-risks.md:53-55`, "should be spot-checked on a real Pi")
- Evidence:
  - The V2.4 reference test is a root-crate integration test (`tests/v24_matches_birdnet_pi.rs`), and CI provides its model only in the x86_64 job (`.github/workflows/ci.yml:400-403`).
  - The `test-aarch64` job runs only `birdnet-core`, `birdnet-scheduler` and `birdnet-timeseries`, and explicitly excludes "the binary crate's integration tests, which … need the model" (`ci.yml:647-666`).
  - Status: VERIFIED-BY-READING.
- Impact: ONNX Runtime uses different MLAS kernels on ARM, so window-for-window agreement on a Pi is unverified. Near-threshold rows (compare the 0.6931/0.7025 robin, `parity.md:82`) could flip.
- Proposed change: fetch the V2.4 model in `test-aarch64` and run `--test v24_matches_birdnet_pi` with `BIRDNET_REQUIRE_V24_MODEL=1`.
- Acceptance gate: the CI job itself. It fails today only by absence, so in the same pull request, temporarily set `TOL_LOGIT` to 0 on the job, check it goes red, then restore it.
- Effort S.

### ACC-13: Decode errors silently drop packets, shifting every later sample earlier
- Severity: low — Kind: bug — NEW
- Evidence: `crates/birdnet-core/src/audio/decode.rs:228` `Err(SymphoniaError::DecodeError(_)) => continue,` drops the packet with no log line, no count and no zero-fill. Status: VERIFIED-BY-READING. The frequency of the error is CONJECTURED: it is rare for WAV from `arecord`, and more likely for uploaded MP3 or FLAC.
- Impact: windows after the bad packet start later in real time than their `start_secs` says. Timestamps, clip extraction and corroboration neighbourhoods are all offset by the packet length.
- Proposed change: substitute zeros for the packet's frame count when it is known (the packet duration). Otherwise, count the dropped frames and warn once per file.
- Acceptance gate: a test with a WAV fixture that has one corrupted mid-file block. Assert the decoded length equals the header's frame count. Today it is shorter.
- Effort S.

### ACC-14: No warning when a source cannot carry the band the model listens to
- Severity: low — Kind: gap — NEW
- Evidence:
  - `resample.rs:83-92` upsamples any rate without comment.
  - A search for a source-rate check in `crates/birdnet-core/src` and `src` (`sample_rate <`, "Nyquist", "band-limited") found none in the detection path.
  - Status: VERIFIED-BY-READING (absence).
- Impact: an 8 or 16 kHz RTSP camera stream gives the model nothing above 4 or 8 kHz. High-pitched species (Goldcrest, many warblers) are silently never detected. The station looks healthy.
- Proposed change: warn once per source at startup and in `--doctor` when `source_rate / 2 < 12 kHz`, and record the source rate on `analysis_runs`.
- Acceptance gate: a unit test of a `band_warning(source_rate, model_rate)` helper. It fails today because the helper does not exist.
- Effort S.

### ACC-15: Sigmoid without logit clipping; an unverified claim about BirdNET-Analyzer
- Severity: low — Kind: polish/parity — NEW
- Evidence:
  - `model.rs:1423-1424` computes `1.0 / (1.0 + (-x).exp())` with no clip.
  - BirdNET-Pi and BirdNET-Analyzer clip the logit (FOLKLORE: ±15 in BirdNET-Pi `models.py`, ±20 in Analyzer; check upstream).
  - The doc at `model.rs:1291-1296` says the mapping is "BirdNET-Analyzer's before it". FOLKLORE: newer Analyzer versions reparametrised sensitivity as a bias shift, so this needs checking against the Analyzer release a user would compare with.
  - Status: VERIFIED-BY-READING for this repo; the upstream side is FOLKLORE.
- Impact: none on the 0.7 decision. The displayed "100 %" differs from 0.99999 for |logit| above about 20. It misleads anyone comparing against Analyzer output.
- Proposed change: clip to BirdNET-Pi's bound after verifying it in `tools/parity/ref_pi.py`'s checked-out source, and correct the doc to name the Analyzer version it matches.
- Acceptance gate: `compute_confidence(1e4, 1.25, false)` equals BirdNET-Pi's clipped value. Today it is 1.0.
- Effort S.

### ACC-16: Stale documentation misdescribes the signal path
- Severity: low — Kind: footgun — NEW
- Evidence, all VERIFIED-BY-READING:
  - `detection/pipeline.rs:135` says "V2.4 models expect a mel spectrogram". `run.rs:678-683` says V2.4 is a waveform model.
  - `docs/architecture/05-audio-pipeline.md:31` shows the mel spectrogram on the inference path. `:106-108` says "BirdNET models were trained on librosa-generated features". `:136-138` claims unit tests compare against librosa references "Matching tolerance is `1e-4`". No librosa golden exists: a search for "librosa" in `audio/spectrogram*` finds comments only.
  - `audio/quality/mod.rs:3` says "Evaluates each audio chunk before ML inference to filter out low-quality recordings", and `corroboration.rs:7` mentions "the SNR gate drops audio too poor to judge". `assess_quality` is called only from `src/integrations/acoustic_health.rs:167`, which is not the detection path.
  - `PipelineConfig::default` still carries `confidence_threshold: 0.25` (`pipeline.rs:150`), an unused third value pinned by a test (`:503`). That is the trap the `ModelConfig` comment (`model.rs:70-77`) describes.
  - The dynamic-threshold doc numbers (ACC-05) and the night-filter margin doc (ACC-11).
- Impact: the next reader "fixes" the wrong layer. A researcher reading the architecture doc believes features are computed in Rust and validated against librosa.
- Proposed change: correct each item; delete `PipelineConfig::confidence_threshold`.
- Acceptance gate: a doc-drift test that greps for the four phrases. It fails today.
- Effort S.

### ACC-17: Detection time is truncated to the whole second
- Severity: low — Kind: gap — NEW
- Evidence: `civil.rs:369` `offset_secs.trunc() as i64`. At `OVERLAP=1.5` the window starts 1.5, 4.5 and 10.5 s are stored as 1, 4 and 10. At a step below 1 s (`OVERLAP` above 2.0), two windows of one species share a `Time`. Reviews are keyed on `(Date, Time, Sci_Name, File_Name)` (`analytics.rs:556-558`), and the module already records detached verdicts for exactly that ambiguity. Status: VERIFIED-BY-READING.
- Impact: researcher only. Time alignment is off by up to 1 s against other sensors, and verdicts can attach to the wrong window at high overlap.
- Proposed change: store `chunk_offset_secs` at millisecond precision (it exists per the `model.rs:445-447` comment) and expose it in the API and exports.
- Acceptance gate: an API test asserting `/api/v2/detections` returns a sub-second offset. It fails today.
- Effort S–M.

### ACC-18: Non-bird classes can be stored as "species"
- Severity: low — Kind: gap — NEW
- Evidence: no code excludes `Dog`, `Engine`, `Siren`, `Gun`, `Fireworks`, `Power tools`, `Environmental` or `Noise` from storage. A search for `non_bird|is_bird` in `crates/birdnet-core/src` and `src/daemon` finds nothing. Only the opt-in noise filter (`noise.rs`) and the human rule touch them. With no coordinates, every label passes (`species_filter.rs:501-503`). Status: CONJECTURED. Whether geomodel v2 scores these labels at or above 0.03 is settled by `occurrence_scores` for any location on `birdnet-v24-range-v2.onnx`.
- Impact: "Engine" appears in a life list and in species richness. BirdNET-Pi parity is likely (FOLKLORE).
- Proposed change: add a `non_bird` flag on labels (V2.4 non-bird labels repeat the name, as in `Dog_Dog`), and either drop them from the `detections` table or store them in a separate `events` table that the noise filter reads.
- Acceptance gate: a daemon run with a stub classifier that emits `Dog_Dog` at 0.95 asserts no `detections` row. It fails today if the probe confirms the conjecture.
- Effort S–M.

### ACC-19: Overlapping windows are separate rows, and counts scale with `OVERLAP`
- Severity: low — Kind: opportunity — ALREADY-TRACKED (documented as parity: `docs/book/guides/parity.md:82`, "overlapping windows are not merged here, as they are not there"; the duplicate filter is off by default, `cli.rs:851`)
- Evidence: one magpie recording stores 3, 5 and 11 rows at `OVERLAP` 0, 1 and 2 (`parity.md:82`).
- Impact: every per-hour count and richness-effort comparison across stations with different overlaps is not comparable.
- Proposed change: an opt-in event layer (merge same-species windows within the step, keep the maximum confidence and the span) as a view, leaving rows untouched.
- Acceptance gate: a view test where 11 magpie rows at overlap 2 give 3 events.
- Effort M.

### ACC-20: The V3.0 window length and its privacy default rest on thin evidence
- Severity: low — Kind: gap — the privacy part is ALREADY-TRACKED (HANDOFF §5 item 5 / G-13); the window part is NEW
- Evidence:
  - The 4.5 s window was chosen on one recording. `model.rs:640-651`: "on the bundled `Pica_pica_30s.wav` fixture the best-chunk Eurasian Magpie confidence climbs from 0.52 … to 0.72".
  - The privacy rule masks 0 of 31 speech windows on V3.0 (`parity.md:120`).
  - Status: VERIFIED-BY-READING.
- Impact: only for stations that opt into V3.0. Window length changes both precision and recall, judged on one species and one file.
- Proposed change: include window lengths 3.0, 4.5 and 5.0 s in the ACC-03 study.
- Acceptance gate: part of ACC-03's golden.
- Effort folded into ACC-03.


### ACC-21: The manual's review-queue table said quarantine holds only per-species threshold misses
- Severity: low — Kind: stale documentation — NEW, added 2026-11-23 by the session that implemented WP-03 (not
  part of the audit of `9d8bb1c`)
- Evidence: `docs/book/guide/reviews.md`, "Reviews vs. quarantine", said the queue holds "Detections that *failed* a
  stricter per-species threshold", while the processor already quarantined a day bird at night
  (`implausible_hour`) and a recording made before the clock was set (`implausible_clock`), and ACC-01 adds out of
  range. Status: VERIFIED-BY-READING against `src/daemon/processor.rs`.
- Change: the cell says the queue holds detections held back, each with its reason, and a paragraph below the table
  names the five reasons (`54859c7` and the WP-03 follow-up commit).
## Numerical notes (no finding)
- Mono mixdown, resampling and confidence are f32. The resampler's delay arithmetic is in integers and f64, and its `wanted` length uses integer `div_ceil` (`resample.rs:113-125, 136-137`).
- Softmax normalisation is accumulated in f64 (`model.rs:944-969`).
- Window starts are computed as `pos as f32 / rate` (`pipeline.rs:284`), exact for the first 2^24 samples (5.8 min at 48 kHz). The error stays below a millisecond for multi-hour uploads.
- The mel spectrogram (`audio/spectrogram*`) is not on the inference path for any shipped model (`run.rs:729-733`). Only displays use it.
- FP32 against BirdNET-Pi's FP16 is measured: a median logit gap of 0.009, and 0.37 at the 99th percentile (`parity.md:107`). Nothing in the repository measures INT8 accuracy. INT8 is mentioned only as a performance measurement (`model.rs:1383`), and no INT8 model is offered in the catalogue.

## Not examined
- Capture-side behaviour (segment timing against filename time, AGC, the `EqProcessor` against ffmpeg `-af` equivalence) and multi-source capture.
- Multi-model merge, routing and the Perch softmax bar, beyond what touches the single-model floor: other agents own these.
- `audio/extraction/*` (clip cutting), `soundlevel`, `biquad.rs` coefficients, and `labels.rs`/`vocabulary.rs` name alignment between the classifier and the geomodel.
- `src/daemon/duplicate.rs` and the per-species minimum intervals.
- `tools/parity/*.py`: read only through `parity.md`. Their numbers were not re-run, and the golden JSON was not opened beyond file names.
- The UI rendering of quarantine and review pages beyond the two panels cited.
- Whether the geomodel admits non-bird labels (ACC-18 probe) and the real-audio size of ACC-04 and ACC-10. Each needs a run, which this audit could not perform.
