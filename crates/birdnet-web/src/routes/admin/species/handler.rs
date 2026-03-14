//! Species list management handlers.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Html;
use axum::Form;
use serde::Deserialize;

use birdnet_db::settings::{SettingsCategory, ensure_settings_table, get, set};

use super::render::{render_species_page, render_species_partial};
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
