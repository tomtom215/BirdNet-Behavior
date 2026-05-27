# O-23 · Signal-context overlay (weather · moon phase · SPL)

<!-- BNB:STATUS-HEADER -->
> **Risk:** low (weather / moon) · medium (SPL — needs audio-daemon write) · **Priority:** 4 · **Status:** RFC + drop-in renderers for weather + moon
> Acceptance: VERIFY.md § O-23 · Rollback: ROLLBACK.md § O-23
<!-- BNB:STATUS-HEADER -->


## What

Bird vocal behaviour tracks pressure, precipitation, moon phase, and ambient noise. None of those are visible on any chart in the dashboard today. Adding even a thin overlay along the bottom of the day-strip and the dawn-chorus polar would close a real why-loop ("activity dropped because the lawnmower started", "the owls called more because the moon was full").

This change introduces three signal channels that can overlay onto any time-axis chart:

1. **Weather** — temperature, precipitation, wind, pressure — from **Open-Meteo** (free, no API key required, station lat/lon already known from onboarding). One small `weather` table in SQLite, populated by a background job that runs every 30 min.
2. **Moon phase** — pure astronomical computation, no network call. A simple Conway approximation against the station's local time.
3. **SPL / ambient noise** — needs an audio-daemon column on `detections` (or its own series table). Listed here as scope but **not implemented in this PR** — see Schema follow-up.

For each channel, an `overlays::*` Rust module emits an SVG fragment sized to the host chart's coord system, plus a small legend chip. The day-strip and the dawn-chorus polar are the first call sites.

## Files

| Action | Path |
|---|---|
| Add | `crates/birdnet-web/src/routes/pages/overlays.rs` — three renderers (`weather_band(...)`, `moon_band(...)`, `spl_band(...)`) returning SVG strings sized to the caller's coord system |
| Add | `crates/birdnet-integrations/src/weather.rs` — Open-Meteo client + 30-min poll job |
| Add | `crates/birdnet-db/migrations/010_weather.sql` — `weather(at, temp_c, precip_mm, wind_kt, pressure_hpa)` |
| Add | `crates/birdnet-db/src/weather.rs` — `WeatherStore` read/write |
| Append | `crates/birdnet-web/static/css/app.css` — see `css/app.css.append` |
| Patch | `pages/today.rs` (day-strip) — include `overlays::weather_band(...)` + `overlays::moon_band(...)` at chart-bottom |
| Patch | `pages/dawn_chorus.rs` (polar) — include `overlays::moon_band(...)` as a thin outer arc |

## Schema (weather)

```sql
CREATE TABLE weather (
    at            TEXT PRIMARY KEY,           -- ISO-8601 UTC; one row per 30-min slot
    temp_c        REAL,
    precip_mm     REAL,
    wind_kt       REAL,
    wind_dir_deg  INTEGER,
    pressure_hpa  REAL,
    cloud_pct     INTEGER,
    code          INTEGER                     -- WMO weather code (cached for the icon)
);
```

The poll job hits `https://api.open-meteo.com/v1/forecast?latitude=…&longitude=…&hourly=…` once every 30 minutes, **never inside a request handler**, and writes the upcoming 24h plus the prior 24h. Total storage: ~17 KB / month. Failures: log + retry once; never propagate to a request.

Open-Meteo is the chosen source because:

- No API key required, no signup, no rate limit headache for the per-Pi traffic profile.
- ToS allows non-commercial use (which a single-station dashboard is).
- Returns the exact metrics that move bird behaviour (precip, pressure, wind), in one query.
- They self-host as well — anyone uneasy about a third-party fetch can run [Open-Meteo locally](https://github.com/open-meteo/open-meteo) and point `BNB_WEATHER_BASE_URL` at it.

If a station chooses to disable weather (no internet on the Pi, or privacy preference), `BNB_WEATHER_DISABLED=1` skips the poller and every overlay quietly drops out — no broken UI, no error states.

## Moon phase (no network)

```rust
// Conway approximation, accurate to ±1 day. The dawn-chorus story doesn't need
// hour-precision — a phase value 0.0..1.0 is enough.
pub fn moon_phase_at(unix_seconds: i64) -> f32 {
    let days_since_epoch = (unix_seconds as f64) / 86_400.0;
    let synodic = 29.530_588_853;     // mean synodic month, days
    let phase = ((days_since_epoch - 6.305) / synodic).fract();   // 0..1
    if phase < 0.0 { phase + 1.0 } else { phase } as f32
}
```

The dawn-chorus polar gains a thin outer arc (4 segments per night: new / waxing / full / waning) shaded with `--night` to `--dawn` mix. The day-strip gains a small moon glyph at the top-right with `data-phase="full"` for screen readers. No new tokens.

## SPL — follow-up

Two paths:

- **Per-detection.** Add `detections.spl_db_lufs REAL` and emit a measurement when each detection fires. Cheap to query (already grouped by hour), but only sampled at detection events.
- **Per-minute series.** A new `spl_minutes(at, source_id, lufs)` table populated by the audio daemon every 60s. Higher cost but lets the operator see "the mower ran 14:10–14:45" without a detection to anchor on.

The visual overlay is identical either way: a faint horizontal band along the chart's bottom, log-scaled, with hairline ticks at 30 / 50 / 70 dB. This DIFF includes the renderer (`overlays::spl_band`) and the CSS, **but no DB writes** — the audio daemon needs to decide which path before storage lands.

## Visual

For the day-strip (Today):

```
┌────────────────────────────────────────────────────────────┐
│  detection density · 24 hours                              │
│  ●● dots on a histogram                                    │
├────────────────────────────────────────────────────────────┤
│ weather  ◐ moon  ━  ━  ━  ━  ━  ━  ━  ━  ━  ━  ━  ━  ━ │  ← overlay strip
└────────────────────────────────────────────────────────────┘
```

The weather strip uses three colour bands: temperature (a wave between `--moss-soft` and `--dawn-soft`), precipitation (vertical droplets in `--dawn-soft`), wind (a thin grey line). Pressure is reserved for tomorrow's "front incoming" annotation. The whole strip is 22px tall — it adds context without dominating.

For the dawn-chorus polar: a single 4-segment outer ring tinted moon-phase, plus a temperature ring just inside it (one numbered value per hour). The polar's existing ribbons are untouched; everything new is *outside* the species ring.

## Risk

- **Weather + Moon:** low — both ride on existing chart machinery, neither blocks rendering when missing.
- **SPL:** medium — requires the audio daemon to write a new column / table. Scoped here as design only.

---

<!-- BNB:CROSSREF-FOOTER -->
## Related

* Uses O-16 skeleton's `bnb-skel-bars` shape as the placeholder while the weather row is still being polled.
* Uses O-25 utility classes (`bnb-row`, `bnb-stack`) for the legend chip layout.
<!-- BNB:CROSSREF-FOOTER -->
