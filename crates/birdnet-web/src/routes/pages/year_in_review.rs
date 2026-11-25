//! Year in Review — an editorial recap of the calendar year so far.
//!
//! A read-only celebration page: big-number tiles, a tape with one cell per
//! week of the year, the species leaderboard, a few milestone facts and a
//! closing statement. Every figure covers 1 January to today and nothing
//! earlier.

use std::fmt::Write as _;

use axum::Router;

use super::atoms::avatar;
use super::{escape_html, group_thousands, simple_url_encode, today_date_string};
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
        // Each species in "who led" is in the colour it owns on the station, as on every chart.
        let slots = super::viz::kit::station_slots(&state)?;
        state.with_read_db(|conn| {
            let (daily, top, new, effort, monthly) = read_year(conn, &start, &today)?;
            Ok::<_, birdnet_db::sqlite::DbError>((today, daily, top, new, effort, monthly, slots))
        })
    })
    .await;

    let (today, daily, top, new, effort, monthly, slots) = match result {
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

    render_content(
        &today,
        &daily,
        &top,
        &new,
        effort.as_ref(),
        &monthly,
        &names,
        &slots,
    )
}

/// What the page reads for `start..=today`: the days, the leaderboard, the new species, the
/// listening ledger and the month-by-month rows. A plain function of its dates, so a test can
/// hold them fixed.
type YearRows = (
    Vec<birdnet_db::sqlite::DailyCount>,
    Vec<(String, String, i64)>,
    Vec<(String, String, String)>,
    Option<std::collections::BTreeMap<String, f64>>,
    Vec<(String, String, u32, i64)>,
);

fn read_year(
    conn: &rusqlite::Connection,
    start: &str,
    today: &str,
) -> Result<YearRows, birdnet_db::sqlite::DbError> {
    let daily = birdnet_db::sqlite::range_daily_counts(conn, start, today)?;
    // 1000 is more species than any station has heard.
    let top = birdnet_db::sqlite::weekly_top_species(conn, start, today, 1000)?;
    let new = birdnet_db::sqlite::weekly_new_species(conn, start, today)?;
    // Month by month, for the "who led" picture; the same rows the leaderboard is made of.
    let monthly = birdnet_db::sqlite::monthly_species_counts(conn, start, today)?;
    // When the station was listening, so a day it was off is not a quiet one in the
    // calendar. Unreadable means unknown, which draws nothing.
    let effort = super::listening::effort_by_date(conn, start, today).ok();
    Ok((daily, top, new, effort, monthly))
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
    monthly: &[(String, String, u32, i64)],
    names: &birdnet_core::i18n::NameLocalizer,
    slots: &super::viz::kit::SpeciesSlots,
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
    // The days the average and the closing line count: the days the station listened, from
    // the ledger, or where there is none the days something was heard, said as such (UIC-01).
    // Counting only the days with a detection called 250 of 320 listened days "250 days of
    // listening", and left the silent days out of the average.
    let listened = effort.and_then(super::listening::listened_day_count);
    let (days, days_are) =
        listened.map_or((active_days, "with a detection"), |n| (n, "of listening"));
    let days_are = format!("{} {days_are}", if days == 1 { "day" } else { "days" });
    let per_day = if days > 0 {
        total / i64::try_from(days).unwrap_or(1).max(1)
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
        "{}",
        super::frame::page_head(&super::frame::Head {
            eyebrow: &format!("Reports · year in review · {year}"),
            help: &help_link,
            title: "Your year in <em>birdsong</em>.",
            lead: "",
            lede: &lead,
            aside: super::frame::PRINT_BUTTON,
        }),
    );

    // ── Stat band ─────────────────────────────────────────────────────────
    let _ = write!(
        html,
        r#"<div class="rp-stats">
  <div class="rp-stat"><div class="v moss">{det}</div><div class="l">detections</div><div class="d">≈ {per_day} {per_what}</div></div>
  <div class="rp-stat"><div class="v">{species}</div><div class="l">species heard</div><div class="d">this year</div></div>
  <div class="rp-stat"><div class="v">{new_this_year}</div><div class="l">new to your list</div></div>
  <div class="rp-stat"><div class="v">{busy}</div><div class="l">busiest day</div><div class="d">{busy_date}</div></div>
</div>"#,
        det = group_thousands(total),
        per_what = if listened.is_some() {
            "a day"
        } else {
            "a day with a detection"
        },
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

    // ── Month by month: who led, and when that changed ───────────────────
    html.push_str(&seasons_card(today, top, monthly, names, slots));

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
  <p class="display yir-close-line">{species} species and <span class="accent">{total}</span> detections across {days} {days_are} in {year}.</p>
  <p class="bnb-meta">Here's to next year's first dawn chorus.</p>
</div>"#,
        species = species,
        total = group_thousands(total),
    );

    html
}

/// Species the month-by-month picture follows: the busiest of the year, each in its own colour.
const SEASON_SPECIES: usize = 8;

/// What the "who led each month" picture is made of.
#[derive(Debug, PartialEq)]
struct Seasons {
    /// `(name, counts)` for each followed species, most heard first. `counts[m]` is the count in
    /// month `m` (0 is January): `None` for a month that has not happened, a real `Some(0)` for a
    /// month that has and had none.
    species: Vec<(String, [Option<i64>; 12])>,
    /// The month now running, 0-based: it is "so far".
    this_month: usize,
}

/// The busiest `SEASON_SPECIES` of the year by month, from the leaderboard and the monthly counts.
///
/// `None` unless there is something to follow over time: two species and two months in which
/// either was heard. One species, or one month, is a number and not a trend.
fn seasons_data(
    today: &str,
    top: &[(String, String, i64)],
    monthly: &[(String, String, u32, i64)],
    names: &birdnet_core::i18n::NameLocalizer,
) -> Option<Seasons> {
    let now: usize = today
        .get(5..7)?
        .parse()
        .ok()
        .filter(|m| (1..=12).contains(m))?;
    let species: Vec<(String, [Option<i64>; 12])> = top
        .iter()
        .filter(|(_, _, total)| *total > 0)
        .take(SEASON_SPECIES)
        .map(|(sci, com, _)| {
            let mut counts = [None; 12];
            for (m, slot) in counts.iter_mut().enumerate().take(now) {
                let n = monthly
                    .iter()
                    .find(|(s, c, month, _)| s == sci && c == com && *month as usize == m + 1)
                    .map_or(0, |r| r.3);
                *slot = Some(n);
            }
            (names.name(sci, com).to_owned(), counts)
        })
        .collect();
    let months_heard = (0..now)
        .filter(|&m| species.iter().any(|(_, c)| c[m].is_some_and(|n| n > 0)))
        .count();
    (species.len() >= 2 && months_heard >= 2).then_some(Seasons {
        species,
        this_month: now - 1,
    })
}

impl Seasons {
    /// The species that led each month so far (`None` where none was heard). A tie goes to the
    /// species heard more over the year, which is the one listed first.
    fn leaders(&self) -> Vec<Option<usize>> {
        (0..=self.this_month)
            .map(|m| {
                let mut best: Option<(usize, i64)> = None;
                for (i, (_, c)) in self.species.iter().enumerate() {
                    let n = c[m].unwrap_or(0);
                    if n > 0 && best.is_none_or(|(_, b)| n > b) {
                        best = Some((i, n));
                    }
                }
                best.map(|(i, _)| i)
            })
            .collect()
    }

    /// One plain sentence for what the picture shows, printed under it (so it is there with no script).
    fn sentence(&self) -> String {
        let led = self.leaders();
        let heard: Vec<usize> = led.iter().flatten().copied().collect();
        let mut wins = vec![0_usize; self.species.len()];
        for &i in &heard {
            wins[i] += 1;
        }
        let top = wins
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(&a.0)))
            .map_or(0, |(i, _)| i);
        let changes = heard.windows(2).filter(|w| w[0] != w[1]).count();
        let name = &self.species[top].0;
        let months = heard.len();
        if wins[top] == months {
            format!("{name} led every month so far.")
        } else {
            format!(
                "{name} led {} of the {months} months so far; the lead changed hands {changes} time{}.",
                wins[top],
                if changes == 1 { "" } else { "s" },
            )
        }
    }

    /// The JSON `static/seasons.js` reads. A species' `slot` is the colour it owns on the
    /// station (1 to 8, as on every other chart), or 0 for the neutral.
    fn json(&self, year: i32, slots: &super::viz::kit::SpeciesSlots) -> serde_json::Value {
        serde_json::json!({
            "year": year,
            "thisMonth": self.this_month,
            "species": self.species.iter().map(|(name, counts)| {
                serde_json::json!({ "name": name, "slot": slots.number(name), "counts": counts })
            }).collect::<Vec<_>>(),
        })
    }
}

/// The "who led each month" card, or nothing when there is nothing yet to follow.
fn seasons_card(
    today: &str,
    top: &[(String, String, i64)],
    monthly: &[(String, String, u32, i64)],
    names: &birdnet_core::i18n::NameLocalizer,
    slots: &super::viz::kit::SpeciesSlots,
) -> String {
    let Some(data) = seasons_data(today, top, monthly, names) else {
        return String::new();
    };
    let year: i32 = today.get(0..4).and_then(|y| y.parse().ok()).unwrap_or(1970);
    let mut html = String::with_capacity(4096);
    let _ = write!(
        html,
        r#"<div class="bnb-card pad" id="seasons"><div class="section-header"><div><div class="bnb-eyebrow">Month by month</div><h2 class="sh-h">Who led, and when it changed</h2></div></div><p class="fig-take">{sentence}</p><div class="ssn" data-ssn data-ssn-json="{json}"></div>"#,
        sentence = escape_html(&data.sentence()),
        json = escape_html(&data.json(year, slots).to_string()),
    );
    html.push_str(r#"<details class="pt-disc"><summary>See the numbers — detections by month</summary><div class="viz-table-scroll"><table class="pt-tbl"><caption class="sr-only">Detections for each of the year's busiest species in each month</caption><thead><tr><th scope="col">Species</th>"#);
    for m in super::viz::kit::MONTHS {
        let _ = write!(html, r#"<th scope="col" class="num">{m}</th>"#);
    }
    html.push_str("</tr></thead><tbody>");
    for (name, counts) in &data.species {
        let _ = write!(html, r#"<tr><th scope="row">{}</th>"#, escape_html(name));
        for c in counts {
            match c {
                Some(n) => {
                    let _ = write!(html, r#"<td class="num">{}</td>"#, group_thousands(*n));
                }
                None => html.push_str(r#"<td class="num">not yet</td>"#),
            }
        }
        html.push_str("</tr>");
    }
    html.push_str("</tbody></table></div></details></div>");
    // The one authored tag on this card, sealed so it gets the page's nonce (see `crate::seal`).
    html.push_str(&crate::seal::seal(&format!(
        r#"<script type="module" src="/static/seasons.js?v={}"></script>"#,
        env!("CARGO_PKG_VERSION")
    )));
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

    fn m(sci: &str, com: &str, month: u32, n: i64) -> (String, String, u32, i64) {
        (sci.to_string(), com.to_string(), month, n)
    }

    fn top_of(rows: &[(&str, &str, i64)]) -> Vec<(String, String, i64)> {
        rows.iter()
            .map(|(s, c, n)| ((*s).to_string(), (*c).to_string(), *n))
            .collect()
    }

    /// A month that has not happened is `None` and one that had none is a real zero; the picture is
    /// only offered where there is a trend to follow (two species, two months heard).
    #[test]
    fn a_month_not_yet_lived_is_not_a_month_with_none() {
        let top = top_of(&[("Aa a", "Alpha", 30), ("Bb b", "Beta", 12)]);
        let monthly = vec![
            m("Aa a", "Alpha", 1, 10),
            m("Aa a", "Alpha", 3, 20),
            m("Bb b", "Beta", 3, 12),
        ];
        let en = birdnet_core::i18n::NameLocalizer::english();
        let d = seasons_data("2026-04-10", &top, &monthly, &en).expect("two species, two months");
        assert_eq!(d.this_month, 3);
        let alpha = &d.species[0].1;
        assert_eq!(
            &alpha[..5],
            &[Some(10), Some(0), Some(20), Some(0), None],
            "February was lived and quiet; May has not come"
        );
        // One month heard only: not a trend.
        assert!(seasons_data("2026-04-10", &top, &monthly[2..], &en).is_none());
        // One species only: not a comparison.
        assert!(seasons_data("2026-04-10", &top[..1], &monthly, &en).is_none());
    }

    /// The sentence counts the months each species led and the times the lead changed hands, and a
    /// tie in a month goes to the species heard more over the year.
    #[test]
    fn the_sentence_says_who_led_and_how_often_it_changed() {
        let top = top_of(&[("Aa a", "Alpha", 60), ("Bb b", "Beta", 50)]);
        let en = birdnet_core::i18n::NameLocalizer::english();
        // Alpha leads Jan, Feb; Beta leads Mar; Apr is a tie (Alpha wins it: heard more over the year).
        let monthly = vec![
            m("Aa a", "Alpha", 1, 9),
            m("Bb b", "Beta", 1, 3),
            m("Aa a", "Alpha", 2, 9),
            m("Bb b", "Beta", 2, 3),
            m("Aa a", "Alpha", 3, 2),
            m("Bb b", "Beta", 3, 8),
            m("Aa a", "Alpha", 4, 5),
            m("Bb b", "Beta", 4, 5),
        ];
        let d = seasons_data("2026-04-30", &top, &monthly, &en).expect("data");
        assert_eq!(d.leaders(), vec![Some(0), Some(0), Some(1), Some(0)]);
        assert_eq!(
            d.sentence(),
            "Alpha led 3 of the 4 months so far; the lead changed hands 2 times."
        );
        // A species that led every month says so, with no count of changes.
        let monthly = vec![
            m("Aa a", "Alpha", 1, 9),
            m("Bb b", "Beta", 1, 3),
            m("Aa a", "Alpha", 2, 9),
            m("Bb b", "Beta", 2, 3),
        ];
        let d = seasons_data("2026-02-20", &top, &monthly, &en).expect("data");
        assert_eq!(d.sentence(), "Alpha led every month so far.");
    }

    /// The card carries the data as escaped JSON in one attribute (untrusted names cannot break out),
    /// a table of the same numbers, and says nothing when there is nothing to follow.
    #[test]
    fn the_seasons_card_carries_escaped_data_and_a_table_or_nothing() {
        let top = top_of(&[("Aa a", "Alpha <b>", 30), ("Bb b", "Beta", 12)]);
        let monthly = vec![m("Aa a", "Alpha <b>", 1, 10), m("Bb b", "Beta", 2, 12)];
        let en = birdnet_core::i18n::NameLocalizer::english();
        let html = seasons_card(
            "2026-03-05",
            &top,
            &monthly,
            &en,
            &crate::routes::pages::viz::kit::SpeciesSlots::default(),
        );
        assert!(
            html.contains(r#"data-ssn-json=""#) && html.contains("&quot;thisMonth&quot;:2"),
            "{html}"
        );
        assert!(
            !html.contains("<b>"),
            "a species name is data, never markup: {html}"
        );
        assert!(
            html.contains("Alpha &lt;b&gt;") && html.contains("not yet"),
            "{html}"
        );
        assert_eq!(
            seasons_card(
                "2026-03-05",
                &top[..1],
                &monthly,
                &en,
                &crate::routes::pages::viz::kit::SpeciesSlots::default()
            ),
            ""
        );
    }

    /// "10 new to your list" was painted in `--rare`, the red that elsewhere means a fault or a
    /// failed check, on the Year and Weekly reports (UIR-23). A count of new species is good news:
    /// its numeral is in the neutral ink of the counts beside it.
    #[test]
    fn good_news_is_not_painted_in_the_fault_red() {
        let sources = [
            ("year_in_review.rs", include_str!("year_in_review.rs")),
            ("weekly_report.rs", include_str!("weekly_report.rs")),
        ];
        let mut seen = 0;
        for (file, src) in sources {
            for (i, _) in src.match_indices(r#"<div class="l">new to your list</div>"#) {
                // The stat band's line, not this test's own needle.
                if !src[..i].ends_with("</div>") {
                    continue;
                }
                seen += 1;
                let value = src[..i].rsplit("<div class=\"v").next().unwrap_or_default();
                let class = value.split('"').next().unwrap_or_default();
                assert!(
                    !class.split_whitespace().any(|c| c == "rare"),
                    "{file}: the new-to-your-list count is classed `v{class}`"
                );
            }
        }
        assert_eq!(
            seen, 2,
            "the Year and Weekly stat bands each carry the count"
        );
    }

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
            &[],
            &birdnet_core::i18n::NameLocalizer::english(),
            &crate::routes::pages::viz::kit::SpeciesSlots::default(),
        );
        assert!(html.contains("Your year in"));
        assert!(html.contains("rp-stats"));
        assert!(html.contains("Northern Cardinal"));
        assert!(html.contains("leaderboard"));
        // The year is a calendar, a cell a day from 1 January to today.
        assert!(
            html.contains("The year in activity") && html.contains("cal-svg"),
            "{html}"
        );
        assert_eq!(
            html.matches("data-tip=").count(),
            142,
            "1 Jan to 22 May is 142 days"
        );
    }

    /// The calendar tells a day the station was off from a day it heard nothing, from the
    /// ledger: here the 21st was listened to and the 19th was not.
    #[test]
    fn the_year_calendar_says_which_days_the_station_was_off() {
        let daily = vec![birdnet_db::sqlite::DailyCount {
            date: "2026-05-20".into(),
            count: 30,
        }];
        let effort: std::collections::BTreeMap<String, f64> =
            [("2026-05-20", 86_400.0), ("2026-05-21", 86_400.0)]
                .iter()
                .map(|(d, s)| ((*d).to_string(), *s))
                .collect();
        let html = render_content(
            "2026-05-22",
            &daily,
            &[],
            &[],
            Some(&effort),
            &[],
            &birdnet_core::i18n::NameLocalizer::english(),
            &crate::routes::pages::viz::kit::SpeciesSlots::default(),
        );
        assert!(
            html.contains("Tue 19 May 2026&#10;The station was not listening"),
            "{html}"
        );
        assert!(
            html.contains("Thu 21 May 2026&#10;Listened all day and heard none"),
            "{html}"
        );
    }

    /// UIC-01. The days of listening come from the ledger, not from the days
    /// something was detected, and the daily average divides by them. Ten
    /// listened days, six with ten detections each: "10 days of listening" and
    /// "≈ 6 a day". Counterpart: with no ledger it says what it can count.
    #[test]
    fn the_days_of_listening_are_the_days_the_station_listened() {
        let daily: Vec<_> = (1..=6)
            .map(|d| birdnet_db::sqlite::DailyCount {
                date: format!("2026-05-{d:02}"),
                count: 10,
            })
            .collect();
        let effort: std::collections::BTreeMap<String, f64> = (1..=10)
            .map(|d| (format!("2026-05-{d:02}"), 86_400.0))
            .collect();
        let render = |effort| {
            render_content(
                "2026-05-22",
                &daily,
                &[],
                &[],
                effort,
                &[],
                &birdnet_core::i18n::NameLocalizer::english(),
                &crate::routes::pages::viz::kit::SpeciesSlots::default(),
            )
        };
        let html = render(Some(&effort));
        assert!(
            html.contains("across 10 days of listening in 2026"),
            "{html}"
        );
        assert!(html.contains("≈ 6 a day"), "{html}");

        let html = render(None);
        assert!(
            html.contains("across 6 days with a detection in 2026"),
            "{html}"
        );
        assert!(!html.contains("days of listening"), "{html}");
    }

    /// The page as it is read from a station's store on the fixed date `today`.
    fn page_of(conn: &rusqlite::Connection, today: &str) -> String {
        let (daily, top, new, effort, monthly) =
            read_year(conn, &year_start(today), today).expect("read");
        render_content(
            today,
            &daily,
            &top,
            &new,
            effort.as_ref(),
            &monthly,
            &birdnet_core::i18n::NameLocalizer::english(),
            &crate::routes::pages::viz::kit::SpeciesSlots::default(),
        )
    }

    /// A station in its `SQLite` store: `seconds` of listening and `n` Blackbirds on each date.
    fn station(days: &[(String, f64, u32)]) -> (tempfile::TempDir, rusqlite::Connection) {
        let dir = tempfile::tempdir().expect("tempdir");
        let conn = birdnet_db::sqlite::open_or_create(&dir.path().join("birds.db")).expect("db");
        for (date, seconds, n) in days {
            conn.execute(
                "INSERT INTO recording_effort (date, source, seconds) VALUES (?1, 'local', ?2)",
                rusqlite::params![date, seconds],
            )
            .expect("effort");
            for i in 0..*n {
                conn.execute(
                    "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
                     VALUES (?1, ?2, 'Turdus merula', 'Eurasian Blackbird', 0.9, ?3)",
                    rusqlite::params![date, format!("06:00:{i:02}"), format!("{date}-{i}.wav")],
                )
                .expect("detection");
            }
        }
        (dir, conn)
    }

    /// 1 Jan to 10 Mar 2024, 70 dates (31 + 29 + 10, a leap year).
    fn early_2024() -> Vec<String> {
        [(1, 31), (2, 29), (3, 10)]
            .iter()
            .flat_map(|&(m, last)| (1..=last).map(move |d| format!("2024-{m:02}-{d:02}")))
            .collect()
    }

    /// RES-26. A whole day of listening (86 400 s) and ten Blackbirds on each of the 70 dates
    /// to 10 Mar 2024, but for the leap day, 61 s and none heard. The typical day is 86 400 and
    /// the floor 43 200, so 69 days of listening, 690 detections, 690 / 69 = 10 a day, and the
    /// 29th is hatched in the calendar. Under the 60-second rule the 29th was a day of
    /// listening that heard nothing: 70 days, 690 / 70 = 9 a day (whole detections).
    #[test]
    fn a_minute_of_audio_is_not_a_day_in_the_year() {
        let days: Vec<_> = early_2024()
            .into_iter()
            .map(|d| {
                if d == "2024-02-29" {
                    (d, 61.0, 0)
                } else {
                    (d, 86_400.0, 10)
                }
            })
            .collect();
        let (_dir, conn) = station(&days);
        let html = page_of(&conn, "2024-03-10");
        assert!(
            html.contains("across 69 days of listening in 2024"),
            "{html}"
        );
        assert!(html.contains("≈ 10 a day"), "{html}");
        assert!(
            html.contains("Thu 29 Feb 2024&#10;The station was not listening"),
            "{html}"
        );
    }

    /// Counterpart: a station on a two-hours-a-day schedule (7 200 s every date) listened on
    /// all 70 of them, and ten a day is 700 / 70 = 10 a day.
    #[test]
    fn a_short_schedule_is_a_year_of_listening() {
        let days: Vec<_> = early_2024().into_iter().map(|d| (d, 7_200.0, 10)).collect();
        let (_dir, conn) = station(&days);
        let html = page_of(&conn, "2024-03-10");
        assert!(
            html.contains("across 70 days of listening in 2024"),
            "{html}"
        );
        assert!(html.contains("≈ 10 a day"), "{html}");
        assert!(!html.contains("The station was not listening"), "{html}");
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
            &[],
            &birdnet_core::i18n::NameLocalizer::english(),
            &crate::routes::pages::viz::kit::SpeciesSlots::default(),
        );
        assert!(html.contains("2026-03-01 · 5 · the loudest"), "{html}");
    }
}
