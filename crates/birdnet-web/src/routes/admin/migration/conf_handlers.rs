//! The routes behind the settings import. See [`super::conf_import`].

use std::path::Path;
use std::time::Instant;

use axum::Form;
use axum::extract::{Multipart, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse as _, Response};
use serde::Deserialize;

use super::conf_import::{
    Bundle, ConfigLines, MAX_FILE_BYTES, STAGE_TTL, Staged, StagedConf, apply, apprise_file_path,
    config_lines_text, next_token, prepare,
};
use super::conf_view as view;
use crate::auth_middleware::RequestUser;
use crate::state::AppState;

/// `POST /admin/migrate/conf/preview`: read what was handed over and show the
/// plan. Changes nothing.
pub(super) async fn preview_handler(
    State(state): State<AppState>,
    mut multipart: Multipart,
    staged: StagedConf,
) -> Result<Html<String>, StatusCode> {
    let mut bundle = Bundle::default();
    let mut pasted = String::new();
    let mut source_dir = String::new();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?
    {
        match field.name() {
            Some("conf_text") => {
                pasted = field.text().await.map_err(|_| StatusCode::BAD_REQUEST)?;
            }
            Some("source_dir") => {
                source_dir = field.text().await.map_err(|_| StatusCode::BAD_REQUEST)?;
            }
            Some("conf_files") => {
                // An untouched file input still posts a part, with no name.
                let Some(name) = field
                    .file_name()
                    .filter(|n| !n.is_empty())
                    .map(str::to_owned)
                else {
                    continue;
                };
                let bytes = field.bytes().await.map_err(|_| StatusCode::BAD_REQUEST)?;
                if bytes.len() as u64 > MAX_FILE_BYTES {
                    return Ok(Html(view::error(&format!(
                        "{name} is {} bytes. A BirdNET-Pi settings file is a few kilobytes, so \
                         this is not one.",
                        bytes.len()
                    ))));
                }
                bundle.add_file(&name, String::from_utf8_lossy(&bytes).into_owned());
            }
            _ => {}
        }
    }
    if !pasted.trim().is_empty() && bundle.conf.is_none() {
        bundle.conf = Some(pasted);
    }
    let source_dir = source_dir.trim().to_owned();

    let result = state
        .run_blocking(move |state| {
            if !source_dir.is_empty() {
                match Bundle::from_dir(Path::new(&source_dir)) {
                    Ok(from_dir) => merge(&mut bundle, from_dir),
                    Err(e) => return Err(e),
                }
            }
            if bundle.is_empty() {
                return Err(
                    "Choose your birdnet.conf (or paste it, or give the folder it is in) first."
                        .to_owned(),
                );
            }
            let current = super::super::settings::handler::load_all_settings(state)
                .map_err(|e| format!("This station's current settings could not be read ({e})."))?;
            let data_dir = crate::data_volume::data_dir_of(state.db_path());
            Ok(prepare(&bundle, &current, &data_dir))
        })
        .await;

    let prepared = match result {
        Ok(p) => p,
        Err(message) => return Ok(Html(view::error(&message))),
    };
    let token = next_token();
    let html = view::preview(&prepared, token);
    *staged
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Staged {
        token,
        at: Instant::now(),
        prepared,
    });
    Ok(Html(html))
}

/// What was uploaded wins over what was read from the folder.
fn merge(bundle: &mut Bundle, from_dir: Bundle) {
    bundle.conf = bundle.conf.take().or(from_dir.conf);
    bundle.apprise = bundle.apprise.take().or(from_dir.apprise);
    bundle.body = bundle.body.take().or(from_dir.body);
    bundle.confirmed = bundle.confirmed.take().or(from_dir.confirmed);
    for (name, text) in from_dir.lists {
        bundle.lists.entry(name).or_insert(text);
    }
}

/// What the Apply button posts.
#[derive(Deserialize)]
pub(super) struct TokenForm {
    token: u64,
}

/// `POST /admin/migrate/conf/apply`: write what the preview showed.
pub(super) async fn apply_handler(
    State(state): State<AppState>,
    request_user: RequestUser,
    staged: StagedConf,
    lines: ConfigLines,
    Form(form): Form<TokenForm>,
) -> Result<Html<String>, StatusCode> {
    let taken = {
        let mut guard = staged
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Peek first: a token that does not match belongs to a preview this
        // admin never saw, and taking it would discard its owner's.
        match guard.as_ref() {
            Some(s) if s.token == form.token => guard.take(),
            _ => None,
        }
    };
    let Some(Staged { at, prepared, .. }) = taken else {
        return Ok(Html(view::error(
            "That preview is no longer waiting: it was applied already, or another was made \
             after it. Read the files again.",
        )));
    };
    if at.elapsed() > STAGE_TTL {
        return Ok(Html(view::error(
            "That preview is more than fifteen minutes old. Read the files again.",
        )));
    }

    let config_text = config_lines_text(&prepared);
    let has_lines = config_text.is_some();
    let wrote_file = prepared.apprise_file.is_some();
    let assignments = prepared.assignments.clone();
    let apprise_file = prepared.apprise_file.clone();

    let outcome = state
        .run_blocking(move |state| {
            if let Some(text) = &apprise_file {
                let path = apprise_file_path(&crate::data_volume::data_dir_of(state.db_path()));
                write_private(&path, text).map_err(|e| {
                    tracing::warn!(error = %e, "could not write the imported Apprise file");
                    "The Apprise file could not be written to this station's data folder, so \
                     nothing was changed."
                        .to_owned()
                })?;
            }
            state
                .with_db(|conn| apply(conn, &assignments))
                .map_err(|e| match e {
                    super::conf_import::ApplyError::Refused(problems) => format!(
                        "Nothing was changed. This station would refuse: {}",
                        problems.join(" ")
                    ),
                    other => {
                        tracing::warn!(error = %other, "settings import failed");
                        "The settings could not be saved, so nothing was changed.".to_owned()
                    }
                })
        })
        .await;

    match outcome {
        Ok(applied) => {
            // Names only: the value of a setting may be a credential.
            let keys = applied.changed.join(",");
            let state2 = state.clone();
            let user = request_user.clone();
            state2
                .run_blocking(move |s| {
                    crate::audit::audit(
                        s,
                        Some(&user),
                        "settings.import_birdnet_pi",
                        None,
                        Some(&format!("keys={keys}")),
                    );
                })
                .await;
            tracing::info!(
                settings = applied.changed.len(),
                "BirdNET-Pi settings imported"
            );
            *lines
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                config_text.map(|t| (form.token, t));
            Ok(Html(view::applied(
                &applied, form.token, has_lines, wrote_file,
            )))
        }
        Err(message) => Ok(Html(view::error(&message))),
    }
}

/// Write `text` to `path` readable by the service user alone.
fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(text.as_bytes())
}

/// What the download link carries.
#[derive(Deserialize)]
pub(super) struct LinesQuery {
    token: u64,
}

/// `GET /admin/migrate/conf/lines`: the lines for `birdnet.conf`, as a file.
pub(super) fn lines_handler(lines: &ConfigLines, Query(q): Query<LinesQuery>) -> Response {
    let found = lines
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .filter(|(token, _)| *token == q.token)
        .map(|(_, text)| text.clone());
    let Some(text) = found else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut response = text.into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"birdnet-from-birdnet-pi.conf\""),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}
