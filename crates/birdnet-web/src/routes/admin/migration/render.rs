//! HTML rendering helpers for the migration UI.

use birdnet_migrate::progress::{MigrationProgress, MigrationStage};
use birdnet_migrate::schema::DetectedSchema;
use birdnet_migrate::traits::ValidationReport;

/// Escape HTML special characters.
pub fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Render the full migration page.
pub fn migration_page(dest_db_path: &str) -> String {
    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>BirdNET-Pi Migration - BirdNet-Behavior</title>
  <script src="/static/htmx.min.js"></script>
  <style>
    body {{ background:#0f172a; color:#e2e8f0; font-family:system-ui,sans-serif; }}
    .container {{ max-width:860px; margin:0 auto; padding:2rem 1rem; }}
    nav a {{ color:#94a3b8; text-decoration:none; margin-right:1.5rem; }}
    nav a:hover {{ color:#38bdf8; }}
    .card {{ background:#1e293b; border:1px solid #334155; border-radius:0.75rem;
             padding:1.5rem; margin-bottom:1.5rem; }}
    label {{ display:block; font-size:0.85rem; color:#94a3b8; margin-bottom:0.25rem; }}
    input[type=text],input[type=file] {{ width:100%; background:#0f172a; border:1px solid #334155;
             border-radius:0.375rem; padding:0.5rem 0.75rem; color:#e2e8f0;
             font-size:0.875rem; box-sizing:border-box; }}
    input[type=file] {{ padding:0.35rem 0.5rem; cursor:pointer; }}
    .btn {{ padding:0.5rem 1.5rem; border-radius:0.375rem; border:none;
            cursor:pointer; font-weight:600; font-size:0.875rem; }}
    .btn-primary {{ background:#0ea5e9; color:#fff; }}
    .btn-primary:hover {{ background:#38bdf8; }}
    .btn-secondary {{ background:#334155; color:#e2e8f0; }}
    .btn-secondary:hover {{ background:#475569; }}
    .hint {{ font-size:0.75rem; color:#64748b; margin-top:0.25rem; }}
    code {{ background:#0f172a; border:1px solid #334155; padding:0.1em 0.4em;
            border-radius:0.2rem; font-family:monospace; font-size:0.875rem; }}
    .tabs {{ display:flex; gap:0.5rem; margin-bottom:1rem; }}
    .tab {{ padding:0.4rem 1rem; border-radius:0.375rem; border:1px solid #334155;
            cursor:pointer; font-size:0.85rem; background:#0f172a; color:#94a3b8; }}
    .tab.active {{ background:#0ea5e9; color:#fff; border-color:#0ea5e9; }}
    .tab-panel {{ display:none; }}
    .tab-panel.active {{ display:block; }}
  </style>
</head>
<body>
<div class="container">
  <nav style="margin-bottom:2rem;padding:1rem 0;border-bottom:1px solid #334155;">
    <a href="/">Dashboard</a>
    <a href="/admin/settings">Settings</a>
    <a href="/admin/migrate" style="color:#38bdf8;">Migration</a>
    <a href="/admin/system">System</a>
    <a href="/admin/notifications">Notifications</a>
  </nav>

  <h1 style="font-size:1.5rem;font-weight:700;margin-bottom:0.5rem;color:#f1f5f9;">
    BirdNET-Pi Migration
  </h1>
  <p style="color:#94a3b8;margin-bottom:1.5rem;">
    Safely import your existing BirdNET-Pi detection history.
    Your source file is <strong style="color:#f1f5f9;">never modified</strong>
    and your original installation is left completely untouched.
  </p>

  <div class="card" style="border-color:#334155">
    <div style="font-weight:600;color:#38bdf8;margin-bottom:0.75rem;">How it works</div>
    <ol style="padding-left:1.25rem;line-height:1.8;color:#cbd5e1;">
      <li>Optionally stop BirdNET-Pi:
          <code>sudo systemctl stop birdnet_analysis birdnet_recording</code></li>
      <li>Find your BirdNET-Pi database (usually
          <code>~/BirdNET-Pi/scripts/BirdDB.txt</code>).</li>
      <li>Upload the file <em>or</em> enter the server-side path below.</li>
      <li>Click <strong>Validate</strong>, review the report, then <strong>Start Import</strong>.</li>
      <li>Your original BirdNET-Pi installation is untouched and safe to restart.</li>
    </ol>
    <p style="color:#64748b;font-size:0.85rem;margin-top:0.5rem;">
      Destination: <code>{dest_db_path}</code>
    </p>
  </div>

  <div class="card">
    <div class="tabs">
      <button class="tab active" onclick="switchTab('upload')">Upload File</button>
      <button class="tab" onclick="switchTab('path')">Server Path</button>
    </div>

    <!-- File upload tab -->
    <div id="tab-upload" class="tab-panel active">
      <label for="source-file">BirdDB.txt or birds.db file</label>
      <form id="upload-form"
            hx-post="/admin/migrate/upload"
            hx-encoding="multipart/form-data"
            hx-target="#migrate-status"
            hx-swap="innerHTML"
            hx-indicator="#upload-spinner">
        <input type="file" id="source-file" name="source_file"
               accept=".db,.txt,.sqlite,.sqlite3"
               style="margin-bottom:0.75rem;">
        <p class="hint">Accepted formats: BirdDB.txt, birds.db, *.db, *.sqlite</p>
        <div style="margin-top:1rem;display:flex;gap:0.75rem;align-items:center;">
          <button type="submit" class="btn btn-primary">Upload &amp; Import</button>
          <span id="upload-spinner" class="htmx-indicator"
                style="color:#94a3b8;font-size:0.85rem;">Uploading…</span>
        </div>
      </form>
    </div>

    <!-- Server path tab -->
    <div id="tab-path" class="tab-panel">
      <label for="migrate-source-path">Absolute path on this server</label>
      <input id="migrate-source-path" name="source_path" type="text"
             placeholder="/home/pi/BirdNET-Pi/scripts/BirdDB.txt"
             style="margin-bottom:0.75rem;">
      <p class="hint">Full path to the BirdNET-Pi BirdDB.txt or birds.db file on this machine</p>
      <div style="margin-top:1rem;display:flex;gap:0.75rem;flex-wrap:wrap;">
        <button class="btn btn-secondary"
                hx-post="/admin/migrate/validate"
                hx-include="#migrate-source-path"
                hx-target="#validate-result"
                hx-swap="innerHTML">
          Validate Only
        </button>
      </div>
      <div id="validate-result" style="margin-top:1rem;"></div>
    </div>
  </div>

  <div id="migrate-status"></div>
</div>

<script>
function switchTab(name) {{
  document.querySelectorAll('.tab-panel').forEach(p => p.classList.remove('active'));
  document.querySelectorAll('.tab').forEach(t => t.classList.remove('active'));
  document.getElementById('tab-' + name).classList.add('active');
  event.target.classList.add('active');
}}
</script>
</body>
</html>"##
    )
}

/// Render the validation result partial.
pub fn validation_result(
    result: Result<(DetectedSchema, ValidationReport), birdnet_migrate::MigrateError>,
    _is_upload: bool,
) -> String {
    match result {
        Ok((schema, report)) => {
            let schema_name = schema.name();
            let rows = report.source_rows;
            let ok = report.passed;
            let checks_html: String = report
                .checks
                .iter()
                .map(|c| {
                    let icon = if c.passed {
                        r#"<span style="color:#4ade80">✔</span>"#
                    } else if c.required {
                        r#"<span style="color:#f87171">✘</span>"#
                    } else {
                        r#"<span style="color:#fbbf24">⚠</span>"#
                    };
                    format!(
                        r#"<li style="margin-bottom:0.4rem">{icon} <strong>{}</strong>: {}</li>"#,
                        escape_html(&c.name),
                        escape_html(&c.detail),
                    )
                })
                .collect();

            let (color, label) = if ok {
                ("#4ade80", "Validation passed")
            } else {
                ("#fbbf24", "Validation passed with warnings")
            };

            format!(
                r##"<div class="card" style="border-color:{color}">
  <div style="font-weight:600;color:{color};margin-bottom:0.75rem;">{label}</div>
  <p><strong>Schema:</strong> {schema_name}</p>
  <p><strong>Rows to import:</strong> {rows}</p>
  <ul style="list-style:none;padding:0;margin:0.75rem 0;">{checks_html}</ul>
  <button class="btn btn-primary"
          hx-post="/admin/migrate/run"
          hx-include="#migrate-source-path"
          hx-target="#migrate-status"
          style="margin-top:0.75rem;">
    Start Import
  </button>
</div>"##
            )
        }
        Err(e) => format!(
            r#"<div class="card" style="border-color:#f87171">
  <div style="font-weight:600;color:#f87171;margin-bottom:0.5rem;">Validation failed</div>
  <p>{}</p>
</div>"#,
            escape_html(&e.to_string())
        ),
    }
}

/// Render an upload error partial.
pub fn upload_error(msg: &str) -> String {
    format!(
        r#"<div class="card" style="border-color:#f87171">
  <div style="font-weight:600;color:#f87171;margin-bottom:0.5rem;">Upload failed</div>
  <p>{}</p>
</div>"#,
        escape_html(msg)
    )
}

/// Render the "import started" partial (triggers progress polling).
pub fn import_started() -> String {
    r#"<div id="migrate-status">
  <p style="color:#94a3b8">Import started. Polling for progress…</p>
  <div id="migrate-progress"
       hx-get="/admin/migrate/progress"
       hx-trigger="every 2s"
       hx-swap="outerHTML">
    <div style="background:#1e293b;border-radius:9999px;height:8px;overflow:hidden;">
      <div style="background:#38bdf8;height:100%;width:0%;transition:width 0.3s;"></div>
    </div>
  </div>
</div>"#
    .to_string()
}

/// Render the progress bar partial.
pub fn progress_bar(p: &MigrationProgress) -> String {
    let pct = p.percent();
    let msg = escape_html(&p.message);
    let trigger = if p.is_terminal() {
        String::new()
    } else {
        r#" hx-get="/admin/migrate/progress" hx-trigger="every 2s" hx-swap="outerHTML""#
            .to_string()
    };
    let color = match p.stage {
        MigrationStage::Complete => "#4ade80",
        MigrationStage::Failed => "#f87171",
        MigrationStage::Cancelled => "#fbbf24",
        _ => "#38bdf8",
    };
    format!(
        r#"<div id="migrate-progress"{trigger}>
  <p style="color:{color};margin-bottom:0.5rem;">{msg}</p>
  <div style="background:#1e293b;border-radius:9999px;height:8px;overflow:hidden;">
    <div style="background:{color};height:100%;width:{pct}%;transition:width 0.3s;"></div>
  </div>
  <p style="color:#64748b;font-size:0.8rem;margin-top:0.25rem;">
    {imported} / {total} rows
  </p>
</div>"#,
        imported = p.rows_imported,
        total = p.rows_total,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_html_special_chars() {
        assert_eq!(escape_html("<script>"), "&lt;script&gt;");
        assert_eq!(escape_html("a & b"), "a &amp; b");
        assert_eq!(escape_html(r#"say "hi""#), "say &quot;hi&quot;");
    }

    #[test]
    fn upload_error_escapes() {
        let html = upload_error("<script>alert(1)</script>");
        assert!(!html.contains("<script>"));
    }

    #[test]
    fn import_started_has_poll() {
        let html = import_started();
        assert!(html.contains("/admin/migrate/progress"));
    }

    #[test]
    fn progress_bar_complete_uses_green() {
        use birdnet_migrate::progress::MigrationStage;
        let p = MigrationProgress {
            stage: MigrationStage::Complete,
            rows_imported: 100,
            rows_total: 100,
            message: "Done".into(),
            error: None,
        };
        let html = progress_bar(&p);
        assert!(html.contains("#4ade80"));
    }
}
