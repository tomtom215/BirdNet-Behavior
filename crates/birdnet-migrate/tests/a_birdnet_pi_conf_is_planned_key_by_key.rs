//! A BirdNET-Pi `birdnet.conf` is read key by key, and nothing in it is dropped
//! without a reason or turned into the opposite of what it meant.
//!
//! The fixture is BirdNET-Pi's own default file as `install_config.sh` writes
//! it (60 `KEY=` lines), with a station's position filled in. Run through this
//! station's `--doctor` before the importer existed, 40 of its 60 keys were
//! reported as "not a setting this station reads".

use std::collections::HashMap;

use birdnet_migrate::conf::{ConfFile, Plan, PlanInput, Verdict, plan};

const DEFAULT_CONF: &str = include_str!("data/birdnet_pi_default.conf");

/// A validator that accepts everything, for tests of the mapping alone.
fn accepts_all(_setting: &str, _value: &str) -> Option<String> {
    None
}

fn plan_of(text: &str) -> Plan {
    let conf = ConfFile::parse(text);
    let current = HashMap::new();
    plan(&PlanInput {
        conf: &conf,
        current: &current,
        check: &accepts_all,
    })
}

fn row<'a>(plan: &'a Plan, key: &str) -> &'a birdnet_migrate::conf::Row {
    plan.rows
        .iter()
        .find(|r| r.pi_key == key)
        .unwrap_or_else(|| panic!("{key} has no row"))
}

/// The gate for A-1: each of the 60 keys has a verdict and a reason a person
/// can read, and none is the catch-all "not a key BirdNET-Pi writes".
#[test]
fn every_key_of_a_default_file_has_a_row_and_none_is_unknown() {
    let plan = plan_of(DEFAULT_CONF);
    let keys = DEFAULT_CONF
        .lines()
        .filter(|l| l.split('=').next().is_some_and(|k| {
            !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        }) && l.contains('='))
        .count();
    assert_eq!(keys, 60, "the fixture is BirdNET-Pi's 60 default keys");
    let from_the_file = plan
        .rows
        .iter()
        .filter(|r| r.pi_key != "(no BirdNET-Pi key)")
        .count();
    assert_eq!(from_the_file, 60);
    for r in &plan.rows {
        assert!(
            !r.why.starts_with("not a key BirdNET-Pi writes"),
            "{} fell through to the unknown-key row",
            r.pi_key
        );
        if matches!(r.verdict, Verdict::NotCarried | Verdict::Attention | Verdict::Refused) {
            assert!(!r.why.is_empty(), "{} is not carried over and says nothing", r.pi_key);
        }
    }
    assert_eq!(plan.unreadable_lines, 1, "the stray closing brace the extraction left");
}

/// The counterpart: a key BirdNET-Pi does not write is said to be unknown, not
/// dropped quietly.
#[test]
fn a_key_nobody_knows_is_reported_as_unknown() {
    let plan = plan_of("MY_OWN_SETTING=1\n");
    let r = row(&plan, "MY_OWN_SETTING");
    assert_eq!(r.verdict, Verdict::NotCarried);
    assert!(r.why.starts_with("not a key BirdNET-Pi writes"), "{}", r.why);
}

/// `APPRISE_ONLY_NOTIFY_SPECIES_NAMES` is the **exclusion** list and `_2` the
/// inclusion list (`notifications.py: should_notify`). A rename by name would
/// swap them: a bird the owner never wanted to hear about would be the only one
/// announced.
#[test]
fn the_two_species_lists_are_not_swapped() {
    let plan = plan_of(
        "APPRISE_ONLY_NOTIFY_SPECIES_NAMES=\"Mourning Dove, American Crow\"\n\
         APPRISE_ONLY_NOTIFY_SPECIES_NAMES_2=\"Northern Cardinal\"\n",
    );
    assert_eq!(
        plan.value_of("notify_species_exclude"),
        Some("Mourning Dove, American Crow"),
        "the key with no suffix excludes"
    );
    assert_eq!(
        plan.value_of("notify_species_only"),
        Some("Northern Cardinal"),
        "the key with the _2 suffix includes"
    );
}

/// The id of a BirdWeather station is a credential: carried, and never shown.
#[test]
fn the_birdweather_id_becomes_the_token_and_is_not_shown() {
    let plan = plan_of("BIRDWEATHER_ID=abcSECRET123\n");
    assert_eq!(plan.value_of("birdweather_token"), Some("abcSECRET123"));
    let r = row(&plan, "BIRDWEATHER_ID");
    assert_eq!(r.shown, "(hidden, 12 characters)");
    assert_eq!(r.becomes, [("birdweather_token".to_owned(), "(hidden, 12 characters)".to_owned())]);
    let printed = format!("{plan:?}");
    assert!(!printed.contains("abcSECRET123"), "a Debug print leaked it: {printed}");
}

/// Every credential in the table is hidden wherever the plan is displayed.
#[test]
fn no_credential_in_the_default_file_reaches_a_display_string() {
    let plan = plan_of(&format!(
        "{DEFAULT_CONF}\nBIRDWEATHER_ID=tok123\nRTSP_STREAM=rtsp://cam:hunter2@10.0.0.5/s\nFLICKR_API_KEY=flk456\nHEARTBEAT_URL=https://hc.example/ping/9f8e\n"
    ));
    for needle in ["tok123", "hunter2", "flk456", "9f8e", "secret"] {
        for r in &plan.rows {
            let shown = format!("{} {:?} {}", r.shown, r.becomes, r.why);
            assert!(
                !shown.contains(needle),
                "{needle} is shown for {}: {shown}",
                r.pi_key
            );
        }
    }
}

/// BirdNET-Pi's privacy rule is a percentage of the species list; this station
/// takes the same rule as a rank. `int(6000 * p / 100)`, at least 10.
#[test]
fn the_privacy_percentage_becomes_the_same_rule_as_a_rank() {
    for (given, rank) in [("0", "10"), ("0.1", "10"), ("1", "60"), ("2", "120"), ("3", "180"), ("0.5", "30")] {
        let plan = plan_of(&format!("PRIVACY_THRESHOLD={given}\n"));
        assert_eq!(plan.value_of("privacy_rank"), Some(rank), "PRIVACY_THRESHOLD={given}");
        assert_eq!(
            plan.value_of("privacy_threshold"),
            Some("0"),
            "the confidence rule is switched off so the number is not read twice"
        );
    }
}

/// The three independent switches of `notifications.py` as one list of rules.
#[test]
fn the_three_notification_switches_become_one_list_of_rules() {
    let triggers = |each: u8, new: u8, daily: u8| {
        plan_of(&format!(
            "APPRISE_NOTIFY_EACH_DETECTION={each}\nAPPRISE_NOTIFY_NEW_SPECIES={new}\nAPPRISE_NOTIFY_NEW_SPECIES_EACH_DAY={daily}\n"
        ))
        .value_of("notify_trigger")
        .map(str::to_owned)
    };
    assert_eq!(triggers(1, 0, 0).as_deref(), Some("each"));
    assert_eq!(triggers(0, 1, 0).as_deref(), Some("new-species"));
    assert_eq!(triggers(0, 0, 1).as_deref(), Some("new-species-daily"));
    assert_eq!(triggers(1, 1, 0).as_deref(), Some("each,new-species"));
    assert_eq!(triggers(0, 1, 1).as_deref(), Some("new-species,new-species-daily"));
    assert_eq!(triggers(1, 1, 1).as_deref(), Some("each,new-species,new-species-daily"));
    assert_eq!(
        triggers(0, 0, 0).as_deref(),
        Some("off"),
        "all three at 0 is BirdNET-Pi sending nothing; this station's default is 'each'"
    );
}

/// BirdNET-Pi has no separate notification minimum; this station's default is
/// 0.80. Without pinning it to 0 a station whose `CONFIDENCE` is 0.70 would stop
/// announcing the detections between 0.70 and 0.80, with nothing to say so.
#[test]
fn a_migrated_station_keeps_notifying_for_every_stored_detection() {
    let plan = plan_of("CONFIDENCE=0.7\nAPPRISE_NOTIFY_EACH_DETECTION=1\n");
    assert_eq!(plan.value_of("notify_confidence"), Some("0"));
    let implied = row(&plan, "(no BirdNET-Pi key)");
    assert!(implied.why.contains("0.80"), "{}", implied.why);

    // Counterpart: a file with nothing about notifications does not touch it.
    let plan = plan_of("CONFIDENCE=0.7\n");
    assert_eq!(plan.value_of("notify_confidence"), None);
    assert!(plan.rows.iter().all(|r| r.pi_key != "(no BirdNET-Pi key)"));
}

/// The cooldown is carried with its zero: BirdNET-Pi's default is no limit and
/// this station's is 300 s.
#[test]
fn the_cooldown_is_set_even_when_it_is_zero() {
    let plan = plan_of("APPRISE_MINIMUM_SECONDS_BETWEEN_NOTIFICATIONS_PER_SPECIES=0\n");
    assert_eq!(plan.value_of("notify_cooldown"), Some("0"));
}

#[test]
fn the_disk_policy_and_threshold_are_carried() {
    let plan = plan_of("FULL_DISK=keep\nPURGE_THRESHOLD=90\n");
    assert_eq!(plan.value_of("full_disk_action"), Some("keep"));
    assert_eq!(plan.value_of("purge_threshold"), Some("90"));
    let plan = plan_of("FULL_DISK=PURGE\n");
    assert_eq!(plan.value_of("full_disk_action"), Some("purge"));
    let plan = plan_of("FULL_DISK=erase\n");
    assert_eq!(row(&plan, "FULL_DISK").verdict, Verdict::Refused);
    assert_eq!(plan.value_of("full_disk_action"), None);
}

/// A station that chose `keep` and also a species cap still has the cap: say so.
#[test]
fn keep_with_a_species_cap_is_flagged() {
    let plan = plan_of("FULL_DISK=keep\nMAX_FILES_SPECIES=500\n");
    let r = row(&plan, "FULL_DISK");
    assert_eq!(r.verdict, Verdict::Attention);
    assert!(r.why.contains("MAX_FILES_SPECIES"), "{}", r.why);
    let r = row(&plan, "MAX_FILES_SPECIES");
    assert_eq!(r.verdict, Verdict::Attention);
    assert!(r.why.contains("most **confident**"), "the rule differs: {}", r.why);

    // Counterpart: no cap, no warning.
    let plan = plan_of("FULL_DISK=keep\nMAX_FILES_SPECIES=0\n");
    assert_eq!(row(&plan, "FULL_DISK").verdict, Verdict::Imported);
    assert_eq!(row(&plan, "MAX_FILES_SPECIES").verdict, Verdict::Imported);
}

/// A position of 0, 0 is BirdNET-Pi's "could not find one".
#[test]
fn a_position_of_zero_zero_is_not_copied_but_a_real_one_is() {
    let plan = plan_of("LATITUDE=0.0000\nLONGITUDE=0.0000\n");
    assert_eq!(plan.value_of("latitude"), None);
    assert_eq!(row(&plan, "LATITUDE").verdict, Verdict::Left);

    let plan = plan_of("LATITUDE=52.0\nLONGITUDE=5.0\n");
    assert_eq!(plan.value_of("latitude"), Some("52.0"));
    assert_eq!(plan.value_of("longitude"), Some("5.0"));

    // On the equator, with a real longitude, is a place.
    let plan = plan_of("LATITUDE=0.0\nLONGITUDE=9.5\n");
    assert_eq!(plan.value_of("latitude"), Some("0.0"));
}

#[test]
fn a_shell_variable_is_not_copied_as_text() {
    let plan = plan_of("SITE_NAME=\"$HOSTNAME\"\n");
    assert_eq!(plan.value_of("site_name"), None);
    let plan = plan_of("SITE_NAME=\"Hollow Oak\"\n");
    assert_eq!(plan.value_of("site_name"), Some("Hollow Oak"));
}

/// The settings page's own validation decides what is acceptable. A value it
/// would refuse is refused in the preview, not after "Apply".
#[test]
fn a_value_the_station_would_refuse_is_refused_in_the_preview() {
    let conf = ConfFile::parse("CONFIDENCE=75\nSENSITIVITY=1.25\n");
    let current = HashMap::new();
    let reject_percentages = |setting: &str, value: &str| {
        (setting == "confidence_threshold" && value.parse::<f64>().is_ok_and(|v| v > 1.0))
            .then(|| "must be between 0 and 1".to_owned())
    };
    let plan = plan(&PlanInput {
        conf: &conf,
        current: &current,
        check: &reject_percentages,
    });
    assert_eq!(row(&plan, "CONFIDENCE").verdict, Verdict::Refused);
    assert_eq!(plan.value_of("confidence_threshold"), None);
    assert_eq!(row(&plan, "SENSITIVITY").verdict, Verdict::Imported);
}

#[test]
fn the_audio_format_is_carried_only_if_this_station_writes_it() {
    let plan = plan_of("AUDIOFMT=mp3\n");
    assert_eq!(plan.value_of("audio_format"), Some("mp3"));
    let plan = plan_of("AUDIOFMT=aiff\n");
    assert_eq!(row(&plan, "AUDIOFMT").verdict, Verdict::Refused);
    assert_eq!(plan.value_of("audio_format"), None);
}

#[test]
fn the_weekly_report_switch_is_a_day_or_off() {
    assert_eq!(plan_of("APPRISE_WEEKLY_REPORT=1\n").value_of("weekly_report_schedule"), Some("monday"));
    assert_eq!(plan_of("APPRISE_WEEKLY_REPORT=0\n").value_of("weekly_report_schedule"), Some("disabled"));
}

/// Keys only `birdnet.conf` can carry are handed back as lines to put there.
#[test]
fn keys_this_station_reads_only_from_its_config_file_come_back_as_lines() {
    let plan = plan_of("IMAGE_PROVIDER=FLICKR\nFLICKR_API_KEY=k123\nFLICKR_FILTER_EMAIL=me@example.com\n");
    assert_eq!(plan.assignments.len(), 0);
    let keys: Vec<&str> = plan.config_lines.iter().map(|l| l.key).collect();
    assert_eq!(keys, ["IMAGE_PROVIDER", "FLICKR_API_KEY", "FLICKR_FILTER_EMAIL"]);
    assert_eq!(row(&plan, "FLICKR_API_KEY").verdict, Verdict::ConfigFile);
}

/// BirdNET-Pi's occurrence-filter version is the thing that changes under the
/// owner's feet: version 1 is not offered here.
#[test]
fn occurrence_model_one_is_flagged_and_two_is_not() {
    let plan = plan_of("DATA_MODEL_VERSION=1\nMODEL=BirdNET_GLOBAL_6K_V2.4_Model_FP16\n");
    let r = row(&plan, "DATA_MODEL_VERSION");
    assert_eq!(r.verdict, Verdict::Attention);
    assert!(r.why.contains("not offered here"), "{}", r.why);
    assert_eq!(row(&plan, "MODEL").verdict, Verdict::Left);

    let plan = plan_of("DATA_MODEL_VERSION=2\n");
    assert_eq!(row(&plan, "DATA_MODEL_VERSION").verdict, Verdict::Left);
}

#[test]
fn the_sign_in_password_is_never_carried_and_the_user_name_is_explained() {
    let plan = plan_of("CADDY_PWD=hunter2\n");
    let r = row(&plan, "CADDY_PWD");
    assert_eq!(r.verdict, Verdict::Attention);
    assert!(r.why.contains("`admin`"), "{}", r.why);
    assert!(plan.assignments.is_empty());
    assert!(!format!("{plan:?}").contains("hunter2"));
}

/// Preview and apply are one computation.
#[test]
fn the_same_file_gives_the_same_plan() {
    assert_eq!(plan_of(DEFAULT_CONF), plan_of(DEFAULT_CONF));
}

#[test]
fn the_default_file_sets_what_a_migrated_station_needs_to_behave_the_same() {
    let plan = plan_of(DEFAULT_CONF);
    for (setting, value) in [
        ("confidence_threshold", "0.7"),
        ("sensitivity", "1.25"),
        ("overlap", "0.0"),
        ("sf_thresh", "0.03"),
        ("audio_format", "mp3"),
        ("full_disk_action", "purge"),
        ("purge_threshold", "95"),
        ("info_site", "allaboutbirds"),
        ("rare_species_days", "30"),
        ("segment_duration", "15"),
        ("notify_trigger", "off"),
        ("notify_cooldown", "0"),
        ("notify_confidence", "0"),
        ("weekly_report_schedule", "monday"),
        ("raw_spectrogram", "false"),
        ("privacy_rank", "10"),
        ("latitude", "52.0"),
        ("longitude", "5.0"),
    ] {
        assert_eq!(plan.value_of(setting), Some(value), "{setting}");
    }
}
