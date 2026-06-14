//! Year in Review — an editorial annual recap of the station's listening year.
//!
//! A read-only celebration page: big-number tiles, a 52-week activity tape,
//! the species leaderboard, a few milestone facts and a closing statement.
//! Everything is computed from the existing SQLite aggregates.

use std::fmt::Write as _;

use axum::Router;

use super::atoms::avatar;
use super::{
    date_to_epoch_days, days_to_date, escape_html, group_thousands, simple_url_encode,
    today_date_string,
};
use crate::state::AppState;

/// Mount the Year in Review page route.
pub fn router() -> Router<AppState> {
    Router::new()
}

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation
)]
/// The year-in-review surface, rendered for embedding by `homes::reports`
/// ("Year in review" tab). Fully server-computed (no HTMX shell), so this is
/// async and touches the database.
pub(super) async fn content(state: AppState) -> String {
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let total = birdnet_db::sqlite::detection_count(conn).unwrap_or(0);
            let species = birdnet_db::sqlite::species_count(conn).unwrap_or(0);
            let dates = birdnet_db::sqlite::distinct_detection_dates(conn).unwrap_or_default();
            // limit 1000 covers every species → doubles as a sci→common lookup
            let all = birdnet_db::sqlite::top_species(conn, 1000).unwrap_or_default();
            let first_seen = birdnet_db::sqlite::species_first_seen(conn).unwrap_or_default();
            let daily = birdnet_db::sqlite::daily_counts(conn, 366).unwrap_or_default();
            (total, species, dates, all, first_seen, daily)
        })
    })
    .await;

    let Ok((total, species, dates, all, first_seen, daily)) = result else {
        return "<p class=\"bnb-meta\">Failed to load the year in review.</p>".to_string();
    };

    render_content(total, species, &dates, &all, &first_seen, &daily)
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    clippy::too_many_lines
)]
fn render_content(
    total: i64,
    species: i64,
    dates: &[String],
    all: &[birdnet_db::sqlite::SpeciesCount],
    first_seen: &std::collections::HashMap<String, String>,
    daily: &[birdnet_db::sqlite::DailyCount],
) -> String {
    let today = today_date_string();
    let year = today.get(0..4).unwrap_or("----");
    let active_days = dates.len();

    // Busiest day.
    let busiest = daily.iter().max_by_key(|d| d.count);

    // 52(+1)-week activity tape over the trailing year.
    let base = date_to_epoch_days(&today).saturating_sub(364);
    let mut weeks = [0i64; 53];
    for dc in daily {
        let e = date_to_epoch_days(&dc.date);
        if e >= base {
            let wk = usize::try_from((e - base) / 7).unwrap_or(0).min(52);
            weeks[wk] += dc.count;
        }
    }
    let week_max = weeks.iter().copied().max().unwrap_or(1).max(1) as f64;

    let mut html = String::with_capacity(8192);

    // ── Editorial hero ────────────────────────────────────────────────────
    // O-20 — help link in the year-in-review masthead.
    let help_link = super::help::help_link(super::help::Topic::Reports);
    let busiest_count = busiest.map_or(0, |d| d.count);
    let new_this_year = first_seen.values().filter(|d| d.starts_with(year)).count();
    let per_day = if active_days > 0 {
        total / i64::try_from(active_days).unwrap_or(1).max(1)
    } else {
        0
    };
    let mut lead = format!(
        "<b>{} detections</b> across <b>{species} species</b>",
        group_thousands(total)
    );
    if new_this_year > 0 {
        let _ = write!(
            lead,
            " — and <b>{new_this_year}</b> heard for the very first time"
        );
    }
    lead.push('.');
    let _ = write!(
        html,
        r#"<div class="rp-hero">
  <div class="eyebrow">Year in review · {year} {help_link}</div>
  <h1>Your year in <em>birdsong</em>.</h1>
  <p class="lead">{lead}</p>
</div>"#,
    );

    // ── Stat band ─────────────────────────────────────────────────────────
    let _ = write!(
        html,
        r#"<div class="rp-stats">
  <div class="rp-stat"><div class="v moss">{det}</div><div class="l">detections</div><div class="d">≈ {per_day} a day</div></div>
  <div class="rp-stat"><div class="v">{species}</div><div class="l">species heard</div><div class="d">on the life list</div></div>
  <div class="rp-stat"><div class="v rare">{new_this_year}</div><div class="l">new to your list</div></div>
  <div class="rp-stat"><div class="v">{busy}</div><div class="l">busiest day</div><div class="d">{busy_date}</div></div>
</div>"#,
        det = group_thousands(total),
        busy = group_thousands(busiest_count),
        busy_date = busiest.map_or_else(|| "—".to_string(), |d| escape_html(&d.date)),
    );

    // ── Year tape ────────────────────────────────────────────────────────
    html.push_str(
        r#"<div class="bnb-card pad"><div class="section-header"><div><div class="bnb-eyebrow">Every week</div><h3>The year in activity</h3></div></div>"#,
    );
    html.push_str(r#"<div class="yir-tape">"#);
    for (wk, &c) in weeks.iter().enumerate() {
        let intensity = (c as f64 / week_max).clamp(0.0, 1.0);
        let pct = (intensity * 92.0).round() as i64 + if c > 0 { 8 } else { 0 };
        let (wy, wm, wd) = days_to_date(base + wk as u64 * 7);
        let bg = if c > 0 {
            format!("color-mix(in oklch, var(--moss) {pct}%, var(--surface-2))")
        } else {
            "var(--surface-2)".to_string()
        };
        let _ = write!(
            html,
            r#"<span class="yir-tape-cell" title="Week of {wy:04}-{wm:02}-{wd:02} — {c} detections" data-style="background:{bg};"></span>"#,
        );
    }
    html.push_str("</div>");
    // Month labels aligned beneath the tape.
    html.push_str(r#"<div class="yir-months">"#);
    let mut prev_month = 0u32;
    for wk in 0..weeks.len() {
        let (_, wm, _) = days_to_date(base + wk as u64 * 7);
        let label = if wm == prev_month {
            ""
        } else {
            prev_month = wm;
            MONTHS
                .get((wm.saturating_sub(1)) as usize)
                .copied()
                .unwrap_or("")
        };
        let _ = write!(
            html,
            r#"<span class="bnb-meta mono yir-month">{label}</span>"#,
        );
    }
    html.push_str("</div></div>");

    // ── Leaderboard + milestones (two columns) ───────────────────────────
    html.push_str(r#"<div class="yir-cols">"#);

    // Leaderboard.
    html.push_str(
        r#"<div class="bnb-card pad"><div class="section-header"><div><div class="bnb-eyebrow">Most heard</div><h3>The year's leaderboard</h3></div><a class="action" href="/species">Full list →</a></div>"#,
    );
    if all.is_empty() {
        html.push_str(r#"<p class="bnb-meta">No detections yet.</p>"#);
    } else {
        for (i, sp) in all.iter().take(10).enumerate() {
            let _ = write!(
                html,
                r#"<div class="rp-row"><span class="rk">{rank}</span>{av}<a class="nm" href="/species/detail?name={enc}">{name}</a><span class="ct">{count}</span></div>"#,
                rank = i + 1,
                av = avatar(&sp.com_name, ""),
                enc = simple_url_encode(&sp.com_name),
                name = escape_html(&sp.com_name),
                count = group_thousands(sp.count),
            );
        }
    }
    html.push_str("</div>");

    // Milestones.
    let sci_to_com: std::collections::HashMap<&str, &str> = all
        .iter()
        .map(|s| (s.sci_name.as_str(), s.com_name.as_str()))
        .collect();
    let first_voice = dates.iter().min().cloned().unwrap_or_default();
    let newest = first_seen
        .iter()
        .max_by(|a, b| a.1.cmp(b.1))
        .map(|(sci, date)| {
            let com = sci_to_com
                .get(sci.as_str())
                .copied()
                .unwrap_or(sci.as_str());
            (com.to_string(), date.clone())
        });
    let busiest_label = busiest.map_or_else(
        || "—".to_string(),
        |d| format!("{} · {}", d.date, group_thousands(d.count)),
    );
    let leader = all.first().map(|s| (s.com_name.clone(), s.count));

    html.push_str(
        r#"<div class="bnb-card pad"><div class="section-header"><div><div class="bnb-eyebrow">Milestones</div><h3>Moments that mattered</h3></div></div>"#,
    );
    milestone(
        &mut html,
        "✦",
        "First voice of the year",
        &format!("{first_voice} · the earliest day on record"),
    );
    if let Some((com, count)) = leader {
        milestone(
            &mut html,
            "♪",
            "Most-heard species",
            &format!(
                "{} · {} detections",
                escape_html(&com),
                group_thousands(count)
            ),
        );
    }
    milestone(
        &mut html,
        "☼",
        "Busiest day",
        &format!("{busiest_label} · the loudest the yard ever got"),
    );
    if let Some((com, date)) = newest {
        milestone(
            &mut html,
            "✸",
            "Newest arrival",
            &format!("{} · first heard {date}", escape_html(&com)),
        );
    }
    html.push_str("</div></div>");

    // ── Closing card ─────────────────────────────────────────────────────
    let _ = write!(
        html,
        r#"<div class="bnb-card pad yir-close">
  <div class="bnb-eyebrow">The tally</div>
  <p class="display yir-close-line">{species} species and <span class="accent">{total}</span> detections across {days} days of listening.</p>
  <p class="bnb-meta">Here's to next year's first dawn chorus.</p>
</div>"#,
        species = species,
        total = group_thousands(total),
        days = active_days,
    );

    html
}

/// A milestone row (glyph · title · detail) inside the milestones card.
fn milestone(html: &mut String, glyph: &str, title: &str, detail: &str) {
    let _ = write!(
        html,
        r#"<div class="rp-milestone"><span class="ic">{glyph}</span><div><div class="t">{title}</div><div class="d">{detail}</div></div></div>"#,
        glyph = escape_html(glyph),
        title = escape_html(title),
        detail = detail,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_to_epoch_days_known() {
        assert_eq!(date_to_epoch_days("1970-01-01"), 0);
        assert!(date_to_epoch_days("2026-05-22") > 20_000);
    }

    #[test]
    fn render_content_smoke() {
        let all = vec![birdnet_db::sqlite::SpeciesCount {
            com_name: "Northern Cardinal".into(),
            sci_name: "Cardinalis cardinalis".into(),
            count: 100,
            avg_confidence: 0.9,
        }];
        let daily = vec![birdnet_db::sqlite::DailyCount {
            date: "2026-05-20".into(),
            count: 30,
        }];
        let mut fs = std::collections::HashMap::new();
        fs.insert(
            "Cardinalis cardinalis".to_string(),
            "2026-05-01".to_string(),
        );
        let html = render_content(100, 1, &["2026-05-20".to_string()], &all, &fs, &daily);
        assert!(html.contains("Your year in"));
        assert!(html.contains("rp-stats"));
        assert!(html.contains("Northern Cardinal"));
        assert!(html.contains("Leaderboard") || html.contains("leaderboard"));
    }
}
