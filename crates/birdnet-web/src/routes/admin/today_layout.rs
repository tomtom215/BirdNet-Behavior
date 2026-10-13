//! The editor for the Today page's layout (`G-22`).
//!
//! A list of the page's blocks in their current order, each with Move up,
//! Move down and Show/Hide buttons — no drag and drop. Dragging is the
//! upstream design; buttons are what a keyboard, a switch or a screen reader
//! can operate, and what a touch screen in a visitor centre gets right first
//! time. Each press stores the whole layout and answers with the redrawn
//! editor, so the list on screen is always what is stored.
//!
//! Lives on the settings page but outside its form: the form posts every
//! field on Save, and a button inside it would submit it.

use std::fmt::Write as _;

use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Response};
use axum::{Router, routing::post};

use crate::routes::pages::escape_html;
use crate::routes::pages::toast::{self, Toast};
use crate::routes::pages::today_layout::{Block, Group, Layout, SETTING_KEY};
use crate::state::AppState;

/// Routes, mounted behind the admin auth middleware.
pub fn router() -> Router<AppState> {
    Router::new().route("/admin/today-layout/{block}/{action}", post(change))
}

/// The settings-page section: `id="set-today-layout"`, swapped whole.
#[must_use]
pub fn render_section(layout: &Layout) -> String {
    let slots = layout.slots();
    let mut rows = String::new();
    for (i, slot) in slots.iter().enumerate() {
        let block = slot.block;
        let group = block.group();
        let first = !slots[..i].iter().any(|s| s.block.group() == group);
        let last = !slots[i + 1..].iter().any(|s| s.block.group() == group);
        let id = block.id();
        let label = escape_html(block.label());
        let where_ = match group {
            Group::Top => "Top of the page",
            Group::Log => "Main column",
            Group::Rail => "Side column",
        };
        let state = if slot.shown { "Shown" } else { "Hidden" };
        let (toggle_action, toggle_text) = if slot.shown {
            ("hide", "Hide")
        } else {
            ("show", "Show")
        };
        let button = |action: &str, text: &str, disabled: bool| {
            let disabled = if disabled { " disabled" } else { "" };
            format!(
                r#"<button type="button" class="bnb-btn ghost sm" hx-post="/admin/today-layout/{id}/{action}" hx-swap="none" aria-label="{text}: {label}"{disabled}>{text}</button>"#
            )
        };
        // The log is alone in its group: buttons that could never do anything
        // are left out rather than shown disabled.
        let (up, down) = if group == Group::Log {
            (String::new(), String::new())
        } else {
            (
                button("up", "Move up", first),
                button("down", "Move down", last),
            )
        };
        let toggle = button(toggle_action, toggle_text, false);
        let hidden_class = if slot.shown { "" } else { " tl-row--hidden" };
        let _ = write!(
            rows,
            r#"<li class="tl-row{hidden_class}"><span class="tl-name">{label}<span class="bnb-meta tl-where">{where_} · {state}</span></span><span class="tl-actions">{up}{down}{toggle}</span></li>"#
        );
    }
    format!(
        r#"<section class="card" id="set-today-layout" aria-labelledby="set-today-layout-h">
  <h2 class="section-title" id="set-today-layout-h">Today Page Layout</h2>
  <p class="hint">Which parts of the Today page appear, and in what order — for a station on a wall or a kitchen tablet. Blocks move within their own part of the page. A new station shows everything until its first detection, so its setup checklist stays visible. <a href="/">View the Today page →</a></p>
  <ol class="tl-list">{rows}</ol>
</section>"#
    )
}

/// Apply one button press and answer with the redrawn editor.
async fn change(
    State(state): State<AppState>,
    request_user: crate::auth_middleware::RequestUser,
    Path((block, action)): Path<(String, String)>,
) -> Response {
    state
        .run_blocking(move |state| {
            let Some(block) = Block::ALL.into_iter().find(|b| b.id() == block) else {
                return toast::not_applied(&Toast::warn("That part of the page does not exist."));
            };
            let stored = state.with_db(|conn| birdnet_db::settings::get_or(conn, SETTING_KEY, ""));
            let mut layout = match stored {
                Ok(v) => Layout::parse(&v),
                Err(e) => {
                    tracing::error!(error = %e, "reading the Today layout failed");
                    return toast::not_applied(&Toast::error(
                        "Could not read the current layout, so nothing was changed.",
                    ));
                }
            };
            match action.as_str() {
                "up" => layout.nudge(block, true),
                "down" => layout.nudge(block, false),
                "show" => layout.set_shown(block, true),
                "hide" => layout.set_shown(block, false),
                _ => return toast::not_applied(&Toast::warn("Unknown layout change.")),
            }
            let value = layout.to_setting();
            let written = state.with_db(|conn| {
                birdnet_db::settings::set(
                    conn,
                    SETTING_KEY,
                    &value,
                    birdnet_db::settings::SettingsCategory::General,
                )
            });
            if let Err(e) = written {
                tracing::error!(error = %e, "storing the Today layout failed");
                return toast::not_applied(&Toast::error("Could not save the layout."));
            }
            crate::audit::audit(
                state,
                Some(&request_user),
                "today.layout.change",
                Some(&format!("{} {action}", block.id())),
                None,
            );
            let mut body = render_section(&layout).replacen(
                r#"<section class="card" id="set-today-layout""#,
                r#"<section class="card" id="set-today-layout" hx-swap-oob="true""#,
                1,
            );
            body.push_str(&Toast::success("Today page layout saved.").render_oob());
            Html(body).into_response()
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_edges_of_each_group_disable_the_move_that_would_do_nothing() {
        let html = render_section(&Layout::default());
        // Hero is first in its group.
        assert!(
            html.contains(r#"hx-post="/admin/today-layout/hero/up" hx-swap="none" aria-label="Move up: Today&#x27;s headline and live signal" disabled"#),
            "{html}"
        );
        // The log can never move, so it is offered no move at all.
        assert!(!html.contains("/admin/today-layout/log/up"), "{html}");
        assert!(!html.contains("/admin/today-layout/log/down"), "{html}");
        assert!(html.contains("/admin/today-layout/log/hide"), "{html}");
        // A middle block can move both ways.
        assert!(html.contains(r#"/admin/today-layout/nudge/up" hx-swap="none" aria-label="Move up: Suggestion line">"#), "{html}");
    }

    #[test]
    fn a_hidden_block_offers_show() {
        let html = render_section(&Layout::parse("-best"));
        assert!(html.contains(r#"/admin/today-layout/best/show""#), "{html}");
        assert!(
            !html.contains(r#"/admin/today-layout/best/hide""#),
            "{html}"
        );
        assert!(html.contains("Side column · Hidden"), "{html}");
    }
}
