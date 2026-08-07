//! Notification filter and message-template construction.

use crate::cli::Cli;
use crate::helpers::resolve;

/// Create a notification filter from CLI flags and/or config.
///
/// Takes `config` — which the settings overlay has already layered the admin UI
/// onto — so the Notifications page's trigger mode and species lists actually
/// govern what gets sent. Reading `cli.notify_trigger` alone meant the clap
/// default (`each`) always won and the dropdown was decorative.
pub fn create_notification_filter(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> birdnet_integrations::notification::NotificationFilter {
    use birdnet_integrations::notification::{NotificationFilter, SpeciesFilter, TriggerMode};

    let trigger_str = resolve::setting_str(
        cli,
        "notify_trigger",
        &cli.notify_trigger,
        config,
        "APPRISE_TRIGGER",
    );
    let trigger = TriggerMode::parse(&trigger_str);

    // These two flags have no clap default, so `Some` already means the
    // operator supplied one; the config carries the admin-UI value otherwise.
    let exclude = cli
        .notify_species_exclude
        .clone()
        .or_else(|| config.and_then(|c| c.get("APPRISE_WATCHLIST_EXCLUDE").map(String::from)));
    let only = cli
        .notify_species_only
        .clone()
        .or_else(|| config.and_then(|c| c.get("APPRISE_WATCHLIST").map(String::from)));

    let species_filter = SpeciesFilter::new(exclude.as_deref(), only.as_deref());

    tracing::info!(
        trigger = %trigger,
        "notification filter configured"
    );

    NotificationFilter {
        trigger,
        species_filter,
    }
}

/// Create a notification template from CLI flags and/or config.
pub fn create_notification_template(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> birdnet_integrations::notification::NotificationTemplate {
    use birdnet_integrations::notification::NotificationTemplate;

    let title = cli
        .notify_title_template
        .clone()
        .or_else(|| config?.get("APPRISE_TITLE_TEMPLATE").map(String::from));

    let body = cli
        .notify_body_template
        .clone()
        .or_else(|| config?.get("APPRISE_BODY_TEMPLATE").map(String::from));

    match (title, body) {
        (Some(t), Some(b)) => {
            tracing::debug!("custom notification template configured");
            NotificationTemplate::new(t, b)
        }
        (Some(t), None) => NotificationTemplate::new(
            t,
            "$comname ($sciname) detected ($confidencepct% confidence) at $time on $date"
                .to_string(),
        ),
        (None, Some(b)) => NotificationTemplate::new("Bird Detection: $comname".to_string(), b),
        (None, None) => NotificationTemplate::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::{create_notification_filter, create_notification_template};
    use crate::integrations::test_support::{config_with, default_cli};

    /// A fully-populated render context for template assertions.
    fn ctx() -> birdnet_integrations::notification::NotificationContext {
        birdnet_integrations::notification::NotificationContext {
            sci_name: "Pica pica".to_owned(),
            com_name: "Eurasian Magpie".to_owned(),
            confidence: 0.9,
            confidence_pct: 90,
            date: "2026-05-19".to_owned(),
            time: "09:00:00".to_owned(),
            week: 20,
            latitude: 0.0,
            longitude: 0.0,
            reason: String::new(),
            listen_url: None,
            image_url: None,
            station_url: None,
        }
    }

    // ── create_notification_filter ─────────────────────────────────────

    #[test]
    fn filter_uses_each_detection_trigger_by_default() {
        use birdnet_integrations::notification::TriggerMode;
        let cli = default_cli();
        let f = create_notification_filter(&cli, None);
        assert_eq!(f.trigger, TriggerMode::EachDetection);
        // No species filter → allow everything.
        assert!(f.species_filter.is_allowed("Pica pica"));
        assert!(f.species_filter.is_allowed("Corvus corax"));
    }

    #[test]
    fn filter_parses_new_species_trigger() {
        use birdnet_integrations::notification::TriggerMode;
        let mut cli = default_cli();
        cli.notify_trigger = "new-species".to_owned();
        let f = create_notification_filter(&cli, None);
        assert_eq!(f.trigger, TriggerMode::NewSpecies);
    }

    #[test]
    fn filter_parses_new_species_daily_trigger() {
        use birdnet_integrations::notification::TriggerMode;
        let mut cli = default_cli();
        cli.notify_trigger = "new-species-daily".to_owned();
        let f = create_notification_filter(&cli, None);
        assert_eq!(f.trigger, TriggerMode::NewSpeciesDaily);
    }

    #[test]
    fn filter_honours_species_exclude_list() {
        let mut cli = default_cli();
        cli.notify_species_exclude = Some("Pica pica, Corvus corax".to_owned());
        let f = create_notification_filter(&cli, None);
        assert!(!f.species_filter.is_allowed("Pica pica"));
        assert!(!f.species_filter.is_allowed("Corvus corax"));
        assert!(f.species_filter.is_allowed("Turdus merula"));
    }

    // ── settings reach the notification filter ─────────────────────────
    //
    // The Notifications page persisted every one of these and the runtime read
    // none of them: `notify_trigger` carries a clap default so `each` always
    // won, and the two species lists were CLI-only. A station whose operator
    // chose "New species (this week)" in the web UI still notified on every
    // single detection.

    #[test]
    fn trigger_from_settings_reaches_the_filter() {
        use birdnet_integrations::notification::TriggerMode;
        let cli = default_cli();
        let cfg = config_with(&[("APPRISE_TRIGGER", "new-species")]);
        let f = create_notification_filter(&cli, Some(&cfg));
        assert_eq!(f.trigger, TriggerMode::NewSpecies);
    }

    #[test]
    fn explicit_trigger_flag_beats_the_settings() {
        use birdnet_integrations::notification::TriggerMode;
        let mut cli = crate::integrations::test_support::cli_with_explicit(&["notify_trigger"]);
        cli.notify_trigger = "new-species-daily".to_owned();
        let cfg = config_with(&[("APPRISE_TRIGGER", "each")]);
        let f = create_notification_filter(&cli, Some(&cfg));
        assert_eq!(f.trigger, TriggerMode::NewSpeciesDaily);
    }

    #[test]
    fn species_lists_from_settings_reach_the_filter() {
        let cli = default_cli();
        let cfg = config_with(&[("APPRISE_WATCHLIST_EXCLUDE", "Pica pica")]);
        let f = create_notification_filter(&cli, Some(&cfg));
        assert!(!f.species_filter.is_allowed("Pica pica"));
        assert!(f.species_filter.is_allowed("Turdus merula"));

        let cfg = config_with(&[("APPRISE_WATCHLIST", "Turdus merula")]);
        let f = create_notification_filter(&cli, Some(&cfg));
        assert!(f.species_filter.is_allowed("Turdus merula"));
        assert!(!f.species_filter.is_allowed("Pica pica"));
    }

    #[test]
    fn filter_honours_species_only_list() {
        let mut cli = default_cli();
        cli.notify_species_only = Some("Turdus merula,Erithacus rubecula".to_owned());
        let f = create_notification_filter(&cli, None);
        assert!(f.species_filter.is_allowed("Turdus merula"));
        assert!(f.species_filter.is_allowed("Erithacus rubecula"));
        assert!(!f.species_filter.is_allowed("Pica pica"));
    }

    // ── create_notification_template ───────────────────────────────────

    #[test]
    fn template_default_when_neither_title_nor_body_supplied() {
        let cli = default_cli();
        let t = create_notification_template(&cli, None);
        let (title, body) = t.render(&ctx());
        assert!(!title.is_empty(), "default title must not be empty");
        assert!(!body.is_empty(), "default body must not be empty");
    }

    #[test]
    fn template_uses_cli_title_and_body() {
        let mut cli = default_cli();
        cli.notify_title_template = Some("Title $comname".to_owned());
        cli.notify_body_template = Some("Body $sciname".to_owned());
        let t = create_notification_template(&cli, None);
        let (title, body) = t.render(&ctx());
        assert!(title.contains("Eurasian Magpie"));
        assert!(body.contains("Pica pica"));
    }

    #[test]
    fn template_falls_back_to_config_when_cli_unset() {
        let cli = default_cli();
        let cfg = config_with(&[
            ("APPRISE_TITLE_TEMPLATE", "Config-$comname"),
            ("APPRISE_BODY_TEMPLATE", "Body-$confidencepct"),
        ]);
        let t = create_notification_template(&cli, Some(&cfg));
        let (title, body) = t.render(&ctx());
        assert!(title.contains("Config-Eurasian Magpie"));
        assert!(body.contains("90"));
    }

    #[test]
    fn template_only_title_uses_default_body_with_full_substitutions() {
        let mut cli = default_cli();
        cli.notify_title_template = Some("only-title".to_owned());
        let t = create_notification_template(&cli, None);
        let (title, body) = t.render(&ctx());
        assert_eq!(title, "only-title");
        // The hand-rolled default body uses $comname / $sciname /
        // $confidencepct / $time / $date placeholders.
        assert!(body.contains("Eurasian Magpie"));
        assert!(body.contains("Pica pica"));
        assert!(body.contains("90"));
    }

    #[test]
    fn template_only_body_uses_default_title() {
        let mut cli = default_cli();
        cli.notify_body_template = Some("only-body".to_owned());
        let t = create_notification_template(&cli, None);
        let (title, body) = t.render(&ctx());
        assert_eq!(body, "only-body");
        // The hand-rolled default title is "Bird Detection: $comname".
        assert!(title.contains("Eurasian Magpie"));
    }
}
