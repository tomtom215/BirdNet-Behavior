# Audit 02 — the multi-model (multi-classifier) inference pipeline

Audited 2026-11-22 at `9d8bb1c` (main). Read-only: no code was changed, no cargo command was run, no model was
executed. Every claim below cites the line it was read from. "VERIFIED-BY-READING" means the code path was followed
end to end by reading; it is not a run. "CONJECTURED" names the probe that would settle it.

## How it works today

1. Up to three classifiers (`MAX_MODELS = 3`, `crates/birdnet-core/src/inference/registry.rs:53`): the primary from
   `MODEL_PATH`/`LABELS_PATH`, plus `MODEL_2_*` / `MODEL_3_*` read from `BIRDNET_*` env, then the config file
   (`src/helpers/models.rs:150-168`). These keys are not bridged to the settings table (`src/helpers/settings_overlay.rs:70,75`
   bridge only `CONFIDENCE`/`SENSITIVITY`), so the settings-table-overrides-config trap does not apply to them.
2. An extra classifier is admitted only if all classifiers (file size + 256 MiB each) fit in half the memory ceiling
   (`src/helpers/models.rs:115-126,196-256`); unknown ceiling admits none. The notes are logged at INFO
   (`src/daemon/mod.rs:418-420`).
3. `ClassifierRegistry::load` loads every spec and **refuses the whole set** on any load error, a duplicate id, an unknown
   route target, or a sample rate differing from the primary's (`registry.rs:253-366`). That error stops the detection
   daemon; the web UI keeps serving (`crates/birdnet-core/src/detection/daemon/run.rs:96-101`, `src/daemon/mod.rs:589-592`).
4. Input contract is inferred from the ONNX input shape: `[_,144000]` → 48 kHz, sigmoid(slope(SENSITIVITY)) (V2.4);
   `[_,96000]` or dynamic → 32 kHz, probability (V3.0); `[_,160000]` → softmax, rate 48 kHz unless `MODEL_n_SAMPLE_RATE`
   declares 32 kHz (Perch v2) (`inference/model.rs:929-935,1230-1240`, `registry.rs:298-303`). Audio is decoded once,
   mixed to mono by averaging (`audio/decode.rs:241-247`), resampled once to the primary's rate (`run.rs:663-678`).
5. With several classifiers, chunks are cut to the longest window and stepped by the shortest (`run.rs:704-743`,
   `detection/pipeline.rs:96-129`); each classifier reads its own prefix of the shared chunk (`model.rs:621-623`).
6. Classifiers run **sequentially** on the single daemon thread, each with 2 intra-op threads (`process.rs:205-262`,
   `model.rs:79-80`, `src/daemon/config.rs:245-250`). Routing is per source id parsed from the segment filename
   (`run.rs:754-761`); an unrouted source gets the primary only (`registry.rs:454-468`).
7. Per chunk, `merge_verdicts` makes a union by **exact scientific name**: winner = (cleared own bar, confidence)
   lexicographic, `model_id` = winner, `agreeing_models` = count (`detection/merge.rs:85-177`). Per-classifier scores go to
   `detection_model_scores` when ≥2 classifiers listened (`process.rs:297-337`).
8. Whole-chunk filters (privacy: max human score / min human rank across classifiers; noise; corroboration) then the
   occurrence filter: geomodel for primary-vocabulary names, operator lists only for names only a secondary knows
   (`process.rs:466-520`).
9. The processor holds each row to its winner's `MODEL_n_THRESHOLD` else the station's, but per-species and dynamic
   thresholds override per species regardless of model (`src/daemon/processor.rs:681-699`, `src/daemon/disposition.rs:175-207`).
10. Provenance: `analysis_runs` hashes the **primary** model/labels only (`src/daemon/run.rs:92-117`); rows carry
    `model_id`, `model_agreement` (migration 50). Health per classifier (fails after 10 bad windows,
    `inference/health.rs:30`) feeds metrics and the station-health notifier (`src/integrations/station_health.rs:915-932`).
11. Catalogue (`crates/birdnet-integrations/src/model_catalog.rs:173-275`): `birdnet-v24` FP32 (default primary,
    `:68`), `birdnet-v3` preview3 FP32, `perch-v2` (third-party ONNX conversion), `birdnet-v24-range-v2`; sha256 pinned,
    streamed, free-space checked. No FP16/INT8, no V3 geomodel, no bat model, no licence field. No hot-swap: a restart
    is needed (`docs/architecture/06-ml-inference.md:139-143`).

**Consequence of 3 + 4 + 11:** the only multi-classifier combination the catalogue can produce is BirdNET+ V3.0 primary
with Perch v2 (both 32 kHz). The default V2.4 primary (48 kHz) cannot run beside Perch.

## Findings

### MMP-01: The documented "install Perch as a second classifier" recipe stops all detection on a default (V2.4) station
- Severity: high — Kind: bug — NEW
- Evidence: the default primary is V2.4 at 48 kHz (`model_catalog.rs:68`, `:240`). The rate check refuses the set:
  `registry.rs:328-335` — `if m.spec.sample_rate != primary_spec.sample_rate { return Err(RegistryError::IncompatibleInput ...`.
  The error is fatal for the daemon: `run.rs:96-101` `.map_err(|e| DaemonError::Model(e.to_string()))?;` and
  `src/daemon/mod.rs:589-591` `tracing::error!(error = %e, "failed to start detection daemon"); None`. Yet the
  operator is told to do exactly this: `--install-model perch-v2` prints `BIRDNET_MODEL_2_PATH=… BIRDNET_MODEL_2_SAMPLE_RATE=32000`
  with no condition on the primary (`src/helpers/models.rs:454-464`); the manual's table says Perch "runs **beside** the
  primary" (`docs/book/admin/classifiers.md:77`). The same all-or-nothing policy also applies to a corrupt extra model
  file or an unreadable extra labels file (`registry.rs:277-297`; `plan` only screens files whose size cannot be read,
  `src/helpers/models.rs:216-223`). Status: VERIFIED-BY-READING.
- Impact: birder — follows the manual, restarts, and the station records nothing (web UI still up) until they find an
  ERROR line or the health alert. Researcher — a gap in the time series caused by an optional add-on.
- Proposed change: (a) degrade instead of refuse for *extra* classifiers: drop an extra spec that fails to load or
  wants another rate, keep the primary, log at WARN/ERROR and surface it on the Models board and in station health;
  keep refusal only for the primary. (b) `--install-model perch-v2` and the catalogue/manual must say "requires a 32 kHz
  primary (birdnet-v3); not usable beside birdnet-v24" and refuse to print MODEL_2 instructions when the configured
  primary is 48 kHz.
- Acceptance gate: a `run_daemon` test with the tiny V2.4 fixture as primary and a `sample_rate: Some(32_000)` extra
  spec must return `Ok` and produce primary detections; today it returns `Err(DaemonError::Model)`. Counterpart: a
  primary that fails to load still refuses.
- Effort M; depends on a maintainer decision to change the "refuse at startup" invariant for extras.

### MMP-02: `--doctor` does not check extra classifiers or routes, although code and manual say it does
- Severity: medium — Kind: gap — NEW
- Evidence: `registry.rs:19-21` "must stop the station before it starts, where the journal and `--doctor` will show it";
  `docs/book/admin/classifiers.md:176` repeats it. `src/doctor/model.rs:12-20` reads only `MODEL_PATH`/`LABELS_PATH`; no
  file under `src/doctor/` references `MODEL_2`, `MODEL_ROUTES` or `plan` (grep). Memory refusals of an extra classifier
  are logged at INFO only (`src/daemon/mod.rs:418-420`). Status: VERIFIED-BY-READING.
- Impact: birder — a mistyped route, a rate mismatch (MMP-01) or a memory refusal is not visible before restart, and the
  last is not visible after either, except as an INFO line. Researcher — cannot tell from the Models board that a
  configured classifier was never loaded.
- Proposed change: have the doctor run `helpers::models::plan` and the registry's non-loading checks (rate compatibility
  from catalogue/declared rate, routes, labels width); log plan refusals at WARN; expose planned-but-not-loaded
  classifiers to the Models board.
- Acceptance gate: doctor test with `MODEL_ROUTES=pond:pecrh` (and one with V2.4 primary + `MODEL_2_SAMPLE_RATE=32000`)
  must produce a failing check; today the doctor passes both.
- Effort S–M.

### MMP-03: Species only a secondary classifier knows are never location-filtered, and the include list can silently stop binding for them
- Severity: high — Kind: bug — NEW
- Evidence: `process.rs:499-520` — names not in the primary's labels go to `allowed_elsewhere`, built from
  `species_filter.allowed_by_lists(m.model.labels())` (lists only, no geomodel). Perch has 3 679 species and 200 sound
  classes not in V3.0 (`docs/book/reference/taxonomy-overlap.md`, table at top). `apply_lists` ignores an include list
  that resolves to nothing **in that vocabulary** (`species_filter.rs:684-691`), so an include list naming only species
  Perch lacks admits every Perch-only class; and the warning there fires on every file for every secondary
  (`allowed_by_lists` is recomputed per file, `process.rs:507-513`). No test covers `allowed_elsewhere` (grep found none).
  Status: VERIFIED-BY-READING; the include-list case needs a probe (a filter with include `["Turdus merula"]`
  against a label set lacking it, assert the result is not all labels).
- Impact: birder — Perch-only extralimital birds, frogs, insects and mammals appear on a station whose range filter is
  on. Researcher — occurrence data from the secondary is unfiltered while the primary's is filtered: two different
  sampling filters in one table, not marked.
- Proposed change: drop (or quarantine) secondary-only names by default unless whitelisted; or map them through the
  geomodel's own label vocabulary (the V3 geomodel covers 12 012 species, `species_filter.rs:344-346`) via the alignment
  already built; treat an include list as binding in every vocabulary (empty intersection → nothing allowed for that
  vocabulary, warned once); cache `allowed_elsewhere` with the list TTL.
- Acceptance gate: unit test on `process_and_infer_filtered` with a geomodel that rejects species X and a secondary
  whose labels contain only X → X must not be emitted; fails today.
- Effort M.

### MMP-04: Perch's 200 sound classes ("Speech", "Engine", "Crow", "Chirp_and_tweet", …) are recorded, notified and posted as species
- Severity: high — Kind: bug — NEW (the catalogue note mentions the classes, `model_catalog.rs:214-215`, but nothing filters them)
- Evidence: `crates/birdnet-core/data/labels/perch-v2.csv` rows such as `Speech,Speech` (line 12760),
  `Conversation,Conversation` (3362), `Dog,Dog` (4281), `Engine,Engine` (4668), `Crow` and `Gull_and_seagull` (sound
  classes, not taxa). `predict_chunk` turns any label over the bar into a `Detection` (`model.rs:475-496`); nothing
  downstream drops non-taxon classes (grep for a non-species filter found none); they are not in the primary's vocabulary,
  so the geomodel never sees them (MMP-03). The processor posts every admitted row to BirdWeather with the
  station's location (`src/daemon/processor.rs:1519-1556`). Status: VERIFIED-BY-READING for the path; that real audio
  produces such rows above a 0.35 Perch bar is CONJECTURED (probe: run `perch-v2.onnx` through the daemon on a traffic
  and a speech recording with `MODEL_2_THRESHOLD=0.35`, count rows whose `Sci_Name` has no space).
- Impact: birder — "Engine" or "Speech" in the species list, notifications and BirdWeather. Researcher — non-taxa in
  occurrence exports; "Crow" (sound class) vs *Corvus* rows for one call.
- Proposed change: classify labels at load (binomial vs sound class; Perch's 200 are enumerable) and never emit
  non-taxon classes as detections; route the relevant ones to the noise/privacy filters instead.
- Acceptance gate: `predict_chunk` on the tiny Perch fixture (`model.rs:1620`) with a labels file whose top class is
  `Engine` must emit no detection for it; today it does.
- Effort S–M.

### MMP-05: The privacy filter does not recognise Perch's speech classes
- Severity: high — Kind: bug — NEW
- Evidence: `names_a_human` is a substring match on "human" (`detection/privacy.rs:211-215`). In Perch's labels only
  `Human_voice` and `Human_group_actions` contain it; `Speech` (12760), `Conversation` (3362),
  `Male_speech_and_man_speaking` (7604), `Female_speech_and_woman_speaking` (5117), `Child_speech_and_kid_speaking`
  (2620), `Whispering` (14459), `Shout`, `Yell`, `Laughter`, `Singing` and `Homo sapiens,Homo sapiens` (6127) do not.
  `human_indices` is built from that matcher (`model.rs:1402-1408`). The startup warning about missing human labels
  checks the primary only (`run.rs:250-255`). On a source routed to Perch alone (`MODEL_ROUTES=pond:perch`) the privacy
  decision rests on Perch. The score rule compares the max of a softmax share and a V3 probability against one
  `PRIVACY_THRESHOLD` (`process.rs:242`, `privacy.rs:117`). Status: VERIFIED-BY-READING for the matcher; whether
  `Human_voice` still ranks in Perch's top 10 on real speech (which would rescue the rank rule) is CONJECTURED — probe:
  run Perch on a speech clip and print `human_rank`.
- Impact: birder — recordings of conversations kept, and with MMP-04 a "Speech" row may be posted publicly.
  Researcher — none directly; this is a privacy defect.
- Proposed change: per-vocabulary human class list (exact names, enumerated for Perch's FSD50K classes and
  *Homo sapiens*), held by a test against `perch-v2.csv`; warn at startup for each routed classifier without human
  classes.
- Acceptance gate: test that every `perch-v2.csv` row in an explicit speech set is a human index; fails today for `Speech`.
- Effort S.

### MMP-06: One confidence bar is applied to scores on different scales, and adding a classifier can quarantine a detection the primary alone accepts
- Severity: medium — Kind: bug — NEW
- Evidence: merge winner across scales: `merge.rs:141-142`
  `let better = (cleared, detection.confidence) > (b.cleared, b.detection.confidence);`. Per-species and dynamic
  thresholds then judge the winner's number whatever its scale: `disposition.rs:191-197`
  `if let Some(&threshold) = per_species_thresholds.get(sci_name) { … if f64::from(confidence) < effective { Quarantine`;
  dynamic adjustment is keyed by species only (`dynamic_threshold/mod.rs:250,289`). BirdWeather's floor is one number
  (`birdweather.rs:548-549`). Scenario: species threshold 0.5; V3 (bar 0.7) scores 0.65; Perch (bar 0.35) scores 0.40 →
  Perch "cleared", wins, 0.40 < 0.5 → quarantined; V3 alone → 0.65 ≥ 0.5 → accepted. Manual concedes scales differ
  (`classifiers.md:136`). Status: VERIFIED-BY-READING.
- Impact: birder — a per-species threshold set while running one model behaves differently, sometimes stricter, after
  adding Perch; BirdWeather receives Perch shares judged against a sigmoid-era floor. Researcher — the `Confidence`
  column mixes three scales; dynamic-threshold learning mixes them too.
- Proposed change: store per-species/dynamic thresholds per `(model_id, species)` or apply them only to the primary's
  scale; when the winner fails a per-species bar, re-judge using any other classifier's own score that clears it
  (the scores are already attached, `process.rs:334-336`).
- Acceptance gate: processor-level test with the scenario above must store an accepted row; today a quarantine.
- Effort M.

### MMP-07: With dynamic thresholds on, a classifier's "own bar" in the merge is the dynamic floor, not the station's threshold
- Severity: medium — Kind: bug — NEW
- Evidence: the model config handed to the registry carries `model_confidence = dynamic_config.model_floor(confidence)`
  (`src/daemon/mod.rs:279-280,483`); `registry.rs:287-291` sets `own_threshold = config.confidence_threshold` for a spec
  without `MODEL_n_THRESHOLD`. The processor's floor for that model is the station's `global_confidence`
  (`processor.rs:491-496,681-682`). So the merge, the Models board (`ModelInfo.bar`, `registry.rs:419`) and the
  "below its own bar" answer state use the floor, while acceptance uses the station bar: a no-threshold primary scoring
  between floor and station bar is "cleared", out-ranks a secondary that cleared its real bar, and the row is then
  dropped (`DropBelowGlobal`) unless the species has an adjustment. Status: VERIFIED-BY-READING.
- Impact: birder — with Perch and dynamic thresholds both on, some Perch-confirmed detections vanish and the Models
  board shows a bar that is not the one applied. Researcher — `detection_model_scores` states are mis-graded.
- Proposed change: compute `own_threshold` from the station's undecorated `confidence` (or the plan's threshold), not
  from the floored model config.
- Acceptance gate: registry/daemon test with dynamic thresholds enabled (min 0.3, station 0.7) and no
  `MODEL_THRESHOLD`: `registry.primary().own_threshold` must be 0.7; today 0.3.
- Effort S.

### MMP-08: Exports and run records attribute every row to the primary model's file and checksum
- Severity: medium — Kind: gap — NEW
- Evidence: the run hashes `manifest.model_path` / `labels_path` only (`src/daemon/run.rs:92-110`); the CSV joins
  `Model_Name`/`Model_SHA256` from the run (`crates/birdnet-web/src/routes/export/csv.rs:209-214,271-272`), so a row
  with `Model_Id=perch` carries the V3 file's sha. BirdWeather's `algorithm` is the run's (`processor.rs:1555`,
  `src/daemon/run.rs:150-152`). The row's `Sens` is written for Perch rows although sensitivity does not apply
  (`classifiers.md:159`). Status: VERIFIED-BY-READING.
- Impact: researcher — cannot prove which Perch build or label file produced a row; contradictory columns in one row.
  Birder — none visible.
- Proposed change: an `analysis_run_models` table (run_id, model_id, sha256, labels_sha256, rate, window, output scale,
  own bar); export columns resolved by `(run_id, model_id)`.
- Acceptance gate: export test with a run of two classifiers and a row with `model_id` = second: `Model_SHA256` must be
  the second file's; fails today.
- Effort M.

### MMP-09: Exact-scientific-name merge splits renamed species into two rows with no agreement
- Severity: medium — Kind: gap — ALREADY-TRACKED (HANDOFF §5 item 40; `taxonomy-overlap.md`)
- Evidence: `merge.rs:106` keys on `detection.scientific_name`; the primary/secondary split in `process.rs:503-520`
  and corroboration (`corroboration.rs:237-244`) key on the same string. 121 V3/Perch renames.
  Status: VERIFIED-BY-READING.
- Impact: duplicate rows for one call; renamed species also miss the geomodel (MMP-03) when only Perch names them.
- Proposed change: as HANDOFF item 40 says — decide the stored name, then apply the verified alias table at load time.
- Acceptance gate: merge test with an alias pair must yield one row with agreement 2.
- Effort M–L; maintainer decision.

### MMP-10: A merged row's common name follows whichever classifier won
- Severity: low — Kind: footgun — NEW
- Evidence: `merge.rs:144` `b.detection = detection.clone();` (common name included). Perch's names come from V2.4/IOC
  (`data/labels/README.md`), V3's from its own CC BY-SA file. Status: CONJECTURED that V3 and Perch name some shared
  species differently — probe: join the V3 labels CSV and `perch-v2.csv` on `sci_name`, count `com_name` mismatches.
- Impact: views grouping by common name could split one species by which model won the chunk.
- Proposed change: take the common name from the primary's labels when the primary knows the species.
- Acceptance gate: merge test where the secondary wins with a different common name must keep the primary's.
- Effort S.

### MMP-11: Output scale and sample rate are inferred from input length, with no override for the scale
- Severity: medium — Kind: footgun — NEW
- Evidence: `model.rs:929-935` — `[_, 160_000] … => OutputScale::Softmax, _ => OutputScale::Logit`; unknown lengths
  default to 48 kHz (`model.rs:1236-1239`). `MODEL_n_SAMPLE_RATE` exists (`registry.rs:85`) but nothing declares the
  scale. Any future 5 s/32 kHz sigmoid-trained model would be read as softmax; any unknown model as a V2.4 logit.
  Status: VERIFIED-BY-READING (impact on a specific future model CONJECTURED).
- Impact: a new model silently gets the wrong confidence function (the Perch-through-sigmoid defect,
  `CHANGELOG.md:167-175`, would recur in another form).
- Proposed change: `MODEL_n_OUTPUT_SCALE` and catalogue-declared scale; refuse an unrecognised shape without one.
- Acceptance gate: loading a fixture of an unrecognised shape without a declared scale must fail; today it loads as Logit.
- Effort S.

### MMP-12: A loaded extra classifier that no route names costs its memory and never runs, silently
- Severity: low — Kind: footgun — NEW
- Evidence: `registry.rs:454-468` unrouted → primary; `run.rs:112-118` logs routes at INFO; no wildcard in
  `parse_routes` (`registry.rs:522-544`). A source must be named by its `audio_sources` row id (`run.rs:1270-1277`).
  Status: VERIFIED-BY-READING.
- Impact: birder sets `MODEL_2_*` per the manual, sees "Waiting" on the Models board forever, ~665 MiB budget spent.
- Proposed change: WARN when an extra classifier is reachable by no route; accept `*:perch` for "every source".
- Acceptance gate: daemon test with an extra model and no routes must emit the warning (capture with tracing-test).
- Effort S.

### MMP-13: Inference is sequential with a fixed 2 threads; no Pi measurement exists for V3 or Perch
- Severity: low — Kind: opportunity — NEW (the memory rule's lack of Pi measurement is stated at `src/helpers/models.rs:32-35`)
- Evidence: `process.rs:205-262` runs routed classifiers one after another; `ModelConfig::default().num_threads = 2`
  (`model.rs:80`) is never overridden (`src/daemon/config.rs:245-250`). The only in-repo timing is x86_64, V2.4 INT8,
  28–32 ms per inference (`model.rs:1383-1388`); `06-ml-inference.md:128-137` labels its Pi numbers as unsourced estimates.
  The Models board records mean time per window on the station (`model_board.rs:20-23`). A 15 s segment is 4 chunks at a
  4.5 s step, so V3 + Perch is 8 inferences per 15 s. Status: VERIFIED-BY-READING; real-time margin UNMEASURED.
- Impact: unknown whether V3 + Perch keeps up on a Pi 4; shedding (`run.rs:472-502`) is the backstop.
- Proposed change: measure on Pi 4/5 with `scripts/hardware-test.sh` for V3 alone and V3 + Perch; make thread count a
  setting; consider running the two sessions on two threads when cores allow.
- Acceptance gate: n/a until measured; record numbers in `06-ml-inference.md`.
- Effort M.

### MMP-14: One classifier's non-finite human score suppresses chunks whichever classifier heard the bird
- Severity: medium — Kind: footgun — ALREADY-TRACKED (HANDOFF §5 item 39b)
- Evidence: `model.rs:1006-1011` non-finite human → 1.0; `process.rs:242` max across classifiers. VERIFIED-BY-READING.

### MMP-15: Perch's bar is measured, not validated on station audio
- Severity: medium — Kind: gap — ALREADY-TRACKED (HANDOFF §5 item 41; `tools/perch-bar`)

### MMP-16: Startup diagnostics describe the primary, or the configured overlap, not what runs
- Severity: low — Kind: polish — NEW
- Evidence: human-label warning checks `model` = primary (`run.rs:250-255`); the confirmation-filter effectiveness check
  uses configured `chunk_overlap_secs` against the longest window (`run.rs:276-280`) while the real step is the shortest
  window (`run.rs:720`). Status: VERIFIED-BY-READING.
- Proposed change: iterate routed classifiers; pass the effective step. Gate: a two-window registry with confirmation
  `lenient` must not log "will not reject anything" when the effective overlap makes it effective. Effort S.

### MMP-17: Stale and contradictory documentation of the input contract
- Severity: low — Kind: polish — NEW
- Evidence: `pipeline.rs:132-138` "V2.4 models expect a mel spectrogram ([1, 144_000] = 128 mel bands × 1125 frames)"
  contradicts `model.rs:690-702`; `model.rs:7-9` header says "Apply sigmoid with sensitivity"; `06-ml-inference.md:33,113-117`
  describes one model and `let mut model = BirdNetModel::load(…)` (now a registry); `model_catalog.rs:189` calls
  `birdnet-v3` "The station's default classifier" while `:68` makes `birdnet-v24` the default (printed by
  `--install-model list`, `src/helpers/models.rs:419`). `InputSpec` has no normalisation field though G-10's plan
  listed one (`FEATURE_GAP_ANALYSIS.md:275`, `model.rs:1045-1056`). Status: VERIFIED-BY-READING.
- Proposed change: correct the four texts; say in `InputSpec` docs that amplitude normalisation is "none, [-1,1] mono".
  Gate: none (docs). Effort S.

### MMP-18: FP16/INT8-I/O and rank-3 models are accepted by the shape code but would fail at run time
- Severity: low — Kind: gap — NEW
- Evidence: the tensor is always `Tensor::<f32>` of shape `[1, expected_len]` (`model.rs:625`) though
  `expected_input_length` accepts `[_, _, n]` (`model.rs:1272`); output read as f32 (`model.rs:403-405`). Status:
  CONJECTURED — probe: a tiny `[1,1,N]` fixture and an FP16-I/O fixture through `predict_chunk`; expect a Runtime error
  that becomes a "failing" classifier rather than a load-time refusal.
- Proposed change: build the tensor with the declared rank; refuse non-f32 I/O at load with a clear message. Effort S.

### MMP-19: Public `process_and_infer` skips every filter and has no caller
- Severity: low — Kind: footgun — NEW
- Evidence: `process.rs:346-414` (no privacy, noise, corroboration or occurrence filter), re-exported at
  `detection/daemon/mod.rs:31`; no caller in `src`, `crates` or `tests` (grep). Proposed: delete or make `pub(crate)`
  test-only. Effort S.

### MMP-20: The catalogue carries no licence, and installs models under different licences without saying so
- Severity: low — Kind: gap — NEW (label-licence questions are in HANDOFF/`data/labels/README.md`; model licences are not)
- Evidence: `CatalogEntry` has no licence field (`model_catalog.rs:126-157`); grep for "licen" in the file finds none.
  Perch is Apache-2.0 per the HF card (`data/labels/README.md`, Licence), V3 labels CC BY-SA 4.0
  (`taxonomy-overlap.md:48`). Perch comes from a third-party conversion (`model_catalog.rs:195-196`).
  Proposed: a `licence` and `source` line per entry, printed by `--install-model list`. Effort S.

### MMP-21: The multi-model invariants that are not tested
- Severity: medium — Kind: gap — NEW
- Tested today (read): registry refusals and routing (`registry.rs:600-983`), merge policy (`merge.rs:213-420`), a
  failing classifier not silencing others, answers, NaN, silence (`process.rs:771-1080`), chunk-step coverage
  (`pipeline.rs:538-707`, `crates/birdnet-core/tests/chunk_step_reaches_the_chunker.rs`), per-source routing through
  `run_daemon` (`run.rs:1270`), plan/memory policy (`src/helpers/models.rs:529-841`), Perch label positions
  (`crates/birdnet-core/tests/the_perch_labels_are_the_classes_the_model_scores.rs`), web/API surfaces
  (`crates/birdnet-web/tests/a_detection_says_which_classifiers_heard_it.rs`,
  `the_api_and_exports_carry_what_each_classifier_said.rs`).
- Not tested: an extra classifier's failure leaving the primary running (MMP-01); geomodel/list filtering of
  secondary-only species (MMP-03); non-taxon classes never stored (MMP-04); privacy on Perch's speech classes (MMP-05);
  per-species threshold across scales (MMP-06); `own_threshold` under dynamic thresholds (MMP-07); export provenance per
  model (MMP-08); any two-real-model run in CI (HANDOFF §4 records real-model runs in a dev environment only,
  `docs/HANDOFF.md:513-531`).
- Acceptance gate: each finding's gate above. Effort M in total.

## Not examined

- `tools/model-convert/*.py` and `tools/perch-bar/scripts` beyond their listings; the Perch-bar results themselves.
- `crates/birdnet-web/src/routes/pages/model_evidence.rs`, `routes/analysis_runs.rs` and `classifier_setup.rs` beyond
  their headers; `labels.rs` and `vocabulary.rs` parsing/alias rules in detail (only their public surface).
- The V3 geomodel alignment (`species_filter.rs` `Alignment`) and whether its week convention matches `civil::birdnet_week`.
- systemd unit `SystemCallFilter` vs two classifier sessions' thread pools (geomodel case tracked in CLAUDE.md; classifier
  sessions use an explicit thread count, `model.rs:1390-1398`, so the same SIGSYS is not expected, not verified).
- Resident memory of V3 + Perch on any board; real-time factor on Pi 4/5 (nothing in-repo measures it).
- BattyBirdNET / Stage 6 (blocked, tracked in `FEATURE_GAP_ANALYSIS.md:298-301`).
- MQTT, Apprise and e-mail treatment of secondary-model rows (assumed identical to BirdWeather's: model-agnostic).
