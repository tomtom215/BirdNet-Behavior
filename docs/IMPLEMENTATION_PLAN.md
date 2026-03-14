# Implementation Plan: Feature Parity & Leapfrog

**Date**: 2026-03-14
**Goal**: Surpass 100% BirdNET-Pi feature parity

## Phase 1: Critical Path (P0)

### 1.1 Species Occurrence Frequency Filter
**Files**: `birdnet-core/src/inference/species_filter.rs` (new), `birdnet-core/src/detection/daemon.rs`
- Load metadata model (ONNX) that maps (lat, lon, week) → species probability vector
- Apply SF_THRESH threshold to filter unlikely species
- Whitelist support: species that bypass the threshold
- Integrate into detection daemon between inference and reporting
- Config: `SF_THRESH`, `LATITUDE`, `LONGITUDE`, `METADATA_MODEL_PATH`

### 1.2 Detection Audio Extraction
**Files**: `birdnet-core/src/audio/extraction.rs` (new)
- Extract audio clips around detections using symphonia + hound
- Configurable extraction length with context padding (BirdNET-Pi formula: `(EXTRACTION_LENGTH - 3) / 2` spacer)
- Save to `Extracted/By_Date/YYYY-MM-DD/Species_Name/` directory structure
- Generate per-detection spectrogram PNG
- Support multiple output formats via sox subprocess (WAV, MP3, FLAC, OGG)
- Wire into daemon event_processor

### 1.3 Disk Management with Auto-Purge
**Files**: `birdnet-core/src/audio/capture/disk.rs` (extend), new `disk_manager.rs`
- Background disk monitor thread (checks every 60s)
- Configurable purge threshold (default 95%)
- Two modes: `purge` (delete oldest) or `keep` (stop recording)
- Per-species file count limits (`MAX_FILES_SPECIES`)
- Exclude list for protected species recordings
- Lock/unlock individual recordings (purge protection)
- Wire into main.rs as background task

### 1.4 Privacy Threshold (Human Voice Filter)
**Files**: `birdnet-core/src/detection/privacy.rs` (new)
- After inference, scan all predictions for "Human" class
- If Human confidence > threshold, mask that chunk AND adjacent chunks
- Configurable threshold (0-3%, 0 = disabled)
- BirdNET-Pi algorithm: `human_cutoff = max(10, int(6000 * priv_thresh / 100.0))`
- Integrate into daemon between inference and reporting

### 1.5 Today's Detections Page
**Files**: `birdnet-web/src/routes/pages/today.rs` (new), `birdnet-web/templates/today.html` (new)
- HTMX page showing today's detections
- Search with NOT prefix exclusion
- Pagination (40 at a time, lazy-load via HTMX)
- Delete individual detections
- Species image thumbnails
- Kiosk mode (auto-refresh)
- HTMX partial endpoints for infinite scroll

### 1.6 Dark Mode
**Files**: `birdnet-web/templates/layout.html` (modify CSS)
- CSS custom properties for theme colors
- Toggle via settings or `?theme=dark` query param
- Persist preference in settings table
- Respect `prefers-color-scheme` media query

## Phase 2: Audio Processing & Streaming

### 2.1 Audio Format Conversion
**Files**: `birdnet-core/src/audio/convert.rs` (new)
- sox subprocess for format conversion (WAV → MP3/FLAC/OGG/etc)
- Configurable output format (`AUDIOFMT` setting)
- Fallback to ffmpeg if sox not available

### 2.2 Frequency Shifting (Accessibility)
**Files**: `birdnet-core/src/audio/freq_shift.rs` (new)
- sox pitch effect or ffmpeg rubberband filter
- Configurable high/low frequency cutoff and pitch shift amount
- On-demand shifting for playback
- Optional activation in livestream

### 2.3 Live Audio Streaming
**Files**: `birdnet-web/src/routes/livestream.rs` (new)
- ffmpeg subprocess capturing from ALSA/RTSP → MP3 stream
- Serve via HTTP chunked transfer (no Icecast dependency)
- axum SSE or raw byte stream endpoint
- Optional frequency shifting in stream pipeline

## Phase 3: Localization & UI

### 3.1 Localization Framework
**Files**: `birdnet-core/src/i18n.rs` (new)
- Load BirdNET label files for 36 languages from `model/labels_l18n/`
- Species name translation map: sci_name → localized common name
- Config: `DATABASE_LANG` setting
- API endpoint for language switching

### 3.2 Recording Browser
**Files**: `birdnet-web/src/routes/pages/recordings.rs` (new), template
- Browse by species or by date
- Custom audio player with spectrogram visualization
- Delete, re-label, lock/unlock actions
- Sort by date or confidence

### 3.3 Daily Charts & Weekly Report
**Files**: `birdnet-web/src/routes/pages/charts.rs` (extend), new template
- Daily bar chart generation (use existing time-series API)
- Date navigation (prev/next/picker)
- Weekly report page with trend indicators

## Phase 4: Scheduler & Notifications

### 4.1 Wire birdnet-scheduler
**Files**: `src/main.rs`, `src/capture.rs`
- Integrate `birdnet-scheduler` crate into capture manager
- Solar-aware recording windows
- Configurable time windows

### 4.2 Enhanced Notifications
**Files**: `birdnet-integrations/src/apprise.rs` (extend)
- Template variable substitution ($sciname, $comname, $confidence, etc.)
- Multiple trigger modes: each detection, new species, new species daily
- Per-species cooldown (not just global)
- Image attachment support
- Weekly report notification
- Heartbeat URL
