# O-13 · Audio Sources management — first-class CRUD

<!-- BNB:STATUS-HEADER -->
> **Risk:** medium — introduces a new SQLite table and replaces the existing `/admin/audio` body. The page handler stays at the same URL; HTTP semantics are unchanged for unauthenticated viewers. **Priority:** 1 (primary support surface). · **Status:** ready for review
> Acceptance: VERIFY.md § O-13 · Rollback: ROLLBACK.md § O-13
<!-- BNB:STATUS-HEADER -->


## What

`admin/audio.rs` says in its own module doc: *"live device enumeration / level metering / persistence are a clearly-scoped stub."* The shipped UI renders two hard-coded rows (UMC202HD + Feeder cam) because `state.audio_source()` returns a single `Option<String>` and there's no storage for multiple sources.

This change makes an audio source a first-class entity:

1. New SQLite table `audio_sources` with columns for kind, device id / URL, friendly label, sample rate, channels, gain, schedule, pipeline flags, and a soft `disabled_at` for tombstones.
2. New trait `AudioSourceStore` in `birdnet-db` that's the only path the web layer takes to read/write sources.
3. A complete CRUD UI at `/admin/audio` with Add / Edit / Remove inline forms, a per-source kind selector (`USB · ALSA`, `PipeWire`, `RTSP`), contextual hints per kind, and live-status pills (`Capturing` moss, `Starting…` dawn, `Paused (schedule)` neutral, `Down` rare).
4. The two existing hard-coded rows are kept as a **seed migration** so anyone with a configured `audio_source` string lands on a populated page after upgrade (the parsed value becomes row #1).
5. Removes the `🎤` / `📡` emoji from the row icons — replaced with mono kind badges matching the rest of the system's voice.

The mock screen in production is a beautiful preview of a feature that doesn't exist yet. This wires the feature.

## Files

| Action | Path |
|---|---|
| Add | `crates/birdnet-web/templates/admin_audio_sources.html` — the page body, with `{{rows_local}}` / `{{rows_rtsp}}` / `{{count_local}}` / `{{count_rtsp}}` placeholders |
| Add | `crates/birdnet-web/templates/_partial_audio_source_row.html` — one row, used both on full render and as the response to add/edit |
| Replace | `crates/birdnet-web/src/routes/admin/audio.rs` — see `src/routes/admin/audio.rs` (full rewrite, ~340 lines; the body-rendering helpers from the old file are deleted) |
| Add | `crates/birdnet-db/migrations/008_audio_sources.sql` — table + seed from existing single-string config |
| Add | `crates/birdnet-db/src/audio_sources.rs` — `AudioSourceStore` trait + SQLite impl |

## New table

```sql
CREATE TABLE audio_sources (
    id            TEXT PRIMARY KEY,                -- e.g. "src_usb_1", "src_rtsp_2"
    kind          TEXT NOT NULL CHECK (kind IN ('usb-alsa','pipewire','rtsp')),
    device_id     TEXT NOT NULL,                   -- ALSA hw:1,0 / pw node.name / rtsp://…
    label         TEXT,
    sample_rate   INTEGER NOT NULL DEFAULT 48000,
    channels      TEXT NOT NULL DEFAULT 'mono'     -- 'mono' / 'left' / 'right' / 'stereo'
                  CHECK (channels IN ('mono','left','right','stereo')),
    bit_depth     INTEGER NOT NULL DEFAULT 24
                  CHECK (bit_depth IN (16, 24)),
    gain_db       REAL NOT NULL DEFAULT 0.0,
    rtsp_transport TEXT NOT NULL DEFAULT 'auto'
                  CHECK (rtsp_transport IN ('auto','tcp','udp')),
    schedule_quiet_start TEXT,                     -- HH:MM, NULL = no schedule
    schedule_quiet_end   TEXT,
    pipeline_high_pass  INTEGER NOT NULL DEFAULT 1,
    pipeline_dc_removal INTEGER NOT NULL DEFAULT 1,
    pipeline_agc        INTEGER NOT NULL DEFAULT 0,
    pipeline_rtsp_keepalive INTEGER NOT NULL DEFAULT 1,
    disabled_at   TEXT,                            -- soft delete (ISO-8601)
    created_at    TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at    TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX audio_sources_kind ON audio_sources (kind, disabled_at);
```

Migration also seeds row #1 from the existing `settings.audio_source` string when present:

```sql
INSERT INTO audio_sources (id, kind, device_id, label)
SELECT 'src_seed_1',
       CASE WHEN value LIKE 'rtsp://%' THEN 'rtsp'
            ELSE 'usb-alsa' END,
       value,
       NULL
  FROM settings
 WHERE key = 'audio_source'
   AND value IS NOT NULL AND value <> ''
   AND NOT EXISTS (SELECT 1 FROM audio_sources LIMIT 1);
```

## Endpoints

| Path | Method | Returns |
|---|---|---|
| `/admin/audio` | GET | full page (admin shell) |
| `/admin/audio/sources` | POST | one new row partial (or 422 + inline error) — target `#{scope}-list` |
| `/admin/audio/sources/{id}` | DELETE | empty body (htmx removes the row via `hx-swap="outerHTML"`) plus an OOB toast |
| `/admin/audio/sources/{id}/edit` | GET | row replaced with an inline edit form |
| `/admin/audio/sources/{id}` | PATCH | updated row |
| `/admin/audio/sources/{id}/probe` | POST | just returns the status pill (`Capturing` / `Starting…` / `Down`) — used by `hx-get="/admin/audio/sources/{id}/probe" hx-trigger="every 8s"` per row |

Live status is a server-rendered pill from a `probe(source) -> Status` call; this opportunity ships the **probe trait** with a stub impl that returns `Capturing` for any source the audio daemon currently reads, and `Down` otherwise. Wiring to real device state is the audio crate's responsibility.

## Trait shape

```rust
// crates/birdnet-db/src/audio_sources.rs

#[derive(Debug, Clone)]
pub struct AudioSource {
    pub id: String,
    pub kind: SourceKind,
    pub device_id: String,
    pub label: Option<String>,
    pub sample_rate: u32,
    pub channels: Channels,
    pub bit_depth: u8,
    pub gain_db: f32,
    pub rtsp_transport: RtspTransport,
    pub schedule_quiet: Option<(NaiveTime, NaiveTime)>,
    pub pipeline: PipelineFlags,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub trait AudioSourceStore: Send + Sync {
    fn list(&self) -> Result<Vec<AudioSource>>;
    fn get(&self, id: &str) -> Result<Option<AudioSource>>;
    fn insert(&self, new: NewAudioSource) -> Result<AudioSource>;
    fn update(&self, id: &str, patch: AudioSourcePatch) -> Result<AudioSource>;
    fn soft_delete(&self, id: &str) -> Result<()>;
}
```

## Connection to existing single-string config

`state.audio_source()` is kept for backwards compatibility but starts returning the **first non-disabled row's `device_id`** so the audio daemon doesn't need to change in this PR. A second follow-up PR can teach the daemon to consume all sources.

## Visual changes

The new page is sectioned into `<section>` blocks per kind (Local microphones / Network streams), matching the audio-sources prototype delivered in this folder. Compared to the current `/admin/audio`:

- **Per-source kind badge** in mono (`USB · ALSA`, `PipeWire`, `RTSP`) replaces the `🎤` / `📡` emoji.
- **Status pill** with semantic colour and a moss-pulse on the live state.
- **Per-row Edit / Remove** buttons (Remove goes through O-17 confirm modal).
- **Inline `<details>` add form** with kind-selector chips, contextual hint switching (`arecord -l` for ALSA, `pw-cli list-objects` for PipeWire, RTSP URL example for RTSP).
- **Restart banner** at page foot (`Changes apply on the next restart.`) routes through O-18 toast with a "Restart now" action when there's anything pending.
- **Empty state** uses `empty_states::no_chorus()`-style illustration when both groups are empty.
- The hard-coded `Combined input · 2 mics` rail card is **replaced** with a live summary derived from `list()` results.

## Risk

Medium-low. The migration is additive (new table) and seed-only; existing audio daemon code reads `state.audio_source()` which still returns a string. Roll back by running the down-migration and reverting the `audio.rs` rewrite — both files independent.

---

<!-- BNB:CROSSREF-FOOTER -->
## Related

* Built on O-17 (every Remove uses the themed confirm modal).
* Built on O-18 (Save Source → success toast; Restart required → warn toast).
* Built on O-16 (page renders skeletons while htmx polls `/probe` for each row's status pill).
* Companion: O-25 (inline-style audit) — the rewrite eliminates the bulk of the inline `<div style="…">` strings in the current `admin/audio.rs`.
<!-- BNB:CROSSREF-FOOTER -->
