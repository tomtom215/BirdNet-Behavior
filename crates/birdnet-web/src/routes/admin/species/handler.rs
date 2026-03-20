//! Species list management handlers.

use axum::Form;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Html;
use serde::Deserialize;

use birdnet_db::settings::{SettingsCategory, ensure_settings_table, get, set};

use super::render::{
    render_filter_test_page, render_species_page, render_species_partial, render_thresholds_partial,
};
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Page handlers
// ---------------------------------------------------------------------------

pub async fn species_page(State(state): State<AppState>) -> Html<String> {
    let (exclude, include) = load_lists(&state);
    Html(render_species_page(&exclude, &include))
}

pub async fn species_partial(State(state): State<AppState>) -> Html<String> {
    let (exclude, include) = load_lists(&state);
    Html(render_species_partial(&exclude, &include))
}

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

#[derive(Debug, Deserialize)]
pub struct SpeciesNameForm {
    pub name: String,
}

pub async fn add_exclude(
    State(state): State<AppState>,
    Form(form): Form<SpeciesNameForm>,
) -> Result<Html<String>, StatusCode> {
    modify_list(&state, "species_exclude", &form.name, ListAction::Add)?;
    let (exclude, include) = load_lists(&state);
    Ok(Html(render_species_partial(&exclude, &include)))
}

pub async fn remove_exclude(
    State(state): State<AppState>,
    Form(form): Form<SpeciesNameForm>,
) -> Result<Html<String>, StatusCode> {
    modify_list(&state, "species_exclude", &form.name, ListAction::Remove)?;
    let (exclude, include) = load_lists(&state);
    Ok(Html(render_species_partial(&exclude, &include)))
}

pub async fn add_include(
    State(state): State<AppState>,
    Form(form): Form<SpeciesNameForm>,
) -> Result<Html<String>, StatusCode> {
    modify_list(&state, "species_include", &form.name, ListAction::Add)?;
    let (exclude, include) = load_lists(&state);
    Ok(Html(render_species_partial(&exclude, &include)))
}

pub async fn remove_include(
    State(state): State<AppState>,
    Form(form): Form<SpeciesNameForm>,
) -> Result<Html<String>, StatusCode> {
    modify_list(&state, "species_include", &form.name, ListAction::Remove)?;
    let (exclude, include) = load_lists(&state);
    Ok(Html(render_species_partial(&exclude, &include)))
}

// ---------------------------------------------------------------------------
// Threshold handlers
// ---------------------------------------------------------------------------

pub async fn thresholds_partial(State(state): State<AppState>) -> Html<String> {
    let thresholds =
        state.with_db(|conn| birdnet_db::sqlite::get_species_thresholds(conn).unwrap_or_default());
    Html(render_thresholds_partial(&thresholds))
}

#[derive(Debug, Deserialize)]
pub struct ThresholdForm {
    pub sci_name: String,
    pub threshold: f64,
}

pub async fn set_threshold(
    State(state): State<AppState>,
    Form(form): Form<ThresholdForm>,
) -> Result<Html<String>, StatusCode> {
    let sci_name = form.sci_name.trim().to_string();
    if sci_name.is_empty() || !(0.0..=1.0).contains(&form.threshold) {
        return Err(StatusCode::BAD_REQUEST);
    }
    state.with_db(|conn| {
        birdnet_db::sqlite::set_species_threshold(conn, &sci_name, form.threshold).ok();
    });
    let thresholds =
        state.with_db(|conn| birdnet_db::sqlite::get_species_thresholds(conn).unwrap_or_default());
    Ok(Html(render_thresholds_partial(&thresholds)))
}

#[derive(Debug, Deserialize)]
pub struct ThresholdDeleteForm {
    pub sci_name: String,
}

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
    state.with_db(|conn| {
        ensure_settings_table(conn).ok();
        let excl = parse_list(get(conn, "species_exclude").ok().as_deref());
        let incl = parse_list(get(conn, "species_include").ok().as_deref());
        (excl, incl)
    })
}

fn parse_list(val: Option<&str>) -> Vec<String> {
    val.unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn modify_list(
    state: &AppState,
    key: &'static str,
    name: &str,
    action: ListAction,
) -> Result<(), StatusCode> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Ok(());
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

    Ok(())
}
