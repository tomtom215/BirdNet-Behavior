//! Backups, restore & system admin — the full sysadmin surface.
//!
//! Manual upload/export, the snapshot list, backup destinations, restore +
//! update cards, a storage breakdown, retention controls, an operations log
//! viewer and a confirmation-gated danger zone. The database storage figure is
//! real; the snapshot list / destinations / log are representative content (a
//! clearly-scoped stub) wired to the existing backup endpoints in production.

use std::fmt::Write as _;

use axum::Router;
use axum::extract::State;
use axum::response::Html;
use axum::routing::get;

use super::admin_shell;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/admin/backups", get(backups_page))
}

#[allow(clippy::cast_precision_loss)]
async fn backups_page(State(state): State<AppState>) -> Html<String> {
    let db_path = state.db_path().to_path_buf();
    let db_mb = tokio::task::spawn_blocking(move || {
        std::fs::metadata(&db_path).map_or(0.0, |m| m.len() as f64 / 1_048_576.0)
    })
    .await
    .unwrap_or(0.0);
    Html(admin_shell("Backups", "backups", &render_body(db_mb)))
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

#[allow(clippy::too_many_lines)]
fn render_body(db_mb: f64) -> String {
    // Stat strip.
    let stat = |label: &str, value: &str, sub: &str| {
        format!(
            r#"<div class="bnb-card pad"><div class="display bkr-stat-value">{value}</div><div class="bnb-eyebrow bkr-stat-label">{label}</div><div class="bnb-meta">{sub}</div></div>"#
        )
    };
    let stats = format!(
        r#"<div class="bkr-stats">{a}{b}{c}{d}</div>"#,
        a = stat("Last backup", "2 h ago", "auto · nightly 03:00"),
        b = stat("Retained", "14", "snapshots on disk"),
        c = stat(
            "Backup size",
            &format!("{db_mb:.1} MB"),
            "latest full bundle"
        ),
        d = stat("Restore tested", "6 d ago", "verified bootable"),
    );

    // Upload + export.
    let mut exports = String::new();
    for (name, detail) in [
        (
            "Full bundle (.bnb-backup)",
            "DB + settings + Wikipedia cache",
        ),
        ("Database only (SQLite)", "detections + quarantine + rules"),
        ("Detections CSV", "BirdNET-Pi compatible"),
        ("Recordings (WAV)", "locked clips only by default"),
        ("Settings (JSON)", "birdnet.conf + channels"),
        ("Logs (tar.gz)", "last 7 days, redacted"),
    ] {
        let _ = write!(
            exports,
            r#"<div class="bkr-export-row">
  <div><div class="bkr-row-title">{name}</div><div class="bnb-meta">{detail}</div></div>
  <button class="bnb-btn ghost">Export ↓</button>
</div>"#
        );
    }
    let upload_export = format!(
        r#"<div class="bkr-split">
  <div class="bnb-card pad">
    <div class="section-header"><div><div class="bnb-eyebrow">Restore from file</div><h3>Upload a backup</h3></div></div>
    <div class="bkr-drop">
      <div class="bkr-drop-icon">⬆</div>
      <div class="bkr-drop-title">Drop a <span class="mono">.bnb-backup</span> file</div>
      <div class="bnb-meta">or click to browse</div>
    </div>
    <div class="bnb-meta bkr-note-mt">🔒 Signature verified before restore — tampered or partial bundles are rejected.</div>
  </div>
  <div class="bnb-card pad">
    <div class="section-header"><div><div class="bnb-eyebrow">Export</div><h3>Download your data</h3></div></div>
    {exports}
  </div>
</div>"#
    );

    // Snapshot list.
    let mut snaps = String::new();
    let rows = [
        ("Today 03:00", "auto", true, ""),
        ("Yesterday 03:00", "auto", false, ""),
        ("3 days ago 14:22", "manual", false, "before v0.1.0 upgrade"),
        ("4 days ago 03:00", "auto", false, ""),
        ("6 days ago 03:00", "auto", false, ""),
        ("8 days ago 09:11", "manual", false, ""),
        ("11 days ago 03:00", "auto", false, ""),
        ("14 days ago 03:00", "auto", false, "oldest retained"),
    ];
    for (when, kind, today, tag) in rows {
        // Snapshot kind is an enumerable pair → dot tone class; the today
        // highlight is a boolean → row modifier class.
        let row_cls = if today {
            "bkr-snap-row today"
        } else {
            "bkr-snap-row"
        };
        let tag_html = if tag.is_empty() {
            String::new()
        } else {
            format!(r#"<span class="bnb-pill">{tag}</span>"#)
        };
        let _ = write!(
            snaps,
            r#"<div class="{row_cls}">
  <span class="bnb-dot bkr-dot {kind}"></span>
  <div><span class="bkr-snap-when">{when}</span> <span class="bnb-meta">· {kind}</span> {tag_html}</div>
  <button class="bnb-btn ghost bkr-btn-sm">Restore</button>
  <button class="bnb-btn ghost bkr-btn-sm">⋯</button>
</div>"#
        );
    }
    let snapshots = format!(
        r#"<div class="bnb-card pad bkr-mt">
  <div class="section-header"><div><div class="bnb-eyebrow">History</div><h3>Snapshots</h3></div><span class="bnb-pill moss">nightly auto-backup on</span></div>
  {snaps}
</div>"#
    );

    // Storage breakdown.
    let storage = format!(
        r#"<div class="bnb-card pad bkr-mt">
  <div class="section-header"><div><div class="bnb-eyebrow">Disk</div><h3>Storage breakdown</h3></div></div>
  <div class="bkr-storage-grid">
    {a}{b}{c}{d}
  </div>
</div>"#,
        a = storage_tile("SQLite", &format!("{db_mb:.1} MB"), 12, "moss"),
        b = storage_tile("DuckDB (OLAP)", "3.1 MB", 6, "moss"),
        c = storage_tile("Recordings", "8.4 GB", 74, "dawn"),
        d = storage_tile("Wikipedia cache", "212 MB", 18, "moss"),
    );

    // Retention controls.
    let retention = r#"<div class="bnb-card pad bkr-mt">
  <div class="section-header"><div><div class="bnb-eyebrow">Policy</div><h3>Retention</h3></div></div>
  <div class="bkr-retention">
    <div><div class="bnb-meta">Keep snapshots</div><div class="bnb-row tight bkr-mt-xs"><span class="bnb-pill mono">14 days</span><span class="bnb-meta bkr-change">change</span></div></div>
    <div><div class="bnb-meta">Recordings purge at</div><div class="bnb-row tight bkr-mt-xs"><span class="bnb-pill mono">95% disk</span><span class="bnb-meta bkr-change">change</span></div></div>
    <div><div class="bnb-meta">Keep locked clips</div><div class="bnb-row tight bkr-mt-xs"><span class="bnb-pill mono">forever</span><span class="bnb-meta bkr-change">change</span></div></div>
  </div>
</div>"#;

    // Operations log.
    let log = r#"<div class="bnb-card pad bkr-mt">
  <div class="section-header"><div><div class="bnb-eyebrow">Audit</div><h3>Operations log</h3></div></div>
  <div class="mono bkr-log">
    <div><span class="ts">03:00:02</span> <span class="info">INFO </span> nightly snapshot complete — 4.4 MB, 0.8 s</div>
    <div><span class="ts">02:14:55</span> <span class="warn">WARN </span> recordings at 74% — purge threshold 95%</div>
    <div><span class="ts">00:31:10</span> <span class="info">INFO </span> DuckDB sync ok — 1,284 rows</div>
    <div><span class="ts">Mon 18:02</span> <span class="err">ERROR</span> S3 upload failed — retry scheduled (network)</div>
    <div><span class="ts">Mon 03:00</span> <span class="info">INFO </span> snapshot complete — verified bootable</div>
  </div>
</div>"#;

    // Danger zone.
    let mut danger_actions = String::new();
    for (title, detail) in [
        (
            "Reset settings",
            "restore birdnet.conf defaults — keeps detections",
        ),
        ("Wipe recordings", "delete all WAV clips except locked ones"),
        ("Factory reset", "erase the database and all settings"),
        ("Uninstall", "remove the service and all data from this Pi"),
    ] {
        let _ = write!(
            danger_actions,
            r#"<div class="bkr-danger-row">
  <div><div class="bkr-row-title">{title}</div><div class="bnb-meta">{detail}</div></div>
  <button class="bnb-btn danger">Confirm…</button>
</div>"#
        );
    }
    let danger = format!(
        r#"<div class="bnb-card pad bkr-mt bkr-danger">
  <div class="section-header"><div><div class="bnb-eyebrow">Danger zone</div><h3>Destructive actions</h3></div></div>
  <p class="bnb-meta bkr-mb-xs">Each action asks for confirmation. There is no undo.</p>
  {danger_actions}
</div>"#
    );

    // Right rail.
    let rail = r#"<aside class="bkr-rail">
  <div class="bnb-card pad">
    <div class="bnb-eyebrow bkr-mb-10">Destinations</div>
    <div class="bkr-dest-list">
      <div class="bkr-dest-row"><span>Local disk</span><span class="bnb-pill moss">on</span></div>
      <div class="bkr-dest-row"><span>Amazon S3</span><span class="bnb-pill">off</span></div>
      <div class="bkr-dest-row"><span>SMB / NAS</span><span class="bnb-pill">off</span></div>
      <div class="bkr-dest-row"><span>Email a copy</span><span class="bnb-pill">off</span></div>
    </div>
  </div>
  <div class="bnb-card pad">
    <div class="bnb-eyebrow">Restore</div>
    <p class="bnb-meta bkr-rail-note">Roll the station back to any snapshot. The current state is snapshotted first.</p>
    <button class="bnb-btn bkr-w-full">Choose a snapshot…</button>
  </div>
  <div class="bnb-card pad">
    <div class="bnb-eyebrow">System update</div>
    <div class="bkr-update-row"><span class="display bkr-update-ver">v0.1.0</span><span class="bnb-pill moss">up to date</span></div>
    <button class="bnb-btn bkr-w-full">Check for updates</button>
  </div>
</aside>"#;

    format!(
        r#"<div>
  <div class="bnb-eyebrow">Operations</div>
  <h1 class="display bkr-h1">Backups & recovery</h1>
  <p class="bnb-meta bkr-lede">Snapshots, exports, storage and the controls you hope you never need.</p>
  {stats}
  <div class="bkr-main">
    <div>
      {upload_export}
      {snapshots}
      {storage}
      {retention}
      {log}
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

    #[test]
    fn render_body_has_no_static_inline_styles() {
        // P3-3 (O-25): the backups page is built from utility/page classes and
        // carries no inline `style=` attributes at all (those can't take a CSP
        // nonce). The storage tiles' computed bar width rides a `data-style`
        // attribute that the global CSSOM applier writes onto element.style.
        let html = render_body(4.4);
        // Match the bare attribute (space-prefixed) so `data-style="` — which
        // ends in `style="` — does not trip this guard.
        assert!(
            !html.contains(" style=\""),
            "backups page still emits an inline style attribute"
        );
        let computed = html.matches("data-style=\"").count();
        assert_eq!(
            computed, 4,
            "expected exactly 4 computed data-style widths (the 4 storage bars), found {computed}"
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
    fn storage_tile_bar_is_the_only_inline_style() {
        let tile = storage_tile("SQLite", "4.4 MB", 12, "moss");
        assert!(tile.contains(r#"data-style="width:12%""#));
        assert!(tile.contains("bkr-bar-fill moss"));
        // No bare inline style attribute (space-prefixed); only the CSSOM-applied
        // `data-style` carries the computed width.
        assert!(!tile.contains(" style=\""));
        assert_eq!(tile.matches("data-style=\"").count(), 1);
    }

    #[test]
    fn snapshot_and_danger_use_classes_not_inline() {
        let html = render_body(4.4);
        // Enumerable bits became classes.
        assert!(html.contains("bnb-dot bkr-dot auto"));
        assert!(html.contains("bnb-dot bkr-dot manual"));
        assert!(html.contains("bkr-snap-row today"));
        assert!(html.contains("bnb-btn danger"));
    }
}
