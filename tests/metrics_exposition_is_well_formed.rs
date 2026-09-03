//! `/api/v2/metrics` must be a document a Prometheus text parser accepts.
//!
//! # What was wrong
//!
//! The endpoint is composed from two halves that never knew about each other.
//! The handler wrote a whole-database row count:
//!
//! ```text
//! crates/birdnet-web/src/routes/health.rs:97
//!     # TYPE birdnet_detections_total gauge
//!     birdnet_detections_total 3
//! ```
//!
//! and then appended `crate::metrics::render_runtime_metrics`, whose first act
//! was:
//!
//! ```text
//! crates/birdnet-web/src/metrics.rs:499
//!     # TYPE birdnet_detections_total counter
//!     birdnet_detections_total{species="Pica pica",chunk_offset="0"} 1
//! ```
//!
//! One name, two `# HELP` lines, two `# TYPE` lines, two meanings — and one of
//! them a gauge that *decreases* when a row is deleted or purged. The
//! Prometheus text format forbids this. `expfmt.TextParser` — what
//! `promtool check metrics`, Telegraf's `inputs.prometheus`, the Python client
//! and most collection agents use — rejects the **entire document**, so such an
//! agent got nothing at all from the station, including
//! `birdnet_detection_silence_seconds`, the one series that says the station has
//! stopped detecting. A Prometheus server's own scrape parser is more forgiving:
//! it takes both series and keeps whichever `# TYPE` came last, so the shipped
//! dashboard's `sum by (species)(rate(birdnet_detections_total[1m]))` folded a
//! decreasing gauge into a counter under `species=""`, where every purge reads
//! as a counter reset and manufactures a spike.
//!
//! Nothing caught it because the only test of this endpoint asserted three
//! substrings. Nothing had ever parsed the response.
//!
//! # What this gate holds
//!
//! Three structural rules over the **composed** body — the bytes actually
//! served, not either half in isolation, which is where the defect lived:
//!
//! 1. no metric name carries more than one `# TYPE` or `# HELP` declaration;
//! 2. every sample line names a metric that was declared;
//! 3. the `_total` suffix is reserved for counters, per the Prometheus naming
//!    convention — which is the rule whose violation produced (1).
//!
//! Observed failing before the fix, all three, with rule 3 printing:
//!
//! ```text
//! the _total suffix is reserved for counters: [
//!   "birdnet_detections_rejected_total is [\"gauge\"]",
//!   "birdnet_detections_total is [\"gauge\", \"counter\"]",
//!   "birdnet_species_total is [\"gauge\"]"]
//! ```
//!
//! — which is one more offender than the audit that prompted this gate had
//! found. `birdnet_species_total` was a gauge wearing a counter's suffix too;
//! nobody had noticed, because nothing had ever looked at the types.

use std::collections::BTreeMap;

use birdnet_web::state::AppState;

/// A station with a couple of rows, so the database half of the exposition is
/// non-trivial, and one recorded detection so the runtime half emits its
/// labelled counter. Both halves must be present or this gate proves nothing.
fn station(dir: &std::path::Path) -> AppState {
    let db = dir.join("birds.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    birdnet_db::migration::migrate(&conn).unwrap();
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, Sens, Overlap, File_Name)
         VALUES ('2026-05-19', '06:30:00', 'Pica pica', 'Eurasian Magpie', 0.91, 0.7, 19, 1.25, 0.0, 'a.wav')",
        [],
    )
    .unwrap();
    AppState::from_connection(conn, db)
}

async fn metrics_body(state: &AppState) -> String {
    use tower::ServiceExt as _;
    let response = birdnet_web::server::build_router(state.clone())
        .oneshot(
            axum::http::Request::builder()
                .uri("/api/v2/metrics")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).expect("the exposition must be UTF-8")
}

/// The metric name a sample line refers to: everything before `{` or the first
/// space.
fn sample_name(line: &str) -> &str {
    let head = line.split(' ').next().unwrap_or(line);
    head.split('{').next().unwrap_or(head)
}

/// `# TYPE <name> <type>` declarations, in order of appearance, per name.
fn type_declarations(body: &str) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for line in body.lines() {
        if let Some(rest) = line.strip_prefix("# TYPE ") {
            let mut parts = rest.split_whitespace();
            if let (Some(name), Some(kind)) = (parts.next(), parts.next()) {
                out.entry(name.to_string())
                    .or_default()
                    .push(kind.to_string());
            }
        }
    }
    out
}

#[tokio::test]
async fn no_metric_name_is_declared_twice() {
    let dir = tempfile::tempdir().unwrap();
    let body = metrics_body(&station(dir.path())).await;

    // A `# HELP` count, tracked separately: `expfmt.TextParser` rejects on the
    // second HELP line specifically, so this is the rule that actually decided
    // whether an agent got any data at all.
    let mut helps: BTreeMap<&str, usize> = BTreeMap::new();
    for line in body.lines() {
        if let Some(rest) = line.strip_prefix("# HELP ")
            && let Some(name) = rest.split_whitespace().next()
        {
            *helps.entry(name).or_default() += 1;
        }
    }
    let repeated_help: Vec<_> = helps.iter().filter(|(_, n)| **n > 1).collect();
    assert!(
        repeated_help.is_empty(),
        "a metric name may carry one HELP line; repeated: {repeated_help:?}"
    );

    let repeated_type: Vec<_> = type_declarations(&body)
        .into_iter()
        .filter(|(_, kinds)| kinds.len() > 1)
        .map(|(name, kinds)| format!("{name} declared {} times as {kinds:?}", kinds.len()))
        .collect();
    assert!(
        repeated_type.is_empty(),
        "a metric name has exactly one type: {repeated_type:?}"
    );
}

#[tokio::test]
async fn every_sample_names_a_declared_metric() {
    let dir = tempfile::tempdir().unwrap();
    let body = metrics_body(&station(dir.path())).await;
    let declared = type_declarations(&body);

    let mut undeclared: Vec<String> = Vec::new();
    for line in body.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let name = sample_name(line);
        // Histograms expand into `_bucket`, `_sum` and `_count` series that
        // share their family's single declaration; summaries likewise.
        let family = name
            .strip_suffix("_bucket")
            .or_else(|| name.strip_suffix("_sum"))
            .or_else(|| name.strip_suffix("_count"))
            .unwrap_or(name);
        if !declared.contains_key(name) && !declared.contains_key(family) {
            undeclared.push(name.to_string());
        }
    }
    undeclared.sort_unstable();
    undeclared.dedup();
    assert!(
        undeclared.is_empty(),
        "every sample must belong to a declared family: {undeclared:?}"
    );

    // The counterpart, so a body that declared nothing and sampled nothing
    // would not pass: both halves of the exposition must actually be present.
    assert!(
        declared.contains_key("birdnet_detections_stored"),
        "the database half of the exposition is missing"
    );
    assert!(
        declared.contains_key("birdnet_http_responses_total"),
        "the runtime half of the exposition is missing"
    );
}

/// `_total` is the Prometheus convention for a counter. A gauge wearing it is
/// how two different metrics came to want the same name in the first place.
#[tokio::test]
async fn only_counters_carry_the_total_suffix() {
    let dir = tempfile::tempdir().unwrap();
    let body = metrics_body(&station(dir.path())).await;

    let offenders: Vec<String> = type_declarations(&body)
        .into_iter()
        .filter(|(name, kinds)| name.ends_with("_total") && kinds.iter().any(|k| k != "counter"))
        .map(|(name, kinds)| format!("{name} is {kinds:?}"))
        .collect();
    assert!(
        offenders.is_empty(),
        "the _total suffix is reserved for counters: {offenders:?}"
    );

    // And the discrimination: a body with no `_total` counters at all would
    // pass the assertion above vacuously.
    let counters = type_declarations(&body)
        .into_iter()
        .filter(|(name, kinds)| name.ends_with("_total") && kinds.iter().all(|k| k == "counter"))
        .count();
    assert!(
        counters >= 3,
        "the exposition must still carry its counters; found {counters}"
    );
}

// ── tri-state gauges ────────────────────────────────────────────────────

/// The gauges that can answer "I cannot tell".
///
/// Three of them now: clock synchronisation (no systemd to ask), MQTT presence
/// (no broker configured), detection freshness (no detections yet). Each must
/// be **absent** in that case rather than rendering a `0`, because a `0` is a
/// statement — "the clock is wrong", "the broker is unreachable" — and an
/// operator who alerts on it gets paged about a station that is fine.
const TRI_STATE_GAUGES: [&str; 3] = [
    "birdnet_clock_synced",
    "birdnet_mqtt_connected",
    "birdnet_detection_silence_seconds",
];

#[tokio::test]
async fn an_unanswerable_gauge_is_absent_rather_than_zero() {
    let dir = tempfile::tempdir().unwrap();
    let state = station(dir.path());
    // Nothing has set any of them: no clock probe has run, no MQTT presence
    // loop, no deadman poll.
    let body = metrics_body(&state).await;
    for name in TRI_STATE_GAUGES {
        assert!(
            !body.lines().any(|l| sample_name(l) == name),
            "{name} must not be emitted before anything has measured it:\n{body}"
        );
    }
}

#[tokio::test]
async fn a_measured_gauge_renders_and_declares_its_type() {
    let dir = tempfile::tempdir().unwrap();
    let state = station(dir.path());
    // The counterpart: absence must mean "unmeasured", not "never emitted".
    state.metrics().set_clock_synced(Some(false));
    state.metrics().set_mqtt_connected(true);
    state.metrics().set_detection_silence_secs(Some(42));

    let body = metrics_body(&state).await;
    for (name, want) in [
        ("birdnet_clock_synced", "birdnet_clock_synced 0"),
        ("birdnet_mqtt_connected", "birdnet_mqtt_connected 1"),
        (
            "birdnet_detection_silence_seconds",
            "birdnet_detection_silence_seconds 42",
        ),
    ] {
        assert!(
            body.lines().any(|l| l == want),
            "expected the sample line `{want}`:\n{body}"
        );
        assert!(
            body.contains(&format!("# TYPE {name} gauge")),
            "{name} must declare its type"
        );
    }
}
