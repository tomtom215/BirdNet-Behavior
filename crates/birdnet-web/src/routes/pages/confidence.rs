//! How confidence is coloured, once, for every surface that shows it.
//!
//! # The defect this replaces
//!
//! Confidence was graded by five different rules. The Today feed's bar used
//! `> 90 %` high / `> 75 %` mid / else grey; the species pills, the dashboard,
//! the kiosk, the quarantine queue and the review queue used `>= 80 %` high /
//! `>= 50 %` mid / else red; the behavioural cards used `>= 50 %`. The same
//! 82 % detection was **amber** on the Today feed and **green** on the Review
//! page. And none of them knew what the station accepts: with the confidence
//! threshold at 70–75 %, every detection between that threshold and 75 % drew
//! grey — the colour of a disabled control — although the station had
//! accepted it, and on a station averaging 80–90 % nearly every row of the
//! species list was amber.
//!
//! # The rule
//!
//! Three tiers, stated in whole percent so that what the reader sees decides
//! the colour (an 80 % badge is never "borderline" because the score under it
//! was 79.6 %):
//!
//! | Tier | Whole percent | Reads as |
//! |---|---|---|
//! | [`Tier::High`] | `>= 90` ([`SURE`]) | very sure |
//! | [`Tier::Mid`] | from the borderline edge up to 89 | sure |
//! | [`Tier::Low`] | below the borderline edge | borderline |
//!
//! The borderline edge is **relative to the station's own cut-off**: the
//! cut-off plus [`BORDERLINE_MARGIN`] (ten points), but never above [`SURE`].
//! A detection ten points or less above the cut-off, or below it (which only
//! happens to rows saved when the cut-off was lower, or held for review),
//! "only just made it"; that is the one thing worth drawing attention to on a
//! list of accepted detections. On a strict station (cut-off 85 % or more) the
//! mid tier is empty and everything under 90 % is borderline, which is true.
//!
//! Why ten points: the model's score is not calibrated to a hit rate, so no
//! absolute figure is "right"; what is stable is the *distance from the line
//! the station drew*. Ten points is wide enough that the amber rows are a
//! minority on a normal day (on the fixture, 9 of 32 at the default cut-off)
//! and narrow enough that it still means "close to the line".
//!
//! # No red
//!
//! The scale never uses the alert hue. A 72 % detection the station accepted is
//! not an error, and red on a confidence is what made the review queue read
//! as a list of faults. Amber says "look twice"; red stays for things that are
//! wrong.
//!
//! # Where the colour lives
//!
//! `--conf-high`, `--conf-mid`, `--conf-low` (fill) and their `-soft` / `-ink`
//! pairs (pill background and text) in `app.css`, defined per theme. This file
//! only chooses a class name.

use birdnet_core::config::DEFAULT_CONFIDENCE_THRESHOLD;

/// The whole-percent score from which a detection is "very sure".
pub const SURE: u32 = 90;

/// How many points above the station's cut-off still count as borderline.
pub const BORDERLINE_MARGIN: u32 = 10;

/// The setting that holds the station's cut-off, as a fraction.
const SETTING_KEY: &str = "confidence_threshold";

/// One of the three confidence tiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// 90 % and up.
    High,
    /// Sure, and clear of the station's cut-off.
    Mid,
    /// At or near the station's cut-off.
    Low,
}

impl Tier {
    /// The CSS class that selects this tier's tokens.
    #[must_use]
    pub(crate) const fn class(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Mid => "mid",
            Self::Low => "low",
        }
    }

    /// What the tier means, in the reader's words.
    #[must_use]
    pub(crate) const fn word(self) -> &'static str {
        match self {
            Self::High => "very sure",
            Self::Mid => "sure",
            Self::Low => "borderline",
        }
    }
}

/// Whole percent, rounded the way every surface prints it.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn whole_percent(fraction: f64) -> u32 {
    if fraction.is_nan() {
        return 0;
    }
    (fraction.clamp(0.0, 1.0) * 100.0).round() as u32
}

/// The lowest whole percent that is *not* borderline for a station whose
/// cut-off is `floor` (a fraction).
#[must_use]
pub fn borderline_edge(floor: f64) -> u32 {
    // A cut-off that is not a number is no cut-off at all; the shipped default
    // stands in rather than "0 %", which would call everything sure.
    let floor = if floor.is_nan() {
        default_floor()
    } else {
        floor
    };
    (whole_percent(floor) + BORDERLINE_MARGIN).min(SURE)
}

/// The tier of a detection scoring `confidence` (a fraction) at a station whose
/// cut-off is `floor` (a fraction).
#[must_use]
pub fn tier(confidence: f64, floor: f64) -> Tier {
    let pct = whole_percent(confidence);
    if pct >= SURE {
        Tier::High
    } else if pct < borderline_edge(floor) {
        Tier::Low
    } else {
        Tier::Mid
    }
}

/// [`tier`] for a score already expressed in percent (0–100).
#[must_use]
pub fn tier_of_percent(percent: f64, floor: f64) -> Tier {
    tier(percent / 100.0, floor)
}

/// The station's confidence cut-off as a fraction, from its settings; the
/// shipped default when none is stored or the stored value is not a number in
/// `0..=1`.
///
/// One indexed read of a small table; callers that render a list read it once
/// per request, not per row.
#[must_use]
pub fn station_floor(conn: &rusqlite::Connection) -> f64 {
    birdnet_db::settings::get_or(conn, SETTING_KEY, "")
        .ok()
        .and_then(|v| parse_floor(&v))
        .unwrap_or_else(default_floor)
}

/// The shipped default cut-off, as a fraction.
#[must_use]
pub fn default_floor() -> f64 {
    f64::from(DEFAULT_CONFIDENCE_THRESHOLD)
}

/// A stored cut-off. Accepts `0.7` and `0,7`; rejects anything outside `0..=1`
/// (the settings form refuses those, but a hand-edited row must not be able to
/// move every colour on every page).
fn parse_floor(raw: &str) -> Option<f64> {
    let v: f64 = raw.trim().replace(',', ".").parse().ok()?;
    (0.0..=1.0).contains(&v).then_some(v)
}

/// The key printed once above a list of confidence bars: what each colour
/// means *at this station*.
#[must_use]
pub fn legend(floor: f64) -> String {
    let edge = borderline_edge(floor);
    let cut = whole_percent(floor);
    let mid = if edge < SURE {
        format!(
            r#"<span class="bnb-conf-key mid"><i aria-hidden="true"></i>{edge}–{top}% sure</span>"#,
            top = SURE - 1,
        )
    } else {
        String::new()
    };
    format!(
        r#"<p class="bnb-conf-legend" aria-label="What the confidence colours mean"><span class="bnb-conf-key high"><i aria-hidden="true"></i>{SURE}% and up very sure</span>{mid}<span class="bnb-conf-key low"><i aria-hidden="true"></i>under {edge}% borderline</span><span class="bnb-conf-cut">this station's cut-off is {cut}%</span></p>"#,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The boundaries, at a station whose cut-off is 70 %.
    #[test]
    fn the_tiers_at_a_seventy_percent_station() {
        let f = 0.70;
        assert_eq!(tier(0.95, f), Tier::High);
        assert_eq!(tier(0.90, f), Tier::High, "90 is the first very-sure score");
        assert_eq!(tier(0.89, f), Tier::Mid);
        assert_eq!(
            tier(0.80, f),
            Tier::Mid,
            "ten points above the cut-off is clear of it"
        );
        assert_eq!(tier(0.79, f), Tier::Low);
        assert_eq!(tier(0.72, f), Tier::Low, "accepted, and only just");
        assert_eq!(tier(0.70, f), Tier::Low);
        assert_eq!(
            tier(0.50, f),
            Tier::Low,
            "below the cut-off: held, or saved earlier"
        );
    }

    /// What the reader reads decides the colour: a score of 79.6 % prints as 80
    /// and must be coloured as 80.
    #[test]
    fn the_printed_percent_decides_the_tier() {
        assert_eq!(tier(0.796, 0.70), Tier::Mid);
        assert_eq!(tier(0.794, 0.70), Tier::Low);
        assert_eq!(tier(0.896, 0.70), Tier::High, "89.6 prints as 90");
    }

    /// The tiers move with the station's own line, which is the point of
    /// having a floor argument at all.
    #[test]
    fn the_borderline_edge_follows_the_stations_cut_off() {
        assert_eq!(tier(0.82, 0.70), Tier::Mid);
        assert_eq!(
            tier(0.82, 0.75),
            Tier::Low,
            "the same score is closer to a 75 % line"
        );
        assert_eq!(tier(0.82, 0.50), Tier::Mid);
        assert_eq!(
            tier(0.62, 0.50),
            Tier::Mid,
            "twelve points above a 50 % line is clear of it"
        );
        assert_eq!(tier(0.58, 0.50), Tier::Low);
        assert_eq!(borderline_edge(0.70), 80);
        assert_eq!(borderline_edge(0.75), 85);
    }

    /// A strict station has no mid tier: under 90 is borderline, 90 and up is
    /// sure. The edge never passes the sure line.
    #[test]
    fn a_strict_station_has_no_mid_tier() {
        assert_eq!(borderline_edge(0.90), SURE);
        assert_eq!(borderline_edge(0.95), SURE);
        assert_eq!(tier(0.89, 0.90), Tier::Low);
        assert_eq!(tier(0.90, 0.90), Tier::High);
        assert!(
            !legend(0.90).contains("bnb-conf-key mid"),
            "no empty tier in the key"
        );
    }

    /// Nothing a database or a caller can hand in panics or leaves the scale.
    #[test]
    fn hostile_input_stays_on_the_scale() {
        for c in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0, 7.0] {
            let _ = tier(c, 0.7);
        }
        assert_eq!(tier(f64::NAN, 0.7), Tier::Low);
        assert_eq!(tier(7.0, 0.7), Tier::High);
        assert_eq!(tier(0.5, f64::NAN), Tier::Low);
    }

    #[test]
    fn a_stored_cut_off_is_parsed_or_ignored() {
        assert_eq!(parse_floor("0.7"), Some(0.7));
        assert_eq!(parse_floor(" 0,75 "), Some(0.75));
        assert_eq!(
            parse_floor("75"),
            None,
            "the percentage slip is not a fraction"
        );
        assert_eq!(parse_floor(""), None);
        assert_eq!(parse_floor("abc"), None);
        assert_eq!(parse_floor("-0.1"), None);
    }

    #[test]
    fn the_station_floor_reads_the_setting_and_falls_back_to_the_default() {
        let conn = rusqlite::Connection::open_in_memory().expect("open");
        birdnet_db::migration::migrate(&conn).expect("migrate");
        assert!((station_floor(&conn) - default_floor()).abs() < 1e-9);
        birdnet_db::settings::set(
            &conn,
            SETTING_KEY,
            "0.6",
            birdnet_db::settings::SettingsCategory::Detection,
        )
        .expect("set");
        assert!((station_floor(&conn) - 0.6).abs() < 1e-9);
        birdnet_db::settings::set(
            &conn,
            SETTING_KEY,
            "75",
            birdnet_db::settings::SettingsCategory::Detection,
        )
        .expect("set");
        assert!(
            (station_floor(&conn) - default_floor()).abs() < 1e-9,
            "a value outside 0..=1 must not move every colour on every page"
        );
    }

    #[test]
    fn the_legend_states_the_stations_numbers() {
        let l = legend(0.70);
        assert!(l.contains("90% and up very sure"), "{l}");
        assert!(l.contains("80–89% sure"), "{l}");
        assert!(l.contains("under 80% borderline"), "{l}");
        assert!(l.contains("cut-off is 70%"), "{l}");
    }
}
