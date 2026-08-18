# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

A production-readiness pass against one question: *if this station is sealed
into an outdoor enclosure and left for a year with nobody on site, what does it
get wrong, and would anybody find out?* The full audit, with evidence and the
gates that were observed failing, is `docs/PRODUCTION_AUDIT.md`.

Several of these were invisible to a fully green 2 190-test suite.

### Fixed

- **"Still expected" read zero for the last six weeks of every year.** The
  migration page's six-week look-ahead was a day-of-year `BETWEEN` against
  `strftime('%j','now')` and `strftime('%j','now','+42 days')`. From 20
  November the end of that window falls in the next calendar year, so its day
  number is *smaller* than the start's — 20 November 2026 gives `'324' … '001'`
  — and the range matches nothing at all. The tile reported a confident "0 ·
  no overdue migrants" through the entire late-autumn arrival season, which is
  the one stretch of the year it exists for. The window is now expressed as
  real dates and the prior year's arrivals are re-based onto both this year and
  next, so crossing the boundary is just the second candidate matching. The
  same rewrite drops two smaller errors in the old form: `'now'` was UTC
  against a locally-dated column, and day-of-year is a day out between a leap
  year and a common one.

- **The migration chart's "today" line was drawn in the wrong place.** It was
  positioned by `(days since 1970 % 365) / 7`, which is not a week number: it
  ignores leap days, so it had drifted a fortnight by 2026, and it counts from
  1970 rather than from January, so on 31 December it returned week 1 and drew
  the marker at the far left of a chart whose data ends at the far right. It
  now uses the same `%W` week the chart's own buckets are grouped by, checked
  against SQLite for agreement. The page's current year is read from the
  station's local clock for the same reason.

- **Arrival dates drifted by a day whenever a leap year was involved.** The
  phenology queries derived `first_doy`/`last_doy` from a raw day-of-year, which
  from 1 March runs one higher in a leap year — 1 May is day 122 of 2024 and day
  121 of 2025. The multi-year percentiles behind the migration window averaged
  the two scales together, so every arrival and departure estimate spanning a
  leap year carried a systematic error of up to a day, and the seasonal window
  was smeared by the same amount. It was worse than noise: a species that
  genuinely advanced by one day between 2024 and 2025 had the shift cancelled
  exactly and was reported as unchanged. Day numbers are now projected onto a
  common year (1–365, with 29 February folding onto 28 February) before any
  comparison, so one calendar date is one number in every year. The ISO dates
  returned beside them were always exact and are unchanged.

- **Your edits never reached the analytics.** Deleting a detection, re-labelling
  one, approving one out of quarantine and "clear all detections" all wrote to
  SQLite alone. The DuckDB copy every behavioural and time-series dashboard
  reads is synced *incrementally*, so it could only ever add newer rows — never
  remove one, never re-read a changed one, never pick up a back-dated one. So a
  deleted false positive kept counting in Patterns forever, a corrected
  identification kept its old name, an approved quarantine detection could never
  arrive at all, and "clear all detections" left the analytics rendering your
  whole history beside a dashboard reporting zero. Nothing reported any of it:
  both stores answered every query, just with different histories.

  All four are now paired writes, and after each start the two row counts are
  compared — when they disagree the copy is rebuilt automatically. That last
  part repairs stations that already diverged, with no operator action.

- **"Today" meant UTC's today.** Five queries compared the local-civil `Date`
  column against `date('now')`. West of UTC the day rolls over during your
  evening, so the RSS/iCal "today" feed returned **nothing** for the last hours
  of every evening — 20:00 to midnight in New York. East of UTC "today" was
  still yesterday. The species sparkline was worse than a shifted window: its
  date axis was built from UTC dates and joined against locally-dated counts.

- **The dawn chorus got slower every season.** Its 30-day window was reading the
  station's entire history — SQLite preferred the species index for GROUP BY
  ordering, then built the temp b-tree anyway. Measured on a synthetic four-year
  station: 72 ms at 60 days, 1 711 ms at four years. Now a range seek: 1 613 ms
  → 27 ms, identical results.

- **A reviewer's verdict changed nothing.** `detection_reviews` has stored
  confirmed/rejected verdicts since 0.11, and exactly one panel ever read them.
  Every other analytic counted a rejected detection exactly as it counted a
  confirmed one, so a season of curation left every chart unchanged. Verdicts
  now exclude a detection from the aggregates in both stores, while the
  record-level views still show it so you can listen again and change your mind.
  Verdicts you have already recorded take effect on upgrade.

- **Live and resynced rows carried different columns.** The real-time DuckDB
  insert wrote six of twelve, so `Lat`, `Lon`, `Cutoff`, `Week`, `Sens` and
  `Overlap` were populated or NULL depending on how a detection got there.

- **Interface.** The phone layout was gated on `pointer: coarse` rather than
  width, so an iPad with a keyboard, a touchscreen laptop and a narrow desktop
  window all got the desktop nav — and the QA tooling, which sets a viewport but
  no touch emulation, had never once rendered the real mobile layout. Half the
  Patterns tabs sat off-screen on a phone with nothing signalling they scrolled.
  Chart series colours were a hash mapped to hue at constant lightness, so pairs
  landed 2–3° apart and were indistinguishable (near-certain at any realistic
  series count). The activity streamgraph had no axes at all, and the caption
  above it described a different chart. "Bursts of singing" listed sessions of
  one detection lasting zero seconds. Row controls ran off the right edge at
  360 px and below, traced to an 8 px footer overflow that was widening the
  layout viewport and dragging the fixed tab bar with it.

- The field runbook's stated memory ceiling was half the real one
  (`MemoryMax=512M` documented, `1G` shipped).

### Added

- **Imports from another station stay attributable.** Importing a BirdNET-Pi
  database used to be indistinguishable from having recorded it: no check
  mentioned coordinates or timezones, and no column separated the two
  afterwards. A merged database could silently hold two sites and two clocks,
  and every location- and hour-dependent analytic read it as one — unrecoverably,
  since nothing could tell the rows apart later.

  The import now profiles the source, warns before it runs when the coordinates
  are not this station's, offers the source station's UTC offset so both
  histories share one clock, and tags every imported row with its origin.

- **Station-health alerts.** The detection deadman answers "is the station
  detecting at all?". This answers the faults a station keeps detecting straight
  through: one microphone down while others record, a disk full enough that
  recordings are being purged, a CPU at its throttling point, a backup or
  integrity check that has not completed in weeks. One alert per episode with a
  recovery notice, after three consecutive polls so a self-healing blip stays
  quiet. On by default; `STATION_HEALTH_ALERTS=false` to disable.

- **Recording effort, and abundance corrected by it.** A detection count is a
  numerator over a denominator nobody was recording: a solar window is six hours
  longer in June than December, a week of downtime removes a week of listening,
  a failed microphone halves the channels. Each moves the count without moving a
  single bird, so comparing raw counts across seasons or years measures the
  station as much as the birds.

  The station now records how long it actually listened, per source per day, and
  `/analytics/abundance` returns detections per hour of listening. `/analytics/phenology`
  exposes per-species arrival and departure, flagging the species for which a
  calendar-year window is not a migration window — a resident would otherwise be
  reported as arriving on 1 January.

- The five operational runbooks — field deployment, security hardening, hardware
  validation, multi-stream deduplication, macOS — are now part of the published
  manual under **Running a Permanent Station**. They were repository files
  reachable only as raw GitHub links.

### Migrations

25, 26 and 27 — import provenance, the denormalised reviewer verdict (backfilled
from existing verdicts), and the recording-effort table. All additive; none
rewrites existing rows, and `import_batch_id IS NULL` continues to mean "this
station recorded it".

## [0.14.0] - 2026-08-16

### Added

- **`--migration-report`: what an upgrade would do to your history, before it
  does it.** Most migrations only change the schema around the data. Migration
  24 (below) rewrites rows already on disk and destroys its own input —
  afterwards nothing records what a detection's timestamp used to be. This
  opens the database read-only and prints how many detections would move, how
  many are left alone and why, the largest shift, how many roll onto the next
  day, and the affected date range. It changes nothing, so it is safe to run on
  a live station.

- **Every history-rewriting migration is now preceded by a backup.** Before
  migration 24 runs, the database is copied to
  `<db>.pre-migration-24.backup` with `VACUUM INTO`, so recovery is a file
  move. Existing backups are never overwritten. A backup that cannot be written
  fails the migration rather than proceeding — the rewrite cannot be undone, so
  "could not make it recoverable" has to mean "did not do it". The error names
  the space required and the escape hatch, `BIRDNET_SKIP_MIGRATION_BACKUP=1`,
  for a station whose disk genuinely cannot hold a copy and whose operator
  accepts an unrecoverable rewrite.

- **`--channel-report`: what a stereo microphone is actually delivering.** The
  model has one audio input, so two channels must become one before inference —
  today by averaging, which is harmless for coincident capsules and a comb
  filter for spaced ones. Which case a station is in depends on its microphone
  and its acoustics, so it cannot be answered anywhere but on the station.

  The report records a few seconds from the configured ALSA device and prints
  each channel's level, the inter-channel delay (with the capsule spacing it
  implies), and what each reduction would hand BirdNET: today's average, the
  louder single channel, and a delay-aligned sum. It then recommends a setting.
  Requires the service to be stopped first — an ALSA capture device is
  exclusive — and says so when the device will not open.

### Fixed

- **A stereo microphone delivering one duplicated channel was reported as
  healthy.** `plughw:` satisfies a two-channel request from a one-channel
  device by copying the channel, and the copy scores perfectly on every measure
  `--channel-report` and `stereo-check.sh --alsa-test` take — correlation
  1.000, zero delay, averaging costs nothing. Both tools called that a
  well-matched coincident pair and told the operator there was nothing to fix,
  which is the opposite of the truth: the second capsule is not reaching the
  software at all.

  Both now check whether the channels are bit-identical before anything else.
  Two capsules never agree sample for sample — each carries its own self-noise
  — so exact equality means one channel copied. Both also point at
  `arecord -D hw:N,M --dump-hw-params`, which asks the hardware with the plug
  layer out of the path; `stereo-check.sh` runs it up front and says plainly
  when the device reports one channel.

- **`--channel-report` discarded `arecord`'s diagnosis.** `Channels count non
  available` (the device is not stereo) and `Device or resource busy` (stop the
  station first) are opposite problems, and both rendered as the same generic
  guess. `arecord`'s own words are now shown first. Its stderr was also piped
  and never read, which would deadlock the report if `arecord` ever filled a
  pipe buffer.

- **The "delay-aligned sum" row was never a sum.** It averages — the measured
  ratio for two aligned identical channels is 1.0, not 2.0. The label, the
  field name and the documentation all said otherwise. Averaging is the right
  behaviour, since summing would add a constant 6 dB that reads as recovered
  signal and is not, so the report now names what it does.

- **A mutation-testing gate had been passing without testing anything.** The
  `sqlite/queries/detections.rs` matrix row kept naming a file that 0.7.2 split
  into a directory, so the job matched no source, produced no mutants, and the
  threshold step read the empty result as "0 missed". A run that generates no
  mutants now fails outright on pushes, cron and manual dispatch, so the next
  stale path announces itself instead of going quietly green. Pull requests are
  exempt, where `--in-diff` makes an empty result legitimate.
  `crates/birdnet-db/src/migration.rs` also joins the matrix.

- **A detection's timestamp is now when it was heard, not when its recording
  started.** A 15-second segment is five 3-second chunks, and all five were
  stamped with the file's start second. `chunk_offset_secs` held the difference
  and the detections API does not return it, so one continuous song produced
  five rows identical in every displayed field — which is exactly what "repeated
  detections" looked like. It also put five *simultaneous* detections into
  `detection_timestamp`, which sessionisation, gap analysis and the dawn-chorus
  curve all group on.

  BirdNET-Pi has always added the offset, in the same place (`Detection.__init__`:
  `file_date + timedelta(seconds=self.start)`), so this table has been holding
  two conventions at once: imported BirdNET-Pi rows with chunk-accurate times,
  natively recorded rows without. The pipeline now adds the offset at inference,
  rolling the date when a chunk crosses midnight, and **migration 24 repairs
  history already on disk** from the stored offsets — so the whole table ends on
  one convention. Rows whose `Date`/`Time` name no point in time are left
  untouched rather than turned into an invented timestamp.

  Row *counts* do not change, and were never wrong: BirdNET-Pi has no UNIQUE
  constraint on `detections` at all and stores one row per chunk exactly as this
  does.

- **The Audio page's Left and Right channel options did nothing.** Both
  collapsed to `channels: 1` at the capture source and were never distinguished
  again, so all three of Mono, Left and Right produced byte-identical captures.
  They now select the channel they name: the device is opened with both, and the
  capture tee keeps the requested half, so the segments written to disk are
  single-channel and nothing downstream needs to know a choice was made.

  This matters because of what `Stereo` does. Both channels are kept and the
  decoder averages them to the mono BirdNET requires — which for a **spaced**
  pair is a comb filter, not a noise reduction. Measured through this project's
  own decode path: one wavefront reaching the capsules half a period apart loses
  about 66 dB to cancellation, a quarter period costs 3 dB, and the notches move
  with the bird's direction. A coincident pair is unaffected. Selecting a
  channel is the mitigation, and it was the one thing the UI offered that had
  never been wired up.

  Not a regression: BirdNET-Pi defaults to `CHANNELS=2` and uses
  `librosa.load(mono=True)`, which averages identically. A stereo source now
  says so on the Audio page and warns once in the journal at start-up.

- **The analytics dashboards were blank, and nothing anywhere said why.** Two
  independent defects, both invisible to a green CI matrix, both only reachable
  on a real station.

  The first is the one that emptied them, and it emptied them **permanently**:
  a station reported dashboards blank for days. Every analytics query filters on
  a look-back window, which reaches DuckDB as
  `detection_date >= CURRENT_DATE - INTERVAL n DAYS`, and `CURRENT_DATE` lives
  in DuckDB's ICU extension — as does every other way to name the current local
  date: `today()`, the `TimeZone` setting, and even `CAST(now() AS DATE)`, which
  fails with `Unimplemented type for cast (TIMESTAMP WITH TIME ZONE -> DATE)`.
  There is no ICU-free spelling to fall back to.

  ICU is **not** statically linked into the `libduckdb` that `duckdb-rs`
  bundles. It reports itself `installed` on a connection that has already
  autoinstalled it, which is what an earlier reading of this — and the first
  version of the fix — was built on. Measured properly, with autoload and
  autoinstall off and no local cache, `duckdb_extensions()` reports `icu` as
  `installed=false, NOT_INSTALLED`, and `LOAD icu` fails outright.
  (`core_functions`, by contrast, genuinely does report `STATICALLY_LINKED`,
  which is why `strftime` and `date_diff` kept working throughout.)

  So DuckDB has to fetch it, and it does that by autoinstalling into
  `$HOME/.duckdb`. The shipped systemd unit sets `ProtectHome=read-only`. The
  station's journal:

  ```text
  Failed to create directory "/home/pi/.duckdb": Read-only file system
  ```

  Every analytics query failed from then on, and the store's `birds.duckdb`
  never appeared. Two things attempt that write — ICU autoinstalling, and stage
  2 of the behavioral loader (`INSTALL behavioral FROM community`) — so both are
  fixed at the source: **DuckDB's extension directory now sits beside the
  analytics database**, inside `DATA_DIR` and therefore inside the unit's
  `ReadWritePaths`, instead of under `$HOME`.

  On top of that, **the ICU binary is now embedded in the release binary** the
  same way the `behavioral` extension already was, and loaded from it at open.
  That removes the network *and* the writable `$HOME` from the path entirely, so
  an air-gapped station gets correct local dates on its first query. Release,
  CI and Docker builds all fetch it per target; `build.rs` refuses to embed
  bytes whose footer it cannot parse, and now also refuses bytes built for a
  different platform than the one being compiled for — cargo does not tell a
  build script which DuckDB version will be linked, but it does tell it the
  target triple, and 20 MB of unloadable ICU is worth catching at build time.

  There was a timing bug underneath all of that too, and it is still fixed:
  even where the autoinstall *could* write, DuckDB resolves ICU while binding
  the query that first needs it, too late for that query. Attempt 1 failed,
  attempts 2–4 passed. One failed query per restart would have been survivable,
  except the web layer maps a query error to a rendered "Analytics temporarily
  unavailable" fragment and caches that fragment for ten minutes — so the first
  page visit after every restart poisoned the cache. ICU is loaded when the
  store opens, before any query runs.

  The test that was supposed to cover this went green against the broken
  implementation, because an earlier probe on the same machine had populated
  `~/.duckdb`; moving that cache aside was what exposed it. Its replacement
  turns both escapes off explicitly — autoload and autoinstall disabled,
  extension directory pointed at an empty one — so the embedded bytes are the
  only route `CURRENT_DATE` has, and a separate gate pins the extension
  directory to the data directory. Verified against the previous code, where
  the first fails with `Catalog Error: … "current_date" is not in the catalog`
  and the second sees DuckDB's default (an empty string).

  The time-series execution gate had caught the same disease from the same
  cache. It opened a bare DuckDB connection and issued `LOAD icu` itself, as an
  approximation of what the application does — and a bare `LOAD` never
  autoinstalls (DuckDB only does that while binding a query that needs the
  extension), so it passed only when *some other test binary in the same run*
  had populated `~/.duckdb` first. It now opens a real `AnalyticsDb`, which is
  literally what `birdnet-web` hands these queries, and drops its private copy
  of the `detections_ts` view along with it.

  The second survives dirty history rather than a cold start. `Date` and `Time`
  are free-form `TEXT NOT NULL` — the column type forbids NULL, not nonsense —
  and the BirdNET-Pi importer turns a NULL `Date` into `""` and copies
  malformed values through verbatim. `detections_ts` cast them with a plain
  `CAST`, and DuckDB raises `Conversion Error` for the *whole query*, so one
  unplaceable row anywhere in a multi-year import took down every behavioural
  and time-series dashboard at once. The view now uses `TRY_CAST`: such a row
  falls out of the time-bucketed results instead of aborting them. Coercing to
  an epoch default was rejected — it would invent detections on 1970-01-01.

  Neither could have been caught by the tests that existed. The time-series
  crate's sixteen public queries had no execution coverage at all: every test
  built a SQL string and asserted it *contained* the right substrings, which a
  query DuckDB refuses to bind passes exactly as well as one that works. There
  is now a gate that executes all sixteen against a real DuckDB and requires
  rows back, plus gates for the cold-start bind and the unplaceable row.

- **Ten of the eleven `phenology` query builders emitted SQL DuckDB refuses to
  run.** `birdnet_behavioral::phenology` is a public API documenting a
  SQLite/DuckDB compatibility matrix, but it emitted `strftime('%Y', Date)` —
  SQLite's `strftime(format, value)` argument order — against DuckDB, which
  takes `strftime(value, format)`. Every query using it failed to bind with
  "Could not choose a best candidate function". `phenology_timing_sql` also used
  `julianday`, which DuckDB does not have, and two builders assembled their
  `WHERE` clause by giving each condition its own `WHERE `/`AND ` prefix, so an
  absent species filter left a dangling `AND` straight after `FROM` — a parser
  error.

  The builders now emit DuckDB SQL, read `detections_ts` so `detection_date`
  arrives typed (and unplaceable rows are excluded rather than grouped under a
  NULL year), and assemble the `WHERE` clause from a list of conditions, which
  makes the dangling-`AND` shape unrepresentable. The compatibility matrix has
  been replaced with the truth: these target DuckDB.

  No dashboard was affected — nothing calls these, and the web phenology card is
  SQLite-backed — but the tests asserted only on generated *text*
  (`sql.contains("month")`), which a query no engine will run passes just as
  well as one that works. `tests/phenology_execute.rs` now executes all eleven
  against a real store; it fails on ten of them against the previous code.

- **The embedded-extension check ignored the platform.** A DuckDB extension is
  locked to a platform as well as a version, and the two fail identically at
  `LOAD`, but `embedded_extension_mismatch()` compared only the version — so
  `linux_amd64` bytes embedded in an `aarch64` build agreed on `v1.5.5`, passed
  the check, and then failed to load on the Pi with nothing having warned. Both
  properties are now compared (the engine's own platform comes from
  `pragma_platform()`, which uses the same identifiers the extension registry
  publishes under) and the report names which one disagrees. A platform that
  cannot be read on either side is not treated as a mismatch, so missing
  information cannot manufacture a false alarm. `release.yml` already selected
  the extension per target, so this gap was reachable from local and cross
  builds — which is exactly what a maintainer tests an air-gapped station with.

- **`scripts/hardening-check.sh` could bind-mount over the host as root.** The
  script re-execs itself under `unshare -rm` and carries a guard meant to abort
  if that did not happen, because everything after it bind-mounts over `$HOME`,
  `/usr` and `/tmp` and then deletes its working directory on exit. The guard
  compared the caller's mount namespace against PID 1's, and refused only when
  the two were *equal*. `/proc/1/ns/mnt` is unreadable to a process whose PID 1
  is a sandbox supervisor rather than real init — ordinary in CI containers and
  nested sandboxes — and `readlink` then yields the empty string, which never
  equals a real namespace id. The guard therefore failed **open** on precisely
  the environments it existed to protect: measured in one such container, it
  returned "proceed" in all four cases tested, including the host mount
  namespace as root. It is now a token handed down by the re-exec — the parent
  records its own namespace and the child refuses unless it is demonstrably in
  a different one — so anything that cannot be positively confirmed is a
  refusal. This only ever affected maintainers running the script; it is not
  installed on a station.

### Added

- `GET /api/v2/analytics/status` reports the analytics **store**, not just the
  build flags. `analytics_compiled` and `analytics_configured` are both `true`
  on a station whose dashboards are empty — they describe intent, and stay true
  through every way this actually fails. The new `store` object carries
  `extension_loaded`, the DuckDB row count, `unplaceable_detections` (rows no
  dashboard can place in time), the engine's own DuckDB version and platform,
  and the embedded extension's version, platform and any mismatch — including
  which property disagrees. It is `null` on a slim build, so "no analytics here"
  stays distinguishable from "analytics present but broken".

### Changed

- BirdNET-Pi import validation no longer claims malformed rows "will be
  skipped". Nothing skipped them: they were imported, counted, and then absent
  from every date- or time-based analytic. The check also missed the cases that
  mattered — it sampled only the first 1 000 rows, never looked at `Time`, and
  could not see a NULL `Date` at all, because `NULL NOT GLOB …` is NULL rather
  than true. It now scans the whole table, inspects both columns, and says what
  actually happens to the rows.

- `scripts/setup-onnxruntime.sh` works against current `ort-sys` again. Its dist
  table was renamed `dist.txt` → `dist.tsv` and had its columns reordered with a
  header added, so the script failed with "ort-sys not found" and cold builds
  behind a TLS-intercepting proxy — sandboxed CI, Claude Code on the web — could
  not fetch ONNX Runtime. It now accepts either filename and identifies columns
  by content rather than position.

## [0.13.1] - 2026-08-13

### Fixed

- **A re-imported BirdNET-Pi database silently doubled itself.** Every
  duplicate-suppression path rests on `idx_detections_unique`, and `File_Name`
  is part of that key and nullable — and SQLite considers NULLs distinct in a
  UNIQUE index. A row with no filename conflicted with nothing, so
  `INSERT OR IGNORE` ignored nothing.

  The CSV/TSV path made it easy to hit: an empty `File_Name` field, `\N`, the
  literal `NULL`, or a row with fewer than twelve columns all yield SQL NULL.
  Re-importing the same export doubled those rows and reported "imported N,
  skipped 0" as success. Anyone who re-ran an import after a failure — the only
  recovery available, since batches commit as they go — doubled their history
  and had every dashboard, rate and analytic computed over it.

  Migration 23 makes the key NULL-insensitive via `COALESCE(File_Name, '')`,
  and **repairs databases that already carry duplicates** by collapsing each
  group to its earliest row. `File_Name` itself stays nullable, because NULL is
  meaningful there — it distinguishes "never had a clip" from "reclaimed"
  (migration 22), and `locks.rs` filters on it.

  Regression tests now cover the SQLite path, the CSV path (using the shipped
  fixture's own rows, which are exactly the NULL-`File_Name` kind), the
  migration's repair of pre-existing duplicates, and — new — the operator's
  actual HTTP journey: upload, poll progress to completion, upload again, and
  assert the row count did not move. Verified against the pre-fix index, where
  it fails with 6 rows where 4 were expected.

  Found while auditing the weekly report: its fixture seeded two detections of
  one species at the same second with no clip, which the corrected key rightly
  calls one detection. That is inflation of exactly the kind this bug produced,
  living in a test.

- **Listen → Live appeared to do nothing, because the button cancelled the
  stream you were waiting for.** `audio.play()` sets `paused` to false
  synchronously but resolves only once the browser has buffered enough to start
  — around a second, since ffmpeg must fill a frame before the first MP3 bytes
  leave the station. The button kept reading "Listen (audio)" for that whole
  window, so the natural response to apparent silence — clicking again — landed
  in the stop branch and killed the stream that was about to start. Clicking
  through that cycle is indistinguishable from live audio being broken, and that
  is how it was reported on 0.13.0.

  The button now shows **Connecting…** and ignores clicks until `play()`
  settles, and an `error` on the element reports "Stream unavailable — retry"
  rather than stranding it mid-connect. `-flush_packets 1` on the encoder halves
  time-to-first-audio (measured through the shipped invocation: 1.13 s → 0.59 s),
  shrinking the window in which the trap could spring at all.

  Nothing was wrong on the server: the tap, the source resolution, the segment
  writer and the MP3 encoding were all delivering correctly throughout.

- **A failed live stream now says why.** ffmpeg's stderr was sent to
  `/dev/null`, so every failure — `Device or resource busy`, an unknown filter,
  a missing codec — reached the operator identically: a `200` response carrying
  no audio and an empty journal. It now runs with `-loglevel error` and its
  stderr is drained to the log, and a stream that ends having delivered zero
  bytes says so. Diagnosing the bug above took three refuted hypotheses for want
  of this one log line.

- **The same trap in both clip players.** The detail-page player swapped to its
  pause icon before `play()` resolved and never handled a rejection, so a clip
  that could not start (autoplay policy, decode error, a clip deleted under it)
  showed a pause icon over silence with an unhandled promise rejection. The
  Recordings row player had the cancelling variant: a second click on a clip
  still loading paused the clip being waited for. Both windows are short for a
  local file — but not zero, and a cold cache or a busy Pi widens them.

- **Listen → Live could strand itself on "Connecting…".** A stream that connects
  but never buffers enough to start fires `stalled`, not `error`, so `play()`
  can stay pending indefinitely — and ignoring clicks while that is true would
  have left the button permanently dead. It now gives up after 20 s and hands
  control back.

- **Bulk clip actions could fire twice and could report success after failing.**
  The lock/delete batch had no in-flight guard, so a second click during a slow
  batch re-sent the whole thing; and because `fetch` resolves for 4xx/5xx, a
  batch that failed outright still reloaded the page as if it had worked. The
  batch is now single-flight and checks each response, reporting how many clips
  could not be updated.

- **Two concurrent restores can no longer run over the live database.** A
  restore unpacks an archive over `birds.db` and the recordings directory, takes
  minutes, and shows nothing while it runs — the same conditions that get a
  button clicked twice. htmx does not dedupe in-flight requests unless told to,
  and nothing on the server refused the second one. The endpoint now rejects a
  concurrent restore outright (a UI guard cannot bind a client that simply POSTs
  twice), and the form and the destructive "clear" controls disable themselves
  while their request is in flight.

### Added

- **An interaction gate in CI** (`tools/visual-qa/interactions.mjs`). Every bug
  above is one the existing suite could not have caught: the server was correct,
  the pages rendered, axe was clean and every screenshot looked right — the
  defects lived entirely in what the *second* click did, and nothing anywhere
  drove a control twice. The gate drives controls the way an impatient operator
  does and asserts they neither cancel nor duplicate their own in-flight work.
  Verified against the shipped 0.13.0 build, where it reproduces the reported
  bug (`pause()` during connect) and catches the bulk batch firing twice.

  The visual-QA fixture no longer rate-limits itself. It is deliberately
  hammered — 152 page captures back to back, plus the new gate driving controls
  as fast as Chromium will go — and the station's 30 req/s limiter throttled the
  harness rather than the product, surfacing as an intermittent `429` on a font
  and a red build. **The station's own limiter is unchanged**: measured, a cold
  dashboard load is 24 requests, the heaviest page 34, and two rapid loads 48 —
  all inside the 60-burst default with no `429`, so there was nothing to loosen
  for real clients. A test now pins that the shipped router keeps the strict
  default, since the opt-out is what makes losing it possible.

### Changed

- `/stream` no longer sets `Transfer-Encoding` by hand. It is a hop-by-hop
  framing header the HTTP layer owns: hyper already chunks a streaming body and
  emits the header itself — verified on the wire, where setting it changed
  nothing but header order — and HTTP/2 forbids it, so a station behind an h2
  reverse proxy would have the response rejected for carrying it.

## [0.13.0] - 2026-08-13

### Changed

- **Live audio now comes from capture itself instead of a second microphone
  open, so it works on a single-microphone station at all.** An ALSA `plughw:`
  device is exclusive: on the Raspberry Pi 4 under test,
  `ffmpeg -f alsa -i plughw:CARD=PRO,DEV=0` returns `Device or resource busy`
  for as long as `arecord` is recording — which, on a station doing its job, is
  always. `GET /stream` did exactly that second open, so Listen → Live could
  never play on the commonest build there is.

  `arecord` no longer segments for us. It streams raw PCM into the process and a
  reader thread drives two consumers: the rotating WAV writer that used to be
  `arecord --max-file-time --use-strftime`, and a bounded live tap that
  `/stream` subscribes to. The tap is **lossy on overflow** and never blocks, so
  a stalled listener cannot backpressure recording — losing live-monitoring
  audio is a click in someone's headphones; losing recorded audio is a detection
  that never happens. Filenames are byte-identical to the ones `arecord`
  produced, including their **local** civil time, which the supervisor now
  refreshes every tick so a station keeps naming files correctly across a
  daylight-saving change it never restarts for.

  `/stream` for a source that is not recording — paused by the schedule or by a
  quiet window, or down — now answers `503` with that explanation, instead of
  holding a connection open producing nothing.

  RTSP and PipeWire sources are unchanged: a second RTSP session is normal and
  PulseAudio permits concurrent opens, so neither has the problem this solves.
  macOS microphone capture is also unchanged (ffmpeg/avfoundation), because
  there is no macOS runner in CI and no macOS hardware behind this change.

- **Per-source capture gain no longer needs ffmpeg, and no longer lies about
  it.** `arecord` has no gain control, so a gain-configured microphone used to
  be captured by `ffmpeg -f alsa` and its `volume` filter — but
  `required_tool()` still reported `arecord` for that source, so a station with
  gain set and no ffmpeg installed passed the availability check and then failed
  to spawn. The gain is now applied to the samples in-process (clipping, as the
  ffmpeg filter did) and that second capture backend is gone.

- **"Station" in the navigation is now "Settings"**, and its inner Settings tab
  is "General". The section is what operators go looking for when they want to
  configure the station; `/station` URLs are unchanged and "station" remains a
  command-palette keyword.

- **Live spectrogram frames now carry a `source`.** The broadcast sends every
  source's frames to every client and they previously carried no attribution, so
  the Listen source picker could not filter and a multi-source station drew both
  inputs into one spectrogram.

- **`LabelSet` retains the `class` column** from the BirdNET+ V3.0 CSV, so
  non-bird taxa (the model is a 11K global classifier, not birds-only) can be
  distinguished from birds rather than appearing as a scientific name with no
  common name.

### Fixed

- **The dashboard's day strip drew "now" and sunrise/sunset on a UTC axis while
  its bars were local.** Detections are timestamped by `arecord --use-strftime`,
  which is local, and `hourly_activity` buckets that `Time` column — but the
  marker came from a raw `epoch % 86400` and the solar times from
  `sunrise_utc_min`. On a CEST station the marker sat two hours behind the
  detections beside it and the hero pills read "sunrise 4:10" for an 06:10
  sunrise. `today_date_string()` was UTC for the same reason, so for the first
  hours of each local day the Today page queried the wrong date entirely.
  The offset now comes from SQLite's `localtime` (no date/time crate in the
  workspace, and `unsafe` is forbidden), cached for a minute.

- **The setup wizard could not display any setting the station already had, and
  silently overwrote two of them.** Latitude and longitude had no `value=`
  attribute and the confidence/notification fields were hardcoded in the markup,
  so a station configured at install time rendered a blank wizard. Because the
  hardcoded fields are never empty they slipped past `onboarding_save`'s
  skip-if-blank guard and were written on every completion: an operator who had
  set `CONFIDENCE=0.6` had it reset to 0.75 by clicking through setup.

- **The installer discarded typed coordinates without saying so.** The prompt
  told the operator to read coordinates off OpenStreetMap — which hands over a
  *pair*, `49.4521, 8.6724` — then offered a single-value field whose validator
  rejected exactly that, warned once, and continued. A decimal comma
  (`49,4521`) was rejected too, though the web settings form accepts it. The
  prompt now parses both shapes and re-prompts on bad input, like the
  audio-source prompt above it.

- **"first today" was shown on every detection of a species, not the first
  one.** The badge compared a species' first-ever *date* to today, which is true
  of all of that day's detections — a station that heard 133 blackcaps on their
  arrival day badged all 133. Now keyed on the first-ever instant, so exactly
  one detection can carry it, and renamed "first ever" since only one row can
  hold it.

- **The live spectrogram decoded every clip while it was still recording.** The
  producer decoded on the watcher's create event after a fixed `sleep(100ms)`,
  against segments `arecord` writes for fifteen seconds — so every frame failed
  with "unexpected end of file" and the dashboard showed "idle" on a healthy
  station. The detection daemon already had the right rule and its own docs
  named this exact error; it was private to that module, so it is now shared in
  `crate::file_settle`.

- **Live audio needed ffmpeg that no microphone station ever installed.**
  `GET /stream` shells out to ffmpeg for every source kind including plain ALSA,
  but the installer ensured it only for RTSP capture and `--doctor`'s check was
  gated on the same condition — so a Linux station with a USB microphone
  returned 500 on every request while reporting itself entirely healthy.

- **The browser tab had no icon.** The PWA manifest and `apple-touch-icon` were
  present, but with no `rel="icon"` the browser fell back to `/favicon.ico`,
  which is not routed.

## [0.12.0] - 2026-08-10

### Fixed

- **`RECORDING_SCHEDULE` in `birdnet.conf` was ignored: a station set to
  `solar` recorded around the clock.** `capture::schedule` read
  `cli.recording_schedule` directly, and that flag carries a clap
  `default_value` of `all-day` — so the default always won and the configured
  schedule never applied. A `fixed:HH:MM-HH:MM` window was dropped just as
  silently.

  Nothing contradicted it. `birdnet_core::config::validate` validates the key,
  and `--doctor`'s clock check reads it from the config to warn that a fixed
  window is evaluated in UTC — so the diagnostic reported on a schedule the
  runtime never used, the same shape as the `CADDY_PWD` and `ALSA_CARD` splits
  fixed earlier in this release. Its sibling `resolve_twilight_offsets` had
  always gone through `resolve::setting`; this one line had not, and every
  existing test set the CLI field by hand, exercising only the path that
  worked.

  Measured before the fix: `RECORDING_SCHEDULE=solar` yielded
  `night_inhibit=false, fixed_window=None` — 24/7 recording, on a station whose
  operator had asked for the dawn window and whose disk and CPU paid for it.

### Removed

- **`--quality-filter` and `--quality-min-snr`, which did nothing at all.**
  They promised that "audio chunks are assessed for SNR, spectral flatness, and
  rain/wind interference before being passed to the ML model". No code read
  either field — not from the config, not from the CLI. The feature was
  advertised in `--help`, in the generated CLI reference and in the tuning
  guide, and was inert.

  The implementation is not missing: `birdnet_core::audio::quality` is ~1300
  lines of SNR, spectral flatness, rain/wind assessment and noise-floor
  tracking, with benchmarks — it was simply never called by the detection
  pipeline. Wiring it changes which chunks reach inference, so it belongs in
  its own change with hardware validation behind it rather than a release-prep
  pass. The flags are gone until then, because a switch that silently does
  nothing is worse than no switch: an operator in a noisy garden would set it
  and believe their false positives were being filtered.

### Added

- **Four settings that were command-line-only are now on the settings page.**
  An operator without a terminal — which is most of them — could not reach any
  of these:

  | Setting | Why it matters |
  |---|---|
  | **Recording window** (`RECORDING_SCHEDULE`) | all-day / solar / fixed hours; the page offered the sunrise and sunset *offsets* while the mode they modify was unreachable |
  | **Heartbeat URL** (`HEARTBEAT_URL`) | lets an outside monitor alert you when the station stops reporting |
  | **Dead-man alert** (`DEADMAN_HOURS`) | notifies you after N hours of silence — the symptom of a microphone that died quietly |
  | **Common-name language** (`DATABASE_LANG`) | a non-English station could not pick its own language from the UI |

  Each goes through the existing wiring guard, and a test walks the whole chain
  per key — the settings row the form writes, through the overlay, to the
  config key the consumer actually reads — with a further test proving that
  choosing *Solar* on the settings page really does stop overnight recording.
  Both fail against the pre-fix code.

  MQTT and Home Assistant discovery (8 flags) remain command-line-only and are
  deliberately deferred; `docs/RELEASE_PLAN.md` § 5 records the rest of the
  audit.

### Changed

- **The hardware harness now measures CPU, and checks the dashboard's CPU
  figure against the kernel's.** Reported as looking broken on a Pi. It could
  not be reproduced: measured against `/proc/stat` over the same window the
  reading agrees exactly — 2 % against 2 % idle, 100 % against 100 % with every
  core pinned. But the report pointed at a real gap. `scripts/hardware-test.sh`
  recorded load average and never a utilisation figure, so no run on real
  hardware had ever established that the CPU monitor worked at all; and the
  unit tests only asserted `0.0 ≤ cpu ≤ 100.0`, which a sampler stuck at zero
  satisfies.

  The `perf` phase now samples CPU utilisation into `perf-samples.csv`, reports
  mean and peak, warns when the peak leaves no headroom, and compares the
  figure the Station page displays with `/proc/stat` — failing outright if the
  dashboard shows 0 % on a busy board. A unit test now pins the machine's cores
  and requires the reading to move, which the old range assertions could not.

- **The out-of-the-box minimum confidence is now 0.75** (was 0.70, BirdNET-Pi's
  default). High enough that a new station's log reads as realistic instead of
  padded with marginal IDs, low enough that quiet and distant birds are still
  recorded. It remains a single shared constant, so the daemon, the settings
  form and the wizard cannot disagree about it; existing stations with an
  explicit `CONFIDENCE` are unaffected.

### Added

- **The setup wizard now asks how picky the station should be.** The minimum
  confidence decides whether anything is recorded at all, and nothing in the
  setup path mentioned it: the installer wrote it as a commented-out line and
  the wizard never raised it, so an operator who wanted stricter or looser
  detection had to find Settings → Detection unprompted. A new **Accuracy** step
  offers four presets (0.90 / 0.75 / 0.60 / 0.40) pre-selected on the shared
  default, so clicking straight through yields exactly what the daemon would
  have enforced anyway.

  The submitted value is range-checked before it is stored. An out-of-range
  `CONFIDENCE` is a *fatal* doctor error, and `--doctor` runs from the unit's
  `ExecStartPre` where exit 2 blocks startup — so an unvalidated write here
  would have turned the setup form into a way to leave the station unable to
  start.

### Fixed

- **The setup wizard showed a station that did not exist.** Its Microphone step
  was a mock-up: a hard-coded "UMC202HD · USB audio · card 1 · 48 kHz" card,
  marked *recommended* and pre-selected, described as "detected automatically";
  a "Built-in microphone · card 0 · 44.1 kHz"; and two more cards offering an
  RTSP camera and folder-watching that did nothing when clicked. The final
  summary card was the same — "Boston, MA · 42.36, −71.06", the same UMC202HD,
  and a dashboard address of `http://birdnet.local/` that does not resolve on
  every network.

  None of it was read from the station. A first-run operator was shown hardware
  they do not own, presented as already found — so on a station whose
  microphone was missing or misconfigured, the wizard's answer to *"will this
  hear anything?"* was a confident yes about a device that is not there. That
  is the failure mode the wizard exists to prevent.

  The Microphone step now renders the real rows from `audio_sources`, reusing
  the Capture tab's own `kind_label`/`detail_for` rather than a second copy that
  could drift, and a station with no source is told plainly that nothing will be
  detected and pointed at where to add one. The summary rows that depend on
  operator input are placeholders the page script fills — location from the
  coordinates actually entered, alerts and confidence from the cards actually
  chosen, and the dashboard address from the URL the operator actually reached
  the page on. Verified by driving the wizard end to end in a real browser, and
  a test pins every one of the removed mock strings so none can reappear.

  Two counts went stale when the Accuracy step was added and nothing would have
  caught either: the welcome copy still read "five steps", and
  `tools/visual-qa/onboarding.mjs` looped to a hard-coded `step <= 5`, so its
  screenshot set silently stopped one short — looking complete while missing
  exactly the new step worth reviewing. The prose count is now asserted by a
  test and the capture script reads the count from the page. Re-audited with
  axe-core across all six steps (stricter than the CI gate, which only ever sees
  the visible first step): no WCAG 2.1 A/AA violations outside the two rules the
  gate defers by design, and no horizontal overflow at 390 px in either theme.

- **Green ticks and a green "Healthy" badge on a station that was not working.**
  Walking the first-run journey end to end turned up four places that reported
  success without checking anything:

  * The dashboard's **"Getting ready"** card — the one thing a brand-new
    operator reads — ticked *Microphone detected* as soon as a source existed in
    the database, which says nothing about audio flowing. A source whose device
    vanished on reboot, or whose `arecord` had died, ticked green. It now reads
    the supervisor's own per-source gauge (the signal the Capture tab already
    used) and reports *Microphone not recording* with a link to the page that
    can fix it.
  * The same card's **"Room to record"** row was a hard-coded `✓`. The
    percentage and the wording were real, so it could render "Room to record ✓ —
    nearly full — 97% used": a pass tick on a station about to stop recording.
  * **"Model loaded … ready"** asserted runtime state the page has no signal
    for. It now says only what is true — the model ships with the app.
  * The **"recording"** pill is driven by time since the last detection, which
    is `None` on a station that has never detected anything — so it rendered a
    confident green *recording* forever on exactly the first-run station whose
    microphone never worked. It now consults the capture gauge first.

  The **header health badge** was the same problem at the top of every page:
  "Healthy" meant nothing more than "SQLite is not corrupt", so a station with a
  dead microphone and a 99 %-full disk showed green on every screen. It now
  grades database, capture and disk — the three things that stop detections —
  and names the problem (*Mic down*, *No microphone*, *Disk full*) with the
  reason on hover. The `data-health` token keeps its `ok`/`warn`/`err`
  vocabulary, and the disk threshold is shared with the dashboard so the two
  surfaces cannot disagree about the same disk.

- **The setup wizard's alerts choice governed nothing.** The Alerts step wrote
  `notification_mode` — a key no code anywhere read. An operator picked "Quiet"
  or "Everything" on their first day and it changed nothing, because the
  notification filter reads `notify_trigger` (bridged onto `APPRISE_TRIGGER`).
  Worse, its four options (`quiet`/`rare`/`daily`/`everything`) matched none of
  the three values the runtime understands, and `TriggerMode::parse` maps
  anything unrecognised to *every detection* — the chattiest mode, the opposite
  of a quiet choice.

  The step now offers exactly the three real modes, writes the key the runtime
  reads, and rejects anything else rather than silently selecting "chatty". It
  also says plainly that nothing is sent until a channel is configured, and
  links to where — replacing a "Pick channels now" disclosure that opened
  twelve non-interactive pills.

  The guard that exists to prevent exactly this (`SETTING_SPECS` must classify
  every settings key, enforced by a test) only ever covered the admin *form*, so
  the wizard wrote outside it. It now covers the wizard's keys too, and a test
  pins the declared list against what a full submit actually persists.

- **The timezone the wizard detected was stored and never used.** It cannot be
  applied from the app — the timezone is a system setting and the service does
  not run as root — but it is not cosmetic either: capture names each recording
  from the system's local time, and those filenames become every detection's
  `Date` and `Time`. A Pi left on UTC in a UTC+2 country files its dawn chorus
  two hours early, rolls "today" over at the wrong moment, and deletes by the
  wrong day. Raspberry Pi OS images default to UTC, so this is a common state.
  `--doctor` now compares the host's timezone with the detected one and hands
  over the exact `timedatectl set-timezone` command. Verified on a real
  container: a station configured for `Europe/Berlin` on a `Etc/UTC` host warns
  with that command.

- **`--doctor` was silent about a confidence threshold that guarantees a
  false-positive firehose.** Validation rejected the percentage mistake
  (`CONFIDENCE=70`) and non-numeric junk as errors, but a *decimal* slip — `0.07`
  for `0.7`, or a `0` copied from `SF_THRESH`, where `0` does mean "disabled" —
  parses, sits inside 0–1, and passed clean. The station then records the
  model's best guess for every three-second window: the disk fills, the species
  list fills with noise, and nothing anywhere says why. Verified against a live
  binary before and after; `0`, `0.001` and `0.07` each now warn while `0.1` and
  above stay silent, and the value remains usable rather than blocking startup.

- **`ModelConfig::default()` carried a third confidence threshold.** It
  hard-coded `0.25` — contradicting both the daemon's enforced default and the
  value the admin form advertises, which is precisely the drift the shared
  constant exists to prevent. Nothing shipped broken, because the daemon always
  names the field explicitly, but any future construction that spread
  `..ModelConfig::default()` without it would have silently reopened the exact
  bug. It now references the shared constant.

- **A station with no coordinates silently disabled species filtering.**
  `SpeciesFilter::filter_species` takes `Option<(lat, lon)>`; with `None` the
  metadata model cannot run, so occurrence filtering is skipped and **every one
  of the ~11 000 species stays a candidate**. The station keeps working and
  reports birds that have never occurred within a thousand miles — which reads
  as a bad model rather than as a missing setting.

  Nothing said so. The config validator checked that a latitude was *in range*,
  and warned when one of the pair was set without the other, but was silent
  when both were absent. `--doctor` now reports it, naming the consequence
  rather than the missing key, and pointing at the dashboard's location detect.

  Resolution goes through `daemon::resolve_station_coords` — the same function
  the detection daemon uses — rather than a third copy of the precedence rule,
  and falls back to the `settings` table because `--doctor` runs from
  `ExecStartPre` before the settings overlay has merged `/admin/settings` into
  the config. Reading the config alone would have warned at exactly the
  operators who configured their station the easy way, through the onboarding
  wizard.

  The installer was the other half of the same silence. Its summary warned
  loudly about a missing audio source and said nothing about missing
  coordinates, and its next-steps list called them "(Optional)" — while the
  location prompt itself is skipped entirely on a non-interactive install
  (`BIRDNET_NONINTERACTIVE=1`, or no TTY under `curl | sudo bash`) and on every
  re-install over an existing config, making "no coordinates" the common state
  rather than the rare one. It now says so, in the same place and tone as the
  audio-source notice.

Found by running the new on-device acceptance harness
(`scripts/hardware-test.sh`) against a Raspberry Pi 4 on Pi OS Trixie — the
"real Raspberry Pi hardware" gap `docs/RELEASE_PLAN.md` § 5 had carried open for
three releases — except where a bullet says otherwise. None was reachable from
CI: each needs a real systemd unit, a real USB microphone, or both.

- **A microphone vanished from the admin page about eight seconds after it
  loaded.** The status pill polls `/admin/audio/sources/{id}/probe` every 8 s
  with `hx-swap="outerHTML"`, but carried no `hx-target` of its own. `hx-target`
  is inherited, the enclosing `<li>` declares `hx-target="this"`, and htmx
  resolves an inherited `"this"` to the element that *declares* the attribute —
  the `<li>`. So each poll swapped the probe response, a bare status `<span>`,
  over the entire row. The header still read "1 mic" (a separate out-of-band
  span), and a page refresh restored the row because it is re-rendered
  server-side, which is what made it look cosmetic.

  Reported from a real station whose microphone was down at the time — which is
  exactly when an operator is on that page and least able to afford the list
  emptying itself. The Edit and Remove buttons in the same row already stated
  `hx-target="closest li"` explicitly; the pill was the one that did not. Both
  the template's pill and the `/probe` replacement now set `hx-target="this"`,
  and a test asserts it on both, since fixing only the replacement would leave
  the first poll after every page load still wrong.

- **Microphone capture could never work on a bare-metal install.** The unit
  granted audio with `DeviceAllow=/dev/snd rw`, but `DeviceAllow=` resolves a
  path to a *device node* and `/dev/snd` is a **directory**, so the rule matched
  nothing. With `DevicePolicy=closed` every ALSA node stayed denied and the PCM
  open failed with *"audio open error: No such file or directory"*. `arecord`
  still exec'd successfully — so the daemon logged *"started microphone capture"*
  — and the supervisor then saw a source producing no samples, killed it, and
  restarted it every 60 s forever.

  Fixed by using systemd's documented subsystem form, `DeviceAllow=char-alsa rw`.
  Verified by A/B under `systemd-run` on the affected board: the old form cannot
  open the device, the new one records normally.

  Present since **v0.6.0** (`5dbc8f1`). RTSP stations were unaffected — `ffmpeg`
  over the network never touches `/dev/snd` — which, together with the hidden
  error below, is why it survived six releases.

- **`/admin` was served to the network on every bare-metal install, while
  `--doctor` reported it protected.** The installer generates an admin password
  on a fresh non-loopback install and writes `CADDY_PWD` to
  `/etc/birdnet/birdnet.conf`; the unit it installs sets no `EnvironmentFile`.
  The auth bootstrap read **only** the environment, so it skipped, the seed admin
  kept its legacy hash, `admin_password_configured` returned false, and the
  cookie middleware took its open-bypass path. `check_admin_exposure` read the
  **config**, found the password, and passed — its doc comment asserting the two
  "can never disagree" while they did. Measured on hardware: `CADDY_PWD` present
  in the config, `/admin/settings` 200 unauthenticated, doctor exit 0.

  Both now call one shared resolver (`helpers::resolve_admin_password`,
  config-then-environment, empty treated as unset), so agreement is structural
  rather than asserted. Stations that set `CADDY_PWD` as an environment variable
  — including Docker — were never affected.

- **A corrupt database bricked the station instead of self-healing.** `--doctor`
  reported SQLite corruption as an *error* (exit 2), and the installed unit gates
  startup on `ExecStartPre=... --doctor ... || [ $? -le 1 ]`. So systemd refused
  to start the daemon — and the daemon is what owns the recovery: `app.rs` runs
  `check_and_recover`, restores from the newest backup that verifies, and failing
  that quarantines the corrupt file and starts fresh. The diagnostic blocked its
  own remedy; `Restart=always` then spent `StartLimitBurst=5` in under a minute
  and parked the unit in `failed`, so even repairing the database left the
  station down until someone ran `systemctl reset-failed` on site.

  Corruption is now a **warning**: still reported, and loudly, but exit 1 so the
  daemon starts and recovers. Exit 2 means "errors that will prevent operation",
  and a corrupt database does not prevent operation. Covered by a regression test
  that corrupts a real database and asserts the check warns rather than fails.

- **A nearly full disk bricked the station the same way.** Found by sweeping the
  remaining `--doctor` checks for the class above rather than by a separate test
  run. Less than 1 GiB free was an *error*, so `ExecStartPre` refused to start
  the daemon — and `start_disk_manager`, the purge that reclaims space at
  `DISK_PURGE_THRESHOLD`, runs inside that daemon. The reclaim therefore never
  ran, `StartLimitBurst` was spent in under a minute, and the unit parked in
  `failed`.

  This one is worse than the database case because it is certain rather than
  unlucky: a full disk is the most predictable end state of a 24/7 recorder, and
  the purge exists precisely to absorb it. It was also mistimed — the purge
  triggers on a *percentage*, so on a small card it fires well below 1 GiB free,
  and the check refused startup before the mechanism that fixes it had been
  reached. Now a warning, with the message naming the purge so an operator knows
  the station recovers on its own.

  The grading logic was extracted into a pure `grade_free_space` so every branch
  is testable. The previous test shelled out to `df` against the host and could
  only assert structure, never the verdict — which is exactly how a hard error
  sat on the low-space branch through six releases.

  Both remaining `Check::fail` sites were reviewed and left alone: a
  non-writable recordings directory and a missing `ffmpeg` for a configured RTSP
  source genuinely prevent operation and do not self-heal. A missing audio
  device was already a warning, correctly — the capture supervisor retries it.

- **A reboot could leave a station serving a healthy dashboard and recording
  nothing.** The installer wrote the detected microphone into the config as an
  ALSA card *index* (`plughw:1,0`). An index is assigned in detection order and
  is not stable. Measured on a Raspberry Pi 4 during the acceptance run: the
  same microphone was `card 1: PRO` before a cold reboot and `card 3: PRO`
  after it. The config still said card 1, `arecord` failed the open with *"No
  such file or directory"* on every attempt, and the capture supervisor retried
  a device that no longer existed — indefinitely, while `/api/v2/health`
  returned `healthy` and the dashboard served normally.

  Detection now prefers the card's **id**, which does not move:
  `plughw:CARD=PRO,DEV=0`. `CARD` is a first-class ALSA argument — alsa-lib's
  own `alsa.conf` declares `pcm.plughw { @args [ CARD DEV SUBDEV ] }` with
  `@args.CARD { type string }`, forwarded to a `type hw` slave as `card $CARD`.
  The index remains the fallback for the case where an id cannot identify a
  single card: two identical microphones report the same id, and then only the
  index tells them apart.

  `--doctor` now understands both forms. The id form was previously
  unparseable, so a correctly configured station was told on every startup that
  its device "was not found in `arecord -l`" — the diagnostic calling the
  robust configuration broken. That id form is exactly what
  [`usb-audio-mapper`](https://github.com/tomtom215/usb-audio-mapper) pins via
  a udev rule (`ATTR{id}="<name>"`), which is the supported way to keep several
  identical microphones straight; `docs/book/admin/audio.md` now says so.
  Index matching is also line-anchored: it previously asked whether the listing
  *contained* `"card 1"`, which is true of `card 12:` as well, so an absent
  card could be reported present. And when the configured card really is
  missing, the check now names the card that *is* present and prints the exact
  `ALSA_CARD=` line to set, instead of advising the operator to go and work it
  out.

  Covered by `installer/test/alsa-device-detect.sh`, which drives the detection
  against the two listings captured from the Pi either side of that reboot and
  asserts they produce an identical device string — plus a counter-test
  asserting the previous implementation did **not**, reproducing `plughw:1,0` →
  `plughw:3,0` exactly as the hardware behaved.

- **`--doctor` validated a device the daemon would never open.** Capture
  resolves its sources from the `audio_sources` table, which is seeded from
  `ALSA_CARD` only while that table is *empty* — after that the table is the
  source of truth, as `capture.rs` says outright. The audio check read only the
  config. So an operator on an established station could correct `ALSA_CARD`,
  restart, watch the diagnostic pass, and still record nothing, because the
  daemon was opening the stale device in the table.

  Measured on a Raspberry Pi 4: config set to `plughw:CARD=PRO,DEV=0`, service
  restarted, and the journal kept reporting `started microphone capture
  device=plughw:1,0` from the table — gauge at 0, nothing recorded, while every
  configuration file on the box said the right thing.

  This is the same shape as the `CADDY_PWD` defect above: two readers of one
  setting, disagreeing, with the diagnostic reading the one the runtime
  ignores. Resolved the same way — the check now consults the table through the
  **same `AudioSourceStore::list` query** the capture path uses, probes the
  devices that will really be opened, and when the config and the table
  disagree, says so and names both values. A missing or corrupt database is not
  a finding here: `check_database` owns that, and a doctor that failed on a
  corrupt database would block the startup that repairs it.

- **The installer told operators to sign in with a username that does not
  exist.** `install.sh` printed `username: birdnet`, wrote `CADDY_USER=birdnet`
  into `birdnet.conf`, and four docs pages repeated it — but the only account
  the dashboard seeds is `admin`, and the login form reads `CADDY_USER` from the
  **process environment**, which the bare-metal unit never sets. Until the
  `/admin` fix above, this was harmless: the panel was open, so nobody ever had
  to sign in. Closing that hole converts it into a lockout — the operator
  follows the installer's own output and cannot get in.

  Found on hardware minutes after the auth fix was verified, by trying to sign
  in. The installer, the generated config's comments, and the docs now say
  `admin`, and record that `CADDY_USER` takes effect only where the environment
  reaches the process (Docker). The docs also stop calling the panel HTTP Basic
  Auth: `/admin*` redirects (303) to a `/login` form and issues a session
  cookie, so `curl -u` never applied to it.

- **Every auto-install path was gated on `apt-get`, so on Fedora, Arch and
  openSUSE the installer printed advice that could not be followed.** The
  binary is a plain ELF and runs on those distributions; only the installer
  assumed Debian. A missing `ffmpeg` on Fedora produced "run `sudo apt-get
  install -y ffmpeg`" — worse than saying nothing, because it looks
  authoritative. Package handling now goes through `detect_pkg_mgr` /
  `pkg_name_for` / `pkg_install` / `pkg_install_hint`, covering **apt, dnf,
  pacman and zypper**, and degrading to "install X with your distribution's
  package manager" when it recognises none.

  Package names were established by installing them in real containers rather
  than assumed: `alsa-utils`, `qrencode` and `util-linux` carry the same name
  on all four, and `ffmpeg` is the sole exception — Fedora ships it as
  `ffmpeg-free` in its main repositories, the unencumbered `ffmpeg` being in
  RPM Fusion, which an application installer has no business enabling on
  someone's machine. `pacman` refreshes with `-Sy` and never `-Syu`: upgrading
  an operator's entire system is not an installer's decision.

  The matrix is preserved as `installer/test/pkg-manager.sh` (Debian trixie,
  Fedora 41, Arch, openSUSE Tumbleweed, plus a no-package-manager case). It
  asserts the tool actually lands on `PATH`, not merely that a command was
  issued. Running it caught two defects that reading could not: the
  unknown-distro branch emitted `install ffmpeg with your distribution's
  package manager && sudo systemctl restart …`, chaining prose into something
  that looks runnable, and the `|| true` guards on the `ensure_capture_tool`
  calls turn out to be load-bearing — the installer runs under `set -e`, so
  without them a warning the operator could act on would abort the install
  instead.

- **`alsa-utils` was never installed, so a microphone station could install
  cleanly and record nothing.** The installer ensures `ffmpeg` when the config
  names an RTSP source, but the ALSA path — the default for a USB microphone —
  only ran `command -v arecord … || true`. `arecord` is both the capture backend
  the daemon spawns and what the installer's own card auto-detect reads, so
  without it detection silently found no device, wrote no `ALSA_CARD`, and the
  station recorded nothing while reporting a clean install. Raspberry Pi OS
  ships `alsa-utils`, which is why this stayed invisible; a minimal Debian does
  not. Both backends now go through one `ensure_capture_tool` helper, `arecord`
  is installed before onboarding so auto-detect has something to read, and a
  still-missing `arecord` at detection time says so instead of returning an
  empty string.

  The install smoke test now **asserts** `arecord` is present after
  `install.sh`, rather than inferring it from the job passing. That distinction
  is the whole point: a failed `alsa-utils` install is deliberately only a
  warning, so the installer exits 0 either way and a green job proved nothing
  about this path. Verified both directions in the job's own `ubuntu:24.04`
  image — with the package manager reachable the assertion passes, and with it
  broken `install.sh` still exits 0 while the assertion fails.

### Changed

- **`birdnet_inference_duration_seconds` no longer claims to be per-chunk.** It
  is observed in `daemon/processor.rs` inside the `DispositionDecision::Accept`
  arm, immediately after `insert_detection` — i.e. **once per stored detection**,
  not once per audio chunk fed to the model. Its `HELP` text said "Per-chunk
  inference latency", which invites exactly the wrong inference: dividing the
  count by elapsed time reads a quiet hour as catastrophic audio loss. The
  exposition text and the surrounding docs now say what it measures, and note
  that no per-chunk counter is exported, so analysed-audio coverage cannot be
  derived from the metrics endpoint.

- **Capture-subprocess failures are now logged at `warn` instead of `debug`.**
  `arecord`/`ffmpeg` stderr — the only place the reason a source will not start
  is ever written down — went through `drain_capture_stderr` at `debug!`, and the
  default filter is `info,birdnet_behavior=debug`. That module is in
  `birdnet_core`, so it sat below the threshold: the supervisor's endless
  "capture (re)start issued" was visible while the error explaining it was not.
  Lines reporting a failure are promoted to `warn`; routine chatter (xruns, RTSP
  reconnects) stays at `debug` so a busy station does not spam the journal.

### Added

- **`scripts/hardware-test.sh`** — an on-device acceptance harness, documented in
  [`docs/book/field/hardware-test.md`](docs/book/field/hardware-test.md). It
  installs from the
  published release, measures mean inference latency per 3 s chunk and peak SoC
  temperature under load, and then deliberately breaks the station — watchdog
  SIGSTOP, microphone hot-unplug, network loss, disk-full, SQLite and DuckDB
  corruption, cold reboot — to establish that each documented recovery path is
  real on the hardware rather than only in `cargo test`. Results are written as
  a pasteable `report.md` plus machine-readable JSONL.

  Two defects in the harness itself, both found by running it rather than
  reading it. **Ctrl-C did not stop a run**: `trap cleanup EXIT INT TERM` with
  a handler that returns does not end a bash script — execution resumes where
  the signal landed, so an interrupt during the destructive suite freed the
  ballast and then carried on into the next fault injection. Signals now clean
  up and `exit 130`. And **`--skip` was missing**, so testing a locally
  installed binary meant either letting the install phase overwrite it with the
  published release, or hand-listing fourteen `--phase` flags — and the
  `--resume` the reboot phase prints would have run the install phase anyway,
  swapping the binary halfway through the suite. Skips are now recorded in the
  state file, which is what makes resume honour them.

  The `diskfull` phase sizes its ballast to cross **both** relevant thresholds:
  the purge fires on a percentage (95 % by default) while doctor grades in
  absolute bytes (under 1 GiB free), and on a 32 GB card filling to 96 % leaves
  1.3 GiB — enough to report success without ever reaching the branch under
  test. It also restarts the service while the disk is full, because the defect
  it exists to catch is on the startup path: a daemon that is already running
  never touches the `ExecStartPre` gate.

## [0.11.0] - 2026-08-09

### Fixed

- **Docker images embedded a behavioral extension the engine could never
  load.** `Dockerfile` pinned the DuckDB community extension to `v1.5.3` while
  the workspace bundles DuckDB 1.5.5. When the engine was bumped (`b35d4f5`)
  `ci.yml` and `release.yml` were updated and the `Dockerfile` was not — and
  because the `v1.5.3` URL still returns HTTP 200, the download *succeeded* and
  the fetch pointed at a real but unloadable artifact.

  The first run of the new CI gate then showed the failure was worse than the
  pin: `curl` is installed only in the *runtime* stage, so in the **builder**
  stage the fetch exits 127 and silently takes the fallback branch. **No Docker
  image has ever embedded the extension, on any architecture** — the wrong pin
  never got as far as being downloaded. Both are fixed: the pin is corrected and
  the builder stage installs `curl` + `ca-certificates`. The extension is also
  now fetched over HTTPS rather than plain HTTP, since it is embedded into a
  binary that is subsequently SLSA-attested and cosign-signed (verified
  byte-identical to the HTTP response).

  DuckDB refuses a version-mismatched extension outright: *"The file was built
  specifically for DuckDB version 'v1.5.3' and can only be loaded with that
  version of DuckDB. (this version of DuckDB is 'v1.5.5')"*. The reason nine
  green workflows never noticed is that the loader tries the extension cache,
  then a community-registry install, and only then the embedded copy — so a
  container *with* network installs the correct build and looks perfectly
  healthy. Only air-gapped and metered stations, exactly the deployments the
  embedding exists to serve, ever saw it, and they saw it as empty analytics
  pages.

  Fixed at four levels so the class cannot return quietly: the pin is corrected;
  `build.rs` now parses the extension's metadata footer and refuses to embed
  bytes it cannot identify, recording what they target; a mismatch between the
  embedded copy and the linked engine fails a test *and* is logged as an error
  at startup even when a network install masks it; and `docker.yml` boots the
  built image with networking disabled and asserts the extension loads.

- **A station whose database directory did not exist refused to start.** SQLite
  will not create a missing parent, so the process exited 1 on a bare *"unable
  to open database file"* — after `--doctor` had reported *"will be created on
  first run — no action needed"* and exited 0. Every sibling directory is
  already created on demand, including the DuckDB analytics store; this was the
  only exception, and the only one whose absence is fatal.

  It did not affect a stock install (the installer pre-creates the directory).
  It affected the storage move `docs/FIELD_DEPLOYMENT.md` recommends — consumer
  SD cards fail after ~6 months of WAL churn — where `RECS_DIR` works because it
  is auto-created and `DB_PATH` did not. The directory is now created before the
  database is opened, and a failure that cannot be fixed automatically (a
  read-only mount, wrong ownership) reports the directory, the cause and the
  remedy instead of a bare SQLite error.

### Added

- **`--verify-extension`.** Opens a throwaway DuckDB database, loads the
  behavioral extension the way the station does, and reports the engine version,
  the extension version and what the build-time embedded copy targets. Exits 0
  when it loads and non-zero when it does not, so it is usable from a monitoring
  script. Run with networking disabled it proves the *offline* guarantee
  specifically: with no network neither the cache nor the community registry can
  satisfy the load, so only the embedded copy can.

  `--doctor` cannot answer this question — it deliberately never opens DuckDB —
  and `TROUBLESHOOTING.md` said to use it, which is corrected.

- **`--doctor` now reports whether `/admin` is exposed without a password.**
  `--listen` defaults to `0.0.0.0:8502`, and with no admin password the cookie
  middleware serves `/admin` to anyone on the network. The station logged this
  at startup, but the diagnostic the docs point operators at checked only that
  the listen address *parsed*. It now warns when the bind is non-loopback and no
  password is set, and passes when either is untrue. Resolution mirrors the
  runtime exactly so the two cannot disagree.

### Changed

- **Dependencies converged.** The lockfile had drifted 150 packages behind, none
  of it visible as a Dependabot PR — Dependabot proposes bumps for *declared*
  dependencies, while the lockfile is what ships. The refresh includes the
  transitive security floor of a networked appliance: `rustls`, `aws-lc-rs`
  (with `aws-lc-sys`), `hyper`, `h2`, `webpki-roots`, `zerocopy` and `regex`.

- **`rubato` 3 → 4 and `audioadapter-buffers` 3 → 4, taken together.** They are a
  version-locked pair: rubato 4 requires `audioadapter ^4.0`, so bumping either
  alone puts two versions of the crate that defines `Adapter` in the graph and
  the resampler's buffer type then implements the wrong one. `process()` moved
  its `input_offset` and channel mask into an `Indexing` struct; our call used
  the defaults, so the migration is behaviour-preserving — verified against the
  real 11 000-species model, which returns bit-identical confidences
  (93.0 / 92.7 / 93.5 % on the reference Eurasian Magpie recording).

- `tower-http` 0.6 → 0.7 and `base64` 0.22 → 0.23, both drop-in.

- **GitHub Actions refreshed, and the toolchain action pinned where it signs.**
  Every third-party action SHA was verified to resolve to the tag it claims
  before being taken. `dtolnay/rust-toolchain@master` in the three release jobs
  that build attested, signed artifacts is now SHA-pinned — safe because each
  passes an explicit `toolchain:` input, so the pin cannot change which Rust is
  installed.

  Dependabot's proposed `dtolnay/rust-toolchain@1.95` → `@1.100` was **not**
  taken: for that action the ref *is* the MSRV declaration, and 1.95 → 1.100 is
  a *minor* bump, so the existing `semver-major` ignore never fired. It is now
  ignored at every update type, and a new CI job fails if the MSRV job's ref and
  `Cargo.toml`'s `rust-version` ever disagree.

- **The model-gated tests can no longer pass by doing nothing.** Rust counts a
  test that returns early as passed, so the suites that exercise the scientific
  core reported the same `ok` line whether they ran real inference or skipped —
  only the elapsed time differed (2.94 s versus 0.00 s). CI now sets
  `BIRDNET_REQUIRE_MODEL=1` in the same step that fetches and checksum-verifies
  the model, which turns a skip into a hard failure; a CDN outage leaves it
  unset, so an upstream problem still degrades to a visible skip rather than
  failing an unrelated build.

  This also fixed a suite that had never run in CI at all: `species_filter_e2e`
  — the regression tests for the species include/exclude fix, where an excluded
  species must never become a stored detection — was absent from the only job
  that exports the model path, so its 10 tests skipped in every run while
  reporting `10 passed`.

- **`CITATION.cff` is now enforced at release time.** It had been stuck at 0.8.0
  through two releases because `validate` checked only `Cargo.toml` and
  `CHANGELOG.md`, while the file's own comment asked maintainers to bump it in
  lock-step. It is the version GitHub's "Cite this repository" widget and Zenodo
  hand to anyone citing this software.

- Documentation-only follow-ups that landed after the 0.10.0 entry was written
  and belonged in no section: three surviving mutants killed in the
  species-list log guard with a refreshed CLI help snapshot (`ce54b61`), and a
  typos-config fix for backticked git SHAs plus one genuine misspelling
  (`14a5bb8`).

## [0.10.0] - 2026-08-07

### Added

- **`--offline` / `BIRDNET_OFFLINE`, and `--no-update-check`.** A station made
  two outbound connections nobody asked for — a release check against
  `api.github.com` 60 seconds after start and every 24 hours after, and
  Wikipedia species-image downloads — and the update check had no off switch at
  all. That is awkward on a metered or cellular link and unanswerable during an
  institutional review. `--offline` turns off both at once; `--no-update-check`
  turns off just the release check. Integrations you configured explicitly
  (Apprise, BirdWeather, MQTT, SMTP, heartbeat, weather) are deliberately
  untouched, because configuring one is the consent — silently muting a
  configured alert channel would be the worse surprise.

  `--doctor` now reports the current posture under **Outbound connections**, and
  the complete inventory — including the one first-run-only DuckDB extension
  fetch — is documented in *Configuration → What the station connects to*.

### Fixed

- **`partial_cmp(..).unwrap()` on floats in two page renderers.** The values are
  sums of integer detection counts, so no reachable input is `NaN` and this was
  latent rather than live. It is fixed anyway because the cost of that
  assessment being wrong is unusually high: `[profile.release]` sets
  `panic = "abort"` and the server mounts no catch-panic layer, so a panic in a
  request handler is not a 500 — it takes the whole process down, web server and
  detection daemon together. The comparisons now use `f32::total_cmp`, and both
  modules deny `unwrap`/`expect` so the class cannot return unnoticed.

  A sweep of every panicking construct reachable from a request handler
  (`unwrap`, `expect`, `panic!`, slice indexing) found no other reachable site:
  the remaining `expect`s are on `HmacSha256::new_from_slice`, which accepts any
  key length, and every `[0]` index is guarded by a length check or a
  fixed-size array.

- **A station stopped being able to start at roughly 2.1 million detections.**
  The initial SQLite → DuckDB analytics sync read the *entire* detections table
  into memory before appending a single row, so peak memory grew with the
  station's whole history rather than with the work in flight. Measured: **541
  MiB at 1 M rows and 967 MiB at 2 M**, against the `MemoryMax=1G` the systemd
  unit sets — and with `Restart=always`, crossing that ceiling produced a
  restart loop rather than a clean failure. A multi-year BirdNET-Pi database,
  which is exactly what the migration importer brings in, is that size on
  arrival.

  The sync now streams rows straight into the DuckDB appender in batches, so
  peak memory tracks the batch and not the row count: syncing 400 000 rows grew
  RSS by 53 MiB where it previously grew by 167 MiB, and 1 M rows now costs 62
  MiB. A soak test asserts the bound and fails on the old implementation.

  A failure part-way through is now also recoverable: the next sync recomputes
  its cutoff from what DuckDB actually holds and resumes, where previously an
  all-or-nothing append meant a station that died mid-sync started over.

- **A corrupt analytics database disabled analytics permanently and silently.**
  A DuckDB file that failed to open was logged once as "not available
  (non-fatal)" and then ignored on every subsequent start, leaving every
  analytics page empty until a human noticed and deleted the file by hand —
  which an unattended field station never gets. The DuckDB store is purely
  derived from SQLite, so it is always safe to discard: an unusable file is now
  moved aside with a timestamped `.corrupt.<unix-seconds>` suffix (its `.wal`
  sidecar with it) and rebuilt from SQLite on the same start. Opening is no
  longer taken as proof of health — DuckDB can attach to a damaged file and only
  fail once a query touches the broken block, so a probe read runs first.
  `--doctor` and `/admin/doctor` report any quarantined file, so the recovery is
  visible rather than buried in the journal.

- **The species allow/exclude lists never filtered a single detection.** The
  daemon built its species filter from `SpeciesFilterConfig::default()` and
  nothing in production ever populated the two lists, so a species excluded on
  `/admin/species` kept being recorded, counted, notified on, and uploaded to
  BirdWeather. The page maintained the list, confirmed every addition, and
  offered a preview page describing exactly the effect that never happened.

  Three separate defects had to be fixed for this to work, any one of which
  would have left it broken:

  - The lists were never read. They now come from the settings table through the
    same function `/admin/species` uses, so the two cannot drift, and they are
    re-read on a 30-second TTL inside the daemon loop — excluding a species is
    something an operator does *because it is spamming them right now*, so it
    takes effect on the next processed file rather than the next restart.
  - The page collects **common** names while the filter worked in **scientific**
    names, so even a populated list would have matched nothing. Entries now
    match either name form, case- and whitespace-insensitively, and the
    `/admin/species/test` preview calls the detection path's own predicate
    rather than a parallel implementation that could drift from it.
  - The filter was skipped entirely unless the station had both coordinates set.
    Only the metadata model needs to know where the station is; the operator's
    lists apply either way.

  An include list that matches no known species is now ignored with a warning
  rather than intersected to nothing — otherwise a single misspelt name would
  have silenced the whole station.

- **The species-frequency filter never ran on a normally-installed station.**
  The daemon read `cli.latitude` / `cli.longitude` with no config fallback, so a
  station configured the usual way — the installer writes `LATITUDE` and
  `LONGITUDE` into `birdnet.conf`, and `/admin/settings` writes the settings
  table layered on top of it — handed the daemon no coordinates and never ran
  the metadata model at all, leaving `SF_THRESH` inert. Coordinates now resolve
  CLI-then-config, the same rule the recording scheduler has always used.

- **Twenty settings-page fields were editable, saved, and connected to
  nothing.** The bridge between the `settings` table and the runtime config was
  a hand-maintained allow-list a new form field could simply be missing from,
  and twenty had accumulated on the wrong side of it — while the page told the
  operator "changes apply on next restart" for values no restart would ever
  read. Most reached the runtime through a flag carrying a clap
  `default_value`, so the default won unconditionally and the field could never
  take effect.

  Every key the form can persist now carries an explicit classification —
  bridged onto the runtime config, owned by a subsystem that reads the settings
  table itself, or removed — and a test fails if one is missing, so a field can
  no longer ship inert. The station resolves each setting *explicit CLI flag or
  `BIRDNET_*` variable → admin settings → config file → default*, which needed
  `clap` to be asked which arguments the operator really supplied rather than
  guessed at with per-flag sentinels.

  Newly working from the web UI: segment duration, frequency shift, night
  inhibit, the pre-sunrise and post-sunset offsets, multi-stream RTSP URLs, the
  custom species-image directory, and the weekly report schedule.

- **Apprise and BirdWeather could be configured in the web UI and would never
  send.** Both clients read only the CLI flag and the config file, so a token or
  notification URL entered on the Settings page was stored and ignored — and the
  admin "Send test notification" button read the *saved* value, so the test
  succeeded while live detections notified nobody. Both, along with the
  notification trigger mode, cooldown, minimum confidence, species allow/exclude
  lists and message templates, now reach the runtime from either surface.

- **Dawn and dusk recording windows can now differ.** The scheduler has always
  carried separate pre-sunrise and post-sunset offsets and the settings page has
  always shown two fields, but the runtime wrote a single `--twilight-offset`
  into both, so no surface could make them differ. Each end now resolves on its
  own via `--pre-sunrise-offset` / `--post-sunset-offset` (or the matching
  settings fields), falling back to `--twilight-offset` when unset — so existing
  stations keep their current symmetric behaviour.

### Removed

- **The Settings page's "Web Authentication" card.** Its password field stored
  whatever was typed as a **plaintext** row in the `settings` table, rendered it
  back into the page HTML on every later load, and changed no credential at all
  — the admin password is an Argon2id hash in the accounts database, seeded
  from `CADDY_PWD`. The section also claimed that clearing the field would
  "disable HTTP Basic Auth", which it never did. The card now explains where the
  credential actually lives, and any plaintext row left by an earlier build is
  deleted on the next start.

- **Two settings inputs with no runtime consumer at all.** "Audio Channels"
  duplicated a control that already works per-source on
  `/admin/audio` (which is where the channel count is really read from), and
  "Include Species Image" drove nothing in the notification stack. The audio
  section now points at the page that works; the notification option is gone.

### Added

- **Time-based clip retention that actually works — and is off by default.**
  The settings form has always shown a "Keep Recordings (days)" field promising
  that older audio was deleted automatically. Nothing ever read it: the key had
  no consumer and no bridge into the runtime config, so the setting was inert
  while the configuration docs correctly stated retention was not time-based.
  Age-based retention now runs on the daily maintenance tick — locked clips are
  exempt, a file shared by several detections goes only when every one of them
  is past the cutoff, and the detection rows survive so counts, species lists,
  trends and exports are unaffected. It uses a **new** setting
  (`clip_retention_days`, default `0` = keep forever) rather than the old inert
  one on purpose: the old field defaulted to 30 in the form, so stations carry
  a value nobody meant, and teaching that key to work would have deleted every
  clip older than a month at the first tick after upgrading.

- **Every disk-retention limit is settable from the web UI and the
  environment.** The purge threshold and the transient stream directory's age
  and size limits previously required hand-editing the config file — which the
  Docker entrypoint does not even use, leaving container operators no way to
  change them. All are now settable via `--disk-purge-threshold`,
  `--stream-retention-secs`, `--stream-max-mb` (each with a `BIRDNET_*` env
  var), via **Settings → System**, or via the config file, resolved in that
  order.

- **Station Health shows RAM `/tmp` (scratch) headroom.** The service streams
  live audio segments through `/tmp`, which on a Pi is a small, RAM-backed tmpfs
  separate from the data disk — and the existing "Disk" tile only watches the
  data partition, so a filling `/tmp` (which silently breaks the capture pipeline
  and even `apt`) was invisible on the dashboard. A new "Scratch" vital tile
  shows its usage, and the attention banner flags it when it runs low. Shown only
  when `/tmp` is a distinct filesystem from the data disk, so it never duplicates
  the Disk tile on systems where `/tmp` lives on the data partition.

- **Per-species recording cap (`MAX_FILES_SPECIES`) now actually works.** The
  old filesystem sweep walked a `By_Date/<species>/` subtree that the flat,
  RAM-backed capture directory never has, so the cap silently did nothing on a
  real install. It is now enforced from the database — the authority on which
  clip belongs to which species, since common names can contain hyphens
  (`Black-capped_Chickadee`) and are not reliably parseable from filenames — on
  the daily maintenance tick: the newest N clips per species are kept and older
  ones are deleted from disk. Detection rows are preserved (stats and counts are
  unaffected; only the audio file is removed). `0`, the default, means unlimited.

### Fixed

- **Scheduled maintenance no longer resets on every restart.** The integrity
  check, session prune, per-species cap and weekly backup + VACUUM were driven by
  timers measured from process start, so any station restarting more often than a
  job's period never ran it — and unattended stations restart constantly: a
  settings change ("applies on restart"), an update, a power cut, a systemd
  watchdog bounce. A station rebooting daily never once reached the weekly
  backup. Because `check_and_recover` can only restore from a backup, that turned
  recoverable corruption into total data loss on exactly the deployments the
  schedule protects. Each job's completion is now recorded in the database
  (`maintenance_runs`, migration 21) and the schedule runs on elapsed wall-clock
  time, so an overdue job fires on the next boot. A clock correction that leaves a
  timestamp in the future re-anchors the schedule instead of suppressing the job,
  and a database that cannot be written still throttles to one run per interval.

- **The persistent recordings directory is now disk-managed.** The bare-metal
  installer always passes `--watch-dir`, so the disk manager attached to the
  RAM-backed stream directory and the data disk — where extracted clips now
  accumulate beside `birds.db` — was never watched at all, while
  `DISK_PURGE_THRESHOLD` appeared to guard it. A 24/7 station filled its card
  until SQLite writes began failing. Both directories are supervised now, each
  with the retention it needs: the stream dir keeps its age and size drain, while
  the recordings dir gets the disk-full backstop only — oldest first, never by
  age, and never a locked clip.

- **Per-species confidence thresholds apply without a restart.** Thresholds were
  read once when the daemon started, so setting one in `/admin/species` did
  nothing until the service was restarted — the row appeared, the page confirmed
  the save, and detections kept being judged by the old value, with nothing
  saying why. They are now re-read on a short interval. The page also claimed
  sub-threshold detections "will be discarded"; they are held in **Quarantine**
  for you to confirm or reject, which it now says.

- **Reclaiming a clip no longer erases its filename.** Retention used to clear
  `File_Name` when it deleted audio, losing the capture timestamp and source the
  clip was cut from — the record of what a detection was matched to. The name is
  kept and a new `Clip_Pruned_At` column records when the audio went, so a row
  now distinguishes "never had a clip" from "had one, reclaimed on this date".
  Every counting, grouping and charting query is unaffected.

- **Locking a recording now protects it immediately.** The purge read the locked
  set once at startup and ran on that snapshot for the lifetime of the process,
  so a clip locked from `/admin/recordings` was unprotected until the next
  restart, with nothing saying so. The set is re-read on every purge cycle. The
  per-species cap ignored locks entirely — setting `MAX_FILES_SPECIES` deleted
  the very recordings a researcher had marked to keep — and now excludes them,
  along with any clip another in-cap detection still references.

- **Pruned clips no longer leave a dead play button.** Retention deleted the
  audio but left the row looking playable, so the clips browser kept offering
  playback for a file that no longer existed, and the daily query re-selected
  every already-pruned row forever. The `Clip_Pruned_At` stamp above resolves
  both. The "has playable audio" test was spelled out at eight call sites and is
  now one shared definition, so no surface can disagree with another about what
  can be played.

- **Backups are visible, downloadable and deletable again.** Snapshots are
  written as `{db_name}.backup.{unix_secs}`, whose extension is the timestamp
  rather than `db`, but the admin surface filtered for names ending in `.db`. It
  matched nothing any station has ever produced: `/admin/system/backups` reported
  "No backups found" on every install, and download and delete rejected every
  real file with a 400 — indistinguishable from simply having no backups.

- **The Station → Data tab reports real numbers.** It rendered a mock-up as live
  telemetry: a fixed "Last backup: 2 h ago · auto · nightly 03:00" (there is no
  nightly backup, and on a restart-prone station none had ever run), a
  "Restore tested · verified bootable" line for something nothing tests, eight
  invented snapshot rows with working-looking Restore buttons, hardcoded storage
  figures, and an operations log quoting an S3 upload failure for an integration
  that does not exist. Every figure is now measured from the running station, and
  a station with no snapshots says so. `POST /admin/system/restore` — which
  existed but had no UI anywhere, so a full backup could be downloaded and never
  restored — is now reachable.

- **A full `/tmp` no longer breaks the station (and `apt`).** Raw capture
  segments are written continuously into the RAM-backed stream directory, but
  nothing ever deleted them once the detector had processed them: the disk
  manager's safety net only purged a `By_Date/` subtree, which that flat
  directory never has, so it ran every minute and reclaimed nothing. A station
  could fill a ~2 GiB tmpfs within hours, breaking the capture pipeline and even
  `apt`, while the dashboard's Disk tile — watching the *data* partition — still
  read healthy. The disk manager now drains the stream directory by age and by a
  total-size ceiling (`STREAM_RETENTION_SECS`, `STREAM_MAX_MB`), and its
  disk-full purge now also considers those flat segments. Draining only ever
  applies to the transient capture directory, never a persistent recordings dir.
- **Extracted detection clips now persist, appear in Recordings, and play.**
  Three separate faults stacked into one broken feature on a default systemd
  install. Clips were written to a sibling `Extracted/` directory next to the
  capture directory — i.e. onto `/tmp`, which `PrivateTmp=yes` wipes on **every
  restart** — while the web server reads recordings from the data disk, which
  nothing ever wrote to. They were also nested under `By_Date/<date>/<species>/`,
  though the recordings API serves and lists by bare filename. And the database
  recorded the *source segment's* name rather than the saved clip's, so even a
  correctly-placed clip could not be found. Clips are now written flat into the
  same directory the web server serves from (one source of truth, so the two
  cannot drift apart), and the clip's own filename and duration are what get
  stored. The filename already encodes species, confidence, date and time, so
  nothing is lost by dropping the nested layout. Detections recorded *before*
  this fix keep their old filename and remain unplayable.
- **Adding two different audio sources within the same second no longer fails.**
  The synthetic source id was `src_<kind>_<seconds>`, so two sources added in the
  same second collided and the second add returned a baffling "Retry — a new id
  will be generated" toast. The id now carries a process-local sequence and is
  always unique.
- **The Audio sources admin page no longer strands you or contradicts itself.**
  Several rough edges are fixed together: the RTSP "Network streams" section was
  *hidden* whenever no stream existed yet, so once you had a microphone the "Add
  stream" form was unreachable — both sections are now always shown. The
  per-section counts ("N mics" / "N streams") update the instant a source is
  added or removed (they used to go stale), the separate empty-state card that
  contradicted a freshly-added row is gone, and the edit form's **Cancel** button
  — which fetched the status pill and swapped nothing, leaving the form stuck
  open — now restores the row.
- **The dashboard "what's new" banner no longer reads "New in vUnreleased."**
  The banner showed the topmost changelog entry, which is the in-progress
  `## [Unreleased]` section, so it rendered a meaningless version to everyone.
  It now shows the latest *released* version (skipping `Unreleased`), or no
  banner at all when there is no release yet.
- **The admin "Restart" button now actually restarts the service.** It shelled
  out to `systemctl restart`, which a non-root, sandboxed service can't do
  (polkit-denied) and which races its own `KillMode=mixed` cgroup teardown. It
  now signals itself (SIGTERM) and lets the unit's `Restart=always` bring it
  back — responding to the browser first so the page can show the status. When
  the binary isn't running under systemd it now says so plainly instead of
  killing itself and reporting a false "restart sent."
- **Adding the same microphone or RTSP stream twice is now prevented.** The
  audio-source form only de-duplicated on a synthetic id (always freshly
  generated), so the same physical device could be added over and over. It now
  rejects a source whose kind + device id already exists, with a clear message
  pointing to the existing entry.
- **Station Health "Vitals" now report real CPU and memory.** The hardened
  systemd unit set `ProcSubset=pid`, which hides the system-wide `/proc` files
  (`/proc/stat`, `/proc/cpuinfo`, `/proc/meminfo`) that the `sysinfo` crate reads
  — so the dashboard showed an impossible **0 CPU cores / 0% CPU** and **0 B / 0 B
  memory**, while temperature (read from `/sys/class/thermal`) and disk (via
  `statvfs`) still worked. The unit no longer restricts `/proc` (a comment marks
  why it must stay at the default), while `ProtectProc=invisible` still hides
  other users' processes. Apply to an existing install with
  `sudo bash install.sh repair`, which rewrites and reloads the unit.
- **A fresh bare-metal install now starts the dashboard immediately, even with
  no audio source.** Previously `install.sh` only ran `systemctl start` when an
  ALSA/RTSP source was already in the config, so an operator who clicked through
  the setup wizard with no microphone auto-detected was left with a service that
  "did not come up" — yet the unit is *enabled*, so the next reboot started it
  anyway, which was both confusing and inconsistent. The installer now starts the
  service unconditionally on a fresh install (the systemd doctor preflight treats
  "no audio source" as a warning, not a failure), so the web dashboard — and its
  first-run onboarding wizard, where the microphone and location are chosen — is
  reachable the moment the installer finishes. This matches the Docker quickstart,
  which already brought the dashboard up regardless of audio. The post-install
  summary now clearly notes when no audio source is set yet and points to the
  in-dashboard setup wizard.
- **A mistyped stream URL is no longer silently accepted as a sound card.** The
  installer's audio-source prompt treated anything that wasn't an `rtsp://` URL
  as an ALSA device name, so a typo'd scheme (`http://camera…`) was written into
  the config as a sound-card string that could never open. Input that looks like
  a URL but isn't `rtsp://` / `rtsps://` is now rejected with an explanation and
  re-prompted. Plain ALSA device names (`plughw:1,0`, `default`) are unaffected.
- **Skipping an installer safety check is now impossible to miss.**
  `BIRDNET_SKIP_MODEL` and `BIRDNET_SKIP_GLIBC_CHECK` announced themselves with a
  single `[WARN]` line that blended into the surrounding install output — and
  lost its colour entirely in a piped or CI install — so the eventual failure
  (a daemon that detects nothing; a `GLIBC_… not found` crash at startup) arrived
  with no obvious cause. Each bypass now prints a boxed, unmissable warning that
  survives a non-interactive install and states the consequence.
- **A disabled notification-test button now says why it's disabled.** The Apprise
  and BirdWeather test buttons greyed out with no explanation when the channel
  had no credentials. Each now carries a tooltip *and* visible hint naming the
  exact setting to fill in — the hint because browsers suppress tooltips on
  disabled buttons.
- **The "what's new" banner no longer vanishes silently when it can't load.**
  After an upgrade, if the release-notes request failed — the server still
  restarting, a 5xx, an older build without the endpoint — the banner simply
  never appeared, indistinguishable from having no news. It now falls back to a
  minimal "updated to vX.Y.Z" banner linking to the full changelog. A server
  that intentionally has no release to announce still stays quiet.

### Dependencies

- `duckdb` 1.10503.1 → **1.10505.0** (bundled DuckDB 1.5.3 → **1.5.5**), to pick
  up **`duckdb-behavioral` v0.9.1**. The behavioral extension is version-locked
  to the DuckDB it was built for — DuckDB refuses to load a mismatch, and
  `allow_extensions_metadata_mismatch` does not bypass that check — so the
  bundled engine moves in lockstep with the published community build. Verified
  before landing rather than assumed: the community CDN's `v1.5.5` artifacts for
  both `linux_amd64` and `linux_arm64` report `behavioral_version v0.9.1`, and
  both load paths succeed (online `INSTALL … FROM community` and the offline
  embedded fallback) with every behavioral function executing. Note the `v1.5.4`
  CDN path is *not* usable — it still serves a byte-identical copy of the old
  v0.8.0/1.5.3 build, which is exactly why an HTTP 200 on a version path is not
  sufficient evidence to bump.

## [0.9.0] - 2026-06-22

### Added

- **OpenAPI 3.1 description of the public JSON API.** The full `/api/v2`
  surface (44 read-only endpoints across detections, species, recordings,
  analytics, time-series, export and system) is now described by a committed,
  hand-maintained OpenAPI 3.1 document (`crates/birdnet-web/openapi.json`),
  served live at `GET /api/v2/openapi.json` so any tool — Swagger UI, Redoc,
  Postman, `openapi-generator` — can map the API or generate a client. The spec
  honestly declares the API as unauthenticated (`security: []`); a committed
  `redocly.yaml` documents why two of Redocly's opinionated default rules don't
  apply (intentional openness, read-only endpoints) so `redocly lint` is clean.
  A test parses the embedded document and asserts every documented path is
  actually routed, so the spec can't drift out of sync with the server. The
  HTTP-API reference doc is corrected alongside it (the `detections/daily` and
  `species/activity` query parameters were documented incorrectly).
- **Recordings now shows each saved clip's duration.** A deferred Wave D
  omission (the Clips grid dropped the column rather than fake it) is now
  backed honestly. **Migration 20** adds a nullable `Duration_Secs` to
  detections; the daemon reads the source recording's length from its file
  header — cheaply, via a new `birdnet-core` `decode::probe_duration_secs`, with
  no re-decode — and persists it. Historical, BirdNET-Pi-imported and
  quarantine-approve rows have no length to record and stay `NULL` (the grid
  omits the column for them, never a guess). The Clips grid renders the length
  as `M:SS` under each row's time.
- **Recordings clips show "first today" / "rare" badges.** Another deferred Wave
  D omission: each clip row now carries the same first-seen badge the Today feed
  shows — "first today" when the species' first-ever record is today, "rare"
  when the clip sits on the species' first-ever (historical) date — reusing the
  existing `species_first_seen` query and `bnb-pill` styling (no new query, no
  new tokens). A clip with no first-ever match shows no badge.
- **Recordings clips show a spectrogram thumbnail.** The last deferred Wave D
  Recordings omission is now backed honestly — by reusing the existing
  `/api/v2/spectrogram/{file}` endpoint (the same renderer, viridis colormap and
  byte-budgeted cache the detection-detail view already uses) rather than a
  second system. That endpoint gains a `?thumb=1` mode that max-pools the time
  axis down to a small fixed width (so a multi-second clip ships a few KB instead
  of a multi-thousand-pixel image, and brief calls still survive the shrink),
  cached separately from the full-size render. The Clips grid links a lazy-loaded
  thumbnail only for rows whose audio is present — gated by a single per-page
  directory scan, the same way the locked-clip set is loaded — so there is no
  per-row stat, no schema change, and historical clips get a preview too; rows
  whose audio is gone show an empty aligned spacer rather than a broken image or
  a faked tile. New CSS only (`.rc-spectro`); no new design tokens, no new
  dependency.
- **CI: an accessibility gate and a structural visual-QA sweep.** A new
  `a11y.yml` workflow boots the seeded `screenshot_server` fixture once and runs
  two gates against it — **axe-core** (WCAG 2.1 A/AA, light + dark themes) fails
  the build on any serious or critical violation, and the **`qa.mjs`** sweep
  fails on a structural regression: horizontal overflow, console/page errors,
  responses ≥ 400, broken images or stuck loaders. Path-filtered to web/tooling
  changes; the visual gate is deterministic (no flaky pixel baselines). The axe
  gate enforces every serious/critical rule except two deferred (with a written
  rationale in `axe.mjs`) to a design-reviewed pass: `color-contrast` (the v3
  palette renders each species' identity hue as text and uses a muted meta-text
  hierarchy — an all-or-nothing design-token decision) and `link-in-text-block`
  (an app-wide link-underline policy).
- **Adopt duckdb-behavioral v0.8.0's new ClickHouse-parity functions.** The
  community `behavioral` extension served for the bundled DuckDB (v1.5.3) is now
  v0.8.0 (pin verified — no engine change needed), which adds `sequence_count`,
  `window_funnel_events` and `sequence_match_events`. `birdnet-behavioral` gains
  typed wrappers for all three — `AnalyticsDb::sequence_count` (how *many* times
  an ordered species sequence occurred per day, not just whether it did),
  `AnalyticsDb::funnel_events` (the timestamp each completed dawn-chorus step
  fired) and `AnalyticsDb::sequence_match_events` (the per-step timestamps of an
  ordered NFA-pattern match — the longest in-order prefix reached that day) —
  with SQL builders, unit tests, and live tests verified against the real
  extension. Exposed over the REST API as
  `/analytics/{sequence-count,funnel-events,sequence-match-events}`.
- **The Patterns → Behavior tab surfaces the dawn "running order."** A new
  defined-in-place card reads the station's own dawn-window data to pick the
  morning's leading voices, then uses v0.8.0's `sequence_count` and
  `sequence_match_events` to show how *often* they sing in that exact order and,
  on a recent morning, the *time* each one checked in. Both halves share the
  same NFA-match semantics, so the headline count and the step timing can't
  disagree. The sequence is derived from the data rather than hard-coded (the
  REST defaults are European), so the card reads honestly at a North-American
  station too. The card now also **leads with a funnel picture** (a new
  server-rendered inline-SVG `viz::sequence_funnel`) built from v0.8.0's
  `window_funnel`: how many mornings reach each step of the running order, the
  bars narrowing as the chorus progresses — drop-off you can read at a glance.
  It is omitted, never drawn empty, when no morning reaches even the first step.
- Permanent (`308`) redirects from every pre-spine route to its new home
  (`/today`, `/heatmap`, `/analytics`, `/migration`, `/correlation`,
  `/timeseries`, `/analytics/dawn-chorus`, `/weekly`, `/year-in-review`,
  `/history`, `/system`, plus the live-audio paths `/listen`, `/livestream`
  and `/live`), so existing bookmarks and BirdNET-Pi muscle memory never 404.
- `recent_clips` / `recent_clips_count` (`birdnet-db`): a cross-date,
  filterable, paginated query of clips that saved an audio file, behind a
  `RecordingsFilter` (All · Best · Rare · Locked) that reuses the Today log's
  "best"/"rare" definitions. Powers the Recordings Clips browser.

- **Self-hosted ingest endpoint for uploads** (`BIRDWEATHER_URL` config key /
  `BIRDNET_BIRDWEATHER_URL` env). Research programmes tracking sensitive
  species can route the entire upload pipeline — including the offline queue
  and ordered replay — at their own endpoint implementing the `BirdWeather`
  station API shape, keeping observation locations under their own
  governance. Only the host changes; the `/stations/<token>/...` path shape
  is preserved, and the active endpoint is logged at startup.
- **End-to-end delivery proof for the store-and-forward queue**
  (`tests/store_forward_e2e.rs`): boots the real compiled binary against a
  local stub `BirdWeather` server with a pre-seeded backlog and asserts the
  drainer replays it oldest-first, in the real camelCase wire format, to the
  station-token path, and leaves the queue empty — closing the one branch of
  the replay loop (deliver → 200 → dequeue) that the outage-side live test
  could not reach.

- **Store-and-forward `BirdWeather` uploads** (`outbound_queue`, migration
  19). Posts that fail after their in-flight retries are parked in the local
  database and replayed automatically when the uplink returns — oldest
  first, capped batches with spacing, exponential backoff to a 1 h ceiling,
  bounded to 5 000 entries and 48 attempts so a weeks-long outage can never
  grow the database without limit. The field runbook had promised
  "buffered locally; retried with exponential backoff" all along; the code
  now keeps that promise. MQTT and Apprise/email deliberately stay
  fire-and-forget (live telemetry / look-now alerts — replaying them hours
  later is worse than dropping them). Exposed as the
  `birdnet_outbound_queue_depth{kind}` gauge and a "Queued Uploads" row on
  the `/system` page whenever non-empty.
- **Detection deadman watchdog.** The end-to-end "is the station actually
  detecting?" check: every component gauge can be green while a clogged
  mic foam or a model/labels mismatch silences the station. The daemon now
  measures seconds-since-last-detection (in SQLite's own localtime lens, so
  no TZ skew), exports it as `birdnet_detection_silence_seconds`, surfaces
  it on `/api/v2/health` (`detection_silence_secs`) and as the `/system`
  page's "Last Detection" row, and after a configurable quiet threshold
  (`--deadman-hours` / `BIRDNET_DEADMAN_HOURS` / `DEADMAN_HOURS`, default
  24 h, `0` disables) logs a loud warning and sends one Apprise alert per
  quiet episode with a recovery notice when detections resume.

- **Silent-stall detection for capture sources.** The supervisor now watches
  each source's newest recording segment: a subprocess that stays alive but
  stops delivering audio (a wedged RTSP session, a USB mic hung after a
  re-enumeration) is detected after several missed segments and restarted
  through the same backoff path as a crash — closing the field failure where
  `is_running` reports healthy but a camera has gone quiet. Fails open while
  the clock is unsynced (segment mtimes aren't trustworthy pre-NTP).

- `cargo-fuzz` harnesses (`fuzz/`) for the untrusted-input parsers: symphonia
  audio decode (WAV/FLAC/MP3 demux of watch-directory files) and the
  species-label parsers, with a seeding recipe in `fuzz/README.md`.
- `CITATION.cff` (with the BirdNET reference), `GOVERNANCE.md`,
  `.gitattributes` (LF normalization + binary markers), and live CI /
  coverage / supply-chain badges in the README.

### Changed

- **Web UI reorganized into six homes (the "v3 spine").** The navigation
  collapses from 9 top-level tabs + a 14-entry "More" menu into six
  task-based homes — **Today · Species · Patterns · Recordings · Reports ·
  Station** — generated from a single nav manifest, with one shared
  vocabulary on desktop and the phone bottom bar (the desktop "More"
  dropdown and the mobile "More" sheet are retired; a Help icon and the ⌘K
  command palette cover the long tail). Every navigation surface and the
  command palette are parity-tested against the manifest.
- **Dashboard and Today merged into one home at `/`.** The old separate
  "right now" dashboard and "today log" pages were the same data twice; the
  Today home now leads with a comparative phrase ("a *busy* morning" vs your
  30-day baseline) and an honest live signal (a flat **idle** baseline when
  no audio is arriving — never a fake waveform), surfaces a review nudge or
  outage banner only when one is warranted, plots the day on a rebuilt strip
  (hourly histogram + in-strip temperature + real sunrise/sunset), and folds
  the live feed and the full searchable/filterable day into one log behind a
  disclosure. A brand-new station gets a "getting ready" checklist instead of
  an empty page.
- **Analytics, reports and system pages fold into tabbed homes.** Activity
  heatmap, dawn chorus, migration, co-occurrence, time-series and behavioral
  analytics are now the six tabs of **Patterns**; the weekly report, year in
  review and history are the three tabs of **Reports**; the read-only system
  health page is the public **Health** tab of **Station**. The underlying
  server-rendered SVG renderers are unchanged.
- **Patterns reskinned: one picture per tab, numbers behind a disclosure.**
  All six tabs now open with a one-paragraph, jargon-free `bnb-lede` that says
  what the chart means before the chart appears ("Darker cells mean more birds
  heard that hour…"; "Who sings, and when…"; "Each ridge is one species'
  abundance across the year…"), and each leads with a single picture, tucking
  the supporting tables and numbers behind a "see the numbers" `<details>`
  disclosure: **Who-sings-together** leads with the co-occurrence chord and
  hides the matrix + strongest-pairs tables; **Dawn chorus** leads with the
  circadian polar and hides the per-species ribbons; **Behavior** becomes a
  masonry of cards that define every term in place; **Trends** leads with the
  two headline lines (detections per week, species richness) and folds the rest
  of the dashboard behind a disclosure; **When-active** drops the duplicated
  dawn/phenology panels (each is now its own tab). The underlying server-rendered
  SVG renderers are unchanged.
- **Reports reskinned into editorial recaps.** Weekly and Year-in-review now
  open with an editorial `rp-hero` (a headline that reads the week/year — "A
  *loud* week.", "Your year in *birdsong*.") over a four-up `rp-stats` band
  (detections vs last week, species, new-to-list, busiest day), then a
  leaderboard and the first-ever/milestone columns. **History** becomes a
  month **heat-calendar**: each day is a cell coloured by its detection count
  and annotated with its species tally; selecting one loads that day's hourly
  chart and top species into a detail panel, with ‹/› month navigation, and an
  **Open day →** link to a full-page recap of that day (`/reports/day`) — its
  hourly shape, every species heard, and the complete chronological detection
  log, read-only (managing detections stays on Today / Recordings). Backed by a
  new `detections_per_day` query.
- **Reports gain a "Save as PDF" button.** Each Reports tab now carries a
  CSP-safe print affordance — a real button whose delegated, nonce'd click
  handler opens the browser's print dialog, which the existing `print.css`
  `@media print` rules turn into a clean, light-palette, page-broken keepsake.
- The detection log gains **category filters** (Rare · First today · High
  confidence) alongside text search.
- **Recordings rebuilt into a Clips + Live home (`/recordings`).** The old
  by-species / by-date browser and the separate `/listen` page merge into one
  Recordings home with a `?view=clips|live` switch. **Clips** is a flat,
  newest-first browser of every detection that saved an audio clip, with
  filter chips (All · Best · Rare · Locked), species search, a now-playing
  player that docks to a floating bar on scroll, per-clip lock/download/delete,
  and a Select mode for bulk actions. **Live** folds the live page's honest
  scrolling sonogram (real spectrogram frames; a flat idle baseline when no
  audio is arriving — never a fake waveform), source picker and live-detection
  trickle. `/listen`, `/livestream` and `/live` permanently redirect to
  `/recordings?view=live`.
- **Species rebuilt into a List + Photos + Life list home (`/species`).** The
  three pre-spine destinations — the species list, the `/gallery` photo wall and
  the `/life-list` journal — merge into one home with a `?view=list|photos|
  lifelist` switcher, an "All / This week" filter and species search. **List** is
  the ranked table (rank · avatar · 14-day sparkline · count · avg confidence);
  **Photos** is the Wikipedia-thumbnail gallery with the gradient banding-code
  fallback; **Life list** leads with the big counters (species all-time · active
  days · new this year), the species-accumulation curve, and a "New to the list"
  feed of the most recent firsts. The per-species detail page keeps its `sd-*`
  treatment with cross-links updated to the new homes. `/gallery` and `/life-list`
  permanently redirect to their view.
- **Station Health is now an operator-grade surface.** The public Station
  Health tab (`/station`, the heir to `/system`) gains an overall status
  banner, a **per-source activity** panel (how many detections each audio
  source produced today and how recently — an honest activity signal, since
  the web process has no live handle on the capture supervisor), a vitals row
  (CPU · memory · temperature · df-correct disk meters), a pipeline row (last
  detection · queued uploads · service uptime · total detections) and a short
  diagnostics checklist, in the `st-*` treatment. (The per-source live
  state-chip, 24 h uptime strip and retry/backoff line are now wired through —
  see the next entry.)
- **Station Health's per-source cards go live.** The capture supervisor now
  publishes per-source health — Connected · Stalled · Backing off · Paused,
  plus last-audio age, restart attempts, next retry, and a rolling 48-segment
  24 h uptime strip — into a shared handle the web layer reads, so each
  `st-source` card shows a real status chip, the uptime strip, time since last
  audio, today's detections, and a retry/backoff line (`↻ reconnecting ·
  attempt 3 · next try in 12 s`); the status banner flags a down source. The
  seam is a new `birdnet-core::audio::capture::status` type shared by the
  binary's supervisor (writer) and `birdnet-web` (reader), so neither depends on
  the other. With no supervisor running (web-only mode, tooling) the cards fall
  back to the detection-activity signal — never a faked chip.
- **The Station toolbox gains five gated management tabs.**
  `/station/{capture,alerts,data,settings,access}` fold the twelve flat
  `/admin/*` pages into the Station home's six task groups, rendered through
  the **main** shell with the shared Station sub-tab row but gated behind the
  same admin auth as `/admin/*`. **Capture** = audio sources · which-birds-count
  filter (with a safe Preview) · the single canonical detection-threshold home ·
  recording & location; **Alerts** = rules · channels with Send-test · where
  alerts flow · recent sends; **Data** = backups & export · BirdNET-Pi import ·
  data quality; **Settings** = per-device display prefs · station & system ·
  the kiosk launcher; **Access** = accounts & sessions · a lockout-aware danger
  zone. The real forms are reused verbatim and keep posting to their existing
  `/admin/...` endpoints — only the page GETs move. The eight folded
  `/admin/*` management pages (`audio` · `species` · `rules` · `notifications` ·
  `backups` · `migrate` · `quality` · `accounts`, plus the `/admin` landing) now
  **permanently redirect** to their Station tab, so old bookmarks never 404; the
  Health-detail pages (`overview` · `system` · `doctor`) and the all-in-one
  `/admin/settings` form stay reachable as gated fallbacks.
- **The admin panel's nav is regrouped into the six Station task groups.**
  `admin/nav.rs`'s twelve flat destinations are ordered into labelled
  **Health · Capture · Alerts · Data · Settings · Access** clusters (one
  labelled group each in the shell nav), so the gated admin area's information
  architecture matches the Station home's six tabs. Single source of truth;
  parity- and grouping-tested.
- **Accessibility: the analytics charts now name and describe themselves.**
  Every server-rendered inline-SVG chart (`viz/`) carries a `<title>` accessible
  name and a one-sentence, jargon-free `<desc>` of what it encodes (e.g. "A
  24-hour clock face with midnight at the top; each species' ribbon swells at
  the hours of day it sang most"), replacing the bare `aria-label` so a screen
  reader announces what the picture *means*, not merely that it exists. The
  Recordings → Live detection trickle is now an `aria-live="polite"` region so
  new detections are announced as they arrive (the Today feed already was). The
  segmented controls (the Today log filter, the Species view switcher, the
  display-preference toggles) drop the incorrect `role="tablist"`/`"radiogroup"`
  they carried over plain `<button>`/`<a>` children — they are honest button/link
  groups, now `role="group"` (the filter conveys its active state with
  `aria-pressed`, the view switcher with `aria-current`) — and the kiosk's
  scrolling recent-feed is now keyboard-focusable.

- The time-series dashboard's 13-row API-endpoints table is collapsed into a
  disclosure ("API endpoints · for scripts & integrations") so the page reads
  as a field tool, not an API manual.
- Kiosk mode gained an escape hatch — a dimmed corner "Exit" link and the
  ESC key both return to the dashboard (it was a dead end with no way back).
- The recordings species list uses the shared illustrated empty-state
  component instead of a bare `<p>No species detected yet.</p>`.

- `unsafe_code` lint raised from `deny` to `forbid` workspace-wide (what the
  README badge always claimed); `missing_docs` is now enforced and the ~250
  previously undocumented public items carry real rustdoc.
- Retry constants unified across `apprise` / `birdweather` / `wikipedia` to
  `MAX_ATTEMPTS` (total attempts) with exclusive ranges — the previous mix of
  inclusive/exclusive `MAX_RETRIES` loops made two of the three doc comments
  wrong. No behavioral change.

### Fixed

- **MQTT publishing no longer runs inline on the detection thread.** It was the
  one network integration (of five) dispatched synchronously in the
  single-threaded event processor, so an offline broker blocked every
  detection for the connect timeout and serialized detection handling behind a
  dead network path. It now fires off the detection path like BirdWeather /
  Apprise / email / heartbeat already did — a multi-day broker outage slows
  detection by nothing.
- System-health disk usage now reports `df`'s `used / (used + available)`
  rather than `used / total`, so a host with reserved blocks or a container
  quota no longer shows a contradictory "11% used · critically low".

- **Post-startup `SIGTERM` no longer hangs the process.** The startup-phase
  signal race in `app::run` kept racing the serve loop after startup; its
  biased arm won every later `SIGTERM`, cancelled the graceful-shutdown
  choreography (waking live connections, stopping the detection daemon), and
  left the runtime blocked forever on the detection loop's blocking thread —
  so every `systemctl stop`/`restart` with a loaded model waited out
  `TimeoutStopSec` and was `SIGKILL`-ed. The race now ends at an explicit
  startup handoff; verified live: clean stop in ~2 s with the pipeline hot.
- `--doctor` now validates the model and labels of a config-file install: it
  read the `MODEL` / `LABELS` keys while the daemon and installer use
  `MODEL_PATH` / `LABELS_PATH`, so every standard install reported
  `SKIP: no --model configured` and the model file was never checked.
- The documented image-cache opt-out (`--image-cache-dir ""`, empty
  `BIRDNET_IMAGE_CACHE_DIR`) actually parses now — clap's stock `PathBuf`
  parser rejects empty values, making the air-gapped opt-out unreachable
  from the CLI/env (the config-file key was unaffected).
- BirdNET-Pi migration no longer aborts on dirty source data: TEXT values in
  numeric columns (empty strings, stringified numbers — the upstream
  "empty-string poisoning") degrade to NULL or parse, instead of failing the
  whole import with `InvalidColumnType`.
- Unmatched paths under `/api/` return a machine-readable JSON 404 instead
  of the branded HTML page, so scripts and dashboards see the real failure.

### Security

- Auto-update HTTP reads are bounded (release metadata 8 MiB, `SHA256SUMS`
  64 KiB, release asset 512 MiB) with `Content-Length` pre-checks, so a
  compromised or misbehaving endpoint cannot stream an unbounded body into
  memory on a small-RAM Pi.
- Every GitHub Actions step is now pinned to a full commit SHA (previously a
  mix of tags and three mutable `@main`/`@master` refs), and `ci.yml` gained
  the least-privilege `permissions: contents: read` block the other
  workflows already had.

### CI

- **Mutation testing is now incremental on PRs and ~4× cheaper per mutant.**
  Three layers, each measured: a `mutants` build profile (no debug info —
  per-mutant cost 132 s → 36 s, baseline 90 s + 91 s → 16 s + 21 s on the
  binary-crate shards); unit-test-only target selection per package
  (`--lib` / `--bins`), so the mutant loop no longer rebuilds eight
  DuckDB-linking integration-test executables nor boots real binaries; and
  `--in-diff` scoping on pull requests, so only mutants on changed lines
  run (a test-only one-line diff finishes in 0.2 s, "No mutants to
  filter") while the weekly cron, pushes to main, and manual dispatch
  still run every shard's full set. Config lives in `.cargo/mutants.toml`
  so local `cargo mutants` runs share the same economics.

### Dependencies

- `mdbook` 0.4.52 → **`mdbook-driver` 0.5.3** (folds dependabot #151): mdbook
  0.5 split the project into facade crates and made the `mdbook` crate
  binary-only, so the docs build now consumes the library through
  `mdbook-driver`. The book config dropped the options 0.5 removed
  (`copy-fonts`, `multilingual`), `build.rs` now surfaces the *underlying*
  load error instead of a silent "could not load" (that silence briefly
  masked exactly this migration), and the rendered manual was verified
  page-for-page. New transitive `font-awesome-as-a-crate` carries
  `CC-BY-4.0 AND MIT` for the icon *assets* (attribution-only, not
  copyleft) — allowed via a crate-scoped `deny.toml` exception rather than
  a global allow.
- `rusqlite` 0.40.0 → 0.40.1 (folds dependabot #147).
- `codecov/codecov-action` v6 → v7.0.0, SHA-pinned (folds dependabot #150).
- `password-hash` 0.5 → 0.6 (dependabot #148) is **deliberately not
  taken**: argon2 0.5.x implements password-hash *0.5*'s hasher traits and
  our accounts code passes those types straight into `Argon2` — the bump
  alone does not compile (verified). A manifest comment now documents the
  lock-step requirement; take both together when argon2 0.6 ships.

## [0.7.2] - 2026-06-07

A pre-release hardening pass: process-crash fixes, memory/DoS bounds for small
Raspberry Pis, data-integrity fixes, and several web-security fixes — plus an
internal module-structure cleanup. No user-facing feature changes; everything
here makes an existing install more robust against malformed input, hostile
station metadata, over-long recordings, and abrupt shutdown.

### Security

- **Neutralised CSV formula injection in data exports (CWE-1236).** A species or
  comment beginning with `=`, `+`, `-`, or `@` is no longer written verbatim into
  exported CSVs, where a spreadsheet would evaluate it as a formula. Such fields
  are now prefixed so they import as literal text, and the record-separator /
  control characters that can splice extra rows are stripped.
- **Pinned auto-update downloads to GitHub release hosts over HTTPS.** The
  self-updater now refuses any release-asset URL that is not an `https://` GitHub
  host, so a tampered release feed cannot redirect the download to an arbitrary
  origin.
- **Escaped Home Assistant MQTT discovery payloads.** Discovery messages are now
  emitted as properly encoded JSON, so a station name containing quotes, braces,
  or control characters can no longer break out of the payload or inject fields.
- **Stopped leaking internal error detail to the admin UI.** Recording-save and
  related failures now surface a generic message to the browser and log the
  detail server-side, instead of echoing internal paths and error strings into
  the page.
- **Bounded request-driven work on the web surface.** On-demand spectrogram
  rendering and the live stream are now concurrency-limited, deterministic `4xx`
  client errors are no longer retried, and spectrogram parameters are sanitised —
  closing several avenues for a single client to pin CPU or memory on a small Pi.
- **Closed an auto-update host-pin bypass via URL userinfo.** The release-asset
  host check parsed the authority by splitting on `:`, so a URL like
  `https://github.com:x@evil.com/…` read as the trusted host `github.com` while
  the download would actually go to `evil.com`. The host is now taken from the
  segment after the last `@` (userinfo stripped), closing the spoof for both the
  binary download and the `SHA256SUMS` fetch.
- **Clamped the public analytics query parameters.** The unauthenticated
  `/analytics` endpoints now cap the `limit` and the `?species=` sequence length,
  so a single request can't force an oversized result set or sequence on a Pi.

### Fixed

- **`stop`, `restart`, and upgrades no longer stall ~10 s on every shutdown.**
  The live dashboard holds a WebSocket open (the listen page a second one, and
  the admin Live Logs page an SSE stream). On `SIGTERM`, axum's graceful drain
  waited for those to close on their own, so with any tab open it always hit the
  `SHUTDOWN_GRACE` cap and force-exited with `shutdown grace elapsed with
  connection(s) still open`. The server now signals those handlers to close the
  moment shutdown begins, so the drain finishes in milliseconds and shutdown is
  clean and quiet. The 10 s cap stays only as a backstop for a client that
  ignores the close.
- **Several panics that would abort the whole process are gone.** Because release
  builds compile with `panic = "abort"`, any unhandled panic in a request handler
  or background task takes the entire daemon down. This pass fixes a class of
  them: date parsing that sliced multibyte UTF-8 rows on a byte boundary, webhook
  URLs truncated mid-character in the rules table, and a `date_to_epoch_days`
  underflow on pre-epoch dates (now clamped to the epoch). Malformed or unusual
  data is handled instead of crashing.
- **Poisoned locks no longer wedge analytics and image fetches.** If a thread
  panicked while holding certain mutexes (the full-analytics resync, the
  Wikipedia image cache), every later caller would panic on the poisoned lock in
  turn. Those paths now recover the guard and continue.
- **The DuckDB analytics copy can no longer be wiped by a failed rebuild.** The
  full resync is now atomic: it builds the new OLAP copy and swaps it in only on
  success, so an error partway through leaves the previous analytics intact
  instead of emptying them.
- **Settings writes are atomic.** A configuration save now lands as a single
  transaction, so a crash or concurrent reader can't observe a half-written
  settings row, and the surrounding DB resilience paths were hardened.
- **Long recordings can't exhaust memory.** On-demand spectrogram decoding is now
  capped at ten minutes of audio (≈115 MB), so an unusually long station
  recording — or a misconfigured multi-minute segment — renders its leading
  portion instead of allocating an unbounded buffer and risking an OOM on a Pi.
  The detection pipeline still decodes every sample.
- **Audio seeking works in the recordings player.** The recording endpoint now
  honours HTTP `Range` requests, so scrubbing within a clip seeks in the browser
  instead of re-fetching from the start.
- **Assorted correctness and robustness edge cases** surfaced by the pre-release
  audit — input validation on several admin forms, daemon and purge edge cases,
  scheduler and identifier handling, and live-frame broadcast sizing.
- **Uploaded BirdNET-Pi databases now rebuild the analytics copy too.** The 0.7.1
  fix that refreshes the DuckDB analytics after an import only covered the
  server-path import; the browser upload path imported history into SQLite but
  skipped the rebuild, so uploaded back-dated history silently never reached the
  behavioural / time-series analytics. The upload path now rebuilds it like the
  server path.
- **The i18n lock recovers from poison instead of aborting the daemon.** It was
  the lone lock in the web layer that propagated a poisoned lock via `expect()`;
  under `panic = "abort"` that would take the daemon down. It now recovers the
  guard like every other lock in the crate.

### Changed

- **Internal module-structure cleanup (no behaviour change).** Several oversized
  files were split into focused submodules behind unchanged public paths: the
  1319-line `capture.rs` supervisor, the detection daemon (into process and
  run-loop submodules), `detections.rs` (by query concern), `viz.rs` (chart
  renderers by visual family), `accounts.rs` (by store), and the version logic
  in `auto_update`. The whole tree is now `cargo fmt`-clean.

## [0.7.1] - 2026-06-05

### Fixed

- **Imported history now reaches the behavioural analytics with its original
  timestamps.** A BirdNET-Pi import writes back-dated detections straight to
  SQLite, but the DuckDB analytics copy only ever synced *incrementally* (rows
  newer than the latest already synced) and was never refreshed after an import —
  so a year of imported history was silently invisible to the behavioural and
  time-series dashboards. The import now rebuilds the DuckDB copy in full once the
  rows land, and the migration progress UI shows the "Rebuilding analytics…" step.
- **The confidence threshold is no longer advertised at one value and enforced at
  another.** The detection daemon defaulted to recording everything ≥ 0.25 while
  the settings form displayed 0.70, so a stock station recorded far more than the
  operator believed. Both now read a single shared default (0.7, matching
  BirdNET-Pi), and the installer's documented default matches.
- **The System page disk panel shows real filesystem usage.** It previously
  reported only the database file's size; it now reports actual used/free space
  for the data filesystem (with a "running low" / "critically low" note) — the
  metric that determines whether recording will run out of room.
- **CPU temperature now reads on a Raspberry Pi.** `sysinfo`'s component sensors
  are routinely empty on a Pi; the System page now falls back to the Linux
  thermal-zone sysfs (`/sys/class/thermal`), preferring the CPU/SoC zone.
- **The dashboard "live signal" is honest.** The idle state no longer animates a
  synthetic sine wave that could be mistaken for live audio — it draws a flat
  baseline, and the indicator reads "live" only while genuine spectrogram frames
  are arriving from the capture device, "idle" otherwise.
- **First-run setup no longer offers a lockout footgun.** The interactive
  installer dropped the "Restrict the dashboard to THIS device only?" prompt that
  could strand a non-technical operator on localhost; the restriction remains an
  explicit, advanced `BIRDNET_LISTEN=127.0.0.1:8502` knob.

### Added

- **Multi-stream source attribution.** Every detection is now tagged with a
  first-class `Source` (the RTSP stream id, e.g. `cam1`, or `local` for the
  on-board mic; migration 18, indexed). Non-destructive — historical / imported
  rows stay `NULL` and nothing is rewritten. The detection-detail page uses it
  for **"also heard by"** corroboration: when other audio sources detected the
  same species at nearly the same time, they're listed as confirmation the
  detection is real (a read-only view; it never merges or hides rows). Single-mic
  stations see no change. Groundwork and the corroboration-first design for
  optional cross-stream collapse are in `docs/MULTISTREAM_DEDUP.md`.
- **A pre-warmed query cache for the heavy analytics.** A short-TTL in-memory
  cache now backs the heaviest fragments on the Heatmap, Migration/phenology,
  Co-occurrence, and Time-series (DuckDB) pages, and a background task pre-warms
  the default views shortly after startup and every few minutes after — so jumping
  between analytics pages is snappy on a Raspberry Pi 4 instead of re-running
  multi-second aggregate scans on every visit. Live surfaces (the detection feed
  and stat tiles) stay uncached and real-time.
- **BirdNET-Pi-style "Best recordings" on the dashboard.** A new at-a-glance card
  shows the day's highest-confidence detections that have a playable clip, so the
  best captures are one glance away instead of a hunt through the recordings
  browser.
- **A composite `(Date, Com_Name)` index** so the per-species date-range
  aggregates (sparklines, phenology, co-occurrence) are index-range scans rather
  than full-table scans.
- **A scannable QR of the dashboard URL** in `install.sh` and `quickstart.sh`, so
  a phone can open the station without anyone typing an IP (best-effort via
  `qrencode`).

### Changed

- **The post-install URL is IP-first.** Both installers now lead with the LAN IP
  (which always resolves on the network) and demote the mDNS `.local` name to a
  clearly-captioned secondary — mDNS is not universal, and leading with it could
  leave a phone unable to open the page.
- **`sysinfo` 0.39.2 → 0.39.3** for a Linux fix that hardens process-information
  retrieval when a process exits mid-refresh (supersedes Dependabot #130).
- **The dawn-chorus query is no longer N+1**: the top species' hourly histograms
  are fetched in a single grouped scan instead of one query per species.

## [0.7.0] - 2026-06-04

### Added

- **`--doctor` now checks the analytics preconditions.** The diagnostic gained
  an "Analytics (behavioral)" check that reports, with an actionable fix,
  whether behavioral analytics will actually work on this install: it **warns**
  when an analytics database is configured but the binary was built without
  analytics (a slim build pointed at a release config — the dashboards would
  silently stay empty), notes when analytics is explicitly disabled, and
  otherwise confirms analytics is enabled and that its DuckDB directory is
  writable. It deliberately opens no DuckDB during the preflight, so it adds no
  startup contention when the unit runs `--doctor` as `ExecStartPre`.
- **Offline / air-gapped install.** `install.sh` can now install from a release
  tarball already on disk — `BIRDNET_BINARY_TARBALL=/path/to/…tar.gz sudo -E
  bash install.sh` — skipping the GitHub fetch and checksum round-trip for a
  local file the operator placed themselves. Paired with `BIRDNET_SKIP_MODEL=1`
  (stage the ~541 MB model out-of-band), a station with no internet can be
  installed end to end. The installer also **degrades gracefully without
  systemd** (containers, chroots, staged images): it writes the binary, config,
  and unit file, then prints how to enable the service on a real host instead of
  aborting at the first `systemctl` call.
- **Install smoke test in CI** (`.github/workflows/install-smoke.yml`). On every
  change to the installer or the binary, CI builds the binary, then runs the
  *real* `install.sh` against it in a clean, network-less, no-systemd
  `ubuntu:24.04` container (via the new air-gapped path) and asserts the install
  completes and the dashboard actually serves (`/api/v2/health` reports
  `healthy`, `/` returns 200). This catches the class of regression that ships
  green unit tests but a broken operator install.

### Changed

- **Network retries now use jittered, capped, overflow-safe backoff.** The
  BirdWeather and Apprise clients retried transient failures on a fixed
  `2^attempt` schedule, so concurrent retries — and many stations posting on the
  same cadence — would wake in lockstep and hammer a recovering endpoint (a
  thundering herd). Both now share a backoff helper that adds **equal jitter**
  (each retry lands in a window rather than at one instant), **caps** the delay
  at 32 s so a long outage settles at a steady cadence, and is **overflow-safe**
  regardless of the attempt count.
- **The admin panel now renders entirely through one shared shell.** Six admin
  pages — Overview, Settings, Audio (already), Migration, Rules, System, and
  Notifications — each shipped (or, for the nav tabs, several still shipped)
  their own standalone HTML document with a bespoke top `<nav>` that disagreed
  with the admin shell's nav and with each other. **Every admin nav destination**
  now renders through the shared `admin_shell`, whose navigation is generated
  from a **single admin-nav manifest** (`routes/admin/nav.rs`) — so they show the
  same tabs with consistent active-state, gain a breadcrumb trail, and pick up
  the command palette / help drawer / toast region. The Migration tab, which was
  missing from the shell nav, is now part of the manifest. A parity test
  (`admin_router_serves_every_nav_destination`) guards that every admin nav
  destination resolves to a real route, and a runtime test
  (`folded_pages_render_through_the_shared_shell`) confirms each folded page
  actually composes the shell — mirroring `cmdk_covers_every_nav_destination`
  for the main nav.
- **Species management is now a first-class admin tab, and the admin sub-pages
  follow the standard "sense of place" pattern.** Managing which birds are
  detected/excluded is core to running a station, so **Species** is now its own
  admin nav tab rather than a quick-link a non-technical operator has to hunt
  for. The remaining sub-pages — the species **Filter test**, **Test
  notifications**, and the **Species images** blacklist — now render through the
  shared shell too: each highlights its **parent tab** (Species or Notifications)
  and shows a breadcrumb down to itself (`Home › Admin › <Parent> › <page>`), so
  you always know where you are and have a one-click way back. No admin page
  ships bespoke chrome any more.

### Fixed

- **The installer's completion summary shows the real dashboard port.** When an
  operator set a custom `BIRDNET_LISTEN` (e.g. `…:8599`), the post-install
  summary still printed the URL with the hardcoded `:8502`. It now derives the
  port from the configured listen address.
- **Installation input is now respected in the web UI.** The installer writes
  station settings (latitude/longitude, audio device, station name, …) to
  `/etc/birdnet/birdnet.conf`, and the Docker image passes them as `BIRDNET_*`
  environment variables — but the admin settings form and the first-run
  onboarding check read only the SQLite `settings` table, so a fully-configured
  station showed blank fields and was bounced to the onboarding wizard it had
  already effectively completed. The installed configuration (file **and**
  env/flags) is now seeded into the `settings` table on first start — insert-only,
  so a value the operator later changes in the UI is never overwritten — and a
  station that already has coordinates is no longer redirected to onboarding.
- **The "More" navigation menu no longer renders as overlapping/garbled text.**
  The topnav dropdown and the mobile bottom sheet both ship a `data-open-more`
  opener, and each opener's script selected the *first* one in the DOM — so the
  topnav button opened **both** menus at once (stacked on top of each other) and
  the mobile button opened none. Each opener is now scoped to its own dialog via
  `aria-controls`.
- **The Admin → Settings "saved" confirmation no longer renders a full-screen
  checkmark.** The success icon referenced utility classes that don't exist in
  the hand-written stylesheet, so the SVG rendered unconstrained; it now carries
  an explicit 16×16 size.
- **Live audio is reachable from the navigation.** The `/listen` page (per-source
  playback + live spectrogram + a live detection trickle) is now linked from the
  "More" menu, the mobile sheet, and the Audio settings section — so confirming a
  microphone is working no longer requires typing the URL by hand.
- **The installer falls back to Zenodo immediately when the GitHub model release
  is absent.** The ~541 MB model fetch no longer retries a definitive `404` five
  times with back-off before trying the next source; a missing GitHub asset now
  falls through to Zenodo at once, matching the labels fetch and the Docker
  entrypoint.
- **Importing a real BirdNET-Pi database works again.** The upload endpoint
  inherited axum's default 2 MiB request-body limit, so any real `birds.db`
  (tens to hundreds of MB, sometimes several GB) was rejected before the importer
  ever ran — the import feature was effectively dead. The DB-upload route now
  accepts large files (admin-only) **and streams the upload straight to disk**
  rather than buffering it (twice) in memory: a 163 MB upload now adds ~7 MB to
  peak RSS instead of ~330 MB, so a multi-hundred-MB database imports with flat
  memory instead of OOM-ing a Raspberry Pi. (For a database already on the Pi,
  the "Server Path" tab imports it with no upload at all.)
- **An RTSP source's transport (TCP/UDP/Auto) is now honoured.** The per-source
  transport the admin UI exposes was silently dropped and ffmpeg was always
  forced to TCP, so a camera that only speaks UDP could never be captured. The
  choice now reaches the capture command (`Auto` keeps the TCP default).
- **A per-source capture gain (`gain_db`) is now applied.** The gain the admin
  UI stores and displays for each source had no effect on capture. A non-zero
  gain now routes that source through `ffmpeg`'s `volume` filter
  (`-af volume=<n>dB`) — for a local microphone this switches it from `arecord`
  to `ffmpeg -f alsa`, since `arecord` has no software-gain control; unity-gain
  microphones stay on the lighter `arecord` path unchanged. A negative value
  cuts the level just as a positive one boosts it.
- **A per-source quiet window (`schedule_quiet`) is now enforced.** The quiet
  window stored per source was previously inert. The capture supervisor now
  pauses a source while the wall clock is inside its window and resumes it
  afterwards, on top of the global recording schedule (the source records only
  when the schedule allows it **and** it is outside its quiet window). The
  window uses the same clock basis as the recording schedule (UTC), wraps past
  midnight (e.g. `22:00`–`06:00`), and — like the schedule — is not enforced
  while the clock looks unsynced, so a bogus boot-time date can't silence a
  source. Editing gain or the quiet window takes effect on the next service
  restart, consistent with the other per-source settings. See
  `docs/FIELD_DEPLOYMENT.md` § 7 for the manual hardware-verification steps.
- **Multiple RTSP streams can be configured from the config file.** A new
  comma-separated `RTSP_URLS` config key drives several RTSP captures without
  the `--rtsp-urls` flag, and a multi-stream station no longer mislabels its
  first stream `rtsp` (every stream is numbered `RTSP_1`, `RTSP_2`, … once there
  is more than one).
- **Restoring a backup works for real archives.** `/admin/system/restore` had the
  same flaw as the import — it inherited the 2 MiB body limit and buffered the
  whole `.tar.gz` in memory — so restoring any real backup (database + recordings,
  often several GB) was rejected or OOM-ed the process. It now streams the upload
  to disk and lifts the limit on that admin-only route.
- **The system-status panel no longer blocks the async runtime.**
  `/admin/system/service/status` read `/proc` and spawned `getconf` / `systemctl`
  synchronously inside the request handler; that work now runs on a blocking
  thread so a slow `/proc` or a hung `systemctl` can't stall unrelated requests.
- **Navigation is consolidated and consistent.** The desktop top-nav, the "More"
  dropdown, the mobile tab bar + sheet, the breadcrumb trail, and the ⌘K command
  palette were separately hand-maintained lists that had drifted: `/live` was an
  orphan reachable from no menu, the mobile sheet was missing `/kiosk` and
  `/help`, `/analytics` was absent from mobile entirely, and seven pages
  highlighted the wrong section. They now all derive from — or are parity-tested
  against — a single navigation manifest. Added **breadcrumbs** on secondary
  pages (there were none), grouped the previously-flat mobile sheet, corrected the
  seven active-state mismatches, and redirected the orphaned `/live` to the
  maintained `/listen`.

### CI

- **CI now proves the behavioral extension loads with no network.** Analytics
  ships bundled — the release binary embeds the community `behavioral` extension
  so `LOAD behavioral` works offline on a fresh, air-gapped install — but the
  test that proves it (`embedded_extension_loads_when_bundled`) previously
  *skipped* in CI because no extension was embedded in the test build. The
  `--all-features` test job now fetches and embeds the extension first (the same
  mechanism release.yml uses), so the test runs its real assertion — loading the
  extension from the embedded bytes via a temp file with no network — and a
  dedicated step surfaces the result. Best-effort: if the registry is
  unreachable the test skips as before, adding no flakiness.
- **The mutation-testing job timeout is now matrix-driven.** The three
  binary-crate shards (`src/daemon/`, `src/capture/supervisor.rs`,
  `src/capture/schedule.rs`) rebuild the binary + web tree per mutant and were
  being `cancelled` at the flat 45-minute limit on cold caches. The job now uses
  `timeout-minutes: ${{ matrix.timeout_minutes || 45 }}` and those three rows set
  `timeout_minutes: 90`, so they report `success` instead of `cancelled`.

## [0.6.0] - 2026-06-03

The largest release since the first public one. BirdNet-Behavior gets a
ground-up dashboard redesign, **DuckDB behavioral analytics on by default**, a
real first-run onboarding wizard, account-based authentication, and a
fully self-contained, offline-capable install — the binary, the ~541 MB
BirdNET+ model, and the operator manual all come from a single GitHub origin,
checksum-verified. The release/CI pipeline is hardened end to end (the
integration branch is now gated, the auto-updater verifies what it installs,
and there are full-pipeline, migration, and soak tests). New schema migrations
(audio sources, accounts/sessions) run automatically and idempotently on first
start — no manual steps.

### Added

- **A ground-up dashboard redesign.** 20+ server-rendered HTMX pages on a
  unified design system: OKLCH color tokens, first-class dark/light and
  reduced-motion support, self-hosted fonts, and SVG-rendered visualizations.
  New surfaces include a command palette, a live homepage spectrogram fed by a
  WebSocket producer, a `/listen` page wiring per-source audio + spectrogram, a
  polar dawn-chorus moon-phase ring, an in-app help drawer, and an
  `/admin/audit` log with date-range and action filters.
- **DuckDB behavioral analytics on by default.** The analytics engine
  (sessionize, retention, funnel, sequence, next-species) is compiled into
  every binary *and enabled out of the box*. The community `behavioral` DuckDB
  extension is embedded into the release binary at build time, so analytics
  work fully offline on first run with no network `INSTALL`.
- **Multi-source audio capture.** Audio sources are now first-class,
  CRUD-managed rows (ALSA / PipeWire / RTSP / multiple RTSP), seeded from the
  CLI and config; the capture pipeline, `/listen`, and the metrics gauges all
  read from them, retiring the legacy single-string source.
- **Account-based authentication.** argon2id password hashing with cookie
  sessions and a CSRF guard, role-based access control enforced on every
  `/admin` write, an admin password reset, and session pruning. The legacy
  HTTP Basic Auth path is removed.
- **A real first-run onboarding wizard.** It persists location, timezone, and
  notification settings and redirects a fresh station to `/onboarding`, with an
  IP-geolocation auto-detect that fills latitude/longitude and the IANA
  timezone. A new doctor clock/timezone check surfaces an unset or unsynced
  system clock in plain language.
- **`doctor --fix` self-heal.** Safe, idempotent repairs (recreating missing
  configured directories — the #1 "service runs but records nothing" cause)
  run before the diagnostic, as the unprivileged service user.
- **Offline-capable model + manual bundling.** The ~541 MB BirdNET+ V3.0 model
  and labels are now a single shared, arch-independent GitHub release asset
  (`models-v3.0-preview3`), fetched from the same origin as the binary,
  **verified against a pinned sha256**, resumable, and falling back to Zenodo
  (the upstream source) when unavailable — so a fresh install needs one network
  origin and is offline-capable afterwards. A `publish-model.yml` workflow
  mirrors the model with checksum-pinned provenance (SHA256SUMS + SLSA).
- **An embedded operator manual at `/help`.** The mdBook manual is rendered at
  build time and shipped both in the Docker image and the install tarball
  (screenshots downscaled for the bundle; the committed source and the GitHub
  Pages site stay full-res), served offline at `/help`. The in-app help links
  are wired across 19 screens.
- **A hardened release & test pipeline.** CI now gates the integration branch
  (`claude/**` PRs run fmt, clippy, tests, rustdoc, MSRV, and an aarch64
  cross-check); a full-pipeline E2E test (audio → infer → DB → web), a
  BirdNET-Pi migration integration test, and a compressed soak/longevity test
  assert bounded memory/fd/DB growth. A deterministic demo-data seeder feeds a
  refreshed 48-image screenshot set.
- **Weather polling** — records conditions alongside detections; off by default.

### Changed

- **Content-Security-Policy hardened.** `script-src` is now a per-request nonce
  plus `strict-dynamic`; every inline `on*` handler moved to
  `addEventListener`; and `style-src 'unsafe-inline'` is dropped — the entire
  template surface was swept off inline styles onto utility classes, guarded by
  an inline-style regression test.
- **The auto-updater now verifies what it installs.** The downloaded archive is
  sha256-checked against the release `SHA256SUMS` and the staged binary is
  smoke-tested (`<binary> --version`) *before* the atomic swap; a wrong-arch,
  truncated, or corrupt download is rejected and the running binary is left
  untouched. (SLSA provenance remains the out-of-band authenticity path.)
- Settings accept locale-tolerant decimals and skip unchanged fields on save.

### Fixed

- **`/help` deep links no longer 404.** mdBook emits `<page>.html`, but the
  in-app help links use clean, extensionless URLs; a small middleware now
  rewrites `/help/…` to the rendered `.html` before serving, while `/help/`
  and static assets pass through.
- **The Docker image builds again and ships correct analytics.** `CHANGELOG.md`
  is kept in the build context (it is embedded into the binary at compile
  time), and each architecture embeds its matching DuckDB `behavioral`
  extension instead of defaulting to the amd64 build.
- Wikipedia species images are fetched on cache-miss, and the admin image
  blacklist is enforced on the serve path.

### Security

- CSP per-request nonce + `strict-dynamic`, with no inline script or style.
- Admin actions require an authenticated session with the right role (RBAC);
  passwords are argon2id-hashed; a stateless CSRF guard covers state changes.
- The auto-updater and the bundled model are both integrity-verified
  (sha256) against a provenance-attested origin before anything touches disk.

## [0.5.3] - 2026-05-27

Field-hardening release from real Raspberry Pi + RTSP testing. The service now
starts and shuts down cleanly, RTSP stations actually record detections, the
dashboard is reachable on the LAN with only its admin panel behind a password,
and `install.sh` gains guided repair/update/reinstall/uninstall flows with
pre-flight and post-install validation. No database migration is required.

### Fixed

- **The systemd service no longer fails to start with
  `Failed to set up mount namespacing: /tmp/birdnet-stream: No such file or directory`
  (exit `226/NAMESPACE`).** The unit listed the tmpfs stream directory in
  `ReadWritePaths=` while also setting `PrivateTmp=yes`; systemd mounts a fresh
  empty `/tmp` for the service, so bind-mounting a path *beneath* it fails
  namespace setup and the service never starts. The stream dir is removed from
  `ReadWritePaths=` (the private `/tmp` is already writable) and an
  `ExecStartPre=/bin/mkdir -p` recreates it on every start. Existing broken
  installs are fixed by `sudo bash install.sh repair` (or any update/reinstall).
- **The detection daemon creates its watch directory before attaching the file
  watcher.** With `PrivateTmp=yes` the service's `/tmp` is wiped on every
  restart, so `start_detection_daemon` now `create_dir_all`s the watch dir
  up front — a missing directory previously made `notify` error out and
  silently disabled detection (web UI up, nothing analysed).
- **The service shuts down promptly instead of hanging until SIGKILL.** A live
  WebSocket/event-stream client (the dashboard keeps one open) kept axum's
  graceful shutdown from ever completing, so `stop`/`restart`/uninstall blocked
  until systemd SIGKILLed the process at `TimeoutStopSec` (30 s) and left a
  ghost `Active: failed (timeout)`. Shutdown now caps the connection drain
  (`SHUTDOWN_GRACE`, 10 s) and signals the detection loop to stop so the runtime
  winds down cleanly.
- **`install.sh uninstall` is clean, idempotent, and fool-proof.** It now runs
  `systemctl reset-failed` so the removed unit no longer lingers as
  `Active: failed (timeout)` in `systemctl status`, reports accurately what was
  (or wasn't) present, can also delete data/config (interactive prompt or
  `BIRDNET_PURGE=1`) behind a path-safety guard, and verifies at the end that no
  service or binary remains. Re-running it when nothing is installed is a clean
  no-op.
- **`uninstall.sh --purge` renders its plan correctly and guides recovery.** It
  printed literal `\033[1m…` escape codes (colours are now real ESC bytes); and
  when the config and service are already gone, the guessed-data-dir guard now
  prints the exact `--data-dir` argument to re-run with.
- **RTSP/segmented captures no longer fail with `decode error: ... unexpected
  end of file`.** The watcher decoded each clip on every create/modify event,
  so an ffmpeg segment still being written (RTSP captures a clip in place over
  ~15 s) was decoded while incomplete and reprocessed on every write — meaning
  **zero detections** for RTSP stations. The daemon now debounces: a file is
  decoded once its size has been stable for a short settle window, and exactly
  once.

### Added

- **`install.sh` commands and an existing-install menu.** Running the installer
  on a machine that already has BirdNet-Behavior now offers **update**,
  **repair**, **reinstall**, and **uninstall** (interactively), or you can pass
  one explicitly (`sudo bash install.sh repair`). Non-interactive runs keep the
  historical auto-update behaviour. `repair` re-creates directories, fixes
  ownership/permissions, rewrites the systemd unit, and restarts — without
  re-downloading the binary or model.
- **Pre-flight and post-install validation in `install.sh`.** Before downloading
  it checks for required tools and sufficient free disk; afterwards it validates
  the binary runs, the unit verifies (`systemd-analyze verify`), directories are
  owned by the service user, the config is readable by the daemon, the doctor
  preflight passes, and the web port is listening.
- **`install.sh` ensures the ffmpeg capture backend for RTSP stations.** When the
  config has an `RTSP_URL` (which captures through ffmpeg), install/repair now
  install ffmpeg automatically (`apt-get`), or warn with the exact command if it
  can't — previously an RTSP station with no ffmpeg passed the installer but the
  daemon then failed the doctor preflight and never started.

- **The dashboard bind address persists across installer re-runs.** `repair`
  and `update` no longer silently re-hide a LAN-exposed dashboard on localhost:
  the bind address is read from `BIRDNET_LISTEN` (env or the config file) and,
  failing that, carried forward from the existing service unit. A fresh install
  records it as `BIRDNET_LISTEN=` in the config so it is visible and editable.

### Changed

- **The dashboard is reachable on the LAN out of the box, with the admin panel
  gated by a password.** The default bind is now `0.0.0.0:8502` (was
  `127.0.0.1:8502`, which left non-technical users at "connection refused").
  Only the `/admin` panel — settings, software update, system controls — now
  requires HTTP Basic Auth (route-level, enforced by the binary); viewing the
  dashboard is open. A fresh install **auto-generates a strong admin password**
  (user `birdnet`, shown in the post-install summary and saved as `CADDY_PWD`),
  so the admin surface is protected by default. Restrict the whole dashboard to
  this host again with `BIRDNET_LISTEN=127.0.0.1:8502` (env, config, or the
  interactive prompt).
- **`install.sh` is now assembled from single-responsibility modules under
  `installer/lib/*.sh` by `installer/build.sh`** (developer-facing only — the
  shipped `install.sh` is still one self-contained, checksummed file). A CI gate
  and pre-commit hook verify the generated `install.sh` stays in sync with its
  modules.

## [0.5.2] - 2026-05-27

Installer- and documentation-focused release: it repairs the bare-metal install
flow on Raspberry Pi OS Trixie, adds guided onboarding, and tightens the install
to least privilege. There are no functional changes to the compiled binary —
only its reported version differs from 0.5.1.

### Added

- **Guided onboarding in `install.sh`.** A fresh interactive install now prompts
  for an audio source (auto-detected ALSA device, a typed ALSA device, or an
  RTSP URL), station latitude/longitude, and whether to expose the dashboard to
  the LAN — writing them into the config so a non-technical user gets a working
  station without hand-editing a file, and the post-install summary says exactly
  which URL to open in a web browser (and from which device). Prompts read from
  `/dev/tty`, so they work under `curl … | sudo bash`. `--noninteractive` (or
  `BIRDNET_NONINTERACTIVE=1`) keeps unattended installs silent.
- **`install.sh --version X.Y.Z` / `-v`** to pin a release through the pipe form
  (`curl … | sudo bash -s -- --version X.Y.Z`); the `VERSION` environment
  variable still works.

### Security

- **The web dashboard binds `127.0.0.1` by default** instead of `0.0.0.0`. The
  admin UI can change settings and update software, so it is no longer exposed to
  the whole LAN unauthenticated out of the box. The interactive installer offers
  LAN exposure and captures a password (HTTP basic auth) when you opt in; the
  bind is overridable with `BIRDNET_LISTEN`.
- **`/etc/birdnet/birdnet.conf` is now `0640 root:<service-group>`** (was
  world-readable `0644`), so secrets such as `CADDY_PWD` and `BIRDWEATHER_TOKEN`
  aren't readable by other local users; existing configs are retightened on
  upgrade.
- **Tighter filesystem and service sandboxing.** Data, recordings, model, and
  tmpfs-stream directories are `0750` (were `0755`); the systemd unit adds
  `CapabilityBoundingSet=` (all dropped), `UMask=0027`, and
  `RestrictAddressFamilies=`. Measured `systemd-analyze security` exposure
  dropped from 4.0 to 1.6.

### Fixed

- **Bare-metal install over `sudo` now works end to end on Raspberry Pi OS
  Trixie:**
  - Version pinning no longer needs the broken `sudo bash <(curl …)` form
    (process substitution + `sudo` closes the pipe's file descriptor, so the
    script vanished); docs and generated release notes use the pipe form.
  - The resolved version is no longer corrupted by an `[INFO]` log line bleeding
    into the captured value (which produced `curl: (3) bad range in URL`) — the
    log helpers now write to stderr.
  - The data directory is created under the service user's real home instead of
    `/root` (where `sudo` pointed `$HOME`), so the non-root service can reach its
    database, recordings, and model.
  - ALSA microphone auto-detection no longer fails with `awk: syntax error` on
    Debian / Raspberry Pi OS (replaced a gawk-only `match()` form with a portable
    one).

### Changed

- **CI:** the `Tests (x86_64)` job frees ~25–30 GB of preinstalled SDKs before
  the all-features build, fixing intermittent `No space left on device` failures.

## [0.5.1] - 2026-05-26

### Added

- **CI now compiles and tests the `analytics` feature** (clippy, tests, MSRV
  check, and rustdoc) and adds an **aarch64 (Raspberry Pi) cross-check** on
  every PR — closing the blind spot that let analytics bugs ship undetected.
- **`/api/v2/health` reports `detection_daemon`** (`running`/`stopped`), so
  monitoring can tell a capturing station from one running web-only or with a
  misconfigured model/labels/watch-dir.
- **`BIRDNET_CORS_ALLOWED_ORIGINS`** to allow specific cross-origin origins.
- **`docs/SECURITY_HARDENING.md`** — a deployment hardening guide (network
  exposure, authentication, CORS, privacy, backups, and release verification).

### Changed

- **Configuration is validated at startup**; the daemon now refuses to start on
  an invalid setting (e.g. a latitude outside ±90, a malformed
  `RECORDING_SCHEDULE`) instead of running silently degraded.
- **Database migrations are atomic** — each migration's schema change and its
  version bump commit in one transaction — and a migration failure is now fatal
  at startup rather than serving an under-migrated schema.
- **The detection-event channel is bounded**, so a stalled consumer applies
  backpressure (tripping the systemd watchdog) instead of buffering until the
  process is OOM-killed; the `--process-existing` backlog now runs after the
  server signals readiness.
- **Routine dependency and CI-action updates** — `rusqlite` 0.39 → 0.40
  (pulling `libsqlite3-sys` 0.38), `reqwest` 0.13.3 → 0.13.4, and
  `codecov/codecov-action` v5 → v6.

### Fixed

- **Capture-subprocess stderr is drained to the log**, fixing a slow
  pipe-buffer stall that could silently stop `arecord`/`ffmpeg` audio while the
  process still appeared alive — and surfacing the subprocess's own errors for
  field debugging.
- **`BNB_BASE_URL` defaults to the server's own port** (`:8502`, was `:8080`)
  for RSS/iCal feeds and share links.
- **Documentation drift**: corrected the `/api/v2/health` response example, the
  `.env.example` image-tag note (analytics is built into *every* image, no
  separate tag), stale version pins, the feed-default port, and minor wording.
- **Release attestation no longer aborts the publish pipeline.** The SBOM
  summary step assumed the CycloneDX 1.5 `metadata.tools` object shape while
  cargo-cyclonedx emits the legacy array, so `jq` errored and its non-zero exit
  killed the `package` job before the SLSA build-provenance attestation and
  artifact upload could run. The summary now tolerates both shapes.

### Security

- **CORS is same-origin by default** — the API no longer emits a wildcard
  `Access-Control-Allow-Origin`, so a site you visit can't read the station
  over the LAN. Opt specific origins back in with `BIRDNET_CORS_ALLOWED_ORIGINS`.
- **5xx API responses no longer leak internal error strings** (DB/SQL detail);
  the detail is logged server-side and a generic message is returned.
- **HTTP Basic Auth (`CADDY_PWD`/`CADDY_USER`) is now read from the
  environment** as well as `birdnet.conf`, so it can be enabled under Docker;
  the server logs a prominent warning when bound to a non-loopback address with
  no password set.

## [0.5.0] - 2026-05-26

### Added

- **Dawn-chorus pattern matching — `GET /api/v2/analytics/patterns`.** The
  previously-stubbed endpoint is implemented on the behavioral extension's
  `sequence_match`, reporting per day whether a configured species sequence was
  detected in order (optionally within a maximum gap between consecutive steps).

### Changed

- **Bundled DuckDB upgraded 1.5.1 → 1.5.3** to match the published `behavioral`
  community extension (v0.6.0), which targets DuckDB 1.5.3. The bump is gated on
  the CDN actually serving a 1.5.3-built extension that `LOAD`s on the bundled
  engine — verified, not assumed from an HTTP 200.

### Fixed

- **Behavioral analytics were built against assumed extension signatures and had
  never executed** (the extension could not `LOAD`, and CI does not exercise the
  `analytics` feature), so every query was malformed against the real extension.
  All builders are corrected and now verified end-to-end against the published
  extension on DuckDB 1.5.3:
  - `sessionize` materialises the window-function session id in a subquery
    before aggregating (a window expression cannot appear in `GROUP BY`).
  - `retention` uses the real `retention(BOOLEAN, …) -> BOOLEAN[]` aggregate
    over per-species detection-day cohorts, replacing a non-existent
    `retention(date, int[])` form.
  - `window_funnel` passes step conditions as variadic booleans, not an array.
  - `sequence_next_node` uses the real
    `(direction, mode, timestamp, value, base_cond, …)` signature.

## [0.4.0] - 2026-05-25

### Added

- **`--refresh-extension`** — a maintenance command that force-reinstalls the
  latest `behavioral` DuckDB extension for the bundled DuckDB version, loads it
  to verify, and exits. Useful for recovering a corrupted extension cache.
  Requires `--analytics-db` (or `ANALYTICS_DB_PATH`) and network access.
- The bundled DuckDB version and the loaded `behavioral` extension version are
  logged at startup, so it is clear which analytics engine and extension build
  a station is running.

### Fixed

- **Behavioral analytics (sessionize, retention, funnel, next-species) failed to
  load.** DuckDB version-locks its extensions, but the bundled engine had
  drifted to DuckDB 1.5.3 while the published `behavioral` community extension
  targets 1.5.1, so `LOAD behavioral` was rejected and the extension-backed
  analytics were unavailable. The bundled DuckDB is now pinned to 1.5.1 to match
  the published extension.

### Changed

- Routine dependency and CI-action updates.

## [0.3.0] - 2026-05-24

### Added

- **Migration & phenology page (`/migration`).** A per-species ridgeline
  ("joyplot") of weekly abundance for migratory species, with first-of-year
  arrivals, peak diversity week, earliest-vs-last-year, and "still expected"
  tiles — built entirely from the existing `detections` table.
- **Dawn-chorus page (`/analytics/dawn-chorus`).** A 24-hour polar clock of
  per-species activity with sunrise/sunset markers from the station
  coordinates (`BNB_STATION_LAT`/`BNB_STATION_LON`, falling back to
  `BIRDNET_LATITUDE`/`BIRDNET_LONGITUDE`).
- **Detection detail + public share links.** Every detection links to a detail
  page (spectrogram, audio, daemon correlation id) and can be shared via a
  signed, public `/r/<token>` page — HMAC-SHA256 over `(date, time, com_name,
  expiry)`, constant-time verify, 30-day expiry, filename-based audio/
  spectrogram redirects. Set `BNB_SHARE_SECRET` so links survive restarts
  (fail-secure random per-process secret otherwise).
- **RSS & iCal feeds.** `/feeds/rare.rss`, `/feeds/rare.ics`, and
  `/feeds/today.rss`, linking back to detection detail pages; the rare RSS feed
  is advertised via `<link rel="alternate">` in the dashboard head. Absolute
  links use `BNB_BASE_URL`.
- **Per-device display preferences** on `/system` — theme, density, motion and
  contrast, applied before first paint (no flash on reload).
- **Comparative "today" phrase**, **species-detail hero/status partials**,
  **illustrated empty states** across six surfaces, and a **print stylesheet**
  for the reports.
- **Detection-review triage (`/detection-reviews`).** A non-destructive
  confirm/reject verdict per detection, stored in a new `detection_reviews`
  table (migration 13). The triage page queues recent unreviewed detections
  with Confirm/Reject actions and lists recent verdicts; each detection-detail
  page gains a self-replacing review widget. Distinct from quarantine, which
  gates uncertain rows *out* of the log before they are admitted.
- **Share from the quarantine queue.** Every quarantine row gets a "Share"
  button issuing the same signed `/r/<token>` link as detection detail; the
  share page now falls back to the quarantine table so a pending rare bird (not
  yet in `detections`) still resolves.
- **`uninstall.sh`** — a safe, idempotent, deterministic uninstaller shipped
  beside the binary (and as a standalone release asset). Removes only the
  software by default (systemd service, tmpfs mount unit, binary) and keeps the
  database, recordings, settings, and model unless you opt in via `--purge` or
  granular `--remove-db` / `--remove-recordings` / `--remove-config` /
  `--remove-models` / `--remove-image-cache` flags. Auto-detects the real data
  directory from the installed config/service, refuses to touch protected
  paths, supports `--dry-run` and `--yes`, and handles the macOS launchd
  LaunchAgent. The doctor also now flags missing ffmpeg when a macOS mic
  (avfoundation) or RTSP source is configured, and its config-path hint is
  platform-aware.
- **`install.sh` is now OS-aware.** On macOS it dispatches (before any root
  check or filesystem change) to a per-user launchd path — offering to
  `brew install` ffmpeg/cmake, downloading the `aarch64-apple-darwin` build when
  a release publishes one (else offering to build in place when run from a
  checkout, or printing the source-build steps), and writing
  a starter config + LaunchAgent — instead of failing partway through the
  Linux/systemd flow. Runs without `sudo` on macOS. Also hardened `SERVICE_USER`
  resolution so a missing `$USER` no longer aborts the script under `set -u`.
- **macOS Apple Silicon runbook + Homebrew formula draft** —
  `packaging/macos/verify-macos.sh` (from-source build, doctor, boot, mic
  enumeration, manual TCC/launchd checklist) and a template
  `packaging/macos/birdnet-behavior.rb` pending a hardware-verified release.

### Fixed

- **Startup crash from a duplicate route.** The new `/migration` page and the
  heatmap page both registered `GET /pages/migration-ridgeline`; axum's
  `Router::merge` panicked at construction, so the server never started. The
  heatmap embed moved to `/pages/seasonal-phenology`, and a lib-level test now
  builds the full router so an overlapping route fails CI (the standard test
  job runs `--lib --bins`, which skips the integration tests that would have
  caught it).
- **Print stylesheet 404.** `/static/css/print.css` was linked but never served
  by the static router; `@media print` output was unstyled and every page
  logged a console error.
- **Broken "Species Accumulation" card** on `/timeseries` (pointed at a
  non-existent `/pages/ts-accumulation`) — now uses `/pages/life-accumulation`.
- **Migration page request flood.** A `hx-trigger="… every 1h"` poll was
  parsed by htmx as 1 ms (it understands `s`/`m` but not `h`), hammering
  `/pages/migration-stats`; changed to `every 60m`.
- **Species photos never loaded** — the gallery card and the detection-detail
  link used image URLs that matched no route; pointed both at
  `/api/v2/species/image/{name}/file`.
- **Placeholder copy + missing skip link** on the public share page.
- **Four phone-width (390px) horizontal overflows** — `/history`,
  `/admin/audio`, `/admin/settings`, and `/onboarding` had inline multi-column
  grids the global responsive rules couldn't reach; they now collapse to a
  single column at ≤520px (and the onboarding stepper drops its text labels).
- **Misleading analytics status.** `/analytics` reported "behavioral analytics
  are active" whenever a DuckDB database was connected, even when the
  `duckdb-behavioral` extension failed to load; the badge now states the
  extension is a separate requirement (which the per-feature cards report on).
- **Duplicate species-photo caching.** Gallery and species-detail keyed photos
  by common name while detection-detail used the scientific name, so the same
  bird was fetched and stored twice (and detection-detail's link often 404'd);
  all three now key by scientific name, with a paced gallery background warmer.
- **Unlogged time-series 500s.** Failed `/api/v2/timeseries/*` queries returned
  500 with the error only in the body; the error is now logged server-side.

### CI

- The Tests job now runs the `tests/` integration suite (`cargo test
  --workspace --tests`), including a new `boot_smoke.rs` that spawns the binary
  in `--web-only` mode and curls `GET /` — closing the gap that let a startup
  panic ship despite green CI.

## [0.2.0] - 2026-05-23

### Security

- **Response-hardening headers on every response.** A new
  `birdnet-web::security` middleware layer sets `Content-Security-Policy`
  (own-origin scripts/styles/`connect-src`; no off-origin script, object, or
  framing), `X-Content-Type-Options: nosniff`, `X-Frame-Options: SAMEORIGIN`,
  and `Referrer-Policy: strict-origin-when-cross-origin`. No HSTS — the binary
  serves plain HTTP and expects a reverse proxy to own TLS.
- **Stateless CSRF protection.** State-changing requests (`POST`/`PUT`/`PATCH`/
  `DELETE`) whose `Origin`/`Referer` authority does not match the request
  `Host` are rejected with `403`. The web UI uses HTTP Basic Auth with no
  sessions, so a same-origin check (rather than a per-form synchroniser token)
  is the appropriate CSRF defence; non-browser clients (the CLI, scripts,
  `curl`) that send neither header are unaffected.

### Added

#### Pre-release hardening for 0.2.0 (release pipeline, docs, web)

- **Analytics built in everywhere, on by default.** Release binaries are built
  with `--features analytics` (one binary, no separate archive), and the
  **Docker image is now a single variant** with analytics compiled in — the
  separate `-analytics` tag is gone. `install.sh` runs the service with
  `--analytics-db` and `docker-compose.yml` sets `BIRDNET_ANALYTICS_DB`, so
  behavioral analytics works out of the box with no extra build, flag, or tag.
  Disable on very low-RAM boards by removing the flag / unsetting the env var.
- **Keyless cosign signatures on the Docker images.** The `docker.yml` merge
  job signs each multi-arch manifest with the workflow's GitHub OIDC identity
  (Fulcio + Rekor), matching the SLSA build-provenance attestation already on
  the binaries. Verification recipe in `RELEASING.md` and the job summary.
- **Rehearsable releases.** A `workflow_dispatch` dry run on `release.yml`
  runs validate → ci → build → package → attest without publishing, so a
  release — including the DuckDB analytics cross-build — can be proven green
  before a tag is pushed.
- **mdBook link checking in CI.** `docs.yml` now runs `mdbook-linkcheck`; a
  broken internal documentation link fails the build.
- **Reconnecting live-detection stream client.** A self-contained
  `/static/live-detections.js` consumes the existing `/api/v2/ws/detections`
  WebSocket, surfaces a live/offline indicator, dispatches `birdnet:detection`
  events, and reconnects with exponential backoff + jitter (capped at 30 s),
  dropping the socket while the tab is hidden. All DOM writes use `textContent`
  (never `innerHTML`).
- **Friendly `404` page.** Unmatched URLs now render the branded app layout
  with a route back to the dashboard, replacing the previous empty response.
- **In-UI configuration diagnostics** at `/admin/doctor` (linked from the admin
  nav as *Diagnostics*). Re-reads the active config and renders the same
  range/consistency findings the CLI `--doctor` reports, reusing the canonical
  `birdnet_core::config::validate` so the two can't drift; points to the CLI
  doctor for audio/model/disk/network checks.
- **CLI-help docs drift-gate.** `scripts/gen-cli-help.sh` regenerates
  `docs/book/_generated/cli-help.txt` from the binary's `--help`, and CI fails
  if the committed copy is stale — so the documented flags/env vars/defaults
  stay in lockstep with `src/cli.rs`.
- **Accessibility.** Added an `.sr-only` visually-hidden utility and live-status
  indicator styling (the existing reduced-motion / focus-visible / chart-ARIA
  coverage was already in place).
- **Supported hardware/OS matrix** added prominently to the README and the
  book, making the glibc 2.39 floor, the Bookworm→Docker path, and the
  no-armv7 caveat unmissable.
- **Upgrade-safe installer.** Re-running `install.sh` stops the service before
  swapping the binary (avoiding `ETXTBSY`) and restarts it on the new version;
  data and config are preserved and schema migrations run on startup. The
  installer also refuses to run on glibc < 2.39 with an actionable message.
- **`RELEASING.md` rewritten** to match the real pipeline (two build targets,
  native GCC cross — not `cargo-zigbuild`, SBOM, cosign, dry run) with a
  copy-paste pre-release checklist and a "what is not automated" section.

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
- **`rusqlite` 0.38 → 0.39** and **`rubato` 2.0 → 3.0** — the two
  out-of-range majors surfaced by the currency review, both verified
  drop-in with no source changes. `rusqlite` 0.39 pulls `libsqlite3-sys`
  0.36 → 0.37 and passes the full `birdnet-db` / `birdnet-migrate` /
  `birdnet-web` suites (and the analytics-gated `birdnet-behavioral`
  connection path); `rubato` 3.0 leaves its `audioadapter` pin unchanged
  and passes the `birdnet-core` lib + `audio_pipeline` integration
  tests. With these, every direct dependency is at its latest release as
  of 2026-05-22.
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

[Unreleased]: https://github.com/tomtom215/BirdNet-Behavior/compare/v0.14.0...HEAD
[0.14.0]: https://github.com/tomtom215/BirdNet-Behavior/compare/v0.13.1...v0.14.0
[0.13.1]: https://github.com/tomtom215/BirdNet-Behavior/compare/v0.13.0...v0.13.1
[0.13.0]: https://github.com/tomtom215/BirdNet-Behavior/compare/v0.12.0...v0.13.0
[0.12.0]: https://github.com/tomtom215/BirdNet-Behavior/compare/v0.11.0...v0.12.0
[0.11.0]: https://github.com/tomtom215/BirdNet-Behavior/compare/v0.10.0...v0.11.0
[0.10.0]: https://github.com/tomtom215/BirdNet-Behavior/compare/v0.9.0...v0.10.0
[0.9.0]: https://github.com/tomtom215/BirdNet-Behavior/compare/v0.7.2...v0.9.0
[0.7.0]: https://github.com/tomtom215/BirdNet-Behavior/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/tomtom215/BirdNet-Behavior/compare/v0.5.3...v0.6.0
[0.5.3]: https://github.com/tomtom215/BirdNet-Behavior/compare/v0.5.2...v0.5.3
[0.5.2]: https://github.com/tomtom215/BirdNet-Behavior/compare/v0.5.1...v0.5.2
[0.5.1]: https://github.com/tomtom215/BirdNet-Behavior/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/tomtom215/BirdNet-Behavior/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/tomtom215/BirdNet-Behavior/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/tomtom215/BirdNet-Behavior/releases/tag/v0.3.0
[0.2.0]: https://github.com/tomtom215/BirdNet-Behavior/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/tomtom215/BirdNet-Behavior/releases/tag/v0.1.0
