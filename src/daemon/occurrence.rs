//! The station's occurrence ("range") filter, as the web crate may read it.
//!
//! The detection daemon owns its [`SpeciesFilter`] on its own thread, behind an
//! ONNX session that needs `&mut` to run, and the web crate must not reach into
//! that thread or start an inference runtime of its own. What `birdnet-web`
//! gets instead is an [`OccurrenceSource`] (installed on the app state at start,
//! as the capture status is) that answers "what does the filter say about this
//! week?".
//!
//! # A second session, built from the same inputs
//!
//! [`DaemonOccurrence`] builds a **private** `SpeciesFilter` from exactly what
//! the daemon builds its own from: the same range-model file, the same
//! label files, the same alias file, the same `SF_THRESH`, and — read afresh on
//! each answer, as the daemon's loop does on a short TTL — the same include,
//! exclude and "always allowed" lists. `SpeciesFilter::occurrence_scores` shares
//! one model run and one decision function with the `filter_species` the daemon
//! calls (`the_scores_an_operator_is_shown_are_the_decision_the_station_makes`
//! holds that on the real models), so the answer is the station's own decision,
//! not a second opinion.
//!
//! The alternative — sharing the daemon's filter behind a lock — would put a
//! mutex into the per-file hot path of the detection loop for the sake of a
//! page, and would make a request for that page wait on, and be waited on by,
//! an analysis. A second session holds a second copy of the model: the files are
//! 14.7 MB (V3 geomodel) and 29.5 MB (V2.4 range model) on disk, and the memory
//! a session takes was not measured. It is only built on the first request, so
//! a station nobody asks costs nothing.
//!
//! # What is remembered
//!
//! * The load is tried once. The daemon tries once, at start; a model that did
//!   not load for it is a filter that is off for the life of the process, and
//!   this reports the same reason rather than quietly succeeding on a retry.
//! * The answer for a week is kept for [`KEEP_FOR`]: the species page asks for
//!   the current week on every view, and the model is a few milliseconds that
//!   need not be spent again.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use birdnet_core::inference::labels::LabelSet;
use birdnet_core::inference::species_filter::{
    SpeciesFilter, SpeciesFilterConfig, SpeciesListsProvider,
};
use birdnet_web::occurrence::{OccurrenceReport, OccurrenceSource, Unavailable};

/// How long an answer for a week is reused.
///
/// Short enough that a change to the lists shows within a minute, long enough
/// to absorb a page that asks once per species view. (A change made now is not
/// on the page until the kept answer has expired; the page does not claim
/// otherwise.)
pub(super) const KEEP_FOR: Duration = Duration::from_secs(60);

/// Whether an answer `age` old is still reused: only while it is younger than
/// `keep_for`. A function of its own because the boundary can be tested exactly
/// here, and cannot against a real clock.
fn is_fresh(age: Duration, keep_for: Duration) -> bool {
    age < keep_for
}

/// The files the range model is built from.
#[derive(Debug, Clone)]
pub(super) struct RangeModelFiles {
    /// The range ("metadata") ONNX model.
    pub model: PathBuf,
    /// The model's own label file, when it has one (the V3 geomodel does; the
    /// V2.4 range model is indexed like the classifier's labels).
    pub labels: Option<PathBuf>,
    /// The operator's alias file.
    pub aliases: Option<PathBuf>,
}

/// What loading the filter came to.
enum Load {
    NotTried,
    Ready {
        filter: Box<SpeciesFilter>,
        classifier: Arc<LabelSet>,
    },
    Failed(String),
}

/// The occurrence filter, for the web crate.
pub(super) struct DaemonOccurrence {
    /// The station's coordinates as the daemon resolved them at start.
    location: Option<(f64, f64)>,
    /// The range model, if one is configured.
    model: Option<RangeModelFiles>,
    /// The classifier's label file: the vocabulary the model is aligned to.
    classifier_labels: PathBuf,
    /// The daemon's own filter configuration (threshold and the lists as of
    /// start).
    config: SpeciesFilterConfig,
    /// The lists as they are now.
    lists: SpeciesListsProvider,
    /// How long a kept answer is reused ([`KEEP_FOR`]; a test shortens it).
    keep_for: Duration,
    load: Mutex<Load>,
    kept: Mutex<HashMap<u32, (Instant, Arc<OccurrenceReport>)>>,
}

impl std::fmt::Debug for DaemonOccurrence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DaemonOccurrence")
            .field("has_location", &self.location.is_some())
            .field("has_model", &self.model.is_some())
            .finish_non_exhaustive()
    }
}

impl DaemonOccurrence {
    /// A source for a station with this location, range model and filter
    /// configuration. Nothing is loaded until the first answer is asked for.
    pub(super) fn new(
        location: Option<(f64, f64)>,
        model: Option<RangeModelFiles>,
        classifier_labels: PathBuf,
        config: SpeciesFilterConfig,
        lists: SpeciesListsProvider,
    ) -> Self {
        Self {
            location,
            model,
            classifier_labels,
            config,
            lists,
            keep_for: KEEP_FOR,
            load: Mutex::new(Load::NotTried),
            kept: Mutex::new(HashMap::new()),
        }
    }

    /// The same source, reusing an answer for `keep_for` instead of
    /// [`KEEP_FOR`]: a test cannot wait a minute for it to expire.
    #[cfg(test)]
    const fn keeping_for(mut self, keep_for: Duration) -> Self {
        self.keep_for = keep_for;
        self
    }

    /// Load the filter, once, from the files the daemon loaded.
    fn load(&self, files: &RangeModelFiles) -> Load {
        let classifier = match LabelSet::load(&self.classifier_labels) {
            Ok(labels) => Arc::new(labels),
            Err(e) => return Load::Failed(format!("classifier labels: {e}")),
        };
        let meta = match files.labels.as_deref().map(LabelSet::load).transpose() {
            Ok(meta) => meta,
            Err(e) => return Load::Failed(format!("range model labels: {e}")),
        };
        let aliases = files
            .aliases
            .as_deref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map_or_else(HashMap::new, |text| {
                birdnet_core::inference::vocabulary::parse_alias_file(&text).0
            });
        match SpeciesFilter::load_with_vocabulary(
            &files.model,
            meta,
            &classifier,
            &aliases,
            self.config.clone(),
        ) {
            Ok(filter) => Load::Ready {
                filter: Box::new(filter),
                classifier,
            },
            Err(e) => Load::Failed(e.to_string()),
        }
    }
}

impl OccurrenceSource for DaemonOccurrence {
    fn report(&self, week: u32) -> Result<Arc<OccurrenceReport>, Unavailable> {
        // The filter being off is the more basic fact: a station with no model
        // has no use for a location.
        let Some(files) = self.model.as_ref() else {
            return Err(Unavailable::NoModel);
        };
        let Some(location) = self.location else {
            return Err(Unavailable::NoLocation);
        };

        if let Some((at, report)) = self
            .kept
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&week)
            && is_fresh(at.elapsed(), self.keep_for)
        {
            return Ok(Arc::clone(report));
        }

        let mut load = self.load.lock().unwrap_or_else(PoisonError::into_inner);
        if matches!(*load, Load::NotTried) {
            *load = self.load(files);
        }
        let (filter, classifier) = match &mut *load {
            Load::Ready { filter, classifier } => (filter, Arc::clone(classifier)),
            Load::Failed(why) => return Err(Unavailable::Failed(why.clone())),
            Load::NotTried => return Err(Unavailable::NoModel),
        };
        // The lists as they are now, as the daemon's loop re-reads them.
        filter.set_lists(self.lists.get());
        let scores = filter
            .occurrence_scores(location, week, &classifier)
            .map_err(|e| Unavailable::Failed(e.to_string()))?
            .ok_or(Unavailable::NoModel)?;
        drop(load);

        let report = Arc::new(OccurrenceReport {
            latitude: location.0,
            longitude: location.1,
            week,
            threshold: self.config.sf_thresh,
            species: scores,
        });
        self.kept
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(week, (Instant::now(), Arc::clone(&report)));
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use birdnet_core::inference::species_filter::SpeciesLists;

    fn source(location: Option<(f64, f64)>, model: Option<&str>) -> DaemonOccurrence {
        DaemonOccurrence::new(
            location,
            model.map(|m| RangeModelFiles {
                model: PathBuf::from(m),
                labels: None,
                aliases: None,
            }),
            PathBuf::from("/nonexistent/labels.txt"),
            SpeciesFilterConfig::default(),
            SpeciesListsProvider::new(SpeciesLists::default),
        )
    }

    // ── the real V2.4 range filter ─────────────────────────────────────

    /// A station in Berlin, and one in central Kansas: each is home to the
    /// birds the other is not.
    const BERLIN: (f64, f64) = (52.52, 13.405);
    const KANSAS: (f64, f64) = (38.5, -98.0);
    /// Week 22 of the model's 48-week year: early June.
    const JUNE: u32 = 22;
    const MAGPIE: &str = "Pica pica";
    const DICKCISSEL: &str = "Spiza americana";

    /// The V2.4 range model and labels (`BIRDNET_TEST_V24_MDATA`,
    /// `BIRDNET_TEST_V24_LABELS`), or `None` to skip. With
    /// `BIRDNET_REQUIRE_V24_MODEL=1` a missing file is a failure instead: CI
    /// promised them, and a skip would report `ok` for a check never made.
    fn v24_files() -> Option<(PathBuf, PathBuf)> {
        let required =
            std::env::var("BIRDNET_REQUIRE_V24_MODEL").is_ok_and(|v| !v.is_empty() && v != "0");
        let file = |var: &str| {
            std::env::var_os(var)
                .map(PathBuf::from)
                .filter(|p| p.is_file())
        };
        if let (Some(mdata), Some(labels)) = (
            file("BIRDNET_TEST_V24_MDATA"),
            file("BIRDNET_TEST_V24_LABELS"),
        ) {
            return Some((mdata, labels));
        }
        assert!(
            !required,
            "BIRDNET_REQUIRE_V24_MODEL is set, but BIRDNET_TEST_V24_MDATA / \
             BIRDNET_TEST_V24_LABELS do not both name files: the occurrence explainer \
             was NOT checked against the real range filter"
        );
        eprintln!("SKIP: set BIRDNET_TEST_V24_MDATA and BIRDNET_TEST_V24_LABELS");
        None
    }

    /// The explainer the binary installs, on the real V2.4 files.
    ///
    /// `lists` reach it the way a change made after the daemon started does:
    /// through the provider, which is read afresh, and **not** through the
    /// configuration it was built with (empty here, as it would be had the
    /// operator not yet added them at start). A source that read only its
    /// start-up configuration would never show them.
    fn real(location: (f64, f64), lists: SpeciesLists) -> Option<DaemonOccurrence> {
        real_reading(location, SpeciesListsProvider::new(move || lists.clone()))
    }

    /// The same, with lists the test can change after the source is built.
    fn real_reading(location: (f64, f64), lists: SpeciesListsProvider) -> Option<DaemonOccurrence> {
        let (mdata, labels) = v24_files()?;
        Some(DaemonOccurrence::new(
            Some(location),
            Some(RangeModelFiles {
                model: mdata,
                labels: None,
                aliases: None,
            }),
            labels,
            SpeciesFilterConfig {
                sf_thresh: 0.03,
                ..SpeciesFilterConfig::default()
            },
            lists,
        ))
    }

    /// What the *daemon's* filter admits for the same place, week and lists:
    /// `filter_species`, the call the detection loop makes.
    fn what_the_daemon_admits(
        location: (f64, f64),
        lists: SpeciesLists,
    ) -> std::collections::HashSet<String> {
        let (mdata, labels) = v24_files().expect("checked by the caller");
        let classifier = LabelSet::load(&labels).expect("labels");
        let mut filter = SpeciesFilter::load_with_vocabulary(
            &mdata,
            None,
            &classifier,
            &HashMap::new(),
            SpeciesFilterConfig {
                sf_thresh: 0.03,
                include_list: lists.include,
                exclude_list: lists.exclude,
                whitelist: lists.whitelist.into_iter().collect(),
            },
        )
        .expect("the V2.4 range filter loads");
        filter
            .filter_species(Some(location), JUNE, &classifier)
            .expect("the filter runs")
    }

    async fn ask(app: &axum::Router, uri: &str) -> (axum::http::StatusCode, String) {
        use tower::ServiceExt as _;
        let res = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri(uri)
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("router responds");
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 24)
            .await
            .expect("body");
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    fn app_for(source: DaemonOccurrence) -> (tempfile::TempDir, axum::Router) {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = birdnet_web::state::AppState::new(dir.path().join("birds.db")).expect("state");
        state.set_occurrence_source(Arc::new(source));
        (dir, birdnet_web::server::build_router(state))
    }

    /// **The question the page exists for**, asked of the real range model
    /// through the real handlers: a European station keeps the magpie and holds
    /// back a North American bird, and is told so in a sentence carrying the
    /// model's own number and the threshold in force.
    #[tokio::test]
    async fn a_european_station_keeps_the_magpie_and_is_told_why_a_north_american_bird_is_held_back()
     {
        let Some(source) = real(BERLIN, SpeciesLists::default()) else {
            return;
        };
        let report = source.report(JUNE).expect("the real model answers");
        assert_eq!(
            (report.latitude, report.longitude),
            BERLIN,
            "the report names the place that was asked about, latitude first"
        );
        // A filter that passed everything or nothing would make the checks
        // below say nothing about occurrence.
        let admitted = report.species.iter().filter(|s| s.admitted).count();
        assert!(
            admitted > 0 && admitted < report.species.len() && report.species.len() > 5000,
            "{admitted} of {} admitted: not filtering",
            report.species.len()
        );

        let (_dir, app) = app_for(source);
        let (status, body) = ask(&app, "/api/v2/species/occurrence?week=22&q=pica+pica").await;
        assert_eq!(status, axum::http::StatusCode::OK, "{body}");
        let json: serde_json::Value = serde_json::from_str(&body).expect("JSON");
        assert_eq!(json["species"][0]["scientific_name"], MAGPIE);
        assert_eq!(json["species"][0]["verdict"], "admitted", "{body}");

        let (_, body) = ask(&app, "/api/v2/species/occurrence?week=22&q=spiza+americana").await;
        let json: serde_json::Value = serde_json::from_str(&body).expect("JSON");
        let row = &json["species"][0];
        assert_eq!(row["scientific_name"], DICKCISSEL, "{body}");
        assert_eq!(row["admitted"], false, "{body}");
        assert_eq!(row["verdict"], "held-back-by-range", "{body}");
        let p = row["probability"].as_f64().expect("probability");
        assert!(p < 0.03, "the Dickcissel scores {p} in Berlin in June");
        // The sentence carries the model's number, formatted as the page formats
        // it, and the real threshold.
        let said = birdnet_web::occurrence::percent_text(report_probability(&report, DICKCISSEL));
        let explanation = row["explanation"].as_str().expect("explanation");
        assert!(
            explanation.starts_with("Held back: the range model gives it ")
                && explanation.contains(&said)
                && explanation.contains("admits species at 3 % and above"),
            "{explanation}"
        );
        // And the page says the same.
        let (_, page) = ask(&app, "/species/occurrence?week=22&q=spiza+americana").await;
        // The page escapes it: a probability under 0.01 % reads `<0.01 %`.
        let escaped = explanation.replace('<', "&lt;");
        assert!(page.contains(&escaped), "{escaped}\n{page}");
    }

    fn report_probability(report: &OccurrenceReport, sci: &str) -> f32 {
        report
            .species
            .iter()
            .find(|s| s.scientific_name == sci)
            .unwrap_or_else(|| panic!("{sci} is not scored"))
            .probability
    }

    /// The counterpart: a station in Kansas is the mirror image. A page that
    /// held the Dickcissel back everywhere, or the magpie in, would pass the
    /// test above.
    #[test]
    fn a_north_american_station_keeps_the_north_american_bird_and_holds_back_the_magpie() {
        let Some(source) = real(KANSAS, SpeciesLists::default()) else {
            return;
        };
        let report = source.report(JUNE).expect("the real model answers");
        assert_eq!(
            (report.latitude, report.longitude),
            KANSAS,
            "west of Greenwich is a negative longitude"
        );
        let find = |sci: &str| {
            report
                .species
                .iter()
                .find(|s| s.scientific_name == sci)
                .unwrap_or_else(|| panic!("{sci} is not scored"))
        };
        assert!(find(DICKCISSEL).admitted, "{:?}", find(DICKCISSEL));
        assert!(!find(MAGPIE).admitted, "{:?}", find(MAGPIE));
    }

    /// What the page admits is what the daemon admits - the same place, week
    /// and lists - not a second opinion: the set of species the explainer marks
    /// admitted equals `filter_species`'s, for no lists, for an exclude list and
    /// for an always-allowed list, which are the three ways a verdict is not the
    /// threshold's.
    #[test]
    fn what_the_page_admits_is_what_the_daemon_admits_with_the_operators_lists_too() {
        if v24_files().is_none() {
            return;
        }
        let cases = [
            ("no lists", SpeciesLists::default()),
            (
                "the magpie excluded",
                SpeciesLists {
                    exclude: vec![MAGPIE.to_owned()],
                    ..SpeciesLists::default()
                },
            ),
            (
                "the Dickcissel always allowed",
                SpeciesLists {
                    whitelist: vec![DICKCISSEL.to_owned()],
                    ..SpeciesLists::default()
                },
            ),
        ];
        for (what, lists) in cases {
            let source = real(BERLIN, lists.clone()).expect("files checked");
            let report = source.report(JUNE).expect("the real model answers");
            let shown: std::collections::HashSet<String> = report
                .species
                .iter()
                .filter(|s| s.admitted)
                .map(|s| s.scientific_name.clone())
                .collect();
            assert_eq!(
                shown,
                what_the_daemon_admits(BERLIN, lists),
                "{what}: the page and the daemon disagree about what Berlin admits in June"
            );
            let verdict = |sci: &str| {
                let s = report
                    .species
                    .iter()
                    .find(|s| s.scientific_name == sci)
                    .expect("scored");
                birdnet_web::occurrence::Verdict::of(s, report.threshold)
            };
            match what {
                "the magpie excluded" => assert_eq!(
                    verdict(MAGPIE),
                    birdnet_web::occurrence::Verdict::HeldBackByLists,
                    "an excluded bird is blamed on the lists, not the range model"
                ),
                "the Dickcissel always allowed" => assert_eq!(
                    verdict(DICKCISSEL),
                    birdnet_web::occurrence::Verdict::AdmittedByWhitelist
                ),
                _ => assert_eq!(verdict(MAGPIE), birdnet_web::occurrence::Verdict::Admitted),
            }
        }
    }

    /// An answer for a week is kept for a short while; another week is another
    /// answer.
    #[test]
    fn an_answer_is_reused_within_its_minute_and_a_different_week_is_asked_afresh() {
        let Some(source) = real(BERLIN, SpeciesLists::default()) else {
            return;
        };
        let first = source.report(JUNE).expect("answers");
        let again = source.report(JUNE).expect("answers");
        assert!(
            Arc::ptr_eq(&first, &again),
            "the model ran twice for one week"
        );
        let winter = source.report(2).expect("answers");
        assert!(!Arc::ptr_eq(&first, &winter));
        assert_eq!(winter.week, 2);
        assert_ne!(
            first
                .species
                .iter()
                .filter(|s| s.admitted)
                .map(|s| s.scientific_name.as_str())
                .collect::<std::collections::BTreeSet<_>>(),
            winter
                .species
                .iter()
                .filter(|s| s.admitted)
                .map(|s| s.scientific_name.as_str())
                .collect::<std::collections::BTreeSet<_>>(),
            "June and January admit different birds in Berlin"
        );
    }

    /// A kept answer expires, and a change to the lists is on the page once it
    /// has. Within its time the old answer stands (the counterpart: a source
    /// that re-ran the model on every look would pass the expiry half alone).
    #[test]
    fn a_change_to_the_lists_shows_once_the_kept_answer_has_expired() {
        let lists = Arc::new(Mutex::new(SpeciesLists::default()));
        let provider = {
            let lists = Arc::clone(&lists);
            SpeciesListsProvider::new(move || {
                lists.lock().unwrap_or_else(PoisonError::into_inner).clone()
            })
        };
        let Some(source) = real_reading(BERLIN, provider.clone()) else {
            return;
        };
        let verdict = |report: &OccurrenceReport| {
            let s = report
                .species
                .iter()
                .find(|s| s.scientific_name == MAGPIE)
                .expect("scored");
            birdnet_web::occurrence::Verdict::of(s, report.threshold)
        };
        let before = source.report(JUNE).expect("answers");
        assert_eq!(verdict(&before), birdnet_web::occurrence::Verdict::Admitted);

        lists
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .exclude
            .push(MAGPIE.to_owned());
        let within = source.report(JUNE).expect("answers");
        assert!(
            Arc::ptr_eq(&before, &within),
            "inside its time the kept answer stands, so the model is not run per look"
        );

        let expired = real_reading(BERLIN, provider)
            .expect("files checked")
            .keeping_for(Duration::ZERO);
        let first = expired.report(JUNE).expect("answers");
        let second = expired.report(JUNE).expect("answers");
        assert!(
            !Arc::ptr_eq(&first, &second),
            "an answer past its time is asked for again"
        );
        assert_eq!(
            verdict(&second),
            birdnet_web::occurrence::Verdict::HeldBackByLists,
            "the exclusion made since is on the page once the answer is fresh"
        );
    }

    /// The daemon tries to load its range filter once, at start; a filter that
    /// did not load is off for the life of the process. This reports the same
    /// reason on every ask rather than succeeding on a retry the daemon never
    /// makes - otherwise the page would explain a filter the station is not
    /// running.
    #[test]
    fn a_filter_that_failed_to_load_is_not_loaded_again_by_a_later_ask() {
        let Some((mdata, labels)) = v24_files() else {
            return;
        };
        let dir = tempfile::tempdir().expect("tempdir");
        // The classifier's labels are not there on the first ask ...
        let late = dir.path().join("labels.txt");
        let source = DaemonOccurrence::new(
            Some(BERLIN),
            Some(RangeModelFiles {
                model: mdata,
                labels: None,
                aliases: None,
            }),
            late.clone(),
            SpeciesFilterConfig::default(),
            SpeciesListsProvider::new(SpeciesLists::default),
        );
        let first = source.report(JUNE).unwrap_err();
        assert!(matches!(first, Unavailable::Failed(_)), "{first:?}");
        // ... and are there on the second, so a retry would succeed.
        std::fs::copy(&labels, &late).expect("copy the labels into place");
        assert_eq!(
            source.report(JUNE + 1).unwrap_err(),
            first,
            "the load was tried again"
        );
    }

    /// Each way of not being able to answer is its own answer, and "no model"
    /// outranks "no location": a station with no model is not filtering by place
    /// whether or not it has one.
    #[test]
    fn each_reason_for_no_answer_is_named() {
        assert_eq!(
            source(Some((52.5, 13.4)), None).report(22).unwrap_err(),
            Unavailable::NoModel
        );
        assert_eq!(
            source(None, None).report(22).unwrap_err(),
            Unavailable::NoModel,
            "no model outranks no location"
        );
        assert_eq!(
            source(None, Some("/nonexistent/range.onnx"))
                .report(22)
                .unwrap_err(),
            Unavailable::NoLocation
        );
        // A model that will not load is a reason of its own, and it is the same
        // reason on the next ask: the daemon tried once.
        let failing = source(Some((52.5, 13.4)), Some("/nonexistent/range.onnx"));
        let first = failing.report(22).unwrap_err();
        assert!(matches!(first, Unavailable::Failed(_)), "{first:?}");
        assert_eq!(failing.report(23).unwrap_err(), first);
    }

    // ── what is kept, without a model ───────────────────────────────────
    //
    // The tests above that reuse an answer need the real V2.4 range model and
    // skip without it, so a build with no model files could change the cache
    // and see every test pass. These need none.

    #[test]
    fn an_answer_is_fresh_only_while_it_is_younger_than_the_keep_time() {
        let keep = Duration::from_secs(60);
        assert!(is_fresh(Duration::ZERO, keep));
        assert!(is_fresh(Duration::from_nanos(59_999_999_999), keep));
        assert!(
            !is_fresh(keep, keep),
            "an answer exactly as old as the keep time has expired"
        );
        assert!(!is_fresh(keep + Duration::from_nanos(1), keep));
        assert!(
            !is_fresh(Duration::ZERO, Duration::ZERO),
            "keeping for nothing keeps nothing"
        );
    }

    /// The cache is consulted before the model is: a fresh kept answer comes
    /// back without the files being read (they do not exist here), and an
    /// expired one sends the ask on to the load, which fails.
    #[test]
    fn a_kept_answer_is_served_while_fresh_and_asked_for_afresh_once_it_is_not() {
        let kept = Arc::new(OccurrenceReport {
            latitude: 52.5,
            longitude: 13.4,
            week: 22,
            threshold: 0.03,
            species: Vec::new(),
        });
        let seeded = |keep_for: Duration| {
            let s =
                source(Some((52.5, 13.4)), Some("/nonexistent/range.onnx")).keeping_for(keep_for);
            s.kept
                .lock()
                .unwrap()
                .insert(22, (Instant::now(), Arc::clone(&kept)));
            s
        };

        let fresh = seeded(Duration::from_secs(3_600));
        let served = fresh.report(22).expect("a fresh kept answer is served");
        assert!(Arc::ptr_eq(&served, &kept));

        let expired = seeded(Duration::ZERO);
        let err = expired.report(22).unwrap_err();
        assert!(
            matches!(err, Unavailable::Failed(_)),
            "an expired answer was served from the cache: {err:?}"
        );
    }

    /// `Debug` is how the source appears in a log line: what is configured, and
    /// not where the station is.
    #[test]
    fn the_debug_form_says_what_is_configured_and_not_where_the_station_is() {
        assert_eq!(
            format!("{:?}", source(Some((52.52, 13.405)), None)),
            "DaemonOccurrence { has_location: true, has_model: false, .. }"
        );
        assert_eq!(
            format!("{:?}", source(None, Some("/nonexistent/range.onnx"))),
            "DaemonOccurrence { has_location: false, has_model: true, .. }"
        );
    }
}
