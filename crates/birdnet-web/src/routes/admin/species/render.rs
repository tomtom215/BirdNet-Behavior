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
    <script src="/static/theme-guard.js"></script><link rel="stylesheet" href="/static/css/app.css">
    <style>
      body {{ background:var(--bg); color:var(--fg); font-family:var(--font-ui); }}
      .container {{ max-width:900px; margin:0 auto; padding:2rem 1rem; }}
      nav {{ margin-bottom:2rem; padding:1rem 0; border-bottom:1px solid var(--border); }}
      nav a {{ color:var(--fg-3); text-decoration:none; margin-right:1.5rem; }}
      nav a.active, nav a:hover {{ color:var(--moss-ink); }}
      h1 {{ font-size:1.5rem; font-weight:700; margin-bottom:1.5rem; color:var(--fg); }}
      .card {{ background:var(--surface); border:1px solid var(--border); border-radius:0.75rem; padding:1.5rem; margin-bottom:1.5rem; }}
      .section-title {{ font-size:1.1rem; font-weight:600; color:var(--moss-ink); margin-bottom:1rem; border-bottom:1px solid var(--border); padding-bottom:0.5rem; }}
      label {{ display:block; font-size:0.85rem; color:var(--fg-3); margin-bottom:0.25rem; }}
      input {{ width:100%; background:var(--bg); border:1px solid var(--border); border-radius:0.375rem; padding:0.5rem 0.75rem; color:var(--fg); font-size:0.875rem; box-sizing:border-box; }}
      input:focus {{ outline:none; border-color:var(--moss-ink); }}
      .btn {{ padding:0.4rem 1rem; border-radius:0.375rem; border:none; cursor:pointer; font-weight:600; font-size:0.85rem; }}
      .btn-primary {{ background:var(--moss); color:#fff; }}
      .btn-danger {{ background:var(--rare); color:#fff; }}
      .pill {{ display:inline-flex; align-items:center; gap:0.4rem; background:var(--bg); border:1px solid var(--border); border-radius:999px; padding:0.2rem 0.7rem; font-size:0.8rem; margin:0.2rem; }}
      .hint {{ font-size:0.75rem; color:var(--fg-4); margin-top:0.25rem; margin-bottom:1rem; }}
      .pills {{ margin-bottom:1rem; min-height:2rem; }}
      .pill-x {{ background:none; border:none; color:var(--rare); cursor:pointer; padding:0; font-size:0.9rem; line-height:1; }}
      .empty-note {{ color:var(--border-2); font-size:0.85rem; }}
      .empty-note.mb {{ margin-bottom:1rem; }}
      .inline-form {{ display:inline; margin:0; }}
      .add-row {{ display:flex; gap:0.5rem; align-items:center; }}
      .grow {{ flex:1; margin:0; }}
      .grow.cap {{ max-width:140px; }}
      .grow-2 {{ flex:2; margin:0; }}
      .thr-table {{ width:100%; margin-bottom:1rem; }}
      .cell-left {{ text-align:left; }}
      .cell-center {{ text-align:center; }}
      .cell-right {{ text-align:right; }}
      .del-btn {{ padding:0.2rem 0.6rem; font-size:0.75rem; }}
    </style>
</head>
<body>
<div class="container">
  <nav>
    <a href="/">Dashboard</a>
    <a href="/species">Species</a>
    <a href="/admin">Admin</a>
    <a href="/admin/species" class="active">Species Lists</a>
    <a href="/admin/species/test">Filter Test</a>
    <a href="/admin/settings">Settings</a>
  </nav>
  <h1>Species List Management</h1>
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
  <div id="{kind}-pills" class="pills">"#
    );

    for name in list {
        let esc = escape_html(name);
        let _ = write!(
            out,
            r##"<span class="pill">
    {esc}
    <form hx-post="/admin/species/{kind}/remove" hx-target="#species-lists" hx-swap="innerHTML" class="inline-form">
      <input type="hidden" name="name" value="{esc}">
      <button type="submit" class="pill-x" title="Remove">&#x2715;</button>
    </form>
  </span>"##
        );
    }

    if list.is_empty() {
        let _ = write!(
            out,
            r#"<span class="empty-note">No species in this list</span>"#
        );
    }

    let _ = write!(
        out,
        r##"</div>
  <form hx-post="/admin/species/{kind}/add" hx-target="#species-lists" hx-swap="innerHTML" class="add-row">
    <input type="text" name="name" placeholder="Add species common name" class="grow">
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
            r#"<p class="empty-note mb">No per-species thresholds configured. The global threshold applies to all species.</p>"#,
        );
    } else {
        out.push_str(
            r#"<table class="thr-table"><thead><tr><th class="cell-left">Species</th><th>Threshold</th><th></th></tr></thead><tbody>"#,
        );
        for t in thresholds {
            let esc = escape_html(&t.sci_name);
            let pct = t.confidence_threshold * 100.0;
            let _ = write!(
                out,
                r##"<tr>
  <td>{esc}</td>
  <td class="cell-center">{pct:.0}%</td>
  <td class="cell-right">
    <form hx-post="/admin/species/thresholds/delete" hx-target="#thresholds-section" hx-swap="innerHTML" class="inline-form">
      <input type="hidden" name="sci_name" value="{esc}">
      <button type="submit" class="btn btn-danger del-btn">Remove</button>
    </form>
  </td>
</tr>"##
            );
        }
        out.push_str("</tbody></table>");
    }

    out.push_str(
        r##"<form hx-post="/admin/species/thresholds/set" hx-target="#thresholds-section" hx-swap="innerHTML" class="add-row">
    <input type="text" name="sci_name" placeholder="Scientific name (e.g. Turdus merula)" class="grow-2">
    <input type="text" inputmode="decimal" pattern="[0-9]*[.,]?[0-9]*"
           name="threshold" value="0.50" placeholder="0.0–1.0 (e.g. 0,50 or 0.50)"
           class="grow cap">
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
#[allow(clippy::too_many_lines)]
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

        let (badge, reason_txt) = blocked_reason.map_or_else(
            || {
                pass_count += 1;
                (r#"<span class="badge-pass">Pass</span>"#, "—")
            },
            |reason| {
                block_count += 1;
                (r#"<span class="badge-block">Blocked</span>"#, reason)
            },
        );

        let esc_com = escape_html(com_name);
        let esc_sci = escape_html(sci_name);
        let _ = std::fmt::write(
            &mut rows,
            format_args!(
                "<tr><td>{esc_com}</td><td class=\"sci\">{esc_sci}</td><td class=\"cell-center\">{count}</td><td class=\"cell-center\">{badge}</td><td class=\"reason\">{reason_txt}</td></tr>"
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
    <script src="/static/theme-guard.js"></script><link rel="stylesheet" href="/static/css/app.css">
    <style>
      body {{ background:var(--bg); color:var(--fg); font-family:var(--font-ui); }}
      .container {{ max-width:960px; margin:0 auto; padding:2rem 1rem; }}
      nav {{ margin-bottom:2rem; padding:1rem 0; border-bottom:1px solid var(--border); }}
      nav a {{ color:var(--fg-3); text-decoration:none; margin-right:1.5rem; }}
      nav a.active, nav a:hover {{ color:var(--moss-ink); }}
      h1 {{ font-size:1.5rem; font-weight:700; margin-bottom:0.5rem; color:var(--fg); }}
      .card {{ background:var(--surface); border:1px solid var(--border); border-radius:0.75rem; padding:1.5rem; margin-bottom:1.5rem; }}
      .section-title {{ font-size:1.1rem; font-weight:600; color:var(--moss-ink); margin-bottom:1rem; border-bottom:1px solid var(--border); padding-bottom:0.5rem; }}
      table {{ width:100%; border-collapse:collapse; }}
      th, td {{ padding:0.5rem 0.75rem; border-bottom:1px solid var(--surface); text-align:left; }}
      th {{ color:var(--fg-3); font-size:0.8rem; font-weight:600; text-transform:uppercase; background:var(--bg); }}
      tr:hover td {{ background:var(--surface)44; }}
      .stat {{ display:inline-block; padding:0.4rem 1rem; border-radius:0.5rem; font-weight:700; font-size:0.9rem; margin-right:0.5rem; }}
      .stat.pass {{ background:var(--moss-soft); color:var(--moss); }}
      .stat.block {{ background:var(--rare-soft); color:var(--rare); }}
      .hint {{ font-size:0.75rem; color:var(--fg-4); margin-bottom:1rem; }}
      .badge-pass {{ background:var(--moss); color:#fff; padding:0.15rem 0.5rem; border-radius:999px; font-size:0.75rem; font-weight:700; }}
      .badge-block {{ background:var(--rare); color:#fff; padding:0.15rem 0.5rem; border-radius:999px; font-size:0.75rem; font-weight:700; }}
      .filter-row {{ margin-bottom:1rem; }}
      .label-muted {{ color:var(--fg-3); }}
      .edit-link {{ color:var(--moss-ink); font-size:0.85rem; }}
      .muted {{ color:var(--fg-4); }}
      .muted-sm {{ color:var(--fg-4); font-size:0.85rem; }}
      .sci {{ color:var(--fg-3); font-style:italic; }}
      .reason {{ color:var(--fg-3); font-size:0.8rem; }}
      .cell-center {{ text-align:center; }}
      .pill-sm {{ display:inline-block; background:var(--bg); border:1px solid var(--border); border-radius:999px; padding:0.15rem 0.6rem; font-size:0.8rem; margin:0.15rem; }}
    </style>
</head>
<body>
<div class="container">
  <nav>
    <a href="/">Dashboard</a>
    <a href="/species">Species</a>
    <a href="/admin">Admin</a>
    <a href="/admin/species">Species Lists</a>
    <a href="/admin/species/test" class="active">Filter Test</a>
    <a href="/admin/settings">Settings</a>
  </nav>
  <h1>Species Filter Preview</h1>
  <p class="hint">Shows which species from your detection history pass or are blocked by the current exclude/allow-list filters.</p>

  <div class="card">
    <div class="section-title">Current Filters</div>
    <div class="filter-row">
      <strong class="label-muted">Exclusion list:</strong>
      {excl_pills}
    </div>
    <div class="filter-row">
      <strong class="label-muted">Allow-list:</strong>
      {incl_pills}
    </div>
    <a href="/admin/species" class="edit-link">Edit filters →</a>
  </div>

  <div class="card">
    <div class="section-title">Detection History Filter Results</div>
    <div class="filter-row">
      <span class="stat pass">{pass_count} Pass</span>
      <span class="stat block">{block_count} Blocked</span>
      <span class="muted-sm">{total} species in history</span>
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
            "<p class=\"muted\">No detections in the database yet.</p>".to_string()
        } else {
            format!(
                r#"<table>
  <thead><tr>
    <th>Common Name</th>
    <th>Scientific Name</th>
    <th class="cell-center">Detections</th>
    <th class="cell-center">Status</th>
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
        return "<span class=\"muted-sm\">None</span>".to_string();
    }
    let mut out = String::new();
    for s in list {
        write!(out, "<span class=\"pill-sm\">{}</span>", escape_html(s)).unwrap_or_default();
    }
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

    #[test]
    fn pages_have_no_inline_style_attributes() {
        // P3-3 (O-25): both species pages and their HTMX fragments carry no
        // inline `style=` attributes — styling lives in each page's own <style>
        // block; Pass/Blocked badges and filter stats use enumerable classes.
        assert!(!render_species_page(&["House Sparrow".to_string()], &[]).contains("style=\""));
        assert!(!render_species_partial(&["House Sparrow".to_string()], &[]).contains("style=\""));
        let species = vec![
            (
                "Turdus merula".to_string(),
                "Eurasian Blackbird".to_string(),
                9_u64,
            ),
            (
                "Passer domesticus".to_string(),
                "House Sparrow".to_string(),
                3_u64,
            ),
        ];
        assert!(
            !render_filter_test_page(&["House Sparrow".to_string()], &[], &species)
                .contains("style=\"")
        );
    }
}
