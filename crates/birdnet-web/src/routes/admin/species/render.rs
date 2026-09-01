//! HTML rendering for species list management.

use std::fmt::Write as _;

use crate::routes::pages::escape_html;

/// Render the full species list management admin page (shell + content).
pub fn render_species_page(exclude: &[String], include: &[String]) -> String {
    crate::routes::admin::admin_shell(
        "Species lists",
        "species",
        &species_lists_body(exclude, include),
    )
}

/// Page-specific body (scoped `<style>` + content).
///
/// Kept separate from the shared shell so the inline-style guard checks the
/// page's own markup; the `.container` / bare `nav` rules are dropped since the
/// shell owns layout + nav. Shared with the Station **Capture** tab via
/// `super::species_body`.
pub(crate) fn species_lists_body(exclude: &[String], include: &[String]) -> String {
    format!(
        r#"<style>
      h1 {{ font-size:1.5rem; font-weight:700; margin-bottom:1.5rem; color:var(--fg); }}
      .card {{ background:var(--surface); border:1px solid var(--border); border-radius:0.75rem; padding:1.5rem; margin-bottom:1.5rem; }}
      .section-title {{ font-size:1.1rem; font-weight:600; color:var(--moss-ink); margin-bottom:1rem; border-bottom:1px solid var(--border); padding-bottom:0.5rem; }}
      label {{ display:block; font-size:0.85rem; color:var(--fg-3); margin-bottom:0.25rem; }}
      input {{ width:100%; background:var(--bg); border:1px solid var(--border); border-radius:0.375rem; padding:0.5rem 0.75rem; color:var(--fg); font-size:0.875rem; box-sizing:border-box; }}
      input:focus {{ outline:none; border-color:var(--moss-ink); }}
      .btn {{ padding:0.4rem 1rem; border-radius:0.375rem; border:none; cursor:pointer; font-weight:600; font-size:0.85rem; }}
      .btn-primary {{ background:var(--moss); color:var(--on-moss); }}
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

  <h1>Species List Management</h1>
  <div id="species-lists">
    {inner}
  </div>"#,
        inner = render_species_partial(exclude, include)
    )
}

/// Render the HTMX partial fragment containing both the exclusion and allow-list cards.
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
    <input type="text" name="name" placeholder="Common or scientific name" class="grow">
    <button type="submit" class="btn btn-primary">Add</button>
  </form>
  <p class="hint">Either name form works. Changes apply within about half a minute — no restart needed.</p>
</div>"##
    );
}

/// Render the per-species confidence thresholds section as an HTMX partial.
pub fn render_thresholds_partial(
    thresholds: &[birdnet_db::sqlite::SpeciesThreshold],
    suggestions: &[SuggestedThreshold],
) -> String {
    let mut out = String::with_capacity(2048);
    out.push_str(r#"<div class="card">
  <div class="section-title">Per-Species Confidence Thresholds</div>
  <p class="hint">Override the global confidence threshold for specific species. A detection that clears the global threshold but falls short of its species threshold is <b>held for review in <a href="/quarantine">Quarantine</a></b> — not discarded — so you can confirm or reject it yourself. Changes take effect within a minute; no restart needed.</p>"#);

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

    out.push_str(&render_threshold_suggestions(suggestions));

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
    species: &[(String, String, u64)],
) -> String {
    crate::routes::admin::admin_subpage_shell(
        "Filter test",
        "species",
        "Filter test",
        &filter_test_body(exclude, include, species),
    )
}

/// Page-specific body for the species filter-test sub-page (scoped `<style>` +
/// content). The shared shell supplies the chrome, the nav (with the Species tab
/// active), and the `Admin › Species › Filter test` breadcrumb.
fn filter_test_body(
    exclude: &[String],
    include: &[String],
    species: &[(String, String, u64)], // (sci_name, com_name, count)
) -> String {
    use birdnet_core::inference::labels::SpeciesLabel;
    use birdnet_core::inference::species_filter::matches_species;

    // `matches_species` is the detection path's own predicate, called here on
    // purpose: this page is offered as "preview the filter before it affects
    // live detections", which is only true while the preview and the runtime
    // decide with the same code. The previous local comparison matched *common
    // names only*, so an entry typed as a scientific name showed as having no
    // effect here — and once the lists reached the daemon, would have had one.
    let has_include = include.iter().any(|i| !i.trim().is_empty());

    let mut rows = String::new();
    let mut pass_count = 0usize;
    let mut block_count = 0usize;

    for (sci_name, com_name, count) in species {
        let label = SpeciesLabel {
            index: 0,
            scientific_name: sci_name.clone(),
            common_name: com_name.clone(),
            // Rebuilt from stored detection rows, which carry no taxonomy;
            // `matches_species` only ever reads the two names.
            class: None,
        };
        let in_exclude = exclude.iter().any(|e| matches_species(e, &label));
        let in_include = include.iter().any(|i| matches_species(i, &label));
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
        r#"<style>
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
      .badge-pass {{ background:var(--moss); color:var(--on-moss); padding:0.15rem 0.5rem; border-radius:999px; font-size:0.75rem; font-weight:700; }}
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
  </div>"#,
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
        // Check the page *bodies*, not the full `render_*` (the shared admin
        // shell adds `data-confirm-style` attributes a naive substring check
        // would otherwise flag).
        assert!(!species_lists_body(&["House Sparrow".to_string()], &[]).contains("style=\""));
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
            !filter_test_body(&["House Sparrow".to_string()], &[], &species).contains("style=\"")
        );
    }

    #[test]
    fn species_pages_render_through_admin_shell() {
        // Both species pages render through the shared shell with the Species
        // tab active; the filter-test sub-page also carries the breadcrumb.
        let lists = render_species_page(&[], &[]);
        assert!(lists.contains(r#"href="/admin/species" class="am-nav-active""#));
        assert!(lists.contains("Species List Management"));
        let test = render_filter_test_page(&[], &[], &[]);
        assert!(test.contains(r#"href="/admin/species" class="am-nav-active""#));
        assert!(test.contains("bnb-crumbs"));
        assert!(test.contains("Filter test"));
    }
}

/// A threshold suggestion, ready to render.
#[derive(Debug, Clone, PartialEq)]
pub struct SuggestedThreshold {
    /// Scientific name — the key the threshold is stored under.
    pub sci_name: String,
    /// Common name, for the operator reading the row.
    pub com_name: String,
    /// The suggestion and the evidence behind it.
    pub suggestion: birdnet_db::thresholds::ThresholdSuggestion,
    /// The threshold already configured for this species, if any.
    pub current: Option<f64>,
}

/// Suggestions weaker than this are not shown at all.
///
/// Youden's J near zero means confidence carries no information about whether
/// the operator will confirm a detection — the reviews and the model disagree
/// at random. Any threshold then looks as good as any other, and offering one
/// would dress up noise as advice.
pub const MIN_SUGGESTION_J: f64 = 0.2;

/// Render the suggestions block, or nothing when there is nothing worth saying.
fn render_threshold_suggestions(suggestions: &[SuggestedThreshold]) -> String {
    let worth_showing: Vec<&SuggestedThreshold> = suggestions
        .iter()
        .filter(|s| s.suggestion.youden_j() >= MIN_SUGGESTION_J)
        .collect();
    if worth_showing.is_empty() {
        return String::new();
    }

    let mut out = String::with_capacity(1024);
    out.push_str(
        r#"<div class="section-title mt">Suggested from your reviews</div>
  <p class="hint">Worked out from the detections you have confirmed and rejected for each species — the threshold that best separates the two. Nothing is applied until you press Apply. The counts are what the suggestion would have done to the reviews it was derived from.</p>
  <table class="thr-table"><thead><tr><th class="cell-left">Species</th><th>Suggested</th><th>Would have kept</th><th>Would have caught</th><th>Separation</th><th></th></tr></thead><tbody>"#,
    );
    for s in worth_showing {
        let sci = escape_html(&s.sci_name);
        let com = escape_html(&s.com_name);
        let pct = s.suggestion.threshold * 100.0;
        let kept = s.suggestion.confirmed_kept;
        let confirmed_total = s.suggestion.confirmed_kept + s.suggestion.confirmed_lost;
        let caught = s.suggestion.rejected_caught;
        let rejected_total = s.suggestion.rejected_caught + s.suggestion.rejected_kept;
        let j = s.suggestion.youden_j();
        // Naming what is already set matters: the row otherwise reads as new
        // advice when it may be advice the operator already took.
        let current = s.current.map_or_else(
            || "<span class='hint'>none set</span>".to_string(),
            |c| format!("<span class='hint'>now {:.0}%</span>", c * 100.0),
        );
        let _ = write!(
            out,
            r##"<tr>
  <td>{com}<br><span class="hint">{sci}</span></td>
  <td class="cell-center">{pct:.0}%<br>{current}</td>
  <td class="cell-center">{kept} of {confirmed_total} confirmed</td>
  <td class="cell-center">{caught} of {rejected_total} rejected</td>
  <td class="cell-center">{j:.2}</td>
  <td class="cell-right">
    <form hx-post="/admin/species/thresholds/set" hx-target="#thresholds-section" hx-swap="innerHTML" class="inline-form">
      <input type="hidden" name="sci_name" value="{sci}">
      <input type="hidden" name="threshold" value="{:.4}">
      <button type="submit" class="btn btn-primary">Apply</button>
    </form>
  </td>
</tr>"##,
            s.suggestion.threshold
        );
    }
    out.push_str("</tbody></table>");
    out
}

#[cfg(test)]
mod suggestion_tests {
    use super::{MIN_SUGGESTION_J, SuggestedThreshold, render_thresholds_partial};
    use birdnet_db::thresholds::ThresholdSuggestion;

    /// A suggestion with the given split.
    fn suggestion(
        sci: &str,
        threshold: f64,
        confirmed_kept: usize,
        confirmed_lost: usize,
        rejected_caught: usize,
        rejected_kept: usize,
        current: Option<f64>,
    ) -> SuggestedThreshold {
        SuggestedThreshold {
            sci_name: sci.to_owned(),
            com_name: "Common Name".to_owned(),
            suggestion: ThresholdSuggestion {
                threshold,
                confirmed_kept,
                confirmed_lost,
                rejected_caught,
                rejected_kept,
            },
            current,
        }
    }

    /// A clean, well-separated suggestion.
    fn strong() -> SuggestedThreshold {
        suggestion("Turdus merula", 0.85, 8, 0, 6, 0, None)
    }

    #[test]
    fn a_suggestion_is_rendered_with_the_evidence_behind_it() {
        // The numbers are the whole point: the operator is being asked to
        // decide, and "0.85" alone gives them nothing to decide on. What it
        // would have cost and what it would have caught are the two facts.
        let html = render_thresholds_partial(&[], &[strong()]);
        assert!(html.contains("85%"), "the threshold is missing: {html}");
        assert!(
            html.contains("8 of 8 confirmed"),
            "the cost is missing: {html}"
        );
        assert!(
            html.contains("6 of 6 rejected"),
            "the benefit is missing: {html}"
        );
        assert!(html.contains("Turdus merula"), "{html}");
    }

    #[test]
    fn applying_a_suggestion_posts_the_exact_threshold_not_the_rounded_one() {
        // The table shows 85%, but the value that must reach the form is the
        // computed one. Posting the rounded percentage would apply a threshold
        // the evidence was never about, and by a margin big enough to change
        // which detections it admits.
        let html = render_thresholds_partial(
            &[],
            &[suggestion("Turdus merula", 0.8123, 5, 1, 4, 0, None)],
        );
        assert!(
            html.contains(r#"name="threshold" value="0.8123""#),
            "the Apply button posts a rounded threshold: {html}"
        );
        assert!(
            html.contains("81%"),
            "the display should still round: {html}"
        );
    }

    #[test]
    fn a_suggestion_that_separates_nothing_is_not_shown() {
        // Youden's J near zero means the reviews carry no information about
        // confidence. Offering a threshold anyway would dress up noise as
        // advice, and the operator has no way to tell the difference.
        let noise = suggestion("Turdus merula", 0.7, 4, 4, 4, 4, None);
        assert!(noise.suggestion.youden_j().abs() < f64::EPSILON);
        let html = render_thresholds_partial(&[], &[noise]);
        assert!(
            !html.contains("Suggested from your reviews"),
            "a suggestion with no separating power was rendered: {html}"
        );
    }

    #[test]
    fn the_cutoff_admits_a_suggestion_just_above_it() {
        // Counterpart: a filter that hid everything would satisfy the gate
        // above and the feature would never appear.
        assert!(strong().suggestion.youden_j() > MIN_SUGGESTION_J);
        assert!(
            render_thresholds_partial(&[], &[strong()]).contains("Suggested from your reviews")
        );
    }

    #[test]
    fn a_species_that_already_has_a_threshold_says_so() {
        // Otherwise the row reads as new advice when it may be advice the
        // operator already took, and pressing Apply looks like a no-op.
        let html = render_thresholds_partial(
            &[],
            &[suggestion("Turdus merula", 0.85, 8, 0, 6, 0, Some(0.90))],
        );
        assert!(html.contains("now 90%"), "{html}");

        let html = render_thresholds_partial(&[], &[strong()]);
        assert!(html.contains("none set"), "{html}");
    }

    #[test]
    fn no_suggestions_renders_no_suggestion_block_at_all() {
        let html = render_thresholds_partial(&[], &[]);
        assert!(!html.contains("Suggested from your reviews"), "{html}");
        // ...and the rest of the page is still there.
        assert!(html.contains("Per-Species Confidence Thresholds"), "{html}");
    }

    #[test]
    fn a_species_name_is_escaped_in_both_the_cell_and_the_form() {
        // The name reaches here from the model's label file, and lands in a
        // table cell *and* an attribute value.
        let html = render_thresholds_partial(
            &[],
            &[suggestion(r#"Evil" onload="x"#, 0.85, 8, 0, 6, 0, None)],
        );
        assert!(
            !html.contains(r#"onload="x"#),
            "unescaped name reached the page: {html}"
        );
    }
}
