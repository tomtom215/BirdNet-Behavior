//! Integration tests for the first-run onboarding flow (G-09).
//!
//! Covers the first-boot redirect (`GET /` → `/onboarding` only when the
//! station has no detections and isn't onboarded) and the persistence of
//! `POST /onboarding/save` (location/timezone settings + the completion flag).

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use rusqlite::Connection;
use tower::ServiceExt;

use birdnet_web::server::build_router;
use birdnet_web::state::AppState;

/// Fresh station: schema applied, no detections, not onboarded.
fn fresh_state() -> AppState {
    let conn = Connection::open_in_memory().unwrap();
    birdnet_db::migration::migrate(&conn).unwrap();
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

/// A fresh station with a real capture source configured, the way the
/// installer leaves one.
fn state_with_source(label: Option<&str>, device: &str) -> AppState {
    let conn = Connection::open_in_memory().unwrap();
    birdnet_db::migration::migrate(&conn).unwrap();
    let mut new = birdnet_db::audio_sources::NewAudioSource::defaults(
        "src_test_1",
        birdnet_db::audio_sources::SourceKind::UsbAlsa,
        device,
    );
    new.label = label.map(ToString::to_string);
    birdnet_db::audio_sources::AudioSourceStore::insert(&conn, &new).unwrap();
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

async fn fetch_wizard(state: AppState) -> String {
    let resp = build_router(state)
        .oneshot(
            Request::builder()
                .uri("/onboarding")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    String::from_utf8(body.to_vec()).unwrap()
}

/// A station that already has a detection (so it is past first-run).
fn state_with_detection() -> AppState {
    let conn = Connection::open_in_memory().unwrap();
    birdnet_db::migration::migrate(&conn).unwrap();
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            "2026-03-12",
            "06:30:00",
            "Turdus merula",
            "Eurasian Blackbird",
            0.9
        ],
    )
    .unwrap();
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

async fn get_status(router: axum::Router, uri: &str) -> StatusCode {
    router
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}

/// POST a urlencoded body (no Origin/Host header → passes the stateless,
/// origin-based CSRF guard, just like a same-origin browser submit).
async fn post_form(
    router: axum::Router,
    uri: &str,
    body: &'static str,
) -> axum::response::Response {
    router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn fresh_station_redirects_to_onboarding() {
    let resp = build_router(fresh_state())
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(resp.headers().get(header::LOCATION).unwrap(), "/onboarding");
}

#[tokio::test]
async fn station_with_detections_sees_dashboard() {
    assert_eq!(
        get_status(build_router(state_with_detection()), "/").await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn onboarding_wizard_serves() {
    assert_eq!(
        get_status(build_router(fresh_state()), "/onboarding").await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn save_persists_location_and_marks_complete() {
    let state = fresh_state();

    let resp = post_form(
        build_router(state.clone()),
        "/onboarding/save",
        "latitude=51.5&longitude=-0.12&timezone=Europe/London&notification_mode=rare",
    )
    .await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    // Not the dashboard: what was just saved needs a restart, and the next page
    // says what is true now and offers it.
    assert_eq!(
        resp.headers().get(header::LOCATION).unwrap(),
        "/onboarding/done"
    );

    let (lat, lon, tz, complete) = state.with_db(|c| {
        (
            birdnet_db::settings::get(c, "latitude").unwrap(),
            birdnet_db::settings::get(c, "longitude").unwrap(),
            birdnet_db::settings::get(c, "timezone").unwrap(),
            birdnet_db::settings::get(c, "onboarding_complete").unwrap(),
        )
    });
    assert_eq!(lat, "51.5");
    assert_eq!(lon, "-0.12");
    assert_eq!(tz, "Europe/London");
    assert_eq!(complete, "true");

    // Having onboarded, `/` now serves the dashboard instead of redirecting.
    assert_eq!(get_status(build_router(state), "/").await, StatusCode::OK);
}

/// The whole point of the step: a first-run operator is *asked* for the
/// threshold instead of silently inheriting one they never see. Before this
/// existed the wizard never mentioned confidence at all, so a station that
/// wanted stricter (or looser) detection had to find Settings → Detection
/// unprompted.
#[tokio::test]
async fn wizard_prompts_for_the_confidence_threshold() {
    let resp = build_router(fresh_state())
        .oneshot(
            Request::builder()
                .uri("/onboarding")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let html = String::from_utf8(body.to_vec()).unwrap();

    assert!(
        html.contains(r#"name="confidence_threshold""#),
        "the wizard must submit a confidence_threshold field"
    );
    let expected_default = format!(
        r#"id="ob-conf" value="{}""#,
        birdnet_core::config::DEFAULT_CONFIDENCE_THRESHOLD
    );
    assert!(
        html.contains(&expected_default),
        "the pre-selected default must be DEFAULT_CONFIDENCE_THRESHOLD ({expected_default}) \
         — the same value the daemon enforces and Settings → Detection advertises"
    );
    let default_card = format!(
        r#"data-value="{}""#,
        birdnet_core::config::DEFAULT_CONFIDENCE_THRESHOLD
    );
    assert!(
        html.contains(&default_card),
        "the recommended card must carry the shared default ({default_card}): a station on the \
         default would otherwise open the wizard with no card selected"
    );
    for preset in ["0.9", "0.6", "0.4"] {
        assert!(
            html.contains(&format!(r#"data-value="{preset}""#)),
            "missing the {preset} preset"
        );
    }
    assert!(
        html.contains("Step <span id=\"ob-cur\">1</span> of 7"),
        "the step counter must match the number of steps actually rendered"
    );
    // `<section` anchors this to the step bodies — a bare `class="ob-step`
    // also matches the `ob-stepper` container that holds the numbered pips.
    let rendered_steps = html.matches(r#"<section class="ob-step"#).count();
    assert_eq!(
        rendered_steps, 7,
        "7 step sections expected, found {rendered_steps}"
    );
    let pips = html.matches(r#"data-pip=""#).count();
    assert_eq!(
        pips, rendered_steps,
        "the stepper must show one pip per step, else the wizard skips a dot"
    );
    // The welcome copy states the count in prose; it went stale when the
    // Accuracy step was added and nothing would have caught it.
    assert!(
        html.contains("seven steps"),
        "the welcome text must state the real number of steps"
    );
    assert!(!html.contains("six steps") && !html.contains("five steps"));
}

/// The microphone step used to be a mock-up: a hard-coded "UMC202HD · USB
/// audio · card 1 · 48 kHz" card marked *recommended* and pre-selected, a
/// "Built-in microphone · card 0", and two cards offering RTSP and folder
/// watching that did nothing. A first-run operator was shown hardware they do
/// not own, described as already detected — and on a station whose microphone
/// was missing, the wizard's answer to "will this hear anything?" was a
/// confident yes about a device that does not exist.
#[tokio::test]
async fn microphone_step_shows_the_stations_real_source() {
    let html = fetch_wizard(state_with_source(
        Some("Backyard feeder"),
        "plughw:CARD=PRO,DEV=0",
    ))
    .await;

    assert!(
        html.contains("Backyard feeder"),
        "the operator's own label must appear"
    );
    assert!(
        html.contains("plughw:CARD=PRO,DEV=0"),
        "the real device id must appear"
    );
    // "USB microphone", not "USB · ALSA": `kind_label` now names the object on
    // the windowsill rather than the transport it speaks. The assertion's point
    // is unchanged and is the reason it caught this — onboarding and the
    // Capture tab must agree, and they are rendered by different code.
    assert!(
        html.contains("USB microphone"),
        "the kind badge must use the same words as the Capture tab"
    );
    assert!(
        !html.contains("USB · ALSA"),
        "the transport-only label must be gone from onboarding too"
    );
    assert!(
        html.contains("48.0 kHz"),
        "the real capture settings must appear, not a plausible constant"
    );
}

/// A label is operator-controlled text. Rendering must not let it collide with
/// the template's own placeholder syntax — a chained `.replace()` would have
/// re-scanned the inserted label and swapped it for the summary line.
#[tokio::test]
async fn a_label_that_looks_like_a_placeholder_renders_literally() {
    let html = fetch_wizard(state_with_source(Some("{{mic_summary}}"), "plughw:1,0")).await;
    assert!(
        html.contains("{{mic_summary}}"),
        "the operator's literal label must survive rendering"
    );
    assert!(
        !html.contains("{{mic_body}}"),
        "no placeholder may be left unsubstituted"
    );
    // The real summary row is still correct — the label did not displace it.
    assert!(html.contains("plughw:1,0 · 48.0 kHz"));
}

/// An unlabelled source falls back to its device id rather than to a blank —
/// and shows it once, not as both the heading and the detail line.
#[tokio::test]
async fn microphone_step_falls_back_to_the_device_id() {
    let html = fetch_wizard(state_with_source(None, "plughw:1,0")).await;
    let card = mic_step(&html);
    assert!(card.contains("plughw:1,0"), "device id must be shown");
    assert_eq!(
        card.matches("plughw:1,0").count(),
        1,
        "an unlabelled source must not print its device id twice: {card}"
    );

    // With a label there are two distinct things to show, so both appear.
    let labelled = fetch_wizard(state_with_source(Some("Backyard feeder"), "plughw:1,0")).await;
    let card = mic_step(&labelled);
    assert!(card.contains("Backyard feeder") && card.contains("plughw:1,0"));
}

/// A whitespace-only label is the admin form's "no label" state, not a name.
#[tokio::test]
async fn microphone_step_ignores_a_blank_label() {
    let html = fetch_wizard(state_with_source(Some("   "), "plughw:2,0")).await;
    let card = mic_step(&html);
    assert!(
        card.contains("plughw:2,0"),
        "a blank label must fall back to the device id, not render an empty heading"
    );
    assert_eq!(card.matches("plughw:2,0").count(), 1);
}

/// The `data-step="4"` section only — so assertions about the microphone card
/// are not satisfied by the same text appearing in the final summary.
fn mic_step(html: &str) -> &str {
    let start = html
        .find(r#"<section class="ob-step" data-step="4">"#)
        .expect("microphone step present");
    let rest = &html[start..];
    let end = rest.find("</section>").expect("step is closed");
    &rest[..end]
}

/// The case that matters most: a station that will detect nothing. The old
/// wizard claimed a USB mic had been found.
#[tokio::test]
async fn microphone_step_is_honest_when_nothing_is_configured() {
    let html = fetch_wizard(fresh_state()).await;
    assert!(
        html.contains("No audio source configured"),
        "a station with no capture source must be told so"
    );
    assert!(
        html.contains("/station/capture"),
        "and pointed at where to add one"
    );
    assert!(
        html.contains("None configured — no birds will be detected"),
        "the summary must not imply a working microphone"
    );
}

/// Counter-test: every fabricated value that used to ship in the wizard,
/// pinned so none of them can reappear. These were shown to every station
/// regardless of its actual hardware, location or address.
#[tokio::test]
async fn wizard_contains_no_mock_content() {
    for state in [
        fresh_state(),
        state_with_source(Some("Backyard feeder"), "plughw:1,0"),
    ] {
        let html = fetch_wizard(state).await;
        for mock in [
            "UMC202HD", // a microphone model nobody's station reported
            "Built-in microphone",
            "card 0 · 44.1 kHz",
            "card 1 · 48 kHz",
            "detected automatically",
            "Boston, MA", // someone else's location, in the summary
            "42.36, −71.06",
            "Pick channels now", // twelve pills that read as selectable and were not
            "birdnet.local",     // an address that does not resolve on every network
            "Watch a folder",    // an option the wizard never implemented
        ] {
            assert!(
                !html.contains(mock),
                "mock content {mock:?} is still served by the wizard"
            );
        }
    }
}

/// The summary rows that depend on operator input are placeholders the page
/// script fills, not fabricated values baked into the HTML.
#[tokio::test]
async fn summary_rows_start_unset_rather_than_invented() {
    let html = fetch_wizard(fresh_state()).await;
    assert!(html.contains(r#"id="ob-sum-loc">Not set<"#));
    assert!(html.contains(r#"id="ob-sum-url">—<"#));
    assert!(html.contains(r#"id="ob-sum-notify""#));
    assert!(
        html.contains("window.location.origin"),
        "the dashboard address must come from the address the operator reached"
    );
}

/// `ONBOARDING_SETTING_KEYS` is what the wiring guard in the binary checks, so
/// it has to be the truth about what the wizard writes — a key the handler
/// persists but the list omits would slip past the guard exactly the way
/// `notification_mode` did. Submitting a fully-populated form and comparing the
/// keys that actually land in the settings table keeps the two honest.
#[tokio::test]
async fn declared_onboarding_keys_match_what_a_full_submit_writes() {
    let state = fresh_state();
    let resp = post_form(
        build_router(state.clone()),
        "/onboarding/save",
        "latitude=51.5&longitude=-0.12&timezone=Europe/London\
         &station_name=Back+garden&database_lang=de\
         &notification_mode=new-species&confidence_threshold=0.75",
    )
    .await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);

    let mut written: Vec<String> = state.with_db(|c| {
        let mut stmt = c.prepare("SELECT key FROM settings").unwrap();
        stmt.query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    });
    written.sort();

    let mut declared: Vec<String> = birdnet_web::routes::pages::onboarding::ONBOARDING_SETTING_KEYS
        .iter()
        .map(ToString::to_string)
        .collect();
    declared.sort();

    assert_eq!(
        written, declared,
        "ONBOARDING_SETTING_KEYS must list exactly the keys the wizard persists — \
         the wiring guard in the binary trusts it"
    );
}

/// The Alerts step used to write `notification_mode`, a key no code anywhere
/// read: an operator picked "Quiet" or "Everything" on their first day and it
/// governed nothing. The live key is `notify_trigger`, bridged onto
/// `APPRISE_TRIGGER` and consumed by the notification filter.
#[tokio::test]
async fn save_persists_the_alerts_choice_to_the_key_the_runtime_reads() {
    for mode in ["each", "new-species", "new-species-daily"] {
        let state = fresh_state();
        let body = format!("notification_mode={mode}");
        let resp = build_router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/onboarding/save")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);

        let stored = state.with_db(|c| birdnet_db::settings::get(c, "notify_trigger").unwrap());
        assert_eq!(stored, mode);
        // The dead key must not come back.
        assert!(
            state
                .with_db(|c| birdnet_db::settings::get(c, "notification_mode"))
                .is_err(),
            "notification_mode is read by nothing and must not be written"
        );
    }
}

/// `TriggerMode::parse` maps anything unrecognised to "every detection" — the
/// chattiest mode. So an unvalidated value is not merely ignored, it silently
/// selects the opposite of a quieter choice.
#[tokio::test]
async fn save_rejects_an_unknown_alerts_value_rather_than_defaulting_to_chatty() {
    for bad in ["rare", "quiet", "daily", "everything", "", "EACH"] {
        let state = fresh_state();
        let body = format!("notification_mode={bad}");
        let resp = build_router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/onboarding/save")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        assert!(
            state
                .with_db(|c| birdnet_db::settings::get(c, "notify_trigger"))
                .is_err(),
            "{bad:?} is not a trigger the runtime understands and must not be stored"
        );
    }
}

/// The step's cards must offer exactly the values the runtime accepts — the old
/// four (`quiet`/`rare`/`daily`/`everything`) matched none of them.
#[tokio::test]
async fn alerts_step_offers_only_real_trigger_modes() {
    let html = fetch_wizard(fresh_state()).await;
    for real in ["new-species", "new-species-daily", "each"] {
        assert!(
            html.contains(&format!(r#"data-radio="notify" data-value="{real}""#)),
            "missing the {real} card"
        );
    }
    assert!(
        html.contains(r#"id="ob-notify" value="new-species""#),
        "the recommended mode must be pre-selected"
    );
}

#[tokio::test]
async fn save_persists_the_chosen_confidence_threshold() {
    let state = fresh_state();
    let resp = post_form(
        build_router(state.clone()),
        "/onboarding/save",
        "latitude=51.5&longitude=-0.12&confidence_threshold=0.85",
    )
    .await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);

    let conf = state.with_db(|c| birdnet_db::settings::get(c, "confidence_threshold").unwrap());
    assert_eq!(
        conf, "0.85",
        "the wizard's choice must land in the settings table the overlay reads"
    );
}

/// Counter-test: the field is a plain form value, and an out-of-range
/// `CONFIDENCE` is a *fatal* doctor error (`ExecStartPre` exit 2). A crafted
/// POST must not be able to leave the station unable to start.
#[tokio::test]
async fn save_rejects_a_confidence_the_daemon_would_refuse_to_start_on() {
    for bad in ["70", "-0.5", "abc", "1.5", ""] {
        let state = fresh_state();
        let body = format!("confidence_threshold={bad}");
        let resp = build_router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/onboarding/save")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);

        let stored = state.with_db(|c| birdnet_db::settings::get(c, "confidence_threshold"));
        assert!(
            stored.is_err(),
            "confidence_threshold={bad:?} must not be persisted, got {stored:?}"
        );
        // The rejection must not derail the rest of the wizard.
        let complete =
            state.with_db(|c| birdnet_db::settings::get(c, "onboarding_complete").unwrap());
        assert_eq!(complete, "true");
    }
}

/// The in-range boundaries the daemon does accept must still be storable —
/// the guard rejects the unusable, not the unusual.
#[tokio::test]
async fn save_accepts_in_range_boundary_confidences() {
    for good in ["0", "1", "0.05", "1.0"] {
        let state = fresh_state();
        let body = format!("confidence_threshold={good}");
        let resp = build_router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/onboarding/save")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        let stored =
            state.with_db(|c| birdnet_db::settings::get(c, "confidence_threshold").unwrap());
        assert_eq!(stored, good);
    }
}

#[tokio::test]
async fn save_with_no_fields_still_completes_without_writing_blanks() {
    let state = fresh_state();

    let resp = post_form(build_router(state.clone()), "/onboarding/save", "").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);

    let complete = state.with_db(|c| birdnet_db::settings::get(c, "onboarding_complete").unwrap());
    assert_eq!(
        complete, "true",
        "clicking through must still mark complete"
    );

    // No empty latitude row was written.
    let lat = state.with_db(|c| birdnet_db::settings::get(c, "latitude"));
    assert!(
        lat.is_err(),
        "an empty submit must not persist a blank latitude"
    );
}

/// UX-1. The seven preference cards were bare `<div>`s with a delegated click
/// handler: no role, no `tabindex`, nothing a keyboard could reach, so an
/// operator without a mouse could set neither the threshold nor the alert
/// mode during first-run setup, and the axe gate — which grades what is
/// there, not what is missing — passed the route clean. Each card is now a
/// `<label>` around a real radio input, so Tab reaches the group and the
/// arrow keys move within it; `tools/visual-qa/interactions.mjs` drives that
/// in a browser. This is the cheap structural half: every card carries the
/// input, and no bare-`<div>` card remains.
#[tokio::test]
async fn every_preference_card_is_a_real_radio_input() {
    let html = fetch_wizard(fresh_state()).await;
    // The page's script also spells `[data-radio="conf"]` in selectors; only
    // the markup pairs it with `data-value`.
    let mut starts: Vec<usize> = ["conf", "notify"]
        .iter()
        .flat_map(|g| {
            html.match_indices(&format!(r#"data-radio="{g}" data-value=""#))
                .map(|(i, _)| i)
                .collect::<Vec<_>>()
        })
        .collect();
    starts.sort_unstable();
    let cards: Vec<&str> = starts
        .into_iter()
        .map(|i| {
            let start = html[..i].rfind('<').expect("a tag opens the card");
            let end = i + html[i..]
                .find("</label>")
                .map_or(html.len() - i, |e| e + "</label>".len());
            &html[start..end]
        })
        .collect();
    assert_eq!(cards.len(), 7, "four threshold cards and three alert cards");
    for card in &cards {
        let head = &card[..card.find('>').unwrap_or(card.len())];
        assert!(
            head.starts_with("<label "),
            "a card must be a <label> so a click still reaches its input: {head}"
        );
        assert!(
            card.contains(r#"type="radio""#) && card.contains(r#"name=""#),
            "a card must wrap a named radio input, so Tab reaches it and the \
             arrow keys move between cards: {card}"
        );
    }
    assert!(
        !html.contains(r#"<div class="ob-card" data-radio="#),
        "a bare <div> card is unreachable from the keyboard"
    );
    for group in ["conf", "notify"] {
        assert!(
            html.contains(&format!(
                r#"role="radiogroup" aria-labelledby="ob-{group}-h""#
            )),
            "the {group} cards must be announced as one group under the step's heading"
        );
    }
}

// ---------------------------------------------------------------------------
// The setup that needs no terminal: station name, language, microphone, model
// ---------------------------------------------------------------------------

/// What `POST /onboarding/save` leaves in `audio_sources`, as (kind, device).
fn sources_of(state: &AppState) -> Vec<(String, String)> {
    state.with_db(|c| {
        let mut stmt = c
            .prepare("SELECT kind, device_id FROM audio_sources WHERE disabled_at IS NULL ORDER BY id")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    })
}

fn setting(state: &AppState, key: &str) -> Option<String> {
    state.with_db(|c| birdnet_db::settings::get(c, key).ok())
}

/// The station's name and the language of its bird names are asked for, and
/// stored under the keys the rest of the station reads.
#[tokio::test]
async fn save_persists_the_station_name_and_the_language() {
    let state = fresh_state();
    let resp = post_form(
        build_router(state.clone()),
        "/onboarding/save",
        "station_name=Back+garden&database_lang=de",
    )
    .await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(setting(&state, "station_name").as_deref(), Some("Back garden"));
    assert_eq!(setting(&state, "database_lang").as_deref(), Some("de"));
}

/// The counterpart: a blank name clears nothing and a language the station has
/// no names for is not stored (it would be read as English and look chosen).
#[tokio::test]
async fn save_ignores_a_blank_name_and_an_unknown_language() {
    let state = fresh_state();
    state.with_db(|c| {
        birdnet_db::settings::set(
            c,
            "station_name",
            "Kept",
            birdnet_db::settings::SettingsCategory::Location,
        )
        .unwrap();
    });
    let resp = post_form(
        build_router(state.clone()),
        "/onboarding/save",
        "station_name=++&database_lang=klingon",
    )
    .await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(setting(&state, "station_name").as_deref(), Some("Kept"));
    assert_eq!(setting(&state, "database_lang"), None);
}

/// A stream address typed into the wizard becomes the same row the Capture tab
/// makes, and the wizard still completes.
#[tokio::test]
async fn a_network_stream_chosen_in_the_wizard_becomes_an_audio_source() {
    let state = fresh_state();
    let resp = post_form(
        build_router(state.clone()),
        "/onboarding/save",
        "mic_choice=rtsp&mic_rtsp_url=rtsp%3A%2F%2Fcamera.local%3A554%2Fstream",
    )
    .await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        resp.headers().get(header::LOCATION).unwrap(),
        "/onboarding/done"
    );
    assert_eq!(
        sources_of(&state),
        [("rtsp".to_owned(), "rtsp://camera.local:554/stream".to_owned())]
    );
    assert_eq!(setting(&state, "onboarding_complete").as_deref(), Some("true"));
}

/// A sound card named by hand is added as a USB microphone.
#[tokio::test]
async fn a_device_named_by_hand_becomes_an_audio_source() {
    let state = fresh_state();
    let resp = post_form(
        build_router(state.clone()),
        "/onboarding/save",
        "mic_choice=manual&mic_manual_device=plughw%3ACARD%3DDevice%2CDEV%3D0",
    )
    .await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        sources_of(&state),
        [("usb-alsa".to_owned(), "plughw:CARD=Device,DEV=0".to_owned())]
    );
}

/// The same device or stream is not added twice, however often the wizard is run.
#[tokio::test]
async fn a_source_the_station_already_has_is_not_added_again() {
    let state = state_with_source(None, "plughw:CARD=Device,DEV=0");
    let resp = post_form(
        build_router(state.clone()),
        "/onboarding/save",
        "mic_choice=manual&mic_manual_device=plughw%3ACARD%3DDevice%2CDEV%3D0",
    )
    .await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(sources_of(&state).len(), 1);
}

/// A refused microphone saves **nothing** and sends the person back to the
/// microphone step with the reason: half a setup, with the wizard marked
/// complete and no way back to it, is the worst outcome.
#[tokio::test]
async fn a_refused_microphone_saves_nothing_and_goes_back_to_that_step() {
    for (body, code) in [
        (
            "latitude=51.5&longitude=-0.12&mic_choice=rtsp&mic_rtsp_url=not-a-url",
            "mic-url",
        ),
        (
            "latitude=51.5&longitude=-0.12&mic_choice=manual&mic_manual_device=rm+-rf+%2F",
            "mic-device",
        ),
        // A card picked from the list must be one the kernel lists *now*: this
        // machine lists none, so a hand-made POST naming one is refused.
        (
            "latitude=51.5&longitude=-0.12&mic_choice=alsa%3Aplughw%3ACARD%3DGhost%2CDEV%3D0",
            "mic-device",
        ),
    ] {
        let state = fresh_state();
        let resp = post_form(build_router(state.clone()), "/onboarding/save", body).await;
        assert_eq!(resp.status(), StatusCode::SEE_OTHER, "{body}");
        assert_eq!(
            resp.headers().get(header::LOCATION).unwrap().to_str().unwrap(),
            format!("/onboarding?error={code}"),
            "{body}"
        );
        assert!(sources_of(&state).is_empty(), "{body}");
        assert_eq!(setting(&state, "latitude"), None, "{body}: nothing else was saved");
        assert_eq!(setting(&state, "onboarding_complete"), None, "{body}");
    }
}

/// Coming back with the refusal opens the wizard on the microphone step and says
/// what was wrong, in the step.
#[tokio::test]
async fn the_wizard_reopens_on_the_microphone_step_with_the_reason() {
    let resp = build_router(fresh_state())
        .oneshot(
            Request::builder()
                .uri("/onboarding?error=mic-url")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = String::from_utf8(
        axum::body::to_bytes(resp.into_body(), 1 << 20)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(body.contains("var step = 4, total = 7"), "opens on step 4");
    assert!(mic_step(&body).contains("That stream address was not accepted"));
    let calm = fetch_wizard(fresh_state()).await;
    assert!(calm.contains("var step = 1, total = 7"));
    assert!(!calm.contains("was not accepted"));
}

// ── a failed read is not an empty station ───────────────────────────────

/// The wizard used to draw itself from `unwrap_or_else(|_| (Vec::new(),
/// Prefill::default()))`: a database that could not be read produced the wizard
/// of a station with no microphone and blank settings, whose Finish would write
/// those blanks (and the never-empty hidden confidence and alert fields) over
/// the real ones.
///
/// Observed failing against that code: the page answered 200 and said "No audio
/// source configured".
#[tokio::test]
async fn an_unreadable_source_table_is_not_drawn_as_a_station_with_no_microphone() {
    let state = state_with_source(Some("Backyard feeder"), "plughw:1,0");
    state.with_db(|c| c.execute("DROP TABLE audio_sources", []).unwrap());
    let resp = build_router(state)
        .oneshot(Request::builder().uri("/onboarding").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = String::from_utf8(
        axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap().to_vec(),
    )
    .unwrap();
    assert!(!body.contains("No audio source configured"), "{body}");
    assert!(!body.contains("ob-stepper"), "no wizard was drawn");
    assert!(body.contains("could not read"), "and it says why: {body}");
}

/// The same for the stored settings, which the hidden fields are filled from.
#[tokio::test]
async fn unreadable_settings_are_not_drawn_as_defaults() {
    let state = fresh_state();
    state.with_db(|c| c.execute("DROP TABLE settings", []).unwrap());
    let resp = build_router(state)
        .oneshot(Request::builder().uri("/onboarding").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

/// The counterparts, so "503 on any failure" is not what passes: an empty
/// station and a station with a source both draw the wizard.
#[tokio::test]
async fn a_readable_station_still_draws_the_wizard() {
    for state in [fresh_state(), state_with_source(Some("Feeder"), "plughw:1,0")] {
        let html = fetch_wizard(state).await;
        assert!(html.contains("ob-stepper"));
    }
}

// ── the model step ──────────────────────────────────────────────────────

fn model_step(html: &str) -> &str {
    let start = html
        .find(r#"<section class="ob-step" data-step="2">"#)
        .expect("model step present");
    let rest = &html[start..];
    &rest[..rest.find("</section>").expect("closed")]
}

/// A station whose classifier is not installed is told so, in the step, with
/// the button that fixes it — and the summary row does not claim otherwise.
#[tokio::test]
async fn the_model_step_offers_the_install_when_nothing_is_installed() {
    let dir = tempfile::tempdir().unwrap();
    let state = fresh_state();
    state
        .classifier_setup()
        .configure(birdnet_web::classifier_setup::Layout {
            model_dir: dir.path().to_path_buf(),
            ..Default::default()
        });
    let html = fetch_wizard(state).await;
    let step = model_step(&html);
    assert!(step.contains(r#"data-cls-state="missing""#), "{step}");
    assert!(step.contains("data-cls-install"), "the button is there: {step}");
    assert!(step.contains("MB"), "and says how big the download is: {step}");
    assert!(!step.contains("Listening"), "nothing is listening: {step}");
}

/// Files in place but not loaded by this start: "restart", never "listening".
#[tokio::test]
async fn installed_but_not_loaded_is_not_reported_as_listening() {
    let dir = tempfile::tempdir().unwrap();
    for id in ["birdnet-v24", "birdnet-v24-range-v2"] {
        let e = birdnet_integrations::model_catalog::find(id).unwrap();
        let (model, labels) = birdnet_integrations::model_catalog::installed_paths(e, dir.path());
        std::fs::File::create(&model)
            .and_then(|f| f.set_len(e.model_bytes))
            .unwrap();
        if let Some(l) = labels {
            std::fs::write(l, "x\n").unwrap();
        }
    }
    let state = fresh_state();
    state
        .classifier_setup()
        .configure(birdnet_web::classifier_setup::Layout {
            model_dir: dir.path().to_path_buf(),
            ..Default::default()
        });
    let step_html = fetch_wizard(state).await;
    let step = model_step(&step_html);
    assert!(step.contains(r#"data-cls-state="needs-restart""#), "{step}");
    assert!(step.contains("data-cls-restart"), "{step}");
    assert!(!step.contains("data-cls-install"), "nothing left to install: {step}");
}

// ── the page after Finish ───────────────────────────────────────────────

async fn fetch(state: AppState, uri: &str) -> (StatusCode, String) {
    let resp = build_router(state)
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let body = String::from_utf8(
        axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap().to_vec(),
    )
    .unwrap();
    (status, body)
}

/// After Finish, with something saved since the station started, the page offers
/// the one restart button and does not say the station is listening.
#[tokio::test]
async fn the_page_after_finish_offers_the_restart_when_a_save_is_waiting() {
    let state = fresh_state();
    state.pending_restart().mark_booted("2000-01-01 00:00:00".to_owned());
    post_form(
        build_router(state.clone()),
        "/onboarding/save",
        "latitude=51.5&longitude=-0.12",
    )
    .await;
    let (status, body) = fetch(state, "/onboarding/done").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("data-restart-now"), "{body}");
    assert!(body.contains("Almost"), "{body}");
    assert!(!body.contains("You're <em>listening"), "{body}");
}

/// The counterpart: nothing waiting and nothing to restart for, no button — and
/// still no claim to be listening without a running classifier.
#[tokio::test]
async fn the_page_after_finish_has_no_restart_button_when_nothing_waits() {
    let (status, body) = fetch(fresh_state(), "/onboarding/done").await;
    assert_eq!(status, StatusCode::OK);
    assert!(!body.contains("data-restart-now"), "{body}");
    assert!(
        !body.contains("You're <em>listening"),
        "no classifier is running, so the page must not say so: {body}"
    );
}
