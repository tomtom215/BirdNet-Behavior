# Implementation Plan: 100% BirdNET-Pi Feature Parity

**Date**: 2026-03-14
**Current Parity**: ~91% (verified against source)
**Goal**: 100% verified feature parity + leapfrog capabilities

This is the **working document** for reaching 100% parity. Each task includes
exact file paths, implementation approach, and acceptance criteria. Items are
ordered by impact-to-effort ratio.

---

## Status Conventions

| Symbol | Meaning |
|--------|---------|
| ✅ | Fully implemented and verified |
| 🔧 | Partially implemented — needs completion |
| ❌ | Not yet started |

---

## Sprint 1: Low-Hanging Fruit (Effort: Low, Impact: High)

### 1.1 ✅ Species Occurrence Frequency Filter
**Files**: `crates/birdnet-core/src/inference/species_filter.rs` (392 LOC), `src/daemon.rs`
**Status**: COMPLETE — full implementation with metadata ONNX model, caching, whitelist/include/exclude lists, wired in daemon.

### 1.2 ✅ Detection Audio Extraction
**Files**: `crates/birdnet-core/src/audio/extraction.rs` (474 LOC)
**Status**: COMPLETE — saves to `Extracted/By_Date/YYYY-MM-DD/Species_Name/`. Uses symphonia decode + hound WAV write. Context padding with BirdNET-Pi formula.

### 1.3 ✅ Privacy Threshold (Human Voice Filter)
**Files**: `crates/birdnet-core/src/detection/privacy.rs` (254 LOC)
**Status**: COMPLETE — cutoff rank formula, adjacent chunk masking, wired in daemon config.

### 1.4 ✅ Today's Detections Page
**Files**: `crates/birdnet-web/src/routes/pages/today.rs` (290 LOC)
**Status**: COMPLETE — search with NOT prefix, pagination (40/page), delete, re-label (DB query + API endpoint).

### 1.5 ✅ Disk Auto-Purge
**Files**: `crates/birdnet-core/src/audio/capture/disk.rs` (732 LOC)
**Status**: COMPLETE — background monitor, configurable threshold, purge/keep modes.

### 1.6 ✅ Scheduler Integration
**Files**: `src/capture.rs`
**Status**: COMPLETE — `birdnet-scheduler` crate wired into `CaptureManager` via `ScheduleConfig` + `Location`.

### 1.7 ✅ Heartbeat URL
**Files**: `crates/birdnet-integrations/src/heartbeat.rs` (116 LOC), `src/daemon.rs`
**Status**: COMPLETE — GET ping after each detection processed, wired in event_processor.

### 1.8 ✅ Notification Templates
**Files**: `crates/birdnet-integrations/src/notification.rs`
**Status**: COMPLETE — `NotificationTemplate::render()` with full $variable substitution ($sciname, $comname, $confidence, $confidencepct, $date, $time, $week, $latitude, $longitude, $listenurl, $image, etc.)

### 1.9 ✅ New Species Notification Triggers
**Files**: `crates/birdnet-integrations/src/notification.rs`
**Status**: COMPLETE — `TriggerMode::NewSpecies`, `TriggerMode::NewSpeciesDaily` implemented.

### 1.10 ✅ i18n / 36 Language Support Framework
**Files**: `crates/birdnet-core/src/i18n.rs` (497 LOC)
**Status**: COMPLETE (framework) — `LanguagePack::load()` with 36 language code validation, sci_name → common_name translation. PARTIAL wiring (needs CLI flag + web integration).

---

## Sprint 2: Critical UI Gaps (Effort: Low-Medium, Impact: High)

### 2.1 ✅ Dark Mode
**Status**: COMPLETE — fully verified in `templates/layout.html`

CSS custom properties for dark (`#0f172a` background) and light (`#f8fafc` background) themes.
Toggle button in nav with ☾/☀ icons. `localStorage` persistence. Respects `prefers-color-scheme`
media query on first load. Both themes fully cover all color variables used throughout the UI.

---

### 2.2 ✅ Weekly Report Web Page
**Status**: COMPLETE — `crates/birdnet-web/src/routes/pages/weekly_report.rs`

GET `/weekly` — full HTMX page with week navigation (prev/next), summary stats (total detections,
species count, new species count), 7-bar SVG chart, top-10 species ranked list with bar
visualizations, and new-species list with "NEW" badge. Week navigation uses HTMX partial swaps.
DB queries added: `weekly_top_species`, `weekly_new_species`, `weekly_detection_count`,
`range_daily_counts`.

### PREVIOUSLY 2.2 ❌ Weekly Report Web Page
**Priority**: P0 — popular engagement feature
**Files to create/modify**:
- `crates/birdnet-web/src/routes/pages/weekly_report.rs` (new)
- `crates/birdnet-web/src/routes/pages/mod.rs` — register route
- Wire `birdnet_integrations::weekly_report::WeeklyReportGenerator`

**Implementation**:
```
GET /weekly — renders WeeklyReport as HTMX page

WeeklyReport content:
  - Top 10 species by detection count this week
  - New species this week (first ever detected)
  - Trend vs prior week (% change)
  - Most active hour this week
  - Total detections count
  - Day with most detections

Page structure:
  - Full-page HTMX render using existing weekly_report.rs generator
  - SVG bar chart (reuse charts.rs helpers)
  - HTMX partial for week navigation (prev/next)
```

**DB queries needed** (add to `birdnet-db/sqlite/queries/analytics.rs`):
- `weekly_top_species(week_start: &str, limit: usize) -> Vec<(String, i64)>`
- `weekly_new_species(week_start: &str) -> Vec<String>`
- `weekly_detection_count(week_start: &str) -> i64`

**Acceptance**: `/weekly` page shows top species, new species, total count; week navigation works.

---

### 2.3 ✅ Daily Charts with Date Navigation (Detection History Page)
**Status**: COMPLETE — `crates/birdnet-web/src/routes/pages/history.rs`

GET `/history` — date-browser page with two-column layout: sidebar listing recent dates (last 90),
chart area showing hourly detection SVG for selected date. HTMX-based prev/next navigation buttons.
Date-specific stats (total detections + species count). DB queries added: `detection_count_for_date`,
`distinct_detection_dates`. Navigation bar updated to include History and Weekly links.

### PREVIOUSLY 2.3 🔧 Daily Charts with Date Navigation
**Priority**: P0 — users check historical charts daily
**Files to modify**:
- `crates/birdnet-web/src/routes/pages/charts.rs` — add date parameter + navigation
- `crates/birdnet-db/src/sqlite/queries/analytics.rs` — date-specific hourly query

**Current state**: `charts.rs` renders hourly SVG for today only.

**Implementation**:
```
1. Add `date` query param: GET /charts?date=2024-06-15
2. Default to today if not specified
3. Add prev/next day buttons (HTMX swap-oob for navigation buttons)
4. Add date picker input (HTMX get on change)
5. Render hourly bar chart for chosen date from DB

HTMX endpoints:
  GET /charts → full page (today)
  GET /pages/charts-hourly?date=YYYY-MM-DD → hourly SVG partial
  GET /pages/charts-daily?month=YYYY-MM → monthly overview partial
```

**Acceptance**: Date picker works, prev/next navigates days, chart updates via HTMX.

---

### 2.4 ✅ Expose Overlap Config in CLI
**Status**: COMPLETE — `src/cli.rs` + `src/daemon.rs`

Added `--overlap` / `BIRDNET_OVERLAP` env var (0.0–2.9s, default 0.0) wired into
`PipelineConfig::chunk_overlap_secs`. Falls back to `OVERLAP` in config file.

### PREVIOUSLY 2.4 🔧 Expose Overlap Config in CLI
**Priority**: P0 — affects detection sensitivity
**Files to modify**:
- `src/cli.rs` — add `--overlap` flag
- `src/daemon.rs` — wire into `PipelineConfig::chunk_overlap_secs`

**Implementation** (5 min change):
```rust
// src/cli.rs
#[arg(long, env = "BIRDNET_OVERLAP", default_value = "0.0")]
pub overlap: f32,
```

```rust
// src/daemon.rs in PipelineConfig setup:
pipeline: birdnet_core::detection::pipeline::PipelineConfig {
    watch_dir,
    chunk_overlap_secs: cli.overlap,
    ..Default::default()
},
```

**Acceptance**: `--overlap 1.5` sets 1.5s overlap between analysis windows.

---

### 2.5 ✅ Language/i18n CLI + Web Integration
**Status**: COMPLETE — `src/cli.rs`, `src/main.rs`, `crates/birdnet-web/src/state.rs`

Added `--lang` / `BIRDNET_LANG` (default "en") and `--labels-dir` / `BIRDNET_LABELS_DIR` CLI flags.
At startup, `init_i18n()` loads `I18nManager` with the configured language pack.
`AppState` stores `Option<RwLock<I18nManager>>` via `with_i18n()` builder, accessible via `with_i18n_ref()`.

---

## Sprint 3: Export & Streaming (Effort: Medium)

### 3.1 ✅ eBird CSV Export
**Status**: COMPLETE — `crates/birdnet-web/src/routes/export.rs`

Added `GET /detections/export/ebird?date=YYYY-MM-DD&lat=&lon=&location=` endpoint.
Groups detections by species+date, converts dates to MM/DD/YYYY format for eBird import,
includes BirdNET avg confidence in comments. Full eBird Record Format CSV with all required fields.

---

### 3.2 ✅ Live Audio Stream Backend
**Status**: COMPLETE — `src/main.rs` (`init_audio_source()`), `crates/birdnet-web/src/state.rs`

Audio source (ALSA device or RTSP URL) is now wired from CLI/config into `AppState` via
`with_audio_source()` builder. The `/stream` route in `livestream.rs` uses the audio source
to spawn ffmpeg subprocess on demand. `init_audio_source()` prefers RTSP URL, then ALSA device,
then config values.

---

### 3.3 ✅ Audio Format Conversion (MP3/FLAC/OGG)
**Status**: COMPLETE — `crates/birdnet-core/src/audio/extraction.rs`, `src/cli.rs`

Added `AudioFormat` enum (Wav/Mp3/Flac/Ogg) with `target_format` field in `ExtractionConfig`.
Post-extraction conversion via ffmpeg (preferred) or sox (fallback). CLI flag `--audio-format` /
`BIRDNET_AUDIO_FORMAT` env var. Falls back to WAV if conversion tools unavailable.

---

## Sprint 4: Notification & Image Enhancements (Effort: Low)

### 4.1 ✅ Per-Species Cooldown
**Status**: COMPLETE — `crates/birdnet-integrations/src/apprise.rs`

`NotifyConfig` now has `per_species_cooldown: HashMap<String, Duration>` for species-specific
cooldown overrides. `should_notify()` checks per-species override before falling back to global cooldown.
Initialized from config in `src/integrations.rs`.

---

### 4.2 ✅ New/Rare Species Highlighting in Dashboard
**Status**: COMPLETE — `crates/birdnet-web/src/routes/pages/dashboard.rs`, `crates/birdnet-db/src/sqlite/queries/species.rs`

Added `species_first_seen()` query (MIN(Date) GROUP BY Sci_Name). Dashboard `detections_partial()`
shows green "NEW" badge for species first seen today, cyan "RARE" badge for species first seen on
the detection date (historical new). Badge styling uses inline CSS pill elements.

---

### 4.3 ✅ Image in Apprise Notifications
**Status**: COMPLETE — `crates/birdnet-integrations/src/apprise.rs`

Added `send_notification_with_image()` method with optional `image_url` parameter.
When provided, includes `"image": "<url>"` in the Apprise JSON payload for rich notifications
on Telegram, Discord, etc.

---

### 4.4 ✅ eBird / AllAboutBirds Species Links
**Status**: COMPLETE — `crates/birdnet-web/src/routes/pages/species_pages.rs`, `crates/birdnet-web/src/state.rs`

Species info partial appends eBird or AllAboutBirds links based on `state.info_site()`.
Configured via `--info-site` / `BIRDNET_INFO_SITE` CLI flag (values: "ebird", "allaboutbirds", "none").
`AppState` stores `info_site: String` with `with_info_site()` builder and `info_site()` accessor.

---

## Sprint 5: Admin & System (Effort: Medium)

### 5.1 ✅ System Controls (clear data, backup)
**Status**: COMPLETE — `crates/birdnet-web/src/routes/admin/system_controls.rs` (new file)

- `POST /admin/system/clear-detections` — Deletes all detections and notification_log
- `POST /admin/system/clear-extracted` — Removes all files/dirs from recordings directory
- `GET /admin/system/backup/full` — Creates tar.gz of DB + config + recordings, streams as download
Admin system page has "Danger Zone" section with confirmation-gated buttons.

---

### 5.2 ✅ Full Backup (config + audio + DB)
**Status**: COMPLETE — integrated into `system_controls.rs`

`GET /admin/system/backup/full` creates tar.gz containing DB + config + recordings directory.
Uses `tokio::process::Command::new("tar")` with streaming response. 5-minute cleanup timer
for temporary archive file. Download button in admin system page.

---

### 5.3 ✅ Custom Site Name
**Status**: COMPLETE — `src/cli.rs`, `src/main.rs`, `crates/birdnet-web/src/state.rs`

Added `--site-name` / `BIRDNET_SITENAME` CLI flag. `AppState` stores `site_name: Option<String>`
with `with_site_name()` builder and `site_name()` accessor (defaults to "BirdNet-Behavior").
Initialized from CLI or config `SITENAME` key via `init_site_name()` in main.

---

## Sprint 6: Advanced Detection (Effort: High, Leapfrog)

### 6.1 ✅ Per-Species Confidence Thresholds
**Status**: COMPLETE — `crates/birdnet-db/` (migration v6 + queries), `crates/birdnet-web/src/routes/admin/species/`, `src/daemon.rs`

Added `species_thresholds` table (migration v6) with CRUD queries. Admin UI with HTMX lazy-loading
for threshold management. Daemon loads thresholds at startup and filters detections per-species
before applying global threshold fallback.

---

### 6.2 ❌ Rare Bird Quarantine
**Priority**: P1 — novel feature (BirdNET-Pi doesn't have this)
**Concept**: Instead of silently discarding species filtered by SF_THRESH, quarantine low-confidence/low-frequency detections for manual review.

**Files to create**:
- `crates/birdnet-db/src/sqlite/` — add `quarantine` table
- `crates/birdnet-web/src/routes/pages/quarantine.rs` — review page

**Schema**:
```sql
CREATE TABLE quarantine (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    date TEXT NOT NULL,
    time TEXT NOT NULL,
    sci_name TEXT NOT NULL,
    com_name TEXT NOT NULL,
    confidence REAL NOT NULL,
    sf_probability REAL,  -- metadata model output
    reason TEXT NOT NULL, -- "below_sf_thresh" | "low_confidence" | "manual"
    reviewed INTEGER NOT NULL DEFAULT 0,
    file_name TEXT
);
```

**Acceptance**: Filtered species appear in quarantine page; user can approve or discard.

---

### 6.3 ❌ Multiple RTSP Streams
**Priority**: P1 — many users have multi-mic setups (GH#459, #177)
**Files to modify**:
- `src/cli.rs` — change `rtsp_url: Option<String>` to `rtsp_urls: Vec<String>`
- `crates/birdnet-core/src/audio/capture/manager.rs` — spawn one CaptureManager per RTSP URL
- Detection filename prefix: `RTSP_1-`, `RTSP_2-`, etc.

**Implementation**:
```rust
// src/cli.rs:
#[arg(long, env = "BIRDNET_RTSP_URLS", value_delimiter = ',')]
pub rtsp_urls: Vec<String>,

// Each URL gets its own CaptureManager with independent lifecycle
// Each writes to separate subdirectory or prefixes filenames
```

**Acceptance**: `--rtsp-urls rtsp://cam1,rtsp://cam2` runs two independent capture pipelines.

---

## Sprint 7: Spectrogram & Audio UX (Effort: Medium)

### 7.1 ✅ Spectrogram Text Overlay
**Status**: COMPLETE — `crates/birdnet-web/src/routes/spectrogram.rs`

Added `SpectrogramLabel` struct and `encode_spectrogram_png_labeled()` with minimal 5x7 bitmap font
renderer (no external dependencies). Species name, confidence %, and timestamp rendered as white text
on darkened background strip. Activated via `?species=&confidence=&time=` query params on spectrogram
endpoint.

---

### 7.2 ❌ Recording Browser: Browse by Date/Species Navigation
**Priority**: P1
**Files to modify**:
- `crates/birdnet-web/src/routes/pages/recordings.rs` — add date/species browse modes

**Current state**: Basic recording list, no structured navigation.

**Implementation**:
```
GET /recordings?view=by_date&date=YYYY-MM-DD → recordings for date
GET /recordings?view=by_species&species=Turdus+merula → recordings for species
GET /recordings?view=calendar → monthly calendar with detection dots
```

**Acceptance**: Three browse modes; calendar view shows days with detections highlighted.

---

## Completion Checklist

### P0 Critical (Block on 1.0 release)
- [x] 2.1 Dark mode ✅
- [x] 2.2 Weekly report web page ✅
- [x] 2.3 Daily charts date navigation ✅
- [x] 2.4 Expose overlap config ✅
- [x] 3.2 Live audio stream wiring ✅
- [x] 5.1 System controls (clear data) ✅

### P1 Important (Ship before competitive comparison)
- [x] 2.5 Language/i18n wiring ✅
- [x] 3.1 eBird CSV export ✅
- [x] 3.3 Audio format conversion ✅
- [x] 4.1 Per-species cooldown ✅
- [x] 4.2 New/rare species highlighting ✅
- [x] 4.3 Image in notifications ✅
- [x] 4.4 eBird/AllAboutBirds links ✅
- [x] 5.2 Full backup (config + audio + DB) ✅
- [x] 5.3 Custom site name ✅
- [x] 6.1 Per-species confidence thresholds ✅ ← leapfrog feature
- [ ] 6.3 Multiple RTSP streams
- [x] 7.1 Spectrogram text overlay ✅
- [ ] 7.2 Recording browser date/species nav

### P2 Defer (Post 1.0)
- [ ] 6.2 Rare bird quarantine ← novel leapfrog
- [ ] 4.1 Frequency shifting
- [ ] Lock/unlock recordings
- [ ] Per-species file limits
- [ ] tmpfs for transient audio
- [ ] mDNS discovery
- [ ] Auto-update mechanism
- [ ] Installation script
- [ ] BirdDB.txt flat file export
- [ ] Image blacklisting
- [ ] ZRAM setup script
- [ ] Perch model support

---

## Estimated Sprint Effort

| Sprint | Items | Effort | Priority |
|--------|-------|--------|----------|
| 1 | Already done (1.1–1.10) | 0 | P0 ✅ |
| 2 | UI gaps (2.1–2.5) | ✅ COMPLETE | P0-P1 ✅ |
| 3 | Export + streaming (3.1–3.3) | ✅ COMPLETE | P1 ✅ |
| 4 | Notifications + images (4.1–4.4) | ✅ COMPLETE | P1 ✅ |
| 5 | Admin + system (5.1–5.3) | ✅ COMPLETE | P0-P1 ✅ |
| 6 | Advanced detection (6.1 done, 6.3 remaining) | ~1 day | P1 |
| 7 | Spectrogram + recording UX (7.1 done, 7.2 remaining) | ~1 day | P1 |
| **Total** | **~2 items remaining for P1 parity** | **~2 dev-days** | |

---

*This is a living document. Update sprint status as items are completed.
Verified against source on 2026-03-14. Branch: `claude/birdnet-pi-feature-parity-0Npie`.*
