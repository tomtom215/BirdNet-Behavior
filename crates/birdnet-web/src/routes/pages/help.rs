//! Help / methodology cross-links into the embedded mdBook at `/help/...`.
//!
//! See O-20 DIFF.md.
//!
//! ## Surfaces
//!
//! - `Topic` (1:1 with `docs/book/<section>/<page>.md`) + `help_link()` /
//!   `help_drawer()` emit the small affordances shown next to analytical
//!   eyebrows and methodology-sensitive controls.
//! - [`router()`] mounts `/help/*` as a `ServeDir` over the mdBook output
//!   so the link targets resolve to real pages.
//!
//! ## mdBook integration
//!
//! The runtime looks for the rendered docs in the directory named by the
//! `BNB_HELP_DIR` env var, then falls back to `docs/book/_generated/html`
//! — the same path the workspace `build.rs` writes to via the
//! `[build-dependencies] mdbook` build step. When neither exists the
//! route returns 404 and the drawer's client script handles that
//! gracefully ("docs are unavailable on this device").
//!
//! Set `BNB_SKIP_DOCS=1` at build time to skip the mdBook render (e.g.
//! for air-gapped releases that ship a pre-rendered docs tree pointed at
//! by `BNB_HELP_DIR`). See the workspace `build.rs` docstring.
//!
//! ## O-20 per-template sweep status
//!
//! Call sites wired so far (the high-traffic + analytical surfaces):
//!
//! * Dashboard hero eyebrow (`pages::dashboard::dashboard_page`).
//! * Today detection-log eyebrow (`templates/today.html`).
//! * Dawn-chorus circadian eyebrow (`templates/dawn_chorus.html`).
//! * Migration phenology eyebrow (`templates/migration.html`).
//! * Activity heatmap top eyebrow (`pages::heatmap::HEATMAP_CONTENT`).
//! * Quarantine review header (`pages::quarantine`).
//! * Life-list "Journal" eyebrow (`pages::life_list`).
//! * Recordings "Browse" eyebrow (`templates/recordings.html`).
//! * Notifications "Operations" eyebrow (`pages::notification_center`).
//! * Weekly report "Backyard bulletin" eyebrow.
//! * Year-in-review masthead.
//! * Admin diagnostics → Troubleshooting (`admin::doctor::card_open`).
//! * Quality low-confidence species → Tuning (shipped in #91).
//!
//! Still TODO(O-20-followup) for the remaining DIFF rows (admin settings
//! sub-cards, admin audio post-O-13, admin remote-access, admin backups,
//! correlation matrix, sharing, etc.) — same one-line pattern per file
//! whenever the maintainer wants to extend.

use std::path::PathBuf;

use axum::Router;
use tower_http::services::ServeDir;

use crate::state::AppState;

/// Resolve the directory the `/help/*` route serves from.
///
/// Order: `BNB_HELP_DIR` env, then `docs/book/_generated/html`.
fn help_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("BNB_HELP_DIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir);
    }
    PathBuf::from("docs/book/_generated/html")
}

/// Mount the `/help/*` static-file route.
///
/// Returns a `Router<AppState>` so callers can `.merge` it like any other
/// page router. If the resolved directory doesn't exist, the route still
/// mounts cleanly — `ServeDir` returns 404 for every request, which the
/// drawer JS surfaces as the "docs unavailable" friendly message.
pub fn router() -> Router<AppState> {
    let dir = help_dir();
    Router::new().nest_service("/help", ServeDir::new(dir))
}

/// Stable identifier for a docs page. Maps 1:1 to `docs/book/<section>/<page>.md`.
///
/// Complete API surface: most variants don't have a call site in main yet
/// — the broader per-template `help_link(Topic::...)` sweep is a
/// TODO(O-20-followup), per the DIFF's 25-row table. Keep the enum whole
/// so adding a call site is a one-line edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum Topic {
    // Field guide
    Dashboard,
    Today,
    Sharing,
    Reviews,
    Species,
    Analytics,
    Phenology,
    DawnChorus,
    Recordings,
    Feeds,
    Reports,
    DisplayPrefs,
    Kiosk,
    // Admin
    AdminSettings,
    AdminAudio,
    AdminRecording,
    AdminNotifications,
    AdminRemoteAccess,
    AdminBackups,
    AdminSystem,
    // Guides
    Tuning,
    Glossary,
    Faq,
    Troubleshooting,
    MigrationImport,
}

impl Topic {
    /// Stable URL for this topic (no `index.html`, matches mdBook conventions).
    #[must_use]
    pub const fn href(self) -> &'static str {
        match self {
            // Field guide
            Self::Dashboard         => "/help/guide/dashboard",
            Self::Today             => "/help/guide/today",
            Self::Sharing           => "/help/guide/sharing",
            Self::Reviews           => "/help/guide/reviews",
            Self::Species           => "/help/guide/species",
            Self::Analytics         => "/help/guide/analytics",
            Self::Phenology         => "/help/guide/phenology",
            Self::DawnChorus        => "/help/guide/dawn-chorus",
            Self::Recordings        => "/help/guide/recordings",
            Self::Feeds             => "/help/guide/feeds",
            Self::Reports           => "/help/guide/reports",
            Self::DisplayPrefs      => "/help/guide/display-preferences",
            Self::Kiosk             => "/help/guide/kiosk",
            // Admin
            Self::AdminSettings     => "/help/admin/settings",
            Self::AdminAudio        => "/help/admin/audio",
            Self::AdminRecording    => "/help/admin/recording",
            Self::AdminNotifications => "/help/admin/notifications",
            Self::AdminRemoteAccess => "/help/admin/remote-access",
            Self::AdminBackups      => "/help/admin/backups",
            Self::AdminSystem       => "/help/admin/system",
            // Guides
            Self::Tuning            => "/help/guides/tuning",
            Self::Glossary          => "/help/guides/glossary",
            Self::Faq               => "/help/guides/faq",
            Self::Troubleshooting   => "/help/guides/troubleshooting",
            Self::MigrationImport   => "/help/guides/migration",
        }
    }

    /// Default link label. Each call site can override.
    #[must_use]
    pub const fn default_label(self) -> &'static str {
        match self {
            Self::Tuning => "Tune detection accuracy",
            Self::Phenology => "What is phenology?",
            Self::DawnChorus => "How dawn-chorus ribbons read",
            Self::Analytics => "Reading these charts",
            Self::Reviews => "How review works",
            Self::Glossary => "Glossary",
            _ => "How this works",
        }
    }
}

/// Inline help link — opens in a new tab. Use this for almost every screen.
#[must_use]
pub fn help_link(topic: Topic) -> String {
    help_link_labeled(topic, topic.default_label())
}

/// Variant with a custom label.
#[must_use]
pub fn help_link_labeled(topic: Topic, label: &str) -> String {
    format!(
        r#"<a class="bnb-help-link" href="{href}" target="_blank" rel="noopener" aria-label="{label_attr} (opens in a new tab)">
  <span class="bnb-help-link__glyph" aria-hidden="true">?</span>
  <span>{label}</span>
  <span class="bnb-help-link__arrow" aria-hidden="true">→</span>
</a>"#,
        href = topic.href(),
        label = label,
        label_attr = label,
    )
}

/// In-place drawer trigger — opens the same docs page inside a right-side
/// `<dialog>` so the user doesn't lose the current screen. Pairs with the
/// `_partial_help_drawer.html` mounted in `layout.html`.
///
/// Part of the documented API surface; no call site exists in main yet
/// (the per-template sweep is TODO(O-20-followup)).
#[must_use]
#[allow(dead_code)]
pub fn help_drawer(topic: Topic, label: &str) -> String {
    format!(
        r#"<button type="button" class="bnb-help-link"
  data-help-drawer="{href}"
  aria-haspopup="dialog">
  <span class="bnb-help-link__glyph" aria-hidden="true">?</span>
  <span>{label}</span>
  <span class="bnb-help-link__arrow" aria-hidden="true">↗</span>
</button>"#,
        href = topic.href(),
        label = label,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_topic_has_unique_href() {
        let topics = [
            Topic::Dashboard, Topic::Today, Topic::Sharing, Topic::Reviews,
            Topic::Species, Topic::Analytics, Topic::Phenology, Topic::DawnChorus,
            Topic::Recordings, Topic::Feeds, Topic::Reports, Topic::DisplayPrefs,
            Topic::Kiosk, Topic::AdminSettings, Topic::AdminAudio, Topic::AdminRecording,
            Topic::AdminNotifications, Topic::AdminRemoteAccess, Topic::AdminBackups,
            Topic::AdminSystem, Topic::Tuning, Topic::Glossary, Topic::Faq,
            Topic::Troubleshooting, Topic::MigrationImport,
        ];
        let mut sorted: Vec<&'static str> = topics.iter().map(|t| t.href()).collect();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), topics.len(), "duplicate hrefs in Topic table");
    }

    #[test]
    fn href_is_help_prefixed() {
        for t in [Topic::Dashboard, Topic::DawnChorus, Topic::AdminAudio] {
            assert!(t.href().starts_with("/help/"));
        }
    }

    #[test]
    fn help_link_emits_anchor() {
        let html = help_link(Topic::DawnChorus);
        assert!(html.contains(r#"href="/help/guide/dawn-chorus""#));
        assert!(html.contains(r#"target="_blank""#));
        assert!(html.contains("How dawn-chorus ribbons read"));
    }

    #[test]
    fn help_drawer_emits_button_with_data() {
        let html = help_drawer(Topic::Tuning, "Why this matters");
        assert!(html.contains(r#"data-help-drawer="/help/guides/tuning""#));
        assert!(html.contains("Why this matters"));
        assert!(html.contains(r#"aria-haspopup="dialog""#));
    }
}
