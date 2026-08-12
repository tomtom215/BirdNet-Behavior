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

use std::collections::{HashMap, HashSet};
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

    let data = fetch_clips(state, filter, search.clone(), 0).await;

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

    let list = render_clips_block(&data, filter, search.as_deref(), 0);

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

    let data = fetch_clips(&state, filter, search.clone(), offset).await;
    let html = render_clips_block(&data, filter, search.as_deref(), offset);
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

/// Pagination / filter context for [`clips_partial`].
#[derive(Debug, Default, Deserialize)]
struct ClipsPartialParams {
    filter: Option<String>,
    q: Option<String>,
    offset: Option<u32>,
}

/// One page of clip rows plus the per-page lookups the renderer needs.
#[derive(Default)]
struct ClipsData {
    rows: Vec<DetectionRow>,
    total: i64,
    /// Basenames of clips locked against auto-purge.
    locked: HashSet<String>,
    /// First-ever date per scientific name (the "first today" / "rare" badge).
    first_seen: HashMap<String, String>,
    /// Basenames present in the recording dir — the rows whose audio still
    /// exists, so the grid links a spectrogram thumbnail only for those (the
    /// same gate the play button effectively has). Loaded once per page like
    /// `locked`, so there is no per-row filesystem stat.
    present: HashSet<String>,
}

/// Run the clip query, count, and per-page lookups on the blocking pool.
async fn fetch_clips(
    state: &AppState,
    filter: RecordingsFilter,
    search: Option<String>,
    offset: u32,
) -> ClipsData {
    let state = state.clone();
    let present = recording_basenames(&state.recording_dir());
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
            // First-ever date per species → the "first today" / "rare" badge,
            // the same first-seen signal the Today feed-row uses.
            let first_seen = birdnet_db::sqlite::species_first_detection(conn).unwrap_or_default();
            ClipsData {
                rows,
                total,
                locked,
                first_seen,
                present,
            }
        })
    })
    .await
    .unwrap_or_default()
}

/// Collect the set of file basenames present in the recording directory.
///
/// One `read_dir` per page (the directory holds only the bounded, purge-managed
/// set of source recordings), mirroring how the locked-file set is loaded once
/// per page rather than stat-ing each row.
fn recording_basenames(dir: &Path) -> HashSet<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return HashSet::new();
    };
    entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .collect()
}

/// Render one page of clip rows plus the "Show more" control when more remain.
/// Shared by the first render and each HTMX page so the two never drift.
fn render_clips_block(
    data: &ClipsData,
    filter: RecordingsFilter,
    search: Option<&str>,
    offset: u32,
) -> String {
    let rows = &data.rows;
    let total = data.total;
    let today = super::today_date_string();
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
        render_clip_row(&mut html, d, data, &today);
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
/// Format a saved clip's length for the grid — `9.2` → `"0:09"`,
/// `75.0` → `"1:15"` (the `M:SS` an audio player would show).
#[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
fn fmt_clip_duration(secs: f64) -> String {
    let total = secs.round().max(0.0) as u64;
    format!("{}:{:02}", total / 60, total % 60)
}

fn render_clip_row(html: &mut String, d: &DetectionRow, page: &ClipsData, today: &str) {
    let file = d.file_name.as_deref().unwrap_or_default();
    let base = base_name(file);
    let is_locked = !base.is_empty() && page.locked.contains(&base);
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

    // The saved clip's length (migration 20), shown under the time. Omitted —
    // not faked — for rows with no recorded duration (historical / imported).
    let dur = d
        .duration_secs
        .filter(|s| *s > 0.0)
        .map(|s| format!(r#"<span class="rc-dur">{}</span>"#, fmt_clip_duration(s)))
        .unwrap_or_default();

    // "first ever" / "rare" badge — the same exact-instant signal the Today
    // feed-row shows: this clip *is* the species' first-ever detection, either
    // today or on a past day being browsed. Keyed on the first-ever instant,
    // not the first-ever date, so a species heard many times on its first day
    // marks only the recording that actually came first.
    let badge = page
        .first_seen
        .get(&d.sci_name)
        .map_or(String::new(), |fs| {
            if *fs != format!("{} {}", d.date, d.time) {
                String::new()
            } else if d.date == today {
                r#" <span class="bnb-pill moss rc-badge">first ever</span>"#.to_string()
            } else {
                r#" <span class="bnb-pill rare rc-badge">rare</span>"#.to_string()
            }
        });

    // Spectrogram thumbnail — the small `?thumb=1` preview from the shared
    // `/api/v2/spectrogram/{file}` endpoint (same renderer + cache the detail
    // view uses, so the tile matches the full image). Linked only when the
    // clip's audio is present (the same gate playback effectively has); absent
    // rows get an empty aligned spacer rather than a broken image or a faked tile.
    let spectro = if !base.is_empty() && page.present.contains(&base) {
        format!(
            r#"<img class="rc-spectro" width="104" height="31" loading="lazy" decoding="async" src="/api/v2/spectrogram/{safe_file}?thumb=1" alt="Spectrogram of {com_name}">"#
        )
    } else {
        r#"<span class="rc-spectro rc-spectro-empty" aria-hidden="true"></span>"#.to_string()
    };

    let _ = write!(
        html,
        r#"<div class="rc-row" data-key="{key}">
  <span class="rc-check" role="checkbox" aria-checked="false" tabindex="0" aria-label="Select {com_name}"></span>
  <span class="rc-time">{time}<span class="d">{date}</span>{dur}</span>
  {spectro}
  <span class="rc-who">{av}<span class="rc-who-text"><a class="nm" href="/species/detail?name={enc_name}">{com_name}</a>{badge}<span class="sc">{sci_name}</span></span></span>
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

    /// Build a [`ClipsData`] for renderer tests with the given rows/total and
    /// optional first-seen and present-audio sets.
    fn data_with(
        rows: Vec<DetectionRow>,
        total: i64,
        first_seen: HashMap<String, String>,
        present: HashSet<String>,
    ) -> ClipsData {
        ClipsData {
            rows,
            total,
            locked: HashSet::new(),
            first_seen,
            present,
        }
    }

    #[test]
    fn empty_first_page_messages_differ_by_context() {
        let unfiltered = render_clips_block(
            &data_with(vec![], 0, HashMap::new(), HashSet::new()),
            RecordingsFilter::All,
            None,
            0,
        );
        assert!(unfiltered.contains("No saved clips yet"));
        let filtered = render_clips_block(
            &data_with(vec![], 0, HashMap::new(), HashSet::new()),
            RecordingsFilter::Rare,
            None,
            0,
        );
        assert!(filtered.contains("No clips match"));
        // A later empty page is silent (just the end of the list).
        assert!(
            render_clips_block(
                &data_with(vec![], 100, HashMap::new(), HashSet::new()),
                RecordingsFilter::All,
                None,
                24,
            )
            .is_empty()
        );
    }

    fn clip_row(duration_secs: Option<f64>) -> DetectionRow {
        DetectionRow {
            date: "2026-06-13".into(),
            time: "06:12:00".into(),
            sci_name: "Turdus migratorius".into(),
            com_name: "American Robin".into(),
            confidence: 0.91,
            lat: None,
            lon: None,
            cutoff: None,
            week: None,
            sens: None,
            overlap: None,
            file_name: Some("robin.wav".into()),
            correlation_id: None,
            source: None,
            duration_secs,
        }
    }

    #[test]
    fn fmt_clip_duration_formats_as_m_ss() {
        assert_eq!(fmt_clip_duration(9.2), "0:09");
        assert_eq!(fmt_clip_duration(75.0), "1:15");
        assert_eq!(fmt_clip_duration(0.0), "0:00");
        assert_eq!(fmt_clip_duration(125.6), "2:06");
    }

    #[test]
    fn clip_row_shows_duration_when_present_and_omits_when_absent() {
        let mut with = String::new();
        render_clip_row(
            &mut with,
            &clip_row(Some(9.2)),
            &data_with(vec![], 0, HashMap::new(), HashSet::new()),
            "2026-06-13",
        );
        assert!(with.contains(r#"<span class="rc-dur">0:09</span>"#));
        // None → no rc-dur element (historical / imported rows aren't faked).
        let mut without = String::new();
        render_clip_row(
            &mut without,
            &clip_row(None),
            &data_with(vec![], 0, HashMap::new(), HashSet::new()),
            "2026-06-13",
        );
        assert!(!without.contains("rc-dur"));
        // A zero/unknown length is also omitted, never rendered as "0:00".
        let mut zero = String::new();
        render_clip_row(
            &mut zero,
            &clip_row(Some(0.0)),
            &data_with(vec![], 0, HashMap::new(), HashSet::new()),
            "2026-06-13",
        );
        assert!(!zero.contains("rc-dur"));
    }

    #[test]
    fn clip_row_badges_only_the_first_ever_recording() {
        let sci = "Turdus migratorius".to_string(); // clip_row()'s species
        // clip_row() is 2026-06-13 06:12:00.
        let this_row = "2026-06-13 06:12:00".to_string();

        // This clip *is* the species' first-ever, and it is today → "first ever".
        let mut first = HashMap::new();
        first.insert(sci.clone(), this_row.clone());
        let mut a = String::new();
        render_clip_row(
            &mut a,
            &clip_row(Some(9.0)),
            &data_with(vec![], 0, first, HashSet::new()),
            "2026-06-13",
        );
        assert!(a.contains(r#"<span class="bnb-pill moss rc-badge">first ever</span>"#));

        // The regression this replaces: the species' first-ever was earlier the
        // SAME day, so the old date-only comparison badged this clip too — and
        // every other clip of that species that day. A station that heard 133
        // blackcaps on their arrival day marked all 133 as the first.
        let mut same_day = HashMap::new();
        same_day.insert(sci.clone(), "2026-06-13 05:01:00".to_string());
        let mut dup = String::new();
        render_clip_row(
            &mut dup,
            &clip_row(Some(9.0)),
            &data_with(vec![], 0, same_day, HashSet::new()),
            "2026-06-13",
        );
        assert!(
            !dup.contains("rc-badge"),
            "a later clip on the first-ever day must not claim to be the first: {dup}"
        );

        // The first-ever clip, browsed on a later day → "rare".
        let mut rare = HashMap::new();
        rare.insert(sci.clone(), this_row);
        let mut b = String::new();
        render_clip_row(
            &mut b,
            &clip_row(Some(9.0)),
            &data_with(vec![], 0, rare, HashSet::new()),
            "2026-06-20",
        );
        assert!(b.contains(r#"<span class="bnb-pill rare rc-badge">rare</span>"#));

        // First heard on an earlier date → no badge.
        let mut old = HashMap::new();
        old.insert(sci, "2025-01-01 07:00:00".to_string());
        let mut c = String::new();
        render_clip_row(
            &mut c,
            &clip_row(Some(9.0)),
            &data_with(vec![], 0, old, HashSet::new()),
            "2026-06-20",
        );
        assert!(!c.contains("rc-badge"));
    }

    #[test]
    fn clip_row_links_spectrogram_only_when_audio_present() {
        // Audio present in the recording dir → a lazy-loaded thumbnail <img>
        // pointing at the per-clip spectrogram route, with alt text.
        let present: HashSet<String> = HashSet::from(["robin.wav".to_string()]);
        let mut shown = String::new();
        render_clip_row(
            &mut shown,
            &clip_row(Some(9.0)),
            &data_with(vec![], 0, HashMap::new(), present),
            "2026-06-13",
        );
        assert!(
            shown.contains(r#"src="/api/v2/spectrogram/robin.wav?thumb=1""#),
            "expected the shared spectrogram endpoint thumb src; got: {shown}"
        );
        assert!(shown.contains(r#"class="rc-spectro""#));
        assert!(shown.contains(r#"loading="lazy""#));
        assert!(shown.contains(r#"alt="Spectrogram of American Robin""#));
        assert!(!shown.contains("rc-spectro-empty"));

        // Audio absent → an empty aligned spacer, never a broken <img> or a
        // faked tile.
        let mut absent = String::new();
        render_clip_row(
            &mut absent,
            &clip_row(Some(9.0)),
            &data_with(vec![], 0, HashMap::new(), HashSet::new()),
            "2026-06-13",
        );
        assert!(absent.contains("rc-spectro-empty"));
        assert!(!absent.contains("/api/v2/spectrogram/"));
        assert!(!absent.contains("<img"));
    }
}
