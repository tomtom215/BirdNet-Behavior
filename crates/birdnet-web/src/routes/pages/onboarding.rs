//! First-run onboarding wizard.
//!
//! A full-bleed, no-chrome six-step setup flow (Welcome → Location →
//! Microphone → Accuracy → Notifications → Done) served at `/onboarding`. The
//! wizard persists: the Location step auto-detects coordinates (and the
//! timezone) via the existing `/admin/settings/detect-location` endpoint and
//! submits to `POST /onboarding/save`, which writes the chosen settings and
//! marks onboarding complete. Audio device *selection* is intentionally
//! delegated to Settings → Audio (richer ALSA/RTSP handling lives there); the
//! Microphone step reports what is configured rather than offering a choice it
//! does not implement.
//!
//! **Every value the page shows is real.** The Microphone step and the final
//! summary card used to be a mock-up — a hard-coded "UMC202HD · card 1 ·
//! 48 kHz" microphone marked *recommended*, a "Boston, MA" location, and an
//! `http://birdnet.local/` dashboard address — shown identically to every
//! station regardless of its hardware, whereabouts or how it was reached. The
//! microphone rows are now rendered from `audio_sources`, and the rows that
//! depend on operator input are placeholders the page script fills from the
//! form, so a station with no capture source is told so instead of being
//! congratulated on a device it does not have.
//!
//! The Accuracy step exists because the minimum-confidence threshold decides
//! whether anything is recorded at all, and nothing in the setup path used to
//! mention it: an operator who wanted stricter or looser detection had to find
//! Settings → Detection unprompted. Its cards are pre-selected on
//! [`DEFAULT_CONFIDENCE_THRESHOLD`](birdnet_core::config::DEFAULT_CONFIDENCE_THRESHOLD),
//! so clicking straight through yields exactly what the daemon would have
//! enforced anyway.
//!
//! A fresh station (no detections yet, not onboarded) is redirected here from
//! the dashboard — see `pages::dashboard`.

use std::fmt::Write as _;

use axum::extract::State;
use axum::response::{Html, Redirect};
use axum::routing::{get, post};
use axum::{Form, Router};

use birdnet_db::audio_sources::{AudioSource, AudioSourceStore, SourceKind};
use birdnet_db::settings::{self, SettingsCategory};

use crate::routes::admin::audio::{detail_for, kind_label};
use crate::routes::pages::escape_html;
use crate::state::AppState;

/// Every settings key `POST /onboarding/save` can persist.
///
/// The admin settings form has had a guard for this since twenty of its fields
/// turned out to be editable, persisted, and connected to nothing: the binary
/// classifies each key in `SETTINGS_FORM_KEYS` and a test fails on any that is
/// unclassified. The wizard wrote *outside* that list, so it was never
/// covered — and did the same thing, shipping a notification choice
/// (`notification_mode`) that nothing anywhere read.
///
/// This list closes the gap: the same test now classifies these keys too, so a
/// new wizard field cannot ship inert either.
///
/// Kept in sync with the wizard form (`OnboardingForm`) by a test in this module.
pub const ONBOARDING_SETTING_KEYS: &[&str] = &[
    "latitude",
    "longitude",
    "timezone",
    "notify_trigger",
    "confidence_threshold",
    "onboarding_complete",
];

/// Mount the first-run onboarding wizard routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/onboarding", get(onboarding_page))
        .route("/onboarding/save", post(onboarding_save))
}

/// The settings the wizard must show as they already are, rather than as it
/// wishes they were.
///
/// Every one of these was hardcoded in the markup. For the two text fields that
/// made the page merely uninformative — a station configured at install time
/// rendered blank boxes, so setup looked like it had lost the operator's
/// answers. For the two hidden fields it was worse: because they are never
/// empty, they slip past `onboarding_save`'s skip-if-blank guard and are
/// written on every completion. An operator who had set `CONFIDENCE=0.6` and
/// then clicked through the wizard had it silently reset to 0.75.
#[derive(Default)]
struct Prefill {
    latitude: String,
    longitude: String,
    confidence: String,
    notify_trigger: String,
}

impl Prefill {
    /// Read the current values, falling back to the wizard's recommended
    /// defaults only where the station genuinely has no setting yet.
    fn load(conn: &rusqlite::Connection) -> Self {
        let get = |key: &str, default: &str| {
            settings::get_or(conn, key, default).unwrap_or_else(|_| default.to_owned())
        };
        Self {
            latitude: get("latitude", ""),
            longitude: get("longitude", ""),
            confidence: get(
                "confidence_threshold",
                &format!("{:.2}", birdnet_core::config::DEFAULT_CONFIDENCE_THRESHOLD),
            ),
            notify_trigger: get("notify_trigger", DEFAULT_NOTIFY_TRIGGER),
        }
    }
}

/// The wizard's recommended notification trigger, used only when the station
/// has none set.
const DEFAULT_NOTIFY_TRIGGER: &str = "new-species";

async fn onboarding_page(State(state): State<AppState>) -> Html<String> {
    // `list` already excludes soft-deleted rows (`WHERE disabled_at IS NULL`).
    let (sources, prefill) = state
        .with_db(|conn| AudioSourceStore::list(conn).map(|s| (s, Prefill::load(conn))))
        .unwrap_or_else(|err| {
            tracing::error!(error = %err, "onboarding: audio_sources list failed");
            (Vec::new(), Prefill::default())
        });

    Html(render_page(
        &render_mic_body(&sources),
        &escape_html(&mic_summary(&sources)),
        &prefill,
    ))
}

/// Substitute the two server-filled placeholders into the wizard template.
///
/// Deliberately a single pass rather than a `.replace()` chain. Both inserted
/// values derive from operator-controlled text — a microphone's friendly label
/// can contain any characters — and a chained replace re-scans what it just
/// inserted, so a source labelled `{{mic_summary}}` would have had its label
/// silently swapped for the summary line. Escaping does not prevent that:
/// `escape_html` neutralises HTML, not the template's own brace syntax.
///
/// Each placeholder appears exactly once. If one is ever removed from the
/// template this degrades to dropping that value rather than panicking in a
/// request handler; the rendering tests pin the output either way.
fn render_page(mic_body: &str, mic_summary: &str, prefill: &Prefill) -> String {
    // Substituted in one pass each. `mic_body` is already-rendered markup and
    // `mic_summary` is pre-escaped by the caller; the `prefill` values come
    // from the database and land inside HTML attributes, so they are escaped
    // here rather than trusted.
    ONBOARDING_HTML
        .replace("{{mic_body}}", mic_body)
        .replace("{{mic_summary}}", mic_summary)
        .replace("{{latitude}}", &escape_html(&prefill.latitude))
        .replace("{{longitude}}", &escape_html(&prefill.longitude))
        .replace("{{confidence}}", &escape_html(&prefill.confidence))
        .replace("{{notify_trigger}}", &escape_html(&prefill.notify_trigger))
        // Versioned stylesheet URL — see `pages::with_asset_version`.
        .replace("{{version}}", env!("CARGO_PKG_VERSION"))
}

/// The microphone step, rendered from the station's actual capture sources.
///
/// This step used to be a mock-up: a hard-coded "UMC202HD · USB audio ·
/// card 1 · 48 kHz" card marked *recommended* and pre-selected, plus a
/// "Built-in microphone · card 0" and two more cards offering RTSP and
/// folder-watching. None of it was real — no station was consulted, nothing was
/// clickable to any effect, and a first-run operator was shown hardware they do
/// not own, described as already detected. On a station whose microphone was
/// missing or misconfigured, the setup wizard's answer to "will this hear
/// anything?" was a confident yes about a device that does not exist.
///
/// The wizard deliberately does not *change* the capture source — Settings →
/// Audio owns that, with the ALSA/RTSP handling this step cannot reproduce — so
/// the honest version reports rather than pretends to offer a choice. The cards
/// are therefore not selectable: nothing here writes a setting.
fn render_mic_body(sources: &[AudioSource]) -> String {
    if sources.is_empty() {
        return concat!(
            r#"<div class="ob-cards"><div class="ob-card"><span class="ic">🔇</span>"#,
            r#"<div class="ob-grow"><div class="t">No audio source configured</div>"#,
            r#"<div class="s">Nothing will be detected until a microphone or RTSP stream is added. "#,
            r#"The installer normally sets this up; if it could not find your device you can add it by hand.</div>"#,
            r#"</div></div></div>"#,
            r#"<p class="bnb-meta ob-mt-16"><a href="/station/capture">Add a microphone or RTSP stream →</a></p>"#,
        )
        .to_string();
    }

    let mut out = String::from(r#"<div class="ob-cards" id="mic-cards">"#);
    for s in sources {
        let icon = if matches!(s.kind, SourceKind::Rtsp) {
            "📡"
        } else {
            "🎤"
        };
        // Unlabelled sources take the device id as their heading, so repeating
        // it on the detail line would print `plughw:CARD=PRO,DEV=0` twice.
        let label = friendly_label(s);
        let detail = match label {
            Some(_) => format!("{} · {}", s.device_id, detail_for(s)),
            None => detail_for(s),
        };
        let _ = write!(
            out,
            concat!(
                r#"<div class="ob-card"><span class="ic">{icon}</span><div class="ob-grow">"#,
                r#"<div class="t">{name} <span class="bnb-pill ob-ml-6">{kind}</span></div>"#,
                r#"<div class="s mono">{detail}</div></div></div>"#,
            ),
            icon = icon,
            name = escape_html(label.unwrap_or(&s.device_id)),
            kind = escape_html(kind_label(s.kind)),
            detail = escape_html(&detail),
        );
    }
    out.push_str("</div>");
    out.push_str(
        r#"<p class="bnb-meta ob-mt-16">Set up by the installer. Add, remove or fine-tune sources (USB, RTSP, gain) any time in <a href="/station/capture">Settings → Capture</a>.</p>"#,
    );
    out
}

/// The operator's own name for a source, when they gave it one.
///
/// A label of `""` or `"   "` is the admin form's "no label" state, not a name,
/// so it is treated as absent rather than rendered as a blank heading.
fn friendly_label(s: &AudioSource) -> Option<&str> {
    s.label.as_deref().map(str::trim).filter(|l| !l.is_empty())
}

/// One-line description of the capture setup for the final step's summary.
///
/// Replaced a hard-coded "UMC202HD · USB · 48 kHz" that was shown to every
/// station regardless of what it actually captures with.
fn mic_summary(sources: &[AudioSource]) -> String {
    match sources {
        [] => "None configured — no birds will be detected".to_string(),
        [one] => format!(
            "{} · {}",
            friendly_label(one).unwrap_or(&one.device_id),
            detail_for(one)
        ),
        many => format!("{} sources configured", many.len()),
    }
}

/// Fields the first-run wizard submits. All optional: clicking straight through
/// still marks onboarding complete (so the first-boot redirect stops firing)
/// without writing empty settings.
#[derive(Debug, Default, serde::Deserialize)]
struct OnboardingForm {
    #[serde(default)]
    latitude: String,
    #[serde(default)]
    longitude: String,
    #[serde(default)]
    timezone: String,
    #[serde(default)]
    notification_mode: String,
    #[serde(default)]
    confidence_threshold: String,
}

/// Whether `raw` is a confidence threshold the daemon will accept.
///
/// The wizard's cards can only emit `0.4`/`0.6`/`0.75`/`0.9`, so this exists
/// for the hand-crafted POST: `birdnet_core::config::validate` treats a
/// `CONFIDENCE` outside 0–1 as an *error*, and `--doctor` runs from the unit's
/// `ExecStartPre` where exit 2 is fatal. An unvalidated write here would let a
/// setup form leave the station unable to start.
fn valid_confidence(raw: &str) -> bool {
    raw.parse::<f64>().is_ok_and(|v| (0.0..=1.0).contains(&v))
}

/// Whether `raw` is a notification trigger the runtime understands.
///
/// `TriggerMode::parse` maps anything it does not recognise to
/// `EachDetection` — an alert on every single detection. So an unvalidated
/// write here would turn a typo, or a stale value from an older wizard, into
/// the chattiest possible setting: the exact opposite of what an operator
/// choosing a quieter mode asked for. Kept in step with
/// `birdnet_integrations::notification::TriggerMode::parse`.
fn valid_trigger(raw: &str) -> bool {
    matches!(raw, "each" | "new-species" | "new-species-daily")
}

/// Persist the wizard's choices and mark onboarding complete, then return to the
/// dashboard. Only non-empty values are written; the DB settings overlay applies
/// latitude/longitude and the confidence threshold on the next start.
async fn onboarding_save(
    State(state): State<AppState>,
    Form(form): Form<OnboardingForm>,
) -> Redirect {
    state.with_db(|conn| {
        // Idempotent safety net; the table is also created by migration 14.
        settings::ensure_settings_table(conn).ok();

        let mut items: Vec<(&str, &str, SettingsCategory)> = Vec::new();
        let lat = form.latitude.trim();
        let lon = form.longitude.trim();
        let tz = form.timezone.trim();
        let mode = form.notification_mode.trim();
        if !lat.is_empty() {
            items.push(("latitude", lat, SettingsCategory::Location));
        }
        if !lon.is_empty() {
            items.push(("longitude", lon, SettingsCategory::Location));
        }
        if !tz.is_empty() {
            items.push(("timezone", tz, SettingsCategory::Location));
        }
        // `notify_trigger` — the key the notification filter actually reads
        // (bridged onto `APPRISE_TRIGGER`). The wizard used to write
        // `notification_mode`, which nothing anywhere consumed: the operator
        // picked "Quiet" or "Everything", it was persisted, and it governed
        // nothing. Only the three values `TriggerMode::parse` understands are
        // accepted, because an unknown value silently parses as "every
        // detection" — the chattiest mode, and the opposite of what someone
        // choosing a quieter one intends.
        if valid_trigger(mode) {
            items.push(("notify_trigger", mode, SettingsCategory::Notifications));
        }
        // Only persisted when it parses inside 0–1. The wizard's own cards can
        // only produce 0.4/0.6/0.75/0.9, but the field is a plain form value:
        // a hand-crafted POST must not be able to seed a threshold the daemon
        // would reject at startup (`--doctor` exits 2 on out-of-range
        // CONFIDENCE, which `ExecStartPre` treats as fatal) — that would turn
        // the setup wizard into a way to brick the service.
        let conf = form.confidence_threshold.trim();
        if valid_confidence(conf) {
            items.push(("confidence_threshold", conf, SettingsCategory::Detection));
        }
        if !items.is_empty() {
            let _ = settings::set_many(conn, &items);
        }
        // Set the completion flag last so a fresh box stops being redirected here
        // even if a settings write above failed.
        let _ = settings::set(
            conn,
            "onboarding_complete",
            "true",
            SettingsCategory::System,
        );
    });
    Redirect::to("/")
}

const ONBOARDING_HTML: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>BirdNet-Behavior · Set up your station</title>
<link rel="stylesheet" href="/static/css/app.css?v={{version}}">
<script src="/static/theme-guard.js"></script>
<style>
  body { margin:0; background:var(--bg); color:var(--fg); min-height:100vh; }
  .ob-root { max-width:980px; margin:0 auto; min-height:100vh; display:flex; flex-direction:column; padding:0 24px; }
  .ob-stepper { display:flex; align-items:center; gap:8px; padding:22px 0 8px; position:sticky; top:0; background:color-mix(in oklch, var(--bg) 92%, transparent); backdrop-filter:saturate(1.4) blur(10px); z-index:5; }
  .ob-pip { display:flex; align-items:center; gap:8px; }
  .ob-pip .dot { width:22px; height:22px; border-radius:999px; border:0.5px solid var(--border-2); display:flex; align-items:center; justify-content:center; font-size:11px; font-family:var(--font-mono); color:var(--fg-3); background:var(--surface); }
  .ob-pip .nm { font-size:11px; letter-spacing:0.06em; text-transform:uppercase; color:var(--fg-3); }
  .ob-pip .bar { width:28px; height:1.5px; background:var(--hairline); }
  .ob-pip.done .dot, .ob-pip.active .dot { background:var(--moss); color:var(--bg); border-color:transparent; }
  .ob-pip.active .nm { color:var(--fg); font-weight:500; }
  .ob-stage { flex:1; display:flex; align-items:center; padding:24px 0; }
  .ob-step { display:none; width:100%; animation:ob-fade .2s ease; }
  .ob-step.active { display:block; }
  @keyframes ob-fade { from { opacity:0; transform:translateY(6px); } to { opacity:1; transform:none; } }
  .ob-two { display:grid; grid-template-columns:1fr 1fr; gap:32px; align-items:center; }
  .ob-eyebrow { font-size:10.5px; letter-spacing:0.10em; text-transform:uppercase; color:var(--fg-3); font-weight:500; }
  .ob-h { font-family:var(--font-display); font-size:46px; line-height:1.08; letter-spacing:-0.02em; margin:8px 0 12px; }
  .ob-h em { font-style:italic; color:var(--moss-ink); }
  .ob-p { color:var(--fg-2); font-size:15px; max-width:42ch; }
  .ob-bullets { list-style:none; padding:0; margin:18px 0 0; display:flex; flex-direction:column; gap:10px; }
  .ob-bullets li { display:flex; gap:10px; align-items:center; font-size:14px; }
  .ob-bullets .tick { width:18px; height:18px; border-radius:999px; background:var(--moss-soft); color:var(--moss-ink); display:inline-flex; align-items:center; justify-content:center; font-size:11px; }
  .ob-nav { display:flex; align-items:center; justify-content:space-between; gap:16px; padding:18px 0 28px; border-top:0.5px solid var(--hairline); }
  .ob-field { display:flex; flex-direction:column; gap:6px; margin-bottom:14px; }
  .ob-field label { font-size:12.5px; font-weight:500; }
  .ob-field input { padding:9px 12px; border-radius:var(--r-sm); border:0.5px solid var(--border-2); background:var(--surface); color:var(--fg); font:inherit; }
  .ob-cards { display:grid; gap:12px; }
  .ob-card { display:flex; gap:14px; align-items:center; padding:14px; border-radius:var(--r-md); border:0.5px solid var(--border); background:var(--surface); cursor:pointer; transition:border-color .12s, background .12s; }
  .ob-card.sel { border-color:var(--moss); background:var(--moss-soft); }
  .ob-card .ic { width:34px; height:34px; flex-shrink:0; border-radius:8px; background:var(--surface-2); display:flex; align-items:center; justify-content:center; color:var(--fg-2); }
  .ob-card .t { font-weight:500; font-size:14px; }
  .ob-card .s { font-size:12px; color:var(--fg-3); }
  .vu { display:flex; align-items:flex-end; gap:2px; height:26px; margin-left:auto; }
  .vu i { width:3px; background:var(--moss); border-radius:1px; animation:vu 1.1s ease-in-out infinite; }
  @keyframes vu { 0%,100% { height:20%; } 50% { height:95%; } }
  .chips { display:flex; flex-wrap:wrap; gap:8px; margin-top:12px; }
  .summary-row { display:flex; justify-content:space-between; gap:16px; padding:11px 0; border-top:0.5px solid var(--hairline); font-size:14px; }
  .summary-row:first-child { border-top:0; }
  .summary-row .k { color:var(--fg-3); }
  .calib { display:flex; align-items:flex-end; gap:2px; height:46px; }
  .calib i { flex:1; background:var(--moss); opacity:.5; border-radius:1px; animation:vu 1.6s ease-in-out infinite; }
  @media (prefers-reduced-motion: reduce) {
    .ob-step, .vu i, .calib i, .sonar * { animation:none !important; }
  }
  /* O-25: static inline style= attributes folded into this page's own <style>
     block (faithful, value-preserving). */
  .ob-center { display:flex; align-items:center; justify-content:center; }
  .ob-grow { flex:1; }
  .ob-mt-6 { margin-top:6px; }
  .ob-mt-16 { margin-top:16px; }
  .ob-mt-18 { margin-top:18px; }
  .ob-mb-18 { margin-bottom:18px; }
  .ob-ml-4 { margin-left:4px; }
  .ob-ml-6 { margin-left:6px; }
  .ob-latlon { display:grid; grid-template-columns:1fr 1fr; gap:12px; margin-top:16px; }
  .ob-map { background:var(--surface-2); border-radius:var(--r-lg); border:0.5px solid var(--border); }
  .ob-cards.cols2 { grid-template-columns:repeat(2,1fr); }
  .ob-summary { cursor:pointer; }
  .ob-calib-m { margin:14px 0; }
  .ob-hidden-init { visibility:hidden; }
  /* Staggered VU / calibration bar animation delays (were per-<i> inline). */
  .vu i:nth-child(1) { animation-delay:0s; }   .vu i:nth-child(2) { animation-delay:.1s; }
  .vu i:nth-child(3) { animation-delay:.2s; }  .vu i:nth-child(4) { animation-delay:.05s; }
  .vu i:nth-child(5) { animation-delay:.25s; } .vu i:nth-child(6) { animation-delay:.15s; }
  .vu i:nth-child(7) { animation-delay:.3s; }  .vu i:nth-child(8) { animation-delay:.08s; }
  .calib i:nth-child(1) { animation-delay:0s; }    .calib i:nth-child(2) { animation-delay:.1s; }
  .calib i:nth-child(3) { animation-delay:.2s; }   .calib i:nth-child(4) { animation-delay:.3s; }
  .calib i:nth-child(5) { animation-delay:.15s; }  .calib i:nth-child(6) { animation-delay:.25s; }
  .calib i:nth-child(7) { animation-delay:.05s; }  .calib i:nth-child(8) { animation-delay:.35s; }
  .calib i:nth-child(9) { animation-delay:.12s; }  .calib i:nth-child(10) { animation-delay:.22s; }
  .calib i:nth-child(11) { animation-delay:.32s; } .calib i:nth-child(12) { animation-delay:.18s; }
  /* Phones: drop the step labels (numbered dots + connectors stay), tighten
     padding, and stack the two-column step bodies so nothing overflows 390px.
     The lat/lon + notify grids were inline grids caught by the global
     `[style*="grid-template-columns"]` reset; now classes, they carry their own. */
  @media (max-width:520px) {
    .ob-root { padding:0 14px; }
    .ob-pip .nm { display:none; }
    .ob-pip .bar { width:16px; }
    .ob-two { grid-template-columns:1fr; gap:18px; }
    .ob-latlon { grid-template-columns:1fr; }
    .ob-cards.cols2 { grid-template-columns:1fr; }
  }
</style>
</head>
<body>
<div class="ob-root">
  <form id="ob-form" method="post" action="/onboarding/save">
  <div class="ob-stepper" id="ob-stepper">
    <div class="ob-pip" data-pip="1"><span class="dot">1</span><span class="nm">Welcome</span></div>
    <span class="bar"></span>
    <div class="ob-pip" data-pip="2"><span class="dot">2</span><span class="nm">Location</span></div>
    <span class="bar"></span>
    <div class="ob-pip" data-pip="3"><span class="dot">3</span><span class="nm">Microphone</span></div>
    <span class="bar"></span>
    <div class="ob-pip" data-pip="4"><span class="dot">4</span><span class="nm">Accuracy</span></div>
    <span class="bar"></span>
    <div class="ob-pip" data-pip="5"><span class="dot">5</span><span class="nm">Alerts</span></div>
    <span class="bar"></span>
    <div class="ob-pip" data-pip="6"><span class="dot">6</span><span class="nm">Done</span></div>
  </div>

  <div class="ob-stage">
    <!-- Step 1 — Welcome -->
    <section class="ob-step active" data-step="1">
      <div class="ob-two">
        <div>
          <div class="ob-eyebrow">Welcome</div>
          <h1 class="ob-h">Let's teach the yard<br>to <em>listen</em>.</h1>
          <p class="ob-p">Ninety seconds, six steps. Your Raspberry Pi will start identifying every bird it hears — no accounts, no cloud, all yours.</p>
          <ul class="ob-bullets">
            <li><span class="tick">✓</span> No accounts — runs entirely on your Pi</li>
            <li><span class="tick">✓</span> Set once — sensible defaults the whole way</li>
            <li><span class="tick">✓</span> Always tweakable — change anything later in Settings</li>
          </ul>
        </div>
        <div class="ob-center">
          <svg class="sonar" width="240" height="240" viewBox="0 0 240 240" aria-hidden="true">
            <g fill="none" stroke="var(--moss)" stroke-width="1">
              <circle cx="120" cy="120" r="30"><animate attributeName="r" values="30;110" dur="5s" repeatCount="indefinite"/><animate attributeName="stroke-opacity" values="0.7;0" dur="5s" repeatCount="indefinite"/></circle>
              <circle cx="120" cy="120" r="30"><animate attributeName="r" values="30;110" dur="5s" begin="1.6s" repeatCount="indefinite"/><animate attributeName="stroke-opacity" values="0.7;0" dur="5s" begin="1.6s" repeatCount="indefinite"/></circle>
              <circle cx="120" cy="120" r="30"><animate attributeName="r" values="30;110" dur="5s" begin="3.2s" repeatCount="indefinite"/><animate attributeName="stroke-opacity" values="0.7;0" dur="5s" begin="3.2s" repeatCount="indefinite"/></circle>
            </g>
            <rect x="96" y="96" width="48" height="48" rx="8" fill="var(--surface)" stroke="var(--border-2)" stroke-width="0.5"/>
            <g stroke="var(--moss)" stroke-width="2" stroke-linecap="round">
              <line x1="110" y1="120" x2="110" y2="120"/><line x1="116" y1="112" x2="116" y2="128"/>
              <line x1="122" y1="106" x2="122" y2="134"/><line x1="128" y1="113" x2="128" y2="127"/>
            </g>
          </svg>
        </div>
      </div>
    </section>

    <!-- Step 2 — Location -->
    <section class="ob-step" data-step="2">
      <div class="ob-two">
        <div>
          <div class="ob-eyebrow">Where</div>
          <h1 class="ob-h">Where is the station?</h1>
          <p class="ob-p">Your coordinates let BirdNET weight species by what's actually likely in your area, and compute sunrise / sunset for the dawn-chorus window.</p>
          <div class="ob-mt-18">
            <button class="bnb-btn" type="button" id="ob-detect">⌖ Auto-detect my location</button>
          </div>
          <div class="ob-latlon">
            <div class="ob-field"><label for="ob-lat">Latitude</label><input id="ob-lat" name="latitude" type="text" placeholder="e.g. 42.3601" inputmode="decimal" value="{{latitude}}"></div>
            <div class="ob-field"><label for="ob-lon">Longitude</label><input id="ob-lon" name="longitude" type="text" placeholder="e.g. -71.0589" inputmode="decimal" value="{{longitude}}"></div>
          </div>
          <input type="hidden" name="timezone" id="ob-tz">
          <div class="bnb-pill ob-mt-6" id="ob-loc-pill">Auto-detect, or type your coordinates — you can change this any time in Settings.</div>
        </div>
        <div class="ob-center">
          <svg width="280" height="220" viewBox="0 0 280 220" aria-hidden="true" class="ob-map">
            <g fill="none" stroke="var(--border-2)" stroke-width="0.75" opacity="0.7">
              <ellipse cx="140" cy="110" rx="40" ry="28"/><ellipse cx="140" cy="110" rx="70" ry="50"/>
              <ellipse cx="140" cy="110" rx="100" ry="72"/><ellipse cx="140" cy="110" rx="128" ry="94"/>
            </g>
            <circle cx="140" cy="110" r="86" fill="none" stroke="var(--moss)" stroke-width="1" stroke-dasharray="4 5"/>
            <circle cx="140" cy="110" r="9" fill="none" stroke="var(--moss)" stroke-width="1.5"/>
            <circle cx="140" cy="110" r="3" fill="var(--moss)"/>
            <text x="140" y="206" text-anchor="middle" font-size="9" class="mono" fill="var(--fg-4)">~100 km radius</text>
          </svg>
        </div>
      </div>
    </section>

    <!-- Step 3 — Microphone -->
    <section class="ob-step" data-step="3">
      <div class="ob-eyebrow">How it hears</div>
      <h1 class="ob-h">What it's listening with.</h1>
      <p class="ob-p ob-mb-18">This is the capture source your station is actually configured to use.</p>
      {{mic_body}}
    </section>

    <!-- Step 4 — Detection threshold -->
    <section class="ob-step" data-step="4">
      <div class="ob-eyebrow">How sure is sure</div>
      <h1 class="ob-h">How picky should it be?</h1>
      <p class="ob-p ob-mb-18">Every guess comes with a confidence score. Anything below your threshold is thrown away — so this is the dial between "only the birds it's certain about" and "everything it thinks it heard".</p>
      <div class="ob-cards cols2">
        <div class="ob-card" data-radio="conf" data-value="0.9"><div class="ob-grow"><div class="t">Strict</div><div class="s">0.90 — only the IDs it is near-certain about. Very few false positives; quiet and distant birds go unlogged.</div></div></div>
        <div class="ob-card" data-radio="conf" data-value="0.75"><div class="ob-grow"><div class="t">Balanced <span class="bnb-pill moss ob-ml-6">recommended</span></div><div class="s">0.75 — realistic results without over-filtering. Start here.</div></div></div>
        <div class="ob-card" data-radio="conf" data-value="0.6"><div class="ob-grow"><div class="t">Sensitive</div><div class="s">0.60 — catches quiet and distant birds, at the cost of more misidentifications.</div></div></div>
        <div class="ob-card" data-radio="conf" data-value="0.4"><div class="ob-grow"><div class="t">Everything</div><div class="s">0.40 — for tuning and curiosity. Expect a lot of noise.</div></div></div>
      </div>
      <input type="hidden" name="confidence_threshold" id="ob-conf" value="{{confidence}}">
      <p class="bnb-meta ob-mt-16">Not permanent — change it any time in <a href="/admin">Settings → Detection</a>, and set per-species thresholds under Species.</p>
    </section>

    <!-- Step 5 — Notifications -->
    <section class="ob-step" data-step="5">
      <div class="ob-eyebrow">Who gets told</div>
      <h1 class="ob-h">When should we ping you?</h1>
      <p class="ob-p ob-mb-18">This sets <em>how often</em> alerts go out. Nothing is sent until you add somewhere to send it — you can do that whenever you like.</p>
      <div class="ob-cards">
        <div class="ob-card" data-radio="notify" data-value="new-species"><div class="ob-grow"><div class="t">New species this week <span class="bnb-pill moss ob-ml-6">recommended</span></div><div class="s">Only birds you have barely heard lately — the interesting ones.</div></div></div>
        <div class="ob-card" data-radio="notify" data-value="new-species-daily"><div class="ob-grow"><div class="t">First of each species, daily</div><div class="s">One alert per species per day. A good middle ground.</div></div></div>
        <div class="ob-card" data-radio="notify" data-value="each"><div class="ob-grow"><div class="t">Every detection</div><div class="s">One alert every single time. Chatty — hundreds a day at a busy feeder.</div></div></div>
      </div>
      <input type="hidden" name="notification_mode" id="ob-notify" value="{{notify_trigger}}">
      <p class="bnb-meta ob-mt-16">Add a channel — Telegram, email, MQTT, ntfy, webhooks and more — under <a href="/admin/settings">Settings → Notifications</a>. Until then this setting is simply waiting.</p>
    </section>

    <!-- Step 6 — Done -->
    <section class="ob-step" data-step="6">
      <div class="ob-two">
        <div>
          <div class="ob-eyebrow">All set</div>
          <h1 class="ob-h">You're <em>listening</em>.</h1>
          <p class="ob-p">The pipeline is warming up. Within a minute or two you'll see the first detections roll in.</p>
          <div class="bnb-card pad ob-mt-16">
            <div class="summary-row"><span class="k">Location</span><span id="ob-sum-loc">Not set</span></div>
            <div class="summary-row"><span class="k">Microphone</span><span>{{mic_summary}}</span></div>
            <div class="summary-row"><span class="k">Minimum confidence</span><span id="ob-sum-conf">0.75 · Balanced</span></div>
            <div class="summary-row"><span class="k">Alerts</span><span id="ob-sum-notify">Rare birds only</span></div>
            <div class="summary-row"><span class="k">Dashboard</span><span class="mono" id="ob-sum-url">—</span></div>
          </div>
        </div>
        <div>
          <div class="bnb-card pad">
            <div class="ob-eyebrow">Warming up</div>
            <div class="calib ob-calib-m"><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i></div>
            <div class="bnb-meta">Calibrating noise floor… <span class="bnb-pill moss ob-ml-4">BirdNET+ V3.0</span></div>
          </div>
        </div>
      </div>
    </section>
  </div>

  <div class="ob-nav">
    <button class="bnb-btn ghost ob-hidden-init" id="ob-back" type="button">← Back</button>
    <div class="bnb-meta">Step <span id="ob-cur">1</span> of 6 · <a href="#" id="ob-skip">Skip for now</a></div>
    <a class="bnb-btn primary" id="ob-next" href="#" role="button">Continue →</a>
  </div>
  </form>
</div>

<script>
(function () {
  var step = 1, total = 6;
  var form = document.getElementById('ob-form');
  var stepsEls = document.querySelectorAll('.ob-step');
  var pips = document.querySelectorAll('.ob-pip');
  var back = document.getElementById('ob-back');
  var next = document.getElementById('ob-next');
  var skip = document.getElementById('ob-skip');
  var cur = document.getElementById('ob-cur');

  function render() {
    stepsEls.forEach(function (s) { s.classList.toggle('active', +s.dataset.step === step); });
    pips.forEach(function (p) {
      var n = +p.dataset.pip;
      p.classList.toggle('active', n === step);
      p.classList.toggle('done', n < step);
    });
    cur.textContent = step;
    back.style.visibility = step === 1 ? 'hidden' : 'visible';
    next.textContent = step === total ? 'Finish & go to dashboard →' : 'Continue →';
    // Auto-detect fills the coordinate inputs programmatically, which fires no
    // `input` event — so recompute here too, or the summary would still read
    // "Not set" for a station that just detected its location.
    refreshSummary();
  }
  function finish() { form.requestSubmit(); }

  back.addEventListener('click', function () { if (step > 1) { step--; render(); } });
  next.addEventListener('click', function (e) {
    e.preventDefault();
    if (step < total) { step++; render(); } else { finish(); }
  });
  skip.addEventListener('click', function (e) { e.preventDefault(); finish(); });

  // Single-select radio cards; mirror the chosen value into the form's hidden
  // input for that group. Keyed by data-radio so a new group only needs an
  // entry here — the confidence group was added this way.
  var mirrors = { notify: 'ob-notify', conf: 'ob-conf' };

  // Select the card matching what the station already has. The `sel` class
  // used to be baked into the "Balanced 0.75" and "New species this week"
  // cards, which told an operator with a different setting that they had
  // chosen the default — and, because the mirrored hidden input carried that
  // same hardcoded value, completing setup then wrote it over theirs. The
  // hidden inputs are now rendered server-side from the settings table and are
  // the single source of truth; the cards follow them.
  //
  // A stored value with no matching card (a threshold hand-set to 0.55, say)
  // deliberately leaves every card unselected rather than highlighting a wrong
  // one. The hidden input still holds the real value, so finishing the wizard
  // preserves it.
  Object.keys(mirrors).forEach(function (group) {
    var input = document.getElementById(mirrors[group]);
    if (!input) { return; }
    document.querySelectorAll('[data-radio="' + group + '"]').forEach(function (c) {
      if (c.dataset.value === input.value) { c.classList.add('sel'); }
    });
  });

  document.querySelectorAll('[data-radio]').forEach(function (card) {
    card.addEventListener('click', function () {
      document.querySelectorAll('[data-radio="' + card.dataset.radio + '"]').forEach(function (c) {
        c.classList.remove('sel');
      });
      card.classList.add('sel');
      var target = mirrors[card.dataset.radio];
      var input = target && document.getElementById(target);
      if (input && card.dataset.value) { input.value = card.dataset.value; }
      refreshSummary();
    });
  });

  // Keep the final step's summary card describing THIS station rather than a
  // mock-up. The microphone row is filled server-side from the real capture
  // sources; the rest is whatever the operator has entered so far, so it is
  // recomputed on every change and again on the way into the last step.
  function cardName(sel) {
    var c = document.querySelector(sel + '.sel .t');
    return c ? c.textContent.trim().replace(/\s*recommended$/, '') : '';
  }
  function refreshSummary() {
    var conf = document.getElementById('ob-conf');
    var sumConf = document.getElementById('ob-sum-conf');
    if (conf && sumConf) {
      var name = cardName('[data-radio="conf"]');
      var n = parseFloat(conf.value);
      sumConf.textContent = (isNaN(n) ? conf.value : n.toFixed(2)) + (name ? ' · ' + name : '');
    }
    var sumNotify = document.getElementById('ob-sum-notify');
    if (sumNotify) {
      var nm = cardName('[data-radio="notify"]');
      if (nm) { sumNotify.textContent = nm; }
    }
    var sumLoc = document.getElementById('ob-sum-loc');
    if (sumLoc) {
      var la = (document.getElementById('ob-lat') || {}).value;
      var lo = (document.getElementById('ob-lon') || {}).value;
      la = (la || '').trim(); lo = (lo || '').trim();
      // Both halves are required — one alone disables the species filter just
      // as completely as neither, so half a location is not a location.
      sumLoc.textContent = (la && lo) ? (la + ', ' + lo) : 'Not set — species filtering stays off';
    }
    var sumUrl = document.getElementById('ob-sum-url');
    // The address the operator actually reached this page on. The hard-coded
    // mDNS URL this replaced does not resolve on every network.
    if (sumUrl) { sumUrl.textContent = window.location.origin + '/'; }
  }
  ['ob-lat', 'ob-lon'].forEach(function (id) {
    var el = document.getElementById(id);
    if (el) { el.addEventListener('input', refreshSummary); }
  });
  refreshSummary();

  // Auto-detect coordinates + timezone via the existing settings endpoint
  // (same-origin fetch; the endpoint itself queries ip-api.com server-side).
  var detect = document.getElementById('ob-detect');
  var pill = document.getElementById('ob-loc-pill');
  if (detect) {
    detect.addEventListener('click', function () {
      var prev = detect.textContent;
      detect.disabled = true;
      detect.textContent = 'Detecting…';
      fetch('/admin/settings/detect-location')
        .then(function (r) { if (!r.ok) { throw new Error('lookup failed'); } return r.json(); })
        .then(function (d) {
          if (d.lat != null) { document.getElementById('ob-lat').value = d.lat; }
          if (d.lon != null) { document.getElementById('ob-lon').value = d.lon; }
          if (d.timezone) { document.getElementById('ob-tz').value = d.timezone; }
          if (pill) {
            var where = [d.city, d.country].filter(Boolean).join(', ');
            pill.textContent = '✓ ' + (where || 'Location found') + (d.timezone ? ' · ' + d.timezone : '');
            pill.classList.add('moss');
          }
        })
        .catch(function () {
          if (pill) { pill.textContent = 'Could not auto-detect — enter your coordinates manually.'; }
        })
        .finally(function () { detect.disabled = false; detect.textContent = prev; });
    });
  }

  render();
})();
</script>
</body>
</html>"##;
