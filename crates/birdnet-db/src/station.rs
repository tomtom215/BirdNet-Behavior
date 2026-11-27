//! Who the station is, where each source was deployed, and which device each
//! source was (RES-22, MSC-13).
//!
//! Three records, all written down rather than inferred:
//!
//! * the **station identity**, one random UUID minted when the schema is
//!   created. A detection, an export or a run names the station by it.
//! * **deployments**: what an operator *states* about where and how a source
//!   listened — from when, at what coordinates and height, facing which way,
//!   with which microphone and recorder. Nothing here measures any of it; a
//!   height is what someone typed. A deployment is a record: once stated it is
//!   not rewritten (the schema's triggers refuse it), and the next deployment
//!   of the same source closes the open one.
//! * **source devices**: which device string a source used, and when. A
//!   source's id is the identity everything else keys on and stays valid; its
//!   `device_seq` tells a researcher whether two stretches of one source were
//!   the same microphone or camera. Re-adding the same device keeps its
//!   number; editing a URL to another camera starts a new one. The table is
//!   written by triggers on `audio_sources`, so every path that changes a
//!   source writes it.
//!
//! Two device strings that name one device (`hw:1,0` and `plughw:1,0`) are
//! two numbers here: the record compares what was stored, so it can split one
//! device's history in two, but it cannot merge two devices into one.

use rusqlite::{Connection, OptionalExtension, params};
use std::fmt;

/// What can go wrong reading or writing these records.
#[derive(Debug)]
pub enum StationError {
    /// `SQLite` refused, including a trigger refusing an overlapping or
    /// rewritten deployment.
    Sqlite(rusqlite::Error),
    /// The input was refused before it reached the database.
    Invalid(String),
}

impl fmt::Display for StationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(e) => write!(f, "station record: {e}"),
            Self::Invalid(m) => write!(f, "invalid deployment: {m}"),
        }
    }
}

impl std::error::Error for StationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(e) => Some(e),
            Self::Invalid(_) => None,
        }
    }
}

impl From<rusqlite::Error> for StationError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}

/// The station's identity.
///
/// # Errors
///
/// Any `SQLite` error, including the table missing on a database older than
/// the migration that creates it. Never a made-up id.
pub fn station_uuid(conn: &Connection) -> Result<String, StationError> {
    Ok(conn.query_row(
        "SELECT station_uuid FROM station_identity WHERE id = 1",
        [],
        |r| r.get(0),
    )?)
}

/// One deployment as stated.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Deployment {
    /// Row id; what an export names the deployment by.
    pub id: i64,
    /// The station it belongs to.
    pub station_uuid: String,
    /// The audio source it describes.
    pub source_id: String,
    /// Start, Unix seconds UTC, inclusive.
    pub start_utc: i64,
    /// End, Unix seconds UTC, exclusive; `None` while it is the current one.
    pub end_utc: Option<i64>,
    /// Stated fields.
    #[serde(flatten)]
    pub stated: DeploymentFields,
    /// When this record was written, Unix seconds UTC.
    pub recorded_at_utc: i64,
}

/// What an operator states about a deployment. Every field is optional; an
/// empty one is "not stated", never zero.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct DeploymentFields {
    /// Decimal degrees, WGS 84.
    pub latitude: Option<f64>,
    /// Decimal degrees, WGS 84.
    pub longitude: Option<f64>,
    /// How far the true position may be from the stated one, metres.
    pub coord_uncertainty_m: Option<f64>,
    /// Ground elevation, metres.
    pub elevation_m: Option<f64>,
    /// Microphone height above the ground, metres, as stated.
    pub height_m: Option<f64>,
    /// Which way the microphone faces, degrees clockwise from true north, as
    /// stated.
    pub orientation_deg: Option<f64>,
    /// Microphone make and model.
    pub microphone: Option<String>,
    /// The recorder or sound card.
    pub recorder: Option<String>,
    /// Enclosure and windscreen.
    pub enclosure: Option<String>,
    /// Habitat around the microphone.
    pub habitat: Option<String>,
    /// Anything else.
    pub notes: Option<String>,
}

/// A deployment to open.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct NewDeployment {
    /// The audio source it describes; must exist.
    pub source_id: String,
    /// Start, Unix seconds UTC.
    pub start_utc: i64,
    /// What is stated about it.
    #[serde(flatten)]
    pub stated: DeploymentFields,
}

fn in_range(
    name: &str,
    v: Option<f64>,
    lo: f64,
    hi: f64,
    hi_open: bool,
) -> Result<(), StationError> {
    let Some(v) = v else { return Ok(()) };
    let ok = v.is_finite() && v >= lo && if hi_open { v < hi } else { v <= hi };
    if ok {
        Ok(())
    } else {
        Err(StationError::Invalid(format!(
            "{name} {v} is outside {lo}..{hi}"
        )))
    }
}

fn validate(new: &NewDeployment) -> Result<(), StationError> {
    let s = &new.stated;
    in_range("latitude", s.latitude, -90.0, 90.0, false)?;
    in_range("longitude", s.longitude, -180.0, 180.0, false)?;
    in_range(
        "coordinate uncertainty",
        s.coord_uncertainty_m,
        0.0,
        f64::MAX,
        false,
    )?;
    in_range("elevation", s.elevation_m, -500.0, 9000.0, false)?;
    in_range("height", s.height_m, 0.0, 1000.0, false)?;
    in_range("orientation", s.orientation_deg, 0.0, 360.0, true)?;
    if s.latitude.is_some() != s.longitude.is_some() {
        return Err(StationError::Invalid(
            "latitude and longitude are stated together or not at all".to_owned(),
        ));
    }
    if new.start_utc < 0 {
        return Err(StationError::Invalid("a start before 1970".to_owned()));
    }
    Ok(())
}

/// State a deployment of a source from `start_utc`.
///
/// The source's open deployment is closed there. Refused when the source does
/// not exist, when it would start at or before the open one's start, or when
/// it would overlap a closed one (the schema's trigger).
///
/// # Errors
///
/// [`StationError::Invalid`] for a refused input, [`StationError::Sqlite`]
/// otherwise. Nothing is written on an error.
pub fn open_deployment(
    conn: &Connection,
    new: &NewDeployment,
    now_utc: i64,
) -> Result<Deployment, StationError> {
    validate(new)?;
    let tx = conn.unchecked_transaction()?;
    let exists: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM audio_sources WHERE id = ?1)",
        [&new.source_id],
        |r| r.get(0),
    )?;
    if !exists {
        return Err(StationError::Invalid(format!(
            "no audio source {}",
            new.source_id
        )));
    }
    let open: Option<i64> = tx
        .query_row(
            "SELECT start_utc FROM deployments WHERE source_id = ?1 AND end_utc IS NULL",
            [&new.source_id],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(open_start) = open {
        if new.start_utc <= open_start {
            return Err(StationError::Invalid(
                "a new deployment starts after the current one did".to_owned(),
            ));
        }
        tx.execute(
            "UPDATE deployments SET end_utc = ?2 WHERE source_id = ?1 AND end_utc IS NULL",
            params![new.source_id, new.start_utc],
        )?;
    }
    let s = &new.stated;
    tx.execute(
        "INSERT INTO deployments (station_uuid, source_id, start_utc, latitude, longitude,
             coord_uncertainty_m, elevation_m, height_m, orientation_deg, microphone, recorder,
             enclosure, habitat, notes, recorded_at_utc)
         SELECT station_uuid, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14
           FROM station_identity WHERE id = 1",
        params![
            new.source_id,
            new.start_utc,
            s.latitude,
            s.longitude,
            s.coord_uncertainty_m,
            s.elevation_m,
            s.height_m,
            s.orientation_deg,
            s.microphone,
            s.recorder,
            s.enclosure,
            s.habitat,
            s.notes,
            now_utc,
        ],
    )?;
    let id = tx.last_insert_rowid();
    let row = tx.query_row(
        &format!("{DEPLOYMENT_SELECT} WHERE id = ?1"),
        [id],
        row_to_deployment,
    )?;
    tx.commit()?;
    Ok(row)
}

/// Close a source's open deployment at `end_utc` (the source was taken down).
/// Returns whether one was open.
///
/// # Errors
///
/// [`StationError::Sqlite`], including an end at or before its start.
pub fn close_deployment(
    conn: &Connection,
    source_id: &str,
    end_utc: i64,
) -> Result<bool, StationError> {
    Ok(conn.execute(
        "UPDATE deployments SET end_utc = ?2 WHERE source_id = ?1 AND end_utc IS NULL",
        params![source_id, end_utc],
    )? > 0)
}

const DEPLOYMENT_SELECT: &str = "SELECT id, station_uuid, source_id, start_utc, end_utc, latitude,
        longitude, coord_uncertainty_m, elevation_m, height_m, orientation_deg, microphone,
        recorder, enclosure, habitat, notes, recorded_at_utc FROM deployments";

fn row_to_deployment(r: &rusqlite::Row<'_>) -> rusqlite::Result<Deployment> {
    Ok(Deployment {
        id: r.get(0)?,
        station_uuid: r.get(1)?,
        source_id: r.get(2)?,
        start_utc: r.get(3)?,
        end_utc: r.get(4)?,
        stated: DeploymentFields {
            latitude: r.get(5)?,
            longitude: r.get(6)?,
            coord_uncertainty_m: r.get(7)?,
            elevation_m: r.get(8)?,
            height_m: r.get(9)?,
            orientation_deg: r.get(10)?,
            microphone: r.get(11)?,
            recorder: r.get(12)?,
            enclosure: r.get(13)?,
            habitat: r.get(14)?,
            notes: r.get(15)?,
        },
        recorded_at_utc: r.get(16)?,
    })
}

/// Every deployment, by source then start.
///
/// # Errors
///
/// Any `SQLite` error.
pub fn list_deployments(conn: &Connection) -> Result<Vec<Deployment>, StationError> {
    let mut stmt = conn.prepare(&format!(
        "{DEPLOYMENT_SELECT} ORDER BY source_id, start_utc"
    ))?;
    let rows = stmt
        .query_map([], row_to_deployment)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// One stretch of time a source used one device.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SourceDevice {
    /// The source.
    pub source_id: String,
    /// The source's n-th distinct device, from 1.
    pub device_seq: i64,
    /// The source kind at the time.
    pub kind: String,
    /// The device string as stored. An RTSP URL's credentials are stripped by
    /// [`list_source_devices`]'s callers that show it, not here.
    pub device_id: String,
    /// Start, Unix seconds UTC; `None` when it began before this was recorded
    /// and the source had been edited since, so its start is unknown.
    pub from_utc: Option<i64>,
    /// End, Unix seconds UTC; `None` while it is current.
    pub to_utc: Option<i64>,
}

/// Every device record, by source and order written.
///
/// # Errors
///
/// Any `SQLite` error.
pub fn list_source_devices(conn: &Connection) -> Result<Vec<SourceDevice>, StationError> {
    let mut stmt = conn.prepare(
        "SELECT source_id, device_seq, kind, device_id, from_utc, to_utc
           FROM audio_source_devices ORDER BY source_id, id",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(SourceDevice {
                source_id: r.get(0)?,
                device_seq: r.get(1)?,
                kind: r.get(2)?,
                device_id: r.get(3)?,
                from_utc: r.get(4)?,
                to_utc: r.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// The deployments and device records in memory, for attributing many rows
/// (an export) without a query per row.
#[derive(Debug, Clone, Default)]
pub struct SourceProvenance {
    /// The station's identity, when it could be read.
    pub station_uuid: Option<String>,
    deployments: Vec<Deployment>,
    devices: Vec<SourceDevice>,
}

impl SourceProvenance {
    /// Read both records.
    ///
    /// # Errors
    ///
    /// Any `SQLite` error; a missing station identity is an error too, since
    /// every database at this schema has one.
    pub fn load(conn: &Connection) -> Result<Self, StationError> {
        Ok(Self {
            station_uuid: Some(station_uuid(conn)?),
            deployments: list_deployments(conn)?,
            devices: list_source_devices(conn)?,
        })
    }

    /// The deployment of `source` in force at `utc`, if one was stated.
    #[must_use]
    pub fn deployment_at(&self, source: &str, utc: i64) -> Option<&Deployment> {
        self.deployments.iter().find(|d| {
            d.source_id == source && d.start_utc <= utc && d.end_utc.is_none_or(|e| utc < e)
        })
    }

    /// The device `source` used at `utc`: the record whose interval holds it,
    /// where an unknown start reaches back to the beginning. `None` when no
    /// record covers the instant (the source was removed then, or the instant
    /// falls before a record with a known start).
    #[must_use]
    pub fn device_at(&self, source: &str, utc: i64) -> Option<&SourceDevice> {
        self.devices.iter().find(|d| {
            d.source_id == source
                && d.from_utc.is_none_or(|f| f <= utc)
                && d.to_utc.is_none_or(|t| utc < t)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_sources::{AudioSourceStore, NewAudioSource, SourceKind};

    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        crate::migration::migrate(&c).unwrap();
        c.insert(&NewAudioSource::defaults(
            "mic",
            SourceKind::UsbAlsa,
            "plughw:1,0",
        ))
        .unwrap();
        c
    }

    fn new(start: i64) -> NewDeployment {
        NewDeployment {
            source_id: "mic".to_owned(),
            start_utc: start,
            stated: DeploymentFields {
                height_m: Some(2.0),
                ..DeploymentFields::default()
            },
        }
    }

    #[test]
    fn the_next_deployment_closes_the_open_one_where_it_starts() {
        let c = db();
        let first = open_deployment(&c, &new(1_000), 1_000).unwrap();
        assert_eq!(first.end_utc, None);
        assert_eq!(first.station_uuid, station_uuid(&c).unwrap());
        open_deployment(&c, &new(5_000), 5_000).unwrap();
        let all = list_deployments(&c).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].end_utc, Some(5_000));
        let p = SourceProvenance::load(&c).unwrap();
        assert_eq!(p.deployment_at("mic", 4_999).map(|d| d.id), Some(all[0].id));
        assert_eq!(p.deployment_at("mic", 5_000).map(|d| d.id), Some(all[1].id));
        assert_eq!(p.deployment_at("mic", 999), None, "before any was stated");
        assert_eq!(p.deployment_at("other", 5_000), None);
    }

    #[test]
    fn a_deployment_that_starts_before_the_open_one_is_refused() {
        let c = db();
        open_deployment(&c, &new(5_000), 5_000).unwrap();
        for start in [5_000, 4_000] {
            assert!(matches!(
                open_deployment(&c, &new(start), 6_000),
                Err(StationError::Invalid(_))
            ));
        }
        assert_eq!(
            list_deployments(&c).unwrap().len(),
            1,
            "nothing was written"
        );
    }

    #[test]
    fn stated_values_out_of_range_and_unknown_sources_are_refused() {
        let c = db();
        let mut d = new(1);
        d.stated.orientation_deg = Some(360.0);
        assert!(open_deployment(&c, &d, 1).is_err());
        let mut d = new(1);
        d.stated.latitude = Some(51.0);
        assert!(
            open_deployment(&c, &d, 1).is_err(),
            "latitude without longitude"
        );
        let mut d = new(1);
        d.stated.height_m = Some(f64::NAN);
        assert!(open_deployment(&c, &d, 1).is_err());
        let mut d = new(1);
        d.source_id = "nobody".to_owned();
        assert!(open_deployment(&c, &d, 1).is_err());
        // Counterpart: the edges that are real values.
        let mut d = new(1);
        d.stated.orientation_deg = Some(0.0);
        d.stated.latitude = Some(-90.0);
        d.stated.longitude = Some(180.0);
        assert!(open_deployment(&c, &d, 1).is_ok());
    }

    #[test]
    fn the_device_at_an_instant_follows_the_records() {
        let c = db();
        c.execute_batch(
            "DELETE FROM audio_source_devices;
             INSERT INTO audio_source_devices (source_id, device_seq, kind, device_id, from_utc, to_utc)
             VALUES ('mic', 1, 'usb-alsa', 'plughw:1,0', NULL, 100),
                    ('mic', 2, 'usb-alsa', 'plughw:2,0', 100, 200),
                    ('mic', 1, 'usb-alsa', 'plughw:1,0', 300, NULL);",
        )
        .unwrap();
        let p = SourceProvenance::load(&c).unwrap();
        let seq = |t| p.device_at("mic", t).map(|d| d.device_seq);
        assert_eq!(seq(-5), Some(1), "an unknown start reaches back");
        assert_eq!(seq(99), Some(1));
        assert_eq!(seq(100), Some(2));
        assert_eq!(seq(250), None, "removed between 200 and 300");
        assert_eq!(seq(10_000), Some(1));
    }
}
