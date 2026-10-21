//! A `birdnet.conf` carried over from BirdNET-Pi draws an answer for every key,
//! not forty "is not a setting this station reads".
//!
//! `--doctor` on BirdNET-Pi's own default file reported 40 of its 60 keys as
//! unread, and only `SITE_NAME` drew a hint (by an accident of edit distance).
//! The table that says where each key goes is `birdnet_migrate::conf::RULES`
//! (the importer's); the small copy the doctor reads is
//! `birdnet_core::config::known_keys::BIRDNET_PI_KEYS`. This holds the two to
//! each other and to the keys this station reads.

use birdnet_core::config::known_keys::{
    BIRDNET_PI_KEYS, KNOWN_CONFIG_KEYS, PiEquivalent, birdnet_pi_equivalent, is_known,
};
use birdnet_core::config::{Config, redact};
use birdnet_migrate::conf::{RULES, Target};
use birdnet_web::routes::admin::settings::form::SETTINGS_FORM_KEYS;

fn pi_default() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("crates/birdnet-migrate/tests/data/birdnet_pi_default.conf");
    std::fs::read_to_string(path).expect("BirdNET-Pi's default birdnet.conf fixture")
}

/// The gate for A-1's doctor half: no key of the 60 gets the generic "check the
/// key against the reference", and a key that does not carry across says so.
#[test]
fn every_unread_key_of_birdnet_pis_default_file_is_told_where_it_went() {
    let cfg = Config::parse(&pi_default()).expect("parses");
    let unknown = cfg.unknown_keys();
    assert!(
        unknown.len() > 20,
        "the fixture should leave many keys unread, saw {}",
        unknown.len()
    );
    for u in &unknown {
        assert!(
            u.birdnet_pi.is_some(),
            "{} drew only the generic message: {u}",
            u.key
        );
        assert!(
            !u.remedy().starts_with("remove the line"),
            "{}: {}",
            u.key,
            u.remedy()
        );
    }
    // The ones the owner has to act on, by name.
    let remedy_of = |key: &str| {
        unknown
            .iter()
            .find(|u| u.key == key)
            .unwrap_or_else(|| panic!("{key} was not reported"))
            .remedy()
    };
    assert!(remedy_of("BIRDWEATHER_ID").starts_with("rename it to BIRDWEATHER_TOKEN"));
    assert!(remedy_of("PURGE_THRESHOLD").starts_with("rename it to DISK_PURGE_THRESHOLD"));
    assert!(remedy_of("REC_CARD").starts_with("rename it to ALSA_CARD"));
    assert!(remedy_of("RTSP_STREAM").starts_with("rename it to RTSP_URLS"));
    assert!(
        remedy_of("APPRISE_ONLY_NOTIFY_SPECIES_NAMES")
            .starts_with("rename it to APPRISE_WATCHLIST_EXCLUDE"),
        "the name that misleads is the exclusion list"
    );
    assert!(
        remedy_of("APPRISE_ONLY_NOTIFY_SPECIES_NAMES_2")
            .starts_with("rename it to APPRISE_WATCHLIST")
    );
    assert!(remedy_of("APPRISE_NOTIFY_NEW_SPECIES").contains("APPRISE_TRIGGER"));
    assert!(remedy_of("COLOR_SCHEME").starts_with("it has no equivalent here"));
    assert!(remedy_of("CUSTOM_IMAGE").contains("dashboard_image"));
    // `FULL_DISK` is read now, so it is not among the unread.
    assert!(unknown.iter().all(|u| u.key != "FULL_DISK"));
}

/// The doctor's table and the importer's are one list of keys.
#[test]
fn the_doctors_table_and_the_importers_agree() {
    for rule in RULES {
        let read_here = is_known(rule.pi_key);
        let hint = birdnet_pi_equivalent(rule.pi_key);
        assert!(
            read_here != hint.is_some(),
            "{}: the station {} this key and the doctor's table {} an entry for it",
            rule.pi_key,
            if read_here { "reads" } else { "does not read" },
            if hint.is_some() { "has" } else { "has no" }
        );
        // The kinds agree where the doctor says something.
        match (rule.target, hint) {
            (
                Target::Setting { .. } | Target::ConfigFile { .. },
                Some(PiEquivalent::None { .. }),
            ) => {
                panic!(
                    "{}: the importer sets it and the doctor says it has no equivalent",
                    rule.pi_key
                )
            }
            (Target::Nothing, Some(PiEquivalent::Renamed { .. })) => {
                panic!(
                    "{}: the importer drops it and the doctor says rename it",
                    rule.pi_key
                )
            }
            _ => {}
        }
    }
    for (key, _) in BIRDNET_PI_KEYS {
        assert!(
            RULES.iter().any(|r| r.pi_key == *key),
            "{key} is in the doctor's table and not the importer's"
        );
    }
}

/// What a rename points at is a key this station reads, and what a "set it in
/// the browser" points at is a setting the Settings form has.
#[test]
fn the_targets_exist() {
    for (key, eq) in BIRDNET_PI_KEYS {
        match eq {
            PiEquivalent::Renamed { ours, .. } => {
                assert!(KNOWN_CONFIG_KEYS.contains(ours), "{key} -> {ours}");
            }
            PiEquivalent::Setting { name, .. } => {
                assert!(SETTINGS_FORM_KEYS.contains(name), "{key} -> {name}");
            }
            PiEquivalent::None { .. } => {}
        }
    }
    for rule in RULES {
        match rule.target {
            Target::Setting { key, .. } => assert!(
                SETTINGS_FORM_KEYS.contains(&key),
                "{} writes `{key}`, which the Settings form does not have",
                rule.pi_key
            ),
            Target::ConfigFile { key, .. } => assert!(
                KNOWN_CONFIG_KEYS.contains(&key),
                "{} hands back `{key}`, which nothing reads",
                rule.pi_key
            ),
            _ => {}
        }
    }
}

/// A credential the importer hides is one the support bundle hides too.
#[test]
fn what_the_importer_hides_the_support_bundle_hides() {
    for (rule_key, sample) in [
        ("BIRDWEATHER_ID", "abcdef0123456789"),
        ("RTSP_STREAM", "rtsp://cam:hunter2@192.168.1.20:554/h264"),
        (
            "HEARTBEAT_URL",
            "https://hc-ping.com/9a1b2c3d-1111-2222-3333-444455556666",
        ),
        ("FLICKR_API_KEY", "0123456789abcdef"),
    ] {
        let rule = RULES.iter().find(|r| r.pi_key == rule_key).expect("rule");
        assert!(rule.secret, "{rule_key} should be marked secret");
        let (Target::Setting { key: ours, .. } | Target::ConfigFile { key: ours, .. }) =
            rule.target
        else {
            panic!("{rule_key} has no target")
        };
        let hidden_by_key = redact::is_secret_key(ours);
        let hidden_by_value = redact::redact_value(sample) != sample;
        assert!(
            hidden_by_key || hidden_by_value,
            "{rule_key} -> {ours}: the importer hides it but the support bundle shows `{sample}`"
        );
    }
}

/// Every key the importer calls a secret is in the table as one.
#[test]
fn credentials_are_marked_wherever_they_are_named() {
    for rule in RULES {
        let looks_secret = redact::is_secret_key(rule.pi_key);
        if looks_secret && !matches!(rule.pi_key, "IMAGE_PROVIDER") {
            assert!(
                rule.secret,
                "{} looks like a credential to the support bundle and is not marked secret here",
                rule.pi_key
            );
        }
    }
}
