# Implementation Plan: 100% BirdNET-Pi Feature Parity

**Date**: 2026-03-14
**Current Parity**: ~78% (verified against source)
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

### 2.5 🔧 Language/i18n CLI + Web Integration
**Priority**: P1 — important for international users
**Files to modify**:
- `src/cli.rs` — add `--lang` flag (default "en")
- `src/main.rs` — load LanguagePack at startup, store in AppState
- `crates/birdnet-web/src/state.rs` — add `language_pack: Option<LanguagePack>`
- `crates/birdnet-web/src/routes/pages/` — translate common names in HTML

**Implementation**:
```
1. Add --lang / BIRDNET_LANG env var to CLI
2. At startup, load LanguagePack if lang != "en" and labels_dir exists
3. Store Option<LanguagePack> in AppState
4. In all page rendering: translate sci_name → localized com_name via pack.translate()
5. Add language selector in admin settings
```

**Acceptance**: Setting `--lang de` displays German common names throughout UI.

---

## Sprint 3: Export & Streaming (Effort: Medium)

### 3.1 ❌ eBird CSV Export
**Priority**: P1 — citizen science community
**Files to create**:
- `crates/birdnet-web/src/routes/export.rs` (modify existing) — add eBird format

**eBird CSV format** (from `ebird.php`):
```
Common Name, Species, Number, Breeding Status, Comments, Location, Latitude, Longitude,
Date, Start Time, State/Province, Country, Protocol, Num Observers, Duration (Min),
All Obs Reported, Distance Traveled (km), Area Covered (ha), Checklist ID, Species Comments
```

**Implementation**:
```rust
// Add to export.rs:
GET /detections/export?format=ebird&date=YYYY-MM-DD

fn build_ebird_row(detection: &Detection, config: &LocationConfig) -> String {
    // One row per detection, grouped by common name if desired
    // BirdNET-Pi uses "1" for count, "S" for protocol
}
```

**DB query needed**: `detections_for_date_ebird(date: &str) -> Vec<Detection>` (existing query works)

**Acceptance**: Export link produces valid eBird-importable CSV.

---

### 3.2 🔧 Live Audio Stream Backend
**Priority**: P1 — page exists, stream not wired
**Current state**: `/live` page has `<audio src="/stream">` but `/stream` route may not produce audio.

**Files to modify**:
- `crates/birdnet-web/src/routes/livestream.rs` — verify ffmpeg subprocess startup
- `src/main.rs` — start stream subprocess at init

**Implementation**:
```
The /stream route should:
1. Start ffmpeg subprocess: `ffmpeg -f alsa -i default -acodec mp3 -ab 320k -f mp3 -`
2. Pipe stdout to HTTP chunked response
3. Handle RTSP source: `ffmpeg -i rtsp://... -acodec mp3 ...`
4. Restart on failure (reuse CaptureManager pattern)

Route: GET /stream → application/octet-stream with mp3 chunks
```

**Acceptance**: Navigating to `/live` plays real-time audio from mic/RTSP.

---

### 3.3 ❌ Audio Format Conversion (MP3/FLAC/OGG)
**Priority**: P1 — extraction currently WAV only
**Files to modify**:
- `crates/birdnet-core/src/audio/extraction.rs` — add sox/ffmpeg subprocess for format conversion

**Implementation**:
```rust
// Add to ExtractionConfig:
pub audio_format: AudioFormat, // WAV | Mp3 | Flac | Ogg

// Post-extraction conversion:
fn convert_to_format(wav_path: &Path, target: AudioFormat) -> Result<PathBuf, ExtractionError> {
    let output = wav_path.with_extension(target.extension());
    Command::new("sox")
        .arg(wav_path)
        .arg(&output)
        .status()?;
    fs::remove_file(wav_path)?;
    Ok(output)
}
```

**Acceptance**: `--audio-format mp3` produces MP3 extraction files.

---

## Sprint 4: Notification & Image Enhancements (Effort: Low)

### 4.1 ❌ Per-Species Cooldown (not just global)
**Priority**: P1 — notification relevance
**Files to modify**:
- `crates/birdnet-integrations/src/apprise.rs` — change `HashMap<(), Duration>` to `HashMap<String, Instant>`

**Implementation** (small change):
```rust
// Change CooldownTracker to per-species:
pub struct CooldownTracker {
    last_sent: HashMap<String, Instant>, // key: sci_name
    global_cooldown: Duration,
    per_species_cooldown: HashMap<String, Duration>, // configurable
}
```

**Acceptance**: Config `COOLDOWN_TURDUS_MERULA=300` prevents robin spam while allowing eagle notifications.

---

### 4.2 ❌ New/Rare Species Highlighting in Dashboard
**Priority**: P1 — discovery excitement
**Files to modify**:
- `crates/birdnet-web/src/routes/pages/dashboard.rs` — add badge CSS classes
- `crates/birdnet-db/src/sqlite/queries/` — add first_seen_date query

**Implementation**:
```
1. For each detection in dashboard: check if first ever seen (query min(Date) for sci_name)
2. If first ever: add "NEW" badge (CSS: green pill)
3. If first seen > RARE_THRESHOLD days ago (configurable, default 30): add "RARE" badge
4. Use HTMX hx-boost to avoid full reload on badge click
```

**Acceptance**: New species get green "NEW" badge; rare species get "RARE" badge; badges respect configurable threshold.

---

### 4.3 ❌ Image in Apprise Notifications
**Priority**: P1 — rich notifications
**Files to modify**:
- `crates/birdnet-integrations/src/apprise.rs` — add `image_url` field to notification payload
- `src/daemon.rs` — resolve image URL from cache before notifying

**Implementation**:
```
If image cache has entry for sci_name:
  notify_ctx.image_url = Some(format!("{station_url}/images/{filename}"))
Include image_url in Apprise notification JSON: "image": "<url>"
```

**Acceptance**: Telegram/Discord notifications include species photo.

---

### 4.4 ❌ eBird / AllAboutBirds Species Links
**Priority**: P1 — education/engagement
**Files to modify**:
- `crates/birdnet-web/src/routes/pages/species_pages.rs` — add external link
- `crates/birdnet-db/src/settings.rs` — add `info_site` setting (ebird | allaboutbirds | none)

**Implementation** (trivial):
```html
<!-- In species detail page -->
<a href="https://ebird.org/species/{ebird_code}" target="_blank">eBird</a>
<a href="https://allaboutbirds.org/guide/{common_name}" target="_blank">AllAboutBirds</a>
```

**Acceptance**: Species pages link to eBird and/or AllAboutBirds based on setting.

---

## Sprint 5: Admin & System (Effort: Medium)

### 5.1 ❌ System Controls (clear data, restart)
**Priority**: P0 — admin completeness
**Files to create**:
- `crates/birdnet-web/src/routes/admin/system_controls.rs` (new)

**Implementation**:
```
POST /admin/system/clear-detections → DELETE FROM detections; DELETE from notification_log;
POST /admin/system/clear-images → rm -rf image_cache_dir/*
POST /admin/system/clear-extracted → rm -rf extracted_dir/*
  (all with HTMX confirmation modal)

Note: No reboot/shutdown — single binary doesn't need that; admin just restarts the process
```

**Acceptance**: Admin panel has "Danger Zone" section with confirmation-gated data clearing.

---

### 5.2 ❌ Full Backup (config + audio + DB)
**Priority**: P1 — data safety
**Files to modify**:
- `crates/birdnet-web/src/routes/admin/backup.rs` — extend to tar archive

**Implementation**:
```
GET /admin/backup/full → streams tar.gz containing:
  - birds.db (SQLite backup)
  - birdnet.conf (config file)
  - BirdSongs/Extracted/ (extraction clips)
  - image cache directory

Use tokio::process::Command::new("tar") with --transform to flatten paths
Stream directly to response as application/gzip
```

**Acceptance**: Download button produces valid .tar.gz that can be restored.

---

### 5.3 ❌ Custom Site Name
**Priority**: P1 — branding
**Files to modify** (trivial):
- `crates/birdnet-db/src/settings.rs` — add `site.name` key
- `crates/birdnet-web/src/routes/pages/mod.rs` — include in layout title
- Admin settings render — add site name input

**Acceptance**: Setting site name changes page `<title>` and header.

---

## Sprint 6: Advanced Detection (Effort: High, Leapfrog)

### 6.1 ❌ Per-Species Confidence Thresholds
**Priority**: P1 — #1 community request not in BirdNET-Pi
**Files to create/modify**:
- `crates/birdnet-db/src/sqlite/` — add `species_thresholds` table
- `crates/birdnet-core/src/inference/model.rs` — accept per-species override map
- `crates/birdnet-web/src/routes/admin/species/` — threshold editor UI

**Schema**:
```sql
CREATE TABLE species_thresholds (
    sci_name TEXT PRIMARY KEY,
    confidence_threshold REAL NOT NULL,
    created_at TEXT NOT NULL
);
```

**Implementation**:
```rust
// In detection pipeline, after inference:
let threshold = species_thresholds.get(&detection.sci_name)
    .copied()
    .unwrap_or(global_threshold);
if detection.confidence >= threshold { ... }
```

**Acceptance**: Robin can be set to 80% threshold while rare warbler defaults to global 25%.

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

### 7.1 🔧 Spectrogram Text Overlay
**Priority**: P1
**Files to modify**:
- `crates/birdnet-core/src/audio/spectrogram.rs` — add text rendering via tiny_skia or image crate

**Current state**: Generates raw mel spectrogram PNG without labels.

**Implementation**:
```
Add SpectrogramConfig::label: Option<SpectrogramLabel> {
    species_name: String,
    confidence: f32,
    timestamp: String,
}

Render using imageproc or embed font bytes + blit text
```

**Acceptance**: Spectrogram PNG shows species name, confidence %, and timestamp as overlay.

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
- [x] 2.1 Dark mode ✅ (already implemented in templates/layout.html)
- [x] 2.2 Weekly report web page ✅ (pages/weekly_report.rs)
- [x] 2.3 Daily charts date navigation ✅ (pages/history.rs)
- [x] 2.4 Expose overlap config ✅ (src/cli.rs + src/daemon.rs)
- [ ] 3.2 Live audio stream wiring
- [ ] 5.1 System controls (clear data)

### P1 Important (Ship before competitive comparison)
- [ ] 2.5 Language/i18n wiring
- [ ] 3.1 eBird CSV export
- [ ] 3.3 Audio format conversion
- [ ] 4.1 Per-species cooldown
- [ ] 4.2 New/rare species highlighting
- [ ] 4.3 Image in notifications
- [ ] 4.4 eBird/AllAboutBirds links
- [ ] 5.2 Full backup (config + audio + DB)
- [ ] 5.3 Custom site name
- [ ] 6.1 Per-species confidence thresholds ← leapfrog feature
- [ ] 6.3 Multiple RTSP streams
- [ ] 7.1 Spectrogram text overlay
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
| 2 | UI gaps (2.1–2.5) | ~3 days | P0-P1 |
| 3 | Export + streaming (3.1–3.3) | ~2 days | P1 |
| 4 | Notifications + images (4.1–4.4) | ~1 day | P1 |
| 5 | Admin + system (5.1–5.3) | ~2 days | P0-P1 |
| 6 | Advanced detection (6.1–6.3) | ~3 days | P1 |
| 7 | Spectrogram + recording UX (7.1–7.2) | ~2 days | P1 |
| **Total** | **~13 items remaining for 100%** | **~13 dev-days** | |

---

*This is a living document. Update sprint status as items are completed.
Verified against source on 2026-03-14. Branch: `claude/birdnet-pi-feature-parity-Clzoi`.*
