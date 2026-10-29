//! One sentence per chart: what it found.
//!
//! Each returns **markup**, not text: a time in a sentence goes through [`crate::display`], so a
//! station set to 12-hour time reads "7:00 AM" here as it does everywhere else (a gate holds
//! every page to that), and a species' name is escaped on the way in.
//!
//! A chart answers a question, and a reader should not have to read the answer off the
//! picture. These functions take the numbers a chart's partial already holds and return the one
//! thing worth saying about them, or `None` when the sample is too small to say anything.
//! They are pure, so what a page claims is a function with tests and not a string typed into a
//! template (the old "Mornings light up first — the dawn chorus" was printed under the grid
//! whatever the grid held, including on stations whose birds sing at noon).
//!
//! The rules they keep:
//!
//! * **Silence is allowed.** Below [`MIN_DETECTIONS`] there is no shape to describe.
//! * **A claim names its basis.** "Sundays are busiest" is said only for a window long enough
//!   to hold several Sundays; for a short one the sentence says that, and what to choose.
//! * **No adjective the number does not earn.** A weekday is "the busiest" always, "stands out"
//!   only when it is a quarter above the average day.

use std::fmt::Write as _;

use super::frame::{DAWN_FIRST_HOUR, DAWN_LAST_HOUR, is_dawn_hour};
use super::{escape_html, group_thousands};
use crate::display::clock_hour;

/// Fewest detections in a window before any claim about its shape.
pub const MIN_DETECTIONS: i64 = 30;

/// Fewest days a window must span before weekdays are compared.
pub const MIN_WEEKDAY_WINDOW_DAYS: u32 = 28;

/// Fewest occurrences of a weekday (listened days of that name) before it is compared.
const MIN_WEEKDAY_OCCURRENCES: u32 = 3;

/// A weekday must beat the average day by this factor to be said to stand out.
const STANDS_OUT: f64 = 1.25;

const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// `part` as a whole percentage of `whole`, rounded half up; 0 for an empty whole.
#[must_use]
pub fn percent(part: i64, whole: i64) -> i64 {
    if whole <= 0 {
        return 0;
    }
    let p = (i128::from(part) * 100 + i128::from(whole) / 2) / i128::from(whole);
    i64::try_from(p).unwrap_or(i64::MAX)
}

/// The hour starting at `hour`, as a span of clock times: 07:00–08:00 for 7, 23:00–00:00 for 23.
#[must_use]
pub fn hour_span(hour: usize) -> String {
    format!("{}–{}", clock_hour(hour), clock_hour((hour + 1) % 24))
}

/// What the busiest hour of the day is, and how much of the song the dawn hours hold.
#[must_use]
pub fn hours_sentence(by_hour: &[i64; 24]) -> Option<String> {
    let total: i64 = by_hour.iter().sum();
    if total < MIN_DETECTIONS {
        return None;
    }
    // The earliest hour on a tie, so the sentence does not change between two loads of the
    // same numbers.
    let (peak, &peak_n) = by_hour
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(&a.0)))?;
    let dawn: i64 = by_hour[DAWN_FIRST_HOUR..=DAWN_LAST_HOUR].iter().sum();
    let (peak_pct, dawn_pct) = (percent(peak_n, total), percent(dawn, total));
    let dawn_span = format!(
        "{} to {}",
        clock_hour(DAWN_FIRST_HOUR),
        clock_hour(DAWN_LAST_HOUR + 1)
    );
    Some(if is_dawn_hour(peak) {
        format!(
            "Song peaks at {} ({peak_pct}% of detections); the dawn hours, {dawn_span}, hold {dawn_pct}% in all.",
            hour_span(peak)
        )
    } else {
        format!(
            "Activity peaks at {} ({peak_pct}% of detections), outside the dawn hours, which hold {dawn_pct}%.",
            hour_span(peak)
        )
    })
}

/// Which weekday is busiest, said only when the window can tell weekdays apart.
///
/// `counts[dow][hour]` is detections (dow 0 = Sunday); `occurrences[dow]` is how many days of
/// that name the window held and the station listened on.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    reason = "counts and day counts are far below 2^53; the ratio is a comparison, not a total"
)]
pub fn weekday_sentence(
    counts: &[[i64; 24]; 7],
    occurrences: [u32; 7],
    window_days: u32,
) -> Option<String> {
    if window_days < MIN_WEEKDAY_WINDOW_DAYS {
        return Some(format!(
            "{window_days} days is too short to tell the weekdays apart; choose 30 days or more to compare them."
        ));
    }
    // Detections per listened day of each name, for the names with enough days to average.
    let per_day: Vec<(usize, f64)> = (0..7)
        .filter(|&d| occurrences[d] >= MIN_WEEKDAY_OCCURRENCES)
        .map(|d| {
            (
                d,
                counts[d].iter().sum::<i64>() as f64 / f64::from(occurrences[d]),
            )
        })
        .collect();
    if per_day.len() < 5 {
        return None;
    }
    let mean = per_day.iter().map(|(_, v)| v).sum::<f64>() / per_day.len() as f64;
    if mean <= 0.0 {
        return None;
    }
    let &(best, best_avg) = per_day
        .iter()
        .max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)))?;
    let above = ((best_avg / mean - 1.0) * 100.0).round();
    Some(if best_avg >= mean * STANDS_OUT {
        format!(
            "{}s are the busiest day, averaging {:.0} detections: {above:.0}% above the average day.",
            WEEKDAYS[best], best_avg
        )
    } else {
        format!(
            "No weekday stands out: the busiest, {}, is only {above:.0}% above the average day.",
            WEEKDAYS[best]
        )
    })
}

/// Who led, from species most-heard first and the window's total (which includes species not
/// in `named`).
#[must_use]
pub fn leader_sentence(named: &[(String, i64)], total: i64) -> Option<String> {
    if total < MIN_DETECTIONS {
        return None;
    }
    let (name, n) = named.first()?;
    let mut s = format!(
        "{} led the window with {} of {} detections ({}%)",
        escape_html(name),
        group_thousands(*n),
        group_thousands(total),
        percent(*n, total)
    );
    if named.len() >= 3 {
        let top3: i64 = named.iter().take(3).map(|(_, n)| n).sum();
        let p3 = percent(top3, total);
        if p3 >= 50 {
            let _ = write!(s, ", and the three busiest made up {p3}%");
        }
    }
    s.push('.');
    Some(s)
}

/// A whole number of hours in words for the few a window takes, else digits.
#[allow(clippy::cast_possible_truncation)] // a window is a few hours
fn hours_in_words(hours: f32) -> String {
    match hours.round() as i32 {
        1 => "one".to_owned(),
        2 => "two".to_owned(),
        3 => "three".to_owned(),
        4 => "four".to_owned(),
        n => n.to_string(),
    }
}

/// What the terrain found about sunrise, as plain text: how much of what was heard fell within
/// `half_window` hours either side of it, against what an even spread of the day would put
/// there (`chance`). `share` and `chance` are fractions, 0 to 1.
///
/// Three readings, because the comparison can go three ways and only one of them is "song
/// peaks at sunrise": clearly more than an even spread, clearly less, or about the same. `None`
/// for numbers that are not fractions.
#[must_use]
pub fn sunrise_sentence(share: f64, chance: f64, half_window: f32) -> Option<String> {
    if !(0.0..=1.0).contains(&share) || !(0.0..=1.0).contains(&chance) || chance <= 0.0 {
        return None;
    }
    let pct = |f: f64| format!("{:.0}", (f * 100.0).round());
    let (has, even) = (pct(share), pct(chance));
    let window = hours_in_words(half_window);
    let ratio = share / chance;
    Some(if ratio >= 1.25 {
        format!(
            "{has}% of what was heard fell within {window} hours either side of sunrise, against {even}% if song were spread evenly through the day."
        )
    } else if ratio <= 0.8 {
        format!(
            "Little of it gathers at sunrise: {has}% fell within {window} hours either side, where an even spread would give {even}%."
        )
    } else {
        format!(
            "Sunrise makes little difference here: {has}% fell within {window} hours either side, about what an even spread would give ({even}%)."
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sentence as a reader sees it with scripting off: the tags gone.
    fn text(html: &str) -> String {
        let mut out = String::new();
        let mut in_tag = false;
        for c in html.chars() {
            match c {
                '<' => in_tag = true,
                '>' => in_tag = false,
                _ if !in_tag => out.push(c),
                _ => {}
            }
        }
        out
    }

    fn day(peak_hour: usize, peak: i64) -> [i64; 24] {
        let mut h = [2_i64; 24];
        h[peak_hour] = peak;
        h
    }

    #[test]
    fn percent_rounds_half_up_and_survives_an_empty_whole() {
        assert_eq!(percent(1, 3), 33);
        assert_eq!(percent(2, 3), 67);
        assert_eq!(percent(1, 2), 50);
        assert_eq!(percent(5, 0), 0);
        assert_eq!(percent(1_600_000, 1_600_000), 100);
    }

    #[test]
    fn hour_spans_wrap_at_midnight() {
        assert_eq!(text(&hour_span(7)), "07:00–08:00");
        assert_eq!(text(&hour_span(23)), "23:00–00:00");
    }

    #[test]
    fn a_dawn_peak_says_song_peaks_and_counts_the_dawn_share() {
        // 2 * 23 + 100 = 146 detections, 100 + 2 * 3 = 106 in hours 5..=8.
        let html = hours_sentence(&day(7, 100)).expect("a sentence");
        let s = text(&html);
        assert!(
            s.starts_with("Song peaks at 07:00–08:00 (68% of detections)"),
            "{s}"
        );
        assert!(s.contains("05:00 to 09:00, hold 73% in all"), "{s}");
        // Every time in it is one the browser can re-print in the reader's clock.
        assert_eq!(html.matches("data-clk=").count(), 4, "{html}");
    }

    #[test]
    fn a_midday_peak_is_not_called_song_at_dawn() {
        let s = text(&hours_sentence(&day(12, 100)).expect("a sentence"));
        assert!(s.starts_with("Activity peaks at 12:00–13:00"), "{s}");
        assert!(s.contains("outside the dawn hours"), "{s}");
        assert!(!s.contains("Song peaks"), "{s}");
    }

    #[test]
    fn too_few_detections_say_nothing() {
        let mut h = [0_i64; 24];
        h[7] = MIN_DETECTIONS - 1;
        assert_eq!(hours_sentence(&h), None);
        h[7] = MIN_DETECTIONS;
        assert!(hours_sentence(&h).is_some());
    }

    #[test]
    fn a_tie_names_the_earlier_hour() {
        let mut h = [0_i64; 24];
        h[9] = 20;
        h[6] = 20;
        let s = text(&hours_sentence(&h).expect("a sentence"));
        assert!(s.contains("06:00–07:00"), "{s}");
    }

    fn week(per_day: [i64; 7]) -> [[i64; 24]; 7] {
        let mut c = [[0_i64; 24]; 7];
        for (d, n) in per_day.iter().enumerate() {
            c[d][8] = *n * 4; // four occurrences each
        }
        c
    }

    #[test]
    fn a_short_window_says_so_instead_of_naming_a_weekday() {
        let s = weekday_sentence(&week([10; 7]), [1; 7], 7).expect("advice");
        assert!(s.contains("7 days is too short"), "{s}");
        assert!(s.contains("30 days or more"), "{s}");
    }

    #[test]
    fn a_weekday_that_beats_the_average_by_a_quarter_stands_out() {
        // Sunday 20 a day, the other six 10: mean = 70/7... (20 + 60) / 7 = 11.43; 20/11.43 = 1.75.
        let s =
            weekday_sentence(&week([20, 10, 10, 10, 10, 10, 10]), [4; 7], 30).expect("a sentence");
        assert!(
            s.starts_with("Sundays are the busiest day, averaging 20 detections"),
            "{s}"
        );
        assert!(s.contains("75% above the average day"), "{s}");
    }

    #[test]
    fn an_even_week_says_nothing_stands_out() {
        let s =
            weekday_sentence(&week([10, 10, 10, 10, 11, 10, 10]), [4; 7], 30).expect("a sentence");
        assert!(
            s.starts_with("No weekday stands out: the busiest, Thursday"),
            "{s}"
        );
    }

    #[test]
    fn weekdays_with_too_few_days_are_not_compared() {
        assert_eq!(weekday_sentence(&week([10; 7]), [2; 7], 30), None);
        // Four of seven is not enough names to call one of them busiest.
        assert_eq!(
            weekday_sentence(&week([10; 7]), [4, 4, 4, 4, 2, 2, 2], 30),
            None
        );
    }

    #[test]
    fn the_leader_is_named_with_its_share_and_the_top_three_when_they_dominate() {
        let named = vec![
            ("Blue Jay".to_owned(), 22),
            ("American Goldfinch".to_owned(), 12),
            ("White-breasted Nuthatch".to_owned(), 12),
            ("Mourning Dove".to_owned(), 11),
        ];
        // 46 of 107 is 43%: the top three do not dominate, so they are not mentioned.
        let s = leader_sentence(&named, 107).expect("a sentence");
        assert_eq!(
            s,
            "Blue Jay led the window with 22 of 107 detections (21%)."
        );
        // 46 of 80 is 58%: the three together dominate and are named.
        let s = leader_sentence(&named, 80).expect("a sentence");
        assert!(s.contains("the three busiest made up 58%"), "{s}");
    }

    #[test]
    fn a_leader_needs_a_window_worth_describing() {
        assert_eq!(leader_sentence(&[("Blue Jay".to_owned(), 5)], 29), None);
        assert_eq!(leader_sentence(&[], 100), None);
    }

    #[test]
    fn a_dawn_concentrated_soundscape_is_compared_with_an_even_spread() {
        let s = sunrise_sentence(0.62, 4.0 / 24.0, 2.0).unwrap();
        assert_eq!(
            s,
            "62% of what was heard fell within two hours either side of sunrise, against 17% if song were spread evenly through the day."
        );
    }

    /// The three ways the comparison can go each say a different thing; a station that sings at
    /// night is not told that song peaks at sunrise.
    #[test]
    fn the_sunrise_reading_follows_the_comparison_both_ways() {
        let chance = 4.0 / 24.0;
        let more = sunrise_sentence(0.40, chance, 2.0).unwrap();
        let about = sunrise_sentence(0.17, chance, 2.0).unwrap();
        let less = sunrise_sentence(0.05, chance, 2.0).unwrap();
        assert!(more.contains("against"), "{more}");
        assert!(about.contains("little difference"), "{about}");
        assert!(less.contains("Little of it gathers"), "{less}");
        assert_ne!(more, about);
        assert_ne!(about, less);
    }

    #[test]
    fn numbers_that_are_not_fractions_say_nothing() {
        assert!(sunrise_sentence(1.2, 0.17, 2.0).is_none());
        assert!(sunrise_sentence(-0.1, 0.17, 2.0).is_none());
        assert!(sunrise_sentence(0.5, 0.0, 2.0).is_none());
        assert!(sunrise_sentence(f64::NAN, 0.17, 2.0).is_none());
    }
}
