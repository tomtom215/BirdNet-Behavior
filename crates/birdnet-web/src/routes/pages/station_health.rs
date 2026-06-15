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
struct Snapshot {
    vitals: Vec<Vital>,
    /// Configured audio sources (count only — the panel keys on activity).
    sources_configured: usize,
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
    service_uptime: Option<u64>,
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

        let (sources_configured, activity, last_detection, queued, total, integrity) = state
            .with_db(|conn| {
                let sources = AudioSourceStore::list(conn)
                    .map_or(0, |s| s.iter().filter(|x| x.disabled_at.is_none()).count());
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
                (sources, activity, last, queued, total, integrity)
            });

        // Live capture-supervisor health, when a supervisor is publishing it.
        let capture = state
            .capture_status()
            .map(|handle| read_capture_status(&handle).sources)
            .unwrap_or_default();

        let vitals = build_vitals(&sys, disk.as_ref());
        Snapshot {
            vitals,
            sources_configured,
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
            service_uptime: system_info::process_uptime_secs(),
        }
    })
    .await
    .unwrap_or_else(|_| Snapshot {
        vitals: Vec::new(),
        sources_configured: 0,
        capture: Vec::new(),
        activity: Vec::new(),
        last_detection: None,
        queued_uploads: 0,
        total_detections: 0,
        integrity_ok: true,
        disk_low: false,
        disk_critical: false,
        service_uptime: None,
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
    vitals
}

/// Compose the full surface from the snapshot.
fn render(s: &Snapshot) -> String {
    format!(
        "<p class=\"bnb-lede\"><b>Everything your station needs to keep listening</b> — the \
         streams, the hardware, and the pipeline behind them. This is the screen to check from \
         the field. {help}</p>{banner}<h2 class=\"st-h3\">Audio sources</h2>{sources}\
         <h2 class=\"st-h3\">Vitals</h2>{vitals}<h2 class=\"st-h3\">Pipeline</h2>{pipeline}\
         <h2 class=\"st-h3\">Diagnostics</h2>{checks}",
        help = super::help::help_link(super::help::Topic::AdminSystem),
        banner = status_banner(s),
        sources = source_panel(s),
        vitals = vitals_row(&s.vitals),
        pipeline = pipeline_row(s),
        checks = diagnostics(s),
    )
}

/// The overall status banner — green unless a real problem is present.
fn status_banner(s: &Snapshot) -> String {
    let mut issues: Vec<&str> = Vec::new();
    if s.disk_critical {
        issues.push("storage is critically low");
    } else if s.disk_low {
        issues.push("storage is running low");
    }
    if !s.integrity_ok {
        issues.push("the database integrity check failed");
    }
    if s.sources_configured == 0 {
        issues.push("no audio sources are configured");
    }
    if s.queued_uploads > 0 {
        issues.push("uploads are waiting for the network");
    }
    if s.capture.iter().any(|c| c.state.is_fault()) {
        issues.push("an audio source is down");
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
        out.push_str(&source_card(src, today));
    }
    out.push_str("</div>");
    out
}

/// One live source card.
fn source_card(src: &SourceStatus, today: i64) -> String {
    let stalled = if src.state == SourceState::Stalled {
        " stalled"
    } else {
        ""
    };
    let last_audio = src
        .last_audio_age_secs
        .map_or_else(|| "—".to_string(), format_freshness);
    format!(
        "<div class=\"bnb-card st-source{stalled}\"><div class=\"st-source-head\">\
         <div><div class=\"st-source-name\">{name}</div>\
         <div class=\"st-source-type\">audio source</div></div>{chip}</div>\
         {strip}<div class=\"st-source-foot\"><span><b>{last_audio}</b> · last audio</span>\
         <span><b>{today}</b> · detections today</span></div>{retry}</div>",
        name = escape_html(&src.label),
        chip = source_chip(src.state),
        strip = uptime_strip(&src.uptime_24h),
        retry = retry_line(src),
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
        SourceState::BackingOff => {
            "<span class=\"bnb-pill dawn\"><span class=\"bnb-dot dawn\"></span> Backing off</span>"
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
         </div>",
        total = s.total_detections,
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
    let sources_detail = format!(
        "{} configured · {} active today",
        s.sources_configured,
        s.activity.len()
    );
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
        a = row(s.sources_configured > 0, "Audio sources", &sources_detail),
        b = row(
            !s.disk_low && !s.disk_critical,
            "Disk headroom",
            disk_detail
        ),
        c = row(
            s.integrity_ok,
            "Database integrity",
            if s.integrity_ok {
                "quick_check passed"
            } else {
                "quick_check FAILED — restore from a backup"
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
            capture: Vec::new(),
            activity: Vec::new(),
            last_detection: Some(30),
            queued_uploads: queued,
            total_detections: 100,
            integrity_ok: integrity,
            disk_low,
            disk_critical: false,
            service_uptime: Some(3_600),
        }
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
            next_retry_in_secs: next,
            uptime_24h: vec![UptimeSegment::Up, UptimeSegment::Down, UptimeSegment::Out],
        }
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
        assert!(status_banner(&snap(false, true, 0, 0)).contains("audio sources are configured"));
        assert!(status_banner(&snap(false, true, 2, 3)).contains("waiting for the network"));
        // Any problem flips the banner to the warn variant.
        assert!(status_banner(&snap(true, true, 2, 0)).contains("st-status warn"));
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
        assert!(html.contains("Backing off"));
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

    #[test]
    fn banner_flags_a_down_capture_source() {
        let mut s = snap(false, true, 2, 0);
        s.capture = vec![cap_source("RTSP_1", SourceState::BackingOff, 1, Some(4))];
        let banner = status_banner(&s);
        // The banner capitalises the joined issue list, so match without the
        // leading article ("…An audio source is down").
        assert!(banner.contains("audio source is down"));
        assert!(banner.contains("st-status warn"));
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
