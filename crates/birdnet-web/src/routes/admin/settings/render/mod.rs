//! HTML rendering for the admin settings page.
//!
//! Split into section sub-modules for maintainability:
//!
//! | Module          | Responsibility                                 |
//! |-----------------|------------------------------------------------|
//! | `audio`         | Audio capture section (ALSA, RTSP, format)     |
//! | `location`      | Location & recording schedule section           |
//! | `detection`     | Detection thresholds section                   |
//! | `notifications` | Apprise + BirdWeather notifications section     |
//! | `species`       | Species filter lists section                   |
//! | `system`        | System, display, auth section                  |
//! | `email`         | SMTP email alerts section                      |

mod audio;
mod detection;
mod email;
mod location;
mod notifications;
mod species;
mod system;

use std::collections::HashMap;

pub(in crate::routes::admin::settings) fn get_setting<'a>(
    map: &'a HashMap<String, String>,
    key: &str,
    default: &'a str,
) -> &'a str {
    map.get(key).map_or(default, String::as_str)
}

pub(super) fn render_settings_page(settings: &HashMap<String, String>) -> String {
    // The document chrome (theme guard, app.css, htmx, the admin nav,
    // breadcrumbs, ⌘K/help/toasts) comes from the shared `admin_shell`; this
    // page contributes only its scoped <style> + content.
    crate::routes::admin::admin_shell("Settings", "settings", &settings_body(settings))
}

/// The page-specific body: a scoped `<style>` block plus the settings form.
///
/// Kept separate from [`render_settings_page`] so the inline-style guard can
/// assert on just the settings-owned markup (the shared shell's partials use
/// `data-*-style` attributes that a naive `style="` substring check would
/// otherwise trip over). The old `.container` / bare `nav` rules are dropped —
/// the shell owns layout and the nav now.
fn settings_body(settings: &HashMap<String, String>) -> String {
    let form_html = render_settings_form(settings);
    format!(
        r#"{SETTINGS_FORM_CSS}

  <h1>
    Admin Settings
  </h1>

  <div id="settings-feedback"></div>

  {form_html}"#
    )
}

/// The scoped stylesheet shared by the standalone settings page and the folded
/// settings sections on the Station tabs.
///
/// Defining it once keeps the folded sections pixel-identical to the standalone
/// page and is the single home for the legacy `.card`/`.grid-2`/`.hint` form
/// classes (a `<style>` block carries no CSP nonce burden — the inline-style
/// guard only forbids `style="` attributes).
pub(crate) const SETTINGS_FORM_CSS: &str = r"<style>
      .card { background: var(--surface); border: 1px solid var(--border); border-radius: 0.75rem;
               padding: 1.5rem; margin-bottom: 1.5rem; }
      .section-title { font-size: 1.1rem; font-weight: 600; color: var(--moss-ink);
                        margin-bottom: 1rem; border-bottom: 1px solid var(--border); padding-bottom: 0.5rem; }
      label { display: block; font-size: 0.85rem; color: var(--fg-3); margin-bottom: 0.25rem; }
      input, textarea, select { width: 100%; background: var(--bg); border: 1px solid var(--border);
                                  border-radius: 0.375rem; padding: 0.5rem 0.75rem; color: var(--fg);
                                  font-size: 0.875rem; box-sizing: border-box; margin-bottom: 1rem; }
      input:focus, textarea:focus { outline: none; border-color: var(--moss-ink); }
      .grid-2 { display: grid; grid-template-columns: 1fr 1fr; gap: 1rem; }
      .btn { padding: 0.5rem 1.5rem; border-radius: 0.375rem; border: none; cursor: pointer;
               font-weight: 600; font-size: 0.875rem; }
      .btn-primary { background: var(--moss); color: #fff; }
      .btn-primary:hover { background: var(--moss-ink); }
      .alert-success { background: var(--moss-soft); border: 1px solid var(--moss-soft); color: var(--moss-ink);
                         border-radius: 0.375rem; padding: 0.75rem 1rem; margin-bottom: 1rem; }
      .alert-error { background: var(--rare-soft); border: 1px solid var(--rare-soft); color: var(--rare);
                       border-radius: 0.375rem; padding: 0.75rem 1rem; margin-bottom: 1rem; }
      .alert-icon { vertical-align: -2px; margin-right: 0.4rem; }
      .hint { font-size: 0.75rem; color: var(--fg-4); margin-top: -0.75rem; margin-bottom: 1rem; }
      @media (max-width: 520px) { .grid-2 { grid-template-columns: 1fr; } }
      h1 { font-size: 1.5rem; font-weight: 700; margin-bottom: 1.5rem; color: var(--fg); }
      a.btn { text-decoration: none; }
      .btn.btn-sm { font-size: 0.8rem; padding: 0.3rem 0.8rem; }
      .hint a { color: var(--moss-ink); }
      .hint.flush { margin: -6px 0 8px; }
      .mt-sm { margin-top: 0.5rem; }
      .mt-md { margin-top: 1rem; }
      .save-row { display: flex; align-items: center; gap: 1rem; }
      .save-note { color: var(--fg-3); font-size: 0.875rem; }
      .save-note.dim { color: var(--fg-4); font-size: 0.8rem; }
    </style>";

/// One settings section, for rendering a task-scoped subset on a Station tab.
#[derive(Clone, Copy)]
pub(crate) enum Section {
    /// Audio capture (ALSA / RTSP / format).
    Audio,
    /// Location & recording schedule.
    Location,
    /// Detection thresholds (the single canonical home is Capture).
    Detection,
    /// Apprise + BirdWeather notifications.
    Notifications,
    /// SMTP email alerts.
    Email,
    /// System, display, retention, auth.
    System,
}

/// Render the chosen settings `sections` wrapped in one form that posts to
/// `/admin/settings`, for a Station management tab.
///
/// Safe because `save_settings` diffs submitted-vs-existing and persists only
/// changed keys (a MERGE — absent fields are skipped), so a tab can carry just
/// its own slice of the settings without clobbering the rest. The caller is
/// responsible for emitting [`SETTINGS_FORM_CSS`] once on the page.
pub(crate) fn render_section_form(
    settings: &HashMap<String, String>,
    sections: &[Section],
) -> String {
    let mut out = String::with_capacity(8_192);
    out.push_str(
        r##"<div id="settings-feedback"></div><form hx-post="/admin/settings" hx-target="#settings-feedback" hx-swap="innerHTML" hx-indicator="#save-spinner">"##,
    );
    for section in sections {
        match section {
            Section::Audio => audio::render(&mut out, settings),
            Section::Location => location::render(&mut out, settings),
            Section::Detection => detection::render(&mut out, settings),
            Section::Notifications => notifications::render(&mut out, settings),
            Section::Email => email::render(&mut out, settings),
            Section::System => system::render(&mut out, settings),
        }
    }
    out.push_str(
        r#"<div class="save-row">
    <button type="submit" class="btn btn-primary">Save settings</button>
    <span id="save-spinner" class="htmx-indicator save-note">Saving…</span>
    <span class="save-note dim">Most settings require a restart to take effect.</span>
  </div>
</form>"#,
    );
    out
}

pub(super) fn render_settings_form(settings: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(16_384);
    out.push_str(
        r##"<form hx-post="/admin/settings" hx-target="#settings-feedback"
               hx-swap="innerHTML" hx-indicator="#save-spinner">"##,
    );
    audio::render(&mut out, settings);
    location::render(&mut out, settings);
    detection::render(&mut out, settings);
    notifications::render(&mut out, settings);
    species::render(&mut out, settings);
    system::render(&mut out, settings);
    email::render(&mut out, settings);
    out.push_str(
        r#"
  <div class="save-row">
    <button type="submit" class="btn btn-primary">Save Settings</button>
    <span id="save-spinner" class="htmx-indicator save-note">
      Saving…
    </span>
    <span class="save-note dim">
      Most settings require a restart to take effect.
    </span>
  </div>
</form>"#,
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_setting_default() {
        let map = HashMap::new();
        assert_eq!(get_setting(&map, "missing", "fallback"), "fallback");
    }

    #[test]
    fn get_setting_present() {
        let map = HashMap::from([("key".to_string(), "val".to_string())]);
        assert_eq!(get_setting(&map, "key", "default"), "val");
    }

    #[test]
    fn render_settings_form_contains_fields() {
        let settings = HashMap::new();
        let html = render_settings_form(&settings);
        // Audio
        assert!(html.contains("alsa_device"));
        assert!(html.contains("audio_format"));
        assert!(html.contains("audio_channels"));
        assert!(html.contains("rtsp_urls"));
        // Location
        assert!(html.contains("latitude"));
        assert!(html.contains("night_inhibit"));
        // Detection
        assert!(html.contains("confidence_threshold"));
        assert!(html.contains("sf_thresh"));
        assert!(html.contains("privacy_threshold"));
        // Notifications
        assert!(html.contains("apprise_url"));
        assert!(html.contains("apprise_config"));
        assert!(html.contains("notify_trigger"));
        assert!(html.contains("notify_species_only"));
        assert!(html.contains("notify_species_exclude"));
        assert!(html.contains("notify_title_template"));
        assert!(html.contains("notify_body_template"));
        assert!(html.contains("weekly_report_schedule"));
        assert!(html.contains("birdweather_token"));
        // System
        assert!(html.contains("max_files_per_species"));
        assert!(html.contains("purge_threshold"));
        assert!(html.contains("custom_image_dir"));
        assert!(html.contains("site_name"));
        assert!(html.contains("info_site"));
        assert!(html.contains("auth_username"));
        assert!(html.contains("auth_password"));
        // Email
        assert!(html.contains("email_smtp_host"));
        assert!(html.contains("email_to"));
    }

    #[test]
    fn settings_page_has_no_inline_style_attributes() {
        // P3-3 (O-25): the settings render modules must not emit inline style
        // attributes — those can't carry a CSP nonce, so they are
        // the blocker for dropping `style-src 'unsafe-inline'`. Reusable
        // layout/width shapes live in app.css utility classes; page-specific
        // styling lives in this page's own <style> block. This guard fails if a
        // new field silently ships an un-migrated inline style.
        let settings = HashMap::new();
        // Check the page-specific body (scoped <style> block + form), not the
        // full `render_settings_page`: the shared admin shell injects partials
        // that use `data-*-style` attributes, which a naive `style="` substring
        // check would flag even though they are not inline style attributes.
        assert!(
            !settings_body(&settings).contains("style=\""),
            "settings page body still emits an inline style attribute"
        );
        assert!(
            !render_settings_form(&settings).contains("style=\""),
            "settings form fragment still emits an inline style attribute"
        );
    }

    #[test]
    fn settings_page_renders_through_admin_shell() {
        // After the E consolidation the page renders via the shared shell: it
        // must carry the shell's admin nav (with Settings active) and the page
        // content, not a bespoke per-page nav.
        let settings = HashMap::new();
        let page = render_settings_page(&settings);
        assert!(page.contains("admin-nav"), "missing shared admin nav");
        assert!(
            page.contains(r#"href="/admin/settings" class="am-nav-active""#),
            "Settings tab should be active in the shell nav"
        );
        assert!(page.contains("Admin Settings"), "missing page heading");
        // The shell wraps the body in `.admin-wrap` and emits a breadcrumb for
        // non-overview pages — proof we render through it, not a bespoke page.
        assert!(page.contains("admin-wrap"), "missing shared shell wrapper");
        assert!(page.contains("bnb-crumbs"), "missing shell breadcrumb");
    }
}
