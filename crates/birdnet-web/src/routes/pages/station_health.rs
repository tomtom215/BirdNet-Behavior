//! The Station **Health** surface — the operator's "is it working?" screen.
//!
//! Composed for the public `/station` Health tab ([`super::homes::station`]),
//! the heir to the read-only `/system` page. It gathers one snapshot and
//! renders it in the v3 `st-*` treatment: an overall status banner, a
//! per-source activity panel, a vitals row (CPU · memory · temperature ·
//! df-correct disk), a pipeline row (last detection · queued uploads · service
//! uptime) and a short diagnostics checklist.
//!
//! Honest by construction: when the capture supervisor is running it publishes
//! live per-source health into [`AppState::capture_status`], so each card shows
//! a real state chip (Live · Stalled · Backing off · Paused), a rolling 24 h
//! uptime strip, the time since last audio, and a retry/backoff line. With no
//! supervisor (web-only mode, or tooling) the panel falls back to **activity**
//! from `detections.Source` — how many detections each source produced today
//! and how recently — never a faked live/stalled chip. Everything shown is real.

use std::fmt::Write as _;

use birdnet_core::audio::capture::{SourceState, SourceStatus, UptimeSegment, read_capture_status};
use birdnet_db::audio_sources::AudioSourceStore;
use birdnet_db::sqlite::SourceActivity;

use super::escape_html;
use crate::metrics::OccurrenceFilterState;
use crate::state::AppState;
use crate::system_info::{self, format_bytes, format_uptime};

/// One vitals tile: CPU / memory / temperature / disk.
struct Vital {
    label: &'static str,
    value: String,
    /// Meter fill 0–100, or `None` for a value with no natural ratio.
    pct: Option<f64>,
    sub: String,
    /// `true` when the metric is in a warning band (amber meter).
    warn: bool,
}

/// Everything the Health surface needs, gathered in one blocking pass.
// A private, render-only snapshot whose flags are independent health signals
// (disk + scratch low/critical, integrity); grouping them into enums would not
// make the render any clearer. Matches the convention already used in src/cli.rs.
#[allow(clippy::struct_excessive_bools)]
struct Snapshot {
    vitals: Vec<Vital>,
    /// Configured audio sources (count only — the panel keys on activity).
    sources_configured: usize,
    /// `stream id -> the name its owner typed`, for the sources that have one.
    ///
    /// The supervisor publishes `SourceStatus::label`, which is the stream id
    /// (`CaptureSource::label`) — an identity shared with the gauge label and
    /// the segment filename, and so not ours to change. But it is not what the
    /// Capture screen shows, and a station with two cameras reading `RTSP_1`
    /// and `RTSP_2` here and `Front-yard` and `Pond` there leaves its owner
    /// unable to tell which camera is the one that stopped.
    source_names: std::collections::HashMap<String, String>,
    /// Live per-source health from the capture supervisor. Empty when no
    /// supervisor is running, in which case the panel falls back to `activity`.
    capture: Vec<SourceStatus>,
    activity: Vec<SourceActivity>,
    last_detection: Option<u64>,
    queued_uploads: u64,
    total_detections: i64,
    integrity_ok: bool,
    disk_low: bool,
    disk_critical: bool,
    /// `/tmp` (scratch / stream-buffer) pressure, tracked separately from the
    /// data disk because it is usually a small, RAM-backed tmpfs that the data
    /// "Disk" tile does not cover.
    scratch_low: bool,
    scratch_critical: bool,
    service_uptime: Option<u64>,
    /// Per-source acoustic health. Empty until the sampler has run.
    acoustic: Vec<birdnet_db::audio_levels::SourceDrift>,
    /// What the species occurrence filter is doing (ON-12).
    occurrence: OccurrenceFilterState,
}

/// Render the operator Health surface for the public Station Health tab.
pub(super) async fn content(state: &AppState) -> String {
    let snap = gather(state).await;
    render(&snap)
}

/// Gather the snapshot on the blocking pool (CPU sampling, `statvfs`, DB reads).
async fn gather(state: &AppState) -> Snapshot {
    let state = state.clone();
    tokio::task::spawn_blocking(move || {
        let sys = system_info::sample();
        let data_dir = state.db_path().parent().map_or_else(
            || state.db_path().to_path_buf(),
            std::path::Path::to_path_buf,
        );
        let disk = birdnet_core::audio::capture::disk_usage(&data_dir).ok();

        // Scratch space: the service writes its live audio stream segments and
        // temp files under /tmp, which on a Pi (and any tmpfs /tmp) is a small,
        // RAM-backed filesystem separate from the data disk. The "Disk" tile
        // watches the data partition, so a filling /tmp was invisible here — yet
        // a full /tmp breaks the capture pipeline (and even `apt`). Track it
        // separately, but only when it is genuinely a different filesystem than
        // the data disk; when /tmp lives on the data partition the Disk tile
        // already covers it and a second identical tile would just be noise.
        let scratch = birdnet_core::audio::capture::disk_usage(&std::env::temp_dir())
            .ok()
            .filter(|s| disk.as_ref().is_none_or(|d| d.total_bytes != s.total_bytes));

        let (sources_configured, source_names, activity, last_detection, queued, total, integrity) =
            state.with_db(|conn| {
                let listed = AudioSourceStore::list(conn).unwrap_or_default();
                let sources = listed.iter().filter(|x| x.disabled_at.is_none()).count();
                let names: std::collections::HashMap<String, String> = listed
                    .iter()
                    .filter(|x| x.disabled_at.is_none())
                    .filter_map(|x| {
                        let label = x.label.as_deref()?.trim();
                        (!label.is_empty() && label != x.id)
                            .then(|| (x.id.clone(), label.to_owned()))
                    })
                    .collect();
                let activity =
                    birdnet_db::sqlite::todays_source_activity(conn, &super::today_date_string())
                        .unwrap_or_default();
                let last = birdnet_db::sqlite::seconds_since_last_detection(conn)
                    .ok()
                    .flatten();
                let queued = birdnet_db::outbound_queue::depth(
                    conn,
                    birdnet_integrations::birdweather::QUEUE_KIND,
                )
                .unwrap_or(0);
                let total = birdnet_db::sqlite::detection_count(conn).unwrap_or(0);
                let integrity = birdnet_db::sqlite::quick_check(conn).unwrap_or(false);
                (sources, names, activity, last, queued, total, integrity)
            });

        // Live capture-supervisor health, when a supervisor is publishing it.
        let capture = state
            .capture_status()
            .map(|handle| read_capture_status(&handle).sources)
            .unwrap_or_default();

        let vitals = build_vitals(&sys, disk.as_ref(), scratch.as_ref());
        Snapshot {
            vitals,
            sources_configured,
            source_names,
            capture,
            activity,
            last_detection,
            queued_uploads: queued,
            total_detections: total,
            integrity_ok: integrity,
            disk_low: disk
                .as_ref()
                .is_some_and(birdnet_core::audio::capture::DiskUsage::is_low),
            disk_critical: disk
                .as_ref()
                .is_some_and(birdnet_core::audio::capture::DiskUsage::is_critical),
            scratch_low: scratch
                .as_ref()
                .is_some_and(birdnet_core::audio::capture::DiskUsage::is_low),
            scratch_critical: scratch
                .as_ref()
                .is_some_and(birdnet_core::audio::capture::DiskUsage::is_critical),
            service_uptime: system_info::process_uptime_secs(),
            acoustic: state.with_db(|conn| {
                birdnet_db::audio_levels::drift_by_source(conn, 7, 30).unwrap_or_default()
            }),
            occurrence: state.metrics().occurrence_filter(),
        }
    })
    .await
    .unwrap_or_else(|_| Snapshot {
        vitals: Vec::new(),
        sources_configured: 0,
        source_names: std::collections::HashMap::new(),
        capture: Vec::new(),
        activity: Vec::new(),
        last_detection: None,
        queued_uploads: 0,
        total_detections: 0,
        // Fail-unsafe until this line: a snapshot we could not take asserted
        // that the database integrity check had *passed*. Every other field
        // here reads as "nothing to report", which is at worst uninformative;
        // this one manufactured a clean bill of health for the one page an
        // operator opens when they suspect something is wrong. `quick_check`
        // inside the closure above is already `unwrap_or(false)` for the same
        // reason, so the two directions no longer contradict each other.
        integrity_ok: false,
        disk_low: false,
        disk_critical: false,
        scratch_low: false,
        scratch_critical: false,
        service_uptime: None,
        acoustic: Vec::new(),
        occurrence: OccurrenceFilterState {
            active: false,
            candidates: None,
        },
    })
}

#[allow(
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn build_vitals(
    sys: &system_info::SystemSnapshot,
    disk: Option<&birdnet_core::audio::capture::DiskUsage>,
    scratch: Option<&birdnet_core::audio::capture::DiskUsage>,
) -> Vec<Vital> {
    let cpu = sys.cpu_usage_pct as f64;
    let mem = sys.memory_usage_pct as f64;
    let mut vitals = vec![
        Vital {
            label: "CPU",
            value: format!("{cpu:.0}%"),
            pct: Some(cpu),
            sub: format!("{} cores", sys.cpu_count),
            warn: cpu > 80.0,
        },
        Vital {
            label: "Memory",
            value: format!("{mem:.0}%"),
            pct: Some(mem),
            sub: sys.memory_summary(),
            warn: mem > 85.0,
        },
    ];
    vitals.push(sys.cpu_temp_celsius.map_or_else(
        || Vital {
            label: "Temperature",
            value: "—".to_string(),
            pct: None,
            sub: "no sensor".to_string(),
            warn: false,
        },
        |t| {
            let t = f64::from(t);
            Vital {
                label: "Temperature",
                value: format!("{t:.0}°C"),
                pct: Some((t / 90.0 * 100.0).clamp(0.0, 100.0)),
                sub: "core".to_string(),
                warn: t > 70.0,
            }
        },
    ));
    vitals.push(disk.map_or_else(
        || Vital {
            label: "Disk",
            value: "—".to_string(),
            pct: None,
            sub: "unavailable".to_string(),
            warn: false,
        },
        |d| {
            let pct = d.used_percent();
            Vital {
                label: "Disk",
                value: format!("{pct:.0}%"),
                pct: Some(pct),
                sub: format!(
                    "{} free of {}",
                    format_bytes(d.available_bytes),
                    format_bytes(d.total_bytes)
                ),
                warn: d.is_low(),
            }
        },
    ));
    // Scratch (RAM-backed /tmp), only when distinct from the data disk. Surfaced
    // so a filling tmpfs — which silently breaks capture and system updates — is
    // visible on the same screen as the data Disk, not just discoverable via
    // `df` on the box.
    if let Some(d) = scratch {
        let pct = d.used_percent();
        vitals.push(Vital {
            label: "Scratch",
            value: format!("{pct:.0}%"),
            pct: Some(pct),
            sub: format!(
                "{} free of {} (RAM)",
                format_bytes(d.available_bytes),
                format_bytes(d.total_bytes)
            ),
            warn: d.is_low(),
        });
    }
    vitals
}

/// Compose the full surface from the snapshot.
fn render(s: &Snapshot) -> String {
    format!(
        "<p class=\"bnb-lede\"><b>Everything your station needs to keep listening</b> — the \
         streams, the hardware, and the pipeline behind them. This is the screen to check from \
         the field. {help}</p>{banner}<h2 class=\"st-h3\">Audio sources</h2>{sources}\
         <h2 class=\"st-h3\">Vitals</h2>{vitals}{acoustic}<h2 class=\"st-h3\">Pipeline</h2>{pipeline}\
         <h2 class=\"st-h3\">Diagnostics</h2>{checks}",
        help = super::help::help_link(super::help::Topic::AdminSystem),
        banner = status_banner(s),
        sources = source_panel(s),
        vitals = vitals_row(&s.vitals),
        acoustic = acoustic_panel(s),
        pipeline = pipeline_row(s),
        checks = diagnostics(s),
    )
}

/// What to call a source on this screen: the name its owner gave it, else the
/// stream id the supervisor publishes.
fn display_name(s: &Snapshot, label: &str) -> String {
    s.source_names
        .get(label)
        .map_or_else(|| label.to_owned(), Clone::clone)
}

/// Name the sources in `faulty`, or count them once there are too many to read.
fn name_sources(s: &Snapshot, faulty: &[&SourceStatus]) -> String {
    match faulty {
        [] => String::new(),
        [one] => display_name(s, &one.label),
        [a, b] => format!(
            "{} and {}",
            display_name(s, &a.label),
            display_name(s, &b.label)
        ),
        many => format!("{} audio sources", many.len()),
    }
}

/// The overall status banner — green unless a real problem is present.
fn status_banner(s: &Snapshot) -> String {
    let mut issues: Vec<String> = Vec::new();
    if s.disk_critical {
        issues.push("storage is critically low".to_owned());
    } else if s.disk_low {
        issues.push("storage is running low".to_owned());
    }
    // "scratch space (RAM /tmp)" named the implementation. What the reader has
    // is a temporary folder that recordings pass through on their way to disk.
    if s.scratch_critical {
        issues.push("the temporary recording folder is critically low on space".to_owned());
    } else if s.scratch_low {
        issues.push("the temporary recording folder is running low on space".to_owned());
    }
    if !s.integrity_ok {
        issues.push("the database integrity check failed".to_owned());
    }
    if s.sources_configured == 0 {
        issues.push("no microphone or camera is set up yet".to_owned());
    }
    if s.queued_uploads > 0 {
        issues.push("uploads are waiting for the network".to_owned());
    }
    // Named, not counted. "An audio source is down" told the owner of two
    // cameras that one of them had a problem and left them to work out which.
    let faulted: Vec<&SourceStatus> = s.capture.iter().filter(|c| c.state.is_fault()).collect();
    if !faulted.is_empty() {
        issues.push(format!(
            "{} stopped sending audio",
            name_sources(s, &faulted)
        ));
    }
    let flapping: Vec<&SourceStatus> = s.capture.iter().filter(|c| c.flapping).collect();
    if !flapping.is_empty() {
        issues.push(format!(
            "{} keeps dropping out and reconnecting",
            name_sources(s, &flapping)
        ));
    }
    if s.occurrence.admits_nothing() {
        issues.push("the species filter admits no species, so nothing can be recorded".to_owned());
    }

    let last = s
        .last_detection
        .map_or_else(|| "no detections yet".to_string(), format_freshness);
    if issues.is_empty() {
        let queued = if s.queued_uploads == 0 {
            "no items queued".to_string()
        } else {
            format!("{} queued", s.queued_uploads)
        };
        format!(
            "<div class=\"st-status\"><span class=\"ico\" aria-hidden=\"true\">✓</span><div>\
             <div class=\"t\">All systems healthy</div>\
             <div class=\"s\">{n} source(s) active today · last detection {last} · {queued}</div>\
             </div></div>",
            n = s.activity.len(),
        )
    } else {
        format!(
            "<div class=\"st-status warn\"><span class=\"ico\" aria-hidden=\"true\">!</span><div>\
             <div class=\"t\">Needs attention</div>\
             <div class=\"s\">{}</div></div></div>",
            escape_html(&capitalize_first(&issues.join(" · "))),
        )
    }
}

/// The per-source panel: live supervisor cards when available, else the
/// detection-derived activity fallback.
fn source_panel(s: &Snapshot) -> String {
    if !s.capture.is_empty() {
        return live_source_panel(s);
    }
    if s.activity.is_empty() {
        return "<div class=\"bnb-card pad st-source-empty\">No detections yet today. Sources \
                appear here once they start classifying birds.</div>"
            .to_string();
    }
    let mut out = String::from("<div class=\"st-sources\">");
    for a in &s.activity {
        let name = a
            .source
            .as_deref()
            .filter(|x| !x.trim().is_empty())
            .map_or_else(|| "Unlabelled source".to_string(), escape_html);
        let last = a.last_time.as_deref().map_or_else(
            || "—".to_string(),
            |t| escape_html(t.get(0..5).unwrap_or(t)),
        );
        let _ = write!(
            out,
            "<div class=\"bnb-card st-source\"><div class=\"st-source-head\">\
             <div><div class=\"st-source-name\">{name}</div>\
             <div class=\"st-source-type\">audio source</div></div>\
             <span class=\"bnb-pill moss\"><span class=\"bnb-dot live\"></span> active today</span>\
             </div><div class=\"st-source-foot\"><span><b>{last}</b> · last detection</span>\
             <span><b>{count}</b> · detections today</span></div></div>",
            count = a.count,
        );
    }
    out.push_str("</div>");
    out
}

/// The operator-grade per-source panel, driven by live supervisor state: a
/// status chip, a rolling 24 h uptime strip, time since last audio, today's
/// detection count (matched from `activity` by label), and a retry line.
fn live_source_panel(s: &Snapshot) -> String {
    let mut out = String::from("<div class=\"st-sources\">");
    for src in &s.capture {
        // Today's detections for this source, matched by label to the DB
        // activity (the gauge label and `detections.Source` tag coincide).
        let today = s
            .activity
            .iter()
            .find(|a| a.source.as_deref() == Some(src.label.as_str()))
            .map_or(0, |a| a.count);
        out.push_str(&source_card(src, today, s.source_names.get(&src.label)));
    }
    out.push_str("</div>");
    out
}

/// One live source card.
///
/// `friendly` is the name its owner gave the source on the Capture screen, when
/// they gave it one. It leads, and the stream id becomes the second line — the
/// id still has to be visible, because it is what the filenames, the metrics
/// and the logs all call this source.
fn source_card(src: &SourceStatus, today: i64, friendly: Option<&String>) -> String {
    let stalled = if src.state == SourceState::Stalled {
        " stalled"
    } else {
        ""
    };
    let last_audio = src
        .last_audio_age_secs
        .map_or_else(|| "—".to_string(), format_freshness);
    let (name, sub) = friendly.map_or_else(
        || (escape_html(&src.label), "audio source".to_string()),
        |f| (escape_html(f), escape_html(&src.label)),
    );
    format!(
        "<div class=\"bnb-card st-source{stalled}\"><div class=\"st-source-head\">\
         <div><div class=\"st-source-name\">{name}</div>\
         <div class=\"st-source-type\">{sub}</div></div>{chip}</div>\
         {strip}<div class=\"st-source-foot\"><span><b>{last_audio}</b> · last audio</span>\
         <span><b>{today}</b> · detections today</span></div>{retry}{flap}</div>",
        chip = source_chip(src.state),
        strip = uptime_strip(&src.uptime_24h),
        retry = retry_line(src),
        flap = restarts_line(src),
    )
}

/// The restart-count line (AD-3): shown whenever the source restarted in the
/// last hour, whatever it reads now. A source that dies every few minutes and
/// comes back in seconds is `Live` with `restart_attempts` 0 at every glance
/// and paints its strip green; this line is where that shows.
fn restarts_line(src: &SourceStatus) -> String {
    if src.restarts_last_hour == 0 {
        return String::new();
    }
    let verdict = if src.flapping {
        " — flapping: check the cable, hub power or stream"
    } else {
        ""
    };
    format!(
        "<div class=\"st-source-retry\">\u{21bb} restarted {}× in the last hour{verdict}</div>",
        src.restarts_last_hour
    )
}

/// The status chip for a source's lifecycle state.
const fn source_chip(state: SourceState) -> &'static str {
    match state {
        SourceState::Connected => {
            "<span class=\"bnb-pill moss\"><span class=\"bnb-dot live\"></span> Live</span>"
        }
        SourceState::Stalled => {
            "<span class=\"bnb-pill rare\"><span class=\"bnb-dot rare\"></span> Stalled</span>"
        }
        // "Reconnecting", not "Backing off". Exponential backoff is the
        // mechanism, not what the owner of a garden microphone needs to read —
        // and `retry_line` has always described this same state as
        // "reconnecting", so one card named one state two ways, one of them in
        // a vocabulary no reader of this screen shares.
        SourceState::BackingOff => {
            "<span class=\"bnb-pill dawn\"><span class=\"bnb-dot dawn\"></span> Reconnecting</span>"
        }
        SourceState::Paused => {
            "<span class=\"bnb-pill\"><span class=\"bnb-dot\"></span> Paused</span>"
        }
    }
}

/// The 48-segment rolling 24 h uptime strip, with a screen-reader / hover
/// summary of the uptime percentage.
fn uptime_strip(segments: &[UptimeSegment]) -> String {
    let observed = segments
        .iter()
        .filter(|seg| !matches!(seg, UptimeSegment::Out))
        .count();
    let up = segments
        .iter()
        .filter(|seg| matches!(seg, UptimeSegment::Up))
        .count();
    let summary = (up * 100).checked_div(observed).map_or_else(
        || "24-hour uptime — no data yet".to_string(),
        |pct| format!("24-hour uptime — {pct}% over the last {observed} half-hours"),
    );
    let summary = escape_html(&summary);
    let mut strip = format!(
        "<div class=\"st-uptime\" role=\"img\" aria-label=\"{summary}\" title=\"{summary}\">"
    );
    for seg in segments {
        let cls = match seg {
            UptimeSegment::Up => "up",
            UptimeSegment::Down => "down",
            UptimeSegment::Out => "out",
        };
        let _ = write!(strip, "<span class=\"{cls}\"></span>");
    }
    strip.push_str("</div>");
    strip
}

/// The retry/backoff line for a faulted source (empty when healthy or paused).
fn retry_line(src: &SourceStatus) -> String {
    if !src.state.is_fault() {
        return String::new();
    }
    let verb = if src.state == SourceState::Stalled {
        "stalled"
    } else {
        "reconnecting"
    };
    let attempt = if src.restart_attempts > 0 {
        format!(" · attempt {}", src.restart_attempts)
    } else {
        String::new()
    };
    let next = src
        .next_retry_in_secs
        .map_or_else(String::new, |secs| format!(" · next try in {secs}s"));
    format!("<div class=\"st-source-retry\">\u{21bb} {verb}{attempt}{next}</div>")
}

/// The four-up vitals meter row.
fn vitals_row(vitals: &[Vital]) -> String {
    let mut out = String::from("<div class=\"st-vitals\">");
    for v in vitals {
        let meter = v.pct.map_or_else(String::new, |p| {
            let cls = if v.warn { " warn" } else { "" };
            format!(
                "<div class=\"meter\"><span class=\"st-meter-fill{cls}\" data-style=\"width:{p:.0}%\"></span></div>",
            )
        });
        let _ = write!(
            out,
            "<div class=\"st-vital\"><div class=\"lab\">{label}</div>\
             <div class=\"v\">{value}</div>{meter}<div class=\"sub\">{sub}</div></div>",
            label = v.label,
            value = escape_html(&v.value),
            sub = escape_html(&v.sub),
        );
    }
    out.push_str("</div>");
    out
}

/// The pipeline row: last detection · queued uploads · service uptime · total.
fn pipeline_row(s: &Snapshot) -> String {
    let last = s
        .last_detection
        .map_or_else(|| "no detections yet".to_string(), format_freshness);
    let queued = if s.queued_uploads == 0 {
        "<span class=\"mono\">0</span> · all delivered".to_string()
    } else {
        format!(
            "<span class=\"mono\">{}</span> · awaiting network",
            s.queued_uploads
        )
    };
    let uptime = s
        .service_uptime
        .map_or_else(|| "—".to_string(), |u| escape_html(&format_uptime(u)));
    format!(
        "<div class=\"st-pipe\">\
         <div><div class=\"lab\">Last detection</div><div class=\"v\">{last}</div></div>\
         <div><div class=\"lab\">Queued uploads</div><div class=\"v\">{queued}</div></div>\
         <div><div class=\"lab\">Service uptime</div><div class=\"v\"><span class=\"mono\">{uptime}</span></div></div>\
         <div><div class=\"lab\">Total detections</div><div class=\"v\"><span class=\"mono\">{total}</span></div></div>\
         <div><div class=\"lab\">Species filter</div><div class=\"v\">{filter}</div></div>\
         </div>",
        total = super::group_thousands(s.total_detections),
        filter = occurrence_cell(s.occurrence),
    )
}

/// The species-filter cell of the pipeline row: the one number that shows a
/// filter admitting nothing, which used to reach only Prometheus.
fn occurrence_cell(f: OccurrenceFilterState) -> String {
    match (f.active, f.candidates) {
        (false, _) => "off · every species the model knows is a candidate".to_string(),
        (true, None) => "on · has not run yet".to_string(),
        (true, Some(0)) => {
            "<span class=\"mono\">0</span> species admitted · nothing can be recorded".to_string()
        }
        (true, Some(n)) => format!("on · admitting <span class=\"mono\">{n}</span> species"),
    }
}

/// What the microphones themselves sound like, and whether that has moved.
///
/// # Why this is on the health page and not an analytics one
///
/// It is not about the birds. A microphone that goes deaf — water, a web across
/// the port, a connector loosened over a year of thermal cycling — keeps its
/// process alive and its `audio_source_up` gauge at 1, and presents only as
/// fewer detections. So does the end of the season. This is the one number that
/// tells them apart, and it belongs beside the other things an operator checks
/// from the field.
///
/// Silent until the sampler has produced something, because a panel of dashes
/// on a station that has been running an hour teaches people to skip it.
fn acoustic_panel(s: &Snapshot) -> String {
    if s.acoustic.is_empty() {
        return String::new();
    }
    let mut rows = String::new();
    for d in &s.acoustic {
        // Deliberately not a verdict. Without a season of real recordings there
        // is no calibrated threshold, and a made-up one on the health page is a
        // false alarm waiting for its first thunderstorm. The number and its
        // own history are shown; the reading is the operator's.
        let moved = d.moved_db().map_or_else(
            || "<span class=\"bnb-meta\">building a baseline</span>".to_string(),
            |m| {
                format!(
                    "<span class=\"mono\">{sign}{m:.1} dB</span> <span class=\"bnb-meta\">vs its own 30-day average</span>",
                    sign = if m >= 0.0 { "+" } else { "" }
                )
            },
        );
        let _ = std::fmt::Write::write_fmt(
            &mut rows,
            format_args!(
                "<div><div class=\"lab\">{name}</div>\
                 <div class=\"v\"><span class=\"mono\">{recent:.1} dBFS</span></div>\
                 <div class=\"bnb-meta\">{moved} · {samples} {noun}</div></div>",
                name = escape_html(&d.source),
                recent = d.recent_dbfs,
                samples = d.recent_samples,
                noun = if d.recent_samples == 1 {
                    "sample"
                } else {
                    "samples"
                },
            ),
        );
    }
    format!(
        "<h2 class=\"st-h3\">Microphone health <span class=\"st-h3-note\">· background noise floor, last 7 days</span></h2>\
         <div class=\"st-pipe\">{rows}</div>\
         <p class=\"bnb-meta\">A microphone going deaf and a season going quiet both show up as fewer \
         detections. The background noise floor does not stop when the birds do, so a large, sustained \
         <em>drop</em> here — with nothing else changed — points at the equipment rather than the wood.</p>"
    )
}

/// A short, honest diagnostics checklist with a link to the full doctor page.
fn diagnostics(s: &Snapshot) -> String {
    let row = |ok: bool, title: &str, detail: &str| {
        let (mk, val) = if ok { ("✓", "OK") } else { ("!", "Check") };
        let cls = if ok { "" } else { " warn" };
        format!(
            "<div class=\"st-check-row\"><span class=\"mk{cls}\">{mk}</span>\
             <div class=\"c\"><div class=\"t\">{t}</div><div class=\"d\">{d}</div></div>\
             <span class=\"val\">{val}</span></div>",
            t = escape_html(title),
            d = escape_html(detail),
        )
    };
    // The supervisor's own verdict, not merely "is anything configured".
    // This row used to ask `sources_configured > 0` alone, which ticked green
    // on the same screen as a warning banner reading "an audio source is down"
    // and two source cards chipped Stalled / Backing off. When a supervisor is
    // publishing, its faults decide this row; without one, the old question is
    // still the only one we can answer.
    let faulted = s.capture.iter().filter(|c| c.state.is_fault()).count();
    let flapping = s.capture.iter().filter(|c| c.flapping).count();
    let sources_ok = s.sources_configured > 0 && faulted == 0 && flapping == 0;
    let sources_detail = if faulted > 0 {
        format!(
            "{} configured · {faulted} not reporting",
            s.sources_configured
        )
    } else if flapping > 0 {
        format!(
            "{} configured · {flapping} keeps dropping and reconnecting",
            s.sources_configured
        )
    } else {
        format!(
            "{} configured · {} active today",
            s.sources_configured,
            s.activity.len()
        )
    };
    let disk_detail = if s.disk_critical {
        "critically low — recordings may stop"
    } else if s.disk_low {
        "running low — auto-purge will reclaim space"
    } else {
        "ample headroom"
    };
    format!(
        "<div class=\"st-check\">{a}{b}{c}</div>\
         <p class=\"bnb-meta st-doctor-link\">Configuration checks live in \
         <a href=\"/admin/doctor\">Diagnostics</a> (sign-in required).</p>",
        a = row(sources_ok, "Audio sources", &sources_detail),
        b = row(
            !s.disk_low && !s.disk_critical,
            "Disk headroom",
            disk_detail
        ),
        c = row(
            s.integrity_ok,
            "Database integrity",
            // `quick_check` is SQLite's name for the pragma this reports, not
            // a word this screen's reader has ever met.
            if s.integrity_ok {
                "no damage found"
            } else {
                "damage found — restore from a backup"
            }
        ),
    )
}

/// Human "X ago" for the seconds-since-last-detection freshness signal.
fn format_freshness(secs: u64) -> String {
    match secs {
        0..=119 => "just now".to_string(),
        120..=7_199 => format!("{} min ago", secs / 60),
        7_200..=172_799 => format!("{} h ago", secs / 3_600),
        _ => format!("{} days ago", secs / 86_400),
    }
}

/// Upper-case the first character (for the warn banner's joined issue list).
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().collect::<String>() + chars.as_str()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(disk_low: bool, integrity: bool, sources: usize, queued: u64) -> Snapshot {
        Snapshot {
            vitals: Vec::new(),
            sources_configured: sources,
            source_names: std::collections::HashMap::new(),
            capture: Vec::new(),
            activity: Vec::new(),
            last_detection: Some(30),
            queued_uploads: queued,
            total_detections: 100,
            integrity_ok: integrity,
            disk_low,
            disk_critical: false,
            scratch_low: false,
            scratch_critical: false,
            service_uptime: Some(3_600),
            acoustic: Vec::new(),
            occurrence: OccurrenceFilterState {
                active: false,
                candidates: None,
            },
        }
    }

    /// The gate for ON-12: the occurrence filter's state is on the station
    /// page, and a filter admitting nothing is flagged as a problem.
    #[test]
    fn the_species_filter_state_is_on_the_page_and_zero_admitted_is_a_problem() {
        let mut s = snap(false, true, 2, 0);
        assert!(
            pipeline_row(&s).contains("every species the model knows is a candidate"),
            "an inactive filter must say so: {}",
            pipeline_row(&s)
        );
        assert!(status_banner(&s).contains("All systems healthy"));

        s.occurrence = OccurrenceFilterState {
            active: true,
            candidates: Some(287),
        };
        assert!(
            pipeline_row(&s).contains("admitting <span class=\"mono\">287</span> species"),
            "{}",
            pipeline_row(&s)
        );
        assert!(status_banner(&s).contains("All systems healthy"));

        s.occurrence = OccurrenceFilterState {
            active: true,
            candidates: Some(0),
        };
        assert!(
            pipeline_row(&s).contains("nothing can be recorded"),
            "{}",
            pipeline_row(&s)
        );
        assert!(
            status_banner(&s).contains("admits no species"),
            "a filter admitting nothing is a problem the banner must name: {}",
            status_banner(&s)
        );
    }

    fn cap_source(
        label: &str,
        state: SourceState,
        attempts: u32,
        next: Option<u64>,
    ) -> SourceStatus {
        SourceStatus {
            label: label.into(),
            state,
            uptime_secs: None,
            last_audio_age_secs: Some(5),
            restart_attempts: attempts,
            restarts_last_hour: 0,
            flapping: false,
            next_retry_in_secs: next,
            uptime_24h: vec![UptimeSegment::Up, UptimeSegment::Down, UptimeSegment::Out],
        }
    }

    /// AD-3. A source that dies and comes back within seconds reads Live,
    /// The name on this card has to be the one its owner chose.
    ///
    /// The supervisor publishes `SourceStatus::label`, which is the stream id
    /// (`CaptureSource::label`) — an identity it shares with the metrics gauge
    /// and the segment filename, so it is not ours to rename. But the Capture
    /// screen shows the friendly label, and this screen showed the id: a
    /// station whose owner named two cameras "Front-yard" and "Pond" was told
    /// here that `RTSP_1` had stopped, which is not a sentence they can act
    /// on. The id stays visible as the second line, because it is what the
    /// filenames and the logs call this source.
    #[test]
    fn a_source_card_leads_with_the_name_its_owner_gave_it() {
        let src = cap_source("RTSP_1", SourceState::Connected, 0, None);
        let friendly = "Front-yard camera".to_string();
        let card = source_card(&src, 3, Some(&friendly));
        assert!(
            card.contains(">Front-yard camera</div>"),
            "the operator's own name must be the card's title: {card}"
        );
        assert!(
            card.contains(">RTSP_1</div>"),
            "the stream id must stay visible — it is what the filenames and \
             the logs call this source: {card}"
        );
    }

    /// The counterpart. Most stations never label anything, and a card that
    /// fell back to an empty title would pass the test above.
    #[test]
    fn an_unlabelled_source_still_names_itself() {
        let src = cap_source("RTSP_1", SourceState::Connected, 0, None);
        let card = source_card(&src, 3, None);
        assert!(
            card.contains(">RTSP_1</div>"),
            "an unlabelled source must still be named by its id: {card}"
        );
        assert!(
            card.contains(">audio source</div>"),
            "and keep its generic subtitle: {card}"
        );
    }

    /// One card must not name one state two ways.
    ///
    /// `source_chip` said "Backing off" — the name of the retry algorithm —
    /// while `retry_line`, one line below it in the same card, said
    /// "reconnecting". The reader of this screen shares neither vocabulary,
    /// and had to reconcile two words for one fact.
    #[test]
    fn a_reconnecting_source_is_described_the_same_way_twice() {
        let src = cap_source("RTSP_1", SourceState::BackingOff, 3, Some(12));
        let card = source_card(&src, 0, None);
        assert!(
            card.contains("Reconnecting"),
            "the chip must say what is happening in plain words: {card}"
        );
        assert!(
            card.contains("reconnecting"),
            "the retry line must still describe the same state: {card}"
        );
        assert!(
            !card.contains("Backing off"),
            "\"backing off\" is the algorithm's name, not the station's: {card}"
        );
    }

    /// attempt 0, strip green; the restart count over the last hour is the
    /// only number that shows it, so the card carries it and the banner calls
    /// it out.
    #[test]
    fn a_flapping_source_is_an_issue_though_it_reads_live() {
        let mut src = cap_source("local", SourceState::Connected, 0, None);
        src.restarts_last_hour = 7;
        src.flapping = true;
        let card = source_card(&src, 3, None);
        assert!(card.contains("restarted 7× in the last hour"), "{card}");
        assert!(card.contains("flapping"), "{card}");
        let mut s = snap(false, true, 2, 0);
        s.capture = vec![src];
        let banner = status_banner(&s);
        assert!(banner.contains("st-status warn"), "{banner}");
        assert!(
            banner.contains("keeps dropping out and reconnecting"),
            "{banner}"
        );

        // Counterpart: one restart is a line, not an issue.
        let mut once = cap_source("local", SourceState::Connected, 0, None);
        once.restarts_last_hour = 1;
        let card = source_card(&once, 3, None);
        assert!(
            card.contains("restarted 1× in the last hour") && !card.contains("flapping"),
            "{card}"
        );
        let mut s = snap(false, true, 2, 0);
        s.capture = vec![once];
        assert!(!status_banner(&s).contains("flapping"));
    }

    #[test]
    fn banner_is_green_only_when_nothing_is_wrong() {
        assert!(status_banner(&snap(false, true, 2, 0)).contains("All systems healthy"));
        assert!(!status_banner(&snap(false, true, 2, 0)).contains("st-status warn"));
    }

    #[test]
    fn banner_flags_each_real_problem() {
        assert!(status_banner(&snap(true, true, 2, 0)).contains("running low"));
        assert!(status_banner(&snap(false, false, 2, 0)).contains("integrity"));
        assert!(
            status_banner(&snap(false, true, 0, 0)).contains("microphone or camera is set up yet")
        );
        assert!(status_banner(&snap(false, true, 2, 3)).contains("waiting for the network"));
        // Any problem flips the banner to the warn variant.
        assert!(status_banner(&snap(true, true, 2, 0)).contains("st-status warn"));
    }

    #[test]
    fn banner_flags_low_scratch_independently_of_the_data_disk() {
        // A healthy data disk but a nearly-full RAM /tmp (the failure mode that
        // silently broke `apt`) must still raise the banner — the data Disk tile
        // alone would have read "healthy" and hidden it.
        let mut s = snap(false, true, 2, 0);
        s.scratch_critical = true;
        let banner = status_banner(&s);
        // `capitalize_first` upper-cases the leading word of the issue list, so
        // assert on the distinctive mid-string text rather than the first word.
        assert!(banner.contains("temporary recording folder is critically low"));
        assert!(banner.contains("st-status warn"));
    }

    #[test]
    fn freshness_rounds_to_operator_units() {
        assert_eq!(format_freshness(30), "just now");
        assert_eq!(format_freshness(600), "10 min ago");
        assert_eq!(format_freshness(7_200), "2 h ago");
        assert_eq!(format_freshness(200_000), "2 days ago");
    }

    #[test]
    fn empty_source_panel_is_honest_not_blank() {
        assert!(source_panel(&snap(false, true, 1, 0)).contains("No detections yet today"));
    }

    #[test]
    fn live_panel_shows_supervisor_chips_strip_and_retry() {
        let mut s = snap(false, true, 2, 0);
        s.capture = vec![
            cap_source("local", SourceState::Connected, 0, None),
            cap_source("RTSP_1", SourceState::BackingOff, 3, Some(12)),
            cap_source("RTSP_2", SourceState::Stalled, 1, None),
        ];
        let html = source_panel(&s);
        assert!(html.contains("Live"));
        // "Reconnecting", not "Backing off" — see
        // `a_reconnecting_source_is_described_the_same_way_twice`.
        assert!(html.contains("Reconnecting"));
        assert!(html.contains("Stalled"));
        // The 24h uptime strip and its segment classes render.
        assert!(html.contains("st-uptime"));
        assert!(html.contains("class=\"up\""));
        assert!(html.contains("class=\"down\""));
        // The backing-off card shows attempt + next-retry; the stalled card
        // carries the modifier that turns its retry line red.
        assert!(html.contains("attempt 3"));
        assert!(html.contains("next try in 12s"));
        assert!(html.contains("st-source stalled"));
    }

    /// The panel is silent before there is anything to say, and states the
    /// number *and* its own history once there is.
    #[test]
    fn the_microphone_panel_waits_until_it_has_something_to_report() {
        let mut s = snap(false, true, 1, 0);
        assert!(
            acoustic_panel(&s).is_empty(),
            "a station an hour old must not show a panel of dashes"
        );

        s.acoustic = vec![birdnet_db::audio_levels::SourceDrift {
            source: "cam1".to_owned(),
            recent_dbfs: -52.4,
            baseline_dbfs: Some(-51.9),
            recent_samples: 288,
        }];
        let html = acoustic_panel(&s);
        assert!(html.contains("-52.4 dBFS"), "{html}");
        assert!(
            html.contains("-0.5 dB"),
            "the move against its own past: {html}"
        );
        assert!(html.contains("288 samples"), "{html}");

        // "1 samples" on the screen an operator checks from the field is the
        // kind of small wrongness that makes the rest look unmaintained.
        s.acoustic[0].recent_samples = 1;
        let html = acoustic_panel(&s);
        assert!(html.contains("1 sample"), "{html}");
        assert!(!html.contains("1 samples"), "{html}");
    }

    /// A source with no month of history behind it must say so rather than
    /// report a drift of zero, which reads as "measured, and unchanged".
    #[test]
    fn a_source_without_a_baseline_says_so_instead_of_reporting_no_change() {
        let mut s = snap(false, true, 1, 0);
        s.acoustic = vec![birdnet_db::audio_levels::SourceDrift {
            source: "local".to_owned(),
            recent_dbfs: -48.0,
            baseline_dbfs: None,
            recent_samples: 12,
        }];
        let html = acoustic_panel(&s);
        assert!(html.contains("building a baseline"), "{html}");
        assert!(
            !html.contains("0.0 dB"),
            "must not imply it has been compared with anything: {html}"
        );
    }

    /// A source name is operator-supplied and reaches the page; it must be
    /// escaped like every other one.
    #[test]
    fn a_source_name_cannot_break_out_of_the_panel() {
        let mut s = snap(false, true, 1, 0);
        s.acoustic = vec![birdnet_db::audio_levels::SourceDrift {
            source: "<img src=x>".to_owned(),
            recent_dbfs: -50.0,
            baseline_dbfs: Some(-50.0),
            recent_samples: 1,
        }];
        let html = acoustic_panel(&s);
        assert!(!html.contains("<img src=x>"), "{html}");
        assert!(html.contains("&lt;img"), "{html}");
    }

    #[test]
    fn banner_flags_a_down_capture_source() {
        let mut s = snap(false, true, 2, 0);
        s.capture = vec![cap_source("RTSP_1", SourceState::BackingOff, 1, Some(4))];
        let banner = status_banner(&s);
        // The banner names the source rather than counting it, so the reader
        // of a two-camera station knows which one to go and look at.
        assert!(banner.contains("RTSP_1 stopped sending audio"), "{banner}");
        assert!(banner.contains("st-status warn"));
    }

    /// The banner has to say *which* source stopped.
    ///
    /// "An audio source is down" is a true sentence that leaves its reader no
    /// better off: a station with a garden mic and two cameras has three
    /// candidates and the banner named none of them. It now names the source
    /// the way the card below it does — the owner's label when there is one.
    #[test]
    fn the_banner_names_the_source_that_stopped() {
        let mut s = snap(false, true, 3, 0);
        s.capture = vec![
            cap_source("local", SourceState::Connected, 0, None),
            cap_source("RTSP_1", SourceState::Stalled, 1, None),
        ];
        s.source_names
            .insert("RTSP_1".to_owned(), "Pond camera".to_owned());
        let banner = status_banner(&s);
        assert!(
            banner.contains("Pond camera stopped sending audio"),
            "the banner must name the source, by its owner's name: {banner}"
        );
        assert!(
            !banner.contains("an audio source"),
            "and must not fall back to the anonymous phrasing: {banner}"
        );
    }

    /// The counterpart: once several sources are down, naming each one would
    /// be a list, not a banner — and the healthy source must never appear.
    #[test]
    fn the_banner_counts_rather_than_lists_a_wide_outage() {
        let mut s = snap(false, true, 4, 0);
        s.capture = vec![
            cap_source("ok", SourceState::Connected, 0, None),
            cap_source("a", SourceState::Stalled, 1, None),
            cap_source("b", SourceState::Stalled, 1, None),
            cap_source("c", SourceState::Stalled, 1, None),
        ];
        let banner = status_banner(&s);
        assert!(
            banner.contains("3 audio sources stopped sending audio"),
            "{banner}"
        );
        assert!(
            !banner.contains(">ok") && !banner.contains(" ok "),
            "the source that is still working must not be named: {banner}"
        );
    }

    /// The Diagnostics checklist must not contradict the banner above it.
    ///
    /// Observed failing against the pre-fix `diagnostics()`, whose Audio
    /// sources row asked only `s.sources_configured > 0`. On the seeded demo
    /// station that produced, on one screen: a warning banner reading "Needs
    /// attention — an audio source is down", two of three source cards chipped
    /// "Backing off" and "Stalled", and underneath them a green "Audio sources
    /// ✓ OK · 3 configured · 3 active today". An operator checking from the
    /// field got a tick from the one list that is meant to be the summary.
    #[test]
    fn diagnostics_audio_row_agrees_with_the_banner() {
        let mut s = snap(false, true, 3, 0);
        s.capture = vec![
            cap_source("local", SourceState::Connected, 0, None),
            cap_source("RTSP_1", SourceState::BackingOff, 3, Some(12)),
            cap_source("RTSP_2", SourceState::Stalled, 1, None),
        ];
        let banner = status_banner(&s);
        let checks = diagnostics(&s);
        assert!(
            banner.contains("stopped sending audio"),
            "precondition: the banner sees the fault: {banner}"
        );
        let audio_row = checks
            .split("<div class=\"st-check-row\">")
            .find(|r| r.contains("Audio sources"))
            .expect("there is an Audio sources row");
        assert!(
            audio_row.contains("mk warn"),
            "the banner reports a source down but Diagnostics ticks it green: {audio_row}"
        );
        assert!(
            audio_row.contains("2 not reporting"),
            "the row should say how many are faulted: {audio_row}"
        );
    }

    /// Counterpart to the gate above: a station whose sources are all healthy
    /// must still get its green tick, so the check discriminates rather than
    /// warning unconditionally.
    #[test]
    fn diagnostics_audio_row_stays_green_when_every_source_is_up() {
        let mut s = snap(false, true, 2, 0);
        s.capture = vec![
            cap_source("local", SourceState::Connected, 0, None),
            cap_source("RTSP_1", SourceState::Connected, 0, None),
        ];
        let audio_row = diagnostics(&s)
            .split("<div class=\"st-check-row\">")
            .find(|r| r.contains("Audio sources"))
            .expect("there is an Audio sources row")
            .to_string();
        assert!(
            !audio_row.contains("mk warn"),
            "all sources up must stay green: {audio_row}"
        );
        assert!(status_banner(&s).contains("All systems healthy"));
    }

    #[test]
    fn live_panel_takes_precedence_over_activity_fallback() {
        let mut s = snap(false, true, 1, 0);
        s.capture = vec![cap_source("local", SourceState::Connected, 0, None)];
        // With a supervisor publishing, the live panel renders even when no
        // detections have landed yet — not the "no detections" fallback.
        let html = source_panel(&s);
        assert!(!html.contains("No detections yet today"));
        assert!(html.contains("st-source"));
    }
}
