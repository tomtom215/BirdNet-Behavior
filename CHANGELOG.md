# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

#### Mutation testing extended to `src/daemon.rs` (item A1, PR #50 carryover)

- **`src/daemon.rs` brought to `missed = 0` cargo-mutants.** PR #50
  explicitly deferred this — the inline struct literals on
  `SpeciesFilterConfig` / `PipelineConfig` / `ModelConfig` /
  `ExtractionConfig` produced ~10 "delete field" mutants, and the
  three orchestrator functions (`start_detection_daemon`,
  `event_processor`, `dispatch_webhook`) had body-replacement
  mutants that no unit test could observe. This release:
  1. **Extracted four per-config builder helpers** —
     `build_pipeline_config`, `build_model_config`,
     `build_species_filter_config`, `build_extraction_config` —
     each pinned by a dedicated unit test covering every field
     individually so a "delete field" mutant on the struct literal
     surfaces as a failing assertion.
  2. **Extracted seven smaller pure helpers** to dissolve the
     remaining inline boundary / arithmetic / boolean mutations:
     `resolve_f32_with_default` (kills the
     `(cli - DEFAULT).abs() < f32::EPSILON` family by using
     bit-exact equality on the documented CLI default — same
     trick PR #50 used for `parse_search_term` /
     `strip_not_prefix`), `confidence_pct_trunc`,
     `confidence_pct_round`, `latency_ms_to_seconds`,
     `is_first_detection_today`, `passes_filter`,
     `should_dispatch_notification`, `species_thresholds_log_count`,
     `resolve_required_paths`, `extraction_output_dir`.
  3. **Refactored `dispatch_webhook`** to return
     `Result<u16, WebhookError>` and introduced `build_webhook_spec`
     + `WebhookSpec` + `WebhookMethod` to encapsulate the inline
     request-builder logic. The typed-error return makes the
     `replace dispatch_webhook with ()` mutant unviable, and the
     `build_webhook_spec` cells (`(GET, body)`, `(POST, body)`,
     `(POST, none)`, unknown-method fallback) are unit-tested.
  4. **Added two in-process integration tests** that catch the
     remaining `replace start_detection_daemon -> Option<...> with None`
     and `replace event_processor with ()` mutants:
     `start_detection_daemon_returns_some_with_valid_inputs` stands
     the daemon up against the tiny `tiny_v24_test.onnx` bundled at
     `crates/birdnet-core/src/testdata/`, in-memory `AppState`, and
     a tempdir watch dir; `event_processor_inserts_row_for_accepted_event`
     drops a fixture `DetectionEvent` through the channel and
     asserts the row lands in the DB (also pinning the migration-12
     correlation-id round trip end-to-end).
  5. **Mutation workflow updated** to include `src/daemon.rs` at
     `max_missed = 0` in the matrix. Path filter updated. The
     workflow's previous "deferred follow-up" note is replaced by
     a record of how the mutants were dissolved.

#### Web UI — `correlation_id` surfaced on detection-detail page (item A5)

- **`/detections/detail?date=...&time=...` now renders the per-row
  correlation id with a "Copy" affordance.** Migration 12 carries
  the daemon's per-file id to durable storage; the operator-facing
  detail page now closes the log → row traceability loop by
  rendering the id alongside a one-click "Copy" button and the
  exact `journalctl -u birdnet | grep <id>` command an admin would
  run to pull the decode/infer/notify slice that produced the row.
  Rows pre-dating migration 12 (BirdNET-Pi imports, quarantine-
  approve writes) render no card at all — no empty-state noise.
  Four new unit tests pin the empty/empty-string/non-empty/
  malicious-content escaping cases.

#### Test-fixture audit — last hand-coded `CREATE TABLE detections` removed (item F15)

- **`crates/birdnet-db/src/sqlite/queries/heatmap.rs` and
  `correlation.rs` test fixtures** were hand-coding a migration-1-
  shape `CREATE TABLE detections` block inside their `setup()`
  helpers. Both follow the exact anti-pattern PR #50 flagged on
  the `tests/web_api*.rs` files — the schema silently drifts the
  moment a new migration adds a column. Replaced both with
  `crate::migration::migrate(&conn)` so the canonical schema is
  always applied. Existing tests still pass; the
  birdnet-migrate crate's own CREATE TABLE blocks (which model
  BirdNET-Pi schemas, *not* our schema) are left alone.

#### Drift gate — `DETECTION_COLS` / `map_detection_row` / `DETECTION_COL_NAMES` (item F16)

- **`DETECTION_COL_NAMES` const list added** as a source-of-truth
  pair to the joined `DETECTION_COLS` string. Four new
  drift-gate tests pin the invariant: `DETECTION_COLS` must
  equal `DETECTION_COL_NAMES.join(", ")`, the projection's
  prepared-statement column count must match the names list, every
  name must resolve against the migrated `detections` schema, and
  `map_detection_row` must round-trip a real
  `DetectionRecord` insert. Migration 12 needed three coordinated
  edits across these three surfaces; the drift-gate tests turn
  the next missed edit into a unit-test failure with a directly-
  actionable message instead of the `"Invalid column type Text at
  index N"` runtime errors that ate half a day in the PR #35
  investigation.

#### Persistence — log-to-row traceability for detections (item C9)

- **Migration 12: `correlation_id TEXT` column on detections.** Closes
  the log→DB→UI traceability loop opened by PR #49. The daemon already
  stamps a short, sortable correlation id on every event for one audio
  file (`new_event_correlation_id` in `birdnet-core::detection::daemon`)
  and threads it through `decode → infer → notify → DB-write` logs;
  this migration carries that id to durable storage so an admin who
  clicks a suspicious row in the web UI can run
  `journalctl -u birdnet | grep <id>` to pull the exact decode/infer/
  notify slice that produced it. The column is NULLABLE so quarantine-
  approve and BirdNET-Pi-importer rows (which have no id to backfill)
  keep working unchanged, and the new `idx_detections_correlation_id`
  index makes "show every row from one file" cheap. `DetectionRecord`
  / `DetectionRow` gain a matching `correlation_id` field; the column
  is serialised on `/api/v2/detections` responses via
  `#[serde(skip_serializing_if = "Option::is_none")]` so historical
  rows don't accumulate a useless `"correlation_id": null` key.

#### Supply-chain — Software Bill of Materials at release (item D14)

- **CycloneDX SBOM attached to every GitHub release.** The release
  pipeline now installs `cargo-cyclonedx@0.5.7` (pinned for repro-
  ducibility), generates both CycloneDX 1.5 JSON and XML BOMs of the
  full workspace, and uploads `birdnet-behavior-<ver>-sbom.cdx.json`
  + `.cdx.xml` alongside the binaries. Both SBOM files are signed by
  the same SLSA build provenance attestation as the binaries and
  hashed in `SHA256SUMS`. Consumers can ingest them into
  Dependency-Track, GitHub Dependency Graph, or any CycloneDX-aware
  vulnerability scanner. The release notes template links to the
  files so operators don't have to dig through the artifact list.

#### Test coverage carryovers from PR #49 (item A1)

- **`src/helpers.rs` lifted from 0 % to ~95 % unit coverage.** Each
  config-and-state helper now has a dedicated test pinning the CLI →
  config → built-in-default precedence — `db_path_from_config`,
  `init_audio_source`, `init_site_name`, `init_i18n`, `init_image_cache`,
  `maybe_install_avahi_service`, `start_disk_manager`. The pattern
  uses `Cli::parse_from(["birdnet-behavior"])` for the "no flags"
  baseline and `Config::parse(...)` for hand-written config snippets,
  so the tests run without filesystem or network I/O. 21 new tests
  total. Closes carryover item A1.
- **`src/integrations.rs` lifted from 0 % to ~90 % unit coverage.**
  Every `create_*_client` and `create_notification_*` helper now has
  precedence tests covering "CLI wins", "config falls through", and
  "neither configured → None". Notable: the MQTT helper's
  `retain` / `port` / `topic_prefix` overrides are pinned per-field
  so a future config-key rename surfaces immediately, and the email
  notifier round-trips through a real settings table seeded via
  `birdnet_db::settings::set`. 32 new tests total.
- **`crates/birdnet-db/src/sqlite/queries/detections.rs` lifted from
  the 11-test smoke surface to a 34-test full-CRUD surface.** The
  remaining helpers — `delete_detection`, `relabel_detection`,
  `lock_detection`/`unlock_detection`/`is_detection_locked`,
  `locked_file_names`, `species_for_date`, `detection_dates`,
  `todays_detections{,_count}` (including the `NOT ` exclusion path
  and whitespace-search behaviour) — are now pinned by dedicated
  tests. The migration-11 chunked-recording contract (5 chunks per
  file each get a row) and the migration-12 correlation-id round
  trip are both regression-tested.
- **Six integration test fixtures fixed (`tests/web_api*.rs`).** Six
  test files had hand-coded `CREATE TABLE detections` declarations
  duplicating migration 1 — the exact anti-pattern ADR-16 flags as
  the source of three of the PR #35 production bugs. Each was rewriting
  the schema to the migration-1 shape on every test run, so the
  fixtures couldn't see any column added by migrations 2–12. Replaced
  with `birdnet_db::migration::migrate(&conn)` so the canonical schema
  is always applied. All 31 web-API integration tests pass on the new
  fixture.

#### Mutation testing matrix expanded (item A2, partial)

- **`crates/birdnet-db/src/sqlite/queries/detections.rs` added to the
  cargo-mutants matrix.** Runs as its own job with the same
  `missed = 0` gate that already pins `validate.rs`,
  `inference/model.rs`, and `extractor.rs`. The 30+ tests added in
  this PR (cover the full CRUD surface plus the migration-12
  correlation-id round trip) make every mutant observable. Path
  filter and PR/cron triggers updated to match. The workflow now
  supports a per-row `package` override so future non-`birdnet-core`
  files plug in cleanly.
- **`src/daemon.rs` deferred to a follow-up PR.** A dry run
  surfaced the right answer to the carryover plan's question: the
  extracted pure helpers (`decide_disposition`,
  `derive_source_label`) are mutation-clean *after* the boundary
  test fix (`<` → `<=` on a float-exact `0.5` rather than a
  non-representable `0.8`) that this PR adds. But the surrounding
  `start_detection_daemon` and `event_processor` orchestrators
  contribute ~10 "delete field from struct" mutants on the
  `SpeciesFilterConfig` / `PipelineConfig` / `ModelConfig` /
  `ExtractionConfig` literals that no unit test can catch without
  either (a) extracting per-config pure builders (the dim_to_usize
  template pattern), or (b) standing up an integration harness that
  actually runs the daemon. Either is a substantial refactor and
  doesn't fit the "dep bump + traceability" theme of this PR.
  Tracking as the highest-priority follow-up; the matrix template
  is already wired so it lands as a one-line addition once the
  helpers exist.

#### Supply chain — last advisory ignore lifted (item A3)

- **RUSTSEC-2026-0097 dropped from `.cargo/audit.toml` and `deny.toml`.**
  The lockfile now pins `rand 0.8.6` (the patched version the
  advisory listed under `>= 0.8.6` ↦ fix). Both ignore lists are now
  empty — the project clears `cargo audit --deny warnings` and
  `cargo deny check advisories` with no exceptions. The comment in
  both files documents the chain that unblocked it for next time.

#### Operability and test coverage on the carryover path from PR #35

- **`birdnet-core::detection::daemon::new_event_correlation_id`** —
  generates a short, sortable ID stamped on every event the daemon emits
  for one audio file. `DetectionEvent` gains a `correlation_id` field
  that propagates through `decode → infer → notify → DB write`, so an
  operator can trace one file end-to-end with a single grep over the log
  stream. Closes the visibility gap noted in the carryover plan ("every
  event currently carries species + confidence but not a recording-id
  or chunk-id").
- **`birdnet-web::metrics`** — process-local Prometheus counters and
  latency histograms surfaced at `/api/v2/metrics`. Replaces the previous
  scrape-time snapshot (DB row count, RSS) with a real time-series
  exposition: `birdnet_detections_total{species,chunk_offset}`,
  `birdnet_inference_duration_seconds`, `birdnet_db_write_duration_seconds`,
  `birdnet_audio_source_up{source}`, `birdnet_watchdog_pings_total`.
  Hand-rolled exposition (no `prometheus` crate dependency); fixed
  histogram buckets bracket the real per-chunk latency on a Pi 5
  (1 ms ... 10 s). 9 new lib tests pin the renderer's escaping,
  bucket-cumulativity, and sort-determinism contracts.
- **`docs/grafana-dashboard.json`** — committed dashboard for the new
  metrics. Five rows: Liveness (audio source up, watchdog ping rate,
  uptime), Detection signal (per-species rate timeseries + lifetime
  table), Pipeline latency (inference + DB-write p50/p95/p99), Resources
  (RSS against the 384 MiB MemoryHigh ceiling, distinct species).
- **`birdnet-behavior --doctor` watchdog check** verifies the daemon's
  systemd-watchdog plumbing is honoured by the supervisor. Walks the
  three-question decision matrix: `NOTIFY_SOCKET` set? `WATCHDOG_USEC`
  set? does a synthetic `WATCHDOG=1` ping reach the socket? Outcomes:
  `Skip` (not under systemd), `Warn` (notify-but-no-watchdog),
  `Pass` (ping delivered, interval echoed), `Fail` (ping rejected —
  supervisor has gone away). Six new unit tests cover the describe and
  probe paths.

### Changed

- **Refactored `src/daemon.rs::event_processor`** to extract its
  threshold gates into a pure-logic helper, `decide_disposition`,
  returning a `DispositionDecision` enum. The 600-line god-function
  shrinks slightly and gains nine unit tests pinning every cell of the
  per-species × global threshold decision matrix — the kind of
  per-file coverage gap the PR #35 carryover identified as the source
  of the production bugs we just shipped fixes for.
- **`crates/birdnet-core/src/inference/model.rs`** refactored to expose
  three new public helpers, `infer_sample_rate_from_shape`,
  `recommended_chunk_samples_from_shape`, and `compute_confidence`,
  each of which used to be inline branching inside a method. The
  helpers are mock-free, branch-pinnable, and now carry 17 additional
  unit tests covering every model-family decision cell — including the
  V3.0 sigmoid-on-probabilities regression that took out the previous
  shipping confidence. The `regression_v30_probability_not_sigmoided`
  test pins the anchor case directly.
- **Mutation testing scope widened** to a 3-file matrix with
  `missed > 0` as the gate on every file:
  `crates/birdnet-core/src/config/validate.rs`,
  `crates/birdnet-core/src/inference/model.rs`,
  `crates/birdnet-core/src/audio/extraction/extractor.rs`. Each file
  is its own job so a surviving mutant in one doesn't tank the
  report on the others. Two embedded ~220-byte ONNX models
  (`crates/birdnet-core/src/testdata/tiny_v24_test.onnx` and
  `tiny_v30_test.onnx`) let the new BirdNetModel tests drive
  `infer_sample_rate`, `recommended_chunk_samples`,
  `is_probability_output`, the setters, and `predict` without the
  real 541 MB BirdNET+ model on disk. The mutation workflow installs
  `ffmpeg` so the freq-shift and format-conversion branch tests in
  extractor.rs actually run instead of skipping. Final mutant counts
  on the touched files: **0 missed / 65 caught on validate.rs**,
  **0 missed / 73 caught on inference/model.rs**, **0 missed / 24
  caught on extractor.rs** (numbers will be re-verified by the
  matrix run after this lands).
- **Eight transitive RUSTSEC advisories lifted** by targeted
  `cargo update --precise`: `rustls-webpki` 0.103.9 → 0.103.13 covers
  RUSTSEC-2026-0049/0098/0099/0104, `aws-lc-rs` 1.16.1 → 1.17.0 brings
  `aws-lc-sys` 0.38.0 → 0.41.0 covering RUSTSEC-2026-0044/0048,
  `tar` 0.4.44 → 0.4.46 covers RUSTSEC-2026-0067/0068. The only
  remaining ignore is RUSTSEC-2026-0097 against `rand` 0.8.5 (no 0.8.x
  patch released upstream as of this writing; rand 0.9.x line is
  current at 0.9.4). `.cargo/audit.toml` and `deny.toml` both reflect
  the new lone-entry state with an explicit justification.
- **`coverage.yml` exclusion comment expanded** to document why
  `crates/birdnet-migrate/` and `crates/birdnet-behavioral/` stay out
  of the per-PR coverage measurement (the analytics crate's DuckDB
  build adds ~10 minutes; the migration crate is fixture-driven and
  per-line numbers would be misleading). Both decisions are revisited
  on each major refactor of those crates.

#### Dependency refresh — folded in PRs #37–#48 from Dependabot

- **GitHub Actions** bumped across every workflow:
  `actions/cache@v4 → v5`, `actions/upload-artifact@v4/v6 → v7`,
  `actions/download-artifact@v7 → v8`,
  `marocchino/sticky-pull-request-comment@v2 → v3`. Pinned SHAs in
  `release.yml` updated to match (`v4.6.2 → v7.0.1` for upload,
  `v4.1.8 → v8.0.1` for download).
- **Cargo patch + minor group**: `clap` 4.6.0 → 4.6.1, `filetime`
  0.2.27 → 0.2.29, `proptest` 1.10 → 1.11, `reqwest` 0.13.2 → 0.13.3,
  `tower-http` 0.6.8 → 0.6.11, `tracing-subscriber` 0.3.22 → 0.3.23.
- **Cargo async runtime group**: `tokio` 1.51 → 1.52 (patch).
- **Cargo web framework group**: `axum` 0.8.8 → 0.8.9,
  `tokio-tungstenite` 0.28 → 0.29 (transitive).
- **`audioadapter-buffers` 2 → 3** — semver-major bump in the audio
  buffer adapter; no API changes needed in this codebase (`rubato`
  consumed it transitively, and our direct uses target only the
  `InterleavedSlice` constructor which is stable across the bump).
- **`criterion` 0.5 → 0.8** — major bench-framework bump; only used
  in `crates/birdnet-core/benches/audio_pipeline.rs`, which compiles
  unchanged against 0.8. Dropped transitive deps `is-terminal` and
  `hermit-abi`.
- **`sysinfo` 0.32 → 0.39** (PR #47) — the 0.39 line requires Rust
  1.95, so it is paired with the **workspace MSRV bump 1.88 → 1.95**
  (see below). The API changes were already adopted on the way through
  0.38 — `RefreshKind::new()` → `RefreshKind::nothing()` (rename, same
  behaviour), `Components::refresh()` takes a `bool` arg, and
  `Component::temperature()` returns `Option<f32>` so we use
  `.and_then` instead of `.map` — so the 0.38 → 0.39 step needed no
  source changes, only the version constraint and the MSRV move.
- **Workspace MSRV raised 1.88 → 1.95**, the current Rust stable as of
  2026-05-22. Driven by `sysinfo` 0.39 (above); 1.95 is both the floor
  that crate demands and the latest released toolchain, so the MSRV
  tracks stable rather than trailing it. Updated in lockstep:
  `Cargo.toml` `rust-version`, `clippy.toml` `msrv`, the Dockerfile
  `RUST_VERSION` arg (`rust:1.95-slim-trixie` builder), the
  `dtolnay/rust-toolchain` pins in `ci.yml` and `release.yml`, and the
  README badge / docs.
- **New clippy nursery lint allowed for the 1.95 toolchain.** Rust
  1.95's clippy enables `duration_suboptimal_units`, which flags ~25
  pre-existing `Duration::from_secs(…)` call sites in favour of
  `from_mins` / `from_hours`. The explicit-seconds form is intentional,
  so the lint is added to the workspace `[lints.clippy]` allowances
  rather than churning those sites (and `from_days` is still unstable
  at this MSRV regardless).
- **Currency sweep (2026-05-22).** In-range `cargo update`: `serde_json`
  1.0.149 → 1.0.150, `duckdb` 1.10502 → 1.10503 (`libduckdb-sys`
  likewise), plus transitive `autocfg` 1.5.0 → 1.5.1 and `either`
  1.15.0 → 1.16.0. The unused `ndarray` workspace entry was aligned
  0.16 → 0.17 to match the version `ort` already resolves transitively
  (0.17.2).
- **`rubato` 1.0.1 → 2.0.0** — major-version bump with no source
  changes needed in our consumer (the resampler API we use is stable
  across the bump). Brought in transitive `audioadapter` 3 to match.
- **`symphonia` 0.5.5 → 0.6.0** — major-version bump that **did**
  break our `decode_file` implementation. Rewrote
  `crates/birdnet-core/src/audio/decode.rs` for the new API:
    * `symphonia::core::probe::Hint` → `symphonia::core::formats::probe::Hint`.
    * `get_probe().format(...)` (taking options by ref, returning a
      `ProbeResult`) → `get_probe().probe(...)` (taking options by
      value, returning a `Box<dyn FormatReader>` directly).
    * `format.default_track()` → `format.default_track(TrackType::Audio)`.
    * `track.codec_params` is now `Option<CodecParameters>` rather
      than a flat struct; access requires `.as_ref().and_then(|p| p.audio())`.
    * `get_codecs().make(...)` → `get_codecs().make_audio_decoder(...)`
      taking the audio-specific `AudioCodecParameters`.
    * `format.next_packet()` now returns `Result<Option<Packet>>`
      (`None` for EOF rather than `UnexpectedEof`).
    * `packet.track_id` is a struct field, not a method.
    * Buffer-copy API switched from
      `SampleBuffer::new(...).copy_interleaved_ref(audio_buf)` to
      `audio_buf.copy_to_slice_interleaved(&mut vec)`, sized via
      `audio_buf.samples_interleaved()`. `num_planes()` now reports
      channel count.
  All 243 birdnet-core lib tests still pass; the live ADR-16 Layer-4
  check (Pica WAV → DB) must run in CI after merge.
- **Skipped: PR #36** (`dtolnay/rust-toolchain` 1.88 → 1.100).
  Rust 1.100 does not exist — current stable is 1.95 and Dependabot
  misordered the `1.x` action tags (it sorts `1.100 > 1.95`
  lexically). The toolchain pins move to **1.95**, the real current
  stable, via the MSRV bump above — not to the bogus 1.100. PR #36
  should be closed.
- **Lockfile**: 8 transitive RUSTSEC advisories now unblocked
  (rustls-webpki 4, aws-lc-sys 2, tar 2 — see A3 above) plus the
  routine churn from the Dependabot bumps. Only RUSTSEC-2026-0097
  (rand 0.8.5) remains, with the same documented justification.

### Fixed

- **Detection confidence on BirdNET+ V3.0 preview models was being
  silently halved** by applying `sigmoid` to the model's `predictions`
  output. The official `birdnet-team/birdnet-V3.0-dev/analyze.py`
  reference uses the model output as already-calibrated probabilities
  in `[0, 1]` (its default threshold is `--min-conf 0.15`, which only
  makes sense against a probability distribution). Our pipeline was
  applying `sigmoid(sensitivity * raw)` to those values, which
  compressed the entire `[0, 1]` range into `[0.5, 0.73]` and turned a
  Magpie that the model rated `0.9247` into a `0.7160` detection. Same
  effect on every species — every detection clustered near 50 % because
  `sigmoid(~0) = 0.5`, which is why every WAV ended up with a long
  list of spurious "owl detections" near the noise floor.
  - Fix: new `is_probability_output` flag set at model-load time from
    the input shape (V3.0 fixed or dynamic ⇒ true). The `predict` path
    branches on it — V3.0 models pass through clamped to `[0, 1]`,
    V2.4 still goes through `sigmoid(sensitivity * logit)`.
  - Live verification on the bundled Pica WAV: confidence climbs from
    71 % to **92.1 %, 91.6 %, 81.9 %, 93.9 %** — matching the V2.4 /
    BirdNET-Pi reference range of 93.9–97.0 % on the same WAV. The
    spurious owl detections at the previous ~50 % noise floor have
    completely disappeared (the real noise floor is below 5 %).
  - `tests/inference_e2e.rs` bumps its assertion from `> 0.50` to
    `> 0.80` so a future regression of this class fails the test
    immediately instead of silently lurking under a tolerant bound.
- **Audio-clip extraction range inversion** (`crates/birdnet-core/src/audio/extraction/extractor.rs`):
  `safe_stop` was clamped to the operator-configured `recording_length`
  rather than the actual decoded audio length. Any detection past that
  window produced `start_sample > stop_sample` and silently dropped the
  clip with the error *"invalid sample range: 1224000..720000"*. The
  fix decodes first, clamps both endpoints to the file's real length,
  rejects empty audio with a clear message, and ships three regression
  tests covering the clamp / EOF / empty-audio paths.
- **Detection rows lost across chunks of one recording**
  (`migration 11`): the previous `UNIQUE(Date, Time, Sci_Name)`
  constraint collapsed every chunk of one recording into a single row
  because every chunk inherits the same `Time` from the file name. A
  Eurasian Magpie that called in chunks 0, 4.5, 9, 13.5, and 18 seconds
  produced **one** database row; the other four were rejected and lost.
  New schema: `chunk_offset_secs REAL NOT NULL DEFAULT 0.0` column plus
  `UNIQUE(Date, Time, Sci_Name, File_Name, chunk_offset_secs)`. Live
  re-run with the bundled Magpie WAV: **5 distinct chunks recorded, top
  confidence 71.9 %**.
- **Test-fixture schema drift**
  (`crates/birdnet-db/src/sqlite/connection.rs::open_or_create`): this
  helper hand-coded its own `CREATE TABLE detections` with only the
  migration-1 columns, so every test using it ran against a stale
  schema. Fixed to apply the full migration chain — surfaced six
  pre-existing test failures masquerading as passes that the new
  migration 11 caught immediately.
- **Three `INSERT INTO detections VALUES (...)` time bombs** with no
  column list in `birdnet-db/sqlite/queries/heatmap.rs`,
  `correlation.rs`, and `birdnet-migrate/birdnet_pi/importer.rs`. Each
  would break the same way as the main daemon insert did when a future
  migration adds a column. Now all use explicit column lists.

- **Detection confidence on BirdNET+ V3.0 preview models** improves
  substantially because the daemon now adopts the model's recommended
  chunk length instead of always using the V2.4-era 3.0-second default.
  Same `Pica_pica_30s.wav` fixture, same model, only chunk length
  changed: Eurasian Magpie confidence went from **52.2 %** (3.0 s × 32 kHz =
  96 000 samples) to **71.5 %** (4.5 s × 32 kHz = 144 000 samples).
  Python ONNX Runtime reference at 4.5 s gives the same 71.8 %, so the
  Rust pipeline now sits at parity with the reference implementation
  rather than 19 percentage points below it. Investigation, evidence
  and the comparison against BirdNET V2.4 (which BirdNET-Pi used and
  which still hits 93–97 % on the same WAV) live in the new ADR
  [`docs/architecture/15-model-chunking.md`](docs/architecture/15-model-chunking.md).
- `BirdNetModel::recommended_chunk_samples()` and
  `recommended_chunk_secs()` expose the per-model chunk size so the
  daemon can pick the right value without hard-coding model knowledge
  in the pipeline.

### Added

#### Field-deployment hardening (24/7/365 unattended operation)

- **systemd watchdog integration** (`src/sd_notify.rs`). The daemon now
  speaks the `sd_notify` protocol natively (no extra dependency): sends
  `READY=1` after the HTTP server binds, `WATCHDOG=1` every
  `WATCHDOG_USEC / 2` from a background tokio task, and `STOPPING=1` on
  graceful shutdown. Verified end-to-end against a real Unix datagram
  socket: `READY=1 → WATCHDOG=1 …  → STOPPING=1`. Fixes the previously
  broken combination of `WatchdogSec=120` (set in the systemd unit) with
  no `sd_notify` call in the binary — under the old config systemd
  would kill the daemon every 2 minutes in production.
- **Periodic database maintenance** (`src/maintenance.rs`) — background
  task that runs a daily `PRAGMA integrity_check`, a weekly WAL
  checkpoint + `VACUUM`, and prunes the backup directory to the most
  recent 14 snapshots. All best-effort with full logging; never crashes
  the loop on transient failure.
- **`vacuum_database` and `checkpoint_wal`** added to
  `birdnet_db::resilience` so the binary can do scheduled maintenance
  without taking a new direct `rusqlite` dependency.
- **Hardened systemd unit** in `install.sh`:
  - `Type=notify` + `NotifyAccess=main` + `WatchdogSec=120` —
    process-supervision contract is now real.
  - `ExecStartPre` runs `birdnet-behavior --doctor`; exit code 2
    (errors) blocks startup so the journal shows *what is broken*
    instead of a restart-loop.
  - `ProtectSystem=strict`, `ProtectHome=read-only`, explicit
    `ReadWritePaths`, `PrivateTmp=yes`, `NoNewPrivileges=yes`,
    `LockPersonality=yes`, `MemoryDenyWriteExecute=yes`,
    `RestrictRealtime=yes`, `RestrictNamespaces=yes`,
    `SystemCallFilter=@system-service` minus the privileged / kernel /
    debug / reboot / mount / cpu-emulation / clock / module groups.
  - Resource ceilings: `MemoryMax=512M`, `MemoryHigh=384M`,
    `TasksMax=512`, `LimitNPROC=256`, `OOMPolicy=stop`.
  - `After=network-online.target sound.target time-sync.target` —
    no startup race with mic enumeration or clock sync on slow-booting
    hardware.
  - `LogRateLimitIntervalSec=30` + `LogRateLimitBurst=1000` — a chatty
    failure mode cannot exhaust the SD card.
- **`docs/FIELD_DEPLOYMENT.md`** — 12-section runbook for unattended
  deployments: hardware checklist, power & thermals, storage planning,
  network resilience, system hardening, time synchronisation, watchdog
  smoke test, backup policy, remote diagnostics, update strategy,
  pre-flight checklist, and a symptom-keyed recovery runbook.

- **`birdnet-behavior --doctor`** (alias `--preflight`) — a one-shot
  preflight diagnostic that runs ~12 environment checks (CPU, temp dir,
  config parse, every config value range, listen address, database
  directory and integrity, recordings dir, audio source reachability with
  ALSA / PulseAudio / RTSP probes, model file sanity, audio encoder
  presence when needed, Apprise CLI when configured, disk free space) and
  prints a one-screen report with a remediation hint per finding. Exit
  code summarises the worst severity (0 = ready, 1 = warnings, 2 = errors)
  so it works in monitoring scripts as well as interactively.
- **`birdnet-behavior --doctor-json`** — same checks, single-line JSON
  output for monitoring integrations (Nagios, Zabbix, Home Assistant
  command sensor, Prometheus textfile collector). String escaping is
  hand-rolled per RFC 8259 §7; control characters become `\uXXXX`.
- Configuration validation at load time
  (`birdnet_core::config::validate`) — surfaces 13 distinct
  misconfigurations (lat/lon pairing and range, CONFIDENCE / SF_THRESH /
  PRIVACY_THRESHOLD / SENSITIVITY / OVERLAP / RECORDING_LENGTH /
  SEGMENT_DURATION bounds, schedule string shape, mutually-exclusive audio
  sources, unsupported AUDIO_FORMAT, unknown INFO_SITE, malformed language
  code) with clear remediation messages.
- Property-based tests (proptest) for the configuration validator cover
  the full reachable numeric range plus a panic-freedom invariant over
  arbitrary string input.
- Supply-chain CI workflow (`.github/workflows/supply-chain.yml`) running
  `cargo-deny`, `cargo-audit`, `cargo-machete`, `typos`, and `shellcheck`
  on every PR and weekly cron.
- Reproducibility files: `rust-toolchain.toml`, `rustfmt.toml`,
  `clippy.toml`, `deny.toml`.
- Repository hygiene: `SECURITY.md`, `.github/CODEOWNERS`,
  `.github/dependabot.yml`, structured GitHub issue forms, and a PR
  template with quality-gate checkboxes.
- Architecture Decision Record `docs/architecture/14-diagnostics.md`
  captures the design and trade-offs of the diagnostic system.
- **Snapshot tests** for the `--doctor` text output. The render is split
  into a pure `render_text(&[Check]) -> String` function; four golden
  files under `src/testdata/doctor_snapshots/` pin the exact bytes of
  the report so accidental wording or formatting drift has to come
  through a PR. Set `UPDATE_DOCTOR_SNAPSHOTS=1 cargo test` to refresh
  after an intentional UX change.
- **Mutation testing** workflow (`.github/workflows/mutation.yml`)
  that runs `cargo-mutants` on the configuration validator. Catches
  "tests pass even after the validator's behaviour changes" — the
  one mutant that survived in the first run revealed a missing minute
  boundary case, which is now covered by a new property test.
  Current score: 0 missed / 61 caught / 4 unviable.
- **Coverage workflow** (`.github/workflows/coverage.yml`) running
  `cargo-llvm-cov` on every PR. Sticky summary comment, HTML + lcov
  artifacts, optional Codecov upload via `CODECOV_TOKEN`.
- **Subprocess smoke tests** for the binary (`tests/doctor_smoke.rs`).
  Builds the actual binary and runs `--version`, `--help`, `--doctor`,
  `--preflight` (alias), `--doctor-json`, and `--check-db` to catch
  "compiles but doesn't run" regressions — exactly the class of bug
  that previously slipped past the unit tests when tracing was writing
  to stdout and silently corrupting the JSON output.
- **`.pre-commit-config.yaml`** mirrors the CI quality gates locally so
  contributors fail fast (rustfmt check, typos, shellcheck, optional
  manual clippy, generic file hygiene, Conventional-Commits message
  format).
- **Top-level `TROUBLESHOOTING.md`** organised by symptom — service
  won't start, web UI not reachable, no detections, database errors,
  memory pressure on small hardware, notifications never arrive,
  cross-cutting "huh, that's weird" checklist. Each section links back
  to the doctor as the first step.

### Changed

- `install.sh` model download now resumes on interrupt (`curl -C -` /
  `wget -c`), shows a progress bar, and keeps the partial file in place
  on failure so a flaky connection no longer forces a 541 MB restart from
  zero. Failure messages list the three common root causes (no internet,
  Zenodo down, disk full) inline.
- `.env.example` gains worked latitude/longitude examples for three
  continents, an OpenStreetMap walk-through for finding coordinates, and
  units + ranges for SF_THRESH, PRIVACY_THRESHOLD, SEGMENT_DURATION, and
  the schedule modes.
- `README.md` troubleshooting section now leads with
  `birdnet-behavior --doctor`.
- `quickstart.sh` post-bootstrap output advertises the diagnostic.

## [0.1.0] - 2026-04-12

First public release. BirdNet-Behavior is a ground-up Rust rewrite of
BirdNET-Pi that ships as a single static binary for Raspberry Pi and
x86_64 Linux.

### Added

#### Core detection pipeline

- Pure-Rust audio pipeline with `symphonia` (decode), `rubato` (resampling),
  and `realfft` (mel spectrogram) — zero C dependencies in the audio path.
- ONNX Runtime inference through the `ort` crate, statically linked into
  release binaries. BirdNET+ V3.0 is the default model; BirdNET V2.4 FP16
  and V1 remain compatible.
- File-watcher detection daemon with configurable chunking, overlap,
  sensitivity, per-species confidence thresholds, and privacy filtering.
- Audio quality pre-filtering: SNR estimation, spectral flatness,
  adaptive noise-floor tracking, and rain / wind detection.
- Species occurrence frequency filter driven by the BirdNET metadata
  model, with whitelist, include, and exclude lists.
- Rare-bird quarantine workflow: detections that fall below per-species
  thresholds are quarantined for manual review rather than dropped.

#### Audio capture

- ALSA, PulseAudio, PipeWire, and RTSP capture sources, each supervised
  as a restart-aware subprocess with gap detection and disk monitoring.
- Multiple simultaneous RTSP streams via `--rtsp-urls`.
- Solar-aware recording scheduler with sunrise / sunset computation,
  twilight offsets, fixed-window schedules, and a night-inhibit mode.
- tmpfs support for transient audio storage to reduce SD card wear on
  Raspberry Pi deployments.
- Automatic disk management: per-species retention caps, auto-purge, and
  configurable disk-usage thresholds.

#### Storage and resilience

- SQLite operational database with WAL mode, ten idempotent schema
  migrations, integrity checks, hot backup, restore, and auto-recovery.
- Per-IP rate limiter on API and admin routes (token-bucket with
  `Retry-After` header).
- HTTP Basic Auth with constant-time comparison.

#### Web server and dashboard

- `axum` HTTP server with REST API, WebSocket, Server-Sent Events, and
  server-rendered HTMX pages. No client-side JavaScript framework.
- HTMX pages: dashboard, today, history, species list, species detail,
  species gallery, life list, activity heatmap, correlation, charts,
  weekly report, recordings browser, audio player, livestream, kiosk,
  notification center, quarantine, system health, and weekly report.
- Admin panel: settings editor, species thresholds, species filter
  tester, BirdNET-Pi migration wizard, system info, backup management,
  live log viewer (SSE), notification history, alert rules, data
  quality dashboard, and binary update check.
- Full dark / light theme support with OS preference detection.

#### Analytics (optional `analytics` feature)

- DuckDB behavioral analytics: sessionize, retention, funnel, sequence,
  and next-species prediction, implemented via the duckdb-behavioral
  extension.
- Phenology analytics: migration timing percentiles, weekly abundance
  index, peak weeks, monthly totals, species richness, and
  effort-corrected abundance.
- Time-series analytics: activity, diversity (Shannon), trend, peak,
  gap, and session windows (tumbling, sliding, hopping, session).

#### Integrations

- BirdWeather detection and soundscape uploads with retry and backoff.
- Apprise notifications across 80+ channels with per-species cooldown,
  watchlist, and template rendering.
- SMTP email alerts via `lettre` with rustls TLS (no OpenSSL).
- Wikipedia species image cache with on-disk and in-memory indexing.
- Pure-Rust MQTT 3.1.1 publisher (no external broker library) with
  Home Assistant auto-discovery.
- GitHub Releases auto-update with atomic binary replacement.
- Heartbeat URL pinging for uptime monitors.

#### Migration

- Non-destructive BirdNET-Pi import wizard. Source database is opened
  read-only. Transactional, idempotent, with pre- and post-migration
  species reports and a data quality report.
- Supports both BirdNET-Pi SQLite databases and `BirdDB.txt` CSV flat
  files.

#### Observability and deployment

- Prometheus metrics endpoint (`/api/v2/metrics`).
- `tracing`-based structured logging with SSE log streaming.
- Multi-architecture Docker images published to GHCR (`linux/amd64`,
  `linux/arm64`), with and without the `analytics` feature.
- Cross-compiled release binaries for `aarch64-unknown-linux-gnu` and
  `x86_64-unknown-linux-gnu`.  The `ort` crate does not ship prebuilt
  ONNX Runtime binaries for `armv7-unknown-linux-gnueabihf`, so 32-bit
  ARM is not supported — Pi 3 / Pi Zero 2W users should install the
  64-bit Raspberry Pi OS, or build from source.
- Release binaries are built on Ubuntu 24.04 (GCC 13, glibc 2.39) to
  match the libstdc++ and glibc baselines that pyke's prebuilt ONNX
  Runtime archives require.  **Runtime requirement: glibc >= 2.39**
  (Raspberry Pi OS Trixie, Debian 13, Ubuntu 24.04, or newer).
- systemd installer script with ALSA microphone auto-detection and
  automatic BirdNET+ model download from Zenodo.

[Unreleased]: https://github.com/tomtom215/BirdNet-Behavior/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/tomtom215/BirdNet-Behavior/releases/tag/v0.1.0
