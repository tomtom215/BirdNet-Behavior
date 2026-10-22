//! When a saved setting takes effect: now, within about half a minute, or only
//! after the station restarts.
//!
//! # Why this is a table and not a default
//!
//! The settings page has said, since it was written, "Settings are applied when
//! the station next starts". That is true of most of them and false of some: the
//! species lists are re-read while the station runs, the spectrogram options on
//! the next picture, the rare-species window on the next page. And one thing a
//! person would assume is live is not: the email settings are read by the
//! notifier **once, at start**, although they are stored in the table the
//! species lists live in. One sentence on the page cannot be true of both, so
//! each setting says which it is.
//!
//! The answer is not a guess. For every setting in the bridge table
//! (`SETTING_SPECS` in the binary) it comes from reading the code that consumes
//! the value, and [`Entry::evidence`] says where. A setting the bridge copies
//! onto the runtime configuration (`Wiring::Bridged` in the binary) is, by
//! construction, read when the consumer is built at start-up, so every bridged
//! key is [`Applies::Restart`]; the binary's tests hold that. A setting its
//! subsystem reads out of the table is whatever that subsystem does, and is
//! listed here with the reason.
//!
//! The table is complete or the binary's tests fail: a setting added without a
//! row here shows no badge, and `every_setting_says_when_it_applies` names it.

/// When a setting's saved value reaches the thing it controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applies {
    /// The next time the value is used: a request, a page, a picture.
    Now,
    /// Within about half a minute: a background loop re-reads it on a timer.
    Soon,
    /// Only when the station starts. Save it, then restart.
    Restart,
    /// Stored for a reader that does not act on it (a record, not a control).
    RecordOnly,
}

impl Applies {
    /// The short phrase shown beside the setting.
    #[must_use]
    pub const fn badge(self) -> &'static str {
        match self {
            Self::Now => "Applies immediately",
            Self::Soon => "Applies within about 30 seconds",
            Self::Restart => "Applies after restart",
            Self::RecordOnly => "Recorded only",
        }
    }

    /// The stable word for a CSS class and the documentation table.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Now => "now",
            Self::Soon => "soon",
            Self::Restart => "restart",
            Self::RecordOnly => "record",
        }
    }

    /// Whether saving the value leaves the running station as it was.
    #[must_use]
    pub const fn needs_restart(self) -> bool {
        matches!(self, Self::Restart)
    }
}

/// One setting, when it applies, and the code that proves it.
#[derive(Debug, Clone, Copy)]
pub struct Entry {
    /// The settings-table key.
    pub key: &'static str,
    /// When a saved value takes effect.
    pub applies: Applies,
    /// Where the consumer is, in words and a path, so the claim can be checked.
    pub evidence: &'static str,
}

/// Every key in the binary's bridge table, in its order.
pub const APPLIES: &[Entry] = &[
    Entry {
        key: "confidence_threshold",
        applies: Applies::Restart,
        evidence: "src/daemon/config.rs resolve_confidence, called when the detection engine starts",
    },
    Entry {
        key: "sensitivity",
        applies: Applies::Restart,
        evidence: "src/daemon/config.rs resolve_sensitivity, called when the detection engine starts",
    },
    Entry {
        key: "overlap",
        applies: Applies::Restart,
        evidence: "src/daemon/mod.rs, read when the detection engine starts",
    },
    Entry {
        key: "sf_thresh",
        applies: Applies::Restart,
        evidence: "src/daemon/mod.rs, read when the detection engine starts",
    },
    Entry {
        key: "privacy_threshold",
        applies: Applies::Restart,
        evidence: "src/daemon/mod.rs, read when the detection engine starts",
    },
    Entry {
        key: "privacy_rank",
        applies: Applies::Restart,
        evidence: "src/daemon/mod.rs, read when the detection engine starts",
    },
    Entry {
        key: "confirmation_level",
        applies: Applies::Restart,
        evidence: "src/daemon/mod.rs, read when the detection engine starts",
    },
    Entry {
        key: "alsa_device",
        applies: Applies::Restart,
        evidence: "src/capture/sources.rs seeds the audio-source table only while it is empty, at start; once an active audio_sources row exists src/capture.rs reads the table alone, so the settings page offers this field only while there is none",
    },
    Entry {
        key: "alsa_devices",
        applies: Applies::Restart,
        evidence: "src/capture/sources.rs seeds the audio-source table only while it is empty, at start; once an active audio_sources row exists src/capture.rs reads the table alone, so the settings page offers this field only while there is none",
    },
    Entry {
        key: "rtsp_url",
        applies: Applies::Restart,
        evidence: "src/capture/sources.rs seeds the audio-source table only while it is empty, at start; once an active audio_sources row exists src/capture.rs reads the table alone, so the settings page offers this field only while there is none",
    },
    Entry {
        key: "rtsp_urls",
        applies: Applies::Restart,
        evidence: "src/capture/sources.rs seeds the audio-source table only while it is empty, at start; once an active audio_sources row exists src/capture.rs reads the table alone, so the settings page offers this field only while there is none",
    },
    Entry {
        key: "audio_format",
        applies: Applies::Restart,
        evidence: "src/daemon/config.rs, read when the detection engine starts",
    },
    Entry {
        key: "segment_duration",
        applies: Applies::Restart,
        evidence: "src/capture.rs resolves it once when capture starts",
    },
    Entry {
        key: "freq_shift_hz",
        applies: Applies::Restart,
        evidence: "src/daemon/config.rs, read when the detection engine starts",
    },
    Entry {
        key: "clip_target_lufs",
        applies: Applies::Restart,
        evidence: "src/daemon/config.rs, read when the detection engine starts",
    },
    Entry {
        key: "livestream_source",
        applies: Applies::Now,
        evidence: "read on every request by routes/livestream.rs",
    },
    Entry {
        key: "analytics_exclude_imports",
        applies: Applies::Now,
        evidence: "the analytics view is rebuilt when the setting is flipped (connection/sync.rs set_exclude_imports)",
    },
    Entry {
        key: "today_layout",
        applies: Applies::Now,
        evidence: "read on every render of the Today page",
    },
    Entry {
        key: "latitude",
        applies: Applies::Restart,
        evidence: "src/daemon/config.rs, read when the detection engine starts",
    },
    Entry {
        key: "longitude",
        applies: Applies::Restart,
        evidence: "src/daemon/config.rs, read when the detection engine starts",
    },
    Entry {
        key: "station_name",
        applies: Applies::Restart,
        evidence: "src/integrations/mqtt.rs, read when the MQTT client is built at start",
    },
    Entry {
        key: "recording_schedule",
        applies: Applies::Restart,
        evidence: "src/capture/schedule.rs parses it once when capture starts",
    },
    Entry {
        key: "timezone",
        applies: Applies::RecordOnly,
        evidence: "kept for the doctor to compare with the host clock; nothing reads it to change behaviour",
    },
    Entry {
        key: "onboarding_complete",
        applies: Applies::Now,
        evidence: "read on every visit to the dashboard",
    },
    Entry {
        key: "night_inhibit",
        applies: Applies::Restart,
        evidence: "src/capture/schedule.rs parses it once when capture starts",
    },
    Entry {
        key: "pre_sunrise_offset",
        applies: Applies::Restart,
        evidence: "src/capture/schedule.rs parses it once when capture starts",
    },
    Entry {
        key: "post_sunset_offset",
        applies: Applies::Restart,
        evidence: "src/capture/schedule.rs parses it once when capture starts",
    },
    Entry {
        key: "site_name",
        applies: Applies::Restart,
        evidence: "src/helpers/state.rs, read when the state is built at start",
    },
    Entry {
        key: "info_site",
        applies: Applies::Restart,
        evidence: "src/helpers/state.rs, read when the state is built at start",
    },
    Entry {
        key: "apprise_url",
        applies: Applies::Restart,
        evidence: "src/integrations/apprise.rs builds the client once at start",
    },
    Entry {
        key: "apprise_config",
        applies: Applies::Restart,
        evidence: "src/integrations/apprise.rs builds the client once at start",
    },
    Entry {
        key: "notify_urls",
        applies: Applies::Restart,
        evidence: "src/integrations/apprise.rs builds the client once at start",
    },
    Entry {
        key: "birdweather_token",
        applies: Applies::Restart,
        evidence: "src/integrations/birdweather.rs builds the client once at start",
    },
    Entry {
        key: "notify_confidence",
        applies: Applies::Restart,
        evidence: "src/integrations/apprise.rs builds the client once at start",
    },
    Entry {
        key: "notify_cooldown",
        applies: Applies::Restart,
        evidence: "src/integrations/apprise.rs builds the client once at start",
    },
    Entry {
        key: "notify_trigger",
        applies: Applies::Restart,
        evidence: "src/integrations/notification.rs builds the filter once at start",
    },
    Entry {
        key: "notify_species_only",
        applies: Applies::Restart,
        evidence: "src/integrations/notification.rs builds the filter once at start",
    },
    Entry {
        key: "notify_species_exclude",
        applies: Applies::Restart,
        evidence: "src/integrations/notification.rs builds the filter once at start",
    },
    Entry {
        key: "notify_title_template",
        applies: Applies::Restart,
        evidence: "src/integrations/notification.rs builds the template once at start",
    },
    Entry {
        key: "notify_body_template",
        applies: Applies::Restart,
        evidence: "src/integrations/notification.rs builds the template once at start",
    },
    Entry {
        key: "weekly_report_schedule",
        applies: Applies::Restart,
        evidence: "src/app.rs resolves it once at start",
    },
    Entry {
        key: "heartbeat_url",
        applies: Applies::Restart,
        evidence: "src/integrations/heartbeat.rs builds the client once at start",
    },
    Entry {
        key: "deadman_hours",
        applies: Applies::Restart,
        evidence: "src/app.rs resolves it once at start",
    },
    Entry {
        key: "database_lang",
        applies: Applies::Restart,
        evidence: "src/helpers/state.rs builds the name table once at start",
    },
    Entry {
        key: "species_include",
        applies: Applies::Soon,
        evidence: "the detection loop re-reads the lists when they are older than SPECIES_LISTS_TTL, before the next file (core/detection/daemon/run.rs)",
    },
    Entry {
        key: "species_exclude",
        applies: Applies::Soon,
        evidence: "the same list reload as species_include: the detection loop re-reads all three lists together (core/detection/daemon/run.rs)",
    },
    Entry {
        key: "species_whitelist",
        applies: Applies::Soon,
        evidence: "the same list reload as species_include: the detection loop re-reads all three lists together (core/detection/daemon/run.rs)",
    },
    Entry {
        key: "image_cache_dir",
        applies: Applies::Restart,
        evidence: "src/helpers/state.rs opens the cache once at start",
    },
    Entry {
        key: "custom_image_dir",
        applies: Applies::Restart,
        evidence: "src/app.rs resolves it once at start",
    },
    Entry {
        key: "max_files_per_species",
        applies: Applies::Restart,
        evidence: "src/helpers/system.rs and src/app.rs resolve it once at start",
    },
    Entry {
        key: "purge_threshold",
        applies: Applies::Restart,
        evidence: "src/helpers/system.rs configures the disk manager once at start",
    },
    Entry {
        key: "full_disk_action",
        applies: Applies::Restart,
        evidence: "src/helpers/system.rs resolve_full_disk_action configures the recordings disk manager once at start",
    },
    Entry {
        key: "extraction_length",
        applies: Applies::Restart,
        evidence: "src/daemon/config.rs, read when the detection engine starts",
    },
    Entry {
        key: "time_format",
        applies: Applies::Now,
        evidence: "read from /api/v2/display by static/display-format.js on every page load (display.rs)",
    },
    Entry {
        key: "temperature_unit",
        applies: Applies::Now,
        evidence: "read from /api/v2/display by static/display-format.js on every page load (display.rs)",
    },
    Entry {
        key: "dashboard_image",
        applies: Applies::Now,
        evidence: "read on each render and poll of the Today picture block (routes/dashboard_image.rs)",
    },
    Entry {
        key: "dashboard_image_title",
        applies: Applies::Now,
        evidence: "read on each render and poll of the Today picture block (routes/dashboard_image.rs)",
    },
    Entry {
        key: "spectrogram_range_db",
        applies: Applies::Now,
        evidence: "read for every spectrogram drawn (routes/spectrogram/mod.rs)",
    },
    Entry {
        key: "raw_spectrogram",
        applies: Applies::Now,
        evidence: "read for every spectrogram drawn (routes/spectrogram/mod.rs); a picture your browser already has is not redrawn",
    },
    Entry {
        key: "rare_species_days",
        applies: Applies::Now,
        evidence: "read per request by tracking.rs and routes/feeds.rs",
    },
    Entry {
        key: "stream_retention_secs",
        applies: Applies::Restart,
        evidence: "src/helpers/system.rs configures the disk manager once at start",
    },
    Entry {
        key: "stream_max_mb",
        applies: Applies::Restart,
        evidence: "src/helpers/system.rs configures the disk manager once at start",
    },
    Entry {
        key: "clip_retention_days",
        applies: Applies::Restart,
        evidence: "src/app.rs resolves it once at start",
    },
    Entry {
        key: "email_smtp_host",
        applies: Applies::Restart,
        evidence: "create_email_notifier reads the table once, at start (src/app.rs)",
    },
    Entry {
        key: "email_smtp_port",
        applies: Applies::Restart,
        evidence: "create_email_notifier reads the table once, at start (src/app.rs)",
    },
    Entry {
        key: "email_smtp_user",
        applies: Applies::Restart,
        evidence: "create_email_notifier reads the table once, at start (src/app.rs)",
    },
    Entry {
        key: "email_smtp_pass",
        applies: Applies::Restart,
        evidence: "create_email_notifier reads the table once, at start (src/app.rs)",
    },
    Entry {
        key: "email_from",
        applies: Applies::Restart,
        evidence: "create_email_notifier reads the table once, at start (src/app.rs)",
    },
    Entry {
        key: "email_to",
        applies: Applies::Restart,
        evidence: "create_email_notifier reads the table once, at start (src/app.rs)",
    },
    Entry {
        key: "email_from_name",
        applies: Applies::Restart,
        evidence: "create_email_notifier reads the table once, at start (src/app.rs)",
    },
    Entry {
        key: "email_starttls",
        applies: Applies::Restart,
        evidence: "create_email_notifier reads the table once, at start (src/app.rs)",
    },
    Entry {
        key: "email_min_confidence",
        applies: Applies::Restart,
        evidence: "create_email_notifier reads the table once, at start (src/app.rs)",
    },
    Entry {
        key: "email_cooldown_secs",
        applies: Applies::Restart,
        evidence: "create_email_notifier reads the table once, at start (src/app.rs)",
    },
    Entry {
        key: "spectrogram_colormap",
        applies: Applies::Now,
        evidence: "read for every spectrogram drawn (routes/spectrogram/mod.rs)",
    },
];

/// Why every table-driven setting ([`super::extra`]) is [`Applies::Restart`]:
/// each is copied by the binary's bridge onto the configuration key its consumer
/// reads, and every one of those consumers is built once, at start.
const EXTRA_EVIDENCE: &str = "bridged onto its configuration key (extra.rs config_key); the consumer is built from the configuration once, at start";

/// Every setting that says when it applies: the hand-written table and the
/// table-driven ones, in that order.
#[must_use]
pub fn every() -> Vec<Entry> {
    APPLIES
        .iter()
        .copied()
        .chain(super::extra::EXTRA_SETTINGS.iter().map(|e| Entry {
            key: e.key,
            applies: Applies::Restart,
            evidence: EXTRA_EVIDENCE,
        }))
        .collect()
}

/// When `key` takes effect, if it is in either table.
#[must_use]
pub fn applies_for(key: &str) -> Option<Applies> {
    every().iter().find(|e| e.key == key).map(|e| e.applies)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn no_key_appears_twice() {
        let keys: BTreeSet<&str> = APPLIES.iter().map(|e| e.key).collect();
        assert_eq!(keys.len(), APPLIES.len());
    }

    #[test]
    fn every_entry_names_its_evidence() {
        for e in APPLIES {
            assert!(
                e.evidence.len() > 20,
                "{}: evidence {:?} says too little to check",
                e.key,
                e.evidence
            );
        }
    }

    /// The badge words are what the page shows and the manual quotes; a
    /// change to one should be a decision.
    #[test]
    fn the_badges_read_as_they_are_documented() {
        assert_eq!(Applies::Now.badge(), "Applies immediately");
        assert_eq!(Applies::Restart.badge(), "Applies after restart");
        assert!(Applies::Restart.needs_restart());
        assert!(!Applies::Now.needs_restart());
        assert!(!Applies::Soon.needs_restart());
        assert!(!Applies::RecordOnly.needs_restart());
    }
}
