//! Species × hour activity heatmap widget for the dashboard.

use std::fmt::Write as _;

use axum::extract::State;
use axum::http::{StatusCode, header};

use crate::routes::pages::{escape_html, today_date_string};
use crate::state::AppState;

/// Species × hour activity heatmap for today (top 12 species, 24-hour grid).
pub(super) async fn activity_heatmap_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    let today = today_date_string();
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::sqlite::today_species_hour_heatmap(conn, &today, 12))
    })
    .await;

    let cells = match result {
        Ok(Ok(c)) if !c.is_empty() => c,
        _ => {
            return (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/html")],
                "<p style=\"color:var(--text-muted);text-align:center;padding:1rem 0;\">No activity recorded today.</p>"
                    .to_string(),
            );
        }
    };

    // Collect species in query order (already sorted by total desc), build hour arrays.
    let mut species_order: Vec<String> = Vec::new();
    let mut species_map: std::collections::HashMap<String, [i64; 24]> =
        std::collections::HashMap::new();
    for (name, hour, count) in &cells {
        let entry = species_map.entry(name.clone()).or_insert([0i64; 24]);
        entry[usize::from(*hour)] = *count;
        if !species_order.contains(name) {
            species_order.push(name.clone());
        }
    }
    let max_count = cells.iter().map(|(_, _, c)| *c).max().unwrap_or(1).max(1);

    let mut html = String::with_capacity(8192);
    html.push_str(
        "<style>\
        .ah{display:grid;grid-template-columns:9rem repeat(24,1fr);gap:2px;overflow-x:auto;}\
        .ah-lbl{font-size:.75rem;color:var(--text);overflow:hidden;text-overflow:ellipsis;\
                white-space:nowrap;padding-right:.25rem;align-self:center;}\
        .ah-hr{font-size:.6rem;color:var(--text-muted);text-align:center;padding:.1rem 0;}\
        .ah-cell{height:20px;border-radius:3px;transition:opacity .15s;cursor:default;}\
        .ah-cell:hover{outline:1px solid var(--accent);z-index:1;position:relative;}\
        </style>\
        <div class=\"ah\">",
    );

    // Header row: empty label + hour columns 0..23
    html.push_str("<div></div>");
    for h in 0u8..24 {
        let _ = write!(html, "<div class=\"ah-hr\">{h}</div>");
    }

    // Species rows
    for name in &species_order {
        let hours = species_map.get(name).copied().unwrap_or([0i64; 24]);
        let safe_name = escape_html(name);
        let _ = write!(
            html,
            "<div class=\"ah-lbl\" title=\"{safe_name}\">{safe_name}</div>"
        );
        for (h, &count) in hours.iter().enumerate() {
            if count == 0 {
                let _ = write!(
                    html,
                    "<div class=\"ah-cell\" style=\"background:var(--bg-hover);\"></div>",
                );
            } else {
                #[allow(clippy::cast_precision_loss)]
                let ratio = count as f64 / max_count as f64;
                let alpha = ratio.mul_add(0.80, 0.12);
                let title = format!("{name} {h:02}:00 \u{2014} {count}");
                let safe_title = escape_html(&title);
                let _ = write!(
                    html,
                    "<div class=\"ah-cell\" \
                       style=\"background:rgba(var(--accent-rgb),{alpha:.2});\" \
                       title=\"{safe_title}\"></div>",
                );
            }
        }
    }

    html.push_str("</div>");
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}
