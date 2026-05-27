//! Help / methodology cross-links into the embedded mdBook at `/help/...`.
//!
//! See O-20 DIFF.md. The mdBook is built by `build.rs` (option A) and
//! embedded under the binary; runtime simply serves the static HTML files.
//!
//! This module only handles the *link emitters* — `help_link()` and
//! `help_drawer()`. The static-file serve is mounted in `routes/mod.rs`:
//!
//! ```ignore
//! .nest_service("/help", tower_http::services::ServeDir::new(help_dir()))
//! ```
//!
//! where `help_dir()` returns the path that `build.rs` extracted to.

/// Stable identifier for a docs page. Maps 1:1 to `docs/book/<section>/<page>.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
            Topic::Dashboard         => "/help/guide/dashboard",
            Topic::Today             => "/help/guide/today",
            Topic::Sharing           => "/help/guide/sharing",
            Topic::Reviews           => "/help/guide/reviews",
            Topic::Species           => "/help/guide/species",
            Topic::Analytics         => "/help/guide/analytics",
            Topic::Phenology         => "/help/guide/phenology",
            Topic::DawnChorus        => "/help/guide/dawn-chorus",
            Topic::Recordings        => "/help/guide/recordings",
            Topic::Feeds             => "/help/guide/feeds",
            Topic::Reports           => "/help/guide/reports",
            Topic::DisplayPrefs      => "/help/guide/display-preferences",
            Topic::Kiosk             => "/help/guide/kiosk",
            // Admin
            Topic::AdminSettings     => "/help/admin/settings",
            Topic::AdminAudio        => "/help/admin/audio",
            Topic::AdminRecording    => "/help/admin/recording",
            Topic::AdminNotifications => "/help/admin/notifications",
            Topic::AdminRemoteAccess => "/help/admin/remote-access",
            Topic::AdminBackups      => "/help/admin/backups",
            Topic::AdminSystem       => "/help/admin/system",
            // Guides
            Topic::Tuning            => "/help/guides/tuning",
            Topic::Glossary          => "/help/guides/glossary",
            Topic::Faq               => "/help/guides/faq",
            Topic::Troubleshooting   => "/help/guides/troubleshooting",
            Topic::MigrationImport   => "/help/guides/migration",
        }
    }

    /// Default link label. Each call site can override.
    #[must_use]
    pub const fn default_label(self) -> &'static str {
        match self {
            Topic::Tuning => "Tune detection accuracy",
            Topic::Phenology => "What is phenology?",
            Topic::DawnChorus => "How dawn-chorus ribbons read",
            Topic::Analytics => "Reading these charts",
            Topic::Reviews => "How review works",
            Topic::Glossary => "Glossary",
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
#[must_use]
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
        sorted.sort();
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
