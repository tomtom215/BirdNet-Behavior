//! One list of what is wrong with the station, read by every surface that says
//! whether it is working.
//!
//! # Why this exists
//!
//! Two surfaces answer "is my station working?": the badge in the header of
//! every page (`/pages/health-badge`, polled every 30 s) and the banner on the
//! Health tab of the Station page. They were written separately and graded
//! different things. The badge looked at the integrity verdict, whether *any*
//! source was up, and one disk percentage; the banner also looked at scratch
//! space, queued uploads, a source that keeps dropping out, a species filter
//! that admits nothing and a classifier failing every window, and named each
//! source that had stopped. A station with a microphone up and two cameras
//! stalled therefore read **Healthy** in the header, in green, on the page that
//! links to a banner reading **Needs attention**.
//!
//! A header that contradicts the page it links to is a trust defect, not a
//! styling one. Both surfaces now build a [`Facts`] and ask [`issues`]; the
//! banner prints the sentences, the badge prints the verdict and carries the
//! same sentences as its title.
//!
//! # Cost
//!
//! The badge runs on every page load and every 30 s per open tab, so the
//! facts it gathers must stay cheap. Everything here is a few small indexed
//! reads, two `statvfs` calls and some in-memory gauges. The one expensive
//! thing the Health page does, `PRAGMA quick_check` (a full read of the
//! database file, roughly 30 s on a Pi's SD card), is *not* in the shared path:
//! the badge passes the verdict the daily maintenance pass recorded and the
//! page passes the result of its own check, in the same
//! [`Facts::integrity`] field. See [`recorded_integrity`].

use std::collections::HashMap;

use birdnet_core::audio::capture::{DiskUsage, SourceStatus};

use super::escape_html;
use super::today::{CaptureState, live_capture_state};
use crate::state::AppState;

/// How bad one issue is. Only a damaged database or an unreadable one is a
/// [`Self::Fault`]; everything else is something to look at, not a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Severity {
    /// Worth a look; the station is probably still recording something.
    Attention,
    /// The station's records are damaged or cannot be read.
    Fault,
}

/// One thing wrong with the station.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Issue {
    pub(super) severity: Severity,
    /// Two or three words for the header badge.
    pub(super) label: &'static str,
    /// A clause that reads after "Needs attention:" and starts in lower case.
    pub(super) sentence: String,
}

/// Everything the issue list is derived from. Both surfaces fill this in; the
/// fields that only one of them can know cheaply are optional by meaning.
// A flat record of independent yes/no readings (disk low, scratch low, capture
// down, …); grouping them into enums would not make [`issues`] any clearer. The
// Health page's own `Snapshot` carries the same allowance for the same reason.
#[allow(clippy::struct_excessive_bools)]
pub(super) struct Facts<'a> {
    /// The station's database could not be read at all. Every other field is
    /// then a guess, so [`issues`] returns this alone.
    pub(super) records_unreadable: bool,
    /// `Some(false)` is damage; `None` is "no verdict yet", which is not damage.
    pub(super) integrity: Option<bool>,
    pub(super) disk_low: bool,
    pub(super) disk_critical: bool,
    pub(super) scratch_low: bool,
    pub(super) scratch_critical: bool,
    /// The data directory is on the container's own writable layer, not a
    /// volume ([`crate::data_volume::on_container_layer`]).
    pub(super) data_on_container_layer: bool,
    /// Enabled audio sources.
    pub(super) sources_configured: usize,
    /// Sources are configured and none is up (or no detection daemon runs).
    pub(super) capture_down: bool,
    pub(super) queued_uploads: u64,
    /// Live per-source health from the capture supervisor; empty without one.
    pub(super) capture: &'a [SourceStatus],
    /// `stream id -> the name its owner typed`.
    pub(super) source_names: &'a HashMap<String, String>,
    pub(super) filter_admits_nothing: bool,
    pub(super) failing_classifiers: &'a [String],
}

/// What to call a source: the name its owner gave it, else the stream id.
fn display_name(names: &HashMap<String, String>, label: &str) -> String {
    names
        .get(label)
        .map_or_else(|| label.to_owned(), Clone::clone)
}

/// Name the sources in `faulty`, or count them once there are too many to read.
fn name_sources(names: &HashMap<String, String>, faulty: &[&SourceStatus]) -> String {
    match faulty {
        [] => String::new(),
        [one] => display_name(names, &one.label),
        [a, b] => format!(
            "{} and {}",
            display_name(names, &a.label),
            display_name(names, &b.label)
        ),
        many => format!("{} audio sources", many.len()),
    }
}

/// The issue list, worst-class first within the order the Health page has
/// always printed them (storage, scratch, database, sources, uploads, faults).
pub(super) fn issues(f: &Facts<'_>) -> Vec<Issue> {
    let mut out: Vec<Issue> = Vec::new();
    let mut push = |severity, label, sentence: String| {
        out.push(Issue {
            severity,
            label,
            sentence,
        });
    };
    // First, and on its own: every issue below is derived from reads that
    // failed, so listing them alongside would be reporting a station we could
    // not see.
    if f.records_unreadable {
        push(
            Severity::Fault,
            "Records unreadable",
            "the station's records could not be read".to_owned(),
        );
        return out;
    }
    if f.disk_critical {
        push(
            Severity::Attention,
            "Disk full",
            "storage is critically low".to_owned(),
        );
    } else if f.disk_low {
        push(
            Severity::Attention,
            "Disk nearly full",
            "storage is running low".to_owned(),
        );
    }
    // "scratch space (RAM /tmp)" named the implementation. What the reader has
    // is a temporary folder that recordings pass through on their way to disk.
    if f.scratch_critical {
        push(
            Severity::Attention,
            "Temp space full",
            "the temporary recording folder is critically low on space".to_owned(),
        );
    } else if f.scratch_low {
        push(
            Severity::Attention,
            "Temp space low",
            "the temporary recording folder is running low on space".to_owned(),
        );
    }
    if f.integrity == Some(false) {
        push(
            Severity::Fault,
            "Error",
            "the database integrity check failed".to_owned(),
        );
    }
    // Not a fault of the moment: the station is running and recording. It is a
    // fault of the next time the container is recreated (an image upgrade is
    // enough), and this is the only place the operator is told before it happens.
    if f.data_on_container_layer {
        push(
            Severity::Attention,
            "Data in container",
            "your data is stored inside the container, not on a volume, so it will be lost \
             when the container is recreated (mount a volume at the data folder)"
                .to_owned(),
        );
    }
    if f.sources_configured == 0 {
        push(
            Severity::Attention,
            "No microphone",
            "no microphone or camera is set up yet".to_owned(),
        );
    }
    if f.queued_uploads > 0 {
        push(
            Severity::Attention,
            "Uploads waiting",
            "uploads are waiting for the network".to_owned(),
        );
    }
    // Named, not counted. "An audio source is down" told the owner of two
    // cameras that one of them had a problem and left them to work out which.
    let faulted: Vec<&SourceStatus> = f.capture.iter().filter(|c| c.state.is_fault()).collect();
    if !faulted.is_empty() {
        let every = faulted.len() >= f.sources_configured;
        push(
            Severity::Attention,
            if every {
                "Not recording"
            } else {
                "Source stopped"
            },
            format!(
                "{} stopped sending audio",
                name_sources(f.source_names, &faulted)
            ),
        );
    } else if f.capture_down && f.sources_configured > 0 {
        // The supervisor's per-source states are not published (a web-only
        // process, or one that never reconciled), but the per-source gauge says
        // nothing is up. Without this branch the page read "All systems
        // healthy" beside a header reading "Not recording".
        push(
            Severity::Attention,
            "Not recording",
            "a microphone is set up, but no audio is reaching the station".to_owned(),
        );
    }
    let flapping: Vec<&SourceStatus> = f.capture.iter().filter(|c| c.flapping).collect();
    if !flapping.is_empty() {
        push(
            Severity::Attention,
            "Unstable audio",
            format!(
                "{} keeps dropping out and reconnecting",
                name_sources(f.source_names, &flapping)
            ),
        );
    }
    if f.filter_admits_nothing {
        push(
            Severity::Attention,
            "Filter admits nothing",
            "the species filter admits no species, so nothing can be recorded".to_owned(),
        );
    }
    for id in f.failing_classifiers {
        push(
            Severity::Attention,
            "Classifier failing",
            format!(
                "the classifier \u{201c}{id}\u{201d} is failing every window, so nothing it \
                 would detect is recorded"
            ),
        );
    }
    out
}

/// Upper-case the first character.
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().collect::<String>() + chars.as_str()
    })
}

/// The sentences of an issue list, joined the way the Health banner prints
/// them. Not escaped.
pub(super) fn sentences(issues: &[Issue]) -> String {
    capitalize_first(
        &issues
            .iter()
            .map(|i| i.sentence.as_str())
            .collect::<Vec<_>>()
            .join(" · "),
    )
}

/// The header badge's reading of an issue list.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Badge {
    /// `bnb-pill` modifier.
    pub(super) pill: &'static str,
    /// `bnb-dot` modifier.
    pub(super) dot: &'static str,
    pub(super) label: &'static str,
    /// The stable `data-health` vocabulary: `ok`, `warn` or `err`.
    pub(super) token: &'static str,
    /// Unescaped; the same sentences the Health page banner prints.
    pub(super) title: String,
}

/// Grade the station for the header.
///
/// One issue names itself ("Disk nearly full"); two or more read "Needs
/// attention", the page's own word, and the title lists them all. A fault
/// outranks everything: the pill goes red and keeps the fault's label.
pub(super) fn badge(issues: &[Issue], integrity: Option<bool>) -> Badge {
    if let Some(fault) = issues.iter().find(|i| i.severity == Severity::Fault) {
        return Badge {
            pill: "rare",
            dot: "rare",
            label: fault.label,
            token: "err",
            title: sentences(issues),
        };
    }
    match issues {
        [] => Badge {
            pill: "moss",
            dot: "live",
            label: "Healthy",
            token: "ok",
            title: if integrity.is_none() {
                // Everything this badge *can* see is fine, but the database has
                // not been verified yet. Say so rather than claiming a check
                // that never happened.
                "capture and disk look fine; the database has not been checked yet".to_owned()
            } else {
                "database, capture and disk all look fine".to_owned()
            },
        },
        [one] => Badge {
            pill: "dawn",
            dot: "dawn",
            label: one.label,
            token: "warn",
            title: sentences(issues),
        },
        _ => Badge {
            pill: "dawn",
            dot: "dawn",
            label: "Needs attention",
            token: "warn",
            title: sentences(issues),
        },
    }
}

/// The badge as markup. A link to the page that explains it.
pub(super) fn badge_html(b: &Badge) -> String {
    let title = escape_html(&b.title);
    format!(
        r#"<a href="/station" class="bnb-pill {pill}" data-health="{token}" title="{title}" aria-label="Station health — {label}: {title}"><span class="bnb-dot {dot}"></span> {label}</a>"#,
        pill = b.pill,
        token = b.token,
        label = b.label,
        dot = b.dot,
    )
}

/// What the last recorded integrity check says about the database.
///
/// `None` means no verdict is on record — either the daily check has not run on
/// this station yet, or the row predates the column. The caller must keep that
/// distinct from a recorded failure.
///
/// # Why this is a read and not a check
///
/// This used to be `birdnet_db::sqlite::quick_check(conn)`, run inline. That
/// pragma reads **every page of the database file**. On a three-year station
/// (2.76 M detections, 1.29 GB) it measured 1.5-1.9 s warm on `NVMe`, and the
/// enclosing partial 3.8 s; a Raspberry Pi reading the same file from an SD
/// card at ~45 MB/s is looking at roughly 30 s — longer than the badge's own
/// refresh interval, so the scans would overlap.
///
/// The badge is mounted in `layout.html` with `hx-trigger="load, every 30s"`,
/// which put that read on every page load and then twice a minute for every
/// open tab, indefinitely, competing with the detection write path for the same
/// card and steadily wearing it. The daily maintenance pass already runs the
/// check properly; since migration 28 it records the answer, so the badge can
/// read one row instead of re-deriving it.
pub(super) fn recorded_integrity(conn: &rusqlite::Connection) -> Option<bool> {
    birdnet_db::sqlite::last_run_result(conn, birdnet_db::sqlite::JOB_INTEGRITY_CHECK)
        .ok()
        .flatten()
        .and_then(|(_when, ok)| ok)
}

/// The data disk and, only when it is a different filesystem, scratch space.
///
/// The service writes its live audio segments and temp files under `/tmp`,
/// which on a Pi (and any tmpfs `/tmp`) is a small, RAM-backed filesystem
/// separate from the data disk, and a full one breaks capture. It is tracked
/// separately from the data disk, but only when it really is another
/// filesystem; when `/tmp` lives on the data partition the disk reading
/// already covers it and a second identical one would be noise.
pub(super) fn disks(db_path: &std::path::Path) -> (Option<DiskUsage>, Option<DiskUsage>) {
    let data_dir = data_dir_of(db_path);
    let disk = birdnet_core::audio::capture::disk_usage(&data_dir).ok();
    let scratch = birdnet_core::audio::capture::disk_usage(&std::env::temp_dir())
        .ok()
        .filter(|s| disk.as_ref().is_none_or(|d| d.total_bytes != s.total_bytes));
    (disk, scratch)
}

/// The directory holding the database: the station's data directory.
pub(super) fn data_dir_of(db_path: &std::path::Path) -> std::path::PathBuf {
    db_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(
            || std::path::PathBuf::from("."),
            std::path::Path::to_path_buf,
        )
}

/// [`crate::data_volume::on_container_layer`] for the badge, read once per
/// database path for the life of the process.
///
/// The answer cannot change under a running process (a volume is not mounted
/// into a container that is already running), and the call reads
/// `/proc/self/mountinfo`, which is not something to repeat on every poll of
/// every open tab. The Health page reads it fresh.
fn on_container_layer_cached(db_path: &std::path::Path) -> bool {
    static CACHE: std::sync::Mutex<Vec<(std::path::PathBuf, bool)>> =
        std::sync::Mutex::new(Vec::new());
    if let Ok(cache) = CACHE.lock()
        && let Some((_, on)) = cache.iter().find(|(p, _)| p == db_path)
    {
        return *on;
    }
    let on = crate::data_volume::on_container_layer(&data_dir_of(db_path));
    if let Ok(mut cache) = CACHE.lock() {
        cache.retain(|(p, _)| p != db_path);
        cache.push((db_path.to_path_buf(), on));
    }
    on
}

/// How long the header badge reuses a disk reading.
///
/// `disk_usage` has no `statvfs` to call (this workspace forbids `unsafe`), so
/// it **spawns `df`**. The old badge spawned one per poll; reading scratch space
/// as well made it two, and the badge's own request time went from a measured
/// 15 ms to 26 ms on a loaded x86 host — on a Pi a fork and exec is several
/// times that, twice a minute per open tab. A disk does not fill between two
/// polls 30 s apart, so the badge reads the filesystem at most once per this
/// interval. The Health page, which is looked at rather than polled, reads it
/// fresh.
const BADGE_DISK_TTL: std::time::Duration = std::time::Duration::from_secs(25);

type DiskReading = (
    std::path::PathBuf,
    std::time::Instant,
    Option<DiskUsage>,
    Option<DiskUsage>,
);

/// [`disks`], at most once per [`BADGE_DISK_TTL`] per database path.
fn disks_cached(db_path: &std::path::Path) -> (Option<DiskUsage>, Option<DiskUsage>) {
    static CACHE: std::sync::Mutex<Vec<DiskReading>> = std::sync::Mutex::new(Vec::new());
    if let Ok(cache) = CACHE.lock()
        && let Some((_, at, disk, scratch)) = cache.iter().find(|(p, ..)| p == db_path)
        && at.elapsed() < BADGE_DISK_TTL
    {
        return (*disk, *scratch);
    }
    // Not held across the `df` calls: a stalled mount must not queue every
    // other poll behind this one.
    let (disk, scratch) = disks(db_path);
    if let Ok(mut cache) = CACHE.lock() {
        cache.retain(|(p, ..)| p != db_path);
        cache.push((
            db_path.to_path_buf(),
            std::time::Instant::now(),
            disk,
            scratch,
        ));
    }
    (disk, scratch)
}

/// Gather [`Facts`] for the header badge — the cheap ones. Blocking.
///
/// Everything that can fail propagates: a station whose sources cannot be read
/// must not be told it has none, nor, worse, that it is healthy.
pub(super) fn gather_for_badge(state: &AppState) -> (Vec<Issue>, Option<bool>) {
    use birdnet_db::audio_sources::AudioSourceStore;

    let read = state.with_read_db(|conn| {
        let listed = AudioSourceStore::list(conn).map_err(|e| e.to_string())?;
        let enabled: Vec<_> = listed.iter().filter(|x| x.disabled_at.is_none()).collect();
        let names: HashMap<String, String> = enabled
            .iter()
            .filter_map(|x| {
                let label = x.label.as_deref()?.trim();
                (!label.is_empty() && label != x.id).then(|| (x.id.clone(), label.to_owned()))
            })
            .collect();
        let queued =
            birdnet_db::outbound_queue::depth(conn, birdnet_integrations::birdweather::QUEUE_KIND)
                .map_err(|e| e.to_string())?;
        Ok::<_, String>((enabled.len(), names, queued, recorded_integrity(conn)))
    });
    let (disk, scratch) = disks_cached(state.db_path());
    let capture = state
        .capture_status()
        .map(|handle| birdnet_core::audio::capture::read_capture_status(&handle).sources)
        .unwrap_or_default();
    let failing: Vec<String> = state
        .metrics()
        .model_health()
        .into_iter()
        .filter(|(_, h)| h.is_failing())
        .map(|(id, _)| id)
        .collect();
    let admits_nothing = state.metrics().occurrence_filter().admits_nothing();
    let capture_down = matches!(live_capture_state(state), CaptureState::Down);

    let (sources_configured, names, queued, integrity, records_unreadable) = match read {
        Ok((n, names, q, i)) => (n, names, q, i, false),
        Err(e) => {
            tracing::warn!(error = %e, "health badge: database read failed");
            (0, HashMap::new(), 0, None, true)
        }
    };
    let facts = Facts {
        records_unreadable,
        integrity,
        disk_low: disk.as_ref().is_some_and(DiskUsage::is_low),
        disk_critical: disk.as_ref().is_some_and(DiskUsage::is_critical),
        scratch_low: scratch.as_ref().is_some_and(DiskUsage::is_low),
        scratch_critical: scratch.as_ref().is_some_and(DiskUsage::is_critical),
        data_on_container_layer: on_container_layer_cached(state.db_path()),
        sources_configured,
        capture_down,
        queued_uploads: queued,
        capture: &capture,
        source_names: &names,
        filter_admits_nothing: admits_nothing,
        failing_classifiers: &failing,
    };
    (issues(&facts), integrity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use birdnet_core::audio::capture::{SourceState, UPTIME_SEGMENTS, UptimeSegment};

    fn src(label: &str, state: SourceState, flapping: bool) -> SourceStatus {
        SourceStatus {
            label: label.to_owned(),
            state,
            uptime_secs: None,
            last_audio_age_secs: Some(1),
            restart_attempts: 0,
            restarts_last_hour: 0,
            flapping,
            next_retry_in_secs: None,
            uptime_24h: vec![UptimeSegment::Up; UPTIME_SEGMENTS],
        }
    }

    /// A station with nothing wrong, as a base to break one thing at a time.
    fn ok<'a>(names: &'a HashMap<String, String>, none: &'a [String]) -> Facts<'a> {
        Facts {
            records_unreadable: false,
            integrity: Some(true),
            disk_low: false,
            disk_critical: false,
            scratch_low: false,
            scratch_critical: false,
            data_on_container_layer: false,
            sources_configured: 2,
            capture_down: false,
            queued_uploads: 0,
            capture: &[],
            source_names: names,
            filter_admits_nothing: false,
            failing_classifiers: none,
        }
    }

    fn label_of(f: &Facts<'_>) -> (&'static str, &'static str) {
        let i = issues(f);
        let b = badge(&i, f.integrity);
        (b.label, b.token)
    }

    #[test]
    fn a_working_station_is_healthy() {
        let (n, none) = (HashMap::new(), Vec::new());
        assert!(issues(&ok(&n, &none)).is_empty());
        assert_eq!(label_of(&ok(&n, &none)), ("Healthy", "ok"));
    }

    /// No gauge published yet is not an outage: a station that has just
    /// started must not flash "Not recording" at its operator. (`capture_down`
    /// is only true when the gauges say nothing is up.)
    #[test]
    fn an_unreconciled_source_does_not_read_as_down() {
        let (n, none) = (HashMap::new(), Vec::new());
        let f = Facts {
            capture_down: false,
            ..ok(&n, &none)
        };
        assert_eq!(label_of(&f).0, "Healthy");
    }

    /// The badge reports the *recorded* verdict, and "no verdict yet" is
    /// neither a pass nor a failure: a fresh install must not show a red
    /// Error, and its title must not claim the database was verified.
    #[test]
    fn a_never_run_check_is_unknown_rather_than_pass_or_fail() {
        let (n, none) = (HashMap::new(), Vec::new());
        let f = Facts {
            integrity: None,
            ..ok(&n, &none)
        };
        let b = badge(&issues(&f), f.integrity);
        assert_eq!((b.label, b.token), ("Healthy", "ok"));
        assert!(
            b.title.contains("not been checked"),
            "the title must not claim the database was checked; got {:?}",
            b.title
        );
    }

    #[test]
    fn a_recorded_failure_is_an_error_and_outranks_everything() {
        let (n, none) = (HashMap::new(), Vec::new());
        let f = Facts {
            integrity: Some(false),
            disk_critical: true,
            capture_down: true,
            ..ok(&n, &none)
        };
        let i = issues(&f);
        let b = badge(&i, f.integrity);
        assert_eq!((b.label, b.token, b.pill), ("Error", "err", "rare"));
        assert!(
            b.title.to_lowercase().contains("storage is critically low"),
            "the fault must not hide the other problems: {}",
            b.title
        );
    }

    /// Data on the container's own layer is lost at the next image upgrade, so
    /// the header must not read Healthy over a Health page that says so. The
    /// healthy station (`a_working_station_is_healthy`) is the counterpart.
    #[test]
    fn data_on_the_container_layer_is_not_healthy_in_the_header_either() {
        let (n, none) = (HashMap::new(), Vec::new());
        let f = Facts {
            data_on_container_layer: true,
            ..ok(&n, &none)
        };
        let i = issues(&f);
        assert_eq!(label_of(&f), ("Data in container", "warn"));
        assert!(
            sentences(&i).contains("stored inside the container, not on a volume"),
            "the sentence the Health banner prints: {}",
            sentences(&i)
        );
    }

    #[test]
    fn a_dead_microphone_is_not_healthy() {
        let (n, none) = (HashMap::new(), Vec::new());
        let f = Facts {
            capture_down: true,
            ..ok(&n, &none)
        };
        assert_eq!(label_of(&f), ("Not recording", "warn"));
    }

    #[test]
    fn no_configured_source_is_not_healthy() {
        let (n, none) = (HashMap::new(), Vec::new());
        let f = Facts {
            sources_configured: 0,
            ..ok(&n, &none)
        };
        assert_eq!(label_of(&f), ("No microphone", "warn"));
    }

    #[test]
    fn the_disk_labels_follow_the_page_s_two_thresholds() {
        let (n, none) = (HashMap::new(), Vec::new());
        let low = Facts {
            disk_low: true,
            ..ok(&n, &none)
        };
        assert_eq!(label_of(&low).0, "Disk nearly full");
        let critical = Facts {
            disk_low: true,
            disk_critical: true,
            ..ok(&n, &none)
        };
        assert_eq!(label_of(&critical).0, "Disk full");
    }

    /// The defect this module exists to remove: one source up, two stalled.
    /// The old badge asked "is any source up?" and answered Healthy.
    #[test]
    fn one_source_up_and_two_stalled_is_not_healthy_and_names_them() {
        let none: Vec<String> = Vec::new();
        let names: HashMap<String, String> =
            HashMap::from([("RTSP_1".to_owned(), "Front-yard RTSP".to_owned())]);
        let capture = vec![
            src("local", SourceState::Connected, false),
            src("RTSP_1", SourceState::BackingOff, false),
            src("RTSP_2", SourceState::Stalled, false),
        ];
        let f = Facts {
            sources_configured: 3,
            capture: &capture,
            ..ok(&names, &none)
        };
        let i = issues(&f);
        let b = badge(&i, f.integrity);
        assert_eq!((b.label, b.token), ("Source stopped", "warn"));
        assert!(
            b.title
                .contains("Front-yard RTSP and RTSP_2 stopped sending audio"),
            "{}",
            b.title
        );
    }

    /// Every source down is "Not recording", not "a source stopped".
    #[test]
    fn every_source_stopped_reads_as_not_recording() {
        let (n, none) = (HashMap::new(), Vec::new());
        let capture = vec![
            src("a", SourceState::Stalled, false),
            src("b", SourceState::BackingOff, false),
        ];
        let f = Facts {
            capture: &capture,
            ..ok(&n, &none)
        };
        assert_eq!(label_of(&f).0, "Not recording");
    }

    #[test]
    fn two_issues_read_needs_attention_and_the_title_lists_both() {
        let (n, none) = (HashMap::new(), Vec::new());
        let f = Facts {
            queued_uploads: 4,
            scratch_low: true,
            ..ok(&n, &none)
        };
        let b = badge(&issues(&f), f.integrity);
        assert_eq!(b.label, "Needs attention");
        assert!(
            b.title.contains("temporary recording folder"),
            "{}",
            b.title
        );
        assert!(b.title.contains("waiting for the network"), "{}", b.title);
    }

    /// A read that failed is not a station with nothing wrong, nor one with no
    /// microphone: it is its own fault, and it stands alone.
    #[test]
    fn an_unreadable_database_is_a_fault_that_stands_alone() {
        let (n, none) = (HashMap::new(), Vec::new());
        let f = Facts {
            records_unreadable: true,
            sources_configured: 0,
            capture_down: true,
            ..ok(&n, &none)
        };
        let i = issues(&f);
        assert_eq!(i.len(), 1, "{i:?}");
        let b = badge(&i, None);
        assert_eq!((b.label, b.token), ("Records unreadable", "err"));
    }

    fn migrated() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().expect("open in-memory");
        birdnet_db::migration::migrate(&conn).expect("migrate schema");
        conn
    }

    fn record(conn: &rusqlite::Connection, ok: Option<bool>) {
        birdnet_db::sqlite::record_run_result(
            conn,
            birdnet_db::sqlite::JOB_INTEGRITY_CHECK,
            1_700_000_000,
            ok,
        )
        .expect("record");
    }

    /// The badge must report the *recorded* verdict of the daily integrity
    /// check, not run one of its own.
    ///
    /// The distinction is the whole point. `PRAGMA quick_check` reads every
    /// page of the database file: 1.5-1.9 s on a three-year station's 1.29 GB
    /// database on `NVMe`, and roughly 30 s on the SD card a Pi actually runs on.
    /// The badge lives in `layout.html` with `hx-trigger="load, every 30s"`, so
    /// running it there meant a full read of the database on every page load
    /// and twice a minute per open tab thereafter, forever.
    ///
    /// This database is healthy — a live check would pass. Recording a failed
    /// verdict is therefore the one thing that can tell the two implementations
    /// apart.
    #[test]
    fn the_badge_reports_the_recorded_verdict_not_a_fresh_scan() {
        let conn = migrated();
        record(&conn, Some(false));
        assert_eq!(
            recorded_integrity(&conn),
            Some(false),
            "a recorded failure must reach the badge from `maintenance_runs`, \
             not be overruled by re-checking a database that is in fact intact"
        );
    }

    /// The counterpart, so the gate above cannot be satisfied by a badge that
    /// simply always reports a failure.
    #[test]
    fn a_recorded_pass_reads_back_as_a_pass() {
        let conn = migrated();
        record(&conn, Some(true));
        assert_eq!(recorded_integrity(&conn), Some(true));
    }

    /// No row at all is "not checked", not "failed" — and a job that records no
    /// verdict (the session prune) must read back the same way.
    #[test]
    fn no_verdict_reads_back_as_none() {
        let conn = migrated();
        assert_eq!(recorded_integrity(&conn), None, "no row means no verdict");
        record(&conn, None);
        assert_eq!(recorded_integrity(&conn), None);
    }

    /// The title is an attribute and carries a name its owner typed.
    #[test]
    fn a_source_name_cannot_break_out_of_the_badge() {
        let none: Vec<String> = Vec::new();
        let names: HashMap<String, String> =
            HashMap::from([("RTSP_1".to_owned(), "\"><script>x()</script>".to_owned())]);
        let capture = vec![src("RTSP_1", SourceState::Stalled, false)];
        let f = Facts {
            sources_configured: 1,
            capture: &capture,
            ..ok(&names, &none)
        };
        let i = issues(&f);
        let html = badge_html(&badge(&i, f.integrity));
        assert!(!html.contains("<script>"), "{html}");
    }
}
