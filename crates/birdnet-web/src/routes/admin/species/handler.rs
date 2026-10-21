//! Species list management handlers.

use axum::Form;
use axum::extract::State;
use axum::response::{Html, IntoResponse as _, Response};
use serde::Deserialize;

use birdnet_db::settings::{SettingsCategory, SettingsError, ensure_settings_table, get, set};

use super::render::{
    SuggestedThreshold, render_filter_test_page, render_intervals_partial, render_species_partial,
    render_thresholds_partial,
};
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
pub async fn species_partial(State(state): State<AppState>) -> Response {
    state.run_blocking(list_partial).await
}

/// The three lists as the partial the Species lists card swaps in, or the
/// inline failure note when they could not be read.
///
/// An unreadable list is not an empty one: three empty lists on screen read as
/// "nothing is excluded", and the operator who then adds a name has no way to
/// know they were looking at a fault.
fn list_partial(state: &AppState) -> Response {
    match try_species_lists(state) {
        Ok(lists) => Html(render_species_partial(
            &lists.exclude,
            &lists.include,
            &lists.whitelist,
            state.names(),
        ))
        .into_response(),
        Err(e) => {
            tracing::warn!(error = %e, "species lists could not be read");
            crate::routes::pages::error_states::failed_partial("the species lists").into_response()
        }
    }
}

/// Render the species filter-test page, which shows every known species and
/// whether the current exclude/include lists would suppress or pass each one.
pub async fn filter_test_page(State(state): State<AppState>) -> Html<String> {
    state
        .run_blocking(move |state| {
            let unreadable = |what: &str, e: &dyn std::fmt::Display| {
                tracing::warn!(error = %e, "{what} could not be read");
                Html(crate::routes::admin::admin_subpage_shell(
                    "Filter test",
                    "species",
                    "Filter test",
                    &crate::routes::pages::error_states::could_not_load(what),
                ))
            };
            let lists = match try_species_lists(state) {
                Ok(lists) => lists,
                Err(e) => return unreadable("the species lists", &e),
            };
            // An empty species table would preview a filter that blocks and
            // passes nothing, as though the station had heard nothing.
            let species =
                match state.with_read_db(|conn| birdnet_db::sqlite::top_species(conn, 10_000)) {
                    Ok(species) => species,
                    Err(e) => return unreadable("the species this station has heard", &e),
                };
            #[allow(clippy::cast_sign_loss)]
            let rows: Vec<(String, String, u64)> = species
                .into_iter()
                .map(|s| (s.sci_name, s.com_name, s.count.max(0) as u64))
                .collect();
            Html(render_filter_test_page(
                &lists.exclude,
                &lists.include,
                &rows,
                state.names(),
            ))
        })
        .await
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
/// A list the database could not read or write is answered with a toast and
/// the page left as it was (`list_changed`), and is not audited: the audit
/// log records what changed.
pub async fn add_exclude(
    State(state): State<AppState>,
    request_user: crate::auth_middleware::RequestUser,
    Form(form): Form<SpeciesNameForm>,
) -> Response {
    state
        .run_blocking(move |state| {
            let result = modify_list(state, "species_exclude", &form.name, &ListAction::Add);
            if result.is_ok() {
                // A filter list decides what the station is allowed to record. A season
                // missing a species is otherwise indistinguishable from a season in which
                // it was not heard.
                crate::audit::audit(
                    state,
                    Some(&request_user),
                    "species.exclude.add",
                    Some(&form.name),
                    None,
                );
            }
            list_changed(state, result, &form.name, "added to the exclusion list")
        })
        .await
}

/// Remove a species from the exclusion list and return the updated partial.
pub async fn remove_exclude(
    State(state): State<AppState>,
    request_user: crate::auth_middleware::RequestUser,
    Form(form): Form<SpeciesNameForm>,
) -> Response {
    state
        .run_blocking(move |state| {
            let result = modify_list(state, "species_exclude", &form.name, &ListAction::Remove);
            if result.is_ok() {
                // A filter list decides what the station is allowed to record. A season
                // missing a species is otherwise indistinguishable from a season in which
                // it was not heard.
                crate::audit::audit(
                    state,
                    Some(&request_user),
                    "species.exclude.remove",
                    Some(&form.name),
                    None,
                );
            }
            list_changed(state, result, &form.name, "removed from the exclusion list")
        })
        .await
}

/// Add a species to the allow-list and return the updated partial.
pub async fn add_include(
    State(state): State<AppState>,
    request_user: crate::auth_middleware::RequestUser,
    Form(form): Form<SpeciesNameForm>,
) -> Response {
    state
        .run_blocking(move |state| {
            let result = modify_list(state, "species_include", &form.name, &ListAction::Add);
            if result.is_ok() {
                // A filter list decides what the station is allowed to record. A season
                // missing a species is otherwise indistinguishable from a season in which
                // it was not heard.
                crate::audit::audit(
                    state,
                    Some(&request_user),
                    "species.include.add",
                    Some(&form.name),
                    None,
                );
            }
            list_changed(state, result, &form.name, "added to the allow-list")
        })
        .await
}

/// Remove a species from the allow-list and return the updated partial.
pub async fn remove_include(
    State(state): State<AppState>,
    request_user: crate::auth_middleware::RequestUser,
    Form(form): Form<SpeciesNameForm>,
) -> Response {
    state
        .run_blocking(move |state| {
            let result = modify_list(state, "species_include", &form.name, &ListAction::Remove);
            if result.is_ok() {
                // A filter list decides what the station is allowed to record. A season
                // missing a species is otherwise indistinguishable from a season in which
                // it was not heard.
                crate::audit::audit(
                    state,
                    Some(&request_user),
                    "species.include.remove",
                    Some(&form.name),
                    None,
                );
            }
            list_changed(state, result, &form.name, "removed from the allow-list")
        })
        .await
}

/// Add a species to the whitelist and return the updated partial.
///
/// The whitelist exempts a species from the occurrence threshold and from
/// nothing else; see `SpeciesFilterConfig::whitelist`.
pub async fn add_whitelist(
    State(state): State<AppState>,
    request_user: crate::auth_middleware::RequestUser,
    Form(form): Form<SpeciesNameForm>,
) -> Response {
    state
        .run_blocking(move |state| {
            let result = modify_list(state, "species_whitelist", &form.name, &ListAction::Add);
            if result.is_ok() {
                crate::audit::audit(
                    state,
                    Some(&request_user),
                    "species.whitelist.add",
                    Some(&form.name),
                    None,
                );
            }
            list_changed(state, result, &form.name, "added to the whitelist")
        })
        .await
}

/// Remove a species from the whitelist and return the updated partial.
pub async fn remove_whitelist(
    State(state): State<AppState>,
    request_user: crate::auth_middleware::RequestUser,
    Form(form): Form<SpeciesNameForm>,
) -> Response {
    state
        .run_blocking(move |state| {
            let result = modify_list(state, "species_whitelist", &form.name, &ListAction::Remove);
            if result.is_ok() {
                crate::audit::audit(
                    state,
                    Some(&request_user),
                    "species.whitelist.remove",
                    Some(&form.name),
                    None,
                );
            }
            list_changed(state, result, &form.name, "removed from the whitelist")
        })
        .await
}

/// Answer a list change: the re-rendered lists when it was made, a toast
/// that leaves the page as it was when it was not.
///
/// `200` with `HX-Reswap: none`, not a `5xx`: htmx drops the body of an error
/// response, so a refusal answered that way changes nothing on the page and
/// says nothing either (see [`not_applied`](crate::routes::pages::toast::not_applied)).
fn list_changed(
    state: &AppState,
    result: Result<(), ListError>,
    name: &str,
    what: &str,
) -> Response {
    match result {
        Ok(()) => list_partial(state),
        Err(e) => {
            tracing::warn!(error = %e, species = %name, "species list not changed");
            crate::routes::pages::toast::not_applied(&crate::routes::pages::toast::Toast::error(
                e.toast(name, what),
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// Threshold handlers
// ---------------------------------------------------------------------------

/// Load the current thresholds and the suggestions derived from reviews.
///
/// One helper because the three threshold handlers all end by re-rendering the
/// same partial, and a suggestion computed in only two of them would appear
/// and vanish as the operator worked.
fn load_thresholds_and_suggestions(
    state: &AppState,
) -> (
    Vec<birdnet_db::sqlite::SpeciesThreshold>,
    Vec<SuggestedThreshold>,
) {
    state.with_read_db(|conn| {
        let thresholds = birdnet_db::sqlite::get_species_thresholds(conn).unwrap_or_default();
        let configured: std::collections::HashMap<&str, f64> = thresholds
            .iter()
            .map(|t| (t.sci_name.as_str(), t.confidence_threshold))
            .collect();
        let suggestions = birdnet_db::sqlite::reviewed_detections_by_species(conn)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(sci_name, com_name, reviews)| {
                birdnet_db::thresholds::suggest_threshold(&reviews).map(|suggestion| {
                    SuggestedThreshold {
                        current: configured.get(sci_name.as_str()).copied(),
                        // Read by the operator, not matched: in the station's language.
                        com_name: state.names().name(&sci_name, &com_name).to_owned(),
                        sci_name,
                        suggestion,
                    }
                })
            })
            .collect();
        (thresholds, suggestions)
    })
}

/// Return the HTMX partial fragment listing all current per-species confidence thresholds.
pub async fn thresholds_partial(State(state): State<AppState>) -> Html<String> {
    state
        .run_blocking(move |state| {
            let (thresholds, suggestions) = load_thresholds_and_suggestions(state);
            Html(render_thresholds_partial(&thresholds, &suggestions))
        })
        .await
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
/// A value that cannot be saved is answered with a toast and the page left as
/// it was (`toast::not_applied`), never a bare `4xx`/`5xx`: htmx discards
/// those, so the percentage slip — `75` in a 0–1 field — changed nothing and
/// said nothing. A write the database refused used to be swallowed with
/// `.ok()`, audited as a successful set, and answered with the list
/// re-rendered as if it had worked.
pub async fn set_threshold(
    State(state): State<AppState>,
    request_user: crate::auth_middleware::RequestUser,
    Form(form): Form<ThresholdForm>,
) -> axum::response::Response {
    state.run_blocking(move |state| {
    use crate::routes::pages::toast::{Toast, not_applied};
    use axum::response::IntoResponse as _;
    let sci_name = form.sci_name.trim().to_string();
    if sci_name.is_empty() {
        return not_applied(&Toast::error("Choose a species for the threshold."));
    }
    let threshold = match birdnet_core::config::locale::parse_decimal(&form.threshold) {
        Ok(t) if (0.0..=1.0).contains(&t) => t,
        Ok(t) if t > 1.0 && t <= 100.0 => {
            return not_applied(&Toast::error(format!(
                "A threshold is between 0 and 1 — for {t}%, enter {:.2}. Nothing was saved.",
                t / 100.0
            )));
        }
        _ => {
            return not_applied(&Toast::error(
                "A threshold is a number between 0 and 1, such as 0.35. Nothing was saved.",
            ));
        }
    };
    if let Err(e) =
        state.with_db(|conn| birdnet_db::sqlite::set_species_threshold(conn, &sci_name, threshold))
    {
        tracing::warn!(error = %e, species = %sci_name, "species threshold not saved");
        return not_applied(&Toast::error(format!(
            "The threshold for {sci_name} was not saved: the database refused the write."
        )));
    }
    // The threshold *is* the metadata here, unlike a settings value: it is a
    // number that changes which detections are kept, and reconstructing "what
    // was it set to in April?" from anything else is impossible.
    crate::audit::audit(
        state,
        Some(&request_user),
        "species.threshold.set",
        Some(&sci_name),
        Some(&format!("threshold={threshold}")),
    );
    let (thresholds, suggestions) = load_thresholds_and_suggestions(state);
    Html(render_thresholds_partial(&thresholds, &suggestions)).into_response()
})
.await
}

/// Form carrying the species whose per-species threshold should be removed.
#[derive(Debug, Deserialize)]
pub struct ThresholdDeleteForm {
    /// Scientific name of the species whose threshold entry is to be deleted.
    pub sci_name: String,
}

/// Delete a per-species confidence threshold and return the updated thresholds partial.
///
/// A refused delete says so in a toast and leaves the list as it was; see
/// [`set_threshold`] for why not a `5xx`.
pub async fn delete_threshold(
    State(state): State<AppState>,
    request_user: crate::auth_middleware::RequestUser,
    Form(form): Form<ThresholdDeleteForm>,
) -> axum::response::Response {
    state.run_blocking(move |state| {
    use crate::routes::pages::toast::{Toast, not_applied};
    use axum::response::IntoResponse as _;
    if let Err(e) =
        state.with_db(|conn| birdnet_db::sqlite::delete_species_threshold(conn, &form.sci_name))
    {
        tracing::warn!(error = %e, species = %form.sci_name, "species threshold not removed");
        return not_applied(&Toast::error(format!(
            "The threshold for {} was not removed: the database refused the write.",
            form.sci_name
        )));
    }
    crate::audit::audit(
        state,
        Some(&request_user),
        "species.threshold.delete",
        Some(&form.sci_name),
        None,
    );
    let (thresholds, suggestions) = load_thresholds_and_suggestions(state);
    Html(render_thresholds_partial(&thresholds, &suggestions)).into_response()
})
.await
}

// ---------------------------------------------------------------------------
// Minimum-interval handlers
// ---------------------------------------------------------------------------

/// The longest per-species interval the form accepts: one day. Anything longer
/// is a unit slip (milliseconds, or minutes typed into a seconds field), and a
/// species held for more than a day is not "recorded again" in any useful sense.
pub const MAX_SPECIES_INTERVAL_SECS: i64 = 86_400;

/// Re-read the table for a re-render. A failed read is an error, not an empty
/// list: an empty table renders as "No species has an interval of its own",
/// which would be a claim about the station that a database fault must not make.
fn load_intervals(
    state: &AppState,
) -> Result<Vec<birdnet_db::sqlite::SpeciesInterval>, birdnet_db::sqlite::DbError> {
    state.with_read_db(birdnet_db::sqlite::get_species_intervals)
}

fn intervals_response(state: &AppState) -> Response {
    match load_intervals(state) {
        Ok(rows) => Html(render_intervals_partial(&rows)).into_response(),
        Err(e) => {
            tracing::warn!(error = %e, "per-species intervals not readable");
            crate::routes::pages::error_states::failed_partial("the per-species intervals")
                .into_response()
        }
    }
}

/// Return the HTMX partial listing every per-species minimum interval.
pub async fn intervals_partial(State(state): State<AppState>) -> Response {
    state.run_blocking(|state| intervals_response(state)).await
}

/// Per-species interval submission.
#[derive(Debug, Deserialize)]
pub struct IntervalForm {
    /// Scientific name of the species the interval is for.
    pub sci_name: String,
    /// Seconds, as typed. A string so a slip is answered with a toast rather
    /// than the bare `422` serde would give a non-number (htmx discards it).
    pub seconds: String,
}

/// Parse the interval field: whole seconds, 1 to [`MAX_SPECIES_INTERVAL_SECS`].
fn parse_interval(raw: &str) -> Result<i64, String> {
    let raw = raw.trim();
    match raw.parse::<i64>() {
        Ok(n) if (1..=MAX_SPECIES_INTERVAL_SECS).contains(&n) => Ok(n),
        Ok(0) => Err(
            "An interval of 0 means no interval; remove the species' row instead. Nothing was saved."
                .to_owned(),
        ),
        Ok(n) if n > MAX_SPECIES_INTERVAL_SECS => Err(format!(
            "An interval is at most {MAX_SPECIES_INTERVAL_SECS} seconds (one day); {n} looks like a unit slip. Nothing was saved."
        )),
        _ => Err(
            "An interval is a whole number of seconds, such as 60. Nothing was saved.".to_owned(),
        ),
    }
}

/// Set a per-species minimum interval and return the updated partial.
///
/// Answers a refusal with a toast and the page unchanged, never a `4xx`: see
/// [`set_threshold`].
pub async fn set_interval(
    State(state): State<AppState>,
    request_user: crate::auth_middleware::RequestUser,
    Form(form): Form<IntervalForm>,
) -> Response {
    state
        .run_blocking(move |state| {
            use crate::routes::pages::toast::{Toast, not_applied};
            let sci_name = form.sci_name.trim().to_owned();
            if sci_name.is_empty() {
                return not_applied(&Toast::error("Choose a species for the interval."));
            }
            let secs = match parse_interval(&form.seconds) {
                Ok(n) => n,
                Err(msg) => return not_applied(&Toast::error(msg)),
            };
            if let Err(e) = state
                .with_db(|conn| birdnet_db::sqlite::set_species_interval(conn, &sci_name, secs))
            {
                tracing::warn!(error = %e, species = %sci_name, "species interval not saved");
                return not_applied(&Toast::error(format!(
                    "The interval for {sci_name} was not saved: the database refused the write."
                )));
            }
            crate::audit::audit(
                state,
                Some(&request_user),
                "species.interval.set",
                Some(&sci_name),
                Some(&format!("min_interval_secs={secs}")),
            );
            intervals_response(state)
        })
        .await
}

/// Form carrying the species whose interval should be removed.
#[derive(Debug, Deserialize)]
pub struct IntervalDeleteForm {
    /// Scientific name of the species whose interval is to be removed.
    pub sci_name: String,
}

/// Remove a per-species minimum interval and return the updated partial.
pub async fn delete_interval(
    State(state): State<AppState>,
    request_user: crate::auth_middleware::RequestUser,
    Form(form): Form<IntervalDeleteForm>,
) -> Response {
    state
        .run_blocking(move |state| {
            use crate::routes::pages::toast::{Toast, not_applied};
            if let Err(e) = state.with_db(|conn| {
                birdnet_db::sqlite::delete_species_interval(conn, &form.sci_name)
            }) {
                tracing::warn!(error = %e, species = %form.sci_name, "species interval not removed");
                return not_applied(&Toast::error(format!(
                    "The interval for {} was not removed: the database refused the write.",
                    form.sci_name
                )));
            }
            crate::audit::audit(
                state,
                Some(&request_user),
                "species.interval.delete",
                Some(&form.sci_name),
                None,
            );
            intervals_response(state)
        })
        .await
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

enum ListAction {
    Add,
    Remove,
}

/// Why a list could not be changed.
///
/// Two kinds because they promise different things: a list that could not be
/// *read* was never written to, which is certain, while a write the database
/// refused is only known to have not been applied.
#[derive(Debug)]
pub(crate) enum ListError {
    /// The stored list could not be read, so nothing was written.
    Read(SettingsError),
    /// The new list could not be written.
    Write(SettingsError),
}

impl std::fmt::Display for ListError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read(e) => write!(f, "the list could not be read: {e}"),
            Self::Write(e) => write!(f, "the list could not be written: {e}"),
        }
    }
}

impl ListError {
    /// What to tell the operator: the species, what was meant to happen to it,
    /// and only what is certainly true about why. The database's own words stay
    /// in the log.
    pub(crate) fn toast(&self, name: &str, what: &str) -> String {
        match self {
            Self::Read(_) => format!(
                "{name} was not {what}: the list could not be read from the database, so it was \
                 left as it was."
            ),
            Self::Write(_) => {
                format!("{name} was not {what}: the database refused the change.")
            }
        }
    }
}

/// Read one comma-separated list setting.
///
/// A key that has never been written is an empty list, because that is what a
/// new station has. Any other failure is an error: an unreadable list is not an
/// empty one, and treating it as one lets the next write replace the whole of
/// it with whatever was just added.
fn read_list(conn: &rusqlite::Connection, key: &str) -> Result<Vec<String>, SettingsError> {
    match get(conn, key) {
        Ok(value) => Ok(parse_list(Some(&value))),
        Err(SettingsError::NotFound(_)) => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

/// The operator's species include, exclude and whitelist lists, or the first
/// error the database returned reading them.
///
/// What the pages use: they must be able to say "could not read" instead of
/// drawing an empty list. The detection daemon uses
/// [`configured_species_lists`], which cannot say anything.
pub(crate) fn try_species_lists(
    state: &AppState,
) -> Result<birdnet_core::inference::species_filter::SpeciesLists, SettingsError> {
    state.with_read_db(|conn| {
        Ok(birdnet_core::inference::species_filter::SpeciesLists {
            include: read_list(conn, "species_include")?,
            exclude: read_list(conn, "species_exclude")?,
            whitelist: read_list(conn, "species_whitelist")?,
        })
    })
}

/// The operator's species include, exclude and whitelist lists as stored by
/// this page and the Settings form.
///
/// Public because the detection daemon has to apply exactly these lists, and
/// two parsers for one stored value is how the two surfaces drift apart. The
/// binary passes this to the daemon as a
/// [`SpeciesListsProvider`](birdnet_core::inference::species_filter::SpeciesListsProvider)
/// so a change here takes effect on the next processed file.
///
/// A list that cannot be read is empty **for the daemon**, one list at a time,
/// and is logged at `warn`: the provider's signature has no way to say "I do
/// not know", and detection continuing is the lesser harm. The pages do not
/// share that fallback; they use `try_species_lists`.
#[must_use]
pub fn configured_species_lists(
    state: &AppState,
) -> birdnet_core::inference::species_filter::SpeciesLists {
    state.with_read_db(|conn| {
        let read = |key: &str| {
            read_list(conn, key).unwrap_or_else(|e| {
                tracing::warn!(error = %e, key, "species list could not be read; applying none");
                Vec::new()
            })
        };
        birdnet_core::inference::species_filter::SpeciesLists {
            include: read("species_include"),
            exclude: read("species_exclude"),
            whitelist: read("species_whitelist"),
        }
    })
}

/// Render the species-list management body (no document shell).
///
/// Shared with the Station **Capture** tab
/// (`crate::routes::pages::homes::station_tabs`), which renders the same
/// include/exclude UI in the main shell.
pub(crate) fn species_body(state: &AppState) -> String {
    match try_species_lists(state) {
        Ok(lists) => super::render::species_lists_body(
            &lists.exclude,
            &lists.include,
            &lists.whitelist,
            state.names(),
        ),
        Err(e) => {
            tracing::warn!(error = %e, "species lists could not be read");
            crate::routes::pages::error_states::inline("the species lists")
        }
    }
}

fn parse_list(val: Option<&str>) -> Vec<String> {
    val.unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Add one species to the operator's exclusion list.
///
/// Public so the suspect-species report can reuse the list the operator
/// already has, rather than inventing a second exclusion mechanism that would
/// be a second place to look when a bird stops appearing.
///
/// # Errors
///
/// A [`ListError`] when the list could not be read or the new one written; the
/// caller must not tell the operator the species is excluded.
pub(crate) fn add_to_exclude_list(state: &AppState, sci_name: &str) -> Result<(), ListError> {
    modify_list(state, "species_exclude", sci_name, &ListAction::Add)
}

fn modify_list(
    state: &AppState,
    key: &'static str,
    name: &str,
    action: &ListAction,
) -> Result<(), ListError> {
    let name = name.trim();
    // A name typed in the station's language is stored as the scientific name.
    // The filter resolves entries against the classifier's label set, which has
    // only the classifier's common names and the scientific names, so *Amsel*
    // would be accepted, shown as a pill, and match nothing for ever. The
    // scientific name is the one spelling that means the same in every language,
    // so the list also survives a change of language.
    let name = match action {
        ListAction::Add => state.names().scientific_name_for(name).unwrap_or(name),
        ListAction::Remove => name,
    }
    .to_string();
    if name.is_empty() {
        return Ok(());
    }

    state.with_db(|conn| {
        ensure_settings_table(conn).map_err(ListError::Write)?;
        let mut list = read_list(conn, key).map_err(ListError::Read)?;
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
        set(conn, key, &joined, SettingsCategory::Species).map_err(ListError::Write)
    })
}
