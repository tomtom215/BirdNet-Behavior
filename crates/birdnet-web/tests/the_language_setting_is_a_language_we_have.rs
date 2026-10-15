//! The "Common-name language" control offers only languages the station has
//! names for, says how much of the installed classifier each covers, and refuses
//! anything else.
//!
//! # The defect
//!
//! The control was a free-text box labelled "Two-letter code", and the setting
//! behind it did nothing: `init_i18n` loaded a language only from a
//! `--labels-dir` that no installer created, logged a warning when there was
//! none, and `I18nManager::translate` had no caller. A station owner could type
//! `de`, see the form accept it, and get English for ever.
//!
//! # What is guarded
//!
//! 1. The page offers every language the build has, as a `<select>`, with the
//!    stored one chosen. A text input for the field would let the old defect
//!    back in.
//! 2. Each option states the share of *this station's classifier* it covers —
//!    from the label set, not from the table's own 7,058 species — so a Korean
//!    station is told before it chooses that most birds stay English.
//! 3. A hand-made request naming no language is refused whole, and a real one
//!    saves.
//! 4. A stored value from the old text box that is no language selects English
//!    and says so, and re-saving the page replaces it rather than being refused
//!    forever over a value the page no longer lets anyone type.
//! 5. `/api/v2/languages` reports the same languages, the active one, and the
//!    same coverage.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use birdnet_core::i18n::{LANGUAGES, LanguagePack, NameLocalizer};
use birdnet_core::inference::labels::LabelSet;
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// Two species both German and Korean tables name, plus one only German does,
/// plus one no table names: German covers 3 of 4, Korean 2 of 4.
const LABELS: &str = "Turdus merula_Common Blackbird\n\
                      Pica pica_Eurasian Magpie\n\
                      Acanthisitta chloris_Rifleman\n\
                      Zzzz zzzz_No Such Bird\n";

fn station(language: &str) -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let labels = LabelSet::load_from_str(LABELS).expect("labels");
    let names = NameLocalizer::new(
        LanguagePack::bundled(language).expect("pack"),
        Some(&labels),
    );
    let state = AppState::new(dir.path().join("birds.db"))
        .expect("state")
        .with_names(names);
    (dir, state)
}

async fn send(state: &AppState, req: Request<Body>) -> (StatusCode, String) {
    let res = build_router(state.clone())
        .oneshot(req)
        .await
        .expect("response");
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 22)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

async fn get(state: &AppState, uri: &str) -> (StatusCode, String) {
    send(
        state,
        Request::builder().uri(uri).body(Body::empty()).unwrap(),
    )
    .await
}

async fn post_settings(state: &AppState, body: &str) -> (StatusCode, String) {
    let req = Request::builder()
        .method("POST")
        .uri("/admin/settings")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(body.to_owned()))
        .expect("request");
    send(state, req).await
}

fn stored(state: &AppState, key: &str) -> Option<String> {
    state.with_db(|conn| {
        conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
            r.get::<_, String>(0)
        })
        .ok()
    })
}

fn store(state: &AppState, key: &str, value: &str) {
    state.with_db(|conn| {
        birdnet_db::settings::ensure_settings_table(conn).expect("table");
        birdnet_db::settings::set(
            conn,
            key,
            value,
            birdnet_db::settings::SettingsCategory::System,
        )
        .expect("set");
    });
}

/// The `<select id="database_lang">…</select>` element, so assertions are about
/// the control and not about words that appear elsewhere on the page.
fn picker(page: &str) -> &str {
    let start = page
        .find(r#"<select id="database_lang""#)
        .expect("the language control must be a <select id=\"database_lang\">");
    let rest = &page[start..];
    &rest[..rest.find("</select>").expect("select closes")]
}

#[tokio::test]
async fn the_page_offers_every_language_in_a_select_with_the_stored_one_chosen() {
    let (_dir, state) = station("en");
    store(&state, "database_lang", "de");
    let (status, page) = get(&state, "/admin/settings").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !page.contains(r#"name="database_lang" type="text""#),
        "a free-text box is how the setting came to promise what it did not do"
    );
    let select = picker(&page);
    for l in LANGUAGES {
        assert!(
            select.contains(&format!(r#"<option value="{}""#, l.code)),
            "no option for {}",
            l.code
        );
    }
    assert_eq!(select.matches(" selected").count(), 1, "exactly one choice");
    assert!(
        select.contains(r#"<option value="de" selected>"#),
        "{select}"
    );
}

#[tokio::test]
async fn each_option_states_its_share_of_the_installed_classifier() {
    // The station is in English, yet can see what German and Korean would give it.
    let (_dir, state) = station("en");
    let (_, page) = get(&state, "/admin/settings").await;
    let select = picker(&page);
    assert!(
        select.contains("German (Deutsch) — 75%"),
        "3 of 4 species have a German name: {select}"
    );
    assert!(
        select.contains("Korean (한국어) — 50%"),
        "2 of 4 species have a Korean name: {select}"
    );
    assert!(
        page.contains("the share of the 4 species your classifier can report"),
        "the percentages must say what they are a share of"
    );
}

#[tokio::test]
async fn a_request_naming_no_language_is_refused_whole_and_a_real_one_saves() {
    for bad in ["xx", "eng", "zh", "english", "de%00"] {
        let (_dir, state) = station("en");
        let (status, body) = post_settings(
            &state,
            &format!("database_lang={bad}&site_name=Should+Not+Persist"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            body.contains("Nothing was saved"),
            "{bad:?} was accepted: {body}"
        );
        assert_eq!(stored(&state, "database_lang"), None, "{bad:?} was written");
        assert_eq!(
            stored(&state, "site_name"),
            None,
            "the rest rode along: {bad:?}"
        );
    }
    for good in ["de", "zh_CN", "pt"] {
        let (_dir, state) = station("en");
        let (_, body) = post_settings(&state, &format!("database_lang={good}")).await;
        assert!(
            !body.contains("Nothing was saved"),
            "{good} refused: {body}"
        );
        assert_eq!(stored(&state, "database_lang").as_deref(), Some(good));
    }
}

#[tokio::test]
async fn an_old_stored_value_that_is_no_language_selects_english_and_is_replaced_on_save() {
    let (_dir, state) = station("en");
    store(&state, "database_lang", "eng");
    let (_, page) = get(&state, "/admin/settings").await;
    let select = picker(&page);
    assert!(
        select.contains(r#"<option value="en" selected>"#),
        "{select}"
    );
    assert!(
        page.contains("The stored value <code>eng</code> is not a language"),
        "the page must say why English is shown"
    );
    // What the page then submits is a real choice, and saves.
    let (_, body) = post_settings(&state, "database_lang=en").await;
    assert!(!body.contains("Nothing was saved"), "{body}");
    assert_eq!(stored(&state, "database_lang").as_deref(), Some("en"));
}

#[tokio::test]
async fn the_languages_endpoint_reports_the_same_languages_and_coverage() {
    let (_dir, state) = station("de");
    let (status, body) = get(&state, "/api/v2/languages").await;
    assert_eq!(status, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(v["active"], "de");
    assert_eq!(v["count"], LANGUAGES.len());
    let langs = v["languages"].as_array().expect("array");
    assert_eq!(langs.len(), LANGUAGES.len());
    let by = |code: &str| langs.iter().find(|l| l["code"] == code).expect(code);
    assert_eq!(by("de")["active"], true);
    assert_eq!(by("ko")["active"], false);
    assert_eq!(by("de")["native_name"], "Deutsch");
    assert_eq!(
        by("de")["coverage"],
        serde_json::json!({"named": 3, "of": 4})
    );
    assert_eq!(
        by("ko")["coverage"],
        serde_json::json!({"named": 2, "of": 4})
    );
    assert!(
        by("en")["coverage"].is_null(),
        "English is every name as stored"
    );
}
