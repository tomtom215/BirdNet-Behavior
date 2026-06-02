//! P3-3 / O-25 inline-style guard.
//!
//! Every page swept onto utility classes must stay swept: this test scans the
//! source of the already-migrated render modules and fails if a bare inline
//! `style="…"` attribute reappears, so a future edit can't silently regress the
//! work toward dropping `style-src 'unsafe-inline'` (see
//! `docs/RELEASE_PUNCHLIST.md` § P3-3).
//!
//! A small allowlist covers the *legitimately* dynamic styles that the sweep
//! deliberately leaves inline — values computed per-request that cannot be a
//! static class (progress/usage bar `width:`/`height:` percentages, the
//! `background:`/`fill:` of data-driven SVG/bars, the per-species `--sp:` avatar
//! custom property) — plus `data-confirm-style=`, which is a data-attribute, not
//! an inline style. Those move into nonce'd `<style>` blocks in the P3-3 endgame.

use std::path::{Path, PathBuf};

/// Source files fully swept of *static* inline styles. Adding a file here makes
/// its no-static-inline-style guarantee permanent.
const SWEPT_FILES: &[&str] = &[
    // ── admin (slices 1–5) ──
    "src/routes/admin/settings/render/mod.rs",
    "src/routes/admin/settings/render/audio.rs",
    "src/routes/admin/settings/render/detection.rs",
    "src/routes/admin/settings/render/email.rs",
    "src/routes/admin/settings/render/location.rs",
    "src/routes/admin/settings/render/notifications.rs",
    "src/routes/admin/settings/render/species.rs",
    "src/routes/admin/settings/render/system.rs",
    "src/routes/admin/notifications.rs",
    "src/routes/admin/notification_test.rs",
    "src/routes/admin/overview.rs",
    "src/routes/admin/logs.rs",
    "src/routes/admin/species/render.rs",
    "src/routes/admin/migration/render.rs",
    "src/routes/admin/system.rs",
    "src/routes/admin/backup_recovery.rs",
    "src/routes/admin/doctor.rs",
    "src/routes/admin/quality.rs",
    "src/routes/admin/accounts.rs",
    "templates/admin_accounts.html",
    // ── public pages (slices 6–7+) ──
    "src/routes/pages/year_in_review.rs",
    "src/routes/pages/weekly_report.rs",
    "src/routes/pages/history.rs",
    "src/routes/pages/life_list.rs",
    "src/routes/pages/recordings.rs",
    "src/routes/pages/dawn_chorus.rs",
    "src/routes/pages/quarantine.rs",
    // ── analytics screens (slice 9) ──
    "src/routes/pages/behavioral.rs",
    "src/routes/pages/correlation.rs",
    "src/routes/pages/timeseries_dash.rs",
    "src/routes/pages/species_pages.rs",
    "src/routes/pages/detection_detail.rs",
    // ── onboarding wizard (slice 10) ──
    "src/routes/pages/onboarding.rs",
    // ── served HTML templates (slice 11) ──
    "templates/today.html",
    "templates/dawn_chorus.html",
    "templates/species.html",
    "templates/species_detail.html",
    "templates/analytics.html",
    "templates/timeseries.html",
    // ── standalone admin & system pages (slice 12) ──
    "src/routes/pages/changelog.rs",
    "src/routes/pages/heatmap.rs",
    "src/routes/pages/notification_center.rs",
    "src/routes/pages/system_dashboard.rs",
    "src/routes/admin/rules.rs",
    "templates/login.html",
    "templates/migration.html",
    "templates/admin_audio_sources.html",
    // ── home (/) + today (/today) ecosystem (slice 13) ──
    "src/routes/pages/today.rs",
    "src/routes/pages/today_phrase.rs",
    "src/routes/pages/health.rs",
    "src/routes/pages/dashboard/stats.rs",
    "src/routes/pages/dashboard/partials.rs",
    "templates/dashboard.html",
    // ── escaped-quote files the hardened guard surfaced (slice 14) ──
    "src/routes/pages/detection_reviews.rs",
    "src/routes/admin/images.rs",
    // ── admin control surface + listen + cross-cutting layout/partials (slice 15) ──
    "src/routes/admin/system_controls/service.rs",
    "src/routes/admin/system_controls/update.rs",
    "src/routes/admin/system_controls/data.rs",
    "src/routes/admin/system_controls/backup.rs",
    "src/routes/admin/backup.rs",
    "src/routes/admin/audio.rs",
    "src/routes/admin/mod.rs",
    "templates/listen.html",
    "templates/layout.html",
    "templates/_partial_footer.html",
    "templates/_partial_tabbar.html",
    "templates/_partial_topnav_more.html",
    "templates/_partial_cmdk.html",
    "templates/_partial_update_banner.html",
    "templates/_partial_help_drawer.html",
    "templates/_partial_confirm_modal.html",
    "templates/_partial_audio_source_row.html",
    "templates/_partial_display_prefs.html",
];

/// Returns true if the inline-style payload (the text inside `style="…"`) is a
/// documented dynamic exception rather than a static shape that belongs in CSS.
fn is_allowed_dynamic(payload: &str) -> bool {
    // Computed bar/meter geometry — width/height driven by a runtime percentage
    // or interpolated into the string (`{…}`), and the data-driven fill/background
    // that travels with it.
    let dynamic_prefixes = ["width:", "height:", "background:", "fill:", "--sp:"];
    if dynamic_prefixes.iter().any(|p| payload.starts_with(p)) {
        return true;
    }
    // SVG <text> presentation styles (font-size + fill) on computed-position
    // labels in chart helpers — inline on the SVG element, not page layout.
    if payload.contains("fill:") && payload.contains("font-size:") {
        return true;
    }
    false
}

/// Scan one file, returning each disallowed inline style occurrence as
/// `(line_number, payload)`.
///
/// Catches both the literal `style="…"` (templates and raw-string Rust) **and**
/// the escaped `style=\"…\"` that ordinary Rust `write!`/`format!` string
/// literals emit — the rendered HTML carries an inline style either way, so the
/// escaped form (invisible to a plain `style="` search) must be guarded too.
fn disallowed_inline_styles(src: &str) -> Vec<(usize, String)> {
    let mut hits = Vec::new();
    for (lineno, line) in src.lines().enumerate() {
        scan_inline_styles(line, "style=\"", "\"", lineno, &mut hits);
        scan_inline_styles(line, "style=\\\"", "\\\"", lineno, &mut hits);
    }
    hits
}

/// Record every disallowed `{open}…{close}` inline-style run on one line.
fn scan_inline_styles(
    line: &str,
    open: &str,
    close: &str,
    lineno: usize,
    hits: &mut Vec<(usize, String)>,
) {
    let mut rest = line;
    while let Some(idx) = rest.find(open) {
        // `data-confirm-style="…"` ends in `-style="`, so the char before the
        // match being part of an identifier means it's a data-attribute, not a
        // bare `style=`.
        let is_data_attr = rest[..idx]
            .chars()
            .last()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '-');
        let after = &rest[idx + open.len()..];
        let Some(end) = after.find(close) else { break };
        let payload = &after[..end];
        // A real inline style is `prop:value…` and always contains a colon;
        // Rust search-strings (`html.split("style=\"")`) and bare data-attribute
        // values (`data-confirm-style=\"danger\"`) have none, so the colon gate
        // skips those while still catching every genuine inline style.
        if !is_data_attr && payload.contains(':') && !is_allowed_dynamic(payload) {
            hits.push((lineno + 1, payload.to_string()));
        }
        rest = &after[end + close.len()..];
    }
}

fn crate_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

#[test]
fn swept_files_have_no_static_inline_styles() {
    let root = crate_root();
    let mut failures = Vec::new();

    for rel in SWEPT_FILES {
        let path = root.join(rel);
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read swept file {rel}: {e}"));
        for (lineno, payload) in disallowed_inline_styles(&src) {
            failures.push(format!("  {rel}:{lineno}  style=\"{payload}\""));
        }
    }

    assert!(
        failures.is_empty(),
        "P3-3 regression: static inline style attributes reappeared in swept files.\n\
         Move them to a utility/page class in app.css (or, if genuinely dynamic, \
         extend the allowlist in this test).\n{}",
        failures.join("\n")
    );
}

#[test]
fn guard_itself_classifies_correctly() {
    // Static shapes are caught.
    assert_eq!(
        disallowed_inline_styles(r#"<div style="display:flex;gap:1rem;">"#).len(),
        1
    );
    // Dynamic exceptions are allowed.
    assert!(disallowed_inline_styles(r#"<div style="width:{pct}%">"#).is_empty());
    assert!(
        disallowed_inline_styles(r#"<span style="background:{c};height:100%;width:{p}%;">"#)
            .is_empty()
    );
    assert!(
        disallowed_inline_styles(r#"<text style="font-size:11px;fill:var(--fg-3);">"#).is_empty()
    );
    assert!(disallowed_inline_styles(r#"<span style="--sp:{color};">"#).is_empty());
    // data-confirm-style is a data-attribute, not an inline style.
    assert!(disallowed_inline_styles(r#"<button data-confirm-style="danger">"#).is_empty());

    // Escaped-quote inline styles (Rust write!/format! literals) are caught too.
    assert_eq!(
        disallowed_inline_styles(r#"write!(h, "<div style=\"display:flex\">")"#).len(),
        1
    );
    assert!(disallowed_inline_styles(r#"format!("<i style=\"width:{p}%\">")"#).is_empty());
    // …but a Rust search-string for the literal attribute is not a style (no colon)
    assert!(disallowed_inline_styles(r#"assert!(html.split("style=\"").count() == 1)"#).is_empty());
    // …nor is an escaped data-attribute value.
    assert!(disallowed_inline_styles(r#"format!("<b data-confirm-style=\"danger\">")"#).is_empty());
}
