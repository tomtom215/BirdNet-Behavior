//! The **Reports** home (`/reports`) — "the recap".
//!
//! Folds the three look-back surfaces into one home (v3 spine,
//! `Reports_home.html`):
//!
//! | Tab     | Question           | Folded surface (old route)       |
//! |---------|--------------------|-----------------------------------|
//! | weekly  | the Sunday recap   | weekly report (`/weekly`)        |
//! | year    | your year in song  | year in review (`/year-in-review`) |
//! | history | browse past days   | history (`/history`)             |

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::Html;
use axum::{Router, routing::get};
use serde::Deserialize;

use super::{SubTab, resolve_tab, subtabs};
use crate::state::AppState;

/// The Reports tab table, in display order. Keys are the `?tab=` values the
/// redirect table targets.
pub const TABS: &[SubTab] = &[
    SubTab {
        key: "weekly",
        label: "Weekly",
        question: "the Sunday recap",
    },
    SubTab {
        key: "year",
        label: "Year in review",
        question: "your year in song",
    },
    SubTab {
        key: "history",
        label: "History",
        question: "browse past days",
    },
];

/// Mount `/reports`.
pub fn router() -> Router<AppState> {
    Router::new().route("/reports", get(reports_page))
}

/// `?tab=` selector for the Reports home.
#[derive(Deserialize)]
struct TabParam {
    tab: Option<String>,
}

async fn reports_page(
    State(state): State<AppState>,
    Query(q): Query<TabParam>,
    headers: HeaderMap,
) -> Html<String> {
    let tab = resolve_tab(TABS, q.tab.as_deref());
    let body = match tab.key {
        "year" => crate::routes::pages::year_in_review::content(state).await,
        "history" => crate::routes::pages::history::content(state).await,
        // "weekly" and the unknown-key clamp.
        _ => crate::routes::pages::weekly_report::content(),
    };
    let content = format!(
        "{tabs}{print}{body}",
        tabs = subtabs("/reports", "tab", TABS, tab.key),
        print = PRINT_BAR,
    );
    let title = format!("Reports · {}", tab.label);
    crate::routes::pages::render_page_for_request(&title, &content, "reports", &headers)
}

/// A CSP-safe "Save as PDF" affordance: a real button that triggers the
/// browser's print dialog (the existing `print.css` `@media print` rules then
/// produce a clean, light-palette, page-broken document). The delegated click
/// handler is an inline `<script>` — the security layer stamps it with the
/// per-request CSP nonce, like every other inline script. `data-print-hide`
/// keeps the button itself out of the printed output.
const PRINT_BAR: &str = r#"<div class="rp-meta">
  <span class="rp-print bnb-meta">Make a keepsake — print this recap or save it as a PDF.</span>
  <button type="button" class="bnb-btn ghost rp-pdf" data-print data-print-hide>⎙ Save as PDF</button>
</div>
<script>
document.addEventListener('click', function (e) {
  if (e.target.closest('[data-print]')) { e.preventDefault(); window.print(); }
});
</script>"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_keys_match_the_redirect_targets() {
        let keys: Vec<&str> = TABS.iter().map(|t| t.key).collect();
        assert_eq!(keys, vec!["weekly", "year", "history"]);
    }
}
