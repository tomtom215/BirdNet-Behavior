//! Recordings home — "let me hear them" (v3 spine).
//!
//! Folds the two pre-spine audio pages into one home with a sub-tab switch on
//! `?view=`:
//!
//! * **Clips** (`/recordings`, `?view=clips`) — a flat, newest-first browser
//!   of every detection that saved an audio file ([`recent_clips`]). Each row
//!   plays through the shared clip player, can be locked against the disk
//!   purge or deleted, and a Select mode drives the bulk bar over those same
//!   per-item endpoints. Filter chips (All · Best · Rare · Locked) and the
//!   search box are real links / a GET form, so every view is bookmarkable and
//!   the page renders server-side on a Pi.
//! * **Live** (`?view=live`) — the folded `/listen` surface: the honest
//!   scrolling sonogram (real spectrogram WebSocket frames, a flat idle
//!   baseline when nothing is arriving — never a fake waveform), a per-source
//!   audio picker, and the live-detection trickle.
//!
//! The pre-spine `/listen`, `/livestream` and `/live` paths permanently
//! redirect here (see [`crate::routes::redirects`]); the live-audio source
//! picker still owns its `<option>` set in the `super::listen` module.
//!
//! [`recent_clips`]: birdnet_db::sqlite::recent_clips

use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::Path;

use axum::extract::{Form, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse};
use axum::{Router, routing::get};
use serde::Deserialize;

use birdnet_db::audio_sources::AudioSourceStore;
use birdnet_db::sqlite::{DetectionRow, RecordingsFilter};

use super::atoms::{avatar, conf_bar};
use super::homes::{SubTab, resolve_tab, subtabs};
use super::{escape_html, render_page_for_request, simple_url_encode};
use crate::state::AppState;

/// The page shell + behaviour scripts; `{{content}}` is the server-rendered
/// view (page-head + sub-tabs + body).
const PAGE_HTML: &str = include_str!("../../../templates/recordings.html");

/// The two Recordings views, in tab order.
const VIEWS: &[SubTab] = &[
    SubTab {
        key: "clips",
        label: "Clips",
        question: "browse & play",
    },
    SubTab {
        key: "live",
        label: "Live",
        question: "listen right now",
    },
];

/// Clip rows fetched per page (initial render and each "Show more").
const CLIP_PAGE: u32 = 24;

/// Mount the Recordings home and its HTMX partial / action routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/recordings", get(recordings_page))
        .route("/pages/recordings-clips", get(clips_partial))
        .route("/pages/recordings-lock", axum::routing::post(lock_clip))
        .route("/pages/recordings-unlock", axum::routing::post(unlock_clip))
        .route("/pages/recordings-delete", axum::routing::post(delete_clip))
}

/// Query string for the home: which view, and (Clips only) the active filter,
/// search term and live-audio source.
#[derive(Debug, Default, Deserialize)]
struct RecordingsParams {
    view: Option<String>,
    filter: Option<String>,
    q: Option<String>,
    source: Option<String>,
}

async fn recordings_page(
    State(state): State<AppState>,
    Query(params): Query<RecordingsParams>,
    headers: HeaderMap,
) -> Html<String> {
    let tab = resolve_tab(VIEWS, params.view.as_deref());
    let nav = subtabs("/recordings", "view", VIEWS, tab.key);

    let body = if tab.key == "live" {
        live_view(&state, params.source.as_deref()).await
    } else {
        clips_view(&state, params.filter.as_deref(), params.q.as_deref()).await
    };

    let help = super::help::help_link(super::help::Topic::Recordings);
    let content = format!(
        r#"<div class="page-head rc-head" data-screen-label="Recordings head">
  <div class="rc-head-main">
    <div class="bnb-eyebrow"><span>Recordings</span>{help}</div>
    <h1 class="display rc-h1">Listen in</h1>
    <p class="bnb-meta rc-sub">Hear what your station caught — browse the clips, or open the live stream and listen along in real time.</p>
  </div>
  <div id="rc-headplayer" class="rc-hp-slot" hidden></div>
</div>
{nav}
<div id="rc-body" data-screen-label="Recordings body">{body}</div>"#
    );

    let page = PAGE_HTML.replace("{{content}}", &content);
    render_page_for_request("Recordings", &page, "recordings", &headers)
}

// ── Clips view ─────────────────────────────────────────────────────────────

/// The Clips browser: lede, controls (filter chips · search · Select), the
/// hidden bulk bar, and the first page of clip rows with a "Show more" gate.
async fn clips_view(state: &AppState, filter: Option<&str>, search: Option<&str>) -> String {
    let filter = RecordingsFilter::from_token(filter);
    let search = search
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let (rows, total, locked) = fetch_clips(state, filter, search.clone(), 0).await;

    let lede = "<p class=\"bnb-lede\"><b>Every detection your station saved a clip for.</b> \
        Play it, lock the keepers so the disk purge can't reclaim them, or switch on Select to \
        review a batch at once.</p>";

    let chips = filter_chips(filter, search.as_deref());
    let search_val = search.as_deref().map(escape_html).unwrap_or_default();
    let controls = format!(
        r#"<div class="rc-controls">
  {chips}
  <form class="rc-search" method="get" action="/recordings" role="search">
    <span class="ico" aria-hidden="true">⌕</span>
    <input type="hidden" name="view" value="clips">
    <input type="hidden" name="filter" value="{filter_tok}">
    <input type="search" name="q" value="{search_val}" placeholder="Find a species…" aria-label="Find a species">
  </form>
  <button type="button" class="bnb-btn ghost rc-sel-btn" id="rc-selmode" aria-pressed="false">Select</button>
</div>
<div class="rc-selbar" id="rc-selbar" role="region" aria-label="Bulk actions">
  <span class="n"><span id="rc-selcount">0</span> selected</span>
  <span class="sp">
    <button type="button" id="rc-bulk-lock">🔒 Lock</button>
    <button type="button" id="rc-bulk-dl">⬇ Download</button>
    <button type="button" id="rc-bulk-del">✕ Delete</button>
  </span>
</div>"#,
        filter_tok = filter.as_token(),
    );

    let list = render_clips_block(&rows, &locked, filter, search.as_deref(), 0, total);

    format!(r#"{lede}{controls}<div class="bnb-card rc-list" id="rc-clips">{list}</div>"#)
}

/// HTMX partial: the next page of clip rows (the "Show more" target).
async fn clips_partial(
    State(state): State<AppState>,
    Query(params): Query<ClipsPartialParams>,
) -> impl IntoResponse {
    let filter = RecordingsFilter::from_token(params.filter.as_deref());
    let search = params
        .q
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let offset = params.offset.unwrap_or(0);

    let (rows, total, locked) = fetch_clips(&state, filter, search.clone(), offset).await;
    let html = render_clips_block(&rows, &locked, filter, search.as_deref(), offset, total);
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

/// Pagination / filter context for [`clips_partial`].
#[derive(Debug, Default, Deserialize)]
struct ClipsPartialParams {
    filter: Option<String>,
    q: Option<String>,
    offset: Option<u32>,
}

/// Run the clip query, count, and locked-file lookup on the blocking pool.
async fn fetch_clips(
    state: &AppState,
    filter: RecordingsFilter,
    search: Option<String>,
    offset: u32,
) -> (Vec<DetectionRow>, i64, HashSet<String>) {
    let state = state.clone();
    tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let rows = birdnet_db::sqlite::recent_clips(
                conn,
                filter,
                search.as_deref(),
                CLIP_PAGE,
                offset,
            )
            .unwrap_or_default();
            let total = birdnet_db::sqlite::recent_clips_count(conn, filter, search.as_deref())
                .unwrap_or(0);
            let locked = birdnet_db::sqlite::locked_file_names(conn)
                .unwrap_or_default()
                .iter()
                .map(|f| base_name(f))
                .collect::<HashSet<String>>();
            (rows, total, locked)
        })
    })
    .await
    .unwrap_or_default()
}

/// Render one page of clip rows plus the "Show more" control when more remain.
/// Shared by the first render and each HTMX page so the two never drift.
fn render_clips_block(
    rows: &[DetectionRow],
    locked: &HashSet<String>,
    filter: RecordingsFilter,
    search: Option<&str>,
    offset: u32,
    total: i64,
) -> String {
    if rows.is_empty() {
        // An empty *first* page distinguishes "no clips yet" from "filter has
        // no matches"; a later empty page just means we reached the end.
        if offset == 0 {
            let msg = match (filter, search) {
                (RecordingsFilter::All, None) => {
                    "No saved clips yet — recordings appear here as your station detects birds."
                }
                _ => "No clips match this filter.",
            };
            return format!(r#"<div class="rc-empty">{msg}</div>"#);
        }
        return String::new();
    }

    let mut html = String::with_capacity(rows.len() * 512);
    for d in rows {
        render_clip_row(&mut html, d, locked);
    }

    let shown = offset + u32::try_from(rows.len()).unwrap_or(0);
    if i64::from(shown) < total {
        let search_param = search
            .map(|s| format!("&q={}", simple_url_encode(s)))
            .unwrap_or_default();
        let _ = write!(
            html,
            r#"<button class="bnb-btn ghost rc-more" hx-get="/pages/recordings-clips?filter={tok}&offset={shown}{search_param}" hx-target="this" hx-swap="outerHTML">Show more ({remaining})</button>"#,
            tok = filter.as_token(),
            remaining = total - i64::from(shown),
        );
    }
    html
}

/// Render a single clip row (checkbox · time · who · confidence · actions).
fn render_clip_row(html: &mut String, d: &DetectionRow, locked: &HashSet<String>) {
    let file = d.file_name.as_deref().unwrap_or_default();
    let base = base_name(file);
    let is_locked = !base.is_empty() && locked.contains(&base);
    let safe_file = escape_html(&base);

    let enc_name = simple_url_encode(&d.com_name);
    let com_name = escape_html(&d.com_name);
    let sci_name = escape_html(&d.sci_name);
    let time = escape_html(&d.time);
    let date = escape_html(&d.date);
    let key = escape_html(&format!("{}|{}|{}", d.date, d.time, d.sci_name));
    let meta = format!("{} · {} · {:.2}", d.time, d.date, d.confidence);
    let meta = escape_html(&meta);

    let av = avatar(&d.com_name, "");
    let conf = conf_bar(d.confidence);
    let lock = lock_button(&d.date, &d.time, &d.sci_name, is_locked);

    let date_raw = escape_html(&d.date);
    let time_raw = escape_html(&d.time);
    let sci_raw = escape_html(&d.sci_name);

    let _ = write!(
        html,
        r#"<div class="rc-row" data-key="{key}">
  <span class="rc-check" role="checkbox" aria-checked="false" tabindex="0" aria-label="Select {com_name}"></span>
  <span class="rc-time">{time}<span class="d">{date}</span></span>
  <span class="rc-who">{av}<span class="rc-who-text"><a class="nm" href="/species/detail?name={enc_name}">{com_name}</a><span class="sc">{sci_name}</span></span></span>
  <span class="rc-conf">{conf}</span>
  <span class="rc-acts">
    <button type="button" class="x-fplay rc-play" data-play-src="/api/v2/recordings/{safe_file}" data-clip-name="{com_name}" data-clip-meta="{meta}" title="Play clip" aria-label="Play {com_name}">▶</button>
    <a class="rc-iact" href="/api/v2/recordings/{safe_file}" download title="Download clip" aria-label="Download {com_name}">↓</a>
    {lock}
    <button type="button" class="rc-iact rc-del" hx-post="/pages/recordings-delete" hx-vals='{{"date":"{date_raw}","time":"{time_raw}","sci_name":"{sci_raw}"}}' hx-target="closest .rc-row" hx-swap="outerHTML" hx-confirm="Delete this clip of {com_name}?" data-confirm-action="hx-post" data-confirm-url="/pages/recordings-delete" data-confirm-title="Delete clip" data-confirm-body="Delete this clip of {com_name}?" data-confirm-confirm-label="Delete" data-confirm-style="danger" title="Delete clip" aria-label="Delete {com_name}">✕</button>
  </span>
</div>"#,
    );
}

/// The lock/unlock toggle button for a clip. Swaps itself out (`outerHTML`)
/// for the opposite state after the POST, so the row reflects the new state
/// without a full-list reload.
fn lock_button(date: &str, time: &str, sci: &str, locked: bool) -> String {
    let date_raw = escape_html(date);
    let time_raw = escape_html(time);
    let sci_raw = escape_html(sci);
    let (endpoint, glyph, cls, title) = if locked {
        (
            "/pages/recordings-unlock",
            "🔒",
            "rc-iact rc-lock on",
            "Locked — click to allow auto-purge",
        )
    } else {
        (
            "/pages/recordings-lock",
            "🔓",
            "rc-iact rc-lock",
            "Lock — protect from auto-purge",
        )
    };
    format!(
        r#"<button type="button" class="{cls}" hx-post="{endpoint}" hx-vals='{{"date":"{date_raw}","time":"{time_raw}","sci_name":"{sci_raw}"}}' hx-swap="outerHTML" title="{title}" aria-label="{title}">{glyph}</button>"#,
    )
}

/// The four filter chips as bookmarkable links (the active one carries
/// `aria-current`). "All" is the canonical bare path; the rest add `?filter=`.
fn filter_chips(active: RecordingsFilter, search: Option<&str>) -> String {
    let search_param = search
        .map(|s| format!("&q={}", simple_url_encode(s)))
        .unwrap_or_default();
    let mut out = String::from(r#"<div class="rc-chips">"#);
    for (filter, label) in [
        (RecordingsFilter::All, "All clips"),
        (RecordingsFilter::Best, "Best"),
        (RecordingsFilter::Rare, "Rare"),
        (RecordingsFilter::Locked, "Locked"),
    ] {
        let href = if matches!(filter, RecordingsFilter::All) {
            format!("/recordings?view=clips{search_param}")
        } else {
            format!(
                "/recordings?view=clips&filter={}{search_param}",
                filter.as_token()
            )
        };
        let (cls, cur) = if filter == active {
            (" active", r#" aria-current="true""#)
        } else {
            ("", "")
        };
        let _ = write!(
            out,
            r#"<a class="rc-chip{cls}" href="{href}"{cur}>{label}</a>"#
        );
    }
    out.push_str("</div>");
    out
}

/// File path → bare file name (the form `/api/v2/recordings/{name}` and the
/// locked-file set both key on).
fn base_name(file: &str) -> String {
    Path::new(file)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

// ── Live view ──────────────────────────────────────────────────────────────

/// The folded `/listen` surface, reskinned to the `rc-live` treatment: the
/// honest streaming sonogram, a source picker, and the live-detection trickle.
async fn live_view(state: &AppState, source: Option<&str>) -> String {
    let state_q = state.clone();
    let sources = tokio::task::spawn_blocking(move || {
        state_q.with_db(|conn| AudioSourceStore::list(conn).unwrap_or_default())
    })
    .await
    .unwrap_or_default();

    let options = super::listen::source_options_for(&sources, source);
    let trickle_skel = super::skeletons::feed_rows(6);

    format!(
        r#"<p class="bnb-lede"><b>Listen along with your station, live.</b> Pick a source and watch the spectrogram scroll as audio arrives — detections appear in the trickle below the moment they're classified. The signal is honest: a flat line when nothing is coming in, never a fake waveform.</p>
<div class="rc-live">
  <div class="rc-live-head">
    <span class="bnb-eyebrow">Live spectrogram · 48 kHz · 128 mels</span>
    <span class="bnb-pill" id="rc-live-status"><span class="bnb-dot"></span> idle</span>
  </div>
  <canvas id="rc-spectrogram" height="200" aria-label="Live spectrogram"></canvas>
  <div class="rc-live-foot">
    <span class="rc-src">
      <span class="bnb-meta rc-src-label">source</span>
      <select id="rc-source" aria-label="Audio source">{options}</select>
    </span>
    <span class="mono bnb-meta" id="rc-frames">— frames</span>
    <button type="button" id="rc-listen-btn" class="bnb-btn rc-listen-btn"><span id="rc-listen-glyph" aria-hidden="true">▶</span> <span id="rc-listen-label">Listen (audio)</span></button>
    <audio id="rc-audio" preload="none" class="rc-audio-hidden"></audio>
  </div>
</div>
<div class="rc-trickle">
  <div class="section-header">
    <div><div class="bnb-eyebrow">As it happens</div><h3>Live detections</h3></div>
    <a class="action" href="/">Full feed →</a>
  </div>
  <div id="rc-trickle-feed" class="feed" hx-get="/pages/detections" hx-trigger="load, every 10s" hx-swap="innerHTML" aria-live="polite">{trickle_skel}</div>
</div>"#
    )
}

// ── Mutating endpoints (lock · unlock · delete) ────────────────────────────

/// `date`/`time`/`sci_name` triple identifying a clip for a lock/delete action.
#[derive(Debug, Deserialize)]
struct ClipAction {
    date: String,
    time: String,
    sci_name: String,
}

async fn lock_clip(State(state): State<AppState>, Form(f): Form<ClipAction>) -> impl IntoResponse {
    // The toggled button is the same whether or not a row matched, so render it
    // before moving the key into the blocking lock.
    let button = lock_button(&f.date, &f.time, &f.sci_name, true);
    let _ = tokio::task::spawn_blocking(move || {
        state
            .with_db(|conn| birdnet_db::sqlite::lock_detection(conn, &f.date, &f.time, &f.sci_name))
    })
    .await;
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html")],
        button,
    )
}

async fn unlock_clip(
    State(state): State<AppState>,
    Form(f): Form<ClipAction>,
) -> impl IntoResponse {
    let button = lock_button(&f.date, &f.time, &f.sci_name, false);
    let _ = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            birdnet_db::sqlite::unlock_detection(conn, &f.date, &f.time, &f.sci_name)
        })
    })
    .await;
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html")],
        button,
    )
}

async fn delete_clip(
    State(state): State<AppState>,
    Form(form): Form<ClipAction>,
) -> impl IntoResponse {
    let _ = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            birdnet_db::sqlite::delete_detection(conn, &form.date, &form.time, &form.sci_name)
        })
    })
    .await;
    // Empty body → the row's `hx-swap="outerHTML"` removes it from the list.
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html")],
        String::new(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_name_strips_directories() {
        assert_eq!(base_name("/var/clips/2026/a.wav"), "a.wav");
        assert_eq!(base_name("b.wav"), "b.wav");
        assert_eq!(base_name(""), "");
    }

    #[test]
    fn filter_chips_mark_active_and_keep_search() {
        let html = filter_chips(RecordingsFilter::Rare, Some("wren"));
        assert_eq!(html.matches("rc-chip active").count(), 1);
        assert!(html.contains(r#"aria-current="true""#));
        // The active "Rare" chip and the others all carry the search term.
        assert!(html.contains("filter=rare&q=wren"));
        assert!(html.contains("filter=best&q=wren"));
        // "All" is the canonical bare view (no filter token).
        assert!(html.contains("/recordings?view=clips&q=wren"));
    }

    #[test]
    fn lock_button_toggles_endpoint_and_glyph() {
        let unlocked = lock_button("2026-06-13", "06:00:00", "Parus major", false);
        assert!(unlocked.contains("/pages/recordings-lock"));
        assert!(unlocked.contains('🔓'));
        let locked = lock_button("2026-06-13", "06:00:00", "Parus major", true);
        assert!(locked.contains("/pages/recordings-unlock"));
        assert!(locked.contains('🔒'));
    }

    #[test]
    fn empty_first_page_messages_differ_by_context() {
        let none = HashSet::new();
        let unfiltered = render_clips_block(&[], &none, RecordingsFilter::All, None, 0, 0);
        assert!(unfiltered.contains("No saved clips yet"));
        let filtered = render_clips_block(&[], &none, RecordingsFilter::Rare, None, 0, 0);
        assert!(filtered.contains("No clips match"));
        // A later empty page is silent (just the end of the list).
        assert!(render_clips_block(&[], &none, RecordingsFilter::All, None, 24, 100).is_empty());
    }
}
