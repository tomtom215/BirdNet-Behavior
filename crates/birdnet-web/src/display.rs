//! How the station prints a clock time and a temperature, in one place.
//!
//! # Why one place
//!
//! A station chooses how it is read: a US visitor-centre wall wants `4:31 PM`
//! and `71°F`, a bird club in Berlin wants `16:31` and `21°C`, and one browser
//! on the same station may want the other. Neither choice can be honoured by
//! forty pages each formatting their own, so there is one formatter and one
//! test (`tests/the_clock_and_temperature_are_printed_in_one_place.rs`) that
//! fails when a page prints a time or a temperature by hand.
//!
//! # How a choice reaches a page
//!
//! The server writes the **canonical** text (24-hour time, degrees Celsius,
//! which is also what a page with scripting off shows) and marks it with the
//! value it was made from: `data-clk="16:31:37"` or `data-tc="21.4"`.
//! `static/display-format.js` re-prints every marked element in the choice in
//! force: the browser's own (`localStorage`, set on the Display card) or, when
//! that says "station default", the station's (`time_format` and
//! `temperature_unit` in the settings table, read from `/api/v2/display`).
//! One implementation cannot serve both the server and a choice made in the
//! browser, so there are two, held to one table of cases
//! (`tests/data/display_cases.json`) by a Rust test and by the browser gate
//! `tools/visual-qa/display-format.mjs`.
//!
//! # What is not here
//!
//! Machine formats stay ISO and 24-hour: RSS and iCalendar feeds, JSON, CSV,
//! `<time datetime>` attributes, URLs and the database. A sentence that must be
//! plain text (a confirmation dialog, a `title` attribute) cannot carry markup
//! and keeps the canonical text.

/// Settings key: the station's default clock, `24h` or `12h`.
pub const TIME_FORMAT_KEY: &str = "time_format";
/// Settings key: the station's default temperature unit, `celsius` or `fahrenheit`.
pub const TEMPERATURE_UNIT_KEY: &str = "temperature_unit";

/// The clock a time is printed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimeFormat {
    /// `16:31:37`
    #[default]
    H24,
    /// `4:31:37 PM`
    H12,
}

impl TimeFormat {
    /// The value stored in the settings table.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::H24 => "24h",
            Self::H12 => "12h",
        }
    }

    /// The format a stored value names, or `None`.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "24h" => Some(Self::H24),
            "12h" => Some(Self::H12),
            _ => None,
        }
    }
}

/// The unit a temperature is printed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TemperatureUnit {
    /// `21°C`
    #[default]
    Celsius,
    /// `70°F`
    Fahrenheit,
}

impl TemperatureUnit {
    /// The value stored in the settings table.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Celsius => "celsius",
            Self::Fahrenheit => "fahrenheit",
        }
    }

    /// The unit a stored value names, or `None`.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "celsius" => Some(Self::Celsius),
            "fahrenheit" => Some(Self::Fahrenheit),
            _ => None,
        }
    }
}

/// Whether `value` is a clock the settings page offers.
#[must_use]
pub fn is_time_format(value: &str) -> bool {
    TimeFormat::parse(value).is_some()
}

/// Whether `value` is a temperature unit the settings page offers.
#[must_use]
pub fn is_temperature_unit(value: &str) -> bool {
    TemperatureUnit::parse(value).is_some()
}

/// The station's two defaults, as stored. A missing or unrecognised value is
/// the default for that half, so a hand-edited row degrades the display and
/// never fails the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StationDisplay {
    /// The clock.
    pub time: TimeFormat,
    /// The unit.
    pub temperature: TemperatureUnit,
}

impl StationDisplay {
    /// Read the two rows.
    pub fn load(conn: &rusqlite::Connection) -> Self {
        let get = |key| birdnet_db::settings::get(conn, key).ok();
        Self {
            time: get(TIME_FORMAT_KEY)
                .as_deref()
                .and_then(TimeFormat::parse)
                .unwrap_or_default(),
            temperature: get(TEMPERATURE_UNIT_KEY)
                .as_deref()
                .and_then(TemperatureUnit::parse)
                .unwrap_or_default(),
        }
    }
}

/// Split `HH:MM` or `HH:MM:SS` (zero-padded, in range) into its parts.
fn parse_clock(time: &str) -> Option<(u32, u32, Option<u32>)> {
    let bytes = time.as_bytes();
    let two = |i: usize| -> Option<u32> {
        let (a, b) = (*bytes.get(i)?, *bytes.get(i + 1)?);
        (a.is_ascii_digit() && b.is_ascii_digit())
            .then(|| u32::from(a - b'0') * 10 + u32::from(b - b'0'))
    };
    let (h, m) = (two(0)?, two(3)?);
    if bytes.get(2) != Some(&b':') || h > 23 || m > 59 {
        return None;
    }
    match bytes.len() {
        5 => Some((h, m, None)),
        8 if bytes.get(5) == Some(&b':') => {
            let s = two(6)?;
            (s <= 59).then_some((h, m, Some(s)))
        }
        _ => None,
    }
}

/// `time` printed in `format`; `None` when it is not `HH:MM` or `HH:MM:SS`.
///
/// 24-hour is the input unchanged. 12-hour has no leading zero on the hour and
/// `AM`/`PM` after a space; midnight is `12:xx AM` and noon `12:xx PM`.
#[must_use]
pub fn format_clock(time: &str, format: TimeFormat) -> Option<String> {
    let (h, m, s) = parse_clock(time)?;
    Some(match format {
        TimeFormat::H24 => time.to_owned(),
        TimeFormat::H12 => {
            let hour = match h % 12 {
                0 => 12,
                n => n,
            };
            let suffix = if h < 12 { "AM" } else { "PM" };
            s.map_or_else(
                || format!("{hour}:{m:02} {suffix}"),
                |s| format!("{hour}:{m:02}:{s:02} {suffix}"),
            )
        }
    })
}

/// A temperature in Celsius, printed in `unit` with `decimals` places.
///
/// Rounds half away from zero (so `20.5` is `21` and `-20.5` is `-21`, as the
/// browser's copy does; the standard formatter rounds half to even) and never
/// prints `-0`. `with_unit` adds `C` or `F` after the degree sign.
#[must_use]
pub fn format_temperature(
    celsius: f64,
    decimals: usize,
    unit: TemperatureUnit,
    with_unit: bool,
) -> String {
    let value = match unit {
        TemperatureUnit::Celsius => celsius,
        TemperatureUnit::Fahrenheit => celsius * 9.0 / 5.0 + 32.0,
    };
    let places = i32::try_from(decimals).unwrap_or(0);
    let scale = 10_f64.powi(places);
    let rounded = (value.abs() * scale).round() / scale;
    let sign = if value < 0.0 && rounded != 0.0 {
        "-"
    } else {
        ""
    };
    let letter = match (with_unit, unit) {
        (false, _) => "",
        (true, TemperatureUnit::Celsius) => "C",
        (true, TemperatureUnit::Fahrenheit) => "F",
    };
    format!("{sign}{rounded:.decimals$}°{letter}")
}

/// The attribute that marks an element as a clock time (for an element the
/// caller writes itself, such as an SVG `<text>`): `data-clk="16:31:37"`.
/// A value that is not a time is left unmarked.
#[must_use]
pub fn clock_attr(time: &str) -> String {
    if parse_clock(time).is_some() {
        format!(r#"data-clk="{time}""#)
    } else {
        String::new()
    }
}

/// A clock time as the HTML to put in a page: canonical text in an element the
/// browser re-prints in the reader's choice. A value that is not a time is
/// returned escaped and unmarked.
#[must_use]
pub fn clock(time: &str) -> String {
    if parse_clock(time).is_some() {
        format!(r#"<span class="clk" data-clk="{time}">{time}</span>"#)
    } else {
        crate::routes::pages::escape_html(time)
    }
}

/// The clock for an hour on the hour (`6` is `06:00`), as [`clock`]: for a
/// table of hours or a "peak hour" figure. An hour past 23 is not a time and is
/// printed as the number it is.
#[must_use]
pub fn clock_hour<H: TryInto<u32>>(hour: H) -> String {
    match hour.try_into() {
        Ok(h) if h <= 23 => clock(&format!("{h:02}:00")),
        Ok(h) => h.to_string(),
        Err(_) => String::new(),
    }
}

/// A time given as hours since midnight (`6.35`), as [`clock`] to the minute.
/// Wraps past midnight rather than printing `24:xx`.
#[must_use]
pub fn clock_from_hours(hours: f64) -> String {
    let total = (hours * 60.0).round().rem_euclid(1440.0);
    // `rem_euclid(1440)` is in 0..1440, so the cast is exact.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let total = total as u32;
    clock(&format!("{:02}:{:02}", total / 60, total % 60))
}

/// A stored timestamp, `YYYY-MM-DD HH:MM[:SS]` (a `T` also separates them), as
/// the HTML to put in a page: the date as written and the time through
/// [`clock`]. Anything else is returned escaped and unmarked.
#[must_use]
pub fn datetime(stamp: &str) -> String {
    match stamp.split_once([' ', 'T']) {
        Some((date, time))
            if date.len() == 10
                && parse_clock(time.get(..time.len().min(8)).unwrap_or("")).is_some() =>
        {
            let (clock_part, rest) = time.split_at(time.len().min(8));
            format!(
                "{} {}{}",
                crate::routes::pages::escape_html(date),
                clock(clock_part),
                crate::routes::pages::escape_html(rest)
            )
        }
        _ => crate::routes::pages::escape_html(stamp),
    }
}

/// The attributes that mark an element as a temperature: the Celsius value and
/// the places, and whether the unit letter is printed. For an element the
/// caller writes itself.
#[must_use]
pub fn temperature_attr(celsius: f64, decimals: usize, with_unit: bool) -> String {
    format!(
        r#"data-tc="{celsius:.4}" data-dp="{decimals}"{}"#,
        if with_unit { r#" data-tu="1""# } else { "" }
    )
}

/// A temperature as the HTML to put in a page (canonical Celsius text, marked).
#[must_use]
pub fn temperature(celsius: f64, decimals: usize, with_unit: bool) -> String {
    format!(
        r#"<span class="tmp" {}>{}</span>"#,
        temperature_attr(celsius, decimals, with_unit),
        format_temperature(celsius, decimals, TemperatureUnit::Celsius, with_unit)
    )
}

/// Mount `GET /api/v2/display`.
pub fn router() -> axum::Router<crate::state::AppState> {
    axum::Router::new().route("/api/v2/display", axum::routing::get(station_defaults))
}

/// The station's defaults, for the browser script to fall back to when the
/// reader has not chosen. Two short strings; revalidated on every use, because
/// an owner who changes the default expects the next page to follow it.
async fn station_defaults(
    axum::extract::State(state): axum::extract::State<crate::state::AppState>,
) -> axum::response::Response {
    use axum::response::IntoResponse as _;
    let d = state
        .run_blocking(|state| state.with_read_db(StationDisplay::load))
        .await;
    (
        [(axum::http::header::CACHE_CONTROL, "private, no-cache")],
        axum::Json(serde_json::json!({
            "time_format": d.time.as_str(),
            "temperature_unit": d.temperature.as_str(),
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CASES: &str = include_str!("../tests/data/display_cases.json");

    fn cases() -> serde_json::Value {
        serde_json::from_str(CASES).expect("the shared table parses")
    }

    #[test]
    fn the_clock_agrees_with_the_shared_table() {
        let table = cases();
        let rows = table["clock"].as_array().expect("clock cases");
        assert!(rows.len() >= 15, "the table is not a stub");
        for row in rows {
            let input = row[0].as_str().unwrap();
            let format = TimeFormat::parse(row[1].as_str().unwrap()).unwrap_or_default();
            let want = row[2].as_str();
            assert_eq!(
                format_clock(input, format).as_deref(),
                want,
                "{input:?} in {}",
                row[1]
            );
        }
    }

    #[test]
    fn the_temperature_agrees_with_the_shared_table() {
        let table = cases();
        let rows = table["temp"].as_array().expect("temperature cases");
        assert!(rows.len() >= 10);
        for row in rows {
            let unit = TemperatureUnit::parse(row[2].as_str().unwrap()).unwrap_or_default();
            let got = format_temperature(
                row[0].as_f64().unwrap(),
                usize::try_from(row[1].as_u64().unwrap()).unwrap(),
                unit,
                row[3].as_bool().unwrap(),
            );
            assert_eq!(got, row[4].as_str().unwrap(), "{row}");
        }
    }

    #[test]
    fn canonical_markup_carries_the_value_it_was_made_from() {
        assert_eq!(
            clock("16:31:37"),
            r#"<span class="clk" data-clk="16:31:37">16:31:37</span>"#
        );
        assert_eq!(clock("<b>"), "&lt;b&gt;", "not a time: escaped, unmarked");
        assert_eq!(clock_attr("nope"), "");
        assert_eq!(clock_attr("06:00"), r#"data-clk="06:00""#);
        let t = temperature(21.4, 0, true);
        assert!(
            t.contains(r#"data-tc="21.4000""#) && t.contains(r#"data-dp="0""#),
            "{t}"
        );
        assert!(
            t.contains(r#"data-tu="1""#) && t.ends_with(">21°C</span>"),
            "{t}"
        );
        assert!(!temperature(21.4, 0, false).contains("data-tu"));
    }

    #[test]
    fn fractional_hours_become_a_clock_that_wraps_at_midnight() {
        assert_eq!(clock_from_hours(6.35), clock("06:21"));
        assert_eq!(clock_from_hours(0.0), clock("00:00"));
        assert_eq!(clock_from_hours(19.5), clock("19:30"));
        assert_eq!(clock_from_hours(24.01), clock("00:01"));
        assert_eq!(clock_from_hours(-0.5), clock("23:30"));
    }

    #[test]
    fn an_hour_on_the_hour_is_a_clock() {
        assert_eq!(clock_hour(6_u32), clock("06:00"));
        assert_eq!(clock_hour(23_i64), clock("23:00"));
        assert_eq!(clock_hour(24_u32), "24");
        assert_eq!(clock_hour(-1_i64), "");
    }

    #[test]
    fn a_stamp_prints_its_date_as_written_and_its_time_through_the_clock() {
        assert_eq!(
            datetime("2026-10-21 16:31:37"),
            r#"2026-10-21 <span class="clk" data-clk="16:31:37">16:31:37</span>"#
        );
        assert_eq!(
            datetime("2026-10-21T06:05"),
            r#"2026-10-21 <span class="clk" data-clk="06:05">06:05</span>"#
        );
        // A zone suffix is kept after the time.
        assert!(datetime("2026-10-21T06:05:00Z").ends_with("</span>Z"));
        assert_eq!(datetime("yesterday <b>"), "yesterday &lt;b&gt;");
        assert_eq!(datetime(""), "");
    }

    #[test]
    fn the_stored_values_round_trip_and_nothing_else_is_accepted() {
        for f in [TimeFormat::H24, TimeFormat::H12] {
            assert_eq!(TimeFormat::parse(f.as_str()), Some(f));
        }
        for u in [TemperatureUnit::Celsius, TemperatureUnit::Fahrenheit] {
            assert_eq!(TemperatureUnit::parse(u.as_str()), Some(u));
        }
        for bad in ["", "12", "24", "12H", "kelvin", "c", "f"] {
            assert!(!is_time_format(bad) || bad == "12h", "{bad:?}");
            assert!(!is_temperature_unit(bad), "{bad:?}");
        }
        assert_eq!(StationDisplay::default().time, TimeFormat::H24);
        assert_eq!(
            StationDisplay::default().temperature,
            TemperatureUnit::Celsius
        );
    }
}
