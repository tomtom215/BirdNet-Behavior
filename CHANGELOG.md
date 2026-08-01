# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

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

- **Locking a recording now protects it immediately.** The purge read the locked
  set once at startup and ran on that snapshot for the lifetime of the process,
  so a clip locked from `/admin/recordings` was unprotected until the next
  restart, with nothing saying so. The set is re-read on every purge cycle. The
  per-species cap ignored locks entirely — setting `MAX_FILES_SPECIES` deleted
  the very recordings a researcher had marked to keep — and now excludes them,
  along with any clip another in-cap detection still references.

- **Pruned clips no longer leave a dead play button.** The per-species cap
  deleted the audio but left `File_Name` set, so the clips browser kept offering
  playback for a file that no longer existed, and the daily query re-selected
  every already-pruned row forever. The reference is cleared with the file; the
  detection row is preserved for stats.

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

[Unreleased]: https://github.com/tomtom215/BirdNet-Behavior/compare/v0.9.0...HEAD
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
