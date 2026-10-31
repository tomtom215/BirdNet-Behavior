//! The data behind Today's "Sky of songs": every accepted detection of the day, placed in time
//! and by how sure the station was, with the day's sun so the picture can paint the real sky.
//!
//! `GET /pages/today-songs` answers JSON for `static/skysong.js`. The shape is the script's
//! contract (documented there): species most heard first (the top eight named, everyone else as
//! one "Other species" band), and one `[t, c, s, href]` object per song. Nothing below the
//! station's own accept cut-off is ever in the list, so the floor the picture draws is a fact and
//! not a decoration.
#![deny(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use birdnet_db::sqlite::{DetectionRow, TodayFilter};
use serde_json::{Value, json};

use super::{name_search, simple_url_encode, today_date_string};
use crate::state::AppState;

/// The most songs one answer carries. A station that hears more in a day than this is shown its
/// most recent: the picture is of the day so far, and a 2,000-mark scatter is already dense.
const MAX_SONGS: u32 = 2000;
/// Species given their own colour; the rest share the neutral one.
const NAMED: usize = 8;

/// `HH:MM:SS` (or `HH:MM`) as seconds since midnight; `None` for anything else.
fn clock_secs(time: &str) -> Option<u32> {
    let mut parts = time.split(':');
    let h: u32 = parts.next()?.trim().parse().ok()?;
    let m: u32 = parts.next()?.trim().parse().ok()?;
    let s: u32 = parts.next().map_or(Some(0), |p| p.trim().parse().ok())?;
    (h < 24 && m < 60 && s < 60).then_some(h * 3600 + m * 60 + s)
}

/// What the station knows of today's sun, in local fractional hours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Sun {
    pub rise: f64,
    pub set: f64,
    pub dawn: Option<f64>,
    pub dusk: Option<f64>,
}

/// The feed's JSON for `date`: `rows` are the day's accepted detections (any order), `floor` the
/// station's accept cut-off in `0..=1`, `now_secs` the local time of day (`None` for a day that
/// is over), `names` the station's language. Species are ranked by how often they were heard,
/// ties by name, so the same day always draws the same colours.
pub(super) fn songs_json(
    date: &str,
    now_secs: Option<u32>,
    sun: Option<Sun>,
    floor: f64,
    rows: &[DetectionRow],
    names: &birdnet_core::i18n::NameLocalizer,
) -> Value {
    let mut tally: HashMap<&str, (usize, &str)> = HashMap::new();
    for r in rows {
        tally
            .entry(r.com_name.as_str())
            .or_insert((0, r.sci_name.as_str()))
            .0 += 1;
    }
    let mut ranked: Vec<(&str, usize, &str)> = tally
        .into_iter()
        .map(|(com, (n, sci))| (com, n, sci))
        .collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    let index_of: HashMap<&str, usize> = ranked
        .iter()
        .take(NAMED)
        .enumerate()
        .map(|(i, r)| (r.0, i))
        .collect();

    let mut species: Vec<Value> = ranked
        .iter()
        .take(NAMED)
        .enumerate()
        .map(|(i, (com, _, sci))| json!({ "name": names.name(sci, com), "slot": i + 1 }))
        .collect();
    let mut other_used = false;
    let songs: Vec<Value> = rows
        .iter()
        .filter_map(|r| {
            let t = clock_secs(&r.time)?;
            let s = index_of
                .get(r.com_name.as_str())
                .copied()
                .unwrap_or_else(|| {
                    other_used = true;
                    NAMED.min(ranked.len())
                });
            Some(json!({
                "t": t,
                "c": r.confidence,
                "s": s,
                "href": format!(
                    "/detections/detail?date={}&time={}&name={}",
                    simple_url_encode(&r.date),
                    simple_url_encode(&r.time),
                    simple_url_encode(&r.com_name),
                ),
            }))
        })
        .collect();
    if other_used {
        species.push(json!({ "name": "Other species", "slot": 0 }));
    }
    json!({
        "date": date,
        "nowSec": now_secs,
        "sun": sun.map(|s| json!({ "rise": s.rise, "set": s.set, "dawn": s.dawn, "dusk": s.dusk })),
        "floor": floor,
        "species": species,
        "songs": songs,
    })
}

/// `GET /pages/today-songs`.
pub(super) async fn today_songs(State(state): State<AppState>) -> impl IntoResponse {
    let today = today_date_string();
    let names_state = state.clone();
    let day = today.clone();
    let result = tokio::task::spawn_blocking(move || {
        state.with_read_db(|conn| {
            let by = name_search::todays_filter(&day, None, TodayFilter::All, state.names());
            let rows = birdnet_db::sqlite::search_detections(conn, &by, MAX_SONGS, 0)?;
            let total = birdnet_db::sqlite::search_detection_count(conn, &by)?;
            let floor = super::confidence::station_floor(conn);
            let sun = super::solar_times_local(conn).map(|(rise, set)| {
                let (dawn, dusk) = super::civil_twilight_local(conn).unwrap_or((None, None));
                Sun {
                    rise,
                    set,
                    dawn,
                    dusk,
                }
            });
            Ok::<_, birdnet_db::sqlite::DbError>((rows, floor, sun, total))
        })
    })
    .await;
    match result {
        Ok(Ok((rows, floor, sun, total))) => {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let now = (super::now_hour_local() * 3600.0).clamp(0.0, 86_399.0) as u32;
            let mut body = songs_json(&today, Some(now), sun, floor, &rows, names_state.names());
            // When the day holds more songs than one answer carries, say how many it holds, so the
            // picture and its summary can say they show part of it.
            if i64::try_from(rows.len()).is_ok_and(|n| total > n) {
                body["total"] = serde_json::json!(total);
            }
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, "application/json"),
                    (header::CACHE_CONTROL, "no-store"),
                ],
                body.to_string(),
            )
        }
        failed => {
            // The script keeps the picture it has and says it could not refresh; the reason is the log's.
            tracing::warn!(error = ?failed.err(), "today songs: query or task failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                [
                    (header::CONTENT_TYPE, "application/json"),
                    (header::CACHE_CONTROL, "no-store"),
                ],
                r#"{"error":"could not read today's songs"}"#.to_string(),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(com: &str, time: &str, conf: f64) -> DetectionRow {
        DetectionRow {
            date: "2026-10-31".into(),
            time: time.into(),
            sci_name: format!("Sci {com}"),
            com_name: com.into(),
            confidence: conf,
            lat: None,
            lon: None,
            cutoff: None,
            week: None,
            sens: None,
            overlap: None,
            file_name: None,
            ..DetectionRow::default()
        }
    }

    fn english() -> birdnet_core::i18n::NameLocalizer {
        birdnet_core::i18n::NameLocalizer::english()
    }

    #[test]
    fn a_clock_reads_as_seconds_and_nonsense_is_refused() {
        assert_eq!(clock_secs("05:12:09"), Some(5 * 3600 + 12 * 60 + 9));
        assert_eq!(clock_secs("23:59"), Some(23 * 3600 + 59 * 60));
        for bad in ["", "24:00:00", "12:61:00", "ab:cd", "12:00:99"] {
            assert_eq!(clock_secs(bad), None, "{bad:?}");
        }
    }

    /// The most-heard species gets slot 1, every song points at its own species, and a ninth
    /// species is "Other species" with slot 0, which exists in the list only when somebody uses it.
    #[test]
    fn species_are_ranked_by_how_often_they_were_heard_and_the_rest_share_one_band() {
        let mut rows = Vec::new();
        for i in 0..9 {
            // Species 0 heard 9 times, species 1 eight times, ... species 8 once.
            for k in 0..(9 - i) {
                rows.push(row(&format!("Bird {i}"), &format!("06:{k:02}:00"), 0.9));
            }
        }
        let v = songs_json("2026-10-31", Some(40_000), None, 0.7, &rows, &english());
        let sp = v["species"].as_array().expect("species");
        assert_eq!(sp.len(), 9, "eight named and one band for the rest");
        assert_eq!(sp[0]["name"], "Bird 0");
        assert_eq!(sp[0]["slot"], 1);
        assert_eq!(sp[8]["name"], "Other species");
        assert_eq!(sp[8]["slot"], 0);
        let songs = v["songs"].as_array().expect("songs");
        assert_eq!(songs.len(), rows.len());
        let in_other = songs.iter().filter(|s| s["s"] == 8).count();
        assert_eq!(in_other, 1, "only Bird 8 (heard once) is in the other band");
        // Eight species and no more: no empty "Other species" entry.
        let few: Vec<_> = rows
            .iter()
            .filter(|r| r.com_name != "Bird 8")
            .cloned()
            .collect();
        let v = songs_json("2026-10-31", None, None, 0.7, &few, &english());
        assert_eq!(v["species"].as_array().map(Vec::len), Some(8));
    }

    /// A row whose time cannot be placed is left out rather than drawn at midnight, and the sun is
    /// `null` when the station has no location (no invented sky).
    #[test]
    fn an_unplaceable_song_is_dropped_and_no_location_means_no_sun() {
        let rows = vec![
            row("Blue Jay", "garbage", 0.9),
            row("Blue Jay", "08:14:00", 0.87),
        ];
        let v = songs_json("2026-10-31", None, None, 0.7, &rows, &english());
        assert_eq!(v["songs"].as_array().map(Vec::len), Some(1));
        assert!(v["sun"].is_null());
        assert!(v["nowSec"].is_null());
        assert_eq!(v["songs"][0]["t"], 8 * 3600 + 14 * 60);
        let href = v["songs"][0]["href"].as_str().expect("href");
        assert!(
            href.starts_with("/detections/detail?date=2026-10-31&time=08"),
            "{href}"
        );
        let with = songs_json(
            "2026-10-31",
            Some(1),
            Some(Sun {
                rise: 6.5,
                set: 17.0,
                dawn: Some(5.9),
                dusk: None,
            }),
            0.7,
            &rows,
            &english(),
        );
        assert_eq!(with["sun"]["rise"], 6.5);
        assert!(with["sun"]["dusk"].is_null());
    }
}
