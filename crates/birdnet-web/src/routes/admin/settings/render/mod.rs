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
mod badges;
mod detection;
mod email;
mod location;
mod notifications;
mod species;
mod system;

use std::collections::HashMap;
use std::fmt::Write as _;

use birdnet_core::i18n::NameLocalizer;

pub(crate) use badges::PENDING_KEY;

/// The pseudo-setting the page loader adds when the station has an active
/// audio-source row: the rows, one per line (`kind<TAB>name<TAB>device`). Never
/// stored. See `handler::audio_sources_summary`.
pub(crate) const AUDIO_SOURCES_KEY: &str = "__audio_sources";
pub(crate) use system::language_options;

/// Every settings section on the full page, in the order it is rendered:
/// `(anchor id, heading text)`.
///
/// This is the single source of truth for the on-page index. The section
/// renderers each open with `id="set-{id}"` and an `<h2 id="set-{id}-h">`
/// carrying the same title, and `section_index` builds the jump list from this
/// array — so an index entry that points at nothing, or a section the index
/// never mentions, is not representable without the two disagreeing.
/// `settings_index_matches_rendered_sections` asserts both directions against
/// the real rendered HTML rather than trusting that.
pub(crate) const SECTIONS: &[(&str, &str)] = &[
    ("audio", "Audio Capture"),
    ("location", "Location &amp; Recording Schedule"),
    ("detection", "Detection Settings"),
    ("filters", "Detection filters"),
    ("notifications", "Notifications"),
    ("alert-limits", "Alert limits"),
    ("mqtt", "Home Assistant &amp; MQTT"),
    ("species", "Species Filters"),
    ("system", "System &amp; Display"),
    ("auth", "Web Authentication"),
    ("storage", "Storage"),
    ("online", "Online services"),
    ("backups", "Backups"),
    ("email", "Email Alerts"),
    // Rendered after the form, not inside it: its buttons each save at once,
    // and inside the form they would submit it.
    ("today-layout", "Today Page Layout"),
];

/// The sticky "On this page" jump list.
///
/// The page carries 54 controls over roughly eleven screens. Splitting it into
/// tabs would duplicate the Station screens, which already own task-scoped
/// access to these same sections; collapsing the sections would break the one
/// thing this page is uniquely for, which is having every setting present at
/// once and findable with the browser's own search. What it lacked was
/// orientation, so that is what this adds — and nothing is hidden to get it.
fn section_index() -> String {
    let mut out = String::with_capacity(1_024);
    out.push_str(
        r#"<nav class="set-index" aria-labelledby="set-index-h">
    <h2 class="set-index__h" id="set-index-h">On this page</h2>
    <div class="set-filter" hidden>
      <label class="sr-only" for="set-filter">Filter settings</label>
      <input type="search" id="set-filter" placeholder="Filter settings…" autocomplete="off" spellcheck="false">
      <p class="set-filter__count" role="status" aria-live="polite"></p>
    </div>
    <ol class="set-index__list">"#,
    );
    for &(id, title) in SECTIONS {
        // `r##"…"##`: the href contains `"#`, which would close an `r#"…"#`.
        let _ = write!(out, r##"<li><a href="#set-{id}">{title}</a></li>"##);
    }
    out.push_str(
        "</ol>
  </nav>",
    );
    out
}

/// A stored setting, **HTML-escaped**, ready to drop into `value="…"` or a
/// `<textarea>`.
///
/// Every caller interpolates the result into markup, and none escaped it: a
/// site name containing `"` broke out of its attribute (a stored-XSS path from
/// anyone holding the bearer API token, which may `PUT /api/v2/settings`), and
/// an SMTP password containing `&lt;` was shown — and on the next save written
/// back — as `<`. Escaping here, once, means a new field cannot forget.
pub(in crate::routes::admin::settings) fn get_setting(
    map: &HashMap<String, String>,
    key: &str,
) -> String {
    map.get(key).map_or_else(
        || crate::routes::pages::escape_html(&form_default(key)),
        |stored| crate::routes::pages::escape_html(stored),
    )
}

/// What a field shows when the settings table has no row for it — the one
/// place that says so, read by the renderer above and by the save's diff.
///
/// The save used to compare each submitted field with its row, or with `""`
/// when there was none. Every field showing a default therefore counted as
/// changed: on a fresh install the first press of Save, with nothing edited,
/// wrote 32 rows, reported "32 values updated", audited 32 keys as changed,
/// and pinned each default in the table, where it outranks a later
/// `birdnet.conf` edit of the same key. Comparing with the value the form
/// actually showed writes only what the operator changed.
pub fn form_default(key: &str) -> String {
    // The detection defaults mirror the daemon's shared constants, so the form
    // never advertises a value the station does not apply. `{:.2}` keeps the
    // familiar two-decimal form (0.70) from the shared 0.7 constant.
    match key {
        "confidence_threshold" => {
            format!("{:.2}", birdnet_core::config::DEFAULT_CONFIDENCE_THRESHOLD)
        }
        "sensitivity" => format!("{:.2}", birdnet_core::config::DEFAULT_SENSITIVITY),
        "privacy_rank" => birdnet_core::config::DEFAULT_PRIVACY_RANK.to_string(),
        other => fixed_form_default(other).to_owned(),
    }
}

/// The literal defaults; every key not named here shows as empty.
fn fixed_form_default(key: &str) -> &'static str {
    match key {
        "audio_format" => "wav",
        "clip_retention_days" | "freq_shift_hz" | "max_files_per_species" => "0",
        // The offsets: `--twilight-offset`'s default, which each end falls
        // back to. rare_species_days: the reader's own default.
        "post_sunset_offset" | "pre_sunrise_offset" | "rare_species_days" => "30",
        "confirmation_level" => "off",
        "database_lang" => "en",
        "deadman_hours" => "24",
        "email_cooldown_secs" | "notify_cooldown" => "300",
        "email_from_name" => "BirdNet-Behavior",
        "email_min_confidence" | "notify_confidence" => "0.80",
        "email_smtp_port" => "587",
        "email_starttls" => "true",
        "extraction_length" => "6",
        "info_site" => "ebird",
        "night_inhibit" | "raw_spectrogram" => "false",
        "notify_trigger" => "each",
        "overlap" | "privacy_threshold" => "0.0",
        "full_disk_action" => "purge",
        "purge_threshold" => "95",
        "recording_schedule" => "all-day",
        "segment_duration" => "15",
        "sf_thresh" => "0.03",
        "spectrogram_colormap" => "viridis",
        "spectrogram_range_db" => "80",
        "temperature_unit" => "celsius",
        "time_format" => "24h",
        "stream_max_mb" => "512",
        "stream_retention_secs" => "600",
        "weekly_report_schedule" => "monday",
        _ => "",
    }
}

pub(super) fn render_settings_page(
    settings: &HashMap<String, String>,
    names: &NameLocalizer,
) -> String {
    // The document chrome (theme guard, app.css, htmx, the admin nav,
    // breadcrumbs, ⌘K/help/toasts) comes from the shared `admin_shell`; this
    // page contributes only its scoped <style> + content.
    crate::routes::admin::admin_shell("Settings", "settings", &settings_body(settings, names))
}

static SETTINGS_FILTER_SCRIPT: crate::seal::Sealed =
    crate::seal::Sealed::new(r#"<script src="/static/settings-filter.js" defer></script>"#);

/// The page-specific body: a scoped `<style>` block plus the settings form.
///
/// Kept separate from [`render_settings_page`] so the inline-style guard can
/// assert on just the settings-owned markup (the shared shell's partials use
/// `data-*-style` attributes that a naive `style="` substring check would
/// otherwise trip over). The old `.container` / bare `nav` rules are dropped —
/// the shell owns layout and the nav now.
fn settings_body(settings: &HashMap<String, String>, names: &NameLocalizer) -> String {
    let form_html = render_settings_form(settings, names);
    let layout = crate::routes::admin::today_layout::render_section(
        &crate::routes::pages::today_layout::Layout::parse(
            settings
                .get(crate::routes::pages::today_layout::SETTING_KEY)
                .map_or("", String::as_str),
        ),
    );
    let index = section_index();
    format!(
        r#"{SETTINGS_FORM_CSS}
<div class="set-scope">
  <h1>
    Admin Settings
  </h1>

  <div class="set-layout">
  {index}
  <div class="set-main">
  <div id="settings-feedback"></div>

  {form_html}
  {layout}
  </div>
  </div>
</div>
  {SETTINGS_FILTER_SCRIPT}"#
    )
}

/// The scoped stylesheet shared by the standalone settings page and the folded
/// settings sections on the Station tabs.
///
/// Defining it once keeps the folded sections pixel-identical to the standalone
/// page and is the single home for the legacy `.card`/`.grid-2`/`.hint` form
/// classes (a `<style>` block carries no CSP nonce burden — the inline-style
/// guard only forbids `style="` attributes).
pub(crate) static SETTINGS_FORM_CSS: crate::seal::Sealed = crate::seal::Sealed::new(
    r"<style>
      .card { background: var(--surface); border: 1px solid var(--border); border-radius: 0.75rem;
               padding: 1.5rem; margin-bottom: 1.5rem; }
      /* `.card h2` in app.css is the card *eyebrow*: 11px, uppercase, muted.
         These section titles are real headings and were `<div>`s until they
         became `<h2>`s, at which point that descendant rule (0,1,1) started
         beating a bare `.section-title` class (0,1,0) and shrank every section
         title below its own field labels. Matching its specificity and
         resetting the properties it sets is deliberate, not incidental. */
      .card h2.section-title { font-size: 1.1rem; font-weight: 600; color: var(--moss-ink);
                        margin: 0 0 1rem; border-bottom: 1px solid var(--border); padding-bottom: 0.5rem;
                        scroll-margin-top: 1rem; text-transform: none; letter-spacing: -0.015em;
                        display: block; }
      /* Two panes: a sticky index beside the sections. Below the breakpoint the
         index becomes an ordinary block above the form, so a phone gets the
         same jump list without losing width to it. */
      .set-layout { display: grid; grid-template-columns: minmax(0, 1fr); gap: 1.25rem; align-items: start; }
      .set-main { min-width: 0; }
      .set-index { background: var(--surface); border: 1px solid var(--border);
                    border-radius: 0.75rem; padding: 1rem 1.15rem; }
      .set-index__h { font-size: 0.7rem; letter-spacing: 0.08em; text-transform: uppercase;
                       color: var(--fg-3); margin: 0 0 0.6rem; font-weight: 600;
                       border: 0; padding: 0; }
      .set-index__list { list-style: none; margin: 0; padding: 0; display: flex;
                          flex-wrap: wrap; gap: 0.35rem 0.9rem; }
      .set-index__list a { color: var(--fg-2); text-decoration: none; font-size: 0.85rem;
                            display: block; padding: 0.15rem 0; border-radius: 0.25rem; }
      .set-index__list a:hover { color: var(--moss-ink); text-decoration: underline; }
      .set-index__list a:focus-visible { outline: 2px solid var(--moss); outline-offset: 2px; }
      .set-index__list a.is-filtered-out { opacity: 0.38; text-decoration: line-through; }
      .set-filter input { margin-bottom: 0.5rem; font-size: 0.85rem; padding: 0.35rem 0.55rem; }
      .set-filter__count { font-size: 0.72rem; color: var(--fg-3); margin: 0 0 0.6rem; min-height: 1em; }
      @media (min-width: 900px) {
        .set-layout { grid-template-columns: 13.5rem minmax(0, 1fr); gap: 1.75rem; }
        .set-index { position: sticky; top: 1rem; max-height: calc(100vh - 2rem); overflow-y: auto; }
        .set-index__list { display: block; }
        .set-index__list li + li { margin-top: 0.15rem; }
      }
      /* Element rules are scoped with `:where()`, which adds no specificity:
         unscoped, they restyled every label and input on a Station tab — the
         audio-source edit form's checkboxes each took a full-width row of
         their own — and a plain `.set-scope` prefix would have raised them
         above the class rules they lose to here. */
      :where(.set-scope) label { display: block; font-size: 0.85rem; color: var(--fg-3); margin-bottom: 0.25rem; }
      :where(.set-scope) input, :where(.set-scope) textarea, :where(.set-scope) select { width: 100%; background: var(--bg); border: 1px solid var(--border);
                                  border-radius: 0.375rem; padding: 0.5rem 0.75rem; color: var(--fg);
                                  font-size: 0.875rem; box-sizing: border-box; margin-bottom: 1rem; }
      :where(.set-scope) input:focus, :where(.set-scope) textarea:focus { outline: none; border-color: var(--moss-ink); }
      .grid-2 { display: grid; grid-template-columns: 1fr 1fr; gap: 1rem; }
      .btn { padding: 0.5rem 1.5rem; border-radius: 0.375rem; border: none; cursor: pointer;
               font-weight: 600; font-size: 0.875rem; }
      .btn-primary { background: var(--moss); color: var(--on-moss); }
      /* Today page layout editor. */
      .tl-list { list-style: none; margin: 0; padding: 0; }
      .tl-row { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between;
                gap: 0.5rem 1rem; padding: 0.6rem 0; border-top: 1px solid var(--border); }
      .tl-row:first-child { border-top: 0; }
      .tl-name { display: flex; flex-direction: column; gap: 0.1rem; min-width: 0; }
      .tl-where { font-size: 0.78rem; }
      .tl-row--hidden .tl-name > :first-child, .tl-row--hidden .tl-name { color: var(--fg-3); }
      .tl-actions { display: flex; flex-wrap: wrap; gap: 0.35rem; }
      /* A move that would do nothing (the edge of a group) is disabled and must
         look it. WCAG 1.4.3 exempts inactive controls from the contrast floor. */
      .tl-actions button:disabled { opacity: 0.4; cursor: not-allowed; }
      .btn-primary:hover { background: var(--moss-ink); }
      .alert-success { background: var(--moss-soft); border: 1px solid var(--moss-soft); color: var(--moss-ink);
                         border-radius: 0.375rem; padding: 0.75rem 1rem; margin-bottom: 1rem; }
      .alert-warn { background: var(--dawn-soft); border: 1px solid var(--dawn-soft); color: var(--dawn-ink);
                       border-radius: 0.375rem; padding: 0.75rem 1rem; margin-bottom: 1rem; }
      .save-picture-note { margin: 0.4rem 0 0; }
      .alert-error { background: var(--rare-soft); border: 1px solid var(--rare-soft); color: var(--rare);
                       border-radius: 0.375rem; padding: 0.75rem 1rem; margin-bottom: 1rem; }
      .alert-icon { vertical-align: -2px; margin-right: 0.4rem; }
      .save-problems { margin: 0.4rem 0 0; padding-left: 1.1rem; }
      .save-problems li { margin-top: 0.25rem; }
      .hint { font-size: 0.75rem; color: var(--fg-4); margin-top: -0.75rem; margin-bottom: 1rem; }
      @media (max-width: 520px) { .grid-2 { grid-template-columns: 1fr; } }
      :where(.set-scope) h1 { font-size: 1.5rem; font-weight: 700; margin-bottom: 1.5rem; color: var(--fg); }
      a.btn { text-decoration: none; }
      .btn.btn-sm { font-size: 0.8rem; padding: 0.3rem 0.8rem; }
      /* Underlined, not just tinted: axe's link-in-text-block flags a link
         inside a paragraph that is distinguishable from the surrounding
         text by colour alone, which is WCAG 1.4.1 — a reader who cannot
         separate the two hues sees no link at all. */
      /* The underline moved to the Prose links block in app.css: scoped here
         it applied on /admin/settings and not on the Station tabs that render
         the same `.hint` markup through the main shell. */
      .hint a { color: var(--moss-ink); }
      .hint.flush { margin: -6px 0 8px; }
      /* A hint with no field above it has nothing to tuck under. */
      .hint.standalone { margin-top: 0; }
      /* A checkbox and its words on one row, like the field above it
         in rhythm: the element rules above give an input the full width. */
      .bnb-check { display: flex; align-items: center; gap: 0.5rem; color: var(--fg); margin-bottom: 1rem; }
      .bnb-check input { width: auto; margin: 0; }
      .mt-sm { margin-top: 0.5rem; }
      .mt-md { margin-top: 1rem; }
      .save-row { display: flex; align-items: center; gap: 1rem; }
      .save-note { color: var(--fg-3); font-size: 0.875rem; }
      .save-note.dim { color: var(--fg-4); font-size: 0.8rem; }
      /* The audio-source list shown in place of the capture addresses. */
      .set-sources { border: 1px solid var(--border); border-radius: 0.5rem; background: var(--bg);
                     padding: 0.75rem 1rem; margin-bottom: 1rem; }
      .set-sources p { margin: 0 0 0.5rem; font-size: 0.875rem; color: var(--fg-2); }
      .set-sources p.hint { margin-top: 0.5rem; margin-bottom: 0; }
      .set-sources__list { margin: 0 0 0.25rem; padding-left: 1.2rem; font-size: 0.875rem; }
      .set-sources__list li { margin-bottom: 0.25rem; overflow-wrap: anywhere; }
    </style>",
);

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
    /// One of the table-driven groups ([`super::extra`]).
    Extra(super::extra::Group),
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
    names: &NameLocalizer,
) -> String {
    let mut out = String::with_capacity(8_192);
    out.push_str(
        r##"<div class="set-scope"><div id="settings-feedback"></div><form hx-post="/admin/settings" hx-target="#settings-feedback" hx-swap="innerHTML" hx-indicator="#save-spinner">"##,
    );
    for section in sections {
        match section {
            Section::Audio => audio::render(&mut out, settings),
            Section::Location => location::render(&mut out, settings),
            Section::Detection => detection::render(&mut out, settings),
            Section::Notifications => notifications::render(&mut out, settings),
            Section::Email => email::render(&mut out, settings),
            Section::System => system::render(&mut out, settings, names),
            Section::Extra(group) => super::extra::render_group(&mut out, *group, settings),
        }
    }
    out.push_str(
        r#"<div class="save-row">
    <button type="submit" class="btn btn-primary">Save settings</button>
    <span id="save-spinner" class="htmx-indicator save-note">Saving…</span>
    <span class="save-note dim">Each setting says when it applies. The ones marked <em>after restart</em> wait until you restart the station.</span>
  </div>
</form></div>"#,
    );
    finish(&out, settings)
}

/// The finished markup: every field badged with when it applies, and the
/// banner for saved changes still waiting for a restart in front.
fn finish(out: &str, settings: &HashMap<String, String>) -> String {
    let badged = badges::add_applies_badges(out);
    let banner = badges::pending_banner(settings, &badged);
    let script = crate::routes::admin::classifier::CONTROLS_SCRIPT.as_str();
    format!("{banner}{badged}{script}")
}

pub(super) fn render_settings_form(
    settings: &HashMap<String, String>,
    names: &NameLocalizer,
) -> String {
    use super::extra::Group;
    let mut out = String::with_capacity(16_384);
    out.push_str(
        r##"<form hx-post="/admin/settings" hx-target="#settings-feedback"
               hx-swap="innerHTML" hx-indicator="#save-spinner">"##,
    );
    audio::render(&mut out, settings);
    location::render(&mut out, settings);
    detection::render(&mut out, settings);
    super::extra::render_group(&mut out, Group::Filters, settings);
    notifications::render(&mut out, settings);
    super::extra::render_group(&mut out, Group::AlertLimits, settings);
    super::extra::render_group(&mut out, Group::Mqtt, settings);
    species::render(&mut out, settings);
    system::render(&mut out, settings, names);
    super::extra::render_group(&mut out, Group::Storage, settings);
    super::extra::render_group(&mut out, Group::Online, settings);
    super::extra::render_group(&mut out, Group::Backups, settings);
    email::render(&mut out, settings);
    out.push_str(
        r#"
  <div class="save-row">
    <button type="submit" class="btn btn-primary">Save Settings</button>
    <span id="save-spinner" class="htmx-indicator save-note">
      Saving…
    </span>
    <span class="save-note dim">
      Each setting says when it applies. The ones marked <em>after restart</em> wait until you restart the station.
    </span>
  </div>
</form>"#,
    );
    finish(&out, settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The jump list and the sections it points at cannot drift apart.
    ///
    /// Checked in both directions and by count, because a one-directional
    /// check passes while the index quietly points at an eighth section that
    /// no longer exists, or misses a ninth that does.
    #[test]
    fn settings_index_matches_rendered_sections() {
        let html = settings_body(&HashMap::new(), &NameLocalizer::english());
        for (id, title) in SECTIONS {
            assert!(
                html.contains(&format!(r##"href="#set-{id}""##)),
                "index has no link to {id}"
            );
            assert!(
                html.contains(&format!(r#"<section class="card" id="set-{id}""#)),
                "no section is anchored at {id}"
            );
            assert!(
                html.contains(&format!(r#"id="set-{id}-h">{title}</h2>"#)),
                "{id} has no <h2> reading {title:?}"
            );
        }
        assert_eq!(
            html.matches(r#"<section class="card""#).count(),
            SECTIONS.len(),
            "a section exists that the index does not list"
        );
        assert_eq!(
            html.matches(r##"href="#set-"##).count(),
            SECTIONS.len(),
            "the index lists an anchor no section provides"
        );
    }

    /// Section titles are headings, not text that merely looks like one.
    ///
    /// All eight were `<div class="section-title">`: styled at 1.1rem, semibold
    /// and underlined, so they read as headings to a sighted user and as
    /// nothing at all to a screen reader or to any "jump to next heading" key.
    /// The page's whole visible structure was invisible to the accessibility
    /// tree.
    #[test]
    fn section_titles_are_headings() {
        let html = settings_body(&HashMap::new(), &NameLocalizer::english());
        assert!(
            !html.contains(r#"<div class="section-title">"#),
            "a section title is still a div"
        );
        assert_eq!(
            html.matches(r#"<h2 class="section-title"#).count(),
            SECTIONS.len()
        );
    }

    /// The Station tabs share these renderers, so they get the same headings —
    /// but must not get the full page's index, which would point at seven
    /// sections the tab does not render.
    #[test]
    fn station_tab_sections_carry_headings_but_no_index() {
        let html = render_section_form(
            &HashMap::new(),
            &[Section::Audio, Section::Detection],
            &NameLocalizer::english(),
        );
        assert!(html.contains(r#"<h2 class="section-title" id="set-audio-h">"#));
        assert!(html.contains(r#"<h2 class="section-title" id="set-detection-h">"#));
        assert_eq!(html.matches(r#"<section class="card""#).count(), 2);
        assert!(
            !html.contains("set-index"),
            "a task tab must not carry the full-page index"
        );
        assert!(!html.contains(r##"href="#set-location""##));
        assert!(
            !html.contains("set-filter"),
            "a task tab must not carry the full-page filter"
        );
    }

    /// The filter must not become a dead control when scripting is off.
    ///
    /// It ships `hidden` and `settings-filter.js` reveals it, so a browser with
    /// JavaScript disabled — or one that failed to fetch the script — shows a
    /// page that behaves exactly as it did before the filter existed: every
    /// section expanded, and nothing to click that does nothing.
    #[test]
    fn filter_ships_hidden_and_is_revealed_by_script() {
        let html = settings_body(&HashMap::new(), &NameLocalizer::english());
        assert!(
            html.contains(r#"<div class="set-filter" hidden>"#),
            "the filter must ship hidden"
        );
        assert!(html.contains(r#"<input type="search" id="set-filter""#));
        assert!(
            html.contains(r#"<label class="sr-only" for="set-filter">"#),
            "the search box needs a real label, not just a placeholder"
        );
        assert!(html.contains(r#"src="/static/settings-filter.js""#));
        // Nothing is hidden server-side: the filter narrows what is on screen,
        // it is not a collapsed-by-default disclosure.
        assert!(!html.contains(r#"id="set-audio" hidden"#));
    }

    #[test]
    fn get_setting_default() {
        let map = HashMap::new();
        assert_eq!(get_setting(&map, "audio_format"), "wav");
        assert_eq!(get_setting(&map, "no_such_key"), "");
    }

    #[test]
    fn get_setting_present() {
        let map = HashMap::from([("audio_format".to_string(), "flac".to_string())]);
        assert_eq!(get_setting(&map, "audio_format"), "flac");
    }

    #[test]
    fn render_settings_form_contains_fields() {
        let settings = HashMap::new();
        let html = render_settings_form(&settings, &NameLocalizer::english());
        // Audio
        assert!(html.contains("alsa_device"));
        assert!(html.contains("audio_format"));
        assert!(html.contains("rtsp_urls"));
        // Location
        assert!(html.contains("latitude"));
        assert!(html.contains("night_inhibit"));
        // Detection
        assert!(html.contains("confidence_threshold"));
        assert!(html.contains("sf_thresh"));
        assert!(html.contains("privacy_threshold"));
        assert!(html.contains("privacy_rank"));
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
        // Email
        assert!(html.contains("email_smtp_host"));
        assert!(html.contains("email_to"));
    }

    #[test]
    fn settings_page_offers_no_credential_inputs() {
        // The Web Authentication section explains where the admin password
        // really lives; it must never again render an input for it. A password
        // field here stored plaintext in `settings`, echoed it back into this
        // HTML on every load, and changed no credential.
        let html = render_settings_form(&HashMap::new(), &NameLocalizer::english());
        assert!(
            !html.contains("name=\"auth_password\""),
            "settings form must not render an admin-password input"
        );
        assert!(
            !html.contains("name=\"auth_username\""),
            "settings form must not render an admin-username input"
        );
        // The SMTP password field stays: `email_smtp_pass` is read back out of
        // the settings table by `create_email_notifier`, so it is a credential
        // the form genuinely owns. The admin password never was.
        assert!(
            html.contains("name=\"email_smtp_pass\""),
            "the working SMTP credential field must not be removed with the inert one"
        );
        // The explanation replacing them still points at the real mechanism.
        assert!(html.contains("CADDY_PWD"));
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
            !settings_body(&settings, &NameLocalizer::english()).contains("style=\""),
            "settings page body still emits an inline style attribute"
        );
        assert!(
            !render_settings_form(&settings, &NameLocalizer::english()).contains("style=\""),
            "settings form fragment still emits an inline style attribute"
        );
    }

    #[test]
    fn settings_page_renders_through_admin_shell() {
        // After the E consolidation the page renders via the shared shell: it
        // must carry the shell's admin nav (with Settings active) and the page
        // content, not a bespoke per-page nav.
        let settings = HashMap::new();
        let page = render_settings_page(&settings, &NameLocalizer::english());
        assert!(page.contains("admin-nav"), "missing shared admin nav");
        assert!(
            page.contains(r#"href="/admin/settings" class="am-nav-active""#),
            "Settings tab should be active in the shell nav"
        );
        assert!(page.contains("Admin Settings"), "missing page heading");
        // The shell is the site's own layout — header, Station tab lit, tab bar —
        // with a breadcrumb for non-overview pages: proof we render through it,
        // not a bespoke page. (It used to be a separate `.admin-wrap` document
        // with none of the site chrome.)
        assert!(
            page.contains(r#"class="topnav""#),
            "missing the site header"
        );
        assert!(
            page.contains(r#"data-home="station""#),
            "an admin page is a page of the Station home"
        );
        assert!(page.contains("bnb-tabbar"), "missing the phone tab bar");
        assert!(page.contains("bnb-crumbs"), "missing shell breadcrumb");
        assert!(
            !page.contains("admin-wrap"),
            "the standalone wrapper is gone"
        );
    }
}
