//! The range filter against BirdNET-Pi's, on every week and every calendar day.
//!
//! `v24_range_filter_matches_birdnet_pi` asks twelve questions. This asks all
//! of them the golden holds: five places (including the two "no location"
//! spellings, `(-1, -1)` and `(0, 0)`) times weeks 1-53, as answered by
//! BirdNET-Pi's own code running its v2 `TFLite` range model
//! (`tools/parity/golden/species_list_allweeks_compact.json`), against the
//! shipping `SpeciesFilter` on the ONNX conversion.
//!
//! It then checks the *calendar*, which is where the two projects deliberately
//! differ. BirdNET-Pi passes the ISO week (1-53) to a model trained on a
//! 48-week year (four weeks a month); this project, like birdnet-go, passes
//! the model's own week (`civil::birdnet_week`). For every day of three years
//! (a leap year, a 53-week ISO year, and another leap year) it asserts that
//! the week this project asks about is in 1-48, that the list the ONNX model
//! returns for it is the list BirdNET-Pi's `TFLite` model returns for the same
//! week, and, as the measured size of the deliberate difference, on how many
//! days the list BirdNET-Pi would have used for that date differs from ours.
//! Those counts were produced by Python's `datetime.isocalendar()` over the
//! golden's hashes (`tools/parity/range_calendar.py` makes the same comparison
//! over the models themselves, and its 2026 London row says 335), not by this
//! file's own ISO-week arithmetic, which is checked against them.

mod common;

use std::collections::HashMap;
use std::path::PathBuf;

use birdnet_core::civil::{birdnet_week_from_date, date_string_from_days, days_from_civil};
use birdnet_core::inference::labels::LabelSet;
use birdnet_core::inference::species_filter::{SpeciesFilter, SpeciesFilterConfig};
use sha2::{Digest, Sha256};

const GOLDEN: &str = "tools/parity/golden/species_list_allweeks_compact.json";

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// ISO 8601 week number of a civil date.
fn iso_week(year: u32, month: u32, day: u32) -> u32 {
    let days = days_from_civil(year, month, day);
    // 1970-01-01 was a Thursday. Monday = 1 ... Sunday = 7.
    let weekday = i64::from((days + 3).rem_euclid(7) as i32) + 1;
    let yday = days - days_from_civil(year, 1, 1) + 1;
    let week = (yday - weekday + 10) / 7;
    let weeks_in = |y: u32| -> i64 {
        // A year has 53 weeks when 1 January is a Thursday, or a Wednesday in a leap year.
        let jan1 = (days_from_civil(y, 1, 1) + 3).rem_euclid(7) + 1;
        let leap = birdnet_core::civil::is_leap_year(y);
        if jan1 == 4 || (leap && jan1 == 3) {
            53
        } else {
            52
        }
    };
    if week < 1 {
        u32::try_from(weeks_in(year - 1)).unwrap()
    } else if week > weeks_in(year) {
        1
    } else {
        u32::try_from(week).unwrap()
    }
}

struct Golden {
    /// `(lat, lon, week)` -> `(count, sha256 of the sorted names)`, v2 only.
    v2: HashMap<(i64, i64, u32), (usize, String)>,
    places: Vec<(f64, f64)>,
}

fn key(lat: f64, lon: f64, week: u32) -> (i64, i64, u32) {
    #[allow(clippy::cast_possible_truncation)]
    ((lat * 1e4).round() as i64, (lon * 1e4).round() as i64, week)
}

fn load_golden() -> Golden {
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(GOLDEN).expect("golden")).expect("json");
    let mut v2 = HashMap::new();
    let mut places: Vec<(f64, f64)> = Vec::new();
    for e in json["entries"].as_array().expect("entries") {
        if e["version"] != 2 {
            continue;
        }
        let (lat, lon) = (e["lat"].as_f64().unwrap(), e["lon"].as_f64().unwrap());
        let week = u32::try_from(e["week"].as_u64().unwrap()).unwrap();
        v2.insert(
            key(lat, lon, week),
            (
                usize::try_from(e["count"].as_u64().unwrap()).unwrap(),
                e["sorted_names_sha256"].as_str().unwrap().to_owned(),
            ),
        );
        if !places.contains(&(lat, lon)) {
            places.push((lat, lon));
        }
    }
    Golden { v2, places }
}

#[test]
fn every_week_and_place_in_the_golden_returns_birdnet_pis_list() {
    let Some((_, labels_path)) = common::v24_model_paths_with_mdata() else {
        return;
    };
    let mdata = PathBuf::from(std::env::var("BIRDNET_TEST_V24_MDATA").expect("checked"));
    let labels = LabelSet::load(&labels_path).expect("labels");
    let mut filter = SpeciesFilter::load_with_vocabulary(
        &mdata,
        None,
        &labels,
        &HashMap::new(),
        SpeciesFilterConfig {
            sf_thresh: 0.03,
            ..SpeciesFilterConfig::default()
        },
    )
    .expect("the v2 range filter loads");
    let golden = load_golden();
    assert_eq!(golden.v2.len(), 5 * 53, "five places x 53 weeks of v2");
    let mut asked = 0;
    for &(lat, lon) in &golden.places {
        for week in 1..=53_u32 {
            let allowed = filter
                .filter_species(Some((lat, lon)), week, &labels)
                .expect("filter");
            let mut names: Vec<&str> = allowed.iter().map(String::as_str).collect();
            names.sort_unstable();
            let (count, sha) = &golden.v2[&key(lat, lon, week)];
            assert_eq!(allowed.len(), *count, "count at ({lat}, {lon}) week {week}");
            assert_eq!(
                &hex(&Sha256::digest(names.join("\n").as_bytes())),
                sha,
                "list at ({lat}, {lon}) week {week}"
            );
            asked += 1;
        }
    }
    assert_eq!(asked, 265);
}

/// For each of three years: the number of days on which, at each golden place,
/// the list for the ISO week differs from the list for the 48-week-year week.
/// Places in the golden's order of appearance sorted by `(lat, lon)`:
/// `(-1,-1)`, `(0,0)`, Ithaca, `(50,5)`, London.
const DAYS_THE_CALENDARS_DISAGREE: [(u32, u32, [usize; 5]); 3] = [
    (2024, 366, [162, 276, 310, 307, 305]),
    (2026, 365, [182, 309, 332, 329, 335]),
    (2028, 366, [156, 256, 286, 287, 287]),
];

#[test]
fn the_calendar_is_in_the_models_domain_and_the_difference_is_the_documented_size() {
    let golden = load_golden();
    let mut places = golden.places.clone();
    places.sort_by(|a, b| a.partial_cmp(b).unwrap());
    for (year, days_in_year, expected) in DAYS_THE_CALENDARS_DISAGREE {
        let first = days_from_civil(year, 1, 1);
        let mut differ = [0_usize; 5];
        let mut weeks_seen_53 = 0;
        for d in 0..i64::from(days_in_year) {
            let date = date_string_from_days(first + d);
            let (y, m, dd) = (
                date[0..4].parse::<u32>().unwrap(),
                date[5..7].parse::<u32>().unwrap(),
                date[8..10].parse::<u32>().unwrap(),
            );
            assert_eq!(y, year);
            let ours = birdnet_week_from_date(&date).expect("a real date");
            assert!(
                (1..=48).contains(&ours),
                "{date}: week {ours} is outside the model's 1-48"
            );
            let iso = iso_week(y, m, dd);
            assert!((1..=53).contains(&iso), "{date}: ISO week {iso}");
            weeks_seen_53 += usize::from(iso == 53);
            for (i, &(lat, lon)) in places.iter().enumerate() {
                if golden.v2[&key(lat, lon, iso)] != golden.v2[&key(lat, lon, ours)] {
                    differ[i] += 1;
                }
            }
        }
        // 2026 has a week 53 (28 Dec - 3 Jan 2027, so four days of it fall in 2026);
        // 2024 and 2028 have none inside their own year.
        assert_eq!(weeks_seen_53, if year == 2026 { 4 } else { 0 }, "{year}");
        assert_eq!(differ, expected, "{year}: days the two calendars disagree");
    }
}

/// The ISO-week arithmetic above, against dates whose ISO week is known by
/// heart (ISO 8601's own examples and the year boundaries the golden's
/// 53-week row exists for).
#[test]
fn the_iso_week_arithmetic_used_here_is_right() {
    for ((y, m, d), want) in [
        ((2026, 1, 1), 1),
        ((2026, 12, 28), 53),
        ((2026, 12, 31), 53),
        ((2027, 1, 1), 53),
        ((2027, 1, 3), 53),
        ((2027, 1, 4), 1),
        ((2024, 12, 30), 1),
        ((2024, 12, 29), 52),
        ((2021, 1, 1), 53),
        ((2020, 12, 31), 53),
        ((2024, 2, 29), 9),
        ((2028, 1, 1), 52),
        ((2028, 1, 3), 1),
    ] {
        assert_eq!(iso_week(y, m, d), want, "{y}-{m:02}-{d:02}");
    }
}
