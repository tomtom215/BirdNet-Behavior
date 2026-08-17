//! Where an imported history came from, and whether that is the same place the
//! station is standing.
//!
//! # Why this exists
//!
//! Until this module, an import was indistinguishable from a recording. The
//! importer copies every BirdNET-Pi row through verbatim, the validator's checks
//! are table-readable / non-empty / date-format / confidence-range, and the
//! destination had no column that said otherwise. So importing a *different*
//! station's history silently produced one database holding two sites and two
//! clocks, with nothing able to tell them apart afterwards.
//!
//! Every location- and hour-dependent analytic then reads the merged history as
//! one station:
//!
//! * solar overlays and the recording-window logic use the *configured* station
//!   coordinates, applied to detections recorded somewhere else;
//! * BirdNET-Pi stores `Date`/`Time` as local wall-clock with no offset
//!   recorded, so rows imported from another timezone are re-read as this
//!   station's local time — a six-hour offset puts the source station's dawn
//!   chorus in this station's afternoon;
//! * "first of year", life-list firsts and the species-richness curves are
//!   computed over the union, so another site's species become this site's
//!   records.
//!
//! For a research station that is not a cosmetic problem: the damage is not
//! detectable after the fact, so a dataset that quietly merged two sites cannot
//! be repaired, only discarded. This module is what makes it detectable *before*
//! the import, and (with `import_batches`, migration 25) attributable after.
//!
//! # What this module does and does not decide
//!
//! It reads the source and reports. It never blocks an import — merging two
//! sites is a legitimate thing to want, and only the operator knows whether
//! these two are the same site with a moved GPS fix or two sites a county apart.
//! The job here is to make sure that is a decision rather than an accident.

use std::path::Path;

use rusqlite::Connection;

use crate::error::MigrateError;
use crate::schema::open_source_readonly;

/// Distance past which two coordinates are reported as different sites.
///
/// Not a scientific threshold — there is no distance at which bird communities
/// become "different" — but a trigger for asking the operator. 5 km is well
/// beyond the scatter of consumer GPS fixes and of an operator typing their
/// coordinates twice from memory, and well inside the scale at which habitat,
/// sunrise time and species pool start to differ enough to matter.
pub const DIFFERENT_SITE_KM: f64 = 5.0;

/// Seconds of clock difference past which the two histories are reported as
/// being on different clocks.
///
/// Half an hour, because real timezone offsets are quantised to 15 minutes and
/// the smallest difference worth flagging is one such step plus slack.
pub const DIFFERENT_CLOCK_SECS: i64 = 1800;

/// What a source database says about the station that produced it.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SourceProfile {
    /// Rows carrying a usable coordinate pair.
    pub located_rows: u64,
    /// Rows with no coordinate at all.
    pub unlocated_rows: u64,
    /// The most common coordinate in the source, rounded to ~100 m.
    ///
    /// Modal rather than mean: a station that was moved once, or whose early
    /// rows carry a placeholder `0,0`, would have a mean somewhere in between —
    /// a coordinate at which the station has never stood. The mode is a place
    /// that actually appears in the data.
    pub modal_lat: Option<f64>,
    /// Longitude of [`Self::modal_lat`]'s coordinate.
    pub modal_lon: Option<f64>,
    /// How many rows carry the modal coordinate.
    pub modal_rows: u64,
    /// Distinct rounded coordinates present, so a source that is *itself* a
    /// merge of several sites is visible rather than reported as its mode.
    pub distinct_sites: u64,
    /// Earliest parseable detection date, `YYYY-MM-DD`.
    pub first_date: Option<String>,
    /// Latest parseable detection date.
    pub last_date: Option<String>,
}

impl SourceProfile {
    /// Read a source database's location profile.
    ///
    /// Opens read-only and never writes. A source with no `Lat`/`Lon` columns at
    /// all (some CSV exports) yields a profile with no coordinates rather than
    /// an error — that is a fact about the source, not a failure to read it.
    ///
    /// # Errors
    ///
    /// Returns `MigrateError` if the source cannot be opened.
    pub fn read(source_path: &Path) -> Result<Self, MigrateError> {
        let conn = open_source_readonly(source_path)?;
        Ok(Self::from_connection(&conn))
    }

    /// Profile an already-open source connection.
    ///
    /// Every query is fallible-but-optional: a source lacking the columns is
    /// described as "no coordinates", which is what the caller needs to know.
    #[must_use]
    pub fn from_connection(conn: &Connection) -> Self {
        let mut profile = Self::default();

        if let Ok((located, unlocated)) = conn.query_row(
            "SELECT
                 SUM(CASE WHEN Lat IS NOT NULL AND Lon IS NOT NULL
                           AND NOT (Lat = 0 AND Lon = 0) THEN 1 ELSE 0 END),
                 SUM(CASE WHEN Lat IS NULL OR Lon IS NULL
                           OR (Lat = 0 AND Lon = 0) THEN 1 ELSE 0 END)
               FROM detections",
            [],
            |r| {
                Ok((
                    r.get::<_, Option<i64>>(0)?.unwrap_or(0),
                    r.get::<_, Option<i64>>(1)?.unwrap_or(0),
                ))
            },
        ) {
            profile.located_rows = u64::try_from(located).unwrap_or(0);
            profile.unlocated_rows = u64::try_from(unlocated).unwrap_or(0);
        }

        // Round to 3 decimal places (~110 m) before grouping. Raw floats would
        // make every row its own "site" on a station whose GPS jitters.
        if let Ok((lat, lon, n)) = conn.query_row(
            "SELECT ROUND(Lat, 3), ROUND(Lon, 3), COUNT(*) c
               FROM detections
              WHERE Lat IS NOT NULL AND Lon IS NOT NULL
                AND NOT (Lat = 0 AND Lon = 0)
              GROUP BY 1, 2
              ORDER BY c DESC
              LIMIT 1",
            [],
            |r| {
                Ok((
                    r.get::<_, f64>(0)?,
                    r.get::<_, f64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            },
        ) {
            profile.modal_lat = Some(lat);
            profile.modal_lon = Some(lon);
            profile.modal_rows = u64::try_from(n).unwrap_or(0);
        }

        if let Ok(n) = conn.query_row(
            "SELECT COUNT(*) FROM (
                 SELECT 1 FROM detections
                  WHERE Lat IS NOT NULL AND Lon IS NOT NULL
                    AND NOT (Lat = 0 AND Lon = 0)
                  GROUP BY ROUND(Lat, 3), ROUND(Lon, 3))",
            [],
            |r| r.get::<_, i64>(0),
        ) {
            profile.distinct_sites = u64::try_from(n).unwrap_or(0);
        }

        if let Ok((first, last)) = conn.query_row(
            "SELECT MIN(Date), MAX(Date) FROM detections WHERE date(Date) IS NOT NULL",
            [],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                ))
            },
        ) {
            profile.first_date = first;
            profile.last_date = last;
        }

        profile
    }

    /// Great-circle distance from this source's modal coordinate to the
    /// station's, in kilometres. `None` when either coordinate is unknown.
    #[must_use]
    pub fn distance_km_to(
        &self,
        station_lat: Option<f64>,
        station_lon: Option<f64>,
    ) -> Option<f64> {
        Some(haversine_km(
            self.modal_lat?,
            self.modal_lon?,
            station_lat?,
            station_lon?,
        ))
    }
}

/// Great-circle distance between two coordinates, in kilometres.
///
/// Haversine on a spherical Earth. Accurate to ~0.5 % — far tighter than
/// anything this is used to decide, and it needs no dependency.
#[must_use]
pub fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    /// Mean Earth radius, km (IUGG).
    const R_KM: f64 = 6_371.008_8;
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let (dp, dl) = ((lat2 - lat1).to_radians(), (lon2 - lon1).to_radians());
    let a = (dp / 2.0).sin().mul_add(
        (dp / 2.0).sin(),
        p1.cos() * p2.cos() * (dl / 2.0).sin() * (dl / 2.0).sin(),
    );
    2.0 * R_KM * a.sqrt().clamp(0.0, 1.0).asin()
}

/// How an import should be reconciled with the station receiving it.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ImportOptions {
    /// Seconds to add to every imported timestamp.
    ///
    /// BirdNET-Pi stores local wall-clock with no offset, so a history recorded
    /// at UTC−5 and imported into a UTC+1 station is six hours out and nothing
    /// in the data says so. This is the correction, applied once at import:
    /// `source_utc_offset − destination_utc_offset`, in seconds.
    ///
    /// Zero means "these two stations keep the same clock", which is the common
    /// case and the safe default — it changes nothing.
    pub shift_secs: i64,
    /// Operator's name for the source station, recorded with the batch.
    pub label: Option<String>,
    /// The source station's UTC offset, as stated by the operator, recorded so
    /// the shift can be explained (and undone) later.
    pub source_utc_offset_secs: Option<i64>,
    /// Free-text note stored with the batch.
    pub notes: Option<String>,
}

impl ImportOptions {
    /// Whether this import rewrites timestamps.
    #[must_use]
    pub const fn shifts_time(&self) -> bool {
        self.shift_secs != 0
    }
}

/// A pre-import verdict on whether the source and the station are the same
/// place, on the same clock.
///
/// Returned as a [`ValidationCheck`] so it lands in the same report the
/// operator already reads, rather than in a second place they have to know to
/// look. It is never `required`: merging two sites is a legitimate thing to
/// want, and only the operator can say whether these two coordinates are one
/// station whose GPS fix moved or two sites a county apart. The job is to make
/// that a decision instead of an accident.
#[must_use]
pub fn location_check(
    profile: &SourceProfile,
    station_lat: Option<f64>,
    station_lon: Option<f64>,
) -> crate::traits::ValidationCheck {
    use crate::traits::ValidationCheck;

    if profile.distinct_sites > 1 {
        return ValidationCheck::fail(
            "source_location",
            format!(
                "this file already contains detections from {} different \
                 coordinates — it is itself a merge of several sites, so importing \
                 it cannot be attributed to one place",
                profile.distinct_sites
            ),
            false,
        );
    }

    let Some(distance) = profile.distance_km_to(station_lat, station_lon) else {
        return if profile.located_rows == 0 {
            ValidationCheck::fail(
                "source_location",
                "this file records no coordinates, so there is no way to check \
                 whether it came from this station. If it did not, every \
                 location- and hour-based analytic will read the merged history \
                 as one site"
                    .to_string(),
                false,
            )
        } else {
            ValidationCheck::fail(
                "source_location",
                "this station has no latitude/longitude set, so the imported \
                 data cannot be checked against it. Set the station location in \
                 Settings first"
                    .to_string(),
                false,
            )
        };
    };

    if distance <= DIFFERENT_SITE_KM {
        return ValidationCheck::pass(
            "source_location",
            format!("recorded {distance:.1} km from this station — the same site"),
        );
    }

    ValidationCheck::fail(
        "source_location",
        format!(
            "recorded {distance:.0} km from this station. These are different \
             sites: sunrise, habitat and species pool all differ, and BirdNET-Pi \
             stores local wall-clock time with no timezone, so the imported \
             hours are in the source station's clock. Set the source's UTC \
             offset below so the two histories share one clock; the import will \
             be tagged with its origin either way"
        ),
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn source_with(rows: &[(f64, f64)]) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE detections (Date TEXT, Time TEXT, Sci_Name TEXT, Com_Name TEXT,
             Confidence REAL, Lat REAL, Lon REAL);",
        )
        .unwrap();
        for (i, (lat, lon)) in rows.iter().enumerate() {
            conn.execute(
                "INSERT INTO detections VALUES (?1,'06:00:00','Turdus merula','Blackbird',0.9,?2,?3)",
                rusqlite::params![format!("2026-01-{:02}", (i % 28) + 1), lat, lon],
            )
            .unwrap();
        }
        conn
    }

    #[test]
    fn haversine_matches_a_known_distance() {
        // London (51.5074, -0.1278) to Paris (48.8566, 2.3522): ~343 km.
        let d = haversine_km(51.5074, -0.1278, 48.8566, 2.3522);
        assert!((d - 343.0).abs() < 5.0, "d={d}");
    }

    #[test]
    fn haversine_is_zero_for_the_same_point() {
        assert!(haversine_km(42.36, -71.06, 42.36, -71.06) < 1e-9);
    }

    /// The mode, not the mean. A station that moved once has a mean coordinate
    /// at which it has never stood; reporting that as "where this data came
    /// from" would be worse than reporting nothing.
    #[test]
    fn modal_site_is_the_commonest_not_the_average() {
        // Seven rows at London, three at Paris. The mean sits in the Channel.
        let mut rows = vec![(51.5074, -0.1278); 7];
        rows.extend(vec![(48.8566, 2.3522); 3]);
        let p = SourceProfile::from_connection(&source_with(&rows));
        assert_eq!(p.modal_rows, 7);
        assert!((p.modal_lat.unwrap() - 51.507).abs() < 0.001);
        assert_eq!(p.distinct_sites, 2, "a two-site source reports two sites");
    }

    #[test]
    fn placeholder_zero_zero_is_not_a_location() {
        let p = SourceProfile::from_connection(&source_with(&[(0.0, 0.0), (0.0, 0.0)]));
        assert_eq!(p.located_rows, 0, "0,0 is BirdNET-Pi's unset marker");
        assert_eq!(p.unlocated_rows, 2);
        assert_eq!(p.modal_lat, None);
    }

    #[test]
    fn distance_is_none_when_either_side_is_unknown() {
        let p = SourceProfile::from_connection(&source_with(&[(51.5, -0.1)]));
        assert!(p.distance_km_to(None, None).is_none());
        assert!(p.distance_km_to(Some(48.85), None).is_none());
        let d = p.distance_km_to(Some(48.8566), Some(2.3522)).unwrap();
        assert!((d - 343.0).abs() < 5.0, "d={d}");
    }

    #[test]
    fn a_source_without_coordinate_columns_profiles_as_unlocated() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE detections (Date TEXT, Time TEXT, Sci_Name TEXT);")
            .unwrap();
        let p = SourceProfile::from_connection(&conn);
        assert_eq!(p.modal_lat, None);
        assert_eq!(p.located_rows, 0);
    }

    #[test]
    fn date_range_is_reported() {
        let p = SourceProfile::from_connection(&source_with(&[(51.5, -0.1); 5]));
        assert_eq!(p.first_date.as_deref(), Some("2026-01-01"));
        assert_eq!(p.last_date.as_deref(), Some("2026-01-05"));
    }

    fn profile_at(lat: f64, lon: f64) -> SourceProfile {
        SourceProfile::from_connection(&source_with(&[(lat, lon); 3]))
    }

    /// The same site passes. Written alongside the failing case below because a
    /// check that fires on everything is not a check.
    #[test]
    fn location_check_passes_for_the_same_site() {
        let c = location_check(&profile_at(51.5074, -0.1278), Some(51.5074), Some(-0.1278));
        assert!(c.passed, "{}", c.detail);
        assert!(c.detail.contains("same site"));
    }

    /// GPS scatter and a re-typed coordinate must not trip it.
    #[test]
    fn location_check_tolerates_a_moved_fix_within_the_threshold() {
        // ~1.5 km apart — a garden, re-surveyed.
        let c = location_check(&profile_at(51.5074, -0.1278), Some(51.52), Some(-0.128));
        assert!(c.passed, "{}", c.detail);
    }

    #[test]
    fn location_check_fails_for_a_different_site_and_says_why() {
        let c = location_check(&profile_at(48.8566, 2.3522), Some(51.5074), Some(-0.1278));
        assert!(!c.passed);
        assert!(!c.required, "a different site is a warning, never a block");
        // ~343 km; the modal coordinate is rounded to 3 dp before the
        // distance is taken, so assert the magnitude rather than the digit.
        let km: f64 = c
            .detail
            .split_whitespace()
            .nth(1)
            .and_then(|t| t.parse().ok())
            .unwrap_or_default();
        assert!((km - 343.0).abs() < 5.0, "detail was: {}", c.detail);
        assert!(
            c.detail.contains("clock"),
            "the timezone consequence must be stated, not just the distance"
        );
    }

    #[test]
    fn location_check_reports_a_source_that_is_itself_a_merge() {
        let mut rows = vec![(51.5074, -0.1278); 3];
        rows.extend(vec![(48.8566, 2.3522); 2]);
        let p = SourceProfile::from_connection(&source_with(&rows));
        let c = location_check(&p, Some(51.5074), Some(-0.1278));
        assert!(!c.passed);
        assert!(c.detail.contains("2 different"), "detail: {}", c.detail);
    }

    #[test]
    fn location_check_distinguishes_no_source_coords_from_no_station_coords() {
        let unlocated = SourceProfile::from_connection(&source_with(&[(0.0, 0.0); 2]));
        let a = location_check(&unlocated, Some(51.5), Some(-0.1));
        assert!(!a.passed);
        assert!(a.detail.contains("records no coordinates"), "{}", a.detail);

        let b = location_check(&profile_at(51.5, -0.1), None, None);
        assert!(!b.passed);
        assert!(
            b.detail.contains("no latitude/longitude set"),
            "{}",
            b.detail
        );
    }

    #[test]
    fn shifts_time_is_false_only_at_zero() {
        assert!(!ImportOptions::default().shifts_time());
        assert!(
            ImportOptions {
                shift_secs: -3600,
                ..Default::default()
            }
            .shifts_time()
        );
    }
}
