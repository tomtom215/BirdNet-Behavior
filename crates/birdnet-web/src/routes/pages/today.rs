//! The **Today** home (`/`) — the v3-spine merge of the old Dashboard and
//! Today pages ("one calm page, five layers", `Today_home.html` in the
//! handover packet).
//!
//! | Layer       | Surface                                                    |
//! |-------------|------------------------------------------------------------|
//! | glance      | comparative phrase (`/pages/today-phrase`) + honest live signal |
//! | conditional | review nudge / outage banner (`/pages/today-nudge`)        |
//! | the shape   | day strip with in-strip temperature (`/pages/today-daystrip`) |
//! | heartbeat   | one unified log: live feed + full-day disclosure           |
//! | support     | right rail: top species · best recordings · station line   |
//!
//! The old Dashboard's "live feed" and Today's "detection log" were the same
//! data twice; the unified log shows the freshest rows and the full paginated
//! day (search, category filter, lock & delete) behind one disclosure.
//! A brand-new station (no detections ever) gets the "empty hour" treatment:
//! a getting-ready checklist in the hero instead of the live-signal card.

use std::fmt::Write as _;

use axum::extract::{Form, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::{Router, routing::get};
use birdnet_db::sqlite::TodayFilter;
use serde::Deserialize;

use super::atoms::{avatar, conf_bar};
use super::{TODAY_PAGE_HTML, escape_html, simple_url_encode, today_date_string};
use crate::state::AppState;

/// Mount the Today home and its HTMX partial routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(today_home))
        .route("/pages/today-list", get(today_partial))
        .route("/pages/today-daystrip", get(today_daystrip_partial))
        .route("/pages/today-count", get(today_count_partial))
        .route("/pages/today-pills", get(today_pills_partial))
        .route("/pages/today-nudge", get(today_nudge_partial))
        .route("/pages/today-delete", axum::routing::post(delete_detection))
        .route(
            "/pages/today-relabel",
            axum::routing::post(relabel_detection),
        )
        .route("/pages/today-lock", axum::routing::post(lock_detection))
        .route("/pages/today-unlock", axum::routing::post(unlock_detection))
}

// ---------------------------------------------------------------------------
// The home page
// ---------------------------------------------------------------------------

/// Capture-silence thresholds for the outage affordance. With a location set
/// the strict threshold applies only in daylight (overnight silence is
/// normal); without one we can't tell night from day, so only a long silence
/// is flagged.
const OUTAGE_DAYTIME_SECS: u64 = 2 * 3600;
const OUTAGE_NO_LOCATION_SECS: u64 = 6 * 3600;

async fn today_home(State(state): State<AppState>, headers: HeaderMap) -> Response {
    // First run: a station with no detections that hasn't completed onboarding
    // is bounced to the setup wizard instead of an empty home.
    if first_run_needs_onboarding(&state) {
        return Redirect::to("/onboarding").into_response();
    }

    let state_for_query = state.clone();
    let (total_ever, sources, disk_pct) = tokio::task::spawn_blocking(move || {
        let (total, sources) = state_for_query.with_db(|conn| {
            use birdnet_db::audio_sources::AudioSourceStore;
            let total = birdnet_db::sqlite::detection_count(conn).unwrap_or(0);
            let sources = AudioSourceStore::list(conn).unwrap_or_default();
            (total, sources)
        });
        let disk_pct = disk_used_percent(&state_for_query);
        (total, sources, disk_pct)
    })
    .await
    .unwrap_or((0, Vec::new(), None));

    let firstrun = total_ever == 0;
    let enabled: Vec<_> = sources.iter().filter(|s| s.disabled_at.is_none()).collect();

    // Whether the first configured source is *actually capturing*, from the
    // supervisor's own gauge — the same signal the Capture tab's status pill
    // reads. A source being configured says nothing about audio flowing.
    let capturing = enabled
        .first()
        .and_then(|s| state.metrics().source_up(&s.id));

    let hero_aside = if firstrun {
        firstrun_checklist(&enabled, disk_pct, capturing)
    } else {
        signal_card(&super::listen::source_options(&sources), enabled.first())
    };
    let rail_extra = if firstrun {
        r#"<div class="bnb-card pad"><div class="bnb-eyebrow td-look-eb">While you wait</div><div class="x-look"><a href="/admin/audio">Add a second microphone <span class="arr">→</span></a><a href="/admin/rules">Set up rare-bird alerts <span class="arr">→</span></a><a href="/admin/migrate">Import your BirdNET-Pi history <span class="arr">→</span></a></div></div>"#
            .to_string()
    } else {
        String::new()
    };
    let hero_phrase = if firstrun {
        FIRSTRUN_PHRASE
    } else {
        r#"<h1 class="display td-h1">You're listening.</h1>
<p class="bnb-meta td-sub">Detections roll in below.</p>"#
    };

    let body = TODAY_PAGE_HTML
        .replace("{{firstrun}}", if firstrun { "true" } else { "false" })
        .replace("{{today_human_date}}", &human_date())
        .replace("{{hero_phrase}}", hero_phrase)
        .replace("{{hero_aside}}", &hero_aside)
        .replace("{{rail_extra}}", &rail_extra)
        .replace("{{moon_inline}}", &moon_inline())
        .replace("{{skel_daystrip}}", super::skeletons::day_strip())
        .replace("{{skel_detections}}", &super::skeletons::feed_rows(8))
        .replace(
            "{{help_link}}",
            &super::help::help_link(super::help::Topic::Today),
        );
    super::render_page_for_request("Today", &body, "today", &headers).into_response()
}

/// The hero copy a brand-new station wakes up with (the comparative-phrase
/// partial returns the same copy until the first detection lands).
pub(super) const FIRSTRUN_PHRASE: &str = r#"<h1 class="display td-h1">Your station is <em class="tp-c-moss-ink">waking up</em>.</h1>
<p class="bnb-meta td-sub">Everything checks out — we're listening for the first call. It usually arrives within the hour, and this page comes alive the moment it does.</p>"#;

/// The live-signal card (real spectrogram canvas + the source row).
///
/// `first` is the station's first enabled source, used for the footer's
/// input/rate line. That line used to read a hard-coded `input · mic` and
/// `48 kHz` for every station — so an RTSP-only station was labelled "mic",
/// and a 16 or 44.1 kHz source was reported as 48 kHz directly beneath a live
/// spectrogram of its own audio.
fn signal_card(
    source_options: &str,
    first: Option<&&birdnet_db::audio_sources::AudioSource>,
) -> String {
    let (input_label, rate) = first.map_or_else(
        || ("input · none".to_string(), "—".to_string()),
        |s| {
            (
                format!("input · {}", s.kind.as_str()),
                format!("{} kHz", s.sample_rate / 1000),
            )
        },
    );
    format!(
        r#"<div class="bnb-card pad db-signal-card">
      <div class="db-signal-head">
        <span class="bnb-eyebrow">Live signal · last 30 s</span>
        <span class="bnb-pill db-live-pill"><span class="bnb-dot"></span> idle</span>
      </div>
      <canvas id="hero-pulse" height="80" class="db-pulse"></canvas>
      <div class="db-signal-foot">
        <span class="mono bnb-meta">{input_label}</span>
        <span class="mono bnb-meta">{rate}</span>
        <span class="mono bnb-meta">BirdNET V3.0</span>
      </div>
      <div class="x-sig-row">
        <span class="x-sig-src"><span class="bnb-meta">source</span><select id="td-source" aria-label="Audio source to monitor">{source_options}</select></span>
        <a class="bnb-btn ghost x-listen" href="/recordings?view=live" title="Open the live spectrogram & audio">Listen live →</a>
      </div>
    </div>"#
    )
}

/// The first-run "Getting ready" checklist.
///
/// This is the single card a brand-new operator reads to answer "is my station
/// working?", so every tick has to be earned. Two of them were not:
///
/// * **The microphone row ticked as soon as a source was *configured*.** Being
///   in the `audio_sources` table says nothing about audio flowing; a source
///   whose device disappeared on reboot (the ALSA card-index bug) or whose
///   `arecord` is dead is configured and silent. `capturing` now carries the
///   supervisor's own gauge — the same signal the Capture tab's pill reads.
/// * **The disk row was a hard-coded `✓`.** The percentage and the wording were
///   real, so the card could read "Room to record ✓ — nearly full — 97% used":
///   a green tick on a station about to run out of space, which a
///   non-technical operator reads as "fine".
///
/// The model row states what is bundled, not that inference has succeeded, and
/// is worded accordingly — the page has no runtime signal for that, and an
/// unearned "loaded ✓" is the same defect as the other two.
fn firstrun_checklist(
    enabled: &[&birdnet_db::audio_sources::AudioSource],
    disk_pct: Option<f64>,
    capturing: Option<bool>,
) -> String {
    let (mic_mark, mic_title, mic_detail, mic_value) = enabled.first().map_or_else(
        || {
            (
                "wait",
                "Waiting for a microphone".to_string(),
                r#"add one under <a href="/admin/audio">Settings → Capture</a>"#.to_string(),
                "—".to_string(),
            )
        },
        |first| {
            let label = first
                .label
                .clone()
                .unwrap_or_else(|| first.device_id.clone());
            let more = if enabled.len() > 1 {
                format!(" +{}", enabled.len() - 1)
            } else {
                String::new()
            };
            let source = format!("{} · {}{more}", first.kind.as_str(), escape_html(&label));
            match capturing {
                // Configured *and* the supervisor reports it up.
                Some(true) => (
                    "done",
                    "Microphone recording".to_string(),
                    source,
                    format!("{} kHz", first.sample_rate / 1000),
                ),
                // Configured but the capture subprocess is not running. This is
                // the silent failure the card existed to rule out, so it says so
                // and points at the page that can diagnose it.
                Some(false) => (
                    "fail",
                    "Microphone not recording".to_string(),
                    format!(
                        "{source} — configured, but no audio is being captured. \
                         Check it under <a href=\"/station/capture\">Settings → Capture</a>"
                    ),
                    "down".to_string(),
                ),
                // No gauge yet: the supervisor has not reconciled this source.
                // Normal for the first seconds after a start, so it waits
                // rather than claiming either outcome.
                None => (
                    "wait",
                    "Microphone starting…".to_string(),
                    source,
                    format!("{} kHz", first.sample_rate / 1000),
                ),
            }
        },
    );
    let mic_mark_html = match mic_mark {
        "done" => r#"<span class="mk done">✓</span>"#,
        "fail" => r#"<span class="mk down">!</span>"#,
        _ => r#"<span class="mk wait"><span class="bnb-dot"></span></span>"#,
    };
    // The mark tracks the same thresholds as the wording. It used to be a
    // hard-coded `✓`, so "nearly full · 97% used" shipped with a green tick.
    let (disk_mark_html, disk_detail, disk_value) = disk_pct.map_or_else(
        || {
            (
                r#"<span class="mk wait"><span class="bnb-dot"></span></span>"#,
                "checking…".to_string(),
                "—".to_string(),
            )
        },
        |pct| {
            let (mark, detail) = if pct < 70.0 {
                (r#"<span class="mk done">✓</span>"#, "plenty of space")
            } else if pct < 90.0 {
                (
                    r#"<span class="mk wait"><span class="bnb-dot"></span></span>"#,
                    "getting full — old recordings are deleted automatically",
                )
            } else {
                (
                    r#"<span class="mk down">!</span>"#,
                    "nearly full — recording may stop soon",
                )
            };
            (mark, detail.to_string(), format!("{pct:.0}% used"))
        },
    );
    format!(
        r#"<div class="bnb-card pad">
      <div class="bnb-eyebrow td-check-eb">Getting ready</div>
      <div class="x-check">
        <div class="x-check-row">{mic_mark_html}<div class="c"><div class="t">{mic_title}</div><div class="d">{mic_detail}</div></div><span class="v">{mic_value}</span></div>
        <div class="x-check-row"><span class="mk done">✓</span><div class="c"><div class="t">Model bundled</div><div class="d">BirdNET V3.0 — ships with the app</div></div><span class="v">included</span></div>
        <div class="x-check-row">{disk_mark_html}<div class="c"><div class="t">Room to record</div><div class="d">{disk_detail}</div></div><span class="v">{disk_value}</span></div>
        <div class="x-check-row"><span class="mk wait"><span class="bnb-dot live"></span></span><div class="c"><div class="t">Listening for the first call…</div><div class="d">this can take a few minutes</div></div><span class="v">—</span></div>
      </div>
    </div>"#
    )
}

/// Whether to bounce a fresh station to the onboarding wizard: no detections
/// yet, onboarding not marked complete, **and** no location configured. Fails
/// safe — any DB error is treated as "already set up" so a hiccup never traps
/// the operator on `/onboarding`.
///
/// The location check is what stops a station the installer already configured
/// (latitude/longitude written to the config file, then seeded into the
/// `settings` table at startup — see `helpers::seed_db_settings_from_config`)
/// from being re-prompted for setup it already completed during installation.
fn first_run_needs_onboarding(state: &AppState) -> bool {
    state.with_db(|conn| {
        let onboarded = birdnet_db::settings::get_or(conn, "onboarding_complete", "false")
            .map_or(true, |v| v == "true");
        let has_detections = birdnet_db::sqlite::detection_count(conn).map_or(true, |n| n > 0);
        let lat = birdnet_db::settings::get_or(conn, "latitude", "").unwrap_or_default();
        let lon = birdnet_db::settings::get_or(conn, "longitude", "").unwrap_or_default();
        let has_location = !lat.trim().is_empty() && !lon.trim().is_empty();
        !onboarded && !has_detections && !has_location
    })
}

// ---------------------------------------------------------------------------
// Shared context helpers (solar, weather, freshness)
// ---------------------------------------------------------------------------

/// Today's sunrise/sunset as fractional hours in **local** time, from the
/// configured station location. `None` when no location is set or the sun never
/// rises/sets at this latitude today.
///
/// Local, because every other value on this axis is: the day strip's bars come
/// from `hourly_activity`, which buckets the local `Time` column, and the "now"
/// marker is [`super::now_hour_local`]. Returning the solver's raw UTC minutes here
/// drew sunrise two hours early on a CEST station and mislabelled the pills
/// with it.
fn solar_times_today(conn: &rusqlite::Connection) -> Option<(f64, f64)> {
    let lat: f64 = birdnet_db::settings::get_or(conn, "latitude", "")
        .ok()?
        .trim()
        .parse()
        .ok()?;
    let lon: f64 = birdnet_db::settings::get_or(conn, "longitude", "")
        .ok()?
        .trim()
        .parse()
        .ok()?;
    let location = birdnet_scheduler::Location::new(lat, lon).ok()?;
    let date = today_date_string();
    let year: u32 = date.get(0..4)?.parse().ok()?;
    let month: u32 = date.get(5..7)?.parse().ok()?;
    let day: u32 = date.get(8..10)?.parse().ok()?;
    let solar = birdnet_scheduler::SolarDay::for_date(location, year, month, day).ok()?;
    #[allow(clippy::cast_precision_loss)]
    let offset_h = super::local_utc_offset_secs() as f64 / 3600.0;
    let sunrise = wrap_hour(f64::from(solar.sunrise_utc_min?) / 60.0 + offset_h);
    let sunset = wrap_hour(f64::from(solar.sunset_utc_min?) / 60.0 + offset_h);
    Some((sunrise, sunset))
}

/// Fold an hour-of-day into `[0, 24)` after a UTC→local shift.
///
/// A station far enough east or west pushes sunrise past midnight; without this
/// the value leaves the axis the strip draws and the marker vanishes off one
/// end rather than wrapping to the other.
fn wrap_hour(h: f64) -> f64 {
    h.rem_euclid(24.0)
}

/// Capture-outage check: the station has detected before, the silence exceeds
/// the threshold, and (with a location) we're inside daylight, when silence
/// is anomalous. Returns the silence duration and the last detection's time.
pub(super) fn capture_outage(conn: &rusqlite::Connection) -> Option<(u64, String)> {
    let silent = birdnet_db::sqlite::seconds_since_last_detection(conn)
        .ok()
        .flatten()?;
    let threshold = match solar_times_today(conn) {
        Some((sunrise, sunset)) => {
            let now = super::now_hour_local();
            if now < sunrise || now > sunset {
                return None; // overnight silence is normal
            }
            OUTAGE_DAYTIME_SECS
        }
        None => OUTAGE_NO_LOCATION_SECS,
    };
    if silent < threshold {
        return None;
    }
    let last = conn
        .query_row(
            "SELECT Time FROM detections ORDER BY Date DESC, Time DESC LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok()?;
    Some((silent, last.get(0..5).unwrap_or(&last).to_string()))
}

/// Filesystem usage of the data directory as a used percentage.
///
/// `pub(super)` so the header health badge grades the same number the
/// dashboard shows, rather than a second measurement that could disagree.
pub(super) fn disk_used_percent(state: &AppState) -> Option<f64> {
    let db_path = state.db_path().to_path_buf();
    let dir = db_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(
            || std::path::PathBuf::from("."),
            std::path::Path::to_path_buf,
        );
    birdnet_core::audio::capture::disk_usage(&dir)
        .ok()
        .map(|u| u.used_percent())
}

/// "Friday, June 13" from today's date — the hero eyebrow's human form.
fn human_date() -> String {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    const DAYS: [&str; 7] = [
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
    ];
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let epoch_days = secs / 86_400;
    let (_, m, d) = super::days_to_date(epoch_days);
    // 1970-01-01 was a Thursday.
    let weekday = DAYS[(epoch_days % 7) as usize];
    let month = MONTHS[(m as usize).saturating_sub(1).min(11)];
    format!("{weekday}, {month} {d}")
}

/// "☾ first quarter" — the day-strip header's inline moon note.
fn moon_inline() -> String {
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0_i64, |x| i64::try_from(x.as_secs()).unwrap_or(i64::MAX));
    let cardinal =
        super::overlays::MoonCardinal::from_phase(super::overlays::moon_phase_at(now_secs));
    format!("{} {}", cardinal.glyph(), cardinal.label())
}

// ---------------------------------------------------------------------------
// Pills + nudge partials
// ---------------------------------------------------------------------------

/// What the capture supervisor says about the station's audio sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CaptureState {
    /// No enabled source is configured at all — nothing can ever be recorded.
    NoSource,
    /// At least one enabled source and the supervisor reports it up.
    Up,
    /// Sources are configured and none is up.
    Down,
    /// Sources are configured but no gauge has been published yet (the
    /// supervisor has not reconciled them, e.g. in the first seconds after a
    /// start, or in a web-only process). Not an outage — just not known.
    Unknown,
}

/// Resolve [`CaptureState`] from the same per-source gauge the Capture tab's
/// status pill reads, so the dashboard and that page cannot disagree.
pub(super) fn live_capture_state(state: &AppState) -> CaptureState {
    use birdnet_db::audio_sources::AudioSourceStore;
    let sources = state.with_db(AudioSourceStore::list).unwrap_or_default();
    if sources.is_empty() {
        return CaptureState::NoSource;
    }
    let gauges: Vec<Option<bool>> = sources
        .iter()
        .map(|s| state.metrics().source_up(&s.id))
        .collect();
    if gauges.contains(&Some(true)) {
        CaptureState::Up
    } else if gauges.iter().all(Option::is_none) {
        CaptureState::Unknown
    } else {
        CaptureState::Down
    }
}

/// HTMX partial: the hero pill row — recording state, weather, sunrise/sunset,
/// station identity. Every pill is backed by real data and absent otherwise.
async fn today_pills_partial(State(state): State<AppState>) -> impl IntoResponse {
    let site_name = state.site_name().to_string();
    // Live capture state from the supervisor's gauge, resolved before the DB
    // work so the pill can say something true on a station that has *never*
    // detected anything. `capture_outage` alone cannot: it measures time since
    // the last detection, and with no detections at all it returns `None` —
    // which the pill rendered as a confident green "recording". That is exactly
    // the first-run state, so a station whose microphone never worked showed
    // "recording" indefinitely and nothing ever contradicted it.
    let capture = live_capture_state(&state);
    let html = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let mut out = String::with_capacity(512);

            // Recording state. The gauge is authoritative when it has an
            // opinion; otherwise fall back to the detection-freshness signal
            // that drives the outage banner, so the two cannot disagree.
            match (capture, capture_outage(conn)) {
                (CaptureState::NoSource, _) => out.push_str(
                    r#"<span class="bnb-pill rare"><span class="bnb-dot"></span> not recording · no microphone configured</span>"#,
                ),
                (CaptureState::Down, _) => out.push_str(
                    r#"<span class="bnb-pill rare"><span class="bnb-dot"></span> not recording · capture is down</span>"#,
                ),
                (_, Some((_, last))) => {
                    let _ = write!(
                        out,
                        r#"<span class="bnb-pill rare"><span class="bnb-dot"></span> recording stopped · last heard {last}</span>"#
                    );
                }
                (_, None) => out.push_str(
                    r#"<span class="bnb-pill moss"><span class="bnb-dot live"></span> recording</span>"#,
                ),
            }

            // Weather: today's cached samples → current temperature + H/L.
            let today = today_date_string();
            let samples: Vec<birdnet_db::weather::WeatherRow> = {
                use birdnet_db::weather::WeatherStore as _;
                let from = format!("{today}T00:00:00Z");
                let to = format!("{today}T23:59:59Z");
                conn.range(&from, &to).unwrap_or_default()
            };
            let temps: Vec<f32> = samples.iter().filter_map(|s| s.temp_c).collect();
            if let (Some(&now_t), Some(min), Some(max)) = (
                temps.last(),
                temps.iter().copied().reduce(f32::min),
                temps.iter().copied().reduce(f32::max),
            ) {
                let _ = write!(
                    out,
                    r#"<span class="bnb-pill x-wx"><span class="mono x-wx-now">{now_t:.0}°</span><span class="mono x-wx-hl">H {max:.0}° · L {min:.0}°</span></span>"#
                );
            }

            // Sunrise / sunset from the configured location.
            if let Some((sunrise, sunset)) = solar_times_today(conn) {
                let _ = write!(
                    out,
                    r#"<span class="bnb-pill">☀ sunrise {}</span><span class="bnb-pill">☾ sunset {}</span>"#,
                    fmt_hour(sunrise),
                    fmt_hour(sunset)
                );
            }

            // Station identity: name and/or coordinates.
            let lat = birdnet_db::settings::get_or(conn, "latitude", "").unwrap_or_default();
            let lon = birdnet_db::settings::get_or(conn, "longitude", "").unwrap_or_default();
            let coords = match (lat.trim().parse::<f64>(), lon.trim().parse::<f64>()) {
                (Ok(lat), Ok(lon)) => {
                    let ns = if lat >= 0.0 { 'N' } else { 'S' };
                    let ew = if lon >= 0.0 { 'E' } else { 'W' };
                    format!("{:.2}°{ns} · {:.2}°{ew}", lat.abs(), lon.abs())
                }
                _ => String::new(),
            };
            if !site_name.is_empty() || !coords.is_empty() {
                let _ = write!(
                    out,
                    r#"<span class="bnb-pill">{}{}</span>"#,
                    escape_html(&site_name),
                    if coords.is_empty() {
                        String::new()
                    } else {
                        format!(r#"<span class="mono x-coord">{coords}</span>"#)
                    }
                );
            }
            out
        })
    })
    .await
    .unwrap_or_default();

    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

/// HTMX partial: the conditional strip under the hero — a review nudge when
/// rare detections await confirmation, an outage banner when capture has gone
/// quiet, otherwise nothing at all ("absent entirely when nothing waits").
async fn today_nudge_partial(State(state): State<AppState>) -> impl IntoResponse {
    let html = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let pending = birdnet_db::sqlite::quarantine_pending_count(conn).unwrap_or(0);
            if pending > 0 {
                let (noun, verb, obj) = if pending == 1 {
                    ("rare sighting is", "it's", "it")
                } else {
                    ("rare sightings are", "they're", "them")
                };
                return format!(
                    r#"<div class="x-nudge" data-screen-label="Review nudge"><span class="ico">✦</span><div class="txt"><b>{pending} {noun} waiting for your eye.</b> Confirm {verb} real to add {obj} to your records.</div><a class="bnb-btn primary" href="/quarantine">Review →</a></div>"#
                );
            }
            if let Some((silent, last)) = capture_outage(conn) {
                let dur = fmt_duration(silent);
                return format!(
                    r#"<div class="x-nudge" data-screen-label="Outage banner"><span class="ico">⚠</span><div class="txt"><b>No detections for {dur}.</b> The last one was at <span class="mono">{last}</span> — the microphone may be unplugged or the recorder stopped.</div><a class="bnb-btn primary" href="/station">Open Settings →</a></div>"#
                );
            }
            String::new()
        })
    })
    .await
    .unwrap_or_default();

    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

/// `6.35` hours → `"6:21"`.
fn fmt_hour(h: f64) -> String {
    let total_min = (h * 60.0).round();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let (hh, mm) = ((total_min / 60.0) as u32 % 24, (total_min % 60.0) as u32);
    format!("{hh}:{mm:02}")
}

/// `8040` seconds → `"2h 14m"` (or `"45m"` under an hour).
fn fmt_duration(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    if h > 0 {
        format!("{h}h {m:02}m")
    } else {
        format!("{m}m")
    }
}

// ---------------------------------------------------------------------------
// The full-day list (search + category filter + pagination)
// ---------------------------------------------------------------------------

/// Query parameters for the today list partial.
#[derive(Debug, Deserialize)]
pub struct TodayParams {
    /// Search filter. Prefix with "NOT " for exclusion.
    pub search: Option<String>,
    /// Category filter token: `all` (default) · `rare` · `first` · `high`.
    pub filter: Option<String>,
    /// Pagination offset.
    pub offset: Option<u32>,
    /// Items per page (default 40).
    pub limit: Option<u32>,
    /// When set, `today-count` returns the bare number (for inline slots).
    pub bare: Option<String>,
}

/// Form data for deleting a detection.
#[derive(Debug, Deserialize)]
pub struct DeleteForm {
    /// Detection date in `YYYY-MM-DD` form.
    pub date: String,
    /// Detection time in `HH:MM:SS` form.
    pub time: String,
    /// Scientific name that uniquely identifies the detection row alongside date/time.
    pub sci_name: String,
}

/// Form data for locking/unlocking a detection.
#[derive(Debug, Deserialize)]
pub struct LockForm {
    /// Detection date in `YYYY-MM-DD` form.
    pub date: String,
    /// Detection time in `HH:MM:SS` form.
    pub time: String,
    /// Scientific name that uniquely identifies the detection row alongside date/time.
    pub sci_name: String,
}

/// Form data for re-labeling a detection.
#[derive(Debug, Deserialize)]
pub struct RelabelForm {
    /// Detection date in `YYYY-MM-DD` form.
    pub date: String,
    /// Detection time in `HH:MM:SS` form.
    pub time: String,
    /// Current scientific name used to locate the detection row.
    pub old_sci_name: String,
    /// Replacement scientific name to write into the row.
    pub new_sci_name: String,
    /// Replacement common name corresponding to `new_sci_name`.
    pub new_com_name: String,
}

/// HTMX partial: today's detection count. Returns the labelled form by
/// default; `?bare=1` returns just the formatted number (the full-day
/// disclosure button's inline count).
async fn today_count_partial(
    State(state): State<AppState>,
    Query(params): Query<TodayParams>,
) -> impl IntoResponse {
    let today = today_date_string();
    let search = params.search.clone();

    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            birdnet_db::sqlite::todays_detection_count(
                conn,
                &today,
                search.as_deref(),
                TodayFilter::All,
            )
        })
    })
    .await;

    match result {
        Ok(Ok(count)) => {
            let label = if params.bare.is_some() {
                super::group_thousands(count)
            } else if params.search.as_ref().is_some_and(|s| !s.trim().is_empty()) {
                format!("{count} matching detections")
            } else {
                format!("{count} detections today")
            };
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], label)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "Error loading count".to_string(),
        ),
    }
}

/// HTMX partial: paginated list of today's detections as cards.
async fn today_partial(
    State(state): State<AppState>,
    Query(params): Query<TodayParams>,
) -> impl IntoResponse {
    let today = today_date_string();
    let limit = params.limit.unwrap_or(40).min(200);
    let offset = params.offset.unwrap_or(0);
    let search = params.search.clone();
    let search2 = params.search.clone();
    let filter = TodayFilter::from_token(params.filter.as_deref());

    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let rows = birdnet_db::sqlite::todays_detections(
                conn,
                &today,
                search.as_deref(),
                filter,
                limit,
                offset,
            )?;
            let total = birdnet_db::sqlite::todays_detection_count(
                conn,
                &today,
                search.as_deref(),
                filter,
            )?;
            Ok::<_, birdnet_db::sqlite::DbError>((rows, total))
        })
    })
    .await;

    match result {
        Ok(Ok((detections, total))) => {
            let mut html = String::with_capacity(4096);

            if detections.is_empty() && offset == 0 {
                html.push_str("<p class=\"tdl-empty\">No detections found today.</p>");
                return (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html);
            }

            for d in &detections {
                render_detection_card(&mut html, d);
            }

            // "Load more" button if there are more results
            let shown = offset + limit;
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                clippy::cast_precision_loss,
                clippy::cast_possible_wrap,
                clippy::cast_lossless
            )]
            let total_u = total as u32;
            if shown < total_u {
                let search_param = search2
                    .as_ref()
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| format!("&search={}", simple_url_encode(s)))
                    .unwrap_or_default();
                let filter_param = params
                    .filter
                    .as_ref()
                    .filter(|f| !f.trim().is_empty() && f.as_str() != "all")
                    .map(|f| format!("&filter={}", simple_url_encode(f)))
                    .unwrap_or_default();
                let remaining = total_u.saturating_sub(shown);
                let _ = write!(
                    html,
                    "<div class=\"tdl-more\">\
                     <button hx-get=\"/pages/today-list?offset={shown}&limit={limit}{search_param}{filter_param}\" \
                     hx-target=\"#today-full\" hx-swap=\"innerHTML\" \
                     class=\"tdl-more-btn\">\
                     Load {limit} more ({remaining} remaining)\
                     </button></div>",
                );
            }

            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading detections</p>".to_string(),
        ),
    }
}

// ---------------------------------------------------------------------------
// The day strip
// ---------------------------------------------------------------------------

/// HTMX partial: the 24-hour `DayStrip` — hourly histogram, in-strip
/// temperature line, sunrise/sunset markers and a "now" line — plus an
/// out-of-band update for the header's stat trio (peak · dawn · total).
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
async fn today_daystrip_partial(State(state): State<AppState>) -> impl IntoResponse {
    let today = today_date_string();
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let rows = birdnet_db::sqlite::todays_detections(
                conn,
                &today,
                None,
                TodayFilter::All,
                1000,
                0,
            )?;
            // O-23 weather samples for the in-strip temperature line. Reads
            // the cached Open-Meteo rows — empty when the poll job hasn't
            // populated them, in which case the strip simply has no line.
            let samples: Vec<birdnet_db::weather::WeatherRow> = {
                use birdnet_db::weather::WeatherStore as _;
                let from = format!("{today}T00:00:00Z");
                let to = format!("{today}T23:59:59Z");
                conn.range(&from, &to).unwrap_or_default()
            };
            let solar = solar_times_today(conn);
            Ok::<_, birdnet_db::sqlite::DbError>((rows, samples, solar))
        })
    })
    .await;

    let Ok(Ok((rows, samples, solar))) = result else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading timeline</p>".to_string(),
        );
    };

    if rows.is_empty() {
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            // Clear the header stats out-of-band too, so a deleted last
            // detection doesn't strand stale numbers.
            r#"<div class="x-daystats" id="td-daystats" hx-swap-oob="true"></div><div class="bnb-meta tdl-strip-empty"><span>No detections yet today.</span></div>"#
                .to_string(),
        );
    }

    let mut hourly = [0i64; 24];
    for d in &rows {
        let hi = parse_hour_fraction(&d.time) as usize;
        if hi < 24 {
            hourly[hi] += 1;
        }
    }

    let temps: Vec<(f64, f64)> = samples
        .iter()
        .filter_map(|row| {
            let hour = row.at.get(11..13).and_then(|hh| hh.parse::<u8>().ok())?;
            let temp = row.temp_c?;
            Some((f64::from(hour) + 0.5, f64::from(temp)))
        })
        .collect();

    let total: i64 = hourly.iter().sum();
    let mut peak_hour = 0usize;
    let mut peak = -1i64;
    for (h, &c) in hourly.iter().enumerate() {
        if c > peak {
            peak = c;
            peak_hour = h;
        }
    }
    let dawn: i64 = hourly[4..9].iter().sum();

    let stats_oob = format!(
        r#"<div class="x-daystats" id="td-daystats" hx-swap-oob="true"><div><div class="v">{peak_hour:02}:00</div><div class="l">peak hour</div></div><div><div class="v x-dawn-v">{dawn}</div><div class="l">in dawn chorus</div></div><div><div class="v">{total_fmt}</div><div class="l">total today</div></div></div>"#,
        total_fmt = super::group_thousands(total),
    );
    let strip = super::viz::day_strip(&hourly, &temps, solar, super::now_hour_local());
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html")],
        format!("{stats_oob}{strip}"),
    )
}

/// Parse "HH:MM:SS" into a fractional hour in [0, 24). Best-effort.
fn parse_hour_fraction(t: &str) -> f64 {
    let mut it = t.split(':');
    let h: f64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let m: f64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
    h + m / 60.0
}

/// Render a single detection row into the HTML buffer.
fn render_detection_card(html: &mut String, d: &birdnet_db::sqlite::DetectionRow) {
    let enc_name = simple_url_encode(&d.com_name);

    // Fixed-size play affordance (shared clip player) — native <audio>
    // controls render at different widths per row, so they never aligned.
    let audio = d
        .file_name
        .as_deref()
        .filter(|f| !f.is_empty())
        .map(|f| {
            let basename = std::path::Path::new(f)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let safe = escape_html(&basename);
            format!(
                "<button type=\"button\" class=\"x-fplay tdl-play\" data-play-src=\"/api/v2/recordings/{safe}\" \
                 title=\"Play clip\" aria-label=\"Play clip\">▶</button>"
            )
        })
        .unwrap_or_default();

    let av = avatar(&d.com_name, "");
    let conf = conf_bar(d.confidence);
    let com_name = escape_html(&d.com_name);
    let sci_name = escape_html(&d.sci_name);
    let time = escape_html(&d.time);
    let date_enc = simple_url_encode(&d.date);
    let time_enc = simple_url_encode(&d.time);
    let date_raw = escape_html(&d.date);
    let time_raw = escape_html(&d.time);
    let sci_name_raw = escape_html(&d.sci_name);

    let _ = write!(
        html,
        "<div class=\"bnb-card tdl-card\">\
         {av}\
         <div class=\"tdl-card-main\">\
         <div class=\"tdl-card-head\">\
         <a href=\"/species/detail?name={enc_name}\" class=\"tdl-name\">{com_name}</a>\
         {conf}\
         </div>\
         <div class=\"bnb-meta mono tdl-card-sci\">{sci_name} · \
         <a href=\"/detections/detail?date={date_enc}&time={time_enc}&name={enc_name}\" class=\"tdl-time\">{time}</a></div>\
         {audio}\
         </div>\
         <div class=\"tdl-card-actions\">\
         <button class=\"bnb-btn ghost\" hx-post=\"/pages/today-lock\" \
         hx-vals='{{\"date\":\"{date_raw}\",\"time\":\"{time_raw}\",\"sci_name\":\"{sci_name_raw}\"}}' \
         hx-target=\"#today-full\" hx-swap=\"innerHTML\" hx-include=\"#search-form\" \
         title=\"Lock this detection (protect from auto-purge)\">🔒</button>\
         <button class=\"bnb-btn danger\" hx-post=\"/pages/today-delete\" \
         hx-vals='{{\"date\":\"{date_raw}\",\"time\":\"{time_raw}\",\"sci_name\":\"{sci_name_raw}\"}}' \
         hx-target=\"#today-full\" hx-swap=\"innerHTML\" hx-include=\"#search-form\" \
         hx-confirm=\"Delete detection of {com_name} at {time}?\" \
         data-confirm-action=\"hx-post\" \
         data-confirm-url=\"/pages/today-delete\" \
         data-confirm-title=\"Delete detection\" \
         data-confirm-body=\"Delete detection of {com_name} at {time}?\" \
         data-confirm-confirm-label=\"Delete\" \
         data-confirm-style=\"danger\" \
         title=\"Delete this detection\">Delete</button>\
         </div></div>",
    );
}

/// Re-render trigger returned by the mutating endpoints: reloads the full-day
/// list (its container) with the current search/filter still applied.
const RELOAD_LIST: &str = "<div hx-get=\"/pages/today-list\" hx-trigger=\"load\" hx-target=\"#today-full\" hx-swap=\"innerHTML\" hx-include=\"#search-form\"></div>";

/// Delete a detection and re-render the list.
async fn delete_detection(
    State(state): State<AppState>,
    Form(form): Form<DeleteForm>,
) -> impl IntoResponse {
    let date = form.date;
    let time = form.time;
    let sci_name = form.sci_name;

    // `state.delete_detection`, not `with_db(delete_detection)`: the analytics
    // copy is incremental and can never notice a removal on its own, so the
    // deletion has to be mirrored at the same moment.
    let _ =
        tokio::task::spawn_blocking(move || state.delete_detection(&date, &time, &sci_name)).await;

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html")],
        RELOAD_LIST.to_string(),
    )
}

/// Re-label a detection and re-render the list.
async fn relabel_detection(
    State(state): State<AppState>,
    Form(form): Form<RelabelForm>,
) -> impl IntoResponse {
    // Paired write — see `delete_detection` above.
    let _ = tokio::task::spawn_blocking(move || {
        state.relabel_detection(
            &form.date,
            &form.time,
            &form.old_sci_name,
            &form.new_sci_name,
            &form.new_com_name,
        )
    })
    .await;

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html")],
        RELOAD_LIST.to_string(),
    )
}

/// Lock a detection to protect it from disk purge.
async fn lock_detection(
    State(state): State<AppState>,
    Form(form): Form<LockForm>,
) -> impl IntoResponse {
    let _ = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            birdnet_db::sqlite::lock_detection(conn, &form.date, &form.time, &form.sci_name)
        })
    })
    .await;

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html")],
        RELOAD_LIST.to_string(),
    )
}

/// Unlock a detection (allow disk purge again).
async fn unlock_detection(
    State(state): State<AppState>,
    Form(form): Form<LockForm>,
) -> impl IntoResponse {
    let _ = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            birdnet_db::sqlite::unlock_detection(conn, &form.date, &form.time, &form.sci_name)
        })
    })
    .await;

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html")],
        RELOAD_LIST.to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hour_formatting() {
        assert_eq!(fmt_hour(6.35), "6:21");
        assert_eq!(fmt_hour(0.0), "0:00");
        assert_eq!(fmt_hour(19.5), "19:30");
        // Wraps past midnight rather than printing "24:xx".
        assert_eq!(fmt_hour(24.01), "0:01");
    }

    #[test]
    fn duration_formatting() {
        assert_eq!(fmt_duration(8_040), "2h 14m");
        assert_eq!(fmt_duration(2_700), "45m");
        assert_eq!(fmt_duration(3_600), "1h 00m");
    }

    #[test]
    fn human_date_shape() {
        let d = human_date();
        // "Friday, June 13" — weekday, comma, month, space, day-of-month.
        assert!(d.contains(", "), "missing comma: {d}");
        assert!(d.split_whitespace().count() >= 3, "too short: {d}");
    }

    #[test]
    fn firstrun_checklist_reflects_missing_microphone() {
        let html = firstrun_checklist(&[], Some(38.0), None);
        assert!(html.contains("Waiting for a microphone"));
        assert!(html.contains("38% used"));
        // Honest waiting mark, not a fake checkmark.
        assert!(html.contains("mk wait"));
    }

    fn src(id: &str) -> birdnet_db::audio_sources::AudioSource {
        birdnet_db::audio_sources::AudioSource {
            id: id.to_string(),
            kind: birdnet_db::audio_sources::SourceKind::UsbAlsa,
            device_id: "plughw:CARD=PRO,DEV=0".to_string(),
            label: None,
            sample_rate: 48_000,
            channels: birdnet_db::audio_sources::Channels::Mono,
            bit_depth: 24,
            gain_db: 0.0,
            rtsp_transport: birdnet_db::audio_sources::RtspTransport::Auto,
            schedule_quiet: None,
            pipeline: birdnet_db::audio_sources::PipelineFlags::default(),
            disabled_at: None,
            created_at: "2026-08-11".to_string(),
            updated_at: "2026-08-11".to_string(),
        }
    }

    /// A configured microphone that is not actually capturing is the silent
    /// failure this card exists to rule out. It used to tick as "detected"
    /// purely for being a row in the table.
    #[test]
    fn firstrun_checklist_flags_a_configured_but_silent_microphone() {
        let s = src("src_1");
        let html = firstrun_checklist(&[&s], Some(38.0), Some(false));
        assert!(html.contains("Microphone not recording"), "{html}");
        assert!(html.contains("mk down"), "must not show a pass mark");
        assert!(
            html.contains("/station/capture"),
            "must point at where to fix it"
        );
    }

    #[test]
    fn firstrun_checklist_ticks_a_microphone_that_is_actually_recording() {
        let s = src("src_1");
        let html = firstrun_checklist(&[&s], Some(38.0), Some(true));
        assert!(html.contains("Microphone recording"));
        assert!(html.contains("mk done"));
    }

    #[test]
    fn firstrun_checklist_waits_when_the_gauge_has_no_opinion_yet() {
        // No gauge published is "not known", not "broken" — the supervisor may
        // simply not have reconciled the source yet.
        let s = src("src_1");
        let html = firstrun_checklist(&[&s], Some(38.0), None);
        assert!(html.contains("Microphone starting…"), "{html}");
        assert!(!html.contains("mk down"));
    }

    /// The disk row was a hard-coded tick, so a station at 97 % showed
    /// "Room to record ✓ — nearly full". A green mark reads as "fine".
    #[test]
    fn firstrun_checklist_does_not_tick_a_nearly_full_disk() {
        let html = firstrun_checklist(&[], Some(97.0), None);
        assert!(html.contains("97% used"));
        assert!(html.contains("nearly full"), "{html}");
        let disk_row = html
            .split("Room to record")
            .next()
            .expect("row present")
            .rsplit("<div class=\"x-check-row\">")
            .next()
            .expect("row start");
        assert!(
            !disk_row.contains("mk done"),
            "a nearly-full disk must not carry a pass mark: {disk_row}"
        );
    }

    #[test]
    fn firstrun_checklist_ticks_a_healthy_disk() {
        let html = firstrun_checklist(&[], Some(12.0), None);
        assert!(html.contains("plenty of space"));
        assert!(html.contains("12% used"));
    }

    /// The model row asserted runtime state ("Model loaded … ready") the page
    /// has no signal for. It now states only what is true: the model ships.
    #[test]
    fn firstrun_checklist_does_not_claim_the_model_loaded() {
        let html = firstrun_checklist(&[], Some(12.0), None);
        assert!(!html.contains("Model loaded"), "{html}");
        assert!(html.contains("Model bundled"));
    }

    /// The signal card's footer hard-coded `input · mic` and `48 kHz` for every
    /// station, directly under a live spectrogram of that station's own audio.
    #[test]
    fn signal_card_footer_describes_the_real_source() {
        let mut s = src("src_1");
        s.sample_rate = 16_000;
        s.kind = birdnet_db::audio_sources::SourceKind::Rtsp;
        let r = &s;
        let html = signal_card("", Some(&r));
        assert!(html.contains("16 kHz"), "{html}");
        assert!(!html.contains("48 kHz"));
        assert!(!html.contains("input · mic"), "an RTSP source is not a mic");
    }

    #[test]
    fn signal_card_footer_is_blank_without_a_source() {
        let html = signal_card("", None);
        assert!(html.contains("input · none"));
        assert!(!html.contains("48 kHz"));
    }
}
