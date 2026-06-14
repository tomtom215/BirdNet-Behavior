//! HTML rendering helpers for the migration UI.

use birdnet_migrate::MigrationReport;
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

/// Render the full migration page (document shell + body).
pub fn migration_page(dest_db_path: &str) -> String {
    crate::routes::admin::admin_shell("Migration", "migrate", &migration_body(dest_db_path))
}

/// Render the BirdNET-Pi import body (no document shell).
///
/// Shared with the Station **Data** tab
/// (`crate::routes::pages::homes::station_tabs`), which renders the importer in
/// the main shell.
#[allow(clippy::too_many_lines)]
pub fn migration_body(dest_db_path: &str) -> String {
    format!(
        r##"<style>
    h1 {{ font-size:1.5rem; font-weight:700; margin-bottom:0.5rem; color:var(--fg); }}
    .card {{ background:var(--surface); border:1px solid var(--border); border-radius:0.75rem;
             padding:1.5rem; margin-bottom:1.5rem; }}
    label {{ display:block; font-size:0.85rem; color:var(--fg-3); margin-bottom:0.25rem; }}
    input[type=text],input[type=file] {{ width:100%; background:var(--bg); border:1px solid var(--border);
             border-radius:0.375rem; padding:0.5rem 0.75rem; color:var(--fg);
             font-size:0.875rem; box-sizing:border-box; }}
    input[type=file] {{ padding:0.35rem 0.5rem; cursor:pointer; }}
    .btn {{ padding:0.5rem 1.5rem; border-radius:0.375rem; border:none;
            cursor:pointer; font-weight:600; font-size:0.875rem; }}
    .btn-primary {{ background:var(--moss); color:#fff; }}
    .btn-primary:hover {{ background:var(--moss-ink); }}
    .btn-secondary {{ background:var(--border); color:var(--fg); }}
    .btn-secondary:hover {{ background:var(--border-2); }}
    .hint {{ font-size:0.75rem; color:var(--fg-4); margin-top:0.25rem; }}
    code {{ background:var(--bg); border:1px solid var(--border); padding:0.1em 0.4em;
            border-radius:0.2rem; font-family:monospace; font-size:0.875rem; }}
    .tabs {{ display:flex; gap:0.5rem; margin-bottom:1rem; }}
    .tab {{ padding:0.4rem 1rem; border-radius:0.375rem; border:1px solid var(--border);
            cursor:pointer; font-size:0.85rem; background:var(--bg); color:var(--fg-3); }}
    .tab.active {{ background:var(--moss); color:#fff; border-color:var(--moss); }}
    .tab-panel {{ display:none; }}
    .tab-panel.active {{ display:block; }}
    /* O-25 sweep: shapes promoted out of inline style= attributes. */
    .lede {{ color:var(--fg-3); margin-bottom:1.5rem; }}
    .fg {{ color:var(--fg); }}
    .card-head {{ font-weight:600; color:var(--moss-ink); margin-bottom:0.75rem; }}
    .steps {{ padding-left:1.25rem; line-height:1.8; color:var(--fg-2); }}
    .note {{ color:var(--fg-4); font-size:0.85rem; margin-top:0.5rem; }}
    .mb-sm {{ margin-bottom:0.75rem; }}
    .mt-sm {{ margin-top:0.75rem; }}
    .mt {{ margin-top:1rem; }}
    .actions {{ margin-top:1rem; display:flex; gap:0.75rem; }}
    .actions.center {{ align-items:center; }}
    .actions.wrap {{ flex-wrap:wrap; }}
    .spinner-note {{ color:var(--fg-3); font-size:0.85rem; }}
    .info-text {{ color:var(--fg-3); }}
    .check-ok {{ color:var(--moss); }}
    .check-err {{ color:var(--rare); }}
    .check-warn {{ color:var(--dawn); }}
    .check-list {{ list-style:none; padding:0; margin:0.75rem 0; }}
    .check-item {{ margin-bottom:0.4rem; }}
    .result-card.ok {{ border-color:var(--moss); }}
    .result-card.warn {{ border-color:var(--dawn); }}
    .result-card.err {{ border-color:var(--rare); }}
    .result-title {{ font-weight:600; margin-bottom:0.75rem; }}
    .result-title.sm {{ margin-bottom:0.5rem; }}
    .result-title.ok {{ color:var(--moss); }}
    .result-title.warn {{ color:var(--dawn); }}
    .result-title.err {{ color:var(--rare); }}
    .more-note {{ color:var(--fg-4); font-size:.8rem; margin-top:.5rem; }}
    .preview-details {{ margin:1rem 0; }}
    .preview-summary {{ cursor:pointer; color:var(--fg-3); font-size:.875rem; }}
    .preview-table {{ width:100%; border-collapse:collapse; font-size:.8rem; margin-top:.75rem; }}
    .preview-table td, .preview-table th {{ padding:.35rem .5rem; }}
    .preview-table thead tr {{ border-bottom:1px solid var(--border); }}
    .preview-table th {{ text-align:left; color:var(--fg-4); font-weight:600; }}
    .preview-table th.num, .preview-table td.num {{ text-align:right; }}
    .preview-table .sci {{ color:var(--fg-4); font-style:italic; font-size:.8rem; }}
    .preview-table .muted {{ color:var(--fg-4); }}
    .progress-track {{ background:var(--surface); border-radius:9999px; height:8px; overflow:hidden; }}
    .progress-fill {{ height:100%; width:0; transition:width 0.3s; }}
    .progress-fill.ok {{ background:var(--moss); }}
    .progress-fill.err {{ background:var(--rare); }}
    .progress-fill.warn {{ background:var(--dawn); }}
    .progress-fill.run {{ background:var(--moss-ink); }}
    .bar-msg {{ margin-bottom:0.5rem; }}
    .bar-msg.ok {{ color:var(--moss); }}
    .bar-msg.err {{ color:var(--rare); }}
    .bar-msg.warn {{ color:var(--dawn); }}
    .bar-msg.run {{ color:var(--moss-ink); }}
    .bar-note {{ color:var(--fg-4); font-size:0.8rem; margin-top:0.25rem; }}
  </style>

  <h1>BirdNET-Pi Migration</h1>
  <p class="lede">
    Safely import your existing BirdNET-Pi detection history.
    Your source file is <strong class="fg">never modified</strong>
    and your original installation is left completely untouched.
  </p>

  <div class="card">
    <div class="card-head">How it works</div>
    <ol class="steps">
      <li>Optionally stop BirdNET-Pi:
          <code>sudo systemctl stop birdnet_analysis birdnet_recording</code></li>
      <li>Find your BirdNET-Pi database (usually
          <code>~/BirdNET-Pi/scripts/BirdDB.txt</code>).</li>
      <li>Upload the file <em>or</em> enter the server-side path below.</li>
      <li>Click <strong>Validate</strong>, review the report, then <strong>Start Import</strong>.</li>
      <li>Your original BirdNET-Pi installation is untouched and safe to restart.</li>
    </ol>
    <p class="note">
      Destination: <code>{dest_db_path}</code>
    </p>
  </div>

  <div class="card">
    <div class="tabs" id="migrate-tabs">
      <button class="tab active" data-tab="upload">Upload File</button>
      <button class="tab" data-tab="path">Server Path</button>
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
               class="mb-sm">
        <p class="hint">Accepted formats: BirdDB.txt, birds.db, *.db, *.sqlite</p>
        <div class="actions center">
          <button type="submit" class="btn btn-primary">Upload &amp; Import</button>
          <span id="upload-spinner" class="htmx-indicator spinner-note">Uploading…</span>
        </div>
      </form>
    </div>

    <!-- Server path tab -->
    <div id="tab-path" class="tab-panel">
      <label for="migrate-source-path">Absolute path on this server</label>
      <input id="migrate-source-path" name="source_path" type="text"
             placeholder="/home/pi/BirdNET-Pi/scripts/BirdDB.txt"
             class="mb-sm">
      <p class="hint">Full path to the BirdNET-Pi BirdDB.txt or birds.db file on this machine</p>
      <div class="actions wrap">
        <button class="btn btn-secondary"
                hx-post="/admin/migrate/validate"
                hx-include="#migrate-source-path"
                hx-target="#validate-result"
                hx-swap="innerHTML">
          Validate Only
        </button>
      </div>
      <div id="validate-result" class="mt"></div>
    </div>
  </div>

  <div id="migrate-status"></div>

<script>
function switchTab(name) {{
  document.querySelectorAll('.tab-panel').forEach(p => p.classList.remove('active'));
  document.querySelectorAll('.tab').forEach(t => t.classList.remove('active'));
  document.getElementById('tab-' + name).classList.add('active');
  var trigger = document.querySelector('.tab[data-tab="' + name + '"]');
  if (trigger) trigger.classList.add('active');
}}
document.getElementById('migrate-tabs').addEventListener('click', function(e) {{
  var btn = e.target.closest('button[data-tab]');
  if (btn) switchTab(btn.dataset.tab);
}});
</script>"##
    )
}

/// Render the validation result partial.
#[allow(clippy::too_many_lines)]
pub fn validation_result(
    result: Result<
        (DetectedSchema, ValidationReport, MigrationReport),
        birdnet_migrate::MigrateError,
    >,
    _is_upload: bool,
) -> String {
    match result {
        Ok((schema, report, migration_report)) => {
            let schema_name = schema.name();
            let rows = report.source_rows;
            let ok = report.passed;
            let checks_html: String = {
                use std::fmt::Write as _;
                let mut buf = String::new();
                for c in &report.checks {
                    let icon = if c.passed {
                        r#"<span class="check-ok">✔</span>"#
                    } else if c.required {
                        r#"<span class="check-err">✘</span>"#
                    } else {
                        r#"<span class="check-warn">⚠</span>"#
                    };
                    let _ = write!(
                        buf,
                        r#"<li class="check-item">{icon} <strong>{}</strong>: {}</li>"#,
                        escape_html(&c.name),
                        escape_html(&c.detail),
                    );
                }
                buf
            };

            // Validation tone is an enumerable pair (passed / passed-with-
            // warnings), so it is an explicit class, not a computed inline colour.
            let tone = if ok { "ok" } else { "warn" };
            let label = if ok {
                "Validation passed"
            } else {
                "Validation passed with warnings"
            };

            // Species breakdown table
            let date_range_html = migration_report
                .date_range
                .as_ref()
                .map(|(start, end)| format!("<p><strong>Date range:</strong> {start} → {end}</p>"))
                .unwrap_or_default();

            let quality_html = if migration_report.null_date_rows > 0 {
                format!(
                    r#"<p class="check-warn">⚠ {} rows have missing dates</p>"#,
                    migration_report.null_date_rows
                )
            } else if migration_report.duplicate_rows > 0 {
                format!(
                    r#"<p class="info-text">ℹ {} duplicate rows will be skipped</p>"#,
                    migration_report.duplicate_rows
                )
            } else {
                String::new()
            };

            let top_species_html: String = {
                use std::fmt::Write as _;
                let mut buf = String::new();
                for s in migration_report.top_species.iter().take(10) {
                    let _ = write!(
                        buf,
                        r#"<tr>
  <td>{}</td>
  <td class="sci">{}</td>
  <td class="num">{}</td>
  <td class="num muted">{:.0}%</td>
</tr>"#,
                        escape_html(&s.common_name),
                        escape_html(&s.scientific_name),
                        s.count,
                        s.avg_confidence * 100.0,
                    );
                }
                buf
            };

            let more_species = if migration_report.unique_species > 10 {
                format!(
                    r#"<p class="more-note">
                      … and {} more species
                    </p>"#,
                    migration_report.unique_species - 10
                )
            } else {
                String::new()
            };

            format!(
                r##"<div class="card result-card {tone}">
  <div class="result-title {tone}">{label}</div>
  <p><strong>Schema:</strong> {schema_name}</p>
  <p><strong>Total detections:</strong> {rows}</p>
  <p><strong>Unique species:</strong> {unique}</p>
  {date_range_html}
  {quality_html}
  <ul class="check-list">{checks_html}</ul>

  <details class="preview-details">
    <summary class="preview-summary">
      Top species preview (click to expand)
    </summary>
    <table class="preview-table">
      <thead>
        <tr>
          <th>Species</th>
          <th>Scientific</th>
          <th class="num">Count</th>
          <th class="num">Avg Conf</th>
        </tr>
      </thead>
      <tbody>{top_species_html}</tbody>
    </table>
    {more_species}
  </details>

  <button class="btn btn-primary mt-sm"
          hx-post="/admin/migrate/run"
          hx-include="#migrate-source-path"
          hx-target="#migrate-status">
    Start Import
  </button>
</div>"##,
                unique = migration_report.unique_species,
            )
        }
        Err(e) => format!(
            r#"<div class="card result-card err">
  <div class="result-title err sm">Validation failed</div>
  <p>{}</p>
</div>"#,
            escape_html(&e.to_string())
        ),
    }
}

/// Render an upload error partial.
pub fn upload_error(msg: &str) -> String {
    format!(
        r#"<div class="card result-card err">
  <div class="result-title err sm">Upload failed</div>
  <p>{}</p>
</div>"#,
        escape_html(msg)
    )
}

/// Render the "import started" partial (triggers progress polling).
pub fn import_started() -> String {
    r#"<div id="migrate-status">
  <p class="info-text">Import started. Polling for progress…</p>
  <div id="migrate-progress"
       hx-get="/admin/migrate/progress"
       hx-trigger="every 2s"
       hx-swap="outerHTML">
    <div class="progress-track">
      <div class="progress-fill run"></div>
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
        r#" hx-get="/admin/migrate/progress" hx-trigger="every 2s" hx-swap="outerHTML""#.to_string()
    };
    // Stage tone is an enumerable set, so it is a class; only the continuous
    // bar fill width stays an inline style (computed per request). That single
    // remaining dynamic style folds into a nonce'd <style> block in the P3-3
    // endgame, when `style-src 'unsafe-inline'` is finally dropped.
    let tone = match p.stage {
        MigrationStage::Complete => "ok",
        MigrationStage::Failed => "err",
        MigrationStage::Cancelled => "warn",
        _ => "run",
    };
    format!(
        r#"<div id="migrate-progress"{trigger}>
  <p class="bar-msg {tone}">{msg}</p>
  <div class="progress-track">
    <div class="progress-fill {tone}" data-style="width:{pct}%"></div>
  </div>
  <p class="bar-note">
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
        // Complete stage carries the "ok" (moss/green) tone class.
        assert!(html.contains("progress-fill ok"));
        assert!(html.contains("bar-msg ok"));
    }

    #[test]
    fn migration_page_has_no_inline_style_attributes() {
        // P3-3 (O-25): the static migration page carries no inline style
        // attributes — everything folds into its own <style> block.
        assert!(!migration_page("/data/birdnet.db").contains("style=\""));
    }

    #[test]
    fn import_started_static_bar_has_no_inline_style() {
        // The initial bar sits at a fixed 0% via the class default, so it carries
        // no inline width. (Only the live progress_bar keeps a computed inline
        // width — the documented P3-3 endgame exception.)
        assert!(!import_started().contains("style=\""));
    }

    #[test]
    fn upload_error_partial_has_no_inline_style() {
        // The upload-failure fragment uses the enumerable result-card tone
        // class, not a computed inline colour.
        assert!(!upload_error("nope").contains("style=\""));
    }
}
