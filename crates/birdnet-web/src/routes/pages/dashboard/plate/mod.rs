//! The kiosk's plate: today's birds as a page from a field guide.
//!
//! A wall display is the one place the station is looked at from across a room, and for years it
//! showed three numbers and a list. This draws what was heard as a natural-history plate
//! instead: each species an oval, the more it was heard the larger, the Latin name beneath,
//! a numbered key at the foot, the sheet numbered by the day of the year.
//!
//! The idea comes from fugleramme (Arne Giacomo Munthe-Kaas, MIT, `arnegiacomo/fugleramme`): a
//! frame that shows the birds heard as hand-cut nineteenth-century plates on paper, redrawn only
//! when the birds change. This is a new implementation of the idea, in this project's own terms;
//! it shares no code and no artwork with it, and its artwork is not ours to ship (CC BY-SA 4.0,
//! which this project's licence cannot carry). The pictures come from where the rest of the
//! station gets them (the species image cache, or the operator's own directory, which wins), and
//! a photograph is engraved in the browser (`static/plate.js`) rather than shown as a photograph.
//!
//! | Path                                | Purpose |
//! |-------------------------------------|---------|
//! | `GET /kiosk`                        | the page (`?mode=plate\|latest\|numbers`) |
//! | `GET /pages/kiosk-content`          | the plate, or `204` when nothing changed since `h` |
//!
//! `layout` decides where each species goes, `render` writes the sheet as one inline SVG, and
//! this module reads the day from the database and serves both.

mod layout;
mod render;

use std::fmt::Write as _;

use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use serde::Deserialize;

use self::render::{Day, Heard, Input};
use crate::routes::pages::today_date_string;
use crate::state::AppState;

/// What the kiosk shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Everything heard today, sized by how much.
    Plate,
    /// The bird heard last, large.
    Latest,
    /// The old display: three counts and a list.
    Numbers,
}

impl Mode {
    /// The `?mode=` value; anything else is the plate.
    fn parse(s: Option<&str>) -> Self {
        match s {
            Some("latest") => Self::Latest,
            Some("numbers") => Self::Numbers,
            _ => Self::Plate,
        }
    }

    /// The `?mode=` value this one is written as.
    const fn key(self) -> &'static str {
        match self {
            Self::Plate => "plate",
            Self::Latest => "latest",
            Self::Numbers => "numbers",
        }
    }
}

/// The query both routes read.
#[derive(Debug, Deserialize, Default)]
pub(super) struct KioskQuery {
    /// `plate`, `latest` or `numbers`.
    mode: Option<String>,
    /// Width over height of the screen, as the page measured it. Blank is "not measured".
    #[serde(default, deserialize_with = "crate::routes::form::blank_is_none")]
    ar: Option<f64>,
    /// The digest of the plate the page already shows.
    h: Option<String>,
    /// `1` or `0` to force lamplight or daylight; anything else follows the sun.
    night: Option<String>,
    /// `0` to leave the paper's fibres out (they cost a filter pass on a small board).
    paper: Option<String>,
}

/// Whether it is dark where the station is: before first light or after last, by half an hour.
/// Without coordinates, before six or from eight at night.
fn is_night(solar: Option<(f64, f64)>, hour_now: f64) -> bool {
    match solar {
        Some((rise, set)) => hour_now < rise - 0.5 || hour_now > set + 0.5,
        None => !(6.0..20.0).contains(&hour_now),
    }
}

/// One line of the operator's credit for their artwork, from `CREDITS.txt` beside it, if any.
fn read_credit(dir: Option<std::path::PathBuf>) -> Option<String> {
    let text = std::fs::read_to_string(dir?.join("CREDITS.txt")).ok()?;
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    Some(line.chars().take(200).collect())
}

/// The date before `date` (`YYYY-MM-DD`).
fn day_before(date: &str) -> String {
    let days = i64::try_from(crate::routes::pages::date_to_epoch_days(date)).unwrap_or(0);
    birdnet_core::civil::date_string_from_days(days - 1)
}

/// `HH:MM` of a stored `HH:MM:SS`. The plate carries canonical 24-hour times and marks them
/// for `display-format.js`; it never prints the reader's clock itself.
fn hhmm(time: &str) -> String {
    crate::display::clock_minutes(time).unwrap_or_else(|| time.to_owned())
}

/// Whether the plate is lamplight, by `forced` (`?night=`) or else by the sun.
fn night_at(conn: &rusqlite::Connection, forced: Option<bool>) -> bool {
    forced.unwrap_or_else(|| {
        is_night(
            crate::routes::pages::solar_times_local(conn),
            crate::routes::pages::now_hour_local(),
        )
    })
}

/// `?night=1` or `?night=0`; anything else follows the sun.
fn forced_night(query: &KioskQuery) -> Option<bool> {
    match query.night.as_deref() {
        Some("1") => Some(true),
        Some("0") => Some(false),
        _ => None,
    }
}

/// What a request needs from the station, read in one go.
struct Gathered {
    heard: Vec<Heard>,
    day: Day,
    date: String,
    night: bool,
    credit: Option<String>,
}

fn gather(state: &AppState, query: &KioskQuery) -> Result<Gathered, birdnet_db::sqlite::DbError> {
    let names = state.names_handle();
    let forced = forced_night(query);
    let credit = read_credit(state.custom_image_dir().map(std::path::Path::to_path_buf));
    state.with_read_db(|conn| {
        let mut date = today_date_string();
        let mut day = Day::Today;
        let mut rows = birdnet_db::sqlite::species_heard_on(conn, &date)?;
        if rows.is_empty() {
            // The small hours: nothing yet today, and a blank sheet is the wrong welcome.
            let yesterday = day_before(&date);
            let before = birdnet_db::sqlite::species_heard_on(conn, &yesterday)?;
            if !before.is_empty() {
                (date, day, rows) = (yesterday, Day::Yesterday, before);
            }
        }
        let heard = rows
            .into_iter()
            .map(|r| Heard {
                com: names.name(&r.sci_name, &r.com_name).to_string(),
                count: u64::try_from(r.count).unwrap_or(0),
                first: hhmm(&r.first_time),
                last: hhmm(&r.last_time),
                sci: r.sci_name,
            })
            .collect();
        Ok(Gathered {
            heard,
            day,
            date,
            night: night_at(conn, forced),
            credit,
        })
    })
}

/// The page.
pub(super) async fn kiosk_page(
    State(state): State<AppState>,
    Query(query): Query<KioskQuery>,
) -> Html<String> {
    let mode = Mode::parse(query.mode.as_deref());
    if mode == Mode::Numbers {
        return Html(crate::routes::pages::with_asset_version(
            super::kiosk::NUMBERS_HTML.as_str(),
        ));
    }
    let forced = forced_night(&query);
    let night = state
        .run_blocking(move |s| s.with_read_db(|conn| night_at(conn, forced)))
        .await;
    // The poll must carry what the address did, or the first swap undoes it: the sheet would go
    // back to the sun's choice of paper however the page was asked.
    let mut overrides = String::new();
    if let Some(night) = forced {
        let _ = write!(
            overrides,
            r#"<input type="hidden" name="night" value="{}">"#,
            u8::from(night)
        );
    }
    if query.paper.as_deref() == Some("0") {
        overrides.push_str(r#"<input type="hidden" name="paper" value="0">"#);
    }
    let page = PLATE_HTML
        .replace("{{overrides}}", &overrides)
        .replace("{{mode}}", mode.key())
        .replace("{{night}}", if night { "1" } else { "0" })
        .replace(
            "{{current_plate}}",
            if mode == Mode::Plate { "true" } else { "false" },
        )
        .replace(
            "{{current_latest}}",
            if mode == Mode::Latest {
                "true"
            } else {
                "false"
            },
        );
    Html(crate::routes::pages::with_asset_version(&page))
}

/// The content: the plate, nothing when it has not changed, or the old numbers.
pub(super) async fn kiosk_content(
    State(state): State<AppState>,
    Query(query): Query<KioskQuery>,
) -> Response {
    let mode = Mode::parse(query.mode.as_deref());
    if mode == Mode::Numbers {
        return super::kiosk::numbers_content(State(state))
            .await
            .into_response();
    }
    let asked = query.h.clone().unwrap_or_default();
    let result = state
        .run_blocking(move |s| {
            gather(s, &query).map(|g| {
                let aspect = query.ar.filter(|a| a.is_finite()).unwrap_or(1.6);
                let input = Input {
                    heard: &g.heard,
                    day: g.day,
                    date: &g.date,
                    night: g.night,
                    aspect,
                    seed: render::day_of_year(&g.date),
                    latest_only: mode == Mode::Latest,
                    credit: g.credit.as_deref(),
                    paper: query.paper.as_deref() != Some("0"),
                };
                let digest = render::digest(&input);
                // The page asks with the digest of what it shows; if nothing changed there is
                // nothing to send, and the wall display does not redraw (or re-engrave) a thing.
                if digest == asked {
                    String::new()
                } else {
                    render::render(&input)
                }
            })
        })
        .await;
    match result {
        Ok(html) if html.is_empty() => StatusCode::NO_CONTENT.into_response(),
        Ok(html) => (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html).into_response(),
        // 200, not 500: htmx discards a 5xx body, so a 500 would leave the wall showing the last
        // good plate for ever. The error state has to be swapped in.
        Err(e) => {
            tracing::warn!(error = %e, "kiosk plate: query failed");
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/html")],
                crate::routes::pages::error_states::inline("what the station has heard"),
            )
                .into_response()
        }
    }
}

/// The plate page. The plate itself arrives by the `kiosk-content` poll; the form carries what
/// the server needs to draw it for this screen.
static PLATE_HTML: crate::seal::Sealed = crate::seal::Sealed::new(
    r##"<!DOCTYPE html>
<html lang="en" data-theme="dark">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>BirdNet-Behavior · Kiosk</title>
<link rel="stylesheet" href="/static/css/app.css?v={{version}}">
<link rel="stylesheet" href="/static/css/plate.css?v={{version}}">
</head>
<body class="plate-page" data-night="{{night}}">
<!-- A wall display is still a document: a heading for it, and its two controls in a landmark. -->
<header>
  <h1 class="sr-only">The birds heard at this station</h1>
  <a class="plate-ui plate-exit" href="/" aria-label="Exit kiosk mode">Exit&nbsp;✕</a>
  <nav class="plate-ui plate-modes" aria-label="What the display shows">
    <a href="/kiosk?mode=plate" aria-current="{{current_plate}}">Plate</a>
    <a href="/kiosk?mode=latest" aria-current="{{current_latest}}">Latest</a>
    <a href="/kiosk?mode=numbers">Numbers</a>
  </nav>
</header>
<main id="kiosk-content"
      hx-get="/pages/kiosk-content"
      hx-include="#kiosk-params"
      hx-trigger="load, every 30s, bnb:reshape"
      hx-swap="innerHTML">
  <p class="bnb-meta" role="status">Loading…</p>
</main>
<form id="kiosk-params" hidden>
  <input type="hidden" name="mode" value="{{mode}}">
  <input type="hidden" name="ar" value="1.6" id="kp-ar">
  <input type="hidden" name="h" value="" id="kp-h">{{overrides}}
</form>
<script src="/static/htmx.min.js?v={{version}}"></script>
<script src="/static/display-format.js?v={{version}}" defer></script>
<script src="/static/plate.js?v={{version}}" defer></script>
<script>
(function () {
  var ar = document.getElementById('kp-ar'), h = document.getElementById('kp-h');
  function shape() {
    ar.value = Math.max(0.5, Math.min(2.4, window.innerWidth / Math.max(1, window.innerHeight))).toFixed(2);
  }
  shape();
  var t;
  window.addEventListener('resize', function () {
    clearTimeout(t);
    t = setTimeout(function () {
      shape();
      h.value = '';
      document.getElementById('kiosk-content').dispatchEvent(new CustomEvent('bnb:reshape'));
    }, 400);
  });
  // What the page shows is what the next poll is compared with, and the sheet's day or night
  // is the page's too, so the margins match the paper.
  document.body.addEventListener('htmx:afterSwap', function () {
    var s = document.querySelector('svg.plate-svg');
    if (!s) return;
    h.value = s.getAttribute('data-hash') || '';
    document.body.setAttribute('data-night', s.getAttribute('data-night') || '0');
  });
  document.addEventListener('keydown', function (e) {
    if (e.key === 'Escape') { window.location.href = (document.body.getAttribute('data-base-path') || '') + '/'; }
  });
})();
</script>
</body>
</html>"##,
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mode_is_the_plate_unless_told_otherwise() {
        assert_eq!(Mode::parse(None), Mode::Plate);
        assert_eq!(Mode::parse(Some("latest")), Mode::Latest);
        assert_eq!(Mode::parse(Some("numbers")), Mode::Numbers);
        assert_eq!(Mode::parse(Some("<script>")), Mode::Plate);
        for m in [Mode::Plate, Mode::Latest, Mode::Numbers] {
            assert_eq!(Mode::parse(Some(m.key())), m);
        }
    }

    #[test]
    fn night_follows_the_sun_by_half_an_hour() {
        let sun = Some((6.75, 16.7));
        assert!(is_night(sun, 3.0));
        assert!(is_night(sun, 6.2), "before first light");
        assert!(!is_night(sun, 6.3), "first light");
        assert!(!is_night(sun, 17.1));
        assert!(is_night(sun, 17.3), "after dusk");
    }

    #[test]
    fn without_coordinates_night_is_the_small_hours_and_the_evening() {
        assert!(is_night(None, 2.0));
        assert!(!is_night(None, 12.0));
        assert!(is_night(None, 21.0));
    }

    #[test]
    fn a_credit_is_the_first_nonblank_line_and_bounded() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("CREDITS.txt"),
            "\n  John Gould, Birds of Europe (1832–37), public domain  \nsecond line\n",
        )
        .unwrap();
        assert_eq!(
            read_credit(Some(dir.path().to_path_buf())).as_deref(),
            Some("John Gould, Birds of Europe (1832–37), public domain")
        );
        std::fs::write(dir.path().join("CREDITS.txt"), "x".repeat(900)).unwrap();
        assert_eq!(
            read_credit(Some(dir.path().to_path_buf()))
                .unwrap()
                .chars()
                .count(),
            200
        );
        assert_eq!(read_credit(None), None);
        assert_eq!(read_credit(Some(dir.path().join("nowhere"))), None);
    }

    #[test]
    fn the_page_names_its_assets_and_has_no_inline_style() {
        let page = PLATE_HTML.as_str();
        assert!(page.contains("/static/css/plate.css"));
        assert!(page.contains("/static/plate.js"));
        assert!(page.contains(r##"hx-include="#kiosk-params""##));
        assert!(
            !page.contains(" style="),
            "the CSP forbids style attributes"
        );
    }
}
