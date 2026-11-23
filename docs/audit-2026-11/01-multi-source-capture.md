# Audit 2026-11 / 01 — Multiple microphones and multiple RTSP streams

Read-only adversarial audit of the path from source configuration to stored, notified
and analysed detection, for stations with more than one source. Nothing was built or run
(another agent owns `target/`); every claim below was read at the cited line on
2026-11-22. **VERIFIED-BY-READING** means the cited code does what is said; it does not
mean a test was seen failing. **CONJECTURED** findings name the probe that would settle
them.

## How it works today (10 lines)

1. Sources are rows of `audio_sources` (migration 15, `crates/birdnet-db/src/migration.rs:361-386`), PK = a synthetic id `src_{usb|pw|rtsp}_{secs}_{seq}` (`crates/birdnet-web/src/routes/admin/audio.rs:1487-1505`); an empty table is seeded from CLI/config as `src_seed_N` (`src/capture/sources.rs:473-497`), and the CLI/config resolver is the fallback when no active row exists (`src/capture.rs:157-186`).
2. Each active row becomes one `CaptureManager` with `stream_id = row.id` (`src/capture/sources.rs:333-356`); all share one output directory (`src/capture.rs:191-195`) and one segment length, default 15 s (`src/cli.rs:609-610`, settable 3..600 `crates/birdnet-core/src/config/validate.rs:629,637`); the supervisor starts every source in the same tick (`src/capture/supervisor.rs:688-706`).
3. Microphones are segmented in-process by byte count, named from the wall clock when the segment's first bytes arrive (`crates/birdnet-core/src/audio/capture/segment.rs:170-180, 243-281`); RTSP and PipeWire are segmented by ffmpeg's `-f segment -segment_time N -strftime 1` (`crates/birdnet-core/src/audio/capture/process.rs:789-795, 698-705`). Names are `YYYY-MM-DD-birdnet-<id>-HH:MM:SS.wav`, one-second resolution (`crates/birdnet-core/src/audio/capture/types.rs:439-459`).
4. Consecutive segments of one source do not overlap; chunks are cut inside a file only, 3 s windows stepped by `3 - OVERLAP` (default overlap 0.0, `src/cli.rs:891-892`), with a trailing remainder under 1.5 s dropped (`crates/birdnet-core/src/detection/pipeline.rs:207, 225-236`).
5. One detection-daemon thread analyses every source's segments serially, after a size-stable settle, with one-in-two shedding above 40 queued segments or under thermal throttling (`crates/birdnet-core/src/detection/daemon/run.rs:447-502`, `daemon/mod.rs:187-192, 258-273`).
6. A detection's time is filename time + truncated chunk offset (`crates/birdnet-core/src/inference/model.rs:454-455`); its source is re-parsed from the filename (`src/daemon/disposition.rs:267-275`) and stored in `detections.Source` (migration 18, `migration.rs:463`; `src/daemon/processor.rs:937, 1014`).
7. There is **no cross-source deduplication**: two sources hearing one bird produce two rows by design (`migration.rs:455-462`); the optional duplicate interval (off by default, `src/cli.rs:850-851`) is keyed per (source, species) (`src/daemon/duplicate.rs:47-55, 148-163`); a read-only "Also heard by" card lists other sources within ±30 s (`crates/birdnet-web/src/routes/pages/detection_detail.rs:90-101`).
8. Noise memory, privacy masking and corroboration are per recording, hence per source and blind across segment boundaries (`crates/birdnet-core/src/detection/noise.rs:70-83`, `privacy.rs:156-163`, `corroboration.rs:260-276`).
9. Per-source settings that reach capture: gain, high-pass/DC/AGC, EQ chain, quiet window, RTSP transport and media mode (`src/capture/sources.rs:264-272`); thresholds, species lists, location and the dynamic threshold are station-wide; classifier routing is per source id (`run.rs:754-761`).
10. Downstream, SQLite keeps `Source`; the DuckDB mirror, BirdWeather posts, WebSocket events and notifications do not (`src/daemon/processor.rs:1185-1203, 1547-1556, 1365-1376`); MQTT carries `rtsp_id` (`processor.rs:1721-1728`).

## Findings

### MSC-01: Shedding drops the same source's segment on every sweep
- Severity: high — Kind: bug — NEW
- Evidence: `crates/birdnet-core/src/detection/daemon/mod.rs:261-271`
  > `ready.sort();` … `if i % 2 == 0 { analyse.push(path); } else { shed.push(path); }`

  Paths sort by `<date>-birdnet-<id>-<time>`, so within a day all of one source's files sort before the next source's. All sources are started in the same supervisor tick (`src/capture/supervisor.rs:696-705`) and microphone segments rotate by byte count at the same rate (`segment.rs:272-278`), so two microphones close segments within milliseconds of each other and settle in the same 500 ms sweep (`run.rs:430-449`). A sweep holding `[A@t, B@t]` analyses A and sheds B, every time. With three sources `[A,B,C]` B is always shed. The only test, `shed_split_analyses_every_second_segment_in_capture_order` (`daemon/mod.rs:309-333`), uses one id-less source.
  Status: the split rule is VERIFIED-BY-READING; that sweeps in the field hold one segment per source is CONJECTURED. Settle with a unit test: `shed_split(vec![A@06:00:00, B@06:00:00])` called for ten consecutive "sweeps" — B is shed ten times out of ten.
- Impact: birder — while the Pi is hot or behind, one microphone stops producing detections entirely and the other keeps working, so the station looks healthy. Researcher — per-source counts carry a silent 0 %/100 % sampling bias rather than the 50 % the log line promises ("analysing one segment in two").
- Proposed change: shed per source, not per sweep. Keep a `HashMap<source_label, u64>` counter in the daemon loop that survives sweeps; shed a segment when that source's counter is odd (round-robin per source), or order the ready list by (time, source) and alternate within each source. Count shed segments by source (`birdnet_segments_shed_total{source}`).
- Acceptance gate: `shedding_is_fair_across_sources` — drive `shed_split` (or the new per-source shedder) for 10 sweeps of `[A@t_i, B@t_i]`; assert each source has 5 analysed ±1. Must fail today (B: 0 analysed). Counterpart: one source alone still sheds every second segment.
- Effort S. No dependencies.

### MSC-02: Every count is a row count, and N sources hearing one bird are N rows; no deduplicated view exists
- Severity: high — Kind: gap — NEW (the design is stated in `migration.rs:455-462`, whose "optional, opt-in cross-stream collapse later" was never built: `crates/birdnet-db/src/sqlite/types.rs:56` still says "(later, opt-in)")
- Evidence: `migration.rs:457-459` "the same bird heard by two streams is recorded as two detections"; the duplicate gate is keyed per source, `src/daemon/duplicate.rs:47-55` "The cost is the other direction: two microphones that hear the *same* bird each record it once"; `crates/birdnet-db/src/sqlite/queries/detections/read.rs:259-301` is the only cross-source query and is display-only ("never collapses or hides anything").
  Status: VERIFIED-BY-READING.
- Impact: birder — adding a second microphone to the same garden roughly doubles "detections today", every species total and the activity heat map overnight; the owner's requirement "no duplications" is not met for co-located sources. Researcher — every rate, session count and trend computed from rows is inflated by the number of sources that overlap acoustically, and the inflation changes whenever a source is added, fails or is removed.
- Proposed change: (a) add a station setting `MULTI_SOURCE_MODE = independent | colocated` and, for `colocated`, an **event** layer rather than deletion: a nullable `detections.event_id` assigned at write time by a cross-source matcher keyed on (species, |Δ instant| ≤ W) where W ≥ the measured inter-source skew (see MSC-14), keeping every row. (b) Count events, not rows, on pages and in analytics when the mode is `colocated` (`COUNT(DISTINCT COALESCE(event_id, rowid))`). (c) Never drop a row across sources: a second individual on another mic within W is indistinguishable from the same bird by time alone, so the event layer must stay reversible and per-source views must remain available. (d) Record which sources are co-located (a `zone` column on `audio_sources`) so a garden mic and a pond camera are never merged.
- Acceptance gate: a processor test with two sources, one species, same second, mode `colocated`: the dashboard/summary count reads 1 and both rows still exist; mode `independent` reads 2. Must fail today (no mode, reads 2). Counterpart: same species 60 s apart on two sources stays 2 events.
- Effort L. Prerequisite for MSC-03, MSC-06, MSC-07 fixes being coherent.

### MSC-03: The effort-corrected abundance rate doubles on a two-source station
- Severity: high — Kind: bug — NEW
- Evidence: `crates/birdnet-behavioral/src/phenology/abundance.rs:275-282`
  > "A day's station time is its **longest** single source, not the sum across sources. Detections are not per source"

  and `:319-327` `MAX(seconds) AS seconds … GROUP BY day`. But detections *are* per source (MSC-02), and the numerator counts all rows. The regression test's fixture hides this: `crates/birdnet-behavioral/tests/effort_rate_counts_what_was_listened_to.rs:21-23, 27-34` seeds two microphones of effort but only **one** detection per day.
  Status: VERIFIED-BY-READING.
- Impact: birder — "detections per listening hour" for a two-mic station reads twice a one-mic station's for the same birds (the exact error the comment claims to remove). Researcher — between-year comparisons across a hardware change are wrong by the source count.
- Proposed change: make numerator and denominator agree. Either divide per source (`detections GROUP BY day, Source` over `recording_effort GROUP BY day, source`, then aggregate rates), or, under MSC-02's `colocated` mode, count events over MAX effort. Requires `Source` in DuckDB (MSC-06).
- Acceptance gate: extend the fixture with the realistic shape — two rows per detection (one per source) on the day with two sources' effort; assert the rate equals the one-source day's. Fails today (2.0 vs 1.0).
- Effort M. Depends on MSC-06.

### MSC-04: A second source's quarantined detection in the same second is discarded; an approved one loses its source and its audio
- Severity: medium — Kind: bug — NEW
- Evidence: `crates/birdnet-db/src/migration.rs:1614` the current `quarantine` table is `UNIQUE(date, time, sci_name)`; `crates/birdnet-db/src/sqlite/queries/quarantine.rs:226-230`
  > `ON CONFLICT(date, time, sci_name) DO NOTHING`

  so source B's quarantine row for the same species/second as source A's is silently not stored (no error, so `processor.rs:880-889` logs nothing). `QuarantineRecord` has no source (`src/daemon/processor.rs:738-767, 857-879`) and its `file_name` is the raw tmpfs segment path (`processor.rs:856, 865-869`), which the stream drain deletes after 600 s (`src/helpers/system.rs:13`). `approve_quarantine` inserts without `Source` (`quarantine.rs:292-300`), so an approved row has `Source = NULL`, which the per-source filters and the "Also heard by" card (`detection_detail.rs:90`) treat as "unknown/imported".
  Status: VERIFIED-BY-READING.
- Impact: birder — approving a night-owl call heard on two mics yields one row with no microphone and no playable clip. Researcher — per-source quarantine statistics are under-counted and approved rows drop out of per-source analysis.
- Proposed change: migration: rebuild `quarantine` with `source TEXT` and `UNIQUE(date, time, sci_name, COALESCE(file_name,''))` (the same identity `detections` uses since migration 23); write `source` in all three `QuarantineRecord` sites; copy it in `approve_quarantine`. Extract the clip for quarantined detections (or copy the segment into a quarantine holding dir) so approval has audio.
- Acceptance gate: processor test — two sources, same second, both below a per-species threshold: two quarantine rows exist; approving each yields a detection with its own `Source`. Fails today (one row; `Source` NULL).
- Effort M.

### MSC-05: Comments attach to every source's detection of the same species in the same second
- Severity: medium — Kind: bug — NEW
- Evidence: `crates/birdnet-db/src/migration.rs:2092-2103` (`detection_comments` keyed by `date, time, sci_name`, no file name); `crates/birdnet-db/src/detection_comments.rs:20-24` "A detection is the `(date, time, sci_name)` triple"; read at `detection_comments.rs:201`. Reviews were moved to a per-clip key (migration description at `migration.rs:2488`), comments were not.
  Status: VERIFIED-BY-READING.
- Impact: a note "this is the neighbour's dog, not a tawny owl" written on the garden mic's row appears on the camera's row, which may be a real owl.
- Proposed change: add `file_name TEXT` to `detection_comments`, keyed as `DetectionKey` is (`processor.rs:1085-1090`); backfill only where the triple matches exactly one detection; keep ambiguous legacy comments flagged as "on all detections of this second".
- Acceptance gate: two detections, same triple, different `File_Name`; comment on one; the other's comment list is empty. Fails today.
- Effort S.

### MSC-06: The DuckDB analytics copy has no `Source`, so every behavioural analytic pools sources
- Severity: medium — Kind: gap — NEW
- Evidence: `src/daemon/processor.rs:1185-1203` `LiveDetection { … file_name, detected_at_utc, run_id }` — no source; `crates/birdnet-behavioral/src/connection/sync.rs:84-114` has no source field; grep of `birdnet-behavioral/src` for `Source` finds only memory-ceiling code.
  Status: VERIFIED-BY-READING.
- Impact: researcher — sessionisation, retention, "who answers whom", co-occurrence and the dawn-chorus funnel treat a robin on the front-garden mic and a wren on the pond camera 200 m away as one soundscape; a single bird heard on two mics is two events at one instant (inflates per-session counts, creates zero-lag "answers"). Birder — no way to see behaviour for one microphone.
- Proposed change: add `Source` to the DuckDB `detections` schema, the bulk sync and `LiveDetection`; give every `birdnet-behavioral` query builder an optional `source` filter and a `partition_by_source` flag (sessions and sequences should default to per source on `independent` stations).
- Acceptance gate: `sync_from_sqlite` then `SELECT COUNT(DISTINCT Source)` = 2 for a two-source fixture; a sequence query with two co-timed sources reports no cross-source "answer" when partitioned. Fails today (column absent).
- Effort M.

### MSC-07: BirdWeather receives one post per source for one bird
- Severity: medium — Kind: bug — NEW
- Evidence: `src/daemon/processor.rs:1519-1556` posts every admitted detection; `DetectionPost` has no source or dedup key; the notification cooldown (`processor.rs:1433-1437`) gates Apprise only.
  Status: VERIFIED-BY-READING that each row posts; how BirdWeather treats two posts with the same timestamp and species from one station is CONJECTURED (settle with the BirdWeather staging API or their docs).
- Impact: a two-mic station's public map shows doubled counts for every co-heard bird; community-science data inherits MSC-02.
- Proposed change: a per-(species, Δt ≤ W) cross-source gate in front of the BirdWeather post only (keep the station's own rows): post the highest-confidence detection of the event, or the first, and record the suppressed ones in `notification_log` as `skipped`. Default on when MSC-02's mode is `colocated`.
- Acceptance gate: processor test with a mocked BirdWeather client: two sources, same species, same second → one post. Fails today (two).
- Effort S (after MSC-02's matcher; a standalone time-window gate is S on its own).

### MSC-08: Segment boundaries are hard edges — calls are split, edge chunks filtered harder, and a non-multiple segment length discards audio
- Severity: medium — Kind: gap — NEW
- Evidence:
  - No cross-file overlap: chunks start at 0 within each file (`pipeline.rs:275`); consecutive segments abut (`segment.rs:114-118`). A call spanning 14.5–15.5 s is half in each file.
  - Tail discard: `pipeline.rs:207` `const MIN_TAIL_SECS: f32 = 1.5;` and `:231` `if pos >= len || len - pos < min_tail { break; }`. With overlap 0, `SEGMENT_DURATION=10` analyses 0–9 s and never the last 1 s (10 %); 16 → loses 1 s (6.25 %); 11 loses nothing (starts 0,3,6,9; the 2 s tail is ≥ 1.5 s and is padded). Loss occurs whenever the remainder after the last full step is under 1.5 s. Validation accepts any 3..600 (`validate.rs:629,637`) and does not warn.
  - Corroboration at edges: `corroboration.rs:274-275` holds an edge chunk to the mid-file count capped at its neighbourhood, so at `balanced`/0 s overlap the last chunk of a file needs 2 of its 2 neighbours; its true neighbour (first chunk of the next segment) is invisible. The module's own test `a_lone_detection_at_a_file_edge_is_dropped` (`:343`) pins this.
  - Privacy at edges: `privacy.rs:161-163` "clips … are not extended into neighbouring segments, whose speech this filter never saw" — a voice in the last chunk of segment N does not mask the first chunk of N+1.
  Status: VERIFIED-BY-READING (arithmetic from the code); the detection loss for boundary-straddling calls is CONJECTURED in magnitude — settle with a probe that feeds a known call file split at 1.5 s offsets across a boundary through `process_file` and counts detections.
- Impact: birder — calls at 15 s, 30 s, 45 s … past each quarter-minute are less likely to be detected; with a 10 s segment, 10 % of all audio is never analysed. Researcher — periodic detection-probability dips aligned to the segment clock, and privacy leakage at boundaries.
- Proposed change: carry the last `chunk_secs - step` (≥ 1.5 s) of each source's previous segment into the next analysis (a per-source "carry" buffer in the daemon keyed by source label, valid only when the previous segment's end abuts this one's start), and report detections in the carried region with the earlier file's time. Until then: make `--doctor`/validation warn when `SEGMENT_DURATION mod step` leaves a tail under 1.5 s, and run corroboration/privacy on the joined timeline.
- Acceptance gate: (1) `chunk_starts(10*48_000, 3*48_000, 1.5*48_000)` covers ≥ 10 s of audio — fails today (covers 9). (2) a two-segment fixture with a call straddling the boundary is detected once. Fails today if the call is under the threshold in both halves.
- Effort M (carry buffer) / S (validation warning).

### MSC-09: With OVERLAP > 0, one call is several rows unless the operator also enables the duplicate interval
- Severity: low — Kind: footgun — NEW (behaviour is described in code, `src/cli.rs:839-851` and `src/daemon/duplicate.rs:1-15`, but not tracked)
- Evidence: there is no window-overlap dedup; `merge.rs` merges classifiers per chunk only (`crates/birdnet-core/src/detection/merge.rs:62-83`); each overlapping window yields its own row (distinct `Time` because the offset is folded in, `model.rs:454`). The dedup that exists is `DuplicateGate`, off by default (`cli.rs:850`).
  Status: VERIFIED-BY-READING.
- Impact: raising OVERLAP for sensitivity (as the corroboration docs encourage, `corroboration.rs:36-56`) multiplies row counts by up to `3/step`.
- Proposed change: when OVERLAP > 0 and the duplicate interval is 0, collapse same-species detections from consecutive overlapping windows of one file into one row (keep max confidence, earliest time) before the processor, or default `DUPLICATE_INTERVAL_SECS` to the window length when overlap is set; `--doctor` warns either way.
- Acceptance gate: `process_file` + infer on a fixture with one 2 s call at overlap 1.5 → one row. Fails today (two or three).
- Effort S.

### MSC-10: The same physical microphone or camera can be configured twice
- Severity: medium — Kind: bug — ALREADY-TRACKED in part (TD-4 covers the `unwrap_or_default` read; the string-equality and update-path gaps are NEW)
- Evidence: `crates/birdnet-web/src/routes/admin/audio.rs:522-526`
  > `.with_read_db(AudioSourceStore::list).unwrap_or_default()` … `.any(|s| s.kind.as_str() == kind.as_str() && s.device_id == device_id)`

  Exact string and same kind only: `plughw:1,0`, `hw:1,0` and `plughw:CARD=USB,DEV=0` are three "different" sources; a USB mic added as `usb-alsa` and again as the PipeWire default is not compared at all; `rtsp://cam/live` and `rtsp://user:pw@cam/live/` are two cameras. `update` (`audio.rs:836-858`) changes `device_id` with no duplicate check, and the schema has no unique index on `device_id` (`migration.rs:361-386`).
  Status: VERIFIED-BY-READING for the checks; the field outcome is CONJECTURED: `hw:` opened twice should fail with EBUSY and restart-loop (probe: two `arecord -D hw:N,0` on a Pi), while a `dsnoop`/PipeWire-shared device opened twice records the same audio twice, doubling every detection.
- Impact: birder — either one source flaps forever or every detection is doubled with nothing on the page saying why. Researcher — silent 2× inflation.
- Proposed change: normalise device ids before comparing (ALSA: resolve `hw`/`plughw`/`CARD=` to card+device via `/proc/asound/cards`; RTSP: strip credentials, lower-case host, default port 554, trim trailing `/`); apply the check in `update` too; refuse when the read fails (TD-4); warn when a PipeWire default source and an ALSA row resolve to the same card.
- Acceptance gate: `create_rejects_duplicate_device` (`audio.rs:2181`) extended with `hw:1,0` after `plughw:1,0`, and with the same RTSP URL plus credentials; and an `update` that sets B's URL to A's. Each fails today.
- Effort M.

### MSC-11: Channel selection is unreachable from the UI and ignored for PipeWire and RTSP; `bit_depth` is ignored everywhere
- Severity: medium — Kind: gap — NEW
- Evidence: the create/edit form has no channels field (`audio.rs:286-326`, and `update` builds no `patch.channels`, `audio.rs:835-921`), so `Channels::Left/Right` (`src/capture/sources.rs:309-318`) can only be set by editing SQLite — yet the start-up warning tells the operator to "Set the source to Left or Right" (`sources.rs:324-331`). PipeWire has no `channel_pick` (`crates/birdnet-core/src/audio/capture/types.rs:38-48`); a PipeWire Left/Right row opens 2 channels (`sources.rs:314-318`) and ffmpeg writes stereo (`process.rs:693-696`), which the decoder averages (`crates/birdnet-core/src/audio/decode.rs:241-247`) — the comb filter the pick exists to avoid. RTSP is always `-ar 48000 -ac 1` (`process.rs:784-787`), so a row's sample rate and channels are ignored and a stereo camera is averaged by ffmpeg. `bit_depth` (default 24, `audio_sources.rs:386`) is never read by capture (only `src/support.rs:182`); capture is `S16_LE` (`process.rs:430`).
  Status: VERIFIED-BY-READING.
- Impact: birder — an operator told how to fix a phase-cancelling spaced pair has no control to do it; the page shows "24-bit" for 16-bit audio. Researcher — stored source metadata does not describe the recorded signal.
- Proposed change: add a channels select (Mono/Left/Right/Stereo, and "both, analysed separately" as a future option that spawns two logical sources from one device) to create/edit; add `pan=mono|c0=c0` / `c0=c1` for PipeWire and RTSP picks; honour or remove `bit_depth`; render the effective rate/channels for RTSP rather than the stored ones.
- Acceptance gate: a web test that submits `channels=left` and reads it back; a `rtsp_command`/PipeWire argv test asserting a `pan` filter for Left. Both fail today.
- Effort M.

### MSC-12: Removing every source does not stop capture; a restart resurrects it from config under a different label
- Severity: medium — Kind: bug — NEW
- Evidence: `list()` excludes soft-deleted rows (`crates/birdnet-db/src/audio_sources.rs:558`). `seed_sources_from_config` seeds when `list()` is empty (`src/capture/sources.rs:482-487`), so after "remove all" it re-inserts `src_seed_N` from `ALSA_CARD`/`RTSP_URL`; if `src_seed_1` already exists (soft-deleted) the insert conflicts (`sources.rs:491-494`), `resolve_sources_from_db` returns `None` for an all-disabled table (`sources.rs:245-247`), and `start_capture_manager` falls back to the CLI/config sources with legacy labels `local`/`rtsp`/`RTSP_n`/`MIC_n` (`src/capture.rs:166-186`, `sources.rs:404-412, 455`). Recording effort is credited only to active `audio_sources` rows (`src/integrations/effort.rs:75, 82`: `for source in sources.iter().filter(|s| s.disabled_at.is_none())` … `source_up(&source.id) == Some(true)`), so that capture accrues no effort.
  Status: VERIFIED-BY-READING.
- Impact: birder — "I removed the camera" and it is recording again after the next restart, under a name that matches no row on the Audio page; charts then mark the station "not listening" while it detects. Researcher — a source-label discontinuity in `Source`, and effort of zero against non-zero detections.
- Proposed change: seed only when the table has **never** held a row (`SELECT COUNT(*) FROM audio_sources` including disabled), and treat "rows exist, none active" as "capture nothing" with a start-up notice; never fall back to CLI/config once the table has been used.
- Acceptance gate: test with `ALSA_CARD` in config and one soft-deleted row → `start_capture_manager` returns `None` (or resolves zero sources). Fails today.
- Effort S.

### MSC-13: Source identity is the row id, not the device — re-adding splits history, editing a URL merges two places
- Severity: low — Kind: footgun — NEW
- Evidence: delete is a soft delete (`audio_sources.rs:736-747`); re-adding the same device after removal passes the duplicate check (it reads `list()`, active only, `audio.rs:522-526`) and mints a new id (`audio.rs:533`), so its detections carry a new `Source`; `MODEL_ROUTES` keyed by the old id stop applying (`run.rs:754-761`). Conversely `update` can change `device_id` (`audio.rs:858`, `audio_sources.rs:651-653`) and keep the id, so camera A's history and camera B's continue under one label.
  Status: VERIFIED-BY-READING.
- Impact: researcher — per-source time series silently split or conflated; birder — a re-added mic loses its classifier route and its per-source filters.
- Proposed change: on create, offer to re-enable a soft-deleted row with the same normalised device (MSC-10); on a `device_id` change, require confirmation and write an `audio_source_history(id, device_id, from, to)` row so analyses can partition by physical device.
- Acceptance gate: remove then re-add `plughw:2,0` → same id re-enabled. Fails today (new id).
- Effort S.

### MSC-14: Timestamps are wall clock at segment open with one-second resolution; RTSP adds unmeasured latency, burst-time stamps and possible same-second overwrites
- Severity: medium — Kind: gap — NEW
- Evidence: microphones: `segment.rs:170-180` stamps `clock.now_unix()` when the segment's first bytes reach the writer; RTSP/PipeWire: ffmpeg `-strftime 1` at segment open (`process.rs:789-795`), i.e. after RTSP setup, ffmpeg probing and network/jitter buffering; nothing subtracts that latency. Names have one-second resolution (`types.rs:453-458`), and the detection time adds a truncated offset (`model.rs:454`; migration 24 notes the truncation, `migration.rs:643-645`). The in-process writer opens with `File::create` (`segment.rs:189`), which truncates an existing same-named file.
  Status: CONJECTURED for magnitudes and the overwrite. Probes: (1) feed a 60 s WAV to the exact `rtsp_command` argv with a `file:` input (faster than real time, as a TCP stream catching up after a stall would be) and `-segment_time 15`: if segment names repeat within one second, later segments overwrite earlier ones and all carry the burst's arrival time; (2) a two-source fixture (USB mic + a local `mediamtx` RTSP re-stream of the same signal) with a click every 10 s, comparing stored `Time` per source to measure skew.
- Impact: birder — the same call can be stamped 1–3 s apart on a mic and a camera; after a Wi-Fi hiccup a camera's caught-up audio is stamped at the reconnection time. Researcher — cross-source alignment (the "Also heard by" 30 s window, any future dedup W, localisation) rests on unmeasured skew; sub-second timing is unavailable.
- Proposed change: stamp segments from the sample clock anchored at stream start (record `start_utc_ms` per process, name = anchor + samples/rate), store `detected_at_utc` in milliseconds, and for RTSP use ffmpeg's `-use_wallclock_as_timestamps 1` or the RTCP sender report to anchor; refuse to overwrite an existing segment name (`OpenOptions::create_new` and a `-N` suffix the parser accepts); record per-source measured skew in capture status.
- Acceptance gate: the probe in (1) as a test of the RTSP argv against a fast `file:` input asserting no two segments share a name. Fails today if the probe shows overwrites.
- Effort L.

### MSC-15: The stream directory's limits do not scale with the number of sources
- Severity: medium — Kind: footgun — NEW
- Evidence: retention 600 s and cap 512 MB (`src/helpers/system.rs:13, 18`); the installer's tmpfs is `size=256M` (`installer/lib/60-dirs.sh:42-57`, which itself notes "~57 MB/source by default"). 15 s × 48 kHz × 16-bit mono = 1.44 MB per segment, 40 segments per 600 s = 57.6 MB per mono source, 115 MB for a stereo one. Five mono sources (288 MB) or three stereo ones exceed 256 MB before either the age drain or the 512 MB cap acts; the cap is above the mount, so it never fires there. The shed threshold of 40 is "ten minutes of 15 s segments from one source" (`daemon/mod.rs:187-192`) and is not divided by the source count.
  Status: VERIFIED-BY-READING for the arithmetic; the field ENOSPC outcome is CONJECTURED (probe: a 256 MB tmpfs and five `SegmentWriter`s writing 15 s segments at 48 kHz for 10 minutes; expect a write error before the drain's first eligible file).
- Impact: birder — adding a fifth camera makes capture fail on every source at once; under load the backlog threshold is reached later per source the more sources there are.
- Proposed change: derive defaults from the source set — cap = min(0.8 × free space of the watch-dir filesystem, Σ per-source bytes/s × retention); shed threshold = 40 × number of sources; `--doctor` and `/admin/audio` warn when Σ bytes/s × retention exceeds the filesystem size.
- Acceptance gate: a unit test of the derived cap given a 256 MB filesystem and five 48 kHz sources: cap ≤ 205 MB. Fails today (fixed 512 MB).
- Effort S.

### MSC-16: Effort counts audio the pipeline never analysed
- Severity: low — Kind: bug — NEW
- Evidence: effort is credited every 5 min to each row whose `birdnet_audio_source_up` gauge is up (`src/integrations/effort.rs:21-27, 82`); shedding (`run.rs:490-499`), vanished segments (`run.rs:454-464`) and drained-before-analysis segments do not touch the gauge. The processor also sets the gauge up whenever a detection arrives (`src/daemon/processor.rs:1063`), including detections from a backlog of a source that has since died.
  Status: VERIFIED-BY-READING.
- Impact: researcher — under thermal shedding the denominator stays at 100 % while half the audio is skipped, so rates halve during hot afternoons.
- Proposed change: credit effort from `birdnet_audio_analysed_seconds_total{source}` deltas (already exported per source, per `docs/FEATURE_GAP_ANALYSIS.md:681`) instead of liveness; drop the processor's `set_source_up(true)`.
- Acceptance gate: with shedding forced on, one hour of capture credits ≈ 0.5 h. Fails today (1 h).
- Effort S.

### MSC-17: Filename-derived source matching is prefix-unsafe for ids containing `-`
- Severity: low — Kind: footgun (latent) — NEW
- Evidence: the stall probe matches `stem.contains(&format!("-birdnet-{id}-"))` (`types.rs:482`), so id `cam` also claims `cam-2`'s files and a stalled `cam` looks alive. The clip namer takes the source as the 5th `-`-separated field (`crates/birdnet-core/src/audio/extraction/extractor.rs:405-410`), truncating an id with a dash. `validate_id` allows `-` (`audio_sources.rs:435-438`); the generated ids (`src_usb_<secs>_<seq>`) and `src_seed_N` have none, so this is latent until an API or import writes such an id.
  Status: VERIFIED-BY-READING.
- Impact: none today; a source stall masked once custom ids exist.
- Proposed change: forbid `-` in ids (validation) or parse ids with `RecordingFile::parse` (which splits on the last `-`, `detection/types.rs:242-253`) everywhere.
- Acceptance gate: `filename_matches_stream("2026-01-01-birdnet-cam-2-06:00:00.wav", Some("cam"), "wav")` is false. Fails today.
- Effort S.

### MSC-18: No per-source thresholds, species lists, location or dynamic threshold
- Severity: low — Kind: opportunity — NEW (multi-station location is a proposal, `docs/design/multi-station.md:3-4`)
- Evidence: the processor's thresholds and dynamic tracker are keyed by species only (`src/daemon/processor.rs:692-699, 1163-1168`); the daemon has one `lat`/`lon` (`run.rs:394-395`) and one species filter (`run.rs:521-527`); per-source routing exists only for classifiers (`run.rs:754-761`).
  Status: VERIFIED-BY-READING.
- Impact: a noisy roadside camera cannot be held to a higher bar than the quiet garden mic; a confirmation on one source lowers the dynamic threshold on all; a remote RTSP site kilometres away is filtered with the home station's occurrence model.
- Proposed change: optional per-source overrides (`audio_sources.threshold_offset`, `lat`, `lon`, `species_list_id`), resolved in the processor by `derive_source_label`; key the dynamic tracker by (source, species) when overrides exist.
- Acceptance gate: two sources, same species, confidence 0.70, source B threshold 0.80 → A recorded, B quarantined. Fails today (no override).
- Effort M.

### MSC-19: Live and notification payloads do not say which source heard the bird
- Severity: low — Kind: polish — ALREADY-TRACKED in part (`docs/FEATURE_GAP_ANALYSIS.md:688`, per-source Home Assistant topics, GAP)
- Evidence: `WsDetectionEvent` has no source (`src/daemon/processor.rs:1365-1376`); `NotificationContext` has none (`processor.rs:1401-1419`), so a template cannot say "at the pond camera"; MQTT does (`processor.rs:1721-1728`).
  Status: VERIFIED-BY-READING.
- Impact: birder — "Tawny Owl detected" with no hint which microphone to walk to.
- Proposed change: add `source` and the row's label to both and a `$source` template variable.
- Acceptance gate: rendered notification for a `src_x` detection contains the source label. Fails today.
- Effort S.

### MSC-20: "Also heard by" hides a failed read and uses a fixed 30 s window
- Severity: low — Kind: polish — NEW
- Evidence: `crates/birdnet-web/src/routes/pages/detection_detail.rs:90-101` `.unwrap_or_default()` on the corroboration query (the class CLAUDE.md names: a defaulted read rendered as fact — here as "no other source heard it"); window hard-coded `30.0`.
  Status: VERIFIED-BY-READING.
- Impact: a database error reads as "only this microphone heard it", which a reviewer uses as evidence.
- Proposed change: propagate the error into an explicit "could not check other sources" state; derive the window from measured skew (MSC-14) and segment length.
- Acceptance gate: with the `detections` read failing for that query, the page says the check failed. Fails today (card absent).
- Effort S.

### MSC-21: One analysis thread for every source
- Severity: low — Kind: opportunity — NEW
- Evidence: the daemon loop is one `std::thread::spawn` processing `ready` serially (`run.rs:355, 503-620`); throughput per source falls as 1/N and only shedding (MSC-01) protects real time.
- Impact: a Pi 4 with four cameras falls behind at dawn, the hour that matters.
- Proposed change: measure first (seconds of inference per second of audio per source on a Pi 4/5); if needed, a small worker pool with per-source FIFO ordering so per-source state (carry buffer, duplicate gate) stays ordered.
- Acceptance gate: a throughput benchmark, not a unit test; then a test that per-source event order is preserved with two workers.
- Effort M.

## Tests that cover multi-source behaviour today

Covered (read, not run): source resolution and numbering from CLI/config (`src/capture/sources.rs:576-696`); stall probe scoped per stream (`segment.rs:691-750`); per-source duplicate gate (`src/daemon/duplicate.rs:499, 515`); per-source classifier routing (`run.rs:1278`); two sources in one second keep their own model scores (`src/daemon/processor.rs:3676`); delete/relabel/review address one row among two co-timed sources (`crates/birdnet-db/src/sqlite/queries/detections/write.rs:367-463`, `crates/birdnet-db/tests/a_review_names_one_detection.rs`); RTSP transport and media mode reach the argv.

Not covered (each is a gap above): fairness of shedding across sources (MSC-01); counts/analytics with co-heard birds (MSC-02/03, and the abundance fixture encodes the opposite assumption); quarantine with two sources (MSC-04); comments with two sources (MSC-05); DuckDB source column (MSC-06); BirdWeather per-source posting (MSC-07); boundary-straddling calls and the tail discard (MSC-08); duplicate physical devices by alias or via update (MSC-10); channels in the form, PipeWire/RTSP picks (MSC-11); remove-all then restart (MSC-12); timestamp skew between a mic and an RTSP source and same-second segment names (MSC-14); stream-dir sizing for N sources (MSC-15).

## Not examined

- `src/capture/runloop.rs`, `schedule.rs`, `watchdog.rs`, `uptime.rs` beyond their headers; `crates/birdnet-core/src/audio/capture/tee.rs`, `live.rs`, `alsa.rs`, `probe.rs`, `control.rs`, `status.rs`, `manager.rs` beyond `latest_output_age`.
- `src/channel_report.rs`; `crates/birdnet-core/src/detection/nocturnal.rs`, `dynamic_threshold/*`, `phantoms` (not located), species filter internals; `src/daemon/occurrence.rs`, `daylight.rs`, `alert_rules.rs`.
- `crates/birdnet-timeseries` (not opened); `birdnet-behavioral` beyond `abundance.rs` and `sync.rs`'s struct.
- Web pages other than `admin/audio.rs` create/update/remove and `detection_detail.rs`; the onboarding wizard's `add_source` (TD-4); the search page's `Source` filter (`crates/birdnet-db/src/sqlite/queries/detections/filter.rs:366, 520` seen only by grep); exports.
- Whether the stream drain runs when the watch directory comes from `RECS_DIR` rather than `--watch-dir` (`src/helpers/system.rs:150` keys on `cli.watch_dir`; `src/helpers/station_defaults.rs:153-155` fills it only when `RECS_DIR` is unset) — seen, not traced.
- Live audio hub / HLS / `/stream` per-source paths, and TD-5 (credentials on ffmpeg's command line), which is tracked and was not re-verified.
