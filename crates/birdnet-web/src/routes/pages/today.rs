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

use crate::routes::form::HtmxForm;
use axum::extract::{Query, State};
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
        .route("/pages/today-songs", get(super::today_songs::today_songs))
        .route("/pages/today-count", get(today_count_partial))
        .route("/pages/today-pills", get(today_pills_partial))
        .route("/pages/today-nudge", get(today_nudge_partial))
}

/// The state-changing half, mounted behind the admin auth middleware.
///
/// These delete records, rename them and pin them against the purge. They were
/// in [`router`] — that is, in the *public* router — until it was noticed that
/// "viewing is open, only `/admin` needs a login" was not true of them.
pub fn mutating_router() -> Router<AppState> {
    Router::new()
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
    // `total_ever` decides `firstrun`, and `firstrun` decides which page this
    // is: the setup checklist and "Let's get you listening", or the dashboard.
    // Defaulted to 0, a database that could not be read did not merely print a
    // wrong number — it replaced a station with years of records with the
    // first-run experience, and told its owner to go and set up a microphone.
    let state_for_query = state.clone();
    let loaded = tokio::task::spawn_blocking(move || {
        // First run: a station with no detections that hasn't completed
        // onboarding is bounced to the setup wizard instead of an empty home.
        if first_run_needs_onboarding(&state_for_query) {
            return Ok(None);
        }
        let counts = state_for_query.with_read_db(|conn| {
            use birdnet_db::audio_sources::AudioSourceStore;
            let total = birdnet_db::sqlite::detection_count(conn).map_err(|e| e.to_string())?;
            let sources = AudioSourceStore::list(conn).map_err(|e| e.to_string())?;
            Ok::<_, String>((total, sources))
        });
        let disk_pct = disk_used_percent(&state_for_query);
        // Decoration, so a failed read is the default layout rather than an
        // error page: every block is still the station's own data.
        let layout = state_for_query.with_read_db(|conn| {
            birdnet_db::settings::get_or(conn, super::today_layout::SETTING_KEY, "")
        });
        let layout = layout.map_or_else(
            |e| {
                tracing::warn!(error = %e, "today: layout setting unreadable; using the default");
                super::today_layout::Layout::default()
            },
            |v| super::today_layout::Layout::parse(&v),
        );
        // Read only for the first-run checklist, which is the only thing that
        // shows it.
        let password_set = matches!(counts, Ok((0, _)))
            && crate::auth_middleware::admin_password_configured(&state_for_query);
        let has_location = station_has_location(&state_for_query);
        // Likewise read only for the checklist: what the microphone is hearing.
        let heard_dbfs = if matches!(counts, Ok((0, _))) {
            recent_noise_floor_dbfs(&state_for_query)
        } else {
            None
        };
        counts.map(|(total, sources)| {
            Some((
                total,
                sources,
                disk_pct,
                password_set,
                layout,
                has_location,
                heard_dbfs,
            ))
        })
    })
    .await;

    let (total_ever, sources, disk_pct, password_set, layout, has_location, heard_dbfs) =
        match loaded {
            Ok(Ok(Some(v))) => v,
            Ok(Ok(None)) => return Redirect::to("/onboarding").into_response(),
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "today: station totals could not be read");
                return today_error_page(&headers);
            }
            Err(e) => {
                tracing::warn!(error = %e, "today: task failed");
                return today_error_page(&headers);
            }
        };

    let firstrun = total_ever == 0;
    let enabled: Vec<_> = sources.iter().filter(|s| s.disabled_at.is_none()).collect();

    // Whether each configured source is *actually capturing*, from the
    // supervisor's own gauge — the same signal the Capture tab's status pill
    // reads. A source being configured says nothing about audio flowing.
    let gauges: Vec<Option<bool>> = enabled
        .iter()
        .map(|s| state.metrics().source_up(&s.id))
        .collect();

    let hero_aside = if firstrun {
        let model_loaded = state.detection_daemon_running();
        firstrun_checklist(
            &enabled,
            disk_pct,
            &gauges,
            password_set,
            model_loaded,
            has_location,
            heard_dbfs,
        )
    } else {
        signal_card(&super::listen::source_options(&sources), enabled.first())
    };
    let rail_extra = if firstrun {
        r#"<div class="bnb-card pad"><div class="bnb-eyebrow td-look-eb">While you wait</div><div class="x-look"><a href="/admin/audio">Add a second microphone <span class="arr">→</span></a><a href="/admin/rules">Set up rare-bird alerts <span class="arr">→</span></a><a href="/admin/migrate">Import your BirdNET-Pi history <span class="arr">→</span></a></div></div>"#
            .to_string()
    } else {
        String::new()
    };
    let (hero_headline, hero_sub) = if firstrun {
        (FIRSTRUN_HEADLINE, FIRSTRUN_SUB)
    } else {
        // A placeholder until the phrase partial answers: it said "You're
        // listening." on stations with no microphone at all.
        ("Today at the station.", "Counting today's detections…")
    };

    // A brand-new station keeps the default: its hero is the setup checklist.
    let layout = if firstrun {
        super::today_layout::Layout::default()
    } else {
        layout
    };
    let arranged = layout
        .apply(TODAY_PAGE_HTML.as_str())
        .unwrap_or_else(|missing| {
            tracing::error!(
                missing,
                "today: the template lost a layout marker; rendering the default"
            );
            super::today_layout::Layout::default()
                .apply(TODAY_PAGE_HTML.as_str())
                .unwrap_or_else(|_| TODAY_PAGE_HTML.replace("{{work2_solo}}", ""))
        });
    // The owner's picture: empty until one is set on the Settings page.
    let picture = crate::routes::dashboard_image::block_for(&state).await;
    let body = arranged
        .replace("{{picture_block}}", &picture)
        .replace("{{firstrun}}", if firstrun { "true" } else { "false" })
        .replace("{{today_human_date}}", &human_date())
        .replace("{{hero_headline}}", hero_headline)
        .replace("{{hero_sub}}", hero_sub)
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

/// The dashboard when the station's own totals could not be read.
///
/// Rendered through the same shell as the page it replaces, so the reader
/// keeps the navigation and can reach Station health from it. It is emphatically
/// *not* the first-run experience, which is what a defaulted count produced.
fn today_error_page(headers: &HeaderMap) -> Response {
    super::render_page_for_request(
        "Today",
        &super::error_states::could_not_load("your dashboard"),
        "today",
        headers,
    )
    .into_response()
}

/// The headline a brand-new station wakes up with, inside Today's static `<h1>`.
pub(super) const FIRSTRUN_HEADLINE: &str =
    r#"Your station is <em class="tp-c-moss-ink">waking up</em>."#;

/// The line under the first-run headline.
pub(super) const FIRSTRUN_SUB: &str = "Everything checks out — we're listening for the first call. It usually arrives within the hour, and this page comes alive the moment it does.";

/// The live-signal card (real spectrogram canvas + the source row).
///
/// `first` is the station's first enabled source, used for the footer's
/// input/rate line. That line used to read a hard-coded `input · mic` and
/// `48 kHz` for every station — so an RTSP-only station was labelled "mic",
/// and a 16 or 44.1 kHz source was reported as 48 kHz directly beneath a live
/// spectrogram of its own audio.
/// A sample rate as a person reads it: `48 kHz`, `44.1 kHz`, `22.05 kHz`.
/// Integer division printed a 44.1 kHz source as "44 kHz".
fn khz(rate: u32) -> String {
    let khz = f64::from(rate) / 1000.0;
    let text = format!("{khz:.2}");
    format!("{} kHz", text.trim_end_matches('0').trim_end_matches('.'))
}

fn signal_card(
    source_options: &str,
    first: Option<&&birdnet_db::audio_sources::AudioSource>,
) -> String {
    let (input_label, rate) = first.map_or_else(
        || ("input · none".to_string(), "—".to_string()),
        |s| {
            (
                // `as_str` is the storage form (`usb-alsa`), which has a
                // matching `FromStr` and belongs in the database, not under a
                // reader's eyes. `kind_label` is the display name.
                format!(
                    "input · {}",
                    crate::routes::admin::audio::kind_label(s.kind)
                ),
                khz(s.sample_rate),
            )
        },
    );
    format!(
        r#"<div class="bnb-card pad db-signal-card">
      <div class="db-signal-head">
        <span class="bnb-eyebrow">Live signal · last 30 s</span>
        <span class="bnb-pill db-live-pill"><span class="bnb-dot"></span> idle</span>
      </div>
      <canvas id="hero-pulse" height="80" class="db-pulse" role="img"
              aria-label="Live audio spectrogram for the selected source"></canvas>
      <p class="bnb-meta db-signal-note" id="hero-pulse-note" aria-live="polite">Waiting for the first audio segment…</p>
      <div class="db-signal-foot">
        <span class="mono bnb-meta">{input_label}</span>
        <span class="mono bnb-meta">{rate}</span>
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
/// A measured noise floor at or below this, in dBFS, is a microphone that hears
/// nothing.
///
/// One least-significant bit of a 16-bit sample is -90.3 dBFS, so a floor here
/// is a converter putting out zeros or its last bit: a capture switched off, a
/// gain at the bottom, an input with nothing plugged in. The sampler reports
/// digital silence as -96 (`birdnet_core::audio::quality`), and a real room,
/// even a quiet one at night, sits tens of dB above this.
const SILENT_FLOOR_DBFS: f64 = -90.0;

/// The loudest noise floor any source reported in the station's current or
/// previous local hour, from `audio_levels` (the acoustic-health sampler's
/// table, written every five minutes by the daemon). `None` when there is no
/// such reading: a web-only process, a station that has just started, or a
/// failed read, which is logged and claims nothing.
fn recent_noise_floor_dbfs(state: &AppState) -> Option<f64> {
    let rows = state.with_read_db(|conn| birdnet_db::audio_levels::recent_hours(conn, 48));
    let rows = rows
        .inspect_err(|e| tracing::warn!(error = %e, "today: audio levels unreadable"))
        .ok()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
    loudest_recent_floor(&rows, now + birdnet_db::clock::local_utc_offset_secs())
}

/// The newest bucket of each source within the hour of `now_local_secs` (local
/// wall-clock seconds) and the hour before it, and of those the loudest floor.
///
/// The loudest, so "silent" is said only when every measured source is; the
/// last two hours, so a reading left from an earlier run is not taken for this
/// one. `rows` is newest first, as `recent_hours` returns it.
fn loudest_recent_floor(
    rows: &[birdnet_db::audio_levels::HourlyLevel],
    now_local_secs: i64,
) -> Option<f64> {
    let key = |secs: i64| {
        let c = birdnet_core::civil::civil_from_unix_secs(secs);
        (
            format!("{:04}-{:02}-{:02}", c.year, c.month, c.day),
            u8::try_from(c.hour).unwrap_or(0),
        )
    };
    let recent = [key(now_local_secs), key(now_local_secs - 3600)];
    let mut seen: Vec<&str> = Vec::new();
    let mut loudest: Option<f64> = None;
    for row in rows {
        if seen.contains(&row.source.as_str())
            || !recent.iter().any(|(d, h)| *d == row.date && *h == row.hour)
        {
            continue;
        }
        seen.push(&row.source);
        loudest = Some(loudest.map_or(row.noise_floor_dbfs, |l| l.max(row.noise_floor_dbfs)));
    }
    loudest
}

fn firstrun_checklist(
    enabled: &[&birdnet_db::audio_sources::AudioSource],
    disk_pct: Option<f64>,
    gauges: &[Option<bool>],
    password_set: bool,
    model_loaded: bool,
    has_location: bool,
    heard_dbfs: Option<f64>,
) -> String {
    // Every configured source counts, not only the first: one live
    // microphone beside two dead cameras ticked "Microphone recording" under
    // a header that said a source had stopped (UIR-11).
    let up = gauges.iter().filter(|g| **g == Some(true)).count();
    let capturing = if enabled.is_empty() || up == enabled.len() {
        gauges.first().copied().flatten()
    } else if up > 0 {
        None
    } else if gauges.contains(&Some(false)) {
        Some(false)
    } else {
        None
    };
    // OOB-02: the row that decides which birds the station may name. Without
    // it the station said so only on `/station`, once the wizard redirect had
    // stopped at the first detection.
    let location_row = if has_location {
        r#"<div class="x-check-row"><span class="mk done">✓</span><div class="c"><div class="t">Location set</div><div class="d">used for sunrise, sunset and the birds expected near you</div></div><span class="v">set</span></div>"#
    } else {
        r#"<div class="x-check-row"><span class="mk down">!</span><div class="c"><div class="t">Location not set</div><div class="d">the station may name birds that don't live near you — set it under <a href="/station/capture#set-location">Settings → Capture</a></div></div><span class="v">unset</span></div>"#
    };
    // The model row used to be a hard-coded tick (ON-10): "Model bundled …
    // included" on a process with no model at all. The detection daemon's
    // liveness flag is the stronger predicate — it is true only while the
    // daemon that loaded the model is running.
    let model_row = if model_loaded {
        r#"<div class="x-check-row"><span class="mk done">✓</span><div class="c"><div class="t">Model loaded</div><div class="d">the detector is running</div></div><span class="v">running</span></div>"#
    } else {
        r#"<div class="x-check-row"><span class="mk down">!</span><div class="c"><div class="t">Detector not running</div><div class="d">the bird-identifying model isn't running — <a href="/admin/doctor">Diagnostics</a> says why</div></div><span class="v">stopped</span></div>"#
    };
    // The one row that is about who can change the station rather than
    // whether it hears anything (DD-14). Until a password exists every
    // `/admin/*` page is open to anyone on the network, and nothing on the
    // first screen said so.
    let admin_row = if password_set {
        r#"<div class="x-check-row"><span class="mk done">✓</span><div class="c"><div class="t">Admin password</div><div class="d">settings are yours to change</div></div><span class="v">set</span></div>"#.to_string()
    } else {
        r#"<div class="x-check-row"><span class="mk down">!</span><div class="c"><div class="t">No admin password</div><div class="d">anyone on the network can change settings — set one in the <a href="/onboarding">setup wizard</a> or <a href="/station/access">Settings → Access</a></div></div><span class="v">open</span></div>"#.to_string()
    };
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
            let label = crate::routes::admin::audio::public_name(first);
            let more = if enabled.len() > 1 {
                format!(" +{}", enabled.len() - 1)
            } else {
                String::new()
            };
            let source = format!(
                "{} · {}{more}",
                crate::routes::admin::audio::kind_label(first.kind),
                escape_html(&label)
            );
            if up > 0 && up < enabled.len() {
                let stopped: Vec<String> = enabled
                    .iter()
                    .zip(gauges.iter().chain(std::iter::repeat(&None)))
                    .filter(|(_, g)| **g != Some(true))
                    .map(|(s, _)| escape_html(&crate::routes::admin::audio::public_name(s)))
                    .collect();
                let of = enabled.len();
                return (
                    "fail",
                    format!("{up} of {of} sources recording"),
                    format!(
                        "no audio from {} — check under <a href=\"/station/capture\">Settings → Capture</a>",
                        stopped.join(", ")
                    ),
                    format!("{up} of {of}"),
                );
            }
            let silent = heard_dbfs.filter(|d| *d <= SILENT_FLOOR_DBFS);
            match capturing {
                // The process is up, and the sampler says the audio carries
                // nothing (OOB-05): a muted capture switch or a gain at zero
                // looked exactly like a working microphone here.
                Some(true) if silent.is_some() => (
                    "fail",
                    "Silent — check the microphone isn't muted".to_string(),
                    format!(
                        "{source} — recording, but the audio carries nothing: a capture switch \
                         off, a gain at zero, or nothing plugged in. Check it under \
                         <a href=\"/station/capture\">Settings → Capture</a>"
                    ),
                    silent.map_or_else(String::new, |d| format!("{d:.0} dBFS")),
                ),
                // Configured *and* the supervisor reports it up.
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
    // The station listens only when the two rows that make it listen pass. A
    // live "Listening for the first call…" under "Microphone not recording"
    // promised what the rows above had just withdrawn.
    let partial = up > 0 && up < enabled.len();
    let listening_row = if (mic_mark == "done" || partial) && model_loaded {
        r#"<div class="x-check-row"><span class="mk wait"><span class="bnb-dot live"></span></span><div class="c"><div class="t">Listening for the first call…</div><div class="d">this can take a few minutes</div></div><span class="v">—</span></div>"#
    } else {
        r#"<div class="x-check-row"><span class="mk wait"><span class="bnb-dot"></span></span><div class="c"><div class="t">Not listening yet</div><div class="d">starts once the microphone and detector above are ready</div></div><span class="v">—</span></div>"#
    };
    format!(
        r#"<div class="bnb-card pad">
      <div class="bnb-eyebrow td-check-eb">Getting ready</div>
      <div class="x-check">
        {admin_row}
        {location_row}
        <div class="x-check-row">{mic_mark_html}<div class="c"><div class="t">{mic_title}</div><div class="d">{mic_detail}</div></div><span class="v">{mic_value}</span></div>
        {model_row}
        <div class="x-check-row">{disk_mark_html}<div class="c"><div class="t">Room to record</div><div class="d">{disk_detail}</div></div><span class="v">{disk_value}</span></div>
        {listening_row}
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
    state.with_read_db(|conn| {
        let onboarded = birdnet_db::settings::get_or(conn, "onboarding_complete", "false")
            .map_or(true, |v| v == "true");
        let has_detections = birdnet_db::sqlite::detection_count(conn).map_or(true, |n| n > 0);
        !onboarded && !has_detections && !location_is_set(conn)
    })
}

/// Whether both coordinates are set. A read that fails counts as set: this
/// decides a redirect and a warning, and neither should fire on a hiccup.
fn location_is_set(conn: &rusqlite::Connection) -> bool {
    let read = |key| birdnet_db::settings::get_or(conn, key, "").map(|v| !v.trim().is_empty());
    read("latitude").unwrap_or(true) && read("longitude").unwrap_or(true)
}

/// [`location_is_set`] through the read pool.
fn station_has_location(state: &AppState) -> bool {
    state.with_read_db(location_is_set)
}

/// Where a model notice sends its reader, and what its button says: the
/// Capture tab's install button when it fixes the condition, the location
/// form for a filter that has no position, and otherwise Station health,
/// which lists every condition beside the models' own cards.
fn model_notice_action(
    condition: &crate::model_conditions::ModelCondition,
    setup: &crate::classifier_setup::Snapshot,
) -> (&'static str, &'static str) {
    if setup.installs_fix(&condition.key) {
        (
            "/station/capture#classifier",
            if setup.repairs() {
                "Repair it"
            } else {
                "Install it"
            },
        )
    } else if condition.state == "off_no_coordinates" {
        ("/station/capture#set-location", "Set your location")
    } else {
        ("/station", "Open Station health")
    }
}

/// Warnings that stand on Today for as long as their cause does, whatever
/// else the strip under the hero is showing.
///
/// `clock_synced` is the system's own answer, as station health last read it:
/// `None` (a container, or not asked yet) says nothing rather than guess.
///
/// `models` come first: a station that is not identifying birds, or is
/// identifying them without a classifier or filter its configuration names,
/// is the one thing on this page that makes every other line on it wrong
/// (MDL-03, MDL-05, MDL-06).
fn standing_notices(
    models: &[crate::model_conditions::ModelCondition],
    setup: &crate::classifier_setup::Snapshot,
    has_location: bool,
    clock_synced: Option<bool>,
) -> String {
    let mut out = String::new();
    for condition in models {
        let (href, action) = model_notice_action(condition, setup);
        let _ = write!(
            out,
            r#"<div class="x-nudge" data-screen-label="Models banner" data-announce="{title}."><span class="ico">⚠</span><div class="txt"><b>{title}.</b> {body}</div><a class="bnb-btn primary" href="{href}">{action} →</a></div>"#,
            title = super::escape_html(&condition.title),
            body = super::escape_html(&condition.body()),
        );
    }
    if !has_location {
        out.push_str(
            r#"<div class="x-nudge" data-screen-label="Location banner" data-announce="This station does not know where it is."><span class="ico">⚠</span><div class="txt"><b>Your station does not know where it is,</b> so it may name birds that don't live near you.</div><a class="bnb-btn primary" href="/station/capture#set-location">Set your location →</a></div>"#,
        );
    }
    if clock_synced == Some(false) {
        out.push_str(
            r#"<div class="x-nudge" data-screen-label="Clock banner" data-announce="The station can't check the time, so new detections may be filed at the wrong time."><span class="ico">⚠</span><div class="txt"><b>The station can't check the time.</b> Its clock is not synchronised with a time server, usually because it has no internet connection, so new detections may be filed at the wrong time.</div><a class="bnb-btn primary" href="/station">Open Station health →</a></div>"#,
        );
    }
    out
}

// ---------------------------------------------------------------------------
// Shared context helpers (solar, weather, freshness)
// ---------------------------------------------------------------------------

/// Capture-outage check: the station has detected before, the silence exceeds
/// the threshold, and (with a location) we're inside daylight, when silence
/// is anomalous. `Ok(Some((silence, last)))` is the duration of the silence and
/// the last detection's time; `Ok(None)` is "no outage".
///
/// `Err` when the detection history could not be read. That is a third answer,
/// not `Ok(None)`: the pills drew "recording" in green for a station whose
/// freshness they had not been able to look at, and the outage banner, which is
/// "absent entirely when nothing waits", was absent for the same reason.
pub(super) fn capture_outage(
    conn: &rusqlite::Connection,
) -> Result<Option<(u64, String)>, birdnet_db::sqlite::DbError> {
    use rusqlite::OptionalExtension as _;
    let Some(silent) = birdnet_db::sqlite::seconds_since_last_detection(conn)? else {
        return Ok(None);
    };
    let threshold = match super::solar_times_local(conn) {
        Some((sunrise, sunset)) => {
            let now = super::now_hour_local();
            if now < sunrise || now > sunset {
                return Ok(None); // overnight silence is normal
            }
            OUTAGE_DAYTIME_SECS
        }
        None => OUTAGE_NO_LOCATION_SECS,
    };
    if silent < threshold {
        return Ok(None);
    }
    let last = conn
        .query_row(
            "SELECT Time FROM detections ORDER BY Date DESC, Time DESC LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    Ok(last.map(|last| (silent, last.get(0..5).unwrap_or(&last).to_string())))
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
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
    human_date_at(secs, super::local_utc_offset_secs())
}

/// [`human_date`] for a given instant and UTC offset.
///
/// The station's **local** date. This divided UTC seconds by a day, so on a
/// UTC+10 station the hero read yesterday's date until ten in the morning,
/// above a feed of today's birds.
fn human_date_at(unix_secs: i64, offset_secs: i64) -> String {
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
    let local_days = (unix_secs + offset_secs).div_euclid(86_400);
    let epoch_days = u64::try_from(local_days).unwrap_or(0);
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
    /// Every enabled source is up, as the supervisor reports it.
    Up,
    /// Some enabled sources are up and the rest are not: `up` of `of`.
    Partial {
        /// Sources the supervisor reports up.
        up: usize,
        /// Enabled sources.
        of: usize,
    },
    /// Sources are configured and none is up.
    Down,
    /// Sources are configured, the detection daemon is running, and no gauge
    /// has been published yet (the supervisor has not reconciled them, e.g.
    /// in the first seconds after a start). Not an outage — just not known.
    /// A process with no running daemon and no gauge is [`Self::Down`].
    Unknown,
    /// The list of audio sources could not be read, so nothing is known about
    /// them. Not [`Self::NoSource`]: that says the station has no microphone,
    /// in the colour of a fault, to an operator whose microphone may be fine.
    Unreadable,
}

/// Resolve [`CaptureState`] from the same per-source gauge the Capture tab's
/// status pill reads, so the dashboard and that page cannot disagree.
pub(super) fn live_capture_state(state: &AppState) -> CaptureState {
    use birdnet_db::audio_sources::AudioSourceStore;
    let sources = match state.with_read_db(AudioSourceStore::list) {
        Ok(sources) => sources,
        Err(e) => {
            tracing::warn!(error = %e, "capture state: the audio sources could not be listed");
            return CaptureState::Unreadable;
        }
    };
    if sources.is_empty() {
        return CaptureState::NoSource;
    }
    let gauges: Vec<Option<bool>> = sources
        .iter()
        .map(|s| state.metrics().source_up(&s.id))
        .collect();
    let up = gauges.iter().filter(|g| **g == Some(true)).count();
    if up == gauges.len() {
        CaptureState::Up
    } else if up > 0 {
        // Every source the station has, not only the first: one live
        // microphone beside two dead cameras is not "recording" (UIR-11).
        CaptureState::Partial {
            up,
            of: gauges.len(),
        }
    } else if gauges.iter().all(Option::is_none) {
        // No gauge was ever published. With the detection daemon running that
        // is the first seconds after a start, before the supervisor has
        // reconciled the sources: not known yet, not an outage. With no
        // running daemon nothing will ever publish one — a source added while
        // the daemon is down, a thread that never started, a `--web-only`
        // process — and this used to read as `Unknown` and grade "Healthy"
        // while `/api/v2/health?strict=1` on the same process said the daemon
        // was stopped. The flag is set before the listener binds and cleared
        // by the loop thread on exit, so this is not a boot-time flash. A
        // gauge that *does* say up is believed even with the flag clear: the
        // capture supervisor is not the detection loop, and a dead detector
        // under a live microphone is `?strict=1`'s to report, not this pill's.
        if state.detection_daemon_running() {
            CaptureState::Unknown
        } else {
            CaptureState::Down
        }
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
    let html = tokio::task::spawn_blocking(move || {
        let capture = live_capture_state(&state);
        state.with_read_db(|conn| {
            let mut out = String::with_capacity(512);

            // Recording state. The gauge is authoritative when it has an
            // opinion; otherwise fall back to the detection-freshness signal
            // that drives the outage banner, so the two cannot disagree.
            let outage = capture_outage(conn);
            if let Err(e) = &outage {
                tracing::warn!(error = %e, "recording pill: the detection history could not be read");
            }
            match (capture, outage) {
                (CaptureState::NoSource, _) => out.push_str(
                    r#"<span class="bnb-pill rare"><span class="bnb-dot"></span> not recording · no microphone configured</span>"#,
                ),
                (CaptureState::Down, _) => out.push_str(
                    r#"<span class="bnb-pill rare"><span class="bnb-dot"></span> not recording · the microphone has stopped</span>"#,
                ),
                // Neither green nor red: a pill that could not look says so.
                (CaptureState::Unreadable, _) => out.push_str(
                    r#"<span class="bnb-pill"><span class="bnb-dot"></span> recording state unknown · couldn't read the audio sources</span>"#,
                ),
                (_, Ok(Some((_, last)))) => {
                    let _ = write!(
                        out,
                        r#"<span class="bnb-pill rare"><span class="bnb-dot"></span> recording stopped · last heard {last}</span>"#
                    );
                }
                // Some sources up and some not: amber, with the count, and a
                // link to the page that names which (UIR-11). Green said
                // "recording" beside a header that said a source had stopped.
                (CaptureState::Partial { up, of }, _) => {
                    let _ = write!(
                        out,
                        r#"<a class="bnb-pill dawn" href="/station"><span class="bnb-dot live"></span> recording · {up} of {of} sources</a>"#
                    );
                }
                // The gauge says it is up, which does not depend on the detection
                // history; the freshness read only ever contradicts a gauge. The
                // same pill when the history reads fine and shows no outage.
                (CaptureState::Up, Err(_)) | (_, Ok(None)) => out.push_str(
                    r#"<span class="bnb-pill moss"><span class="bnb-dot live"></span> recording</span>"#,
                ),
                // Nothing says it is up and the one thing that could say it is
                // down could not be read: a pill that could not look says so.
                (_, Err(_)) => out.push_str(
                    r#"<span class="bnb-pill"><span class="bnb-dot"></span> recording state unknown · couldn't read the detection history</span>"#,
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
                out.push_str(&weather_pill(now_t, max, min));
            }

            // Sunrise / sunset from the configured location.
            if let Some((sunrise, sunset)) = super::solar_times_local(conn) {
                let _ = write!(
                    out,
                    r#"<span class="bnb-pill">☀ sunrise {}</span><span class="bnb-pill">☾ sunset {}</span>"#,
                    crate::display::clock_from_hours(sunrise),
                    crate::display::clock_from_hours(sunset)
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
        let clock_synced = state.metrics().clock_synced();
        let models = crate::model_conditions::conditions(&state.models_state());
        let setup = crate::routes::admin::classifier::snapshot_of(&state);
        state.with_read_db(|conn| {
            let notices = standing_notices(&models, &setup, location_is_set(conn), clock_synced);
            // Not defaulted: this strip is "absent entirely when nothing waits",
            // so a count that fell to zero on a failed read hid the one prompt
            // that asks the operator to act.
            let pending = birdnet_db::sqlite::quarantine_pending_count(conn);
            if let Ok(pending) = pending
                && pending > 0
            {
                let (noun, verb, obj) = if pending == 1 {
                    ("rare sighting is", "it's", "it")
                } else {
                    ("rare sightings are", "they're", "them")
                };
                return format!(
                    r#"<div class="x-nudge" data-screen-label="Review nudge" data-announce="{pending} {noun} waiting for review."><span class="ico">✦</span><div class="txt"><b>{pending} {noun} waiting for your eye.</b> Confirm {verb} real to add {obj} to your records.</div><a class="bnb-btn primary" href="/quarantine">Review →</a></div>{notices}"#
                );
            }
            let mut strip = String::new();
            let outage = capture_outage(conn);
            if let Err(e) = &outage {
                tracing::warn!(error = %e, "outage banner: the detection history could not be read");
                strip.push_str(&super::error_states::inline(
                    "whether the microphone has gone quiet",
                ));
            }
            if let Ok(Some((silent, last))) = outage {
                let dur = fmt_duration(silent);
                let last = crate::display::clock(&last);
                let _ = write!(
                    strip,
                    r#"<div class="x-nudge" data-screen-label="Outage banner" data-announce="No detections recently: the microphone may be unplugged or the recorder stopped."><span class="ico">⚠</span><div class="txt"><b>No detections for {dur}.</b> The last one was at <span class="mono">{last}</span> — the microphone may be unplugged or the recorder stopped.</div><a class="bnb-btn primary" href="/station">Open Settings →</a></div>"#
                );
            }
            if let Err(e) = pending {
                tracing::warn!(error = %e, "review nudge: the sightings waiting could not be counted");
                strip.push_str(&super::error_states::inline(
                    "the list of sightings waiting for review",
                ));
            }
            strip.push_str(&notices);
            strip
        })
    })
    .await
    .unwrap_or_else(|e| {
        tracing::warn!(error = %e, "review nudge: task failed");
        super::error_states::inline("the list of sightings waiting for review")
    });

    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
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
    /// The row's clip (`""` for none), so the one source's row is relabelled
    /// and not every source's; absent from older callers.
    #[serde(default)]
    pub file_name: Option<String>,
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
    let names = state.names_handle();

    let result = tokio::task::spawn_blocking(move || {
        state.with_read_db(|conn| {
            let filter = super::name_search::todays_filter(
                &today,
                search.as_deref(),
                TodayFilter::All,
                &names,
            );
            birdnet_db::sqlite::search_detection_count(conn, &filter)
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
        // The one partial that must NOT use `error_states::failed_partial`.
        // Its target is `#td-total`, a `<span>` holding a number inside the
        // "Show the full day (34) — search, filter, lock & delete" button, so
        // the shared renderer's `<p>` is not legal content there — and neither
        // is what the page did instead. Driven to 500 in a browser, the
        // `layout.html` fallback put a `<div role="alert">` and an `<a href>`
        // *inside the `<button>`*, giving "Show the full day (This section
        // could not load (HTTP 500). Reload the page) — search, filter…" and
        // an anchor nested in a button: interactive content inside interactive
        // content, which no keyboard or screen-reader user can resolve. The
        // accessibility sweep never saw it, because it grades the page in the
        // state it happened to be in.
        //
        // So: phrasing content only, and a mark that claims nothing. `?` is
        // not a count; an em dash would read as "none", which is the lie this
        // release has been taking out of the rest of the app.
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "today count: query failed");
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/html")],
                COUNT_UNKNOWN.to_string(),
            )
        }
        Err(e) => {
            tracing::warn!(error = %e, "today count: task failed");
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/html")],
                COUNT_UNKNOWN.to_string(),
            )
        }
    }
}

/// What `#td-total` shows when the count could not be read.
///
/// Phrasing content, because it lands inside a `<button>`'s label. The
/// `sr-only` half is the whole explanation a screen-reader user gets, since
/// `?` on its own is not one.
const COUNT_UNKNOWN: &str = r#"?<span class="sr-only"> — this count could not be loaded</span>"#;

/// What the full-day list says when it has nothing to show.
///
/// "No detections found today." whatever was asked, so choosing *Rare* on a
/// day with three hundred detections told the reader the yard had been silent.
/// A narrowed list says what it was narrowed by, and how to widen it.
fn empty_day_message(search: Option<&str>, filter: TodayFilter) -> String {
    let search = search.map(str::trim).filter(|s| !s.is_empty());
    let narrowed = match filter {
        TodayFilter::All => None,
        TodayFilter::FirstToday => Some("new species"),
        TodayFilter::Rare => Some("rare birds"),
        TodayFilter::HighConfidence => Some("very sure detections"),
    };
    match (search, narrowed) {
        (None, None) => r#"<p class="tdl-empty">No detections yet today.</p>"#.to_owned(),
        (Some(q), None) => format!(
            r#"<p class="tdl-empty">Nothing heard today matches “{}”. Clear the search to see the whole day.</p>"#,
            super::escape_html(q)
        ),
        (None, Some(what)) => format!(
            r#"<p class="tdl-empty">No {what} today. Choose <b>All</b> to see the whole day.</p>"#
        ),
        (Some(q), Some(what)) => format!(
            r#"<p class="tdl-empty">No {what} today match “{}”. Clear the search or choose <b>All</b> to see the whole day.</p>"#,
            super::escape_html(q)
        ),
    }
}

/// HTMX partial: paginated list of today's detections as cards.
async fn today_partial(
    State(state): State<AppState>,
    Query(params): Query<TodayParams>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    // Delete and the lock toggle post to routes behind the sign-in; a visitor
    // was shown both and answered with a redirect to the login page (UIC-24).
    let manage = crate::session::looks_signed_in(&headers);
    let today = today_date_string();
    let limit = params.limit.unwrap_or(40).min(200);
    let offset = params.offset.unwrap_or(0);
    let search = params.search.clone();
    let search2 = params.search.clone();
    let filter = TodayFilter::from_token(params.filter.as_deref());
    // `state` moves into the blocking task; the names are needed after it.
    let names_state = state.clone();

    let result = tokio::task::spawn_blocking(move || {
        state.with_read_db(|conn| {
            // The typed search also covers the station's own language.
            let by =
                super::name_search::todays_filter(&today, search.as_deref(), filter, state.names());
            let rows = birdnet_db::sqlite::search_detections(conn, &by, limit, offset)?;
            let total = birdnet_db::sqlite::search_detection_count(conn, &by)?;
            // Which clips are locked, so each card offers the right toggle.
            let locked: std::collections::HashSet<String> =
                birdnet_db::sqlite::locked_file_names(conn)?
                    .iter()
                    .map(|f| super::recordings::base_name(f))
                    .collect();
            let floor = super::confidence::station_floor(conn);
            Ok::<_, birdnet_db::sqlite::DbError>((rows, total, locked, floor))
        })
    })
    .await;

    match result {
        Ok(Ok((detections, total, locked, floor))) => {
            let mut html = String::with_capacity(4096);

            if detections.is_empty() && offset == 0 {
                html.push_str(&empty_day_message(search2.as_deref(), filter));
                return (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html);
            }

            for d in &detections {
                render_detection_card(&mut html, d, &locked, names_state.names(), floor, manage);
            }

            // "Load more" button if there are more results. It replaces
            // itself with the next page — rows and, if any remain, the next
            // button — so the list grows; it used to swap the next page into
            // the whole list, throwing away what the reader had scrolled past.
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
                    .map_or_default(|s| format!("&search={}", simple_url_encode(s)));
                let filter_param = params
                    .filter
                    .as_ref()
                    .filter(|f| !f.trim().is_empty() && f.as_str() != "all")
                    .map_or_default(|f| format!("&filter={}", simple_url_encode(f)));
                let remaining = total_u.saturating_sub(shown);
                let _ = write!(
                    html,
                    "<div class=\"tdl-more\">\
                     <button hx-get=\"/pages/today-list?offset={shown}&limit={limit}{search_param}{filter_param}\" \
                     hx-target=\"closest .tdl-more\" hx-swap=\"outerHTML\" \
                     class=\"tdl-more-btn\">\
                     Load {limit} more ({remaining} remaining)\
                     </button></div>",
                );
            }

            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        // See `error_states::failed_partial` for why this is a 200.
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "today log: query failed");
            super::error_states::failed_partial("today's detections")
        }
        Err(e) => {
            tracing::warn!(error = %e, "today log: task failed");
            super::error_states::failed_partial("today's detections")
        }
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
    let names = state.names_handle();
    let result = tokio::task::spawn_blocking(move || {
        // The colour each species owns on the station, so the strip agrees with every chart.
        let slots = super::viz::kit::station_slots(&state)?;
        state.with_read_db(|conn| {
            // The hour totals come from the database, not from counting the rows of a
            // page of the day's detections: that page stops at 1000, and a busy station
            // passes it before lunch, so the chart quietly stopped adding up.
            let rows = birdnet_db::sqlite::hourly_activity(conn, &today)?;
            // The species heard most each hour, for the tooltip.
            let by_hour = birdnet_db::sqlite::today_species_hour_heatmap(conn, &today, 1000)?;
            // O-23 weather samples for the in-strip temperature line. Reads
            // the cached Open-Meteo rows — empty when the poll job hasn't
            // populated them, in which case the strip simply has no line.
            let samples: Vec<birdnet_db::weather::WeatherRow> = {
                use birdnet_db::weather::WeatherStore as _;
                let from = format!("{today}T00:00:00Z");
                let to = format!("{today}T23:59:59Z");
                conn.range(&from, &to).unwrap_or_default()
            };
            let solar = super::solar_times_local(conn);
            Ok::<_, birdnet_db::sqlite::DbError>((rows, by_hour, samples, solar, slots))
        })
    })
    .await;

    // See `error_states::failed_partial` for why this is a 200.
    let Ok(Ok((rows, by_hour, samples, solar, slots))) = result else {
        tracing::warn!("today daystrip: query or task failed");
        return super::error_states::failed_partial("today's timeline");
    };

    if rows.iter().all(|r| r.count <= 0) {
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
    for r in &rows {
        if let Ok(hi) = r.hour.parse::<usize>()
            && hi < 24
        {
            hourly[hi] += r.count;
        }
    }
    // Per hour, the three species with the most detections, in the station's language.
    let mut top: [Vec<(String, i64)>; 24] = std::array::from_fn(|_| Vec::new());
    for (name, hour, count) in &by_hour {
        if let Some(slot) = top.get_mut(usize::from(*hour)) {
            slot.push((names.name_of_stored(name).to_owned(), *count));
        }
    }
    for slot in &mut top {
        slot.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        slot.truncate(3);
    }
    // The day's eight busiest species, each its own colour in the columns, and whatever else was
    // heard is one more band: the column stays as tall as the hour's count.
    let (stack_names, stack_counts) = day_stack(&by_hour, &names);
    let stack = (!stack_names.is_empty()).then_some(super::viz::bars::Stack {
        names: &stack_names,
        slots: &slots,
        counts: &stack_counts,
    });

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
    let dawn: i64 = hourly[super::frame::DAWN_FIRST_HOUR..=super::frame::DAWN_LAST_HOUR]
        .iter()
        .sum();

    let stats_oob = format!(
        r#"<div class="x-daystats" id="td-daystats" hx-swap-oob="true"><div><div class="v">{peak_clock}</div><div class="l">peak hour</div></div><div><div class="v x-dawn-v">{dawn}</div><div class="l">in dawn chorus</div></div><div><div class="v">{total_fmt}</div><div class="l">total today</div></div></div>"#,
        total_fmt = super::group_thousands(total),
        peak_clock = crate::display::clock_hour(peak_hour),
    );
    // The caption's temperature clause, kept in step with whether the line is
    // actually drawn (`day_strip` draws it only for a non-empty `temps`).
    let tempkey_oob = if temps.is_empty() {
        r#"<span id="td-tempkey" hx-swap-oob="true"></span>"#.to_string()
    } else {
        r#"<span id="td-tempkey" hx-swap-oob="true"> · temperature runs along the bottom</span>"#
            .to_string()
    };
    let strip = super::viz::day_strip(
        &hourly,
        &top,
        stack.as_ref(),
        &temps,
        solar,
        super::now_hour_local(),
    );
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html")],
        format!("{stats_oob}{tempkey_oob}{strip}"),
    )
}

/// The species the day strip's columns are made of: the eight with the most detections today,
/// most heard first (ties by name), as the station names them, with how many of each were heard
/// in every hour. `rows` is `(stored name, hour, count)`, as the heat-map query returns it.
fn day_stack(
    rows: &[(String, u8, i64)],
    names: &birdnet_core::i18n::NameLocalizer,
) -> (Vec<String>, Vec<[i64; 24]>) {
    let mut per: std::collections::BTreeMap<&str, [i64; 24]> = std::collections::BTreeMap::new();
    for (name, hour, count) in rows {
        if let Some(slot) = per
            .entry(name.as_str())
            .or_insert([0; 24])
            .get_mut(usize::from(*hour))
        {
            *slot += count;
        }
    }
    let mut ranked: Vec<(&str, [i64; 24])> = per.into_iter().collect();
    ranked.sort_by(|a, b| {
        let (ta, tb): (i64, i64) = (a.1.iter().sum(), b.1.iter().sum());
        tb.cmp(&ta).then_with(|| a.0.cmp(b.0))
    });
    ranked.truncate(8);
    let list = ranked
        .iter()
        .map(|(name, _)| names.name_of_stored(name).to_owned())
        .collect();
    (list, ranked.into_iter().map(|(_, c)| c).collect())
}

/// Render a single detection row into the HTML buffer.
///
/// `locked` holds the base names of locked clips. The lock toggle is the one
/// Recordings uses, in its real state, and only on a detection with a clip:
/// every card used to carry the same "Lock" button, so a locked clip offered
/// to be locked again, could not be unlocked here, and a detection with no
/// clip offered a lock that protected nothing (M9).
///
/// `manage` is whether the reader looks signed in: the lock toggle and Delete
/// are drawn only then, since both post to routes behind the sign-in.
fn render_detection_card(
    html: &mut String,
    d: &birdnet_db::sqlite::DetectionRow,
    locked: &std::collections::HashSet<String>,
    names: &birdnet_core::i18n::NameLocalizer,
    floor: f64,
    manage: bool,
) {
    // The link and the detail URL carry the stored name; only what is *drawn*
    // is in the station's language.
    let enc_name = simple_url_encode(&d.com_name);
    let shown = names.name(&d.sci_name, &d.com_name);

    // Fixed-size play affordance (shared clip player) — native <audio>
    // controls render at different widths per row, so they never aligned.
    let audio = d
        .file_name
        .as_deref()
        .filter(|f| !f.is_empty())
        .map_or_default(|f| {
            let basename = std::path::Path::new(f)
                .file_name()
                .map_or_default(|n| n.to_string_lossy().to_string());
            let safe = escape_html(&basename);
            // Named for what it plays (UIC-12): the name and the time the card shows.
            // The clock as text, not `display::clock`'s markup.
            let label = escape_html(&format!(
                "Play {shown}, {}",
                crate::display::clock_minutes(&d.time).unwrap_or_else(|| d.time.clone())
            ));
            format!(
                "<button type=\"button\" class=\"x-fplay tdl-play\" data-play-src=\"/api/v2/recordings/{safe}\" \
                 title=\"Play clip\" aria-label=\"{label}\">▶</button>"
            )
        });

    let lock = d
        .file_name
        .as_deref()
        .filter(|f| manage && !f.is_empty())
        .map_or_default(|f| {
            let is_locked = locked.contains(&super::recordings::base_name(f));
            super::recordings::lock_button(&crate::state::RowRef::from(d), is_locked)
        });
    let av = avatar(&d.com_name, shown, &d.sci_name, "");
    let conf = conf_bar(d.confidence, floor);
    let com_name = escape_html(shown);
    let sci_name = escape_html(&d.sci_name);
    // `time` is plain text for the confirmation dialogs below, which cannot
    // carry markup; the visible link prints through the display formatter.
    let time = escape_html(&d.time);
    let time_clk = crate::display::clock(&d.time);
    let date_enc = simple_url_encode(&d.date);
    let time_enc = simple_url_encode(&d.time);
    // The clip too: two sources that heard one bird in one second share the
    // date, time and name, and a link without it opens whichever comes first.
    let file_enc = simple_url_encode(d.file_name.as_deref().unwrap_or_default());
    let row_vals = super::recordings::row_vals(&crate::state::RowRef::from(d));
    let delete = if manage {
        delete_button(&row_vals, &com_name, &time)
    } else {
        String::new()
    };

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
         <a href=\"/detections/detail?date={date_enc}&time={time_enc}&name={enc_name}&file={file_enc}\" class=\"tdl-time\">{time_clk}</a></div>\
         {audio}\
         </div>\
         <div class=\"tdl-card-actions\">\
         {lock}{delete}\
         </div></div>",
    );
}

/// The Delete control for one Today row.
fn delete_button(row_vals: &str, com_name: &str, time: &str) -> String {
    format!(
        "<button class=\"bnb-btn danger\" hx-post=\"/pages/today-delete\" \
         hx-vals='{row_vals}' \
         hx-target=\"#today-full\" hx-swap=\"innerHTML\" hx-include=\"#search-form\" \
         hx-confirm=\"Delete detection of {com_name} at {time}?\" \
         data-confirm-action=\"hx-post\" \
         data-confirm-url=\"/pages/today-delete\" \
         data-confirm-title=\"Delete detection\" \
         data-confirm-body=\"Delete detection of {com_name} at {time}?\" \
         data-confirm-confirm-label=\"Delete\" \
         data-confirm-style=\"danger\" \
         title=\"Delete this detection\">Delete</button>"
    )
}

/// Re-render trigger returned by the mutating endpoints: reloads the full-day
/// list (its container) with the current search/filter still applied.
const RELOAD_LIST: &str = "<div hx-get=\"/pages/today-list\" hx-trigger=\"load\" hx-target=\"#today-full\" hx-swap=\"innerHTML\" hx-include=\"#search-form\"></div>";

/// The list reloaded, and a toast that says what the write did.
fn after_write(
    write: super::toast::RowWrite,
    done: &str,
    failed: &str,
) -> axum::response::Response {
    use super::toast::{RowWrite, Toast};
    match write {
        RowWrite::Done => super::toast::with(
            axum::response::Html(RELOAD_LIST.to_string()),
            Toast::success(done),
        )
        .into_response(),
        RowWrite::Gone => super::toast::with(
            axum::response::Html(RELOAD_LIST.to_string()),
            Toast::info("That detection is no longer there — it may have been removed elsewhere."),
        )
        .into_response(),
        RowWrite::Failed => super::toast::not_applied(&Toast::error(failed)),
    }
}

/// Delete a detection and re-render the list.
async fn delete_detection(
    State(state): State<AppState>,
    request_user: crate::auth_middleware::RequestUser,
    HtmxForm(row): HtmxForm<crate::state::RowRef>,
) -> axum::response::Response {
    // `state.delete_detection`, not `with_db(delete_detection)`: the analytics
    // copy is incremental and can never notice a removal on its own, so the
    // deletion has to be mirrored at the same moment.
    let result = tokio::task::spawn_blocking(move || {
        let result = state.delete_detection(&row);
        // Recorded when a row went, not when a button was pressed: a refused
        // write or a row somebody else removed first changed nothing.
        if matches!(result, Ok(true)) {
            crate::audit::audit(
                &state,
                Some(&request_user),
                "detection.delete",
                Some(&crate::audit::detection_target(
                    &row.date,
                    &row.time,
                    &row.sci_name,
                )),
                Some("via=today"),
            );
        }
        result
    })
    .await;
    after_write(
        super::toast::RowWrite::from_result(result, "delete detection"),
        "Detection deleted.",
        "The station could not delete that detection.",
    )
}

/// Re-label a detection and re-render the list.
async fn relabel_detection(
    State(state): State<AppState>,
    request_user: crate::auth_middleware::RequestUser,
    HtmxForm(form): HtmxForm<RelabelForm>,
) -> axum::response::Response {
    // Paired write — see `delete_detection` above.
    let row = crate::state::RowRef {
        date: form.date,
        time: form.time,
        sci_name: form.old_sci_name,
        file_name: form.file_name,
    };
    let result = tokio::task::spawn_blocking(move || {
        let result = state.relabel_detection(&row, &form.new_sci_name, &form.new_com_name);
        // A relabel rewrites what the station says it heard, and changes which
        // species every count and share link names. The old name is the target
        // (it is what the row was); the new one is the event.
        if matches!(result, Ok(true)) {
            crate::audit::audit(
                &state,
                Some(&request_user),
                "detection.relabel",
                Some(&crate::audit::detection_target(
                    &row.date,
                    &row.time,
                    &row.sci_name,
                )),
                Some(&format!("via=today to={}", form.new_sci_name)),
            );
        }
        result
    })
    .await;
    after_write(
        super::toast::RowWrite::from_result(result, "relabel detection"),
        "Detection relabelled.",
        "The station could not relabel that detection.",
    )
}

/// Lock a detection to protect it from disk purge.
async fn lock_detection(
    State(state): State<AppState>,
    request_user: crate::auth_middleware::RequestUser,
    HtmxForm(row): HtmxForm<crate::state::RowRef>,
) -> axum::response::Response {
    let result = tokio::task::spawn_blocking(move || {
        let result = state.set_detection_lock(&row, true);
        if matches!(result, Ok(true)) {
            crate::audit::audit(
                &state,
                Some(&request_user),
                "detection.lock",
                Some(&crate::audit::detection_target(
                    &row.date,
                    &row.time,
                    &row.sci_name,
                )),
                Some("via=today"),
            );
        }
        result
    })
    .await;
    after_write(
        super::toast::RowWrite::from_result(result, "lock clip"),
        "Clip locked — it will not be removed to free space.",
        "The station could not lock that clip, so it is not protected.",
    )
}

/// Unlock a detection (allow disk purge again).
async fn unlock_detection(
    State(state): State<AppState>,
    request_user: crate::auth_middleware::RequestUser,
    HtmxForm(row): HtmxForm<crate::state::RowRef>,
) -> axum::response::Response {
    let result = tokio::task::spawn_blocking(move || {
        let result = state.set_detection_lock(&row, false);
        if matches!(result, Ok(true)) {
            crate::audit::audit(
                &state,
                Some(&request_user),
                "detection.unlock",
                Some(&crate::audit::detection_target(
                    &row.date,
                    &row.time,
                    &row.sci_name,
                )),
                Some("via=today"),
            );
        }
        result
    })
    .await;
    after_write(
        super::toast::RowWrite::from_result(result, "unlock clip"),
        "Clip unlocked.",
        "The station could not unlock that clip.",
    )
}

/// Today's weather pill: the latest temperature, then the day's high and low, each with its unit
/// (the display script rewrites value and unit into the scale the station or reader chose).
fn weather_pill(now: f32, high: f32, low: f32) -> String {
    format!(
        r#"<span class="bnb-pill x-wx"><span class="mono x-wx-now">{now}</span><span class="mono x-wx-hl">H {hi} · L {lo}</span></span>"#,
        now = crate::display::temperature(f64::from(now), 0, true),
        hi = crate::display::temperature(f64::from(high), 0, true),
        lo = crate::display::temperature(f64::from(low), 0, true),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "11° / 2°" said nothing about the scale (UIR-23). Every temperature in the pill carries
    /// its unit, marked so the display script writes the unit the station (or the reader) chose.
    #[test]
    fn todays_temperature_says_its_unit() {
        let pill = weather_pill(11.2, 13.0, 2.4);
        assert_eq!(
            pill.matches(r#"data-tu="1""#).count(),
            3,
            "the now, high and low temperatures each carry the unit flag: {pill}"
        );
        for shown in [">11°C<", ">13°C<", ">2°C<"] {
            assert!(pill.contains(shown), "{shown} missing from {pill}");
        }
    }

    /// A configured source on a process whose detection daemon is not running
    /// is not capturing, whatever the gauges say — and they say nothing, since
    /// nothing ever reconciled them. The badge read that as `Unknown` and
    /// graded it "Healthy" while `/api/v2/health?strict=1` on the same process
    /// said `detection_daemon: stopped` (`ops-4`). With the daemon running,
    /// an unpublished gauge is still "not known yet", not an outage.
    #[test]
    fn a_configured_source_without_a_running_daemon_reads_as_down() {
        use birdnet_db::audio_sources::{AudioSourceStore, NewAudioSource, SourceKind};
        use std::sync::atomic::Ordering;
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        birdnet_db::migration::migrate(&conn).unwrap();
        let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));
        assert_eq!(live_capture_state(&state), CaptureState::NoSource);

        state
            .with_db(|conn| {
                conn.insert(&NewAudioSource::defaults(
                    "mic0",
                    SourceKind::Rtsp,
                    "rtsp://127.0.0.1:1/dead",
                ))
            })
            .unwrap();
        assert_eq!(
            live_capture_state(&state),
            CaptureState::Down,
            "a source on a process with no detection daemon is not capturing"
        );

        state.detection_status_flag().store(true, Ordering::SeqCst);
        assert_eq!(
            live_capture_state(&state),
            CaptureState::Unknown,
            "the counterpart: a running daemon that has not reconciled yet is not an outage"
        );
        state.metrics().set_source_up("mic0", true);
        assert_eq!(live_capture_state(&state), CaptureState::Up);
        state.metrics().set_source_up("mic0", false);
        assert_eq!(live_capture_state(&state), CaptureState::Down);

        // A gauge that says up is the capture supervisor's word and stands
        // even when the detection flag is clear: capture and detection are
        // different threads, and `?strict=1` is what reports a dead detector.
        state.metrics().set_source_up("mic0", true);
        state.detection_status_flag().store(false, Ordering::SeqCst);
        assert_eq!(live_capture_state(&state), CaptureState::Up);
    }

    fn fresh_state() -> AppState {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        birdnet_db::migration::migrate(&conn).unwrap();
        AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
    }

    async fn body_of(response: Response) -> String {
        String::from_utf8(
            axum::body::to_bytes(response.into_body(), 1 << 20)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap()
    }

    fn set_location(state: &AppState, lat: &str, lon: &str) {
        use birdnet_db::settings::{SettingsCategory, set};
        state
            .with_db(|conn| {
                set(conn, "latitude", lat, SettingsCategory::Location)?;
                set(conn, "longitude", lon, SettingsCategory::Location)
            })
            .unwrap();
    }

    /// OOB-02: the wizard redirect stops at the first detection, and the
    /// checklist goes with it, so a station that never learnt where it is
    /// said so nowhere a birder looks. The strip under the hero says it for
    /// as long as it is true, with or without detections.
    #[tokio::test]
    async fn a_station_without_a_location_says_so_on_today_after_its_first_detection() {
        let state = fresh_state();
        state
            .with_db(|conn| {
                conn.execute(
                    "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, detected_at_utc) \
                     VALUES ('2026-11-20', '07:00:00', 'Pluvialis squatarola', 'Grey Plover', 0.9, 1795158000)",
                    [],
                )
            })
            .unwrap();
        let count = state
            .with_read_db(birdnet_db::sqlite::detection_count)
            .unwrap();
        assert_eq!(count, 1, "the fixture must hold the one detection");

        let html = body_of(
            today_nudge_partial(State(state.clone()))
                .await
                .into_response(),
        )
        .await;
        assert!(
            html.contains("does not know where it is")
                && html.contains("/station/capture#set-location"),
            "no location, one detection: {html}"
        );

        set_location(&state, "40.0", "-83.0");
        let html = body_of(today_nudge_partial(State(state)).await.into_response()).await;
        assert!(
            !html.contains("does not know where it is"),
            "the counterpart: a station with a location is not warned: {html}"
        );
    }

    /// OOB-02: the first-run checklist named the admin password, microphone,
    /// model and disk, and not the one setting that decides which birds the
    /// station may name.
    #[test]
    fn firstrun_checklist_has_a_location_row() {
        let missing = firstrun_checklist(&[], Some(12.0), &[], true, true, false, None);
        let row = missing
            .split("<div class=\"x-check-row\">")
            .find(|r| r.contains("Location"))
            .unwrap_or_else(|| panic!("no location row: {missing}"));
        assert!(row.contains("Location not set"), "{row}");
        assert!(
            !row.contains("mk done"),
            "an unset location is not a pass: {row}"
        );
        assert!(row.contains("/station/capture#set-location"), "{row}");

        let set = firstrun_checklist(&[], Some(12.0), &[], true, true, true, None);
        let row = set
            .split("<div class=\"x-check-row\">")
            .find(|r| r.contains("Location"))
            .unwrap_or_else(|| panic!("no location row: {set}"));
        assert!(row.contains("mk done"), "the counterpart: {row}");
        assert!(!row.contains("not set"), "{row}");
    }

    /// OOB-17: an offline Pi with no real-time clock boots on its last saved
    /// time and files detections hours late. Station health already reads
    /// NTP state; Today said nothing.
    #[tokio::test]
    async fn an_unsynchronised_clock_is_said_on_today() {
        let state = fresh_state();
        set_location(&state, "40.0", "-83.0");
        state.metrics().set_clock_synced(Some(false));
        let html = body_of(
            today_nudge_partial(State(state.clone()))
                .await
                .into_response(),
        )
        .await;
        assert!(html.contains("can't check the time"), "unsynced: {html}");

        for synced in [Some(true), None] {
            state.metrics().set_clock_synced(synced);
            let html = body_of(
                today_nudge_partial(State(state.clone()))
                    .await
                    .into_response(),
            )
            .await;
            assert!(
                !html.contains("can't check the time"),
                "the counterpart, {synced:?}: {html}"
            );
        }
    }

    /// MDL-03, MDL-05, MDL-06: a station whose models are not doing what its
    /// configuration asks — a detector that did not start, a second classifier
    /// left out, a filter off — said so nowhere on Today, the page an owner
    /// opens. Each is a notice now, in the words the Health page uses, with
    /// the way to fix it.
    #[tokio::test]
    async fn models_not_doing_what_they_were_set_up_for_are_said_on_today() {
        use birdnet_core::inference::model_state::{
            ClassifierFault, ClassifierState, ClassifierStatus, DetectorState, FilterOff,
            FilterState,
        };
        let state = fresh_state();
        set_location(&state, "40.0", "-83.0");
        let classifier = |id: &str, primary: bool, status| ClassifierState {
            id: id.to_owned(),
            primary,
            model_path: std::path::PathBuf::from(format!("/m/{id}.onnx")),
            labels_path: None,
            status,
        };
        let judging = || ClassifierStatus::Judging {
            sources: Vec::new(),
            unrouted_sources: true,
        };
        let healthy = || {
            let m = state.metrics();
            m.set_classifier_states(vec![
                classifier("birdnet", true, judging()),
                classifier("perch", false, judging()),
            ]);
            m.set_occurrence_filter(FilterState::Active {
                candidates: Some(12),
            });
            m.set_detector_state(DetectorState::Running);
        };
        let nudge = || async {
            let html = body_of(
                today_nudge_partial(State(state.clone()))
                    .await
                    .into_response(),
            )
            .await;
            html.split("<div class=\"x-nudge\"")
                .filter(|n| n.contains("Models banner"))
                .collect::<Vec<_>>()
                .join("\n")
        };

        healthy();
        assert_eq!(nudge().await, "", "the counterpart: a healthy station");

        state.metrics().set_detector_state(DetectorState::Failed {
            why: "its labels file names 12".to_owned(),
        });
        let html = nudge().await;
        assert!(
            html.contains("Nothing is being identified")
                && html.contains("its labels file names 12"),
            "a detector that did not start: {html}"
        );

        healthy();
        state.metrics().set_classifier_states(vec![
            classifier("birdnet", true, judging()),
            classifier(
                "perch",
                false,
                ClassifierStatus::left_out(
                    ClassifierFault::FileMissing,
                    "its model file is not there",
                ),
            ),
        ]);
        let html = nudge().await;
        assert!(
            html.contains("perch") && html.contains("its model file is not there"),
            "a second classifier left out: {html}"
        );

        healthy();
        state.metrics().set_occurrence_filter(FilterState::off(
            FilterOff::ModelUnusable,
            "its model file will not load; install it again",
        ));
        let html = nudge().await;
        assert!(
            html.contains("species occurrence filter is off") && html.contains("install it again"),
            "a damaged filter: {html}"
        );
    }

    /// UIR-11: one source of three up drew a green "recording" pill and a
    /// ticked "Microphone recording" beside a header reading "Source
    /// stopped". All three now say how many of how many.
    #[tokio::test]
    async fn one_source_of_three_up_is_not_called_recording() {
        use birdnet_db::audio_sources::{AudioSourceStore, NewAudioSource, SourceKind};
        let state = fresh_state();
        set_location(&state, "40.0", "-83.0");
        for id in ["mic0", "mic1", "mic2"] {
            state
                .with_db(|conn| {
                    conn.insert(&NewAudioSource::defaults(
                        id,
                        SourceKind::Rtsp,
                        "rtsp://127.0.0.1:1/x",
                    ))
                })
                .unwrap();
        }
        state.metrics().set_source_up("mic0", true);
        state.metrics().set_source_up("mic1", false);
        state.metrics().set_source_up("mic2", false);
        assert_eq!(
            live_capture_state(&state),
            CaptureState::Partial { up: 1, of: 3 }
        );

        let pill = body_of(
            today_pills_partial(State(state.clone()))
                .await
                .into_response(),
        )
        .await;
        // The recording pill is the row's first; the sun pills follow it.
        let pill = pill
            .split("bnb-pill")
            .nth(1)
            .unwrap_or_default()
            .to_string();
        assert!(pill.contains("1 of 3"), "{pill}");
        assert!(
            !pill.contains("moss"),
            "a partial station is not green: {pill}"
        );

        let (a, b, c) = (src("mic0"), src("mic1"), src("mic2"));
        let html = firstrun_checklist(
            &[&a, &b, &c],
            Some(12.0),
            &[Some(true), Some(false), Some(false)],
            true,
            true,
            true,
            None,
        );
        let row = html
            .split("<div class=\"x-check-row\">")
            .find(|r| r.contains("1 of 3"))
            .unwrap_or_else(|| panic!("no '1 of 3' microphone row: {html}"));
        assert!(!row.contains("mk done"), "not ticked: {row}");

        // The counterpart: every source up is the plain green "recording".
        state.metrics().set_source_up("mic1", true);
        state.metrics().set_source_up("mic2", true);
        assert_eq!(live_capture_state(&state), CaptureState::Up);
        let pill = body_of(today_pills_partial(State(state)).await.into_response()).await;
        assert!(pill.contains("bnb-pill moss"), "{pill}");
        assert!(!pill.contains(" of 3"), "{pill}");
        let html = firstrun_checklist(
            &[&a, &b, &c],
            Some(12.0),
            &[Some(true), Some(true), Some(true)],
            true,
            true,
            true,
            None,
        );
        assert!(
            html.contains("Microphone recording") && html.contains("mk done"),
            "{html}"
        );
    }

    /// The gauge says the capture process is up, and the sampler says the audio
    /// carries nothing (OOB-05): a muted or zero-gain microphone ticked
    /// "Microphone recording" and promised "Listening for the first call…".
    #[test]
    fn firstrun_checklist_says_silent_for_a_recording_microphone_that_hears_nothing() {
        let s = src("src_1");
        for heard in [-96.0, -90.0] {
            let html = firstrun_checklist(
                &[&s],
                Some(38.0),
                &[Some(true)],
                true,
                true,
                true,
                Some(heard),
            );
            assert!(
                html.contains("Silent — check the microphone isn't muted"),
                "{heard} dBFS: {html}"
            );
            assert!(html.contains("mk down"), "{heard} dBFS: not a tick");
            assert!(
                !html.contains("Listening for the first call"),
                "{heard} dBFS: {html}"
            );
        }
    }

    /// The counterpart: a room's floor, a floor just above the line, or no
    /// reading at all is not "Silent", and a gauge that is down says so first.
    #[test]
    fn firstrun_checklist_is_not_silent_for_a_microphone_that_hears_a_room() {
        let s = src("src_1");
        for heard in [Some(-40.0), Some(-89.9), None] {
            let html =
                firstrun_checklist(&[&s], Some(38.0), &[Some(true)], true, true, true, heard);
            assert!(html.contains("Microphone recording"), "{heard:?}: {html}");
            assert!(!html.contains("Silent"), "{heard:?}: {html}");
            assert!(html.contains("Listening for the first call"), "{heard:?}");
        }
        let down = firstrun_checklist(
            &[&s],
            Some(38.0),
            &[Some(false)],
            true,
            true,
            true,
            Some(-96.0),
        );
        assert!(down.contains("Microphone not recording"), "{down}");
        assert!(!down.contains("Silent"), "{down}");
    }

    fn level(
        date: &str,
        hour: u8,
        source: &str,
        floor: f64,
    ) -> birdnet_db::audio_levels::HourlyLevel {
        birdnet_db::audio_levels::HourlyLevel {
            date: date.to_owned(),
            hour,
            source: source.to_owned(),
            samples: 3,
            noise_floor_dbfs: floor,
            noise_floor_min_dbfs: floor,
            snr_db: 0.0,
            spectral_flatness: 0.5,
            rain_samples: 0,
        }
    }

    /// The reading is the newest bucket per source in this hour or the last,
    /// and the loudest of those. 2026-11-01 00:30 local, so the hour before is
    /// on the previous day (and month).
    #[test]
    fn the_recent_floor_is_the_loudest_source_in_this_hour_or_the_last() {
        // 2026-11-01T00:30:00 as local wall-clock seconds.
        let now = 1_793_493_000;
        let c = birdnet_core::civil::civil_from_unix_secs(now);
        assert_eq!(
            (c.year, c.month, c.day, c.hour),
            (2026, 11, 1, 0),
            "the fixture's clock"
        );

        assert_eq!(loudest_recent_floor(&[], now), None);
        let silent = [level("2026-11-01", 0, "local", -96.0)];
        assert_eq!(loudest_recent_floor(&silent, now), Some(-96.0));
        let last_hour = [level("2026-10-31", 23, "local", -96.0)];
        assert_eq!(
            loudest_recent_floor(&last_hour, now),
            Some(-96.0),
            "the hour before, across midnight and a month"
        );
        let stale = [level("2026-10-31", 22, "local", -96.0)];
        assert_eq!(
            loudest_recent_floor(&stale, now),
            None,
            "two hours ago is stale"
        );
        let older_run = [level("2026-10-01", 0, "local", -96.0)];
        assert_eq!(
            loudest_recent_floor(&older_run, now),
            None,
            "same hour, other day"
        );

        // Newest first: this hour's room floor wins over last hour's silence.
        let recovered = [
            level("2026-11-01", 0, "local", -48.0),
            level("2026-10-31", 23, "local", -96.0),
        ];
        assert_eq!(loudest_recent_floor(&recovered, now), Some(-48.0));
        let newly_muted = [
            level("2026-11-01", 0, "local", -96.0),
            level("2026-10-31", 23, "local", -48.0),
        ];
        assert_eq!(loudest_recent_floor(&newly_muted, now), Some(-96.0));

        // Two sources, one silent: not silent.
        let two = [
            level("2026-11-01", 0, "RTSP_1", -55.0),
            level("2026-11-01", 0, "local", -96.0),
        ];
        assert_eq!(loudest_recent_floor(&two, now), Some(-55.0));
    }

    #[test]
    fn duration_formatting() {
        assert_eq!(fmt_duration(8_040), "2h 14m");
        assert_eq!(fmt_duration(2_700), "45m");
        assert_eq!(fmt_duration(3_600), "1h 00m");
    }

    #[test]
    fn a_narrowed_list_does_not_say_the_day_was_empty() {
        let plain = empty_day_message(None, TodayFilter::All);
        assert!(plain.contains("No detections yet today"), "{plain}");
        let rare = empty_day_message(None, TodayFilter::Rare);
        assert!(!rare.contains("No detections"), "{rare}");
        assert!(
            rare.contains("rare birds") && rare.contains("All"),
            "{rare}"
        );
        let search = empty_day_message(Some("<robin>"), TodayFilter::All);
        assert!(
            search.contains("&lt;robin&gt;") && !search.contains("<robin>"),
            "{search}"
        );
        assert!(!empty_day_message(Some("   "), TodayFilter::All).contains("matches"));
    }

    #[test]
    fn the_hero_date_is_the_stations_local_date() {
        // 2026-09-24 22:00 UTC is already Friday the 25th in Sydney (UTC+10)
        // and still Thursday the 24th in New York (UTC-4).
        let t = 1_790_287_200;
        assert_eq!(human_date_at(t, 36_000), "Friday, September 25");
        assert_eq!(human_date_at(t, -14_400), "Thursday, September 24");
        assert_eq!(human_date_at(t, 0), "Thursday, September 24");
    }

    #[test]
    fn human_date_shape() {
        let d = human_date();
        // "Friday, June 13" — weekday, comma, month, space, day-of-month.
        assert!(d.contains(", "), "missing comma: {d}");
        assert!(d.split_whitespace().count() >= 3, "too short: {d}");
    }

    /// DD-14: the first screen says when the station is open to anyone.
    #[test]
    fn firstrun_checklist_says_when_there_is_no_admin_password() {
        let open = firstrun_checklist(&[], Some(12.0), &[], false, true, true, None);
        assert!(open.contains("No admin password"), "{open}");
        assert!(open.contains(r#"href="/onboarding""#), "{open}");
        assert!(
            open.contains("mk down"),
            "an open station is a fault mark, not a tick"
        );
        let owned = firstrun_checklist(&[], Some(12.0), &[], true, true, true, None);
        assert!(owned.contains("Admin password"), "{owned}");
        assert!(!owned.contains("No admin password"), "{owned}");
    }

    #[test]
    fn firstrun_checklist_reflects_missing_microphone() {
        let html = firstrun_checklist(&[], Some(38.0), &[], true, true, true, None);
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
            media_mode: birdnet_db::audio_sources::RtspMediaMode::Full,
            schedule_quiet: None,
            pipeline: birdnet_db::audio_sources::PipelineFlags::default(),
            eq_chain: String::new(),
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
        let html = firstrun_checklist(&[&s], Some(38.0), &[Some(false)], true, true, true, None);
        assert!(html.contains("Microphone not recording"), "{html}");
        assert!(html.contains("mk down"), "must not show a pass mark");
        assert!(
            html.contains("/station/capture"),
            "must point at where to fix it"
        );
    }

    /// "Listening for the first call…" with a live dot sat under
    /// "Microphone not recording" and "Detector not running": a promise the
    /// two rows above it had just withdrawn.
    #[test]
    fn firstrun_checklist_listens_only_when_the_microphone_and_detector_are_up() {
        let s = src("src_1");
        for (capturing, model) in [(Some(false), true), (None, true), (Some(true), false)] {
            let html = firstrun_checklist(&[&s], Some(38.0), &[capturing], true, model, true, None);
            assert!(
                !html.contains("bnb-dot live") && !html.contains("Listening for the first call"),
                "capturing={capturing:?} model={model}: {html}"
            );
        }
        let none = firstrun_checklist(&[], Some(38.0), &[], true, true, true, None);
        assert!(!none.contains("bnb-dot live"), "no microphone: {none}");

        let html = firstrun_checklist(&[&s], Some(38.0), &[Some(true)], true, true, true, None);
        assert!(html.contains("Listening for the first call"), "{html}");
        assert!(html.contains("bnb-dot live"), "{html}");
    }

    #[test]
    fn firstrun_checklist_ticks_a_microphone_that_is_actually_recording() {
        let s = src("src_1");
        let html = firstrun_checklist(&[&s], Some(38.0), &[Some(true)], true, true, true, None);
        assert!(html.contains("Microphone recording"));
        assert!(html.contains("mk done"));
    }

    #[test]
    fn firstrun_checklist_waits_when_the_gauge_has_no_opinion_yet() {
        // No gauge published is "not known", not "broken" — the supervisor may
        // simply not have reconciled the source yet.
        let s = src("src_1");
        let html = firstrun_checklist(&[&s], Some(38.0), &[None], true, true, true, None);
        assert!(html.contains("Microphone starting…"), "{html}");
        assert!(!html.contains("mk down"));
    }

    /// The disk row was a hard-coded tick, so a station at 97 % showed
    /// "Room to record ✓ — nearly full". A green mark reads as "fine".
    #[test]
    fn firstrun_checklist_does_not_tick_a_nearly_full_disk() {
        let html = firstrun_checklist(&[], Some(97.0), &[], true, true, true, None);
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
        let html = firstrun_checklist(&[], Some(12.0), &[], true, true, true, None);
        assert!(html.contains("plenty of space"));
        assert!(html.contains("12% used"));
    }

    /// The model row asserted runtime state ("Model loaded … ready") the page
    /// has no signal for. It now states only what is true: the model ships.
    /// ON-10: the model row reads the detector's liveness rather than
    /// rendering a tick. This test used to assert the opposite — that the
    /// row never said "loaded" and always said "bundled" — which was the
    /// hard-coded tick stated as a requirement.
    #[test]
    fn firstrun_checklist_reports_the_detector_not_a_bundled_claim() {
        let stopped = firstrun_checklist(&[], Some(12.0), &[], true, false, true, None);
        assert!(stopped.contains("Detector not running"), "{stopped}");
        assert!(stopped.contains(r#"href="/admin/doctor""#), "{stopped}");
        assert!(!stopped.contains("Model loaded"), "{stopped}");
        assert!(!stopped.contains("Model bundled"), "{stopped}");
        let running = firstrun_checklist(&[], Some(12.0), &[], true, true, true, None);
        assert!(running.contains("Model loaded"), "{running}");
        assert!(!running.contains("Detector not running"), "{running}");
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
    fn a_sample_rate_is_printed_as_written() {
        assert_eq!(khz(48_000), "48 kHz");
        assert_eq!(khz(44_100), "44.1 kHz");
        assert_eq!(khz(22_050), "22.05 kHz");
        assert_eq!(khz(16_000), "16 kHz");
    }

    #[test]
    fn the_signal_card_does_not_name_a_model_it_was_not_told() {
        // `MODEL_ID` and `MODEL_ROUTES` choose the classifier; this card never
        // asked which, and said "BirdNET V3.0" beneath a Perch station too.
        assert!(!signal_card("", None).contains("V3.0"));
    }

    #[test]
    fn signal_card_footer_is_blank_without_a_source() {
        let html = signal_card("", None);
        assert!(html.contains("input · none"));
        assert!(!html.contains("48 kHz"));
    }

    /// Today's list as `cookie` sees it, on a station with one detection today
    /// that has a clip.
    async fn todays_list_as(cookie: Option<String>) -> String {
        use tower::ServiceExt as _;
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        birdnet_db::migration::migrate(&conn).unwrap();
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name) \
             VALUES (?1, '06:00:00', 'Turdus merula', 'Eurasian Blackbird', 0.9, 'blackbird.mp3')",
            [super::super::today_date_string()],
        )
        .unwrap();
        let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));
        let mut req = axum::http::Request::get("/pages/today-list");
        if let Some(c) = cookie {
            req = req.header(axum::http::header::COOKIE, c);
        }
        let resp = router()
            .with_state(state)
            .oneshot(req.body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        String::from_utf8(body.to_vec()).unwrap()
    }

    /// UIC-24: every Today row drew Delete and the lock toggle for anyone,
    /// though both posts answer a visitor with a redirect to the login page.
    #[tokio::test]
    async fn a_visitor_is_not_offered_delete_or_lock() {
        let html = todays_list_as(None).await;
        assert!(
            html.contains("Eurasian Blackbird"),
            "the fixture row renders: {html}"
        );
        assert!(
            !html.contains("/pages/today-delete"),
            "Delete offered to a visitor: {html}"
        );
        assert!(
            !html.contains("recordings-lock"),
            "the lock toggle offered to a visitor: {html}"
        );
    }

    /// The counterpart: a signed-in owner still has both.
    #[tokio::test]
    async fn a_signed_in_owner_is_offered_delete_and_lock() {
        let token = crate::session::issue_token(&crate::session::generate_session_id(), 60_000);
        let html = todays_list_as(Some(format!("{}={token}", crate::session::COOKIE_NAME))).await;
        assert!(html.contains("/pages/today-delete"), "{html}");
        assert!(html.contains("recordings-lock"), "{html}");
    }
}
