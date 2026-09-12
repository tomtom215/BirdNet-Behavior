//! Overlay admin-UI settings on top of the file-based configuration.
//!
//! The admin settings form (`/admin/settings`) persists key/value pairs to the
//! SQLite `settings` table, but the rest of the binary reads its configuration
//! from `/etc/birdnet/birdnet.conf` (parsed into [`Config`]) plus CLI flags.
//! Without a bridge the settings form is write-only: changing the confidence
//! threshold or the audio device in the UI has no effect on the running
//! station.
//!
//! This module is that bridge. At startup we read the settings table and layer
//! every known, non-empty value on top of the file config as a runtime
//! override — the database wins. The mapping is explicit (UI key → config key)
//! rather than blanket, because the two namespaces differ (the UI uses
//! lowercase `confidence_threshold`; the runtime reads BirdNET-Pi's uppercase
//! `CONFIDENCE`) and because only a curated, side-effect-free subset of
//! station/detection settings is wired here. Notification/integration and auth
//! settings are applied by their own subsystems so their activation can be
//! reasoned about (and messaged) on its own.
//!
//! Changes take effect on the next restart, matching the "Changes apply on next
//! restart" notice the settings page already shows.

use crate::cli::Cli;
use birdnet_core::config::Config;
use birdnet_db::settings::{SettingsCategory, ensure_settings_table, list, set};
use birdnet_web::state::AppState;
use std::collections::{BTreeMap, HashSet};

/// How an admin-UI setting reaches the running station — or why it does not.
///
/// Every key the settings form can persist carries one of these, and
/// `settings_form_keys_are_all_classified` fails the build's test run if one
/// does not. That total classification is the point: the mapping used to be an
/// allow-list that a new form field could simply be missing from, and twenty
/// fields ended up editable, persisted, and connected to nothing — the page
/// promising "changes apply on next restart" for values no restart would ever
/// read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wiring {
    /// Overlaid onto the runtime [`Config`] under this key by
    /// [`overlay_db_settings`], and seeded back out of it by
    /// [`seed_db_settings_from_config`]. The consumer reads the config, so the
    /// value takes effect on the next restart.
    Bridged(&'static str),
    /// Read straight out of the `settings` table by the named subsystem, which
    /// therefore needs no config key. Recorded so the key counts as wired
    /// without pretending it flows through the config.
    OwnedBy(&'static str),
}

/// Bridge specs: one per key the admin settings form can persist.
///
/// Single source of truth for both directions of the settings ↔ config bridge:
///
/// * [`overlay_db_settings`] reads the settings table and applies each
///   [`Wiring::Bridged`] row on top of the file config (UI key → config key) so
///   settings saved in the web UI take effect on the daemon.
/// * [`seed_db_settings_from_config`] does the inverse on first run: it copies
///   the installer-written file config *into* the settings table (config key →
///   UI key, tagged with `category`) so the values the operator entered during
///   installation actually appear in — and can be edited from — the web UI.
///
/// The list must cover
/// [`birdnet_web::routes::admin::settings::form::SETTINGS_FORM_KEYS`] exactly;
/// the test at the foot of this module enforces it in both directions.
const SETTING_SPECS: &[(&str, Wiring, SettingsCategory)] = &[
    // ── Detection tuning (consumed in `crate::daemon`) ─────────────────────
    (
        "confidence_threshold",
        Wiring::Bridged("CONFIDENCE"),
        SettingsCategory::Detection,
    ),
    (
        "sensitivity",
        Wiring::Bridged("SENSITIVITY"),
        SettingsCategory::Detection,
    ),
    (
        "overlap",
        Wiring::Bridged("OVERLAP"),
        SettingsCategory::Detection,
    ),
    (
        "sf_thresh",
        Wiring::Bridged("SF_THRESH"),
        SettingsCategory::Detection,
    ),
    (
        "privacy_threshold",
        Wiring::Bridged("PRIVACY_THRESHOLD"),
        SettingsCategory::Detection,
    ),
    (
        "confirmation_level",
        Wiring::Bridged("CONFIRMATION_LEVEL"),
        SettingsCategory::Detection,
    ),
    // ── Audio capture (consumed in `crate::capture`) ───────────────────────
    (
        "alsa_device",
        Wiring::Bridged("ALSA_CARD"),
        SettingsCategory::Audio,
    ),
    (
        "alsa_devices",
        Wiring::Bridged("ALSA_CARDS"),
        SettingsCategory::Audio,
    ),
    (
        "rtsp_url",
        Wiring::Bridged("RTSP_URL"),
        SettingsCategory::Audio,
    ),
    (
        "rtsp_urls",
        Wiring::Bridged("RTSP_URLS"),
        SettingsCategory::Audio,
    ),
    (
        "audio_format",
        Wiring::Bridged("AUDIOFMT"),
        SettingsCategory::Audio,
    ),
    (
        "segment_duration",
        Wiring::Bridged("SEGMENT_DURATION"),
        SettingsCategory::Audio,
    ),
    (
        "freq_shift_hz",
        Wiring::Bridged("FREQ_SHIFT"),
        SettingsCategory::Audio,
    ),
    (
        "clip_target_lufs",
        Wiring::Bridged("CLIP_TARGET_LUFS"),
        SettingsCategory::Audio,
    ),
    (
        "livestream_source",
        Wiring::OwnedBy("birdnet_web::routes::livestream (the station's default /stream source)"),
        SettingsCategory::Audio,
    ),
    (
        // `birdnet_behavioral::queries::EXCLUDE_IMPORTS_SETTING`, written from
        // `/admin/migration` and read by both analytics engines to decide
        // whether detections imported from a BirdNET-Pi count as this
        // station's own data.
        "analytics_exclude_imports",
        Wiring::OwnedBy("birdnet_behavioral::queries and the SQLite analytics reader"),
        SettingsCategory::System,
    ),
    // ── Station / location ─────────────────────────────────────────────────
    (
        "latitude",
        Wiring::Bridged("LATITUDE"),
        SettingsCategory::Location,
    ),
    (
        "longitude",
        Wiring::Bridged("LONGITUDE"),
        SettingsCategory::Location,
    ),
    (
        "station_name",
        Wiring::Bridged("STATION_NAME"),
        SettingsCategory::Location,
    ),
    // The recording window itself, not just its offsets. `capture::schedule`
    // read the CLI field directly until 0.12.0, so this key existed, was
    // validated, and was ignored — a station set to `solar` recorded all day.
    (
        "recording_schedule",
        Wiring::Bridged("RECORDING_SCHEDULE"),
        SettingsCategory::Location,
    ),
    // Written by the onboarding wizard's location auto-detect. Not bridged:
    // the station's clock is a *system* setting, and nothing in this process
    // runs as root or can change it. It is recorded so `--doctor` can compare
    // it against the host's actual timezone and hand the operator the one
    // command that fixes a mismatch — which matters because the system clock
    // is what names recording files, and those filenames become each
    // detection's Date and Time.
    (
        "timezone",
        Wiring::OwnedBy("crate::doctor::clock (system-timezone comparison)"),
        SettingsCategory::Location,
    ),
    // The first-run redirect's own flag: `pages::today` reads it straight from
    // the settings table to decide whether to bounce to `/onboarding`.
    (
        "onboarding_complete",
        Wiring::OwnedBy("birdnet_web::routes::pages::today (first-run redirect)"),
        SettingsCategory::System,
    ),
    (
        "night_inhibit",
        Wiring::Bridged("NIGHT_INHIBIT"),
        SettingsCategory::Location,
    ),
    (
        "pre_sunrise_offset",
        Wiring::Bridged("PRE_SUNRISE_OFFSET"),
        SettingsCategory::Location,
    ),
    (
        "post_sunset_offset",
        Wiring::Bridged("POST_SUNSET_OFFSET"),
        SettingsCategory::Location,
    ),
    (
        "site_name",
        Wiring::Bridged("SITENAME"),
        SettingsCategory::System,
    ),
    (
        "info_site",
        Wiring::Bridged("INFO_SITE"),
        SettingsCategory::System,
    ),
    // ── Notifications / integrations ───────────────────────────────────────
    //
    // These reach the runtime through the same config overlay: the Apprise and
    // BirdWeather constructors already fall back to these config keys, and
    // `overlay_db_settings` runs before they are built, so bridging the key is
    // all that is needed for a value typed in the UI to be the one that sends.
    (
        "apprise_url",
        Wiring::Bridged("APPRISE_URL"),
        SettingsCategory::Notifications,
    ),
    (
        "apprise_config",
        Wiring::Bridged("APPRISE_CONFIG_FILE"),
        SettingsCategory::Notifications,
    ),
    (
        "notify_urls",
        Wiring::Bridged("NOTIFY_URLS"),
        SettingsCategory::Notifications,
    ),
    (
        "birdweather_token",
        Wiring::Bridged("BIRDWEATHER_TOKEN"),
        SettingsCategory::Notifications,
    ),
    (
        "notify_confidence",
        Wiring::Bridged("APPRISE_MIN_CONFIDENCE"),
        SettingsCategory::Notifications,
    ),
    (
        "notify_cooldown",
        Wiring::Bridged("APPRISE_COOLDOWN"),
        SettingsCategory::Notifications,
    ),
    (
        "notify_trigger",
        Wiring::Bridged("APPRISE_TRIGGER"),
        SettingsCategory::Notifications,
    ),
    (
        "notify_species_only",
        Wiring::Bridged("APPRISE_WATCHLIST"),
        SettingsCategory::Notifications,
    ),
    (
        "notify_species_exclude",
        Wiring::Bridged("APPRISE_WATCHLIST_EXCLUDE"),
        SettingsCategory::Notifications,
    ),
    (
        "notify_title_template",
        Wiring::Bridged("APPRISE_TITLE_TEMPLATE"),
        SettingsCategory::Notifications,
    ),
    (
        "notify_body_template",
        Wiring::Bridged("APPRISE_BODY_TEMPLATE"),
        SettingsCategory::Notifications,
    ),
    (
        "weekly_report_schedule",
        Wiring::Bridged("WEEKLY_REPORT_SCHEDULE"),
        SettingsCategory::Notifications,
    ),
    (
        "heartbeat_url",
        Wiring::Bridged("HEARTBEAT_URL"),
        SettingsCategory::Notifications,
    ),
    (
        "deadman_hours",
        Wiring::Bridged("DEADMAN_HOURS"),
        SettingsCategory::Notifications,
    ),
    (
        "database_lang",
        Wiring::Bridged("DATABASE_LANG"),
        SettingsCategory::System,
    ),
    // ── Species filtering (consumed in `crate::daemon`) ────────────────────
    //
    // Read from the settings table directly rather than through the config: the
    // lists are multi-valued and the daemon reloads them, so round-tripping
    // them through a comma-joined config string would only lose fidelity.
    (
        "species_include",
        Wiring::OwnedBy("crate::daemon::config"),
        SettingsCategory::Species,
    ),
    (
        "species_exclude",
        Wiring::OwnedBy("crate::daemon::config"),
        SettingsCategory::Species,
    ),
    // ── System / disk management ───────────────────────────────────────────
    (
        "image_cache_dir",
        Wiring::Bridged("IMAGE_CACHE_DIR"),
        SettingsCategory::System,
    ),
    (
        "custom_image_dir",
        Wiring::Bridged("CUSTOM_IMAGE_DIR"),
        SettingsCategory::System,
    ),
    (
        "max_files_per_species",
        Wiring::Bridged("MAX_FILES_SPECIES"),
        SettingsCategory::System,
    ),
    (
        "purge_threshold",
        Wiring::Bridged("DISK_PURGE_THRESHOLD"),
        SettingsCategory::System,
    ),
    (
        "extraction_length",
        Wiring::Bridged("EXTRACTION_LENGTH"),
        SettingsCategory::System,
    ),
    (
        "raw_spectrogram",
        // Read straight from the settings table by the spectrogram route at
        // render time, so a change takes effect on the next image rather than
        // the next restart.
        Wiring::OwnedBy("the spectrogram renderer"),
        SettingsCategory::System,
    ),
    (
        "rare_species_days",
        Wiring::OwnedBy("the rare-species feeds"),
        SettingsCategory::System,
    ),
    (
        "stream_retention_secs",
        Wiring::Bridged("STREAM_RETENTION_SECS"),
        SettingsCategory::System,
    ),
    (
        "stream_max_mb",
        Wiring::Bridged("STREAM_MAX_MB"),
        SettingsCategory::System,
    ),
    (
        "clip_retention_days",
        Wiring::Bridged("CLIP_RETENTION_DAYS"),
        SettingsCategory::System,
    ),
    // ── Email alerts ───────────────────────────────────────────────────────
    //
    // `create_email_notifier` reads every one of these out of the settings
    // table itself, which is why they need no config key.
    (
        "email_smtp_host",
        Wiring::OwnedBy("crate::integrations::email"),
        SettingsCategory::Notifications,
    ),
    (
        "email_smtp_port",
        Wiring::OwnedBy("crate::integrations::email"),
        SettingsCategory::Notifications,
    ),
    (
        "email_smtp_user",
        Wiring::OwnedBy("crate::integrations::email"),
        SettingsCategory::Notifications,
    ),
    (
        "email_smtp_pass",
        Wiring::OwnedBy("crate::integrations::email"),
        SettingsCategory::Notifications,
    ),
    (
        "email_from",
        Wiring::OwnedBy("crate::integrations::email"),
        SettingsCategory::Notifications,
    ),
    (
        "email_to",
        Wiring::OwnedBy("crate::integrations::email"),
        SettingsCategory::Notifications,
    ),
    (
        "email_from_name",
        Wiring::OwnedBy("crate::integrations::email"),
        SettingsCategory::Notifications,
    ),
    (
        "email_starttls",
        Wiring::OwnedBy("crate::integrations::email"),
        SettingsCategory::Notifications,
    ),
    (
        "email_min_confidence",
        Wiring::OwnedBy("crate::integrations::email"),
        SettingsCategory::Notifications,
    ),
    (
        "email_cooldown_secs",
        Wiring::OwnedBy("crate::integrations::email"),
        SettingsCategory::Notifications,
    ),
];

/// Resolve the runtime config key a given admin-UI setting maps to, if any.
///
/// `None` for a key that is wired some other way (or not a settings key at all)
/// — only [`Wiring::Bridged`] entries flow through the config.
fn config_key_for(setting_key: &str) -> Option<&'static str> {
    SETTING_SPECS
        .iter()
        .find(|(ui, _, _)| *ui == setting_key)
        .and_then(|(_, wiring, _)| match wiring {
            Wiring::Bridged(config_key) => Some(*config_key),
            Wiring::OwnedBy(_) => None,
        })
}

/// Layer the given settings rows on top of the file config as overrides.
///
/// Pure (no I/O) so the mapping and merge precedence are unit-testable without
/// a database. Empty values are skipped so clearing a field falls back to the
/// file config rather than blanking a key. When the file config is absent but
/// overrides exist, a fresh [`Config`] is created to carry them.
fn apply_setting_overrides<'a, I>(config: Option<Config>, settings: I) -> Option<Config>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let mut merged = config;
    let mut applied = 0_usize;

    for (key, value) in settings {
        if value.trim().is_empty() {
            continue;
        }
        let Some(config_key) = config_key_for(key) else {
            continue;
        };
        merged
            .get_or_insert_with(Config::empty)
            .set(config_key, value);
        applied += 1;
    }

    if applied > 0 {
        tracing::info!(
            count = applied,
            "applied admin settings as runtime config overrides (database wins over the config file)"
        );
    }

    merged
}

/// Read the admin settings table and overlay its known values on the file
/// config, returning the merged configuration the rest of startup should use.
///
/// The database value wins over the config file. This is what makes the
/// settings form take effect: the confidence threshold, audio device and other
/// station/detection settings configured in the web UI are applied here before
/// the capture and detection subsystems are built.
#[must_use]
pub fn overlay_db_settings(config: Option<Config>, state: &AppState) -> Option<Config> {
    let rows = state.with_db(|conn| {
        // The table may not exist yet on a brand-new database; treat that (and
        // any read error) as "no overrides" rather than failing startup.
        ensure_settings_table(conn).ok();
        list(conn, None).unwrap_or_default()
    });

    let pairs: Vec<(String, String)> = rows.into_iter().map(|s| (s.key, s.value)).collect();
    apply_setting_overrides(config, pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())))
}

/// Compute the settings rows to seed from the file config.
///
/// Pure (no I/O) so the mapping is unit-testable without a database. For each
/// bridge spec, a row is produced only when the config carries a non-empty
/// value for the runtime key **and** the settings table does not already hold
/// the UI key — so an operator's later UI edits (which create the row) are
/// never overwritten, and re-running with the same config is a no-op.
fn settings_to_seed(
    config: &Config,
    existing: &HashSet<String>,
    from_secret_file: &[&str],
) -> Vec<(&'static str, String, SettingsCategory)> {
    let mut out = Vec::new();
    for &(ui_key, wiring, category) in SETTING_SPECS {
        if existing.contains(ui_key) {
            continue;
        }
        // Only bridged keys have a config key to seed *from*. A key its
        // subsystem reads straight out of the settings table has no file-config
        // counterpart, so there is nothing to copy across.
        let Wiring::Bridged(config_key) = wiring else {
            continue;
        };
        // A value that came from a mounted secret file is in the config only
        // because `resolve_secret_files` put it there. Copying it into the
        // settings table would write the credential into the database — and so
        // into every backup of it — which is the precise thing an operator
        // choosing `BIRDNET_<KEY>_FILE` was avoiding.
        if from_secret_file.contains(&config_key) {
            continue;
        }
        if let Some(value) = config.get(config_key) {
            let value = value.trim();
            if !value.is_empty() {
                out.push((ui_key, value.to_string(), category));
            }
        }
    }
    out
}

/// Trim a borrowed value and drop it if it is empty.
fn nonblank(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

/// Station settings the operator supplies via CLI flag or `BIRDNET_*`
/// environment variable rather than a config-file line — the Docker path, where
/// `docker run -e BIRDNET_LATITUDE=…` never touches `birdnet.conf`. Mapped to
/// the same admin-UI keys + categories as the file-config bridge so a container
/// configured purely through the environment seeds the same settings rows and
/// is no more "re-prompted" than a bare-metal install.
fn cli_station_settings(cli: &Cli) -> Vec<(&'static str, String, SettingsCategory)> {
    let mut out = Vec::new();
    if let Some(lat) = cli.latitude {
        out.push(("latitude", lat.to_string(), SettingsCategory::Location));
    }
    if let Some(lon) = cli.longitude {
        out.push(("longitude", lon.to_string(), SettingsCategory::Location));
    }
    if let Some(dev) = nonblank(cli.alsa_device.as_deref()) {
        out.push(("alsa_device", dev.to_string(), SettingsCategory::Audio));
    }
    if let Some(url) = nonblank(cli.rtsp_url.as_deref()) {
        out.push(("rtsp_url", url.to_string(), SettingsCategory::Audio));
    }
    if !cli.rtsp_urls.is_empty() {
        out.push((
            "rtsp_urls",
            cli.rtsp_urls.join(","),
            SettingsCategory::Audio,
        ));
    }
    out
}

/// Seed the admin-UI `settings` table from the station's installed
/// configuration on first run.
///
/// The operator supplies station settings (latitude/longitude, audio device,
/// station name, …) at install time: the bare-metal installer writes them to
/// `/etc/birdnet/birdnet.conf`, and the Docker image passes them as `BIRDNET_*`
/// environment variables (i.e. CLI flags). The admin settings form and the
/// first-run onboarding check, however, read **only** the SQLite `settings`
/// table. Without this bridge a freshly-installed station shows blank settings
/// fields and is bounced to the onboarding wizard even though it is already
/// fully configured — the "installation input is not respected" bug. This
/// copies each known, non-empty installed value into the settings table so it
/// appears in, and can be edited from, the web UI.
///
/// Both install paths are covered: file-config and CLI/env values are merged,
/// with the CLI/env value winning for a given key (matching how the daemon
/// resolves an input supplied both ways). Insert-only: a key that already has a
/// row is left untouched, so this never clobbers a setting the operator changed
/// in the UI, and it is safe to call on every startup. Returns the number of
/// rows seeded.
pub fn seed_db_settings_from_config(
    config: Option<&Config>,
    cli: &Cli,
    state: &AppState,
    from_secret_file: &[&str],
) -> usize {
    state.with_db(|conn| {
        // The table may not exist yet on a brand-new database; treat that (and
        // any read error) as "nothing already present" rather than failing.
        ensure_settings_table(conn).ok();
        let existing: HashSet<String> = list(conn, None)
            .map(|rows| rows.into_iter().map(|s| s.key).collect())
            .unwrap_or_default();

        // Merge the two install sources into one row-per-key set. File config
        // first; CLI/env then overrides for the same key. A BTreeMap keeps the
        // seed order (and the logged count) deterministic.
        let mut merged: BTreeMap<&'static str, (String, SettingsCategory)> = BTreeMap::new();
        if let Some(config) = config {
            for (key, value, category) in settings_to_seed(config, &existing, from_secret_file) {
                merged.insert(key, (value, category));
            }
        }
        for (key, value, category) in cli_station_settings(cli) {
            if !existing.contains(key) {
                merged.insert(key, (value, category));
            }
        }

        let mut seeded = 0_usize;
        for (key, (value, category)) in &merged {
            if set(conn, key, value, *category).is_ok() {
                seeded += 1;
            }
        }
        if seeded > 0 {
            tracing::info!(
                count = seeded,
                "seeded admin settings from the installed configuration (installer/env values are now editable at /admin/settings)"
            );
        }
        seeded
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use birdnet_web::routes::admin::settings::form::SETTINGS_FORM_KEYS;
    use std::collections::{BTreeMap, BTreeSet};

    /// Bridge keys that are deliberately *not* settings-form fields.
    ///
    /// These are config/env-only inputs that still need a seed+overlay mapping
    /// (so a Docker station configured purely through `BIRDNET_*` keeps working)
    /// but have no editable control. Anything else in [`SETTING_SPECS`] that is
    /// not a form key is a mistake — most likely a renamed field.
    ///
    /// The onboarding wizard's keys live here too: it persists settings the
    /// admin form does not expose (the first-run completion flag, the detected
    /// timezone), and they are legitimate rather than orphaned.
    const NON_FORM_BRIDGE_KEYS: &[&str] = &[
        "alsa_devices",
        "timezone",
        "onboarding_complete",
        // Written from `/admin/audio`, where the source it names is chosen from
        // a list of real rows rather than typed as an id. Covered by
        // `audio_admin_keys_are_all_classified` below.
        "livestream_source",
        // Written from `/admin/migration`, a page with its own controls rather
        // than a settings form. Found by
        // `every_settings_key_written_anywhere_is_classified`, which is the
        // gate that exists because a per-writer list keeps missing writers.
        "analytics_exclude_imports",
    ];

    #[test]
    fn settings_form_keys_are_all_classified() {
        // The guard-rail. Every key the admin form can persist must say how it
        // reaches the runtime. Adding a form field without classifying it here
        // fails, instead of silently shipping another editable control that
        // does nothing — the defect this whole mapping exists to prevent, and
        // which had accumulated twenty instances before it was enforced.
        let classified: BTreeSet<&str> = SETTING_SPECS.iter().map(|(ui, _, _)| *ui).collect();
        let unclassified: Vec<&str> = SETTINGS_FORM_KEYS
            .iter()
            .copied()
            .filter(|key| !classified.contains(key))
            .collect();

        assert!(
            unclassified.is_empty(),
            "settings-form keys with no wiring classification: {unclassified:?}\n\
             Add each to SETTING_SPECS as Wiring::Bridged(config key) or \
             Wiring::OwnedBy(subsystem), or remove the form field."
        );
    }

    /// The same guard, for the *other* place settings get written.
    ///
    /// The admin form has been guarded since twenty of its fields turned out to
    /// be inert, but the first-run wizard writes its own keys and was never
    /// covered — so it shipped `notification_mode`, a four-way choice of how
    /// often to be alerted that no code anywhere read. A non-technical operator
    /// picked one on their first day and it governed nothing.
    #[test]
    fn onboarding_wizard_keys_are_all_classified() {
        use birdnet_web::routes::pages::onboarding::ONBOARDING_SETTING_KEYS;

        let classified: BTreeSet<&str> = SETTING_SPECS.iter().map(|(ui, _, _)| *ui).collect();
        let unclassified: Vec<&str> = ONBOARDING_SETTING_KEYS
            .iter()
            .copied()
            .filter(|key| !classified.contains(key))
            .collect();

        assert!(
            unclassified.is_empty(),
            "onboarding-wizard keys with no wiring classification: {unclassified:?}\n\
             The setup wizard must not persist a setting nothing reads. Add each \
             to SETTING_SPECS as Wiring::Bridged(config key) or \
             Wiring::OwnedBy(subsystem), or stop writing it."
        );
    }

    /// And for the third place settings get written.
    ///
    /// `/admin/audio` writes `livestream_source` — the source `/stream` serves
    /// when the listener has not chosen one — because the value is an
    /// `audio_sources` row id, which belongs beside the rows and not in a text
    /// field on the settings page. That made the audio page a settings writer
    /// with no membership of the guard the other two writers have, which is
    /// how a key nothing reads gets shipped.
    #[test]
    fn audio_admin_keys_are_all_classified() {
        use birdnet_web::routes::admin::audio::AUDIO_ADMIN_SETTING_KEYS;

        let classified: BTreeSet<&str> = SETTING_SPECS.iter().map(|(ui, _, _)| *ui).collect();
        let unclassified: Vec<&str> = AUDIO_ADMIN_SETTING_KEYS
            .iter()
            .copied()
            .filter(|key| !classified.contains(key))
            .collect();

        assert!(
            unclassified.is_empty(),
            "audio-admin keys with no wiring classification: {unclassified:?}\n\
             Add each to SETTING_SPECS as Wiring::Bridged(config key) or \
             Wiring::OwnedBy(subsystem), or stop writing it."
        );
    }

    // ── the general form of the same guard (G-34) ───────────────────────────
    //
    // The three gates above each cover one *writer*, by name, from a list
    // maintained by hand — the admin form, the wizard, the audio page. A fourth
    // writer is outside all of them, which is what happened:
    // `routes/admin/migration.rs` writes `exclude_imports` and nothing was
    // checking that anything read it.
    //
    // These two read the source instead, so a new writer anywhere in the
    // workspace is covered the moment it is added, and a subsystem that stops
    // reading a key it claims to own is caught from the other side.

    use std::path::{Path, PathBuf};

    /// Every `.rs` file under the workspace's production source trees.
    fn production_sources() -> Vec<PathBuf> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut roots = vec![root.join("src")];
        for entry in std::fs::read_dir(root.join("crates")).expect("crates/") {
            let dir = entry.expect("entry").path();
            if dir.join("src").is_dir() {
                roots.push(dir.join("src"));
            }
        }
        let mut files = Vec::new();
        for r in roots {
            walk_rs(&r, &mut files);
        }
        files.sort();
        files
    }

    fn walk_rs(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("read_dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                walk_rs(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs")
                && path.file_name().is_none_or(|n| n != "tests.rs")
            {
                out.push(path);
            }
        }
    }

    /// The source with every `#[cfg(test)]` item removed by brace depth, and
    /// comment lines dropped so prose about a write is not a write.
    ///
    /// Without it the scan reads every fixture's
    /// `settings::set(conn, "latitude", "51.5074", …)` as a production writer.
    fn production_half(src: &str) -> String {
        let mut out = String::with_capacity(src.len());
        let mut lines = src.lines();
        while let Some(line) = lines.next() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            if line.trim_start().starts_with("#[cfg(test)]") {
                let mut depth: i32 = 0;
                let mut seen_open = false;
                for l in lines.by_ref() {
                    depth += i32::try_from(l.matches('{').count()).unwrap_or(0);
                    depth -= i32::try_from(l.matches('}').count()).unwrap_or(0);
                    if l.contains('{') {
                        seen_open = true;
                    }
                    if l.trim_start().starts_with('#') && !seen_open {
                        continue;
                    }
                    if (seen_open && depth <= 0) || (!seen_open && l.trim_end().ends_with(';')) {
                        break;
                    }
                }
                continue;
            }
            out.push_str(line);
            out.push('\n');
        }
        out
    }

    /// `const NAME: &str = "value";` across the workspace, so a key written as
    /// a named constant resolves to the string it holds.
    fn string_constants(sources: &[(PathBuf, String)]) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        for (_, src) in sources {
            for line in src.lines() {
                let line = line.trim();
                let Some(rest) = line
                    .strip_prefix("pub const ")
                    .or_else(|| line.strip_prefix("const "))
                    .or_else(|| line.strip_prefix("pub(crate) const "))
                    .or_else(|| line.strip_prefix("pub(super) const "))
                else {
                    continue;
                };
                let Some((name, tail)) = rest.split_once(':') else {
                    continue;
                };
                if !tail.trim_start().starts_with("&str") {
                    continue;
                }
                let Some(open) = tail.find('"') else { continue };
                let after = &tail[open + 1..];
                let Some(close) = after.find('"') else {
                    continue;
                };
                out.insert(name.trim().to_owned(), after[..close].to_owned());
            }
        }
        out
    }

    /// The argument after the connection in a `settings::` call: a string
    /// literal, or a path ending in a constant's name.
    ///
    /// `rest` starts immediately after the opening parenthesis. `None` means
    /// the key is a runtime value, which the caller reports rather than drops.
    fn key_after_conn(rest: &str, consts: &BTreeMap<String, String>) -> Option<String> {
        // Calls are formatted across lines; a bounded window keeps a failed
        // match from running into the next call.
        let window = &rest[..rest.len().min(400)];
        let comma = window.find(',')?;
        let tail = window[comma + 1..].trim_start();
        if let Some(quoted) = tail.strip_prefix('"') {
            let end = quoted.find('"')?;
            return Some(quoted[..end].to_owned());
        }
        let ident: String = tail
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == ':')
            .collect();
        let last = ident.rsplit("::").next()?.to_owned();
        consts.get(&last).cloned()
    }

    /// Keys written through `settings::set`, and the sites whose key is built
    /// at runtime.
    fn keys_written(
        sources: &[(PathBuf, String)],
        consts: &BTreeMap<String, String>,
    ) -> (BTreeSet<String>, Vec<String>) {
        let mut keys = BTreeSet::new();
        let mut unresolved = Vec::new();
        for (path, src) in sources {
            for (idx, _) in src.match_indices("settings::set(") {
                let rest = &src[idx + "settings::set(".len()..];
                match key_after_conn(rest, consts) {
                    Some(key) => {
                        keys.insert(key);
                    }
                    None => unresolved.push(path.display().to_string()),
                }
            }
            // `set_many` takes a slice built at runtime; its keys are bounded
            // where the slice is built, and DYNAMIC_WRITE_SITES names where.
            if src.contains("settings::set_many(") {
                unresolved.push(format!("{} (set_many)", path.display()));
            }
        }
        unresolved.sort();
        unresolved.dedup();
        (keys, unresolved)
    }

    /// Keys read in production code, through any shape this scan knows.
    fn keys_read(
        sources: &[(PathBuf, String)],
        consts: &BTreeMap<String, String>,
    ) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for (_, src) in sources {
            for shape in [
                "settings::get(",
                "settings::get_or(",
                "settings::get_parsed(",
            ] {
                for (idx, _) in src.match_indices(shape) {
                    if let Some(key) = key_after_conn(&src[idx + shape.len()..], consts) {
                        out.insert(key);
                    }
                }
            }
            // `get_parsed::<T>(conn, "KEY")` — the turbofish sits between the
            // name and the parenthesis.
            for (idx, _) in src.match_indices("settings::get_parsed::<") {
                let rest = &src[idx..];
                if let Some(open) = rest.find('(')
                    && let Some(key) = key_after_conn(&rest[open + 1..], consts)
                {
                    out.insert(key);
                }
            }
            // `--doctor` opens its own read-only connection and reads through
            // a helper of its own, so the key is the second argument there too
            // but the receiver is a `Config`, not a connection.
            for (idx, _) in src.match_indices("setting_from_db(config, \"") {
                let rest = &src[idx + "setting_from_db(config, \"".len()..];
                if let Some(end) = rest.find('"') {
                    out.insert(rest[..end].to_owned());
                }
            }
            // The admin pages read an already-loaded map, so the key is the
            // first argument after it rather than after a connection.
            for (idx, _) in src.match_indices("get_setting(s, \"") {
                let rest = &src[idx + "get_setting(s, \"".len()..];
                if let Some(end) = rest.find('"') {
                    out.insert(rest[..end].to_owned());
                }
            }
        }
        out
    }

    /// Call sites whose settings keys are bounded elsewhere, with where.
    ///
    /// Each builds a `(key, value, category)` slice at runtime, so the keys
    /// cannot be read out of the call — and re-deriving them here would only
    /// duplicate a check that already exists. A *new* file using `set_many`
    /// trips the gate and has to be added deliberately.
    const DYNAMIC_WRITE_SITES: &[&str] = &[
        // Validated against `SETTINGS_FORM_KEYS` before the write; an unknown
        // key is refused with a 400.
        "routes/api_write.rs",
        // The wizard's own keys, pinned by `ONBOARDING_SETTING_KEYS` and the
        // gate above.
        "routes/pages/onboarding.rs",
        // The admin settings form's own keys, pinned by `SETTINGS_FORM_KEYS`
        // and the two gates in `routes/admin/settings/`.
        "routes/admin/settings/handler.rs",
    ];

    fn scanned_sources() -> Vec<(PathBuf, String)> {
        production_sources()
            .into_iter()
            .map(|p| {
                let src = std::fs::read_to_string(&p).expect("read");
                let half = production_half(&src);
                (p, half)
            })
            .collect()
    }

    /// Every settings key production code writes is classified in
    /// [`SETTING_SPECS`].
    #[test]
    fn every_settings_key_written_anywhere_is_classified() {
        let sources = scanned_sources();
        let consts = string_constants(&sources);
        let (written, unresolved) = keys_written(&sources, &consts);

        assert!(
            written.len() >= 3,
            "the scan found only {written:?} — it has stopped recognising the write \
             shape and would pass vacuously"
        );

        let classified: BTreeSet<&str> = SETTING_SPECS.iter().map(|(ui, _, _)| *ui).collect();
        let unwired: Vec<&String> = written
            .iter()
            .filter(|k| !classified.contains(k.as_str()))
            .collect();
        assert!(
            unwired.is_empty(),
            "these settings keys are written by production code and classified nowhere, \
             so nothing says what reads them: {unwired:?}\n\
             Add each to SETTING_SPECS as Wiring::Bridged(config key) or \
             Wiring::OwnedBy(subsystem), or stop writing it."
        );

        let unexpected: Vec<&String> = unresolved
            .iter()
            .filter(|site| !DYNAMIC_WRITE_SITES.iter().any(|a| site.contains(a)))
            .collect();
        assert!(
            unexpected.is_empty(),
            "these call sites write a settings key this scan cannot resolve to a \
             literal, and are not on the allowlist: {unexpected:?}\n\
             Write the key as a literal or a `const NAME: &str`, or add the site to \
             DYNAMIC_WRITE_SITES naming what bounds its keys."
        );
    }

    /// The other direction: a key classified as read straight out of the
    /// settings table by a named subsystem is read somewhere.
    ///
    /// `Wiring::Bridged` keys are excluded — those are read through the
    /// *config*, which `tests/every_config_key_is_known.rs` checks.
    #[test]
    fn every_subsystem_owned_setting_is_read_somewhere() {
        let sources = scanned_sources();
        let consts = string_constants(&sources);
        let read = keys_read(&sources, &consts);

        assert!(
            read.len() >= 5,
            "the scan found only {read:?} — it has stopped recognising the read shapes"
        );

        let unread: Vec<&str> = SETTING_SPECS
            .iter()
            .filter(|(_, wiring, _)| matches!(wiring, Wiring::OwnedBy(_)))
            .map(|(ui, _, _)| *ui)
            .filter(|k| !read.contains(*k))
            .collect();
        assert!(
            unread.is_empty(),
            "these keys are classified as read straight out of the settings table by a \
             named subsystem, and no read of them appears in the source: {unread:?}\n\
             Either the subsystem stopped reading it — in which case the control is \
             inert, which is the bug this classification exists to prevent — or it \
             reads it through a shape this scan does not know, in which case teach the \
             scan."
        );
    }

    /// The scan itself, against a synthetic source.
    ///
    /// Without this, a scan that had stopped matching anything would satisfy
    /// both gates above by finding nothing to complain about.
    #[test]
    fn the_settings_scan_recognises_the_shapes() {
        let src = concat!(
            "pub const A_KEY: &str = \"from_a_const\";\n",
            "fn w(conn: &C) {\n",
            "    settings::set(conn, \"a_literal\", \"v\", Cat::System);\n",
            "    birdnet_db::settings::set(\n        conn,\n        A_KEY,\n        \"v\",\n    );\n",
            "}\n",
            "fn r(conn: &C) {\n",
            "    let _ = settings::get_or(conn, \"read_or\", \"d\");\n",
            "    let _ = settings::get(conn, A_KEY);\n",
            "    let _ = get_setting(s, \"read_map\", \"d\");\n",
            "}\n",
            "#[cfg(test)]\nmod tests {\n    fn t(conn: &C) {\n",
            "        settings::set(conn, \"test_only\", \"v\", Cat::System);\n    }\n}\n",
        );
        let sources = vec![(PathBuf::from("synthetic.rs"), production_half(src))];
        let consts = string_constants(&sources);
        assert_eq!(
            consts.get("A_KEY").map(String::as_str),
            Some("from_a_const")
        );

        let (written, unresolved) = keys_written(&sources, &consts);
        assert!(written.contains("a_literal"), "{written:?}");
        assert!(
            written.contains("from_a_const"),
            "a key given as a constant must resolve: {written:?}"
        );
        assert!(
            !written.contains("test_only"),
            "a write inside #[cfg(test)] is a fixture, not a production writer: {written:?}"
        );
        assert!(unresolved.is_empty(), "{unresolved:?}");

        let read = keys_read(&sources, &consts);
        assert!(read.contains("read_or"), "{read:?}");
        assert!(read.contains("from_a_const"), "{read:?}");
        assert!(read.contains("read_map"), "{read:?}");
    }

    #[test]
    fn every_bridge_spec_is_a_real_form_key() {
        // The reverse direction: a spec for a key the form no longer has is
        // dead weight that reads as coverage. Renaming a field trips this.
        let form: BTreeSet<&str> = SETTINGS_FORM_KEYS.iter().copied().collect();
        let allowed: BTreeSet<&str> = NON_FORM_BRIDGE_KEYS.iter().copied().collect();
        let orphans: Vec<&str> = SETTING_SPECS
            .iter()
            .map(|(ui, _, _)| *ui)
            .filter(|key| !form.contains(key) && !allowed.contains(key))
            .collect();

        assert!(
            orphans.is_empty(),
            "SETTING_SPECS entries that are not settings-form keys: {orphans:?}"
        );
    }

    #[test]
    fn no_duplicate_bridge_specs() {
        // A duplicated UI key makes `config_key_for` depend on list order,
        // which would silently pick one of two mappings.
        let unique: BTreeSet<&str> = SETTING_SPECS.iter().map(|(ui, _, _)| *ui).collect();
        assert_eq!(unique.len(), SETTING_SPECS.len(), "duplicate UI key");

        // Two UI keys writing the same config key would have them clobber each
        // other through the overlay.
        let config_keys: Vec<&str> = SETTING_SPECS
            .iter()
            .filter_map(|(_, w, _)| match w {
                Wiring::Bridged(k) => Some(*k),
                Wiring::OwnedBy(_) => None,
            })
            .collect();
        let unique_config: BTreeSet<&str> = config_keys.iter().copied().collect();
        assert_eq!(
            unique_config.len(),
            config_keys.len(),
            "two settings keys map to the same config key"
        );
    }

    #[test]
    fn subsystem_owned_keys_do_not_flow_through_the_config() {
        // `OwnedBy` means "this subsystem reads the settings table itself", so
        // the overlay must not also invent a config key for it — that would be
        // two sources of truth for one value.
        assert_eq!(config_key_for("email_smtp_host"), None);
        assert_eq!(config_key_for("species_include"), None);
    }

    #[test]
    fn removed_credential_keys_are_not_classified() {
        // The admin credential is an Argon2id hash in the accounts table. If
        // these ever come back as settings keys, they come back as plaintext.
        let classified: BTreeSet<&str> = SETTING_SPECS.iter().map(|(ui, _, _)| *ui).collect();
        assert!(!classified.contains("auth_password"));
        assert!(!classified.contains("auth_username"));
        assert!(!SETTINGS_FORM_KEYS.contains(&"auth_password"));
        assert!(!SETTINGS_FORM_KEYS.contains(&"auth_username"));
    }

    #[test]
    fn maps_known_detection_keys() {
        assert_eq!(config_key_for("confidence_threshold"), Some("CONFIDENCE"));
        assert_eq!(config_key_for("sensitivity"), Some("SENSITIVITY"));
        assert_eq!(config_key_for("alsa_device"), Some("ALSA_CARD"));
        assert_eq!(config_key_for("alsa_devices"), Some("ALSA_CARDS"));
    }

    #[test]
    fn unknown_key_maps_to_nothing() {
        assert_eq!(config_key_for("totally_unknown_setting"), None);
        // The removed credential fields must never map to anything.
        assert_eq!(config_key_for("auth_password"), None);
    }

    #[test]
    fn notification_keys_are_bridged() {
        // These used to map to nothing, which is precisely why an Apprise URL
        // or BirdWeather token entered in the web UI was stored and then
        // ignored: the constructors read `APPRISE_URL` / `BIRDWEATHER_TOKEN`
        // from the config, and nothing put the saved value there.
        assert_eq!(config_key_for("apprise_url"), Some("APPRISE_URL"));
        assert_eq!(
            config_key_for("birdweather_token"),
            Some("BIRDWEATHER_TOKEN")
        );
        assert_eq!(
            config_key_for("notify_confidence"),
            Some("APPRISE_MIN_CONFIDENCE")
        );
        assert_eq!(config_key_for("notify_trigger"), Some("APPRISE_TRIGGER"));
    }

    #[test]
    fn newly_bridged_capture_keys_reach_the_config() {
        assert_eq!(config_key_for("segment_duration"), Some("SEGMENT_DURATION"));
        assert_eq!(config_key_for("freq_shift_hz"), Some("FREQ_SHIFT"));
        assert_eq!(config_key_for("night_inhibit"), Some("NIGHT_INHIBIT"));
        assert_eq!(config_key_for("rtsp_urls"), Some("RTSP_URLS"));
        assert_eq!(
            config_key_for("pre_sunrise_offset"),
            Some("PRE_SUNRISE_OFFSET")
        );
        assert_eq!(
            config_key_for("post_sunset_offset"),
            Some("POST_SUNSET_OFFSET")
        );
    }

    #[test]
    fn override_wins_over_file_config() {
        let file = Config::parse("CONFIDENCE=0.25\nSENSITIVITY=1.0").unwrap();
        let merged =
            apply_setting_overrides(Some(file), [("confidence_threshold", "0.8")]).unwrap();
        assert_eq!(merged.get("CONFIDENCE"), Some("0.8"));
        // Untouched file values are preserved.
        assert_eq!(merged.get("SENSITIVITY"), Some("1.0"));
    }

    #[test]
    fn wizard_written_confidence_reaches_the_daemon() {
        // The full chain the onboarding wizard starts: it writes the
        // `confidence_threshold` setting, the overlay maps it onto CONFIDENCE,
        // and the daemon must then enforce *that* value rather than its
        // default. Asserting only the mapping would leave the last hop —
        // the one that actually decides whether a bird is recorded — untested.
        for (written, expected) in [("0.5", 0.5_f32), ("0.85", 0.85), ("0.7", 0.7)] {
            let merged =
                apply_setting_overrides(None, [("confidence_threshold", written)]).unwrap();
            let enforced = crate::daemon::resolve_confidence(Some(&merged));
            assert!(
                (enforced - expected).abs() < f32::EPSILON,
                "wizard wrote {written}, daemon would enforce {enforced}"
            );
        }
    }

    /// Parity keys added in 0.12.0. Each was command-line-only, so an operator
    /// without a terminal could not reach it at all — and `recording_schedule`
    /// was worse than unreachable: the runtime ignored the config key outright,
    /// so a station set to `solar` recorded around the clock.
    ///
    /// This asserts the whole chain per key: the settings row the form writes,
    /// through the overlay, to the config key the consumer actually reads.
    #[test]
    fn parity_settings_reach_their_config_keys() {
        for (ui_key, value, config_key) in [
            ("recording_schedule", "solar", "RECORDING_SCHEDULE"),
            ("heartbeat_url", "https://hc-ping.com/abc", "HEARTBEAT_URL"),
            ("deadman_hours", "6", "DEADMAN_HOURS"),
            ("database_lang", "de", "DATABASE_LANG"),
        ] {
            let merged = apply_setting_overrides(None, [(ui_key, value)])
                .unwrap_or_else(|| panic!("{ui_key} produced no config"));
            assert_eq!(
                merged.get(config_key),
                Some(value),
                "{ui_key} must land on {config_key}"
            );
        }
    }

    /// And the schedule's last hop, which is the one that was broken: the
    /// capture supervisor must build a solar window from the overlaid config.
    #[test]
    fn a_settings_page_schedule_reaches_the_capture_supervisor() {
        let merged = apply_setting_overrides(
            None,
            [
                ("recording_schedule", "solar"),
                ("latitude", "52.5"),
                ("longitude", "13.4"),
            ],
        )
        .expect("overlay applies");
        let cli = crate::helpers::test_support::default_cli();
        let sc = crate::capture::schedule_config_for_test(&cli, Some(&merged));
        assert!(
            sc.night_inhibit,
            "choosing Solar on the settings page must actually stop overnight recording"
        );
    }

    #[test]
    fn empty_value_does_not_override() {
        let file = Config::parse("ALSA_CARD=plughw:1,0").unwrap();
        let merged = apply_setting_overrides(Some(file), [("alsa_device", "  ")]).unwrap();
        // Blank UI field falls back to the file config rather than blanking it.
        assert_eq!(merged.get("ALSA_CARD"), Some("plughw:1,0"));
    }

    #[test]
    fn unknown_keys_are_ignored_in_merge() {
        let merged = apply_setting_overrides(
            None,
            [
                ("totally_unknown_setting", "x"),
                ("confidence_threshold", "0.7"),
            ],
        )
        .expect("a known override should produce a config");
        assert_eq!(merged.get("CONFIDENCE"), Some("0.7"));
        assert_eq!(merged.get("totally_unknown_setting"), None);
    }

    #[test]
    fn creates_config_when_file_absent_but_overrides_present() {
        let merged = apply_setting_overrides(None, [("confidence_threshold", "0.9")])
            .expect("overrides with no file config should still produce a config");
        assert_eq!(merged.get("CONFIDENCE"), Some("0.9"));
    }

    #[test]
    fn no_overrides_preserves_none() {
        assert!(apply_setting_overrides(None, []).is_none());
    }

    #[test]
    fn no_known_overrides_preserves_original_config() {
        let file = Config::parse("CONFIDENCE=0.42").unwrap();
        let merged = apply_setting_overrides(Some(file), [("unknown", "v")]).unwrap();
        assert_eq!(merged.get("CONFIDENCE"), Some("0.42"));
    }

    #[test]
    fn seeds_known_keys_from_config_with_categories() {
        // The installer-written file config is copied into the settings table
        // under the UI keys + categories the admin form reads.
        let config =
            Config::parse("LATITUDE=42.36\nRTSP_URL=rtsp://cam/stream\nCONFIDENCE=0.7").unwrap();
        let seed = settings_to_seed(&config, &HashSet::new(), &[]);

        let lat = seed
            .iter()
            .find(|(k, _, _)| *k == "latitude")
            .expect("latitude seeded");
        assert_eq!(lat.1, "42.36");
        assert_eq!(lat.2, SettingsCategory::Location);

        let rtsp = seed
            .iter()
            .find(|(k, _, _)| *k == "rtsp_url")
            .expect("rtsp_url seeded");
        assert_eq!(rtsp.1, "rtsp://cam/stream");
        assert_eq!(rtsp.2, SettingsCategory::Audio);

        let conf = seed
            .iter()
            .find(|(k, _, _)| *k == "confidence_threshold")
            .expect("confidence seeded");
        assert_eq!(conf.2, SettingsCategory::Detection);
    }

    #[test]
    fn seed_skips_keys_already_present() {
        // A key that already has a row (an operator edit made in the UI) is
        // never re-seeded, so seeding can run on every startup without
        // clobbering changes.
        let config = Config::parse("LATITUDE=42.36\nLONGITUDE=-71.06").unwrap();
        let mut existing = HashSet::new();
        existing.insert("latitude".to_string());
        let seed = settings_to_seed(&config, &existing, &[]);
        assert!(!seed.iter().any(|(k, _, _)| *k == "latitude"));
        assert!(seed.iter().any(|(k, _, _)| *k == "longitude"));
    }

    #[test]
    fn seed_skips_empty_values_and_unmapped_keys() {
        let mut config = Config::empty();
        config.set("ALSA_CARD", ""); // installer skipped → empty, must not seed
        config.set("SOME_UNMAPPED_KEY", "x"); // not a bridge key → never seeded
        config.set("LATITUDE", "51.5");
        let seed = settings_to_seed(&config, &HashSet::new(), &[]);
        assert!(!seed.iter().any(|(k, _, _)| *k == "alsa_device"));
        assert!(seed.iter().any(|(k, _, _)| *k == "latitude"));
        // Only mapped UI keys are ever produced.
        assert!(seed.iter().all(|(k, _, _)| config_key_for(k).is_some()));
    }

    /// A credential that came from a mounted file is not copied into the
    /// settings table.
    ///
    /// The seed's whole job is to make an installed value editable in the web
    /// UI, and for every ordinary setting that is right. For a secret the
    /// operator deliberately mounted as a file it is the opposite: writing it
    /// into the `settings` table puts it in the database, and so in every
    /// backup, restore bundle and support archive taken from it — which is the
    /// exact exposure `BIRDNET_<KEY>_FILE` exists to avoid.
    #[test]
    fn a_credential_read_from_a_file_is_not_seeded_into_the_database() {
        let config = Config::parse("NOTIFY_URLS=ntfy://ntfy.sh/garden\nLATITUDE=51.5").unwrap();

        let seeded_normally = settings_to_seed(&config, &HashSet::new(), &[]);
        assert!(
            seeded_normally.iter().any(|(k, _, _)| *k == "notify_urls"),
            "the counterpart: a value that did NOT come from a file is still seeded, so \
             the assertion below is about the exclusion and not about the key having \
             stopped being seeded at all"
        );

        let seeded_from_file = settings_to_seed(&config, &HashSet::new(), &["NOTIFY_URLS"]);
        assert!(
            !seeded_from_file.iter().any(|(k, _, _)| *k == "notify_urls"),
            "a file-supplied credential must not reach the settings table"
        );
        assert!(
            seeded_from_file.iter().any(|(k, _, _)| *k == "latitude"),
            "and the exclusion must be per key, not a switch that stops seeding"
        );
    }

    #[test]
    fn seed_and_overlay_are_inverse_through_the_same_mapping() {
        // Round-trip invariant: a value seeded under a UI key overlays back onto
        // exactly the config key it was read from.
        let config = Config::parse("LATITUDE=12.34").unwrap();
        let seed = settings_to_seed(&config, &HashSet::new(), &[]);
        let (ui_key, value, _) = seed
            .iter()
            .find(|(k, _, _)| *k == "latitude")
            .expect("latitude seeded");
        let merged = apply_setting_overrides(None, [(*ui_key, value.as_str())]).unwrap();
        assert_eq!(merged.get("LATITUDE"), Some("12.34"));
    }

    #[test]
    fn cli_station_settings_maps_provided_flags() {
        // The Docker path: settings arrive as flags / BIRDNET_* env, not a file.
        let mut cli = crate::helpers::test_support::default_cli();
        cli.latitude = Some(42.36);
        cli.longitude = Some(-71.06);
        cli.alsa_device = Some("plughw:1,0".to_string());
        cli.rtsp_url = Some("   ".to_string()); // blank → must be skipped
        cli.rtsp_urls = vec!["rtsp://a".to_string(), "rtsp://b".to_string()];
        let seeds = cli_station_settings(&cli);

        let get = |k: &str| {
            seeds
                .iter()
                .find(|(key, _, _)| *key == k)
                .map(|(_, v, c)| (v.clone(), *c))
        };
        assert_eq!(
            get("latitude"),
            Some(("42.36".to_string(), SettingsCategory::Location))
        );
        assert_eq!(
            get("longitude"),
            Some(("-71.06".to_string(), SettingsCategory::Location))
        );
        assert_eq!(
            get("alsa_device"),
            Some(("plughw:1,0".to_string(), SettingsCategory::Audio))
        );
        assert!(get("rtsp_url").is_none(), "blank rtsp_url must not seed");
        // rtsp_urls joins on comma — the delimiter the settings form expects.
        assert_eq!(
            get("rtsp_urls"),
            Some(("rtsp://a,rtsp://b".to_string(), SettingsCategory::Audio))
        );
    }

    #[test]
    fn cli_station_settings_empty_when_nothing_supplied() {
        let cli = crate::helpers::test_support::default_cli();
        assert!(cli_station_settings(&cli).is_empty());
    }
}
