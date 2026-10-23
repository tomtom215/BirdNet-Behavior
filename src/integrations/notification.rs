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
    if let Some(notice) = trigger_off_notice(&trigger_str, &trigger) {
        tracing::warn!("{notice}");
    }

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

/// The warning for a trigger that turns every notification off.
///
/// `off`, `none` and `never` used to be read as "each" (an unknown word falls
/// back to that); they now mean what they say. A station whose file still says
/// `off` was being notified about every detection and is not any more, and a
/// default BirdNET-Pi `birdnet.conf` imports as `off` as it did there. Said once
/// at start-up, in the journal, because nothing else on screen says that the
/// phone is quiet by choice.
fn trigger_off_notice(
    raw: &str,
    trigger: &birdnet_integrations::notification::TriggerMode,
) -> Option<String> {
    (*trigger == birdnet_integrations::notification::TriggerMode::Off).then(|| {
        format!(
            "APPRISE_TRIGGER={:?} turns detection notifications OFF: no detection is sent. \
             Earlier versions read off, none and never as \"each\" and notified about every \
             detection. For every detection set it to each; for rare species new-species",
            raw.trim()
        )
    })
}

/// Create a notification template from CLI flags and/or config.
///
/// `station_url` is how the device that reads the notification reaches this
/// station (see [`station_url`]); it is what `$listenurl`, `$friendlyurl` and
/// `$image` point at, and without it they render as nothing.
pub fn create_notification_template(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
    station_url: Option<String>,
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

    let template = match (title, body) {
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
    };
    template.with_station_url(station_url)
}

/// A configured address, if it is one a notification can safely carry: an
/// `http(s)://` origin with no whitespace, quotes or angle brackets, trailing
/// slash dropped.
fn usable_base_url(raw: &str) -> Option<String> {
    let url = raw.trim().trim_end_matches('/');
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let clean = !rest.is_empty()
        && url.len() <= 256
        && !url
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '"' | '\'' | '<' | '>'));
    clean.then(|| url.to_owned())
}

/// How a phone on the station's network reaches the dashboard, for the links in
/// a notification — the decision, kept apart from the environment it reads.
///
/// The operator's own words first: `BNB_PUBLIC_URL`, then `BNB_BASE_URL` (the
/// address the feeds already use). Failing those, `BirdNET-Pi`'s fallback: the
/// machine's name on the local network, `http://<hostname>.local[:port]`, which
/// is what the installer prints at the end of a setup. `None` when the station
/// cannot say how it is reached — a link to `localhost` would reach the phone,
/// not the station.
///
/// `served` is the scheme and port the dashboard really answers on: `https`
/// only when HTTPS is the single socket, because otherwise plain HTTP is
/// still there.
#[must_use]
pub fn station_url_from(
    public_url: Option<&str>,
    base_url: Option<&str>,
    hostname: Option<&str>,
    served: (&str, u16),
) -> Option<String> {
    if let Some(url) = public_url.and_then(usable_base_url) {
        return Some(url);
    }
    if let Some(url) = base_url.and_then(usable_base_url) {
        return Some(url);
    }
    let host = hostname?.trim().trim_end_matches('.');
    if host.is_empty()
        || host
            .chars()
            .any(|c| !(c.is_ascii_alphanumeric() || c == '-' || c == '.'))
    {
        return None;
    }
    let host = if host.contains('.') {
        host.to_owned()
    } else {
        format!("{host}.local")
    };
    let (scheme, port) = served;
    let default_port = (scheme == "http" && port == 80) || (scheme == "https" && port == 443);
    Some(if default_port {
        format!("{scheme}://{host}")
    } else {
        format!("{scheme}://{host}:{port}")
    })
}

/// [`station_url_from`] with the environment and the machine's hostname read.
#[must_use]
pub fn station_url(
    addr: std::net::SocketAddr,
    tls: &crate::helpers::tls::TlsPlan,
) -> Option<String> {
    let served = match tls.https_addr {
        Some(https) if tls.shares_socket => ("https", https.port()),
        _ => ("http", addr.port()),
    };
    station_url_from(
        std::env::var("BNB_PUBLIC_URL").ok().as_deref(),
        std::env::var("BNB_BASE_URL").ok().as_deref(),
        birdnet_web::tls::system_hostname().as_deref(),
        served,
    )
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
            shown_name: None,
            confidence: 0.9,
            confidence_pct: 90,
            date: "2026-05-19".to_owned(),
            time: "09:00:00".to_owned(),
            week: 20,
            latitude: None,
            longitude: None,
            reason: String::new(),
            cutoff: None,
            sensitivity: None,
            overlap: None,
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

    // ── station_url_from ────────────────────────────────────────────────

    const HTTP_8502: (&str, u16) = ("http", 8502);

    #[test]
    fn the_operators_address_comes_first_then_the_feed_address() {
        use super::station_url_from as url;
        assert_eq!(
            url(
                Some("https://birds.example.com/"),
                Some("http://other"),
                Some("pi"),
                HTTP_8502
            )
            .as_deref(),
            Some("https://birds.example.com")
        );
        assert_eq!(
            url(None, Some("http://birdnet.local"), Some("pi"), HTTP_8502).as_deref(),
            Some("http://birdnet.local")
        );
        // An address that is not an http(s) origin is skipped, not carried into
        // a push notification.
        assert_eq!(
            url(
                Some("ftp://x"),
                Some("http://birdnet.local"),
                None,
                HTTP_8502
            )
            .as_deref(),
            Some("http://birdnet.local")
        );
        assert_eq!(
            url(
                Some("http://a b"),
                Some("http://c\"d"),
                Some("pi"),
                HTTP_8502
            )
            .as_deref(),
            Some("http://pi.local:8502"),
            "whitespace and quotes disqualify an address"
        );
    }

    #[test]
    fn without_an_address_the_machine_is_found_by_name() {
        use super::station_url_from as url;
        assert_eq!(
            url(None, None, Some("birdnet\n"), HTTP_8502).as_deref(),
            Some("http://birdnet.local:8502")
        );
        // A name that already has a domain is not given a second one.
        assert_eq!(
            url(None, None, Some("pi.example.org"), HTTP_8502).as_deref(),
            Some("http://pi.example.org:8502")
        );
        // The default port of the scheme is left off.
        assert_eq!(
            url(None, None, Some("pi"), ("http", 80)).as_deref(),
            Some("http://pi.local")
        );
        assert_eq!(
            url(None, None, Some("pi"), ("https", 443)).as_deref(),
            Some("https://pi.local")
        );
        assert_eq!(
            url(None, None, Some("pi"), ("https", 8503)).as_deref(),
            Some("https://pi.local:8503"),
            "HTTPS alone on its socket is linked as https"
        );
    }

    /// The counterpart that stops the fallback inventing an address: a station
    /// that cannot say how it is reached has no link, not a link to itself.
    #[test]
    fn a_station_that_cannot_say_how_it_is_reached_has_no_address() {
        use super::station_url_from as url;
        assert_eq!(url(None, None, None, HTTP_8502), None);
        assert_eq!(url(None, None, Some(""), HTTP_8502), None);
        assert_eq!(url(None, None, Some("bad host!"), HTTP_8502), None);
        assert_eq!(url(Some(""), Some(" "), None, HTTP_8502), None);
    }

    // ── create_notification_template ───────────────────────────────────

    #[test]
    fn template_default_when_neither_title_nor_body_supplied() {
        let cli = default_cli();
        let t = create_notification_template(&cli, None, None);
        let (title, body) = t.render(&ctx());
        assert!(!title.is_empty(), "default title must not be empty");
        assert!(!body.is_empty(), "default body must not be empty");
    }

    #[test]
    fn template_uses_cli_title_and_body() {
        let mut cli = default_cli();
        cli.notify_title_template = Some("Title $comname".to_owned());
        cli.notify_body_template = Some("Body $sciname".to_owned());
        let t = create_notification_template(&cli, None, None);
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
        let t = create_notification_template(&cli, Some(&cfg), None);
        let (title, body) = t.render(&ctx());
        assert!(title.contains("Config-Eurasian Magpie"));
        assert!(body.contains("90"));
    }

    #[test]
    fn template_only_title_uses_default_body_with_full_substitutions() {
        let mut cli = default_cli();
        cli.notify_title_template = Some("only-title".to_owned());
        let t = create_notification_template(&cli, None, None);
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
        let t = create_notification_template(&cli, None, None);
        let (title, body) = t.render(&ctx());
        assert_eq!(body, "only-body");
        // The hand-rolled default title is "Bird Detection: $comname".
        assert!(title.contains("Eurasian Magpie"));
    }

    /// M3: `off`, `none` and `never` silence notifications and the journal says
    /// so; the counterparts (a real rule, an unknown word that falls back to
    /// "each") say nothing.
    #[test]
    fn a_trigger_that_silences_notifications_is_announced() {
        use super::trigger_off_notice;
        use birdnet_integrations::notification::TriggerMode;
        for word in ["off", "none", "never", " OFF "] {
            let notice = trigger_off_notice(word, &TriggerMode::parse(word))
                .unwrap_or_else(|| panic!("{word:?} was not announced"));
            assert!(
                notice.contains("OFF") && notice.contains("each"),
                "{notice}"
            );
        }
        for word in ["each", "new-species", "each,new-species", "sometimes", ""] {
            assert!(
                trigger_off_notice(word, &TriggerMode::parse(word)).is_none(),
                "{word:?} was announced as off"
            );
        }
    }
}
