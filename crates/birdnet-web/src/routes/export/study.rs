//! The study bundle: one ZIP that carries what a methods section needs
//! (RES-08, RES-11, RES-21, RES-32).
//!
//! `GET /api/v2/detections/export/study?from=YYYY-MM-DD&to=YYYY-MM-DD[&tolerance_secs=][&sites=]`
//!
//! | Member | What it is |
//! |--------|------------|
//! | `detections.csv` | every column of every detection row of the local dates, rejected rows included |
//! | `classifier_answers.csv` | each classifier's own answer on those rows (`detection_model_scores`) |
//! | `review_events.csv` | the review history of the range: every verdict, who, when |
//! | `runs.csv` | every analysis run, with its deciding configuration and the labels' taxonomy edition |
//! | `run_classifiers.csv` | every classifier of every run, with its files' SHA-256 |
//! | `source_effort.csv` | seconds analysed per local hour and source (the effort ledger) |
//! | `site_effort.csv` | each site's listening per local hour, as bounds, with sensor time apart |
//! | `events.csv`, `event_members.csv` | events of the UTC dates, under the stated policy version |
//! | `weather.csv` | the weather hours of the UTC dates, with where each came from when recorded |
//! | `README.txt` | what each member and column is, and what none of them claims |
//! | `manifest.json` | schema version, generator, parameters, policies, and each member's SHA-256 |
//!
//! **Deterministic.** The same database and parameters give the same bytes: no
//! member holds the time of the export, every read has a total order, a real
//! number is printed in the shortest form that reads back as the stored `f64`
//! (so a score of `0.70004` is not rounded across a `0.7` threshold), and the
//! archive is built by [`super::zip`] with fixed dates. `manifest.json` cannot
//! hold its own digest; the digest of the archive is the caller's to record.
//!
//! The existing exports are unchanged; this is a separate, versioned format.

use std::fmt::Write as _;

use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use birdnet_db::sqlite::queries::events::{
    self, DEFAULT_TOLERANCE_SECS, EVENT_POLICY_VERSION, MAX_TOLERANCE_SECS, SiteMap,
};
use birdnet_db::sqlite::queries::run_config::{RUN_CONFIG_SCHEMA, sha256_hex};
use birdnet_db::sqlite::queries::site_effort::{self, LISTENED_DAY};
use birdnet_db::sqlite::queries::study::{Cell, StudyTable, TableDump, dump};
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{MAX_EXPORT_ROWS, escape_csv, export_too_large};
use crate::state::AppState;

/// The bundle's format name, in its manifest.
pub const BUNDLE_SCHEMA: &str = "birdnet-behavior.study-bundle";

/// The bundle's format version. Bumped when a member or column changes
/// meaning or is removed; a column a migration adds to a table does not bump
/// it, since every table is exported whole and the manifest lists the columns.
pub const BUNDLE_SCHEMA_VERSION: u32 = 1;

/// Longest range of one bundle, in days.
pub const MAX_BUNDLE_DAYS: i64 = 366;

/// What the bundle is asked for.
#[derive(Debug, Clone, PartialEq)]
pub struct BundleParams {
    /// First local date, inclusive.
    pub from: String,
    /// Last local date, inclusive.
    pub to: String,
    /// The event tolerance, seconds.
    pub tolerance_secs: f64,
    /// The site map.
    pub sites: SiteMap,
}

/// Why a bundle was not built.
#[derive(Debug)]
pub enum BundleError {
    /// A table had more rows than [`MAX_EXPORT_ROWS`].
    TooLarge,
    /// The database.
    Db(birdnet_db::sqlite::DbError),
    /// The archive.
    Zip(super::zip::ZipError),
}

impl From<birdnet_db::sqlite::DbError> for BundleError {
    fn from(e: birdnet_db::sqlite::DbError) -> Self {
        Self::Db(e)
    }
}

/// What every bundle says it does not claim, in `manifest.json` and
/// `README.txt`.
const STATEMENTS: [&str; 7] = [
    "Confidence is the classifier's output score for a window. It is not a calibrated \
     probability that the species was present, and scores from different classifiers are on \
     different scales.",
    "A detection is one classifier answer above a threshold in one window of audio. It is not an \
     individual bird, and counts of detections or events are not counts of birds.",
    "No detection in an interval is not an absence. The effort members say how long each source \
     analysed audio; windows analysed that produced no detection are not recorded per window.",
    "Rows a reviewer rejected are included; review_verdict says which. Review in this station is \
     not a random sample unless review_events come from a validation sample, so verdicts do not \
     estimate precision on their own.",
    "Weather recorded before its provenance was stored has provider and product empty: not \
     recorded. A forecast model's hour is a modelled value, not an observation.",
    "taxonomy_edition is 'not recorded' where the labels file matches no cited edition; \
     scientific names from different editions are not equated here.",
    "An empty CSV field is NULL. A text value beginning with = + - @, a tab or a carriage return \
     is prefixed with an apostrophe, so a spreadsheet does not run it as a formula.",
];

fn cell_text(cell: &Cell) -> String {
    match cell {
        Cell::Null => String::new(),
        Cell::Int(v) => v.to_string(),
        // Rust's `Display` for `f64` is the shortest decimal that parses back
        // to the same value, and never uses an exponent.
        Cell::Real(v) => format!("{v}"),
        Cell::Text(t) => escape_csv(t),
        Cell::Blob(n) => format!("<blob {n} bytes>"),
    }
}

/// A CSV of `header` and `rows`, CRLF-free (`\n` line ends), every line ended.
fn csv_of(header: &[String], rows: impl IntoIterator<Item = Vec<String>>) -> Vec<u8> {
    let mut out = String::new();
    out.push_str(
        &header
            .iter()
            .map(|h| escape_csv(h))
            .collect::<Vec<_>>()
            .join(","),
    );
    out.push('\n');
    for row in rows {
        out.push_str(&row.join(","));
        out.push('\n');
    }
    out.into_bytes()
}

/// An extra column a table member carries: its name, and how a row gives it.
type ExtraColumn<'a> = (&'a str, &'a dyn Fn(&[Cell]) -> String);

fn dump_csv(d: &TableDump, extra: &[ExtraColumn<'_>]) -> Vec<u8> {
    let mut header = d.columns.clone();
    header.extend(extra.iter().map(|(name, _)| (*name).to_owned()));
    csv_of(
        &header,
        d.rows.iter().map(|r| {
            let mut line: Vec<String> = r.iter().map(cell_text).collect();
            line.extend(extra.iter().map(|(_, f)| escape_csv(&f(r))));
            line
        }),
    )
}

fn column(d: &TableDump, name: &str) -> Option<usize> {
    d.columns.iter().position(|c| c == name)
}

/// The taxonomy edition a row's `labels_sha256` follows.
fn edition_of(d: &TableDump) -> impl Fn(&[Cell]) -> String + '_ {
    let at = column(d, "labels_sha256");
    move |row: &[Cell]| {
        let sha = match at.map(|i| &row[i]) {
            Some(Cell::Text(s)) => s.as_str(),
            _ => "",
        };
        birdnet_integrations::model_catalog::taxonomy_edition(sha)
            .map_or_else(|| "not recorded".to_owned(), |t| t.edition.to_owned())
    }
}

/// Unix seconds of a local-date string read as a UTC date.
fn utc_day_start(date: &str) -> Option<i64> {
    crate::routes::pages::viz::kit::epoch_day(date).map(|d| d * 86_400)
}

/// The members of a bundle, in archive order, `manifest.json` last.
///
/// # Errors
///
/// A table past the row bound, or the database.
pub fn bundle_members(
    conn: &Connection,
    p: &BundleParams,
) -> Result<Vec<(String, Vec<u8>)>, BundleError> {
    let max = MAX_EXPORT_ROWS as usize;
    let read = |t: StudyTable| -> Result<TableDump, BundleError> {
        let d = dump(conn, t, &p.from, &p.to, max)?;
        if d.truncated {
            return Err(BundleError::TooLarge);
        }
        Ok(d)
    };
    let mut members: Vec<(String, Vec<u8>, usize, Vec<String>)> = Vec::new();
    let mut push = |name: &str, bytes: Vec<u8>, rows: usize, cols: Vec<String>| {
        members.push((name.to_owned(), bytes, rows, cols));
    };

    for (name, table) in [
        ("detections.csv", StudyTable::Detections),
        ("classifier_answers.csv", StudyTable::ModelScores),
        ("review_events.csv", StudyTable::ReviewEvents),
    ] {
        let d = read(table)?;
        push(name, dump_csv(&d, &[]), d.rows.len(), d.columns.clone());
    }
    for (name, table) in [
        ("runs.csv", StudyTable::Runs),
        ("run_classifiers.csv", StudyTable::RunModels),
    ] {
        let d = read(table)?;
        let edition = edition_of(&d);
        let mut cols = d.columns.clone();
        cols.push("taxonomy_edition".to_owned());
        push(
            name,
            dump_csv(&d, &[("taxonomy_edition", &edition)]),
            d.rows.len(),
            cols,
        );
    }
    let effort = read(StudyTable::Effort)?;
    push(
        "source_effort.csv",
        dump_csv(&effort, &[]),
        effort.rows.len(),
        effort.columns.clone(),
    );

    let hours = site_effort::site_effort_hours(conn, &p.from, &p.to, &p.sites)?;
    let site_header: Vec<String> = [
        "date",
        "hour",
        "site",
        "sources",
        "lower_bound_secs",
        "upper_bound_secs",
        "sensor_secs",
        "capacity_secs",
    ]
    .map(str::to_owned)
    .to_vec();
    let site_rows = hours.len();
    push(
        "site_effort.csv",
        csv_of(
            &site_header,
            hours.iter().map(|h| {
                vec![
                    escape_csv(&h.date),
                    h.hour.to_string(),
                    escape_csv(&h.site),
                    escape_csv(&h.sources.iter().cloned().collect::<Vec<_>>().join(";")),
                    format!("{}", h.lower_bound_secs),
                    format!("{}", h.upper_bound_secs),
                    format!("{}", h.sensor_secs),
                    h.capacity_secs.map_or_default(|c| format!("{c}")),
                ]
            }),
        ),
        site_rows,
        site_header,
    );

    let (ev, event_members) = event_members(conn, p)?;
    for (name, bytes, rows, cols) in event_members {
        push(name, bytes, rows, cols);
    }

    let weather = read(StudyTable::Weather)?;
    push(
        "weather.csv",
        dump_csv(&weather, &[]),
        weather.rows.len(),
        weather.columns.clone(),
    );
    push("README.txt", readme().into_bytes(), 0, Vec::new());

    let listed: Vec<Value> = members
        .iter()
        .map(|(name, bytes, rows, cols)| {
            json!({
                "path": name,
                "bytes": bytes.len(),
                "sha256": sha256_hex(bytes),
                "rows": rows,
                "columns": cols,
            })
        })
        .collect();
    let explicit: serde_json::Map<String, Value> = p
        .sites
        .explicit()
        .iter()
        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
        .collect();
    let manifest = json!({
        "schema": BUNDLE_SCHEMA,
        "schema_version": BUNDLE_SCHEMA_VERSION,
        "generator": {
            "name": "birdnet-behavior",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "parameters": {
            "from": p.from,
            "to": p.to,
            "dates": "detections, answers, reviews and effort by the station's local date; \
                      events and weather by UTC date",
            "tolerance_secs": p.tolerance_secs,
            "sites": Value::Object(explicit),
            "sites_note": "a source not named is its own site",
        },
        "policies": {
            "event_policy_version": EVENT_POLICY_VERSION,
            "events_edges_settled": ev.edges_settled,
            "events_rows_without_instant": ev.rows_without_instant,
            "listened_day_rule": format!("{LISTENED_DAY:?}"),
            "run_config_schema": RUN_CONFIG_SCHEMA,
        },
        "licence": Value::Null,
        "licence_note": "No licence is applied by the software. The licence of exported \
                         observations is the station operator's or study lead's to choose \
                         and state; the software's own licence does not extend to them.",
        "statements": STATEMENTS,
        "members": listed,
        "manifest_note": "manifest.json is not in its own member list; record the SHA-256 of \
                          the archive to cite the whole bundle.",
    });
    let mut manifest_text = serde_json::to_string_pretty(&manifest).unwrap_or_default();
    manifest_text.push('\n');

    let mut out: Vec<(String, Vec<u8>)> = members.into_iter().map(|(n, b, _, _)| (n, b)).collect();
    out.push(("manifest.json".to_owned(), manifest_text.into_bytes()));
    Ok(out)
}

/// A member before it is listed: name, bytes, row count, columns.
type Member = (&'static str, Vec<u8>, usize, Vec<String>);

/// `events.csv` and `event_members.csv`, and the read they came from.
fn event_members(
    conn: &Connection,
    p: &BundleParams,
) -> Result<(events::EventsRead, [Member; 2]), BundleError> {
    let (from_utc, to_utc) = (
        utc_day_start(&p.from).unwrap_or(0),
        utc_day_start(&p.to).unwrap_or(0) + 86_400,
    );
    let ev = events::events_between(conn, from_utc, to_utc, p.tolerance_secs, &p.sites)?;
    let ev_header: Vec<String> = [
        "event_id",
        "policy_version",
        "tolerance_secs",
        "site",
        "sci_name",
        "com_name",
        "first_utc",
        "last_utc",
        "members",
    ]
    .map(str::to_owned)
    .to_vec();
    let mem_header: Vec<String> = ["event_id", "row_id", "source", "instant_utc", "confidence"]
        .map(str::to_owned)
        .to_vec();
    let mut mem_rows = Vec::new();
    for e in &ev.events {
        for m in &e.members {
            mem_rows.push(vec![
                escape_csv(&e.event_id),
                escape_csv(&m.row_id),
                escape_csv(m.source.as_deref().unwrap_or("")),
                m.instant.to_string(),
                format!("{}", m.confidence),
            ]);
        }
    }
    let ev_count = ev.events.len();
    let mem_count = mem_rows.len();
    let events_member = (
        "events.csv",
        csv_of(
            &ev_header,
            ev.events.iter().map(|e| {
                vec![
                    escape_csv(&e.event_id),
                    EVENT_POLICY_VERSION.to_owned(),
                    format!("{}", p.tolerance_secs),
                    escape_csv(e.site.as_deref().unwrap_or("")),
                    escape_csv(&e.sci_name),
                    escape_csv(&e.com_name),
                    e.first.to_string(),
                    e.last.to_string(),
                    e.members.len().to_string(),
                ]
            }),
        ),
        ev_count,
        ev_header,
    );
    let members_member = (
        "event_members.csv",
        csv_of(&mem_header, mem_rows),
        mem_count,
        mem_header,
    );
    Ok((ev, [events_member, members_member]))
}

fn readme() -> String {
    let mut s = String::from(
        "BirdNet-Behavior study bundle\n\
         =============================\n\n\
         manifest.json gives the format version, the parameters, the policy versions and the \
         SHA-256 of every other member. Every table member holds every column of its table, \
         named as the database names it; the manifest lists them.\n\n\
         detections.csv         every detection row of the local dates, rejected ones included.\n\
         \x20                      run_id joins runs.csv; (run_id, model_id) joins run_classifiers.csv.\n\
         classifier_answers.csv each classifier's answer on a detection's window: state heard,\n\
         \x20                      below_bar, no_label or failed; score and bar where it had one.\n\
         review_events.csv      every verdict on those rows, append-only, with who and when.\n\
         runs.csv               every analysis run: model files and SHA-256, the five run columns,\n\
         \x20                      config_json (the deciding configuration; empty = not recorded,\n\
         \x20                      for a run registered before it was) and taxonomy_edition.\n\
         run_classifiers.csv    every classifier of every run and the SHA-256 of its files.\n\
         source_effort.csv      seconds of audio analysed per local hour and source.\n\
         site_effort.csv        per site and local hour: the site listened at least\n\
         \x20                      lower_bound_secs and at most upper_bound_secs; sensor_secs is\n\
         \x20                      the sum over sources, a per-sensor denominator, not the site's.\n\
         events.csv             detections of one species at one site, at most one per source,\n\
         \x20                      within tolerance_secs of one another (policy_version).\n\
         event_members.csv      the detections of each event.\n\
         weather.csv            hourly weather; provider and product say where it came from\n\
         \x20                      where recorded.\n\n\
         What this bundle does not claim\n\
         -------------------------------\n",
    );
    for st in STATEMENTS {
        let _ = writeln!(s, "- {st}");
    }
    s
}

#[derive(Debug, Deserialize)]
pub(super) struct StudyQuery {
    from: Option<String>,
    to: Option<String>,
    tolerance_secs: Option<f64>,
    sites: Option<String>,
}

fn bad_request(message: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        [(header::CONTENT_TYPE, "application/json")],
        json!({ "error": message }).to_string(),
    )
        .into_response()
}

/// Check a request's parameters.
pub fn parse(q: &StudyQuery) -> Result<BundleParams, String> {
    let (Some(from), Some(to)) = (q.from.as_deref(), q.to.as_deref()) else {
        return Err("from and to are required, as YYYY-MM-DD".to_owned());
    };
    if !crate::routes::is_valid_date(from) || !crate::routes::is_valid_date(to) {
        return Err("from and to are required, as YYYY-MM-DD".to_owned());
    }
    let day = crate::routes::pages::viz::kit::epoch_day;
    match (day(from), day(to)) {
        (Some(f), Some(t)) if (0..MAX_BUNDLE_DAYS).contains(&(t - f)) => {}
        _ => {
            return Err(format!(
                "from must not be after to, and a bundle is at most {MAX_BUNDLE_DAYS} days"
            ));
        }
    }
    let tolerance_secs = q.tolerance_secs.unwrap_or(DEFAULT_TOLERANCE_SECS);
    if !(tolerance_secs.is_finite() && (0.0..=MAX_TOLERANCE_SECS).contains(&tolerance_secs)) {
        return Err(format!(
            "tolerance_secs is a number of seconds from 0 to {MAX_TOLERANCE_SECS}"
        ));
    }
    let sites = crate::routes::events::parse_sites(q.sites.as_deref())?;
    Ok(BundleParams {
        from: from.to_owned(),
        to: to.to_owned(),
        tolerance_secs,
        sites,
    })
}

/// `GET /api/v2/detections/export/study`.
pub(super) async fn export_study(
    State(state): State<AppState>,
    Query(query): Query<StudyQuery>,
) -> Response {
    let params = match parse(&query) {
        Ok(p) => p,
        Err(m) => return bad_request(&m),
    };
    let name = format!("study-{}-{}.zip", params.from, params.to);
    let built = state
        .run_blocking_heavy(move |s| {
            s.with_read_db(|conn| bundle_members(conn, &params))
                .and_then(|m| super::zip::build(&m).map_err(BundleError::Zip))
        })
        .await;
    match built {
        Ok(bytes) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "application/zip".to_owned()),
                (
                    header::CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{name}\""),
                ),
            ],
            bytes,
        )
            .into_response(),
        Err(BundleError::TooLarge) => export_too_large(),
        Err(BundleError::Db(e)) => internal(&e),
        Err(BundleError::Zip(e)) => internal(&e),
    }
}

fn internal<E: std::fmt::Display>(e: &E) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(header::CONTENT_TYPE, "application/json")],
        json!({ "error": crate::routes::log_internal("study bundle", e) }).to_string(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn station() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let conn = birdnet_db::sqlite::open_or_create(&dir.path().join("birds.db")).unwrap();
        conn.execute_batch(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name,
                                     Source, detected_at_utc, review_verdict)
             VALUES ('2026-05-01', '06:00:00', 'Pica pica', 'Magpie', 0.70004, 'a.wav',
                     'mic', 1777615200, NULL),
                    ('2026-05-01', '06:00:03', 'Pica pica', 'Magpie', 0.69996, 'a.wav',
                     'mic', 1777615203, 'rejected'),
                    ('2026-05-02', '07:00:00', 'Turdus merula', 'Blackbird', 0.9, 'b.wav',
                     'mic', 1777705200, NULL);
             INSERT INTO recording_effort_hourly (date, hour, source, seconds_analysed)
             VALUES ('2026-05-01', 6, 'mic', 3600.0);",
        )
        .unwrap();
        (dir, conn)
    }

    fn params() -> BundleParams {
        BundleParams {
            from: "2026-05-01".into(),
            to: "2026-05-02".into(),
            tolerance_secs: DEFAULT_TOLERANCE_SECS,
            sites: SiteMap::identity(),
        }
    }

    fn member<'a>(m: &'a [(String, Vec<u8>)], name: &str) -> &'a str {
        let (_, bytes) = m.iter().find(|(n, _)| n == name).expect(name);
        std::str::from_utf8(bytes).unwrap()
    }

    fn archive(conn: &Connection) -> Vec<u8> {
        super::super::zip::build(&bundle_members(conn, &params()).unwrap()).unwrap()
    }

    /// RES-08: the same database and parameters give byte-identical archives.
    #[test]
    fn the_same_station_and_parameters_give_the_same_bytes() {
        let (_d, conn) = station();
        let a = archive(&conn);
        assert_eq!(a, archive(&conn));
        // Counterpart: a different database gives a different archive.
        conn.execute(
            "UPDATE detections SET Confidence = 0.8 WHERE File_Name = 'b.wav'",
            [],
        )
        .unwrap();
        assert_ne!(a, archive(&conn));
        let names: Vec<String> = super::super::zip::read(&a)
            .into_iter()
            .map(|m| m.0)
            .collect();
        assert_eq!(names.last().map(String::as_str), Some("manifest.json"));
        assert!(names.iter().any(|n| n == "site_effort.csv"), "{names:?}");
    }

    /// A score either side of a 0.7 threshold survives the export exactly, and
    /// a rejected row is in it, marked.
    #[test]
    fn a_score_is_exported_exactly_and_a_rejected_row_is_kept() {
        let (_d, conn) = station();
        let m = bundle_members(&conn, &params()).unwrap();
        let det = member(&m, "detections.csv");
        assert!(det.contains(",0.70004,"), "{det}");
        assert!(det.contains(",0.69996,"), "{det}");
        assert!(det.contains("rejected"), "{det}");
        assert_eq!(det.lines().count(), 4, "header and three rows: {det}");
    }

    /// The manifest names every other member with its SHA-256, the format
    /// version, the policy and the parameters.
    #[test]
    fn the_manifest_hashes_every_member_and_states_its_policies() {
        let (_d, conn) = station();
        let m = bundle_members(&conn, &params()).unwrap();
        let manifest: Value = serde_json::from_str(member(&m, "manifest.json")).unwrap();
        assert_eq!(manifest["schema_version"], BUNDLE_SCHEMA_VERSION);
        assert_eq!(
            manifest["policies"]["event_policy_version"],
            EVENT_POLICY_VERSION
        );
        assert_eq!(manifest["parameters"]["from"], "2026-05-01");
        assert!(manifest["licence"].is_null());
        let listed = manifest["members"].as_array().unwrap();
        assert_eq!(listed.len(), m.len() - 1);
        for entry in listed {
            let name = entry["path"].as_str().unwrap();
            let (_, bytes) = m.iter().find(|(n, _)| n == name).unwrap();
            assert_eq!(entry["sha256"], sha256_hex(bytes), "{name}");
        }
        let site = member(&m, "site_effort.csv");
        assert!(
            site.contains("2026-05-01,6,mic,mic,3600,3600,3600"),
            "{site}"
        );
        let events = member(&m, "events.csv");
        assert!(events.contains(EVENT_POLICY_VERSION), "{events}");
        let runs = member(&m, "runs.csv");
        assert!(
            runs.starts_with("id,") && runs.contains("config_json"),
            "{runs}"
        );
    }
}
