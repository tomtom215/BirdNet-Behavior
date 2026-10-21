//! Year in Review — an editorial recap of the calendar year so far.
//!
//! A read-only celebration page: big-number tiles, a tape with one cell per
//! week of the year, the species leaderboard, a few milestone facts and a
//! closing statement. Every figure covers 1 January to today and nothing
//! earlier.

use std::fmt::Write as _;

use axum::Router;

use super::atoms::avatar;
use super::{
    escape_html, group_thousands, simple_url_encode,
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
    // Every one of these was `.unwrap_or(0)` / `.unwrap_or_default()` *inside*
    // the closure, which returned a plain tuple — so the `Err` arm below only
    // ever fired on a task panic. A database error produced a complete,
    // ordinary-looking page reading "0 detections across 0 species", "busiest
    // day —", for a station with three years of records. There is nothing on
    // this page that is not a claim about the reader's own birds, so a query
    // that fails has to say so rather than round down to nothing.
    //
    // Every figure is the calendar year so far, 1 January to today, because
    // that is what the masthead's "· 2026" says. The page used to mix three
    // scopes under that label: all-time totals, leaderboard and first day; a
    // trailing 366 days for the busiest day; and the calendar year for "new
    // to your list" — so a station with three years of records showed all
    // three years' detections as "your year".
    let today = today_date_string();
    let start = year_start(&today);
    let names = state.names_handle();
    let result = tokio::task::spawn_blocking(move || {
        state.with_read_db(|conn| {
            let daily = birdnet_db::sqlite::range_daily_counts(conn, &start, &today)?;
            // 1000 is more species than any station has heard.
            let top = birdnet_db::sqlite::weekly_top_species(conn, &start, &today, 1000)?;
            let new = birdnet_db::sqlite::weekly_new_species(conn, &start, &today)?;
            // When the station was listening, so a day it was off is not a quiet one in the
            // calendar. Unreadable means unknown, which draws nothing.
            let effort = super::listening::effort_by_date(conn, &start, &today).ok();
            Ok::<_, birdnet_db::sqlite::DbError>((today, daily, top, new, effort))
        })
    })
    .await;

    let (today, daily, top, new, effort) = match result {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "year in review: query failed");
            return super::error_states::could_not_load("your year in review");
        }
        Err(e) => {
            tracing::warn!(error = %e, "year in review: task failed");
            return super::error_states::could_not_load("your year in review");
        }
    };

    render_content(&today, &daily, &top, &new, effort.as_ref(), &names)
}

/// `YYYY-01-01` for the year `today` falls in.
fn year_start(today: &str) -> String {
    format!("{}-01-01", today.get(0..4).unwrap_or("1970"))
}

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    clippy::too_many_lines
)]
fn render_content(
    today: &str,
    daily: &[birdnet_db::sqlite::DailyCount],
    top: &[(String, String, i64)],
    new: &[(String, String, String)],
    effort: Option<&std::collections::BTreeMap<String, f64>>,
    names: &birdnet_core::i18n::NameLocalizer,
) -> String {
    let year = today.get(0..4).unwrap_or("----");
    let total: i64 = daily.iter().map(|d| d.count).sum();
    let species = top.len();
    let active_days = daily.len();

    // Busiest day: the first of any tie, so the answer does not depend on
    // which of two equal days the iterator met last.
    let busiest = daily.iter().fold(
        None::<&birdnet_db::sqlite::DailyCount>,
        |best, d| match best {
            Some(b) if b.count >= d.count => Some(b),
            _ => Some(d),
        },
    );

    let mut html = String::with_capacity(8192);

    // ── Editorial hero ────────────────────────────────────────────────────
    // O-20 — help link in the year-in-review masthead.
    let help_link = super::help::help_link(super::help::Topic::Reports);
    let busiest_count = busiest.map_or(0, |d| d.count);
    let new_this_year = new.len();
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
  <div class="rp-stat"><div class="v">{species}</div><div class="l">species heard</div><div class="d">this year</div></div>
  <div class="rp-stat"><div class="v rare">{new_this_year}</div><div class="l">new to your list</div></div>
  <div class="rp-stat"><div class="v">{busy}</div><div class="l">busiest day</div><div class="d">{busy_date}</div></div>
</div>"#,
        det = group_thousands(total),
        busy = group_thousands(busiest_count),
        busy_date = busiest.map_or_else(|| "—".to_string(), |d| escape_html(&d.date)),
    );

    // ── The year as a calendar ───────────────────────────────────────────
    // A cell a day from 1 January to today (not a cell a week: a week hides the day
    // the yard was loudest, which is the day a reader looks for).
    html.push_str(
        r#"<div class="bnb-card pad"><div class="section-header"><div><div class="bnb-eyebrow">Every day</div><h2 class="sh-h">The year in activity</h2></div></div>"#,
    );
    let counts: std::collections::BTreeMap<String, i64> = daily
        .iter()
        .filter(|d| d.count > 0)
        .map(|d| (d.date.clone(), d.count))
        .collect();
    html.push_str(&super::viz::heat::calendar(&super::viz::heat::Calendar {
        id: "year-in-review",
        from: &year_start(today),
        to: today,
        counts: &counts,
        effort,
        unit: "detections",
        name: "Your year in activity",
    }));
    html.push_str("</div>");

    // ── Leaderboard + milestones (two columns) ───────────────────────────
    html.push_str(r#"<div class="yir-cols">"#);

    // Leaderboard.
    html.push_str(
        r#"<div class="bnb-card pad"><div class="section-header"><div><div class="bnb-eyebrow">Most heard</div><h2 class="sh-h">The year's leaderboard</h2></div><a class="action" href="/species">Full list →</a></div>"#,
    );
    if top.is_empty() {
        let _ = write!(
            html,
            r#"<p class="bnb-meta">Nothing heard yet in {year}.</p>"#
        );
    } else {
        for (i, (sci, com, count)) in top.iter().take(10).enumerate() {
            let shown = names.name(sci, com);
            let _ = write!(
                html,
                r#"<div class="rp-row"><span class="rk">{rank}</span>{av}<a class="nm" href="/species/detail?name={enc}">{name}</a><span class="ct">{count}</span></div>"#,
                rank = i + 1,
                av = avatar(com, shown, sci, ""),
                enc = simple_url_encode(com),
                name = escape_html(shown),
                count = group_thousands(*count),
            );
        }
    }
    html.push_str("</div>");

    // Milestones.
    // `daily` is in date order and `new` in first-heard order.
    let first_voice = daily.first().map(|d| d.date.as_str());
    let newest = new
        .last()
        .map(|(sci, com, date)| (names.name(sci, com), date.as_str()));
    let leader = top
        .first()
        .map(|(sci, com, count)| (names.name(sci, com), *count));

    html.push_str(
        r#"<div class="bnb-card pad"><div class="section-header"><div><div class="bnb-eyebrow">Milestones</div><h2 class="sh-h">Moments that mattered</h2></div></div>"#,
    );
    match first_voice {
        Some(date) => milestone(
            &mut html,
            "✦",
            "First voice of the year",
            &format!("{} · the first day heard in {year}", escape_html(date)),
        ),
        None => milestone(
            &mut html,
            "✦",
            "First voice of the year",
            &format!("Not yet — nothing heard in {year} so far"),
        ),
    }
    if let Some((shown, count)) = leader {
        milestone(
            &mut html,
            "♪",
            "Most-heard species",
            &format!(
                "{} · {} detections",
                escape_html(shown),
                group_thousands(count)
            ),
        );
    }
    if let Some(d) = busiest {
        milestone(
            &mut html,
            "☼",
            "Busiest day",
            &format!(
                "{} · {} · the loudest the yard got this year",
                escape_html(&d.date),
                group_thousands(d.count)
            ),
        );
    }
    if let Some((shown, date)) = newest {
        milestone(
            &mut html,
            "✸",
            "Newest arrival",
            &format!("{} · first heard {}", escape_html(shown), escape_html(date)),
        );
    }
    html.push_str("</div></div>");

    // ── Closing card ─────────────────────────────────────────────────────
    let _ = write!(
        html,
        r#"<div class="bnb-card pad yir-close">
  <div class="bnb-eyebrow">The tally</div>
  <p class="display yir-close-line">{species} species and <span class="accent">{total}</span> detections across {days} days of listening in {year}.</p>
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
    fn render_content_smoke() {
        let daily = vec![birdnet_db::sqlite::DailyCount {
            date: "2026-05-20".into(),
            count: 30,
        }];
        let top = vec![(
            "Cardinalis cardinalis".to_string(),
            "Northern Cardinal".to_string(),
            30,
        )];
        let html = render_content(
            "2026-05-22",
            &daily,
            &top,
            &[],
            None,
            &birdnet_core::i18n::NameLocalizer::english(),
        );
        assert!(html.contains("Your year in"));
        assert!(html.contains("rp-stats"));
        assert!(html.contains("Northern Cardinal"));
        assert!(html.contains("leaderboard"));
        // The year is a calendar, a cell a day from 1 January to today.
        assert!(html.contains("The year in activity") && html.contains("cal-svg"), "{html}");
        assert_eq!(html.matches("data-tip=").count(), 142, "1 Jan to 22 May is 142 days");
    }

    /// The calendar tells a day the station was off from a day it heard nothing, from the
    /// ledger: here the 21st was listened to and the 19th was not.
    #[test]
    fn the_year_calendar_says_which_days_the_station_was_off() {
        let daily = vec![birdnet_db::sqlite::DailyCount { date: "2026-05-20".into(), count: 30 }];
        let effort: std::collections::BTreeMap<String, f64> =
            [("2026-05-20", 86_400.0), ("2026-05-21", 86_400.0)].iter().map(|(d, s)| ((*d).to_string(), *s)).collect();
        let html = render_content("2026-05-22", &daily, &[], &[], Some(&effort), &birdnet_core::i18n::NameLocalizer::english());
        assert!(html.contains("Tue 19 May 2026&#10;The station was not listening"), "{html}");
        assert!(html.contains("Thu 21 May 2026&#10;Listened all day and heard none"), "{html}");
    }

    /// Two equal days: the busiest is the first, whatever order a fold meets
    /// them in — `max_by_key` returns the last.
    #[test]
    fn the_busiest_day_of_a_tie_is_the_first() {
        let day = |date: &str| birdnet_db::sqlite::DailyCount {
            date: date.into(),
            count: 5,
        };
        let html = render_content(
            "2026-05-22",
            &[day("2026-03-01"), day("2026-04-01")],
            &[],
            &[],
            None,
            &birdnet_core::i18n::NameLocalizer::english(),
        );
        assert!(html.contains("2026-03-01 · 5 · the loudest"), "{html}");
    }
}
