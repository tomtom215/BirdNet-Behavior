//! Species list management handlers.

use axum::Form;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Html;
use serde::Deserialize;

use birdnet_db::settings::{SettingsCategory, ensure_settings_table, get, set};

use super::render::{render_filter_test_page, render_species_partial, render_thresholds_partial};
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Page handlers
// ---------------------------------------------------------------------------

/// Redirect the standalone `/admin/species` GET into the Station **Capture** tab.
///
/// The page folded there; the add/remove/threshold endpoints and the Filter-test
/// sub-page keep their `/admin/species...` paths.
pub async fn species_page() -> axum::response::Redirect {
    axum::response::Redirect::permanent("/station/capture#species")
}

/// Return the HTMX partial fragment containing the current species lists.
pub async fn species_partial(State(state): State<AppState>) -> Html<String> {
    let (exclude, include) = load_lists(&state);
    Html(render_species_partial(&exclude, &include))
}

/// Render the species filter-test page, which shows every known species and
/// whether the current exclude/include lists would suppress or pass each one.
pub async fn filter_test_page(State(state): State<AppState>) -> Html<String> {
    let (exclude, include) = load_lists(&state);
    let species =
        state.with_db(|conn| birdnet_db::sqlite::top_species(conn, 10_000).unwrap_or_default());
    #[allow(clippy::cast_sign_loss)]
    let rows: Vec<(String, String, u64)> = species
        .into_iter()
        .map(|s| (s.sci_name, s.com_name, s.count.max(0) as u64))
        .collect();
    Html(render_filter_test_page(&exclude, &include, &rows))
}

// ---------------------------------------------------------------------------
// Mutation handlers
// ---------------------------------------------------------------------------

/// Form carrying a single species name for list-mutation endpoints.
#[derive(Debug, Deserialize)]
pub struct SpeciesNameForm {
    /// Common name of the species to add or remove.
    pub name: String,
}

/// Add a species to the exclusion list and return the updated partial.
///
/// # Errors
///
/// This function currently always returns `Ok`.
pub async fn add_exclude(
    State(state): State<AppState>,
    Form(form): Form<SpeciesNameForm>,
) -> Result<Html<String>, StatusCode> {
    modify_list(&state, "species_exclude", &form.name, &ListAction::Add);
    let (exclude, include) = load_lists(&state);
    Ok(Html(render_species_partial(&exclude, &include)))
}

/// Remove a species from the exclusion list and return the updated partial.
///
/// # Errors
///
/// This function currently always returns `Ok`.
pub async fn remove_exclude(
    State(state): State<AppState>,
    Form(form): Form<SpeciesNameForm>,
) -> Result<Html<String>, StatusCode> {
    modify_list(&state, "species_exclude", &form.name, &ListAction::Remove);
    let (exclude, include) = load_lists(&state);
    Ok(Html(render_species_partial(&exclude, &include)))
}

/// Add a species to the allow-list and return the updated partial.
///
/// # Errors
///
/// This function currently always returns `Ok`.
pub async fn add_include(
    State(state): State<AppState>,
    Form(form): Form<SpeciesNameForm>,
) -> Result<Html<String>, StatusCode> {
    modify_list(&state, "species_include", &form.name, &ListAction::Add);
    let (exclude, include) = load_lists(&state);
    Ok(Html(render_species_partial(&exclude, &include)))
}

/// Remove a species from the allow-list and return the updated partial.
///
/// # Errors
///
/// This function currently always returns `Ok`.
pub async fn remove_include(
    State(state): State<AppState>,
    Form(form): Form<SpeciesNameForm>,
) -> Result<Html<String>, StatusCode> {
    modify_list(&state, "species_include", &form.name, &ListAction::Remove);
    let (exclude, include) = load_lists(&state);
    Ok(Html(render_species_partial(&exclude, &include)))
}

// ---------------------------------------------------------------------------
// Threshold handlers
// ---------------------------------------------------------------------------

/// Return the HTMX partial fragment listing all current per-species confidence thresholds.
pub async fn thresholds_partial(State(state): State<AppState>) -> Html<String> {
    let thresholds =
        state.with_db(|conn| birdnet_db::sqlite::get_species_thresholds(conn).unwrap_or_default());
    Html(render_thresholds_partial(&thresholds))
}

/// Per-species threshold submission.
///
/// `threshold` is received as a string so the handler can accept both
/// `.` and `,` decimal separators (EU operators) via
/// [`birdnet_core::config::locale::parse_decimal`]. Receiving it as
/// `f64` here would let serde's default parser reject any comma value
/// with a 422.
#[derive(Debug, Deserialize)]
pub struct ThresholdForm {
    /// Scientific name of the species whose threshold is being set.
    pub sci_name: String,
    /// Confidence threshold as a decimal string (e.g. `"0.75"`); accepts either `.` or `,`
    /// as the decimal separator.
    pub threshold: String,
}

/// Set a per-species confidence threshold and return the updated thresholds partial.
///
/// # Errors
///
/// Returns `StatusCode::BAD_REQUEST` if the species name is empty or the threshold is out of range.
pub async fn set_threshold(
    State(state): State<AppState>,
    Form(form): Form<ThresholdForm>,
) -> Result<Html<String>, StatusCode> {
    let sci_name = form.sci_name.trim().to_string();
    let Ok(threshold) = birdnet_core::config::locale::parse_decimal(&form.threshold) else {
        return Err(StatusCode::BAD_REQUEST);
    };
    if sci_name.is_empty() || !(0.0..=1.0).contains(&threshold) {
        return Err(StatusCode::BAD_REQUEST);
    }
    state.with_db(|conn| {
        birdnet_db::sqlite::set_species_threshold(conn, &sci_name, threshold).ok();
    });
    let thresholds =
        state.with_db(|conn| birdnet_db::sqlite::get_species_thresholds(conn).unwrap_or_default());
    Ok(Html(render_thresholds_partial(&thresholds)))
}

/// Form carrying the species whose per-species threshold should be removed.
#[derive(Debug, Deserialize)]
pub struct ThresholdDeleteForm {
    /// Scientific name of the species whose threshold entry is to be deleted.
    pub sci_name: String,
}

/// Delete a per-species confidence threshold and return the updated thresholds partial.
///
/// # Errors
///
/// Returns `StatusCode::INTERNAL_SERVER_ERROR` if the database operation fails.
pub async fn delete_threshold(
    State(state): State<AppState>,
    Form(form): Form<ThresholdDeleteForm>,
) -> Result<Html<String>, StatusCode> {
    state.with_db(|conn| {
        birdnet_db::sqlite::delete_species_threshold(conn, &form.sci_name).ok();
    });
    let thresholds =
        state.with_db(|conn| birdnet_db::sqlite::get_species_thresholds(conn).unwrap_or_default());
    Ok(Html(render_thresholds_partial(&thresholds)))
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

enum ListAction {
    Add,
    Remove,
}

fn load_lists(state: &AppState) -> (Vec<String>, Vec<String>) {
    let lists = configured_species_lists(state);
    (lists.exclude, lists.include)
}

/// The operator's species include/exclude lists as stored by this page.
///
/// Public because the detection daemon has to apply exactly these lists, and
/// two parsers for one stored value is how the two surfaces drift apart. The
/// binary passes this to the daemon as a
/// [`SpeciesListsProvider`](birdnet_core::inference::species_filter::SpeciesListsProvider)
/// so a change here takes effect on the next processed file.
#[must_use]
pub fn configured_species_lists(
    state: &AppState,
) -> birdnet_core::inference::species_filter::SpeciesLists {
    state.with_db(|conn| {
        ensure_settings_table(conn).ok();
        birdnet_core::inference::species_filter::SpeciesLists {
            include: parse_list(get(conn, "species_include").ok().as_deref()),
            exclude: parse_list(get(conn, "species_exclude").ok().as_deref()),
        }
    })
}

/// Render the species-list management body (no document shell).
///
/// Shared with the Station **Capture** tab
/// (`crate::routes::pages::homes::station_tabs`), which renders the same
/// include/exclude UI in the main shell.
pub(crate) fn species_body(state: &AppState) -> String {
    let (exclude, include) = load_lists(state);
    super::render::species_lists_body(&exclude, &include)
}

fn parse_list(val: Option<&str>) -> Vec<String> {
    val.unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn modify_list(state: &AppState, key: &'static str, name: &str, action: &ListAction) {
    let name = name.trim().to_string();
    if name.is_empty() {
        return;
    }

    state.with_db(|conn| {
        ensure_settings_table(conn).ok();
        let mut list = parse_list(get(conn, key).ok().as_deref());
        match action {
            ListAction::Add => {
                if !list.iter().any(|s| s.eq_ignore_ascii_case(&name)) {
                    list.push(name);
                }
            }
            ListAction::Remove => {
                list.retain(|s| !s.eq_ignore_ascii_case(&name));
            }
        }
        let joined = list.join(", ");
        set(conn, key, &joined, SettingsCategory::Species).ok();
    });
}
