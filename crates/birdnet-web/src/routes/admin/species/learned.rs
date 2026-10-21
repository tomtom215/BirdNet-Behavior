//! The "Learned thresholds" card on the Capture tab (`P-11`): what the station
//! has learned about each species, and a way to forget it.
//!
//! The JSON behind it is `GET /api/v2/dynamic-thresholds`; the reset is the
//! same [`crate::routes::dynamic_thresholds::reset`] the token-gated
//! `POST /api/v2/dynamic-thresholds/reset` calls.

use std::fmt::Write as _;

use axum::Form;
use axum::extract::State;
use axum::response::{Html, IntoResponse as _, Response};
use serde::Deserialize;

use birdnet_core::detection::dynamic_threshold::Reset;

use crate::routes::dynamic_thresholds::{Learned, Snapshot, reset, snapshot};
use crate::routes::pages::escape_html;
use crate::state::AppState;

/// A span of milliseconds as the largest whole unit: `5 min`, `3 h`, `2 d`.
fn span(ms: i64) -> String {
    let secs = ms.max(0) / 1000;
    if secs < 90 {
        format!("{secs} s")
    } else if secs < 90 * 60 {
        format!("{} min", (secs + 30) / 60)
    } else if secs < 36 * 3600 {
        format!("{} h", (secs + 1800) / 3600)
    } else {
        format!("{} d", (secs + 43_200) / 86_400)
    }
}

fn pct(fraction: f64) -> String {
    format!("{:.0}%", fraction * 100.0)
}

/// The card, from a snapshot taken at `now_ms`.
pub fn render_learned_partial(
    snap: &Snapshot,
    now_ms: i64,
    name_of: &dyn Fn(&str) -> String,
) -> String {
    let mut out = String::with_capacity(2048);
    out.push_str(
        r#"<div class="card">
  <div class="section-title">Learned thresholds</div>
  <p class="hint">When dynamic thresholds are on, a species the station has heard confidently, and that passed every other check, is held to a lower bar for a while: each level multiplies its threshold down, never below the floor, and the level lapses if the species is not confirmed again. This is what the station has learned. <b>Reset</b> forgets a species: its next confident detection starts again at level 1. The detections themselves are not touched.</p>"#,
    );
    match snap.config {
        None => out.push_str(
            r#"<p class="empty-note mb">This process is not running the detection service, so these are the levels it last stored, not live ones. A reset clears them for the next start.</p>"#,
        ),
        Some(c) if !c.enabled => out.push_str(
            r#"<p class="empty-note mb">Dynamic thresholds are <b>off</b> (<code>BIRDNET_DYNAMIC_THRESHOLD</code>), so nothing is being learned and every species is held to its own threshold. Rows below, if any, are left from an earlier run and have no effect.</p>"#,
        ),
        Some(c) => {
            let _ = write!(
                out,
                r#"<p class="hint">Confirmed by a detection of {trigger} or more; floor {floor}; a level lasts {hours} h without another confirmation.</p>"#,
                trigger = pct(f64::from(c.trigger)),
                floor = pct(f64::from(c.min)),
                hours = c.valid_hours,
            );
        }
    }
    if snap.rows.is_empty() {
        out.push_str(r#"<p class="empty-note mb">Nothing has been learned yet.</p>"#);
    } else {
        for l in &snap.rows {
            render_row(&mut out, l, now_ms, name_of);
        }
        out.push_str(
            r##"<form hx-post="/admin/species/learned/reset-all" hx-target="#learned-section" hx-swap="innerHTML" hx-confirm="Forget everything the station has learned? Every species starts again from its own threshold." class="add-row">
    <button type="submit" class="btn btn-danger">Reset all</button>
  </form>"##,
        );
    }
    out.push_str("</div>");
    out
}

fn render_row(out: &mut String, l: &Learned, now_ms: i64, name_of: &dyn Fn(&str) -> String) {
    let sci = escape_html(&l.row.sci_name);
    let shown = escape_html(&name_of(&l.row.sci_name));
    let name = if shown == sci {
        sci.clone()
    } else {
        format!(r#"{shown} <span class="muted-sm">{sci}</span>"#)
    };
    let threshold = if !l.active {
        format!(
            "{} (lapsed {} ago)",
            pct(l.base_threshold),
            span(now_ms - l.row.expires_at_ms)
        )
    } else if let Some(t) = l.effective_threshold {
        format!(
            "{} now, from {} &times;{:.2}; lapses in {}",
            pct(t),
            pct(l.base_threshold),
            l.multiplier,
            span(l.row.expires_at_ms - now_ms)
        )
    } else {
        // No floor is known here, so no number is claimed.
        format!(
            "&times;{:.2} of {}, floor unknown; lapses in {}",
            l.multiplier,
            pct(l.base_threshold),
            span(l.row.expires_at_ms - now_ms)
        )
    };
    let _ = write!(
        out,
        r##"<div class="st-list-row"><div class="lr-main">
  <div>{name}</div>
  <div class="lr-sub">level {level} of {max} &middot; {threshold} &middot; confirmed {ago} ago</div>
  <form hx-post="/admin/species/learned/reset" hx-target="#learned-section" hx-swap="innerHTML" class="inline-form">
    <input type="hidden" name="sci_name" value="{sci}">
    <button type="submit" class="btn btn-danger del-btn">Reset {sci}</button>
  </form>
</div></div>"##,
        level = l.row.level,
        max = birdnet_core::detection::dynamic_threshold::MAX_LEVEL,
        ago = span(now_ms - l.row.last_confirmed_ms),
    );
}

fn epoch_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// Draw the card, or say the read failed. An unreadable table is not "nothing
/// learned yet".
fn card(state: &AppState) -> Response {
    let now_ms = epoch_ms();
    match snapshot(state, now_ms) {
        Ok(snap) => {
            let names = state.names();
            let html =
                render_learned_partial(&snap, now_ms, &|sci| names.name(sci, sci).to_owned());
            Html(html).into_response()
        }
        Err(e) => {
            tracing::warn!(error = %e, "learned thresholds not readable");
            crate::routes::pages::error_states::failed_partial("the learned thresholds")
                .into_response()
        }
    }
}

/// `GET /admin/species/learned`.
pub async fn learned_partial(State(state): State<AppState>) -> Response {
    state.run_blocking(card).await
}

/// Form for resetting one species.
#[derive(Debug, Deserialize)]
pub struct ResetForm {
    /// Scientific name.
    pub sci_name: String,
}

/// `POST /admin/species/learned/reset`.
pub async fn reset_one(
    State(state): State<AppState>,
    request_user: crate::auth_middleware::RequestUser,
    Form(form): Form<ResetForm>,
) -> Response {
    state
        .run_blocking(move |state| {
            let sci = form.sci_name.trim().to_owned();
            apply(state, &request_user, &Reset::Species(sci))
        })
        .await
}

/// `POST /admin/species/learned/reset-all`.
pub async fn reset_everything(
    State(state): State<AppState>,
    request_user: crate::auth_middleware::RequestUser,
) -> Response {
    state
        .run_blocking(move |state| apply(state, &request_user, &Reset::All))
        .await
}

fn apply(state: &AppState, user: &crate::auth_middleware::RequestUser, which: &Reset) -> Response {
    use crate::routes::pages::toast::{Toast, not_applied};
    if matches!(&which, Reset::Species(s) if s.is_empty()) {
        return not_applied(&Toast::error("Choose a species to reset."));
    }
    match reset(state, &which) {
        Ok(outcome) => {
            let (action, target, meta) = match &which {
                Reset::Species(s) => (
                    "species.learned_threshold.reset",
                    Some(s.as_str()),
                    format!("rows={}", outcome.rows_deleted),
                ),
                Reset::All => (
                    "species.learned_threshold.reset_all",
                    None,
                    format!("rows={}", outcome.rows_deleted),
                ),
            };
            crate::audit::audit(state, Some(user), action, target, Some(&meta));
            card(state)
        }
        Err(e) => {
            tracing::warn!(error = %e, "learned thresholds not reset");
            not_applied(&Toast::error(
                "The learned thresholds were not reset: the database refused the write. Nothing was changed.",
            ))
        }
    }
}
