//! HTML rendering for species list management.

use std::fmt::Write as _;

use crate::routes::pages::escape_html;

pub fn render_species_page(exclude: &[String], include: &[String]) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Species Lists — BirdNet-Behavior</title>
    <script src="/static/htmx.min.js"></script>
    <link rel="stylesheet" href="/static/style.css">
    <style>
      body {{ background:#0f172a; color:#e2e8f0; font-family:system-ui,sans-serif; }}
      .container {{ max-width:900px; margin:0 auto; padding:2rem 1rem; }}
      nav a {{ color:#94a3b8; text-decoration:none; margin-right:1.5rem; }}
      nav a.active, nav a:hover {{ color:#38bdf8; }}
      .card {{ background:#1e293b; border:1px solid #334155; border-radius:0.75rem; padding:1.5rem; margin-bottom:1.5rem; }}
      .section-title {{ font-size:1.1rem; font-weight:600; color:#38bdf8; margin-bottom:1rem; border-bottom:1px solid #334155; padding-bottom:0.5rem; }}
      label {{ display:block; font-size:0.85rem; color:#94a3b8; margin-bottom:0.25rem; }}
      input {{ width:100%; background:#0f172a; border:1px solid #334155; border-radius:0.375rem; padding:0.5rem 0.75rem; color:#e2e8f0; font-size:0.875rem; box-sizing:border-box; }}
      input:focus {{ outline:none; border-color:#38bdf8; }}
      .btn {{ padding:0.4rem 1rem; border-radius:0.375rem; border:none; cursor:pointer; font-weight:600; font-size:0.85rem; }}
      .btn-primary {{ background:#0ea5e9; color:#fff; }}
      .btn-danger {{ background:#ef4444; color:#fff; }}
      .pill {{ display:inline-flex; align-items:center; gap:0.4rem; background:#0f172a; border:1px solid #334155; border-radius:999px; padding:0.2rem 0.7rem; font-size:0.8rem; margin:0.2rem; }}
      .hint {{ font-size:0.75rem; color:#64748b; margin-top:0.25rem; margin-bottom:1rem; }}
    </style>
</head>
<body>
<div class="container">
  <nav style="margin-bottom:2rem; padding:1rem 0; border-bottom:1px solid #334155;">
    <a href="/">Dashboard</a>
    <a href="/species">Species</a>
    <a href="/admin">Admin</a>
    <a href="/admin/species" class="active">Species Lists</a>
    <a href="/admin/settings">Settings</a>
  </nav>
  <h1 style="font-size:1.5rem;font-weight:700;margin-bottom:1.5rem;color:#f1f5f9;">Species List Management</h1>
  <div id="species-lists">
    {inner}
  </div>
</div>
</body>
</html>"#,
        inner = render_species_partial(exclude, include)
    )
}

pub fn render_species_partial(exclude: &[String], include: &[String]) -> String {
    let mut out = String::with_capacity(4096);
    render_list_card(
        &mut out,
        "Exclusion List",
        "species_exclude",
        exclude,
        "Species that will <strong>never</strong> be saved or notified.",
        "exclude",
    );
    render_list_card(
        &mut out,
        "Allow-List (include only)",
        "species_include",
        include,
        "When non-empty, <strong>only</strong> these species are saved or notified.",
        "include",
    );
    // Per-species thresholds section (loaded via HTMX)
    out.push_str(
        r#"<div id="thresholds-section" hx-get="/admin/species/thresholds" hx-trigger="load" hx-swap="innerHTML"></div>"#,
    );
    out
}

fn render_list_card(
    out: &mut String,
    title: &str,
    _key: &str,
    list: &[String],
    description: &str,
    kind: &str,
) {
    let _ = write!(
        out,
        r#"<div class="card">
  <div class="section-title">{title}</div>
  <p class="hint">{description}</p>
  <div id="{kind}-pills" style="margin-bottom:1rem;min-height:2rem;">"#
    );

    for name in list {
        let esc = escape_html(name);
        let _ = write!(
            out,
            r##"<span class="pill">
    {esc}
    <form hx-post="/admin/species/{kind}/remove" hx-target="#species-lists" hx-swap="innerHTML" style="display:inline;margin:0;">
      <input type="hidden" name="name" value="{esc}">
      <button type="submit" style="background:none;border:none;color:#ef4444;cursor:pointer;padding:0;font-size:0.9rem;line-height:1;" title="Remove">&#x2715;</button>
    </form>
  </span>"##
        );
    }

    if list.is_empty() {
        let _ = write!(
            out,
            r#"<span style="color:#475569;font-size:0.85rem;">No species in this list</span>"#
        );
    }

    let _ = write!(
        out,
        r##"</div>
  <form hx-post="/admin/species/{kind}/add" hx-target="#species-lists" hx-swap="innerHTML"
        style="display:flex;gap:0.5rem;align-items:center;">
    <input type="text" name="name" placeholder="Add species common name" style="flex:1;margin:0;">
    <button type="submit" class="btn btn-primary">Add</button>
  </form>
</div>"##
    );
}

/// Render the per-species confidence thresholds section as an HTMX partial.
pub fn render_thresholds_partial(thresholds: &[birdnet_db::sqlite::SpeciesThreshold]) -> String {
    let mut out = String::with_capacity(2048);
    out.push_str(r#"<div class="card">
  <div class="section-title">Per-Species Confidence Thresholds</div>
  <p class="hint">Override the global confidence threshold for specific species. Detections below the species threshold will be discarded.</p>"#);

    if thresholds.is_empty() {
        out.push_str(
            r#"<p style="color:#475569;font-size:0.85rem;margin-bottom:1rem;">No per-species thresholds configured. The global threshold applies to all species.</p>"#,
        );
    } else {
        out.push_str(
            r#"<table style="width:100%;margin-bottom:1rem;"><thead><tr><th style="text-align:left;">Species</th><th>Threshold</th><th></th></tr></thead><tbody>"#,
        );
        for t in thresholds {
            let esc = escape_html(&t.sci_name);
            let pct = t.confidence_threshold * 100.0;
            let _ = write!(
                out,
                r##"<tr>
  <td>{esc}</td>
  <td style="text-align:center;">{pct:.0}%</td>
  <td style="text-align:right;">
    <form hx-post="/admin/species/thresholds/delete" hx-target="#thresholds-section" hx-swap="innerHTML" style="display:inline;margin:0;">
      <input type="hidden" name="sci_name" value="{esc}">
      <button type="submit" class="btn btn-danger" style="padding:0.2rem 0.6rem;font-size:0.75rem;">Remove</button>
    </form>
  </td>
</tr>"##
            );
        }
        out.push_str("</tbody></table>");
    }

    out.push_str(
        r##"<form hx-post="/admin/species/thresholds/set" hx-target="#thresholds-section" hx-swap="innerHTML"
      style="display:flex;gap:0.5rem;align-items:center;">
    <input type="text" name="sci_name" placeholder="Scientific name (e.g. Turdus merula)" style="flex:2;margin:0;">
    <input type="number" name="threshold" min="0" max="1" step="0.05" value="0.50" placeholder="0.0–1.0" style="flex:1;margin:0;max-width:100px;">
    <button type="submit" class="btn btn-primary">Set</button>
  </form>
</div>"##,
    );

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_empty_lists() {
        let html = render_species_partial(&[], &[]);
        assert!(html.contains("No species in this list"));
        assert!(html.contains("Exclusion List"));
        assert!(html.contains("Allow-List"));
    }

    #[test]
    fn render_with_species() {
        let html = render_species_partial(
            &["House Sparrow".to_string()],
            &["European Robin".to_string()],
        );
        assert!(html.contains("House Sparrow"));
        assert!(html.contains("European Robin"));
    }
}
