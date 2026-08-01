//! Backups, restore & storage — the operator's "is my data safe?" surface.
//!
//! Every figure on this page is measured from the running station: snapshot
//! files are read from the backup directory, sizes come from the filesystem,
//! and the schedule comes from the `maintenance_runs` table the background
//! maintenance loop writes. Nothing here is illustrative.
//!
//! That is a deliberate correction. This page used to render a designer's
//! mock-up as if it were live telemetry — a fixed "Last backup: 2 h ago · auto
//! · nightly 03:00" headline, eight invented snapshot rows with working-looking
//! Restore buttons, a hardcoded "Recordings 8.4 GB / 74 %" storage bar, and an
//! operations log quoting an S3 upload failure for an integration that does not
//! exist. There is no nightly backup (the schedule is weekly), and on a station
//! that restarted more often than weekly no backup had *ever* run — so the one
//! number an operator most needs to trust was not merely stale but confidently
//! wrong. For a station somebody leaves running for a season, a fabricated
//! "your data is backed up" is worse than showing nothing at all.
//!
//! Controls are likewise limited to endpoints that exist. Where the station
//! genuinely cannot do something (off-site destinations, restore-verification),
//! the page says so rather than showing a dead switch.

use std::fmt::Write as _;

use axum::Router;
use axum::routing::get;

use crate::state::AppState;

/// Mount the backup and recovery admin route.
pub fn router() -> Router<AppState> {
    Router::new().route("/admin/backups", get(backups_page))
}

/// The standalone `/admin/backups` page GET folded into the Station **Data**
/// tab; its old URL permanently redirects there.
async fn backups_page() -> axum::response::Redirect {
    axum::response::Redirect::permanent("/station/data")
}

// ───────────────────────────────────────────────────────────────────────────
// Measured facts
// ───────────────────────────────────────────────────────────────────────────

/// One real snapshot file on disk.
#[derive(Debug, Clone)]
struct Snapshot {
    name: String,
    bytes: u64,
    modified_unix: u64,
}

/// Everything the page reports, measured from the live station.
#[derive(Debug, Default)]
struct DataFacts {
    /// Size of `birds.db` (plus its `-wal` sidecar, which is real occupancy).
    db_bytes: u64,
    /// Extracted detection clips in the directory the web actually serves.
    recordings_count: u32,
    recordings_bytes: u64,
    /// Snapshot files, newest first.
    snapshots: Vec<Snapshot>,
    snapshot_bytes: u64,
    /// When the scheduled backup + VACUUM job last completed, if ever.
    scheduled_last_run_unix: Option<i64>,
    /// Filesystem holding the database and recordings.
    disk_used_pct: Option<f64>,
    disk_available_bytes: u64,
    now_unix: u64,
}

/// Read the real state of the station's data. Blocking; callers already run
/// inside `spawn_blocking` (see `pages::homes::station_tabs::data_page`).
fn gather(state: &AppState) -> DataFacts {
    let db_path = state.db_path();
    let data_dir = db_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_path_buf();
    let backup_dir = data_dir.join("backups");

    let file_len = |p: &std::path::Path| std::fs::metadata(p).map_or(0, |m| m.len());
    // SQLite names its sidecars by *appending* to the database path, so build
    // them that way rather than with `with_extension` (which would replace
    // `.db` and miss the files whenever the database has no extension).
    // The WAL can hold megabytes of not-yet-checkpointed pages; counting it
    // keeps the reported database size from understating real disk use.
    let sidecar = |suffix: &str| {
        let mut p = db_path.as_os_str().to_owned();
        p.push(suffix);
        std::path::PathBuf::from(p)
    };
    let db_bytes = file_len(db_path) + file_len(&sidecar("-wal")) + file_len(&sidecar("-shm"));

    let (recordings_count, recordings_bytes) =
        birdnet_core::audio::capture::recording_stats(&state.recording_dir()).unwrap_or((0, 0));

    let mut snapshots: Vec<Snapshot> = std::fs::read_dir(&backup_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| super::backup::is_backup_file_name(&e.file_name().to_string_lossy()))
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            Some(Snapshot {
                name: e.file_name().to_string_lossy().into_owned(),
                bytes: meta.len(),
                modified_unix: meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_secs()),
            })
        })
        .collect();
    snapshots.sort_by_key(|s| std::cmp::Reverse(s.modified_unix));
    let snapshot_bytes = snapshots.iter().map(|s| s.bytes).sum();

    let scheduled_last_run_unix = state.with_db(|conn| {
        birdnet_db::sqlite::last_run_unix(conn, birdnet_db::sqlite::JOB_BACKUP_VACUUM)
            .unwrap_or(None)
    });

    let (disk_used_pct, disk_available_bytes) = birdnet_core::audio::capture::disk_usage(&data_dir)
        .map_or((None, 0), |u| (Some(u.used_percent()), u.available_bytes));

    DataFacts {
        db_bytes,
        recordings_count,
        recordings_bytes,
        snapshots,
        snapshot_bytes,
        scheduled_last_run_unix,
        disk_used_pct,
        disk_available_bytes,
        now_unix: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
    }
}

/// Render the backups & restore body (no document shell).
///
/// Shared with the Station **Data** tab
/// (`crate::routes::pages::homes::station_tabs`), which renders the same
/// backup/restore/export surface in the main shell.
pub(crate) fn backups_body(state: &AppState) -> String {
    render_body(&gather(state))
}

// ───────────────────────────────────────────────────────────────────────────
// Formatting helpers
// ───────────────────────────────────────────────────────────────────────────

#[allow(clippy::cast_precision_loss)]
fn format_bytes(bytes: u64) -> String {
    const GB: u64 = 1_073_741_824;
    const MB: u64 = 1_048_576;
    const KB: u64 = 1_024;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// Human "how long ago", or "never" for an absent timestamp.
fn ago(now: u64, then: u64) -> String {
    if then == 0 || then > now {
        return "just now".to_string();
    }
    let secs = now - then;
    match secs {
        0..=90 => "just now".to_string(),
        91..=5399 => format!("{} min ago", secs / 60),
        5400..=172_799 => format!("{} h ago", secs / 3600),
        _ => format!("{} d ago", secs / 86_400),
    }
}

/// Human "in how long" for a future instant.
fn until(now: i64, then: i64) -> String {
    let secs = then - now;
    if secs <= 0 {
        return "due now".to_string();
    }
    if secs < 5400 {
        format!("in {} min", secs / 60)
    } else if secs < 172_800 {
        format!("in {} h", secs / 3600)
    } else {
        format!("in {} d", secs / 86_400)
    }
}

/// Percentage of `total` that `part` represents, clamped to 0–100.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn pct_of(part: u64, total: u64) -> u32 {
    if total == 0 {
        return 0;
    }
    ((part as f64 / total as f64) * 100.0)
        .clamp(0.0, 100.0)
        .round() as u32
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// A storage-breakdown tile with a usage bar. The bar fill colour is an
/// enumerable tone (`moss`/`dawn`) class; only its width is computed inline.
fn storage_tile(label: &str, value: &str, pct: u32, tone: &str) -> String {
    format!(
        r#"<div class="bnb-card pad">
  <div class="bnb-eyebrow">{label}</div>
  <div class="display bkr-tile-value">{value}</div>
  <div class="bkr-bar"><span class="bkr-bar-fill {tone}" data-style="width:{pct}%"></span></div>
</div>"#
    )
}

// ───────────────────────────────────────────────────────────────────────────
// Render
// ───────────────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_lines)]
fn render_body(f: &DataFacts) -> String {
    let stat = |label: &str, value: &str, sub: &str| {
        format!(
            r#"<div class="bnb-card pad"><div class="display bkr-stat-value">{value}</div><div class="bnb-eyebrow bkr-stat-label">{label}</div><div class="bnb-meta">{sub}</div></div>"#
        )
    };

    // ── Stat strip — all four measured ────────────────────────────────────
    let newest = f.snapshots.first();
    let (last_value, last_sub) = newest.map_or_else(
        || {
            (
                "never".to_string(),
                "no snapshot has been taken on this station".to_string(),
            )
        },
        |s| {
            (
                ago(f.now_unix, s.modified_unix),
                format!("{} · newest snapshot", format_bytes(s.bytes)),
            )
        },
    );

    let next_sub = f.scheduled_last_run_unix.map_or_else(
        || "scheduled backup has not run yet".to_string(),
        |last| {
            format!(
                "scheduled backup {}",
                until(
                    i64::try_from(f.now_unix).unwrap_or(i64::MAX),
                    last + birdnet_db::sqlite::BACKUP_VACUUM_INTERVAL_SECS,
                )
            )
        },
    );

    let disk_value = f
        .disk_used_pct
        .map_or_else(|| "—".to_string(), |p| format!("{p:.0}%"));
    let disk_sub = if f.disk_available_bytes == 0 {
        "disk usage unavailable".to_string()
    } else {
        format!("{} free", format_bytes(f.disk_available_bytes))
    };

    let stats = format!(
        r#"<div class="bkr-stats">{a}{b}{c}{d}</div>"#,
        a = stat("Last snapshot", &last_value, &escape_html(&last_sub)),
        b = stat(
            "Snapshots kept",
            &f.snapshots.len().to_string(),
            &escape_html(&next_sub)
        ),
        c = stat(
            "Snapshot size",
            &format_bytes(f.snapshot_bytes),
            "total on disk"
        ),
        d = stat("Disk used", &disk_value, &escape_html(&disk_sub)),
    );

    // ── Backup / restore, both wired to real endpoints ────────────────────
    let backup_restore = r##"<div class="bkr-split">
  <div class="bnb-card pad">
    <div class="section-header"><div><div class="bnb-eyebrow">Back up</div><h3>Take a copy now</h3></div></div>
    <p class="bnb-meta">A <b>snapshot</b> copies the database only — small and quick, kept here on the station. A <b>full backup</b> bundles the database, recordings and config into one archive you can download and keep somewhere else.</p>
    <div class="bnb-row tight bkr-mt">
      <button class="bnb-btn"
              hx-post="/admin/system/backup"
              hx-target="#bkr-backup-result"
              hx-swap="innerHTML">Snapshot the database</button>
      <a class="bnb-btn ghost" href="/admin/system/backup/full" download="birdnet-backup.tar.gz">Download full backup ↓</a>
    </div>
    <div id="bkr-backup-result" class="bnb-meta bkr-note-mt"></div>
  </div>
  <div class="bnb-card pad">
    <div class="section-header"><div><div class="bnb-eyebrow">Restore</div><h3>Upload a full backup</h3></div></div>
    <form hx-post="/admin/system/restore"
          hx-encoding="multipart/form-data"
          hx-target="#bkr-restore-result"
          hx-swap="innerHTML">
      <input type="file" name="backup" accept=".gz,.tgz,application/gzip" required class="bkr-file">
      <button type="submit" class="bnb-btn danger bkr-mt-xs"
              data-confirm-action="submit"
              data-confirm-title="Restore from backup"
              data-confirm-body="This overwrites the current database and recordings with the contents of the archive. There is no undo. Continue?"
              data-confirm-confirm-label="Restore"
              data-confirm-style="danger">Restore from archive</button>
    </form>
    <p class="bnb-meta bkr-note-mt">Restoring overwrites the current database and recordings, then asks you to restart the service. Take a full backup first.</p>
    <div id="bkr-restore-result" class="bnb-meta bkr-note-mt"></div>
  </div>
</div>"##;

    // ── Snapshot list — the real files, or an honest empty state ──────────
    let snapshots = if f.snapshots.is_empty() {
        r#"<div class="bnb-card pad bkr-mt">
  <div class="section-header"><div><div class="bnb-eyebrow">History</div><h3>Snapshots</h3></div></div>
  <p class="bnb-meta">No snapshots yet. The station takes one automatically every 7 days, and keeps the most recent 14. Use <b>Snapshot the database</b> above to take one now.</p>
</div>"#
            .to_string()
    } else {
        let mut rows = String::new();
        for (i, s) in f.snapshots.iter().enumerate() {
            let name = escape_html(&s.name);
            let row_cls = if i == 0 {
                "bkr-snap-row today"
            } else {
                "bkr-snap-row"
            };
            let _ = write!(
                rows,
                r#"<div class="{row_cls}">
  <span class="bnb-dot bkr-dot auto"></span>
  <div><span class="bkr-snap-when">{when}</span> <span class="bnb-meta">· {size} · <span class="mono">{name}</span></span></div>
  <a class="bnb-btn ghost bkr-btn-sm" href="/admin/system/backups/{name}" download="{name}">Download</a>
  <button class="bnb-btn ghost bkr-btn-sm"
          hx-delete="/admin/system/backups/{name}"
          hx-target="closest .bkr-snap-row"
          hx-swap="outerHTML"
          data-confirm-action="hx-delete"
          data-confirm-url="/admin/system/backups/{name}"
          data-confirm-title="Delete snapshot"
          data-confirm-body="Delete {name}? This cannot be undone."
          data-confirm-confirm-label="Delete"
          data-confirm-style="danger">Delete</button>
</div>"#,
                when = ago(f.now_unix, s.modified_unix),
                size = format_bytes(s.bytes),
            );
        }
        format!(
            r#"<div class="bnb-card pad bkr-mt">
  <div class="section-header"><div><div class="bnb-eyebrow">History</div><h3>Snapshots</h3></div><span class="bnb-pill moss">automatic · every 7 days</span></div>
  {rows}
</div>"#
        )
    };

    // ── Storage — measured, with the two directories that actually grow ───
    let measured_total = f.db_bytes + f.recordings_bytes + f.snapshot_bytes;
    let storage = format!(
        r#"<div class="bnb-card pad bkr-mt">
  <div class="section-header"><div><div class="bnb-eyebrow">Disk</div><h3>Storage breakdown</h3></div><span class="bnb-meta">share of the {total} this station is using</span></div>
  <div class="bkr-storage-grid">
    {a}{b}{c}
  </div>
</div>"#,
        total = format_bytes(measured_total),
        a = storage_tile(
            "Database",
            &format_bytes(f.db_bytes),
            pct_of(f.db_bytes, measured_total),
            "moss"
        ),
        b = storage_tile(
            &format!("Recordings ({} clips)", f.recordings_count),
            &format_bytes(f.recordings_bytes),
            pct_of(f.recordings_bytes, measured_total),
            "dawn"
        ),
        c = storage_tile(
            "Snapshots",
            &format_bytes(f.snapshot_bytes),
            pct_of(f.snapshot_bytes, measured_total),
            "moss"
        ),
    );

    // ── Exports — every one a route that exists ───────────────────────────
    let mut exports = String::new();
    for (name, detail, href, file) in [
        (
            "Detections (CSV)",
            "every detection with date, species and confidence",
            "/detections/export",
            "detections.csv",
        ),
        (
            "Species summary (CSV)",
            "per-species totals and first-seen dates",
            "/species/export",
            "species.csv",
        ),
        (
            "eBird checklist",
            "record format for submission to eBird",
            "/detections/export/ebird",
            "ebird.csv",
        ),
        (
            "BirdNET-Pi BirdDB.txt",
            "tab-separated, for tools expecting the original format",
            "/detections/export/birddb",
            "BirdDB.txt",
        ),
    ] {
        let _ = write!(
            exports,
            r#"<div class="bkr-export-row">
  <div><div class="bkr-row-title">{name}</div><div class="bnb-meta">{detail}</div></div>
  <a class="bnb-btn ghost" href="{href}" download="{file}">Export ↓</a>
</div>"#
        );
    }
    let export_card = format!(
        r#"<div class="bnb-card pad bkr-mt">
  <div class="section-header"><div><div class="bnb-eyebrow">Export</div><h3>Take your data elsewhere</h3></div></div>
  {exports}
</div>"#
    );

    // ── Retention — the values actually in force ──────────────────────────
    let retention = r#"<div class="bnb-card pad bkr-mt">
  <div class="section-header"><div><div class="bnb-eyebrow">Policy</div><h3>Retention</h3></div></div>
  <div class="bkr-retention">
    <div><div class="bnb-meta">Automatic snapshots</div><div class="bnb-row tight bkr-mt-xs"><span class="bnb-pill mono">every 7 days</span></div></div>
    <div><div class="bnb-meta">Snapshots kept</div><div class="bnb-row tight bkr-mt-xs"><span class="bnb-pill mono">14</span></div></div>
    <div><div class="bnb-meta">Clips purged at</div><div class="bnb-row tight bkr-mt-xs"><span class="bnb-pill mono">DISK_PURGE_THRESHOLD</span></div></div>
    <div><div class="bnb-meta">Locked clips</div><div class="bnb-row tight bkr-mt-xs"><span class="bnb-pill mono">never purged</span></div></div>
  </div>
  <p class="bnb-meta bkr-note-mt">Purge and per-species limits are set in the station config (<span class="mono">DISK_PURGE_THRESHOLD</span>, <span class="mono">MAX_FILES_SPECIES</span>). Lock a clip from <a href="/recordings">Recordings</a> to keep it regardless.</p>
</div>"#;

    // ── Danger zone — only the two destructive endpoints that exist ───────
    let danger = r##"<div class="bnb-card pad bkr-mt bkr-danger">
  <div class="section-header"><div><div class="bnb-eyebrow">Danger zone</div><h3>Destructive actions</h3></div></div>
  <p class="bnb-meta bkr-mb-xs">Each action asks for confirmation. There is no undo — take a full backup first.</p>
  <div class="bkr-danger-row">
    <div><div class="bkr-row-title">Clear all detections</div><div class="bnb-meta">empties the detections and notification tables; keeps settings</div></div>
    <button class="bnb-btn danger"
            hx-post="/admin/system/clear-detections"
            hx-target="#bkr-danger-result"
            hx-swap="innerHTML"
            data-confirm-action="hx-post"
            data-confirm-url="/admin/system/clear-detections"
            data-confirm-title="Clear all detections"
            data-confirm-body="Delete ALL detections and notification logs? This cannot be undone."
            data-confirm-confirm-label="Delete all"
            data-confirm-style="danger">Confirm…</button>
  </div>
  <div class="bkr-danger-row">
    <div><div class="bkr-row-title">Clear extracted audio</div><div class="bnb-meta">deletes the saved WAV clips; detection records stay</div></div>
    <button class="bnb-btn danger"
            hx-post="/admin/system/clear-extracted"
            hx-target="#bkr-danger-result"
            hx-swap="innerHTML"
            data-confirm-action="hx-post"
            data-confirm-url="/admin/system/clear-extracted"
            data-confirm-title="Clear extracted audio"
            data-confirm-body="Delete ALL extracted audio clips? This cannot be undone."
            data-confirm-confirm-label="Delete clips"
            data-confirm-style="danger">Confirm…</button>
  </div>
  <div id="bkr-danger-result" class="bnb-meta bkr-note-mt"></div>
</div>"##;

    // ── Right rail ────────────────────────────────────────────────────────
    let rail = format!(
        r##"<aside class="bkr-rail">
  <div class="bnb-card pad">
    <div class="bnb-eyebrow bkr-mb-10">Where backups live</div>
    <p class="bnb-meta">Snapshots are written to <span class="mono">backups/</span> beside the database — <b>on this station</b>. A card failure loses them with everything else, so download a full backup periodically and keep it elsewhere.</p>
  </div>
  <div class="bnb-card pad">
    <div class="bnb-eyebrow">Logs</div>
    <p class="bnb-meta bkr-rail-note">Backup, purge and integrity-check results are written to the service log.</p>
    <a class="bnb-btn ghost bkr-w-full" href="/admin/logs">Open the log viewer</a>
  </div>
  <div class="bnb-card pad">
    <div class="bnb-eyebrow">System update</div>
    <div class="bkr-update-row"><span class="display bkr-update-ver">v{version}</span></div>
    <button class="bnb-btn bkr-w-full"
            hx-get="/admin/system/update/check"
            hx-target="#bkr-update-result"
            hx-swap="innerHTML">Check for updates</button>
    <div id="bkr-update-result" class="bnb-meta bkr-note-mt"></div>
  </div>
</aside>"##,
        version = env!("CARGO_PKG_VERSION"),
    );

    format!(
        r#"<div>
  <div class="bnb-eyebrow">Operations</div>
  <h1 class="display bkr-h1">Backups &amp; recovery</h1>
  <p class="bnb-meta bkr-lede">Snapshots, exports, storage and the controls you hope you never need.</p>
  {stats}
  <div class="bkr-main">
    <div>
      {backup_restore}
      {snapshots}
      {storage}
      {export_card}
      {retention}
      {danger}
    </div>
    {rail}
  </div>
</div>"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts_with_snapshots() -> DataFacts {
        DataFacts {
            db_bytes: 4_194_304,
            recordings_count: 812,
            recordings_bytes: 1_073_741_824,
            snapshots: vec![
                Snapshot {
                    name: "birds.db.backup.1773532800".into(),
                    bytes: 4_194_304,
                    modified_unix: 1_773_532_800,
                },
                Snapshot {
                    name: "birds.db.backup.1772928000".into(),
                    bytes: 4_100_000,
                    modified_unix: 1_772_928_000,
                },
            ],
            snapshot_bytes: 8_294_304,
            scheduled_last_run_unix: Some(1_773_532_800),
            disk_used_pct: Some(61.4),
            disk_available_bytes: 12_884_901_888,
            now_unix: 1_773_619_200, // one day after the newest snapshot
        }
    }

    #[test]
    fn render_body_has_no_static_inline_styles() {
        // P3-3 (O-25): the backups page is built from utility/page classes and
        // carries no inline `style=` attributes at all (those can't take a CSP
        // nonce). The storage tiles' computed bar width rides a `data-style`
        // attribute that the global CSSOM applier writes onto element.style.
        let html = render_body(&facts_with_snapshots());
        // Match the bare attribute (space-prefixed) so `data-style="` — which
        // ends in `style="` — does not trip this guard.
        assert!(
            !html.contains(" style=\""),
            "backups page still emits an inline style attribute"
        );
        let computed = html.matches("data-style=\"").count();
        assert_eq!(
            computed, 3,
            "expected exactly 3 computed data-style widths (the 3 storage bars), found {computed}"
        );
        // Every computed style that remains is a width on a bar fill.
        for frag in html.split("data-style=\"").skip(1) {
            assert!(
                frag.starts_with("width:"),
                "unexpected non-width data-style: {}",
                &frag[..frag.len().min(40)]
            );
        }
    }

    #[test]
    fn no_snapshots_says_never_rather_than_inventing_a_time() {
        // The headline regression: this page used to state "2 h ago" on a
        // station that had never once completed a backup.
        let html = render_body(&DataFacts {
            now_unix: 1_773_619_200,
            ..DataFacts::default()
        });
        assert!(
            html.contains("never"),
            "a station with no snapshots must say so"
        );
        assert!(
            html.contains("No snapshots yet"),
            "the snapshot list needs an honest empty state"
        );
    }

    #[test]
    fn reported_times_and_sizes_come_from_the_facts() {
        let html = render_body(&facts_with_snapshots());
        // The newest snapshot is exactly one day old, and `ago` only switches
        // to days past 48 h, so this reads as hours.
        assert!(html.contains("24 h ago"), "last snapshot age is computed");
        assert!(
            html.contains("birds.db.backup.1773532800"),
            "real snapshot filenames are listed"
        );
        assert!(html.contains("812 clips"), "real clip count is shown");
        assert!(html.contains("61%"), "real disk usage is shown");
        assert!(html.contains("12.0 GB free"), "real free space is shown");
    }

    #[test]
    fn every_snapshot_row_links_to_the_real_download_and_delete_routes() {
        let html = render_body(&facts_with_snapshots());
        for s in &facts_with_snapshots().snapshots {
            assert!(
                html.contains(&format!("/admin/system/backups/{}", s.name)),
                "snapshot {} must link to its real route",
                s.name
            );
        }
    }

    #[test]
    fn no_fabricated_content_remains() {
        // Guards every invented string the mock-up used to present as live
        // telemetry, so none of it can drift back in.
        let html = render_body(&facts_with_snapshots());
        for ghost in [
            "nightly 03:00",
            "verified bootable",
            "Restore tested",
            "Amazon S3",
            "SMB / NAS",
            "8.4 GB",
            "Wikipedia cache",
            "Factory reset",
            "Uninstall",
            "oldest retained",
            "before v0.1.0 upgrade",
        ] {
            assert!(
                !html.contains(ghost),
                "fabricated content is back on the page: {ghost}"
            );
        }
    }

    #[test]
    fn ago_reads_naturally_across_the_ranges() {
        assert_eq!(ago(1000, 1000), "just now");
        assert_eq!(ago(100_000, 100_000 - 600), "10 min ago");
        assert_eq!(ago(500_000, 500_000 - 7200), "2 h ago");
        assert_eq!(ago(1_000_000, 1_000_000 - 3 * 86_400), "3 d ago");
        // Boundary: exactly 48 h is the last reading in hours.
        assert_eq!(ago(1_000_000, 1_000_000 - 2 * 86_400), "2 d ago");
        assert_eq!(ago(1_000_000, 1_000_000 - 86_400), "24 h ago");
        // A timestamp from the future (clock skew) must not underflow.
        assert_eq!(ago(10, 99_999), "just now");
        // A zero timestamp means "unknown", not 1970.
        assert_eq!(ago(1_773_619_200, 0), "just now");
    }

    #[test]
    fn until_handles_overdue_and_future() {
        assert_eq!(until(100, 100), "due now");
        assert_eq!(until(100, 50), "due now");
        assert_eq!(until(0, 3600), "in 60 min");
        assert_eq!(until(0, 7 * 86_400), "in 7 d");
    }

    #[test]
    fn pct_of_is_bounded_and_zero_safe() {
        assert_eq!(pct_of(0, 0), 0, "no division by zero on an empty station");
        assert_eq!(pct_of(50, 100), 50);
        assert_eq!(pct_of(100, 100), 100);
        assert_eq!(pct_of(500, 100), 100, "clamped, never over 100%");
    }

    #[test]
    fn snapshot_names_are_html_escaped() {
        let html = render_body(&DataFacts {
            snapshots: vec![Snapshot {
                name: "birds.db.backup.1<script>".into(),
                bytes: 1,
                modified_unix: 1,
            }],
            now_unix: 100,
            ..DataFacts::default()
        });
        assert!(!html.contains("<script>"), "snapshot names must be escaped");
        assert!(html.contains("&lt;script&gt;"));
    }
}
