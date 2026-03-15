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
    <a href="/admin/species/test">Filter Test</a>
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

/// Render the species filter test page.
///
/// Shows all species seen in the detection history alongside their current
/// filter status (Pass / Blocked) based on the loaded exclude/include lists.
pub fn render_filter_test_page(
    exclude: &[String],
    include: &[String],
    species: &[(String, String, u64)], // (sci_name, com_name, count)
) -> String {
    use std::collections::HashSet;

    let exclude_set: HashSet<&str> = exclude.iter().map(String::as_str).collect();
    let include_set: HashSet<&str> = include.iter().map(String::as_str).collect();
    let has_include = !include_set.is_empty();

    let mut rows = String::new();
    let mut pass_count = 0usize;
    let mut block_count = 0usize;

    for (sci_name, com_name, count) in species {
        let in_exclude = exclude_set.iter().any(|e| e.eq_ignore_ascii_case(com_name));
        let in_include = include_set.iter().any(|i| i.eq_ignore_ascii_case(com_name));
        let blocked_reason = if in_exclude {
            Some("Excluded")
        } else if has_include && !in_include {
            Some("Not in allow-list")
        } else {
            None
        };

        let (badge, reason_txt) = if let Some(reason) = blocked_reason {
            block_count += 1;
            (
                r#"<span style="background:#ef4444;color:#fff;padding:0.15rem 0.5rem;border-radius:999px;font-size:0.75rem;font-weight:700;">Blocked</span>"#,
                reason,
            )
        } else {
            pass_count += 1;
            (
                r#"<span style="background:#22c55e;color:#fff;padding:0.15rem 0.5rem;border-radius:999px;font-size:0.75rem;font-weight:700;">Pass</span>"#,
                "—",
            )
        };

        let esc_com = escape_html(com_name);
        let esc_sci = escape_html(sci_name);
        let _ = std::fmt::write(
            &mut rows,
            format_args!(
                "<tr><td>{esc_com}</td><td style=\"color:#94a3b8;font-style:italic;\">{esc_sci}</td><td style=\"text-align:center;\">{count}</td><td style=\"text-align:center;\">{badge}</td><td style=\"color:#94a3b8;font-size:0.8rem;\">{reason_txt}</td></tr>"
            ),
        );
    }

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Species Filter Test — BirdNet-Behavior</title>
    <script src="/static/htmx.min.js"></script>
    <link rel="stylesheet" href="/static/style.css">
    <style>
      body {{ background:#0f172a; color:#e2e8f0; font-family:system-ui,sans-serif; }}
      .container {{ max-width:960px; margin:0 auto; padding:2rem 1rem; }}
      nav a {{ color:#94a3b8; text-decoration:none; margin-right:1.5rem; }}
      nav a.active, nav a:hover {{ color:#38bdf8; }}
      .card {{ background:#1e293b; border:1px solid #334155; border-radius:0.75rem; padding:1.5rem; margin-bottom:1.5rem; }}
      .section-title {{ font-size:1.1rem; font-weight:600; color:#38bdf8; margin-bottom:1rem; border-bottom:1px solid #334155; padding-bottom:0.5rem; }}
      table {{ width:100%; border-collapse:collapse; }}
      th, td {{ padding:0.5rem 0.75rem; border-bottom:1px solid #1e293b; text-align:left; }}
      th {{ color:#94a3b8; font-size:0.8rem; font-weight:600; text-transform:uppercase; background:#0f172a; }}
      tr:hover td {{ background:#1e293b44; }}
      .stat {{ display:inline-block; padding:0.4rem 1rem; border-radius:0.5rem; font-weight:700; font-size:0.9rem; margin-right:0.5rem; }}
      .hint {{ font-size:0.75rem; color:#64748b; margin-bottom:1rem; }}
    </style>
</head>
<body>
<div class="container">
  <nav style="margin-bottom:2rem; padding:1rem 0; border-bottom:1px solid #334155;">
    <a href="/">Dashboard</a>
    <a href="/species">Species</a>
    <a href="/admin">Admin</a>
    <a href="/admin/species">Species Lists</a>
    <a href="/admin/species/test" class="active">Filter Test</a>
    <a href="/admin/settings">Settings</a>
  </nav>
  <h1 style="font-size:1.5rem;font-weight:700;margin-bottom:0.5rem;color:#f1f5f9;">Species Filter Preview</h1>
  <p class="hint">Shows which species from your detection history pass or are blocked by the current exclude/allow-list filters.</p>

  <div class="card">
    <div class="section-title">Current Filters</div>
    <div style="margin-bottom:1rem;">
      <strong style="color:#94a3b8;">Exclusion list:</strong>
      {excl_pills}
    </div>
    <div style="margin-bottom:1rem;">
      <strong style="color:#94a3b8;">Allow-list:</strong>
      {incl_pills}
    </div>
    <a href="/admin/species" style="color:#38bdf8;font-size:0.85rem;">Edit filters →</a>
  </div>

  <div class="card">
    <div class="section-title">Detection History Filter Results</div>
    <div style="margin-bottom:1rem;">
      <span class="stat" style="background:#1e4620;color:#22c55e;">{pass_count} Pass</span>
      <span class="stat" style="background:#4c1818;color:#ef4444;">{block_count} Blocked</span>
      <span style="color:#64748b;font-size:0.85rem;">{total} species in history</span>
    </div>
    {table_or_empty}
  </div>
</div>
</body>
</html>"#,
        excl_pills = pills_or_none(exclude),
        incl_pills = pills_or_none(include),
        pass_count = pass_count,
        block_count = block_count,
        total = species.len(),
        table_or_empty = if species.is_empty() {
            "<p style=\"color:#64748b;\">No detections in the database yet.</p>".to_string()
        } else {
            format!(
                r#"<table>
  <thead><tr>
    <th>Common Name</th>
    <th>Scientific Name</th>
    <th style="text-align:center;">Detections</th>
    <th style="text-align:center;">Status</th>
    <th>Reason</th>
  </tr></thead>
  <tbody>{rows}</tbody>
</table>"#
            )
        },
    )
}

fn pills_or_none(list: &[String]) -> String {
    if list.is_empty() {
        "<span style=\"color:#64748b;font-size:0.85rem;\">None</span>".to_string()
    } else {
        list.iter()
            .map(|s| {
                format!(
                    "<span style=\"display:inline-block;background:#0f172a;border:1px solid #334155;border-radius:999px;padding:0.15rem 0.6rem;font-size:0.8rem;margin:0.15rem;\">{}</span>",
                    escape_html(s)
                )
            })
            .collect::<Vec<_>>()
            .join("")
    }
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
