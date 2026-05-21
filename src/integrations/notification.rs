//! Notification filter and message-template construction.

use crate::cli::Cli;

/// Create a notification filter from CLI flags.
pub fn create_notification_filter(
    cli: &Cli,
) -> birdnet_integrations::notification::NotificationFilter {
    use birdnet_integrations::notification::{NotificationFilter, SpeciesFilter, TriggerMode};

    let trigger = TriggerMode::parse(&cli.notify_trigger);
    let species_filter = SpeciesFilter::new(
        cli.notify_species_exclude.as_deref(),
        cli.notify_species_only.as_deref(),
    );

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
        let f = create_notification_filter(&cli);
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
        let f = create_notification_filter(&cli);
        assert_eq!(f.trigger, TriggerMode::NewSpecies);
    }

    #[test]
    fn filter_parses_new_species_daily_trigger() {
        use birdnet_integrations::notification::TriggerMode;
        let mut cli = default_cli();
        cli.notify_trigger = "new-species-daily".to_owned();
        let f = create_notification_filter(&cli);
        assert_eq!(f.trigger, TriggerMode::NewSpeciesDaily);
    }

    #[test]
    fn filter_honours_species_exclude_list() {
        let mut cli = default_cli();
        cli.notify_species_exclude = Some("Pica pica, Corvus corax".to_owned());
        let f = create_notification_filter(&cli);
        assert!(!f.species_filter.is_allowed("Pica pica"));
        assert!(!f.species_filter.is_allowed("Corvus corax"));
        assert!(f.species_filter.is_allowed("Turdus merula"));
    }

    #[test]
    fn filter_honours_species_only_list() {
        let mut cli = default_cli();
        cli.notify_species_only = Some("Turdus merula,Erithacus rubecula".to_owned());
        let f = create_notification_filter(&cli);
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
