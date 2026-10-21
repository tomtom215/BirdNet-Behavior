//! Settings that were reachable only from `birdnet.conf` or a command-line flag,
//! now with a place on the settings pages.
//!
//! # Why a table
//!
//! The first sixty settings each have a hand-written field in the form struct,
//! a hand-written arm in the save, a hand-written control and a hand-written
//! check. That is the right amount of ceremony for the settings everyone
//! touches and the wrong amount for twenty-seven that one person in fifty needs
//! (MQTT for Home Assistant, a duplicate-suppression window, the eBird key).
//! Left at that price they stayed in the file, and "never needs a terminal"
//! was false for exactly the people with the most elaborate stations.
//!
//! Each [`Extra`] here says what the setting is called in the settings table
//! (`key`), what the station reads it as (`config_key`, which the binary's
//! settings bridge uses), how it is edited ([`Kind`]) and what it means in
//! plain words. One renderer draws them, one check validates them, and one
//! function turns a submission into rows. Everything that was true of the
//! hand-written fields stays true of these because they go through the same
//! save: only what changed is written, a bad value refuses the whole
//! submission, and the page says when each applies (every extra is read when
//! its consumer is built at start, so each is *after restart*).
//!
//! The binary adds these to its bridge table
//! (`helpers::settings_overlay::all_specs`) and a test holds [`EXTRA_SETTINGS`]
//! equal to what the bridge has, so a row cannot be added here and left unread.

use std::collections::HashMap;
use std::fmt::Write as _;

use birdnet_db::settings::SettingsCategory;

use crate::routes::pages::escape_html;

/// How a setting is edited and what it may hold.
#[derive(Debug, Clone, Copy)]
pub enum Kind {
    /// Free text, at most 200 characters, no control characters.
    Text {
        /// Shown in the empty box.
        placeholder: &'static str,
    },
    /// Text that is a credential: drawn as a password box, never echoed to a
    /// viewer (the settings API and the forms mask it by its name).
    Secret,
    /// A whole number in `min..=max`.
    Int {
        /// Smallest accepted.
        min: i64,
        /// Largest accepted.
        max: i64,
    },
    /// A decimal number in `min..=max`; `,` is read as the decimal point.
    Float {
        /// Smallest accepted.
        min: f64,
        /// Largest accepted.
        max: f64,
    },
    /// One of a short list, as `(stored value, words shown)`.
    Choice(&'static [(&'static str, &'static str)]),
}

/// The pages' named groups of extras. Each is a card on its own tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    /// Home Assistant and any MQTT broker.
    Mqtt,
    /// How loud the station may be.
    AlertLimits,
    /// Which detections are kept.
    Filters,
    /// How often the database is backed up.
    Backups,
    /// How much audio is kept, and what is never thrown away.
    Storage,
    /// Services on the internet the station can use.
    Online,
}

impl Group {
    /// The anchor id (`set-<id>`) and heading of the group's card.
    #[must_use]
    pub const fn card(self) -> (&'static str, &'static str) {
        match self {
            Self::Mqtt => ("mqtt", "Home Assistant &amp; MQTT"),
            Self::AlertLimits => ("alert-limits", "Alert limits"),
            Self::Filters => ("filters", "Detection filters"),
            Self::Backups => ("backups", "Backups"),
            Self::Storage => ("storage", "Storage"),
            Self::Online => ("online", "Online services"),
        }
    }

    /// One sentence under the heading.
    #[must_use]
    pub const fn blurb(self) -> &'static str {
        match self {
            Self::Mqtt => {
                "Publish every detection to an MQTT broker, such as the Mosquitto add-on of Home Assistant, so automations can react to a bird. Leave the address empty to switch it off."
            }
            Self::AlertLimits => {
                "Keep alerts useful: cap how many a destination gets, and say whether the station may tell you about its own faults."
            }
            Self::Filters => {
                "Optional filters that drop or hold back detections that are probably not what they seem. All are off until you turn them on, because they change how many rows the station records."
            }
            Self::Backups => {
                "The station keeps a copy of its database beside it. How often it makes one."
            }
            Self::Storage => {
                "What happens to audio when the disk fills, and how much raw audio is kept for checking."
            }
            Self::Online => {
                "Services on the internet the station can use, and who else may show it. Nothing is contacted until you give the station a key, and no other site can put the station in a frame until you list it."
            }
        }
    }
}

/// One setting beyond the hand-written fields.
#[derive(Debug, Clone, Copy)]
pub struct Extra {
    /// The key in the settings table, and the name of the form control.
    pub key: &'static str,
    /// The key the station reads it as (`birdnet.conf`), which the binary's
    /// settings bridge copies the saved value onto at start.
    pub config_key: &'static str,
    /// Which card it is drawn in.
    pub group: Group,
    /// The words beside the control.
    pub label: &'static str,
    /// What it does, in plain language, with the default.
    pub help: &'static str,
    /// How it is edited.
    pub kind: Kind,
    /// The settings-table category it is stored under.
    pub category: SettingsCategory,
}

const ON_OFF: &[(&str, &str)] = &[("", "Not set (off)"), ("true", "On"), ("false", "Off")];
const ON_OFF_DEFAULT_ON: &[(&str, &str)] =
    &[("", "Not set (on)"), ("true", "On"), ("false", "Off")];

/// Every extra, in the order it is drawn.
pub const EXTRA_SETTINGS: &[Extra] = &[
    // ── Home Assistant & MQTT ───────────────────────────────────────────
    Extra {
        key: "mqtt_host",
        config_key: "MQTT_HOST",
        group: Group::Mqtt,
        label: "Broker address",
        help: "The name or address of the MQTT broker, for example homeassistant.local. Leave empty for no MQTT.",
        kind: Kind::Text {
            placeholder: "homeassistant.local",
        },
        category: SettingsCategory::Notifications,
    },
    Extra {
        key: "mqtt_port",
        config_key: "MQTT_PORT",
        group: Group::Mqtt,
        label: "Broker port",
        help: "Empty means 1883, or 8883 when the secure connection below is on.",
        kind: Kind::Int { min: 1, max: 65535 },
        category: SettingsCategory::Notifications,
    },
    Extra {
        key: "mqtt_username",
        config_key: "MQTT_USERNAME",
        group: Group::Mqtt,
        label: "Broker user name",
        help: "Only if the broker asks for one.",
        kind: Kind::Text { placeholder: "" },
        category: SettingsCategory::Notifications,
    },
    Extra {
        key: "mqtt_password",
        config_key: "MQTT_PASSWORD",
        group: Group::Mqtt,
        label: "Broker password",
        help: "Stored on the station like the email password.",
        kind: Kind::Secret,
        category: SettingsCategory::Notifications,
    },
    Extra {
        key: "mqtt_tls",
        config_key: "MQTT_TLS",
        group: Group::Mqtt,
        label: "Secure connection (TLS)",
        help: "Encrypt the connection to the broker. A broker with a certificate of its own needs a trust file, which is set in the configuration file.",
        kind: Kind::Choice(ON_OFF),
        category: SettingsCategory::Notifications,
    },
    Extra {
        key: "mqtt_ca_file",
        config_key: "MQTT_CA_FILE",
        group: Group::Mqtt,
        label: "Broker trust file (PEM)",
        help: "Only for a broker with a certificate of its own: the path of a file on the station holding the certificate of the authority that signed it (or the broker's own, if self-signed). Setting it turns the secure connection on.",
        kind: Kind::Text {
            placeholder: "/etc/birdnet/mqtt-ca.pem",
        },
        category: SettingsCategory::Notifications,
    },
    Extra {
        key: "mqtt_client_cert",
        config_key: "MQTT_CLIENT_CERT",
        group: Group::Mqtt,
        label: "Client certificate (PEM)",
        help: "Only for a broker that demands a certificate from the station (mutual TLS): the path of the station's certificate file. Needs the private key below; setting it turns the secure connection on. The files stay on the station: nothing is uploaded through this page.",
        kind: Kind::Text {
            placeholder: "/etc/birdnet/mqtt-client.pem",
        },
        category: SettingsCategory::Notifications,
    },
    Extra {
        key: "mqtt_client_key",
        config_key: "MQTT_CLIENT_KEY",
        group: Group::Mqtt,
        label: "Client private key (PEM)",
        help: "The private key that belongs to the client certificate. A path, not the key itself. `birdnet-behavior --doctor` checks the pair without connecting.",
        kind: Kind::Text {
            placeholder: "/etc/birdnet/mqtt-client-key.pem",
        },
        category: SettingsCategory::Notifications,
    },
    Extra {
        key: "mqtt_topic_prefix",
        config_key: "MQTT_TOPIC_PREFIX",
        group: Group::Mqtt,
        label: "Topic prefix",
        help: "Messages are published under this name. Empty means birdnet.",
        kind: Kind::Text {
            placeholder: "birdnet",
        },
        category: SettingsCategory::Notifications,
    },
    Extra {
        key: "mqtt_retain",
        config_key: "MQTT_RETAIN",
        group: Group::Mqtt,
        label: "Keep the last message at the broker",
        help: "Lets a subscriber that connects later see the latest detection at once.",
        kind: Kind::Choice(ON_OFF),
        category: SettingsCategory::Notifications,
    },
    Extra {
        key: "mqtt_ha_discovery",
        config_key: "MQTT_HA_DISCOVERY",
        group: Group::Mqtt,
        label: "Tell Home Assistant about this station",
        help: "Publishes the messages that make the station appear in Home Assistant by itself, with its sensors, when the station starts.",
        kind: Kind::Choice(ON_OFF),
        category: SettingsCategory::Notifications,
    },
    // ── Alert limits ────────────────────────────────────────────────────
    Extra {
        key: "notify_rate_per_minute",
        config_key: "NOTIFY_RATE_PER_MINUTE",
        group: Group::AlertLimits,
        label: "At most this many alerts per minute",
        help: "Per destination, so a flock at the feeder cannot flood your phone. 0 turns the limit off; empty keeps the station's own limit.",
        kind: Kind::Int { min: 0, max: 600 },
        category: SettingsCategory::Notifications,
    },
    Extra {
        key: "station_health_alerts",
        config_key: "STATION_HEALTH_ALERTS",
        group: Group::AlertLimits,
        label: "Tell me when the station has a problem",
        help: "A microphone down while others work, a nearly full disk, an overheating board, a backup that has not run in weeks. One alert per problem, and one when it recovers. On unless you turn it off.",
        kind: Kind::Choice(ON_OFF_DEFAULT_ON),
        category: SettingsCategory::Notifications,
    },
    // ── Detection filters ───────────────────────────────────────────────
    Extra {
        key: "duplicate_interval_secs",
        config_key: "DUPLICATE_INTERVAL_SECS",
        group: Group::Filters,
        label: "Ignore repeats of the same bird for (seconds)",
        help: "A bird that sings through a whole 15-second recording is otherwise recorded several times. 0 or empty records every one.",
        kind: Kind::Int { min: 0, max: 3600 },
        category: SettingsCategory::Detection,
    },
    Extra {
        key: "noise_threshold",
        config_key: "NOISE_THRESHOLD",
        group: Group::Filters,
        label: "Drop recordings with a barking dog above (0 to 1)",
        help: "When a noise the identifier knows (a dog by default) scores at least this, every bird in that recording is dropped. 0 or empty is off; 0.5 to 0.8 is typical.",
        kind: Kind::Float { min: 0.0, max: 1.0 },
        category: SettingsCategory::Detection,
    },
    Extra {
        key: "noise_classes",
        config_key: "NOISE_CLASSES",
        group: Group::Filters,
        label: "Noises to watch for",
        help: "Names, separated by commas: Dog, Siren, Engine, Power tools, Fireworks, Gun. Empty means Dog.",
        kind: Kind::Text { placeholder: "Dog" },
        category: SettingsCategory::Detection,
    },
    Extra {
        key: "noise_remember_secs",
        config_key: "NOISE_REMEMBER_SECS",
        group: Group::Filters,
        label: "Keep dropping recordings for this long after a noise (seconds)",
        help: "A dog does not bark once. With the noise filter on, also drop the recordings that follow a noise for this many seconds, up to 300. 0 or empty drops only the recording it was heard in.",
        kind: Kind::Float {
            min: 0.0,
            max: 300.0,
        },
        category: SettingsCategory::Detection,
    },
    Extra {
        key: "night_filter",
        config_key: "NIGHT_FILTER",
        group: Group::Filters,
        label: "Hold back day birds heard at night",
        help: "A blue tit heard at 02:30 is almost always the identifier hearing something else. Owls, nightjars and other night birds are exempt, and held detections go to review, not the bin. Needs the station's location.",
        kind: Kind::Choice(ON_OFF),
        category: SettingsCategory::Detection,
    },
    Extra {
        key: "night_margin_mins",
        config_key: "NIGHT_MARGIN_MINS",
        group: Group::Filters,
        label: "Minutes of dusk and dawn that still count as day",
        help: "Birds sing through dusk and start well before sunrise. Empty means 60.",
        kind: Kind::Int { min: 0, max: 360 },
        category: SettingsCategory::Detection,
    },
    Extra {
        key: "night_extra_nocturnal",
        config_key: "NIGHT_EXTRA_NOCTURNAL",
        group: Group::Filters,
        label: "Extra birds allowed at night",
        help: "Genus or scientific names, separated by commas, for example migrating thrushes calling overhead.",
        kind: Kind::Text {
            placeholder: "Catharus, Turdus iliacus",
        },
        category: SettingsCategory::Detection,
    },
    Extra {
        key: "pre_capture_secs",
        config_key: "PRE_CAPTURE_SECS",
        group: Group::Filters,
        label: "Extra seconds of sound before each call in saved clips",
        help: "Adds lead-in to the saved clip, up to 30. Empty or 0 saves the usual clip.",
        kind: Kind::Float {
            min: 0.0,
            max: 30.0,
        },
        category: SettingsCategory::Detection,
    },
    // ── Backups ─────────────────────────────────────────────────────────
    Extra {
        key: "backup_schedule",
        config_key: "BACKUP_SCHEDULE",
        group: Group::Backups,
        label: "Back up the database",
        help: "How often the station makes a copy of its database. Copies beside the database protect against a damaged file, not against losing the card or the whole device; copying them elsewhere is on the Data tab.",
        kind: Kind::Choice(&[
            ("", "Weekly (the default)"),
            ("weekly", "Weekly"),
            ("daily", "Daily"),
        ]),
        category: SettingsCategory::System,
    },
    // ── Storage ─────────────────────────────────────────────────────────
    Extra {
        key: "purge_species_floor",
        config_key: "PURGE_SPECIES_FLOOR",
        group: Group::Storage,
        label: "Clips of every species that are never purged",
        help: "When the disk is full the oldest clips of the most-recorded birds go first; a species with this many or fewer is left alone, so the one clip of the year's rarest bird survives. 0 removes the protection; empty means 5.",
        kind: Kind::Int { min: 0, max: 10000 },
        category: SettingsCategory::System,
    },
    Extra {
        key: "raw_audio_keep_every",
        config_key: "RAW_AUDIO_KEEP_EVERY",
        group: Group::Storage,
        label: "Keep one raw recording in every",
        help: "Raw recordings are normally deleted once analysed. Keeping one in N lets you check what the microphone really hears, at the cost of disk. 0 or empty keeps none.",
        kind: Kind::Int {
            min: 0,
            max: 100_000,
        },
        category: SettingsCategory::System,
    },
    // ── Online services ─────────────────────────────────────────────────
    Extra {
        key: "ebird_api_key",
        config_key: "EBIRD_API_KEY",
        group: Group::Online,
        label: "eBird key",
        help: "From ebird.org/api/keygen. With a key, the station compares its detections with what other people report nearby and flags the unlikely ones. Without one, eBird is never contacted.",
        kind: Kind::Secret,
        category: SettingsCategory::Notifications,
    },
    Extra {
        key: "ebird_region",
        config_key: "EBIRD_REGION",
        group: Group::Online,
        label: "eBird region",
        help: "A region code, for example US-NY or GB. Empty uses a circle around the station's location.",
        kind: Kind::Text { placeholder: "GB" },
        category: SettingsCategory::Notifications,
    },
    Extra {
        key: "ebird_dist_km",
        config_key: "EBIRD_DIST_KM",
        group: Group::Online,
        label: "Radius around the station (km)",
        help: "eBird allows up to 50. Empty means the station's default.",
        kind: Kind::Int { min: 0, max: 50 },
        category: SettingsCategory::Notifications,
    },
    Extra {
        key: "ebird_back_days",
        config_key: "EBIRD_BACK_DAYS",
        group: Group::Online,
        label: "Look back this many days",
        help: "eBird allows 1 to 30. Empty means 14.",
        kind: Kind::Int { min: 1, max: 30 },
        category: SettingsCategory::Notifications,
    },
    Extra {
        key: "image_provider",
        config_key: "IMAGE_PROVIDER",
        group: Group::Online,
        label: "Where species photos come from",
        help: "Wikipedia needs nothing. Flickr needs a key. AviCommons are openly licensed bird photographs, falling back to Wikipedia.",
        kind: Kind::Choice(&[
            ("", "Wikipedia (the default)"),
            ("wikipedia", "Wikipedia"),
            ("flickr", "Flickr"),
            ("avicommons", "AviCommons"),
        ]),
        category: SettingsCategory::System,
    },
    Extra {
        key: "flickr_api_key",
        config_key: "FLICKR_API_KEY",
        group: Group::Online,
        label: "Flickr key",
        help: "Only for Flickr photos. From flickr.com/services/apps/create.",
        kind: Kind::Secret,
        category: SettingsCategory::System,
    },
    Extra {
        key: "flickr_filter_email",
        config_key: "FLICKR_FILTER_EMAIL",
        group: Group::Online,
        label: "Only photos by this Flickr account (email)",
        help: "Optional: use only one photographer's pictures.",
        kind: Kind::Text { placeholder: "" },
        category: SettingsCategory::System,
    },
    Extra {
        key: "birdweather_threshold",
        config_key: "BIRDWEATHER_THRESHOLD",
        group: Group::Online,
        label: "BirdWeather upload threshold (0 to 1)",
        help: "Upload to BirdWeather only the detections at or above this confidence, on top of the station's own threshold. The station's own log keeps everything. Empty or 0 uploads every detection the station records; 0.85 is a common choice for a public map.",
        kind: Kind::Float { min: 0.0, max: 1.0 },
        category: SettingsCategory::Notifications,
    },
    Extra {
        key: "birdweather_location_accuracy",
        config_key: "BIRDWEATHER_LOCATION_ACCURACY",
        group: Group::Online,
        label: "Blur the BirdWeather position by up to (metres)",
        help: "BirdWeather shows the position on a public map. With a distance here, every upload carries your position moved by one fixed offset (up to this far, in one direction, drawn once from a secret kept beside the database), so the map does not show your address. The offset is the same on every upload, so collecting many uploads does not average it away; it is not hidden from anyone who learns it, and changing this distance or losing the secret draws a new one. Empty or 0 posts the exact position. A value that cannot be used (such as 5km, or more than 50000) turns BirdWeather uploads off instead of posting the exact position.",
        kind: Kind::Int { min: 0, max: 50000 },
        category: SettingsCategory::Notifications,
    },
    Extra {
        key: "embed_origins",
        config_key: "EMBED_ORIGINS",
        group: Group::Online,
        label: "Sites that may show this station in a frame",
        help: "Empty (the default): no other site can. To put the station in a Home Assistant card, list that site's address, such as http://homeassistant.local:8123, several separated by commas. Only the sites you list; there is no way to allow every site. They can show the dashboard, the wall display and the live view; the sign-in and settings pages can never be framed.",
        kind: Kind::Text {
            placeholder: "http://homeassistant.local:8123",
        },
        category: SettingsCategory::System,
    },
];

/// The extras of one group, in order.
pub fn in_group(group: Group) -> impl Iterator<Item = &'static Extra> {
    EXTRA_SETTINGS.iter().filter(move |e| e.group == group)
}

/// Every extra's settings key.
pub fn keys() -> impl Iterator<Item = &'static str> {
    EXTRA_SETTINGS.iter().map(|e| e.key)
}

/// The extra called `key`, if there is one.
#[must_use]
pub fn find(key: &str) -> Option<&'static Extra> {
    EXTRA_SETTINGS.iter().find(|e| e.key == key)
}

fn value<'a>(settings: &'a HashMap<String, String>, key: &str) -> &'a str {
    settings.get(key).map_or("", String::as_str)
}

/// Draw one group's card.
pub(super) fn render_group(out: &mut String, group: Group, settings: &HashMap<String, String>) {
    let (id, title) = group.card();
    let _ = write!(
        out,
        r#"
  <section class="card" id="set-{id}" aria-labelledby="set-{id}-h">
    <h2 class="section-title" id="set-{id}-h">{title}</h2>
    <p class="hint flush">{blurb}</p>
    <div class="grid-2">"#,
        blurb = group.blurb(),
    );
    for e in in_group(group) {
        let _ = write!(
            out,
            r#"
      <div>
        <label for="{key}">{label}</label>
        {control}
        <p class="hint">{help}</p>
      </div>"#,
            key = e.key,
            label = e.label,
            control = control(e, value(settings, e.key)),
            help = e.help,
        );
    }
    out.push_str("\n    </div>\n  </section>");
}

/// The control for one extra with `stored` in it.
fn control(e: &Extra, stored: &str) -> String {
    let v = escape_html(stored);
    match e.kind {
        Kind::Text { placeholder } => format!(
            r#"<input id="{k}" name="{k}" type="text" value="{v}" placeholder="{p}" maxlength="200" autocomplete="off" spellcheck="false">"#,
            k = e.key,
            p = escape_html(placeholder),
        ),
        Kind::Secret => format!(
            r#"<input id="{k}" name="{k}" type="password" value="{v}" autocomplete="new-password" maxlength="200">"#,
            k = e.key,
        ),
        Kind::Int { min, max } => format!(
            r#"<input id="{k}" name="{k}" type="number" value="{v}" min="{min}" max="{max}" step="1" class="bnb-w-num">"#,
            k = e.key,
        ),
        Kind::Float { min, max } => format!(
            r#"<input id="{k}" name="{k}" type="text" inputmode="decimal" value="{v}" placeholder="{min} to {max}" class="bnb-w-num">"#,
            k = e.key,
        ),
        Kind::Choice(options) => {
            let mut s = format!(r#"<select id="{k}" name="{k}">"#, k = e.key);
            // A stored value that is not on the list is kept as an option and
            // chosen, so saving the page does not silently replace it.
            let known = options.iter().any(|(val, _)| *val == stored);
            if !known {
                let _ = write!(
                    s,
                    r#"<option value="{v}" selected>{v} (not a choice this page offers)</option>"#
                );
            }
            for (val, words) in options {
                let sel = if *val == stored { " selected" } else { "" };
                let _ = write!(s, r#"<option value="{val}"{sel}>{words}</option>"#);
            }
            s.push_str("</select>");
            s
        }
    }
}

/// `C:\certs\ca.pem`: a drive letter, a colon and a slash.
const fn is_windows_path(raw: &str) -> bool {
    let b = raw.as_bytes();
    b.len() > 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')
}

/// What is wrong with one submitted value, or `None`. A blank value is fine
/// everywhere: it clears the setting and the station runs on its default.
fn problem(e: &Extra, raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let label = e.label;
    match e.kind {
        Kind::Text { .. } | Kind::Secret => {
            if raw.chars().count() > 200 || raw.chars().any(char::is_control) {
                return Some(format!(
                    "{label} must be plain text of at most 200 characters."
                ));
            }
            match e.key {
                "mqtt_host"
                    if raw.contains("://")
                        || raw.contains(char::is_whitespace)
                        || raw.contains('/') =>
                {
                    Some(format!(
                        "{label} is a name or address such as homeassistant.local, without mqtt:// or a path. You entered “{raw}”."
                    ))
                }
                "ebird_region"
                    if !(2..=16).contains(&raw.len())
                        || !raw.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') =>
                {
                    Some(format!(
                        "{label} is an eBird region code such as GB or US-NY. You entered “{raw}”."
                    ))
                }
                "embed_origins" => crate::embed::EmbedOrigins::parse(raw)
                    .err()
                    .map(|e| format!("{label}: {e}")),
                "mqtt_ca_file" | "mqtt_client_cert" | "mqtt_client_key"
                    if !raw.starts_with('/') && !raw.starts_with('~') && !is_windows_path(raw) =>
                {
                    Some(format!(
                        "{label} is the full path of a file on the station, such as /etc/birdnet/mqtt-ca.pem. You entered “{raw}”."
                    ))
                }
                "mqtt_topic_prefix"
                    if raw.contains(['#', '+'])
                        || raw.starts_with('/')
                        || raw.contains(char::is_whitespace) =>
                {
                    Some(format!(
                        "{label} cannot contain spaces, # or +, or start with a slash. You entered “{raw}”."
                    ))
                }
                _ => None,
            }
        }
        Kind::Int { min, max } => {
            let rule = format!("{label} must be a whole number from {min} to {max}.");
            if raw.bytes().all(|b| b.is_ascii_digit()) {
                match raw.parse::<i64>() {
                    Ok(v) if (min..=max).contains(&v) => None,
                    _ => Some(format!("{rule} You entered {raw}.")),
                }
            } else {
                Some(format!("{rule} You entered “{raw}”."))
            }
        }
        Kind::Float { min, max } => {
            let normalised = birdnet_core::config::locale::normalize_decimal(raw);
            match normalised.parse::<f64>() {
                Ok(v) if v.is_finite() && (min..=max).contains(&v) => None,
                _ => Some(format!(
                    "{label} must be a number from {min} to {max}. You entered “{raw}”."
                )),
            }
        }
        Kind::Choice(options) => {
            if options.iter().any(|(val, _)| *val == raw) {
                None
            } else {
                Some(format!(
                    "{label} must be one of {}. You entered “{raw}”.",
                    options
                        .iter()
                        .filter(|(val, _)| !val.is_empty())
                        .map(|(val, _)| *val)
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            }
        }
    }
}

/// Everything wrong with the extras in a submission, phrased for the person who
/// typed them.
///
/// Checks what was **submitted**, not what changed: a bad value already stored
/// is exactly what the person came to the page to fix.
#[must_use]
pub fn problems<S: std::hash::BuildHasher>(submitted: &HashMap<String, String, S>) -> Vec<String> {
    let mut out: Vec<String> = EXTRA_SETTINGS
        .iter()
        .filter_map(|e| submitted.get(e.key).and_then(|raw| problem(e, raw)))
        .collect();
    // A client certificate and its key come as a pair (mutual TLS). Checked on
    // the submission, so a half-pair already stored is found when the page is
    // next saved, not left to fail at the broker.
    let set = |k: &str| submitted.get(k).is_some_and(|v| !v.trim().is_empty());
    if set("mqtt_client_cert") != set("mqtt_client_key") {
        out.push(
            "The MQTT client certificate and its private key go together: fill in both, or neither."
                .to_owned(),
        );
    }
    out
}

/// The rows a submission changes: each extra that was submitted and differs
/// from what is stored (or, with nothing stored, from what the page showed,
/// which for every extra is empty).
#[must_use]
pub fn changed<S: std::hash::BuildHasher>(
    submitted: &HashMap<String, String, S>,
    existing: &HashMap<String, String, S>,
) -> Vec<(&'static str, String, SettingsCategory)> {
    EXTRA_SETTINGS
        .iter()
        .filter_map(|e| {
            let raw = submitted.get(e.key)?;
            let v = match e.kind {
                Kind::Float { .. } => birdnet_core::config::locale::normalize_decimal(raw.trim()),
                _ => raw.trim().to_owned(),
            };
            let same = existing
                .get(e.key)
                .map_or(v.is_empty(), |stored| *stored == v);
            (!same).then_some((e.key, v, e.category))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn one(key: &str, value: &str) -> HashMap<String, String> {
        HashMap::from([(key.to_owned(), value.to_owned())])
    }

    #[test]
    fn keys_are_unique_and_do_not_collide_with_the_hand_written_form() {
        let mut seen = BTreeSet::new();
        for e in EXTRA_SETTINGS {
            assert!(seen.insert(e.key), "{} twice", e.key);
            assert!(
                !crate::routes::admin::settings::form::SETTINGS_FORM_KEYS.contains(&e.key),
                "{} is also a hand-written field",
                e.key
            );
            assert_eq!(
                e.key.to_ascii_uppercase(),
                e.config_key.replace("BIRDNET_", ""),
                "{}: the settings key is the config key in lower case",
                e.key
            );
        }
    }

    /// Both directions of every kind: a good value passes and the edges just
    /// outside fail, with the value quoted back.
    #[test]
    fn each_kind_accepts_its_range_and_refuses_the_edge_outside_it() {
        let port = find("mqtt_port").unwrap();
        assert!(
            problem(port, "1883").is_none()
                && problem(port, "1").is_none()
                && problem(port, "65535").is_none()
        );
        for bad in ["0", "65536", "-1", "18 83", "1883.5", "abc"] {
            assert!(problem(port, bad).is_some_and(|m| m.contains(bad)), "{bad}");
        }
        let noise = find("noise_threshold").unwrap();
        assert!(
            problem(noise, "0.6").is_none()
                && problem(noise, "0,6").is_none()
                && problem(noise, "1").is_none()
        );
        for bad in ["1.01", "-0.1", "75", "dog", "NaN"] {
            assert!(problem(noise, bad).is_some(), "{bad}");
        }
        let backup = find("backup_schedule").unwrap();
        assert!(problem(backup, "daily").is_none() && problem(backup, "").is_none());
        assert!(problem(backup, "hourly").is_some_and(|m| m.contains("weekly, daily")));
    }

    /// Blank clears; it is never an error and never "zero".
    #[test]
    fn blank_is_always_fine() {
        for e in EXTRA_SETTINGS {
            assert!(problem(e, "  ").is_none(), "{}", e.key);
        }
    }

    #[test]
    fn a_broker_address_is_not_a_url() {
        let host = find("mqtt_host").unwrap();
        assert!(problem(host, "homeassistant.local").is_none());
        assert!(problem(host, "192.168.1.5").is_none());
        for bad in ["mqtt://homeassistant.local", "broker/path", "two words"] {
            assert!(problem(host, bad).is_some(), "{bad}");
        }
    }

    #[test]
    fn region_codes_and_topic_prefixes_are_shaped() {
        let region = find("ebird_region").unwrap();
        assert!(problem(region, "GB").is_none() && problem(region, "US-NY").is_none());
        assert!(problem(region, "United Kingdom").is_some() && problem(region, "G").is_some());
        let topic = find("mqtt_topic_prefix").unwrap();
        assert!(problem(topic, "birdnet/garden").is_none());
        assert!(problem(topic, "birdnet/#").is_some() && problem(topic, "/birdnet").is_some());
    }

    /// The submission is validated, not the diff: a bad value that is already
    /// stored and submitted unchanged is still reported.
    #[test]
    fn problems_read_the_submission_not_the_difference() {
        let existing = one("mqtt_port", "99999");
        let submitted = one("mqtt_port", "99999");
        assert!(
            changed(&submitted, &existing).is_empty(),
            "unchanged writes nothing"
        );
        assert_eq!(problems(&submitted).len(), 1, "but it is still refused");
    }

    #[test]
    fn only_what_changed_is_written() {
        let existing = HashMap::from([
            ("mqtt_host".to_owned(), "a.local".to_owned()),
            ("noise_threshold".to_owned(), "0.6".to_owned()),
        ]);
        let submitted = HashMap::from([
            ("mqtt_host".to_owned(), " a.local ".to_owned()),
            ("noise_threshold".to_owned(), "0,7".to_owned()),
            ("mqtt_port".to_owned(), String::new()),
            ("night_filter".to_owned(), "true".to_owned()),
            ("not_an_extra".to_owned(), "x".to_owned()),
        ]);
        let rows: Vec<_> = changed(&submitted, &existing)
            .into_iter()
            .map(|(k, v, _)| (k, v))
            .collect();
        assert_eq!(
            rows,
            [
                ("noise_threshold", "0.7".to_owned()),
                ("night_filter", "true".to_owned())
            ]
        );
    }

    /// A stored choice this page does not offer is shown, not replaced.
    #[test]
    fn an_unknown_stored_choice_stays_selected() {
        let e = find("backup_schedule").unwrap();
        let html = control(e, "monthly");
        assert!(html.contains(r#"value="monthly" selected"#), "{html}");
    }

    #[test]
    fn every_group_renders_every_one_of_its_controls() {
        for group in [
            Group::Mqtt,
            Group::AlertLimits,
            Group::Filters,
            Group::Backups,
            Group::Storage,
            Group::Online,
        ] {
            let mut html = String::new();
            render_group(&mut html, group, &HashMap::new());
            let (id, _) = group.card();
            assert!(html.contains(&format!(r#"id="set-{id}""#)));
            for e in in_group(group) {
                assert!(html.contains(&format!(r#"name="{}""#, e.key)), "{}", e.key);
                assert!(
                    html.contains(&format!(r#"<label for="{}">"#, e.key)),
                    "{}",
                    e.key
                );
            }
        }
        let grouped: usize = [
            Group::Mqtt,
            Group::AlertLimits,
            Group::Filters,
            Group::Backups,
            Group::Storage,
            Group::Online,
        ]
        .into_iter()
        .map(|g| in_group(g).count())
        .sum();
        assert_eq!(
            grouped,
            EXTRA_SETTINGS.len(),
            "an extra belongs to no group"
        );
    }

    // ── the fields added for the parity rows (P-2, P-9, P-10) ───────────────

    /// P-9: the framing list is checked by the same parser the station starts
    /// with, so what the page accepts is what the station will use.
    #[test]
    fn the_embed_list_is_refused_with_the_parsers_own_words() {
        assert!(
            problems(&one(
                "embed_origins",
                "http://homeassistant.local:8123, https://abcd1234.ui.nabu.casa, https://*.example.com"
            ))
            .is_empty()
        );
        assert!(
            problems(&one("embed_origins", "")).is_empty(),
            "empty is the default: nobody"
        );
        for bad in [
            "*",
            "https://ha.example/x",
            "homeassistant.local",
            "https://*.com",
            // A wildcard over a suffix strangers register under: the docs' own
            // example used to be one of these.
            "https://*.ui.nabu.casa",
            "https://*.co.uk",
        ] {
            let found = problems(&one("embed_origins", bad));
            assert_eq!(found.len(), 1, "{bad:?} was accepted");
            assert!(found[0].contains("scheme://host"), "{bad:?}: {}", found[0]);
        }
    }

    /// P-10: the certificate and its key are a pair, and both are paths.
    #[test]
    fn a_client_certificate_needs_its_key_and_both_are_paths() {
        let both = HashMap::from([
            (
                "mqtt_client_cert".to_owned(),
                "/etc/birdnet/c.pem".to_owned(),
            ),
            (
                "mqtt_client_key".to_owned(),
                "/etc/birdnet/k.pem".to_owned(),
            ),
        ]);
        assert!(problems(&both).is_empty());
        assert!(
            problems(&one("mqtt_client_cert", "/etc/birdnet/c.pem"))
                .iter()
                .any(|p| p.contains("go together"))
        );
        assert!(
            problems(&one("mqtt_client_key", "/etc/birdnet/k.pem"))
                .iter()
                .any(|p| p.contains("go together"))
        );
        assert!(problems(&HashMap::new()).is_empty(), "neither is fine");
        for bad in ["c.pem", "-----BEGIN CERTIFICATE-----", "relative/c.pem"] {
            assert!(
                problems(&one("mqtt_ca_file", bad))
                    .iter()
                    .any(|p| p.contains("full path")),
                "{bad:?} was accepted as a path"
            );
        }
        assert!(problems(&one("mqtt_ca_file", "/etc/ca.pem")).is_empty());
    }

    /// P-2: a percentage typed in a 0 to 1 field is refused, not clamped; the
    /// blur is metres up to 50 km.
    #[test]
    fn the_birdweather_fields_refuse_the_obvious_slips() {
        assert!(problems(&one("birdweather_threshold", "0.85")).is_empty());
        assert!(problems(&one("birdweather_threshold", "0,85")).is_empty());
        assert_eq!(problems(&one("birdweather_threshold", "85")).len(), 1);
        assert!(problems(&one("birdweather_location_accuracy", "250")).is_empty());
        assert_eq!(
            problems(&one("birdweather_location_accuracy", "50001")).len(),
            1
        );
        assert_eq!(
            problems(&one("birdweather_location_accuracy", "2.5km")).len(),
            1
        );
    }
}
