//! The Station **Health** surface — the operator's "is it working?" screen.
//!
//! Composed for the public `/station` Health tab ([`super::homes::station`]),
//! the heir to the read-only `/system` page. It gathers one snapshot and
//! renders it in the v3 `st-*` treatment: an overall status banner, a
//! per-source activity panel, a vitals row (CPU · memory · temperature ·
//! df-correct disk), a pipeline row (last detection · queued uploads · service
//! uptime) and a short diagnostics checklist.
//!
//! Honest by construction: the web process has no live handle on the capture
//! supervisor, so the per-source panel reports **activity** from
//! `detections.Source` (how many detections each source produced today and how
//! recently) rather than a faked live/stalled stream chip, and the per-source
//! 24 h uptime strip and retry/backoff line — which need supervisor state — are
//! deferred (Wave D), not stubbed. Everything shown is real data the station
//! already records.

use std::fmt::Write as _;

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

        let vitals = build_vitals(&sys, disk.as_ref());
        Snapshot {
            vitals,
            sources_configured,
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

/// The per-source activity panel (honest activity, not supervisor state).
fn source_panel(s: &Snapshot) -> String {
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
}
