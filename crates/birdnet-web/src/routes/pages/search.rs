//! The searchable detection log.
//!
//! # Routes
//!
//! | Method | Path                   | Description                          |
//! |--------|------------------------|--------------------------------------|
//! | GET    | `/search`              | The search page: filter form + results |
//! | GET    | `/pages/search-results`| HTMX partial: the matching rows        |
//! | POST   | `/pages/search-bulk`   | Apply one action to the selected rows  |
//!
//! # Why a page rather than more tabs on Today
//!
//! The Today log answers "what happened today", and its four category
//! shortcuts — all, rare, first, high — are the questions a person asks while
//! looking at one day. Everything else an operator wants is a *query*: every
//! rejected record from May, this species below 40 % confidence, whatever the
//! pond microphone heard between 22:00 and 04:00. Those do not fit on a tab
//! strip, and pretending otherwise is how a filter UI ends up with eleven of
//! them.
//!
//! The nine criteria all live in
//! [`birdnet_db::sqlite::DetectionFilter`], which composes them
//! into one statement; this module is the translation from a query string to
//! that type, plus the HTML.
//!
//! # Bulk actions
//!
//! Reviewing a season of detections one row at a time is not review, it is
//! attrition — so the results carry checkboxes and one action bar. The endpoint
//! is in [`mutating_router`] and therefore behind the admin gate, which is not
//! incidental: `POST /pages/search-bulk` with `action=delete` is the single most
//! destructive request this application accepts, and it used to be the case that
//! its per-row equivalents needed no login at all.

use std::fmt::Write as _;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse};
use axum::{Router, routing::get};
use birdnet_db::sqlite::{
    DateRange, DetectionFilter, HourWindow, LockFilter, SortOrder, TodayFilter, VerdictFilter,
};
use serde::Deserialize;

use super::escape_html;
use crate::state::AppState;

/// Rows shown per page. Generous because the list is the point of the screen,
/// and bounded because each row carries a spectrogram thumbnail.
const PAGE_SIZE: u32 = 50;

/// The most rows one bulk action may touch.
///
/// A ceiling, not a page size: the action bar operates on what is *selected*,
/// and "select all" on a station with a five-year history would otherwise post
/// a request that deletes a hundred thousand records in one transaction. Two
/// pages' worth is more than anybody selects deliberately.
const BULK_LIMIT: usize = 100;

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// The read-only half: the page and its results partial.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/search", get(search_page))
        .route("/pages/search-results", get(search_results_partial))
}

/// The state-changing half, mounted behind the admin auth middleware.
pub fn mutating_router() -> Router<AppState> {
    Router::new().route("/pages/search-bulk", axum::routing::post(bulk_action))
}

// ---------------------------------------------------------------------------
// Query parameters
// ---------------------------------------------------------------------------

/// The search form, as it arrives on the query string.
///
/// Every field is optional and every field is a `String`: a search URL is
/// something people edit by hand and share, and a typo in one parameter should
/// narrow the search wrongly rather than fail the request with a 422. The
/// translation in [`SearchParams::to_filter`] is where a malformed value becomes
/// "no restriction".
#[derive(Debug, Default, Deserialize)]
pub struct SearchParams {
    /// Free text over common and scientific name; a leading `NOT ` inverts.
    pub q: Option<String>,
    /// A single species, exactly.
    pub species: Option<String>,
    /// Inclusive start date, `YYYY-MM-DD`.
    pub from: Option<String>,
    /// Inclusive end date, `YYYY-MM-DD`.
    pub to: Option<String>,
    /// First hour of day to include, `0`–`23`.
    pub hour_from: Option<String>,
    /// Last hour of day to include, `0`–`23`. May be less than `hour_from`,
    /// which asks for a window through midnight.
    pub hour_to: Option<String>,
    /// Lowest confidence, as a percentage (`0`–`100`) because that is what the
    /// UI shows everywhere else.
    pub conf_min: Option<String>,
    /// Highest confidence, as a percentage.
    pub conf_max: Option<String>,
    /// Audio source label.
    pub source: Option<String>,
    /// `confirmed` · `rejected` · `unreviewed`.
    pub verdict: Option<String>,
    /// `locked` · `unlocked`.
    pub locked: Option<String>,
    /// One of the Today log's four category shortcuts.
    pub category: Option<String>,
    /// Sort token — see [`SortOrder::from_token`].
    pub sort: Option<String>,
    /// Pagination offset.
    pub offset: Option<u32>,
}

/// Trim, and treat blank as absent.
///
/// An empty form field posts as `""`, not as an absent parameter, so without
/// this every unfilled box would be a filter matching nothing.
fn present(v: Option<&String>) -> Option<&str> {
    v.map(|s| s.trim()).filter(|s| !s.is_empty())
}

/// Parse an hour, rejecting anything outside `0..=23`.
fn hour(v: Option<&String>) -> Option<u8> {
    present(v)?.parse::<u8>().ok().filter(|h| *h <= 23)
}

/// Parse a percentage into a `0.0..=1.0` confidence.
///
/// The UI talks in percent because every other confidence display in the
/// application does; the database stores a fraction. Values outside the range
/// are dropped rather than clamped — `conf_min=900` is a typo, and silently
/// reading it as 100 % would return nothing with no indication why.
fn percent(v: Option<&String>) -> Option<f64> {
    let raw: f64 = present(v)?.parse().ok()?;
    (0.0..=100.0).contains(&raw).then(|| raw / 100.0)
}

impl SearchParams {
    /// Translate the query string into a database filter.
    ///
    /// Pure, and the only interesting part of this module: everything that can
    /// be got wrong about a search URL is got wrong here or not at all.
    #[must_use]
    pub fn to_filter(&self) -> DetectionFilter {
        let dates = match (present(self.from.as_ref()), present(self.to.as_ref())) {
            (Some(a), Some(b)) => DateRange::between(a, b),
            // One end alone is still a range, not a single day: "everything
            // since the 1st" is a question people ask, and treating a lone
            // `from` as `On` would answer a different one silently.
            (Some(a), None) => DateRange::between(a, "9999-12-31"),
            (None, Some(b)) => DateRange::between("0000-01-01", b),
            (None, None) => DateRange::Any,
        };

        let hours = match (hour(self.hour_from.as_ref()), hour(self.hour_to.as_ref())) {
            (Some(a), Some(b)) => Some(HourWindow::new(a, b)),
            // A single end means "from then to the end of the day" / "from
            // midnight until then", which is what a half-filled pair of boxes
            // reads as.
            (Some(a), None) => Some(HourWindow::new(a, 23)),
            (None, Some(b)) => Some(HourWindow::new(0, b)),
            (None, None) => None,
        };

        DetectionFilter {
            text: present(self.q.as_ref()).map(str::to_owned),
            species: present(self.species.as_ref())
                .map(str::to_owned)
                .into_iter()
                .collect(),
            dates,
            hours,
            min_confidence: percent(self.conf_min.as_ref()),
            max_confidence: percent(self.conf_max.as_ref()),
            sources: present(self.source.as_ref())
                .map(str::to_owned)
                .into_iter()
                .collect(),
            verdict: match present(self.verdict.as_ref()) {
                Some("confirmed") => VerdictFilter::Confirmed,
                Some("rejected") => VerdictFilter::Rejected,
                Some("unreviewed") => VerdictFilter::Unreviewed,
                _ => VerdictFilter::Any,
            },
            locked: match present(self.locked.as_ref()) {
                Some("locked") => LockFilter::Locked,
                Some("unlocked") => LockFilter::Unlocked,
                _ => LockFilter::Any,
            },
            category: TodayFilter::from_token(present(self.category.as_ref())),
            sort: SortOrder::from_token(present(self.sort.as_ref())),
        }
    }

    /// The query string that reproduces this search, minus the offset.
    ///
    /// Used for the "load more" link and for keeping the URL shareable. Built
    /// from the same fields the filter reads, so a criterion cannot be
    /// applied-but-not-carried — which is how paging silently drops a filter on
    /// page two.
    #[must_use]
    pub fn to_query_string(&self) -> String {
        let mut out = String::new();
        for (key, value) in [
            ("q", self.q.as_ref()),
            ("species", self.species.as_ref()),
            ("from", self.from.as_ref()),
            ("to", self.to.as_ref()),
            ("hour_from", self.hour_from.as_ref()),
            ("hour_to", self.hour_to.as_ref()),
            ("conf_min", self.conf_min.as_ref()),
            ("conf_max", self.conf_max.as_ref()),
            ("source", self.source.as_ref()),
            ("verdict", self.verdict.as_ref()),
            ("locked", self.locked.as_ref()),
            ("category", self.category.as_ref()),
            ("sort", self.sort.as_ref()),
        ] {
            if let Some(v) = present(value) {
                let _ = write!(out, "&{key}={}", super::simple_url_encode(v));
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// The search page.
async fn search_page(
    State(state): State<AppState>,
    Query(params): Query<SearchParams>,
    headers: HeaderMap,
) -> Html<String> {
    let sources = tokio::task::spawn_blocking({
        let state = state.clone();
        move || {
            state
                .with_read_db(birdnet_db::sqlite::known_sources)
                .unwrap_or_default()
        }
    })
    .await
    .unwrap_or_default();

    let content = render_page(&params, &sources);
    super::render_page_for_request("Search detections", &content, "search", &headers)
}

/// HTMX partial: the matching rows, a count, and a "load more" control.
async fn search_results_partial(
    State(state): State<AppState>,
    Query(params): Query<SearchParams>,
) -> impl IntoResponse {
    let filter = params.to_filter();
    let offset = params.offset.unwrap_or(0);

    let result = tokio::task::spawn_blocking(move || {
        state.with_read_db(|conn| {
            let rows = birdnet_db::sqlite::search_detections(conn, &filter, PAGE_SIZE, offset)?;
            let total = birdnet_db::sqlite::search_detection_count(conn, &filter)?;
            Ok::<_, birdnet_db::sqlite::DbError>((rows, total))
        })
    })
    .await;

    let html = match result {
        Ok(Ok((rows, total))) => render_results(&rows, total, offset, &params),
        _ => "<p class=\"sr-error\">Could not run that search.</p>".to_string(),
    };
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

/// One bulk action over the selected rows.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct BulkForm {
    /// `delete` · `lock` · `unlock` · `confirm` · `reject`.
    pub action: String,
    /// The selected rows, each `date|time|sci_name` — the same triple the
    /// per-row forms use to identify a detection.
    pub selected: Vec<String>,
}

/// Parse an `application/x-www-form-urlencoded` body into [`BulkForm`].
///
/// Hand-parsed rather than `axum::Form`, and not by preference. A page of
/// checkboxes posts `selected=a&selected=b&selected=c` — one key, repeated —
/// which is what the HTML form specification says a checkbox group is.
/// `axum::Form` deserialises through `serde_urlencoded`, which has no
/// representation for a repeated key and rejects the body outright:
///
/// ```text
/// Failed to deserialize form body: selected: invalid type: string "…",
/// expected a sequence
/// ```
///
/// That was found by posting a real form to a running server, not by any test
/// — every unit test around this code constructed the struct directly and so
/// never went near the deserialiser. `form_urlencoded::parse` is the browser's
/// own grammar and handles repetition by construction; it is already in the
/// tree through `url`.
#[must_use]
pub fn parse_bulk_form(body: &[u8]) -> BulkForm {
    let mut form = BulkForm::default();
    for (key, value) in form_urlencoded::parse(body) {
        match key.as_ref() {
            "action" => form.action = value.into_owned(),
            "selected" => form.selected.push(value.into_owned()),
            _ => {}
        }
    }
    form
}

/// A detection identified by the triple the UI carries.
#[derive(Debug, PartialEq, Eq)]
pub struct RowKey {
    /// Local date, `YYYY-MM-DD`.
    pub date: String,
    /// Local time, `HH:MM:SS`.
    pub time: String,
    /// Scientific name.
    pub sci_name: String,
}

/// Parse the `date|time|sci_name` triple the checkboxes carry.
///
/// `splitn(3, …)` rather than `split`: a scientific name cannot contain `|`
/// today, but a future label set is not this function's to promise, and
/// silently dropping everything after a second separator would delete the wrong
/// row rather than none.
#[must_use]
pub fn parse_row_key(raw: &str) -> Option<RowKey> {
    let mut it = raw.splitn(3, '|');
    let date = it.next()?.trim();
    let time = it.next()?.trim();
    let sci_name = it.next()?.trim();
    if date.is_empty() || time.is_empty() || sci_name.is_empty() {
        return None;
    }
    Some(RowKey {
        date: date.to_owned(),
        time: time.to_owned(),
        sci_name: sci_name.to_owned(),
    })
}

/// What a bulk action does to one row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BulkAction {
    /// Remove the detection.
    Delete,
    /// Pin its clip against the disk purge.
    Lock,
    /// Unpin it.
    Unlock,
    /// Record a confirming review verdict.
    Confirm,
    /// Record a rejecting review verdict.
    Reject,
}

impl BulkAction {
    /// Parse the form token. Unknown actions are refused rather than defaulted:
    /// a typo must not become a delete.
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        match token.trim() {
            "delete" => Some(Self::Delete),
            "lock" => Some(Self::Lock),
            "unlock" => Some(Self::Unlock),
            "confirm" => Some(Self::Confirm),
            "reject" => Some(Self::Reject),
            _ => None,
        }
    }

    /// Past-tense verb for the toast.
    const fn past(self) -> &'static str {
        match self {
            Self::Delete => "deleted",
            Self::Lock => "locked",
            Self::Unlock => "unlocked",
            Self::Confirm => "confirmed",
            Self::Reject => "rejected",
        }
    }
}

async fn bulk_action(
    State(state): State<AppState>,
    axum::extract::RawForm(body): axum::extract::RawForm,
) -> impl IntoResponse {
    let form = parse_bulk_form(&body);
    let Some(action) = BulkAction::parse(&form.action) else {
        return super::toast::Toast::error("That action is not one this page offers.").render();
    };

    let keys: Vec<RowKey> = form
        .selected
        .iter()
        .filter_map(|s| parse_row_key(s))
        .take(BULK_LIMIT)
        .collect();

    if keys.is_empty() {
        return super::toast::Toast::info("Nothing was selected.").render();
    }
    let requested = keys.len();

    let applied = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let mut done = 0_usize;
            for k in &keys {
                let ok = match action {
                    BulkAction::Delete => {
                        birdnet_db::sqlite::delete_detection(conn, &k.date, &k.time, &k.sci_name)?
                    }
                    BulkAction::Lock => {
                        birdnet_db::sqlite::lock_detection(conn, &k.date, &k.time, &k.sci_name)?
                    }
                    BulkAction::Unlock => {
                        birdnet_db::sqlite::unlock_detection(conn, &k.date, &k.time, &k.sci_name)?
                    }
                    BulkAction::Confirm | BulkAction::Reject => {
                        // The common name comes from the row, not from the
                        // checkbox: `set_detection_review` stores it for
                        // display, and a review naming a different bird from
                        // the detection it reviews is a quiet corruption of the
                        // curation record. The lookup doubles as the existence
                        // check — a row somebody else already deleted is a
                        // skip, not an error.
                        match birdnet_db::sqlite::com_name_for(conn, &k.date, &k.time, &k.sci_name)?
                        {
                            Some(com_name) => {
                                let status = if action == BulkAction::Confirm {
                                    birdnet_db::sqlite::ReviewStatus::Confirmed
                                } else {
                                    birdnet_db::sqlite::ReviewStatus::Rejected
                                };
                                birdnet_db::sqlite::set_detection_review(
                                    conn,
                                    &k.date,
                                    &k.time,
                                    &k.sci_name,
                                    &com_name,
                                    status,
                                    None,
                                )?;
                                true
                            }
                            None => false,
                        }
                    }
                };
                if ok {
                    done += 1;
                }
            }
            Ok::<_, birdnet_db::sqlite::DbError>(done)
        })
    })
    .await;

    match applied {
        Ok(Ok(done)) => {
            // Reporting what was *asked* alongside what happened, because a
            // silent shortfall — a row somebody else already deleted — reads as
            // the action having failed.
            let verb = action.past();
            let msg = if done == requested {
                format!("{done} detections {verb}.")
            } else {
                format!("{done} of {requested} detections {verb}; the rest were already gone.")
            };
            super::toast::Toast::success(&msg).render()
        }
        _ => super::toast::Toast::error("That bulk action could not be completed.").render(),
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// One `<option>`, marked selected when it matches.
fn option(value: &str, label: &str, current: Option<&str>) -> String {
    let sel = if current == Some(value) {
        " selected"
    } else {
        ""
    };
    format!(
        "<option value=\"{}\"{sel}>{}</option>",
        escape_html(value),
        escape_html(label)
    )
}

/// The filter form plus the results container.
fn render_page(p: &SearchParams, sources: &[String]) -> String {
    let val = |v: Option<&String>| present(v).map(escape_html).unwrap_or_default();
    let (q, species, from, to) = (
        val(p.q.as_ref()),
        val(p.species.as_ref()),
        val(p.from.as_ref()),
        val(p.to.as_ref()),
    );
    let (hf, ht) = (val(p.hour_from.as_ref()), val(p.hour_to.as_ref()));
    let (cmin, cmax) = (val(p.conf_min.as_ref()), val(p.conf_max.as_ref()));

    let verdict_opts = ["", "confirmed", "rejected", "unreviewed"]
        .iter()
        .zip(["Any verdict", "Confirmed", "Rejected", "Not yet reviewed"])
        .map(|(v, l)| option(v, l, present(p.verdict.as_ref())))
        .collect::<String>();
    let lock_opts = ["", "locked", "unlocked"]
        .iter()
        .zip(["Locked or not", "Locked", "Not locked"])
        .map(|(v, l)| option(v, l, present(p.locked.as_ref())))
        .collect::<String>();
    let sort_opts = [
        ("newest", "Newest first"),
        ("oldest", "Oldest first"),
        ("confidence", "Most confident"),
        ("confidence-asc", "Least confident"),
        ("species", "Species A–Z"),
        ("species-desc", "Species Z–A"),
    ]
    .iter()
    .map(|(v, l)| option(v, l, present(p.sort.as_ref())))
    .collect::<String>();
    let source_opts = std::iter::once(option("", "Any source", present(p.source.as_ref())))
        .chain(
            sources
                .iter()
                .map(|s| option(s, s, present(p.source.as_ref()))),
        )
        .collect::<String>();

    let initial_query = p.to_query_string();

    format!(
        "<div class=\"sr-head\">\
  <h1 class=\"sr-h1\">Search detections</h1>\
  <p class=\"sr-lede\">Every record this station has kept, narrowed by any \
   combination below. The address bar carries the search, so a useful one can be \
   bookmarked or sent to somebody.</p>\
</div>\
<form class=\"sr-form\" id=\"sr-form\" \
      hx-get=\"/pages/search-results\" hx-target=\"#sr-results\" \
      hx-trigger=\"submit, change delay:250ms from:.sr-live\" \
      hx-push-url=\"true\" hx-swap=\"innerHTML\">\
  <div class=\"sr-row\">\
    <label class=\"sr-field sr-grow\"><span>Name contains</span>\
      <input class=\"sr-live\" type=\"search\" name=\"q\" value=\"{q}\" \
             placeholder=\"robin — or NOT crow\"></label>\
    <label class=\"sr-field\"><span>Exact species</span>\
      <input class=\"sr-live\" type=\"text\" name=\"species\" value=\"{species}\" \
             placeholder=\"Erithacus rubecula\"></label>\
  </div>\
  <div class=\"sr-row\">\
    <label class=\"sr-field\"><span>From</span>\
      <input class=\"sr-live\" type=\"date\" name=\"from\" value=\"{from}\"></label>\
    <label class=\"sr-field\"><span>To</span>\
      <input class=\"sr-live\" type=\"date\" name=\"to\" value=\"{to}\"></label>\
    <label class=\"sr-field sr-narrow\"><span>Hour from</span>\
      <input class=\"sr-live\" type=\"number\" name=\"hour_from\" min=\"0\" max=\"23\" \
             value=\"{hf}\"></label>\
    <label class=\"sr-field sr-narrow\"><span>Hour to</span>\
      <input class=\"sr-live\" type=\"number\" name=\"hour_to\" min=\"0\" max=\"23\" \
             value=\"{ht}\"></label>\
  </div>\
  <div class=\"sr-row\">\
    <label class=\"sr-field sr-narrow\"><span>Confidence &ge; %</span>\
      <input class=\"sr-live\" type=\"number\" name=\"conf_min\" min=\"0\" max=\"100\" \
             value=\"{cmin}\"></label>\
    <label class=\"sr-field sr-narrow\"><span>Confidence &le; %</span>\
      <input class=\"sr-live\" type=\"number\" name=\"conf_max\" min=\"0\" max=\"100\" \
             value=\"{cmax}\"></label>\
    <label class=\"sr-field\"><span>Source</span>\
      <select class=\"sr-live\" name=\"source\">{source_opts}</select></label>\
    <label class=\"sr-field\"><span>Review</span>\
      <select class=\"sr-live\" name=\"verdict\">{verdict_opts}</select></label>\
    <label class=\"sr-field\"><span>Clip</span>\
      <select class=\"sr-live\" name=\"locked\">{lock_opts}</select></label>\
    <label class=\"sr-field\"><span>Order</span>\
      <select class=\"sr-live\" name=\"sort\">{sort_opts}</select></label>\
  </div>\
  <div class=\"sr-actions\">\
    <button type=\"submit\" class=\"sr-btn sr-btn-primary\">Search</button>\
    <a href=\"/search\" class=\"sr-btn\">Clear</a>\
  </div>\
</form>\
<div id=\"sr-results\" hx-get=\"/pages/search-results?offset=0{initial_query}\" \
     hx-trigger=\"load\" hx-swap=\"innerHTML\">\
  <p class=\"sr-loading\">Searching\u{2026}</p>\
</div>\
<script src=\"/static/search-select.js\" defer></script>"
    )
}

/// The results list: a count, the rows with checkboxes, an action bar, paging.
fn render_results(
    rows: &[birdnet_db::sqlite::DetectionRow],
    total: i64,
    offset: u32,
    params: &SearchParams,
) -> String {
    if rows.is_empty() {
        return "<p class=\"sr-empty\">Nothing matched. Widen a filter, or \
                <a href=\"/search\">clear them all</a>.</p>"
            .to_string();
    }

    let mut html = String::with_capacity(8192);
    // `i64` throughout because `total` is a SQL `COUNT(*)`. Saturating rather
    // than casting: a station with more detections than fit in a `u32` offset
    // is not a real case, and a wrap here would print a negative page range.
    let shown_to = i64::from(offset).saturating_add(i64::try_from(rows.len()).unwrap_or(i64::MAX));
    let _ = write!(
        html,
        "<p class=\"sr-count\">Showing {}\u{2013}{shown_to} of {total}</p>",
        i64::from(offset) + 1
    );

    html.push_str(
        "<form class=\"sr-bulk\" hx-post=\"/pages/search-bulk\" hx-target=\"#toast-region\" \
         hx-swap=\"innerHTML\">\
         <div class=\"sr-bulkbar\">\
           <label class=\"sr-selall\"><input type=\"checkbox\" \
             data-sr-toggle-all=\"1\"> Select all on this page</label>\
           <select name=\"action\" class=\"sr-bulkaction\">\
             <option value=\"confirm\">Confirm</option>\
             <option value=\"reject\">Reject</option>\
             <option value=\"lock\">Lock clip</option>\
             <option value=\"unlock\">Unlock clip</option>\
             <option value=\"delete\">Delete</option>\
           </select>\
           <button type=\"submit\" class=\"sr-btn\" \
             data-confirm=\"Apply this action to every selected detection?\">Apply</button>\
         </div>\
         <ul class=\"sr-list\">",
    );

    for d in rows {
        let key = format!("{}|{}|{}", d.date, d.time, d.sci_name);
        let conf = (d.confidence * 100.0).round();
        let verdict = match d.review_verdict.as_deref() {
            Some("confirmed") => "<span class=\"sr-verdict sr-confirmed\">confirmed</span>",
            Some("rejected") => "<span class=\"sr-verdict sr-rejected\">rejected</span>",
            _ => "",
        };
        let source = d
            .source
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| format!("<span class=\"sr-source\">{}</span>", escape_html(s)))
            .unwrap_or_default();
        let _ = write!(
            html,
            "<li class=\"sr-item\">\
               <label class=\"sr-check\"><input type=\"checkbox\" name=\"selected\" \
                 value=\"{key}\"></label>\
               <a class=\"sr-name\" href=\"/species/detail?name={name_q}\">{name}</a>\
               <span class=\"sr-when\">{date} {time}</span>\
               <span class=\"sr-conf\">{conf:.0}%</span>\
               {source}{verdict}\
             </li>",
            key = escape_html(&key),
            name_q = super::simple_url_encode(&d.com_name),
            name = escape_html(&d.com_name),
            date = escape_html(&d.date),
            time = escape_html(&d.time),
        );
    }
    html.push_str("</ul></form>");

    let next = shown_to;
    if next < total {
        let query = params.to_query_string();
        let remaining = total - next;
        let _ = write!(
            html,
            "<div class=\"sr-more\">\
               <button class=\"sr-btn\" hx-get=\"/pages/search-results?offset={next}{query}\" \
                 hx-target=\"#sr-results\" hx-swap=\"innerHTML\">\
                 Next {PAGE_SIZE} ({remaining} more)</button></div>"
        );
    }
    html
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(pairs: &[(&str, &str)]) -> SearchParams {
        let mut p = SearchParams::default();
        for (k, v) in pairs {
            let v = Some((*v).to_string());
            match *k {
                "q" => p.q = v,
                "species" => p.species = v,
                "from" => p.from = v,
                "to" => p.to = v,
                "hour_from" => p.hour_from = v,
                "hour_to" => p.hour_to = v,
                "conf_min" => p.conf_min = v,
                "conf_max" => p.conf_max = v,
                "source" => p.source = v,
                "verdict" => p.verdict = v,
                "locked" => p.locked = v,
                "category" => p.category = v,
                "sort" => p.sort = v,
                other => panic!("unknown test parameter {other}"),
            }
        }
        p
    }

    #[test]
    fn an_empty_query_string_is_an_unfiltered_search() {
        assert!(SearchParams::default().to_filter().is_unfiltered());
    }

    #[test]
    fn blank_form_fields_are_not_filters() {
        // Every unfilled box posts as "", so without the blank check the first
        // search anybody runs would match nothing.
        let p = params(&[
            ("q", "   "),
            ("species", ""),
            ("from", ""),
            ("source", ""),
            ("verdict", ""),
        ]);
        assert!(
            p.to_filter().is_unfiltered(),
            "an empty form must search everything, not nothing"
        );
    }

    #[test]
    fn confidence_is_entered_as_percent_and_stored_as_a_fraction() {
        let f = params(&[("conf_min", "40"), ("conf_max", "95")]).to_filter();
        assert_eq!(f.min_confidence, Some(0.4));
        assert_eq!(f.max_confidence, Some(0.95));
    }

    #[test]
    fn an_impossible_percentage_is_dropped_rather_than_clamped() {
        // `conf_min=900` is a typo. Clamping it to 100 % returns nothing and
        // looks like the station has no detections.
        let f = params(&[("conf_min", "900")]).to_filter();
        assert_eq!(f.min_confidence, None);
        let f = params(&[("conf_min", "-5")]).to_filter();
        assert_eq!(f.min_confidence, None);
        let f = params(&[("conf_min", "banana")]).to_filter();
        assert_eq!(f.min_confidence, None);
    }

    #[test]
    fn an_out_of_range_hour_is_dropped() {
        assert_eq!(params(&[("hour_from", "24")]).to_filter().hours, None);
        assert_eq!(params(&[("hour_from", "-1")]).to_filter().hours, None);
        assert_eq!(
            params(&[("hour_from", "23")]).to_filter().hours,
            Some(HourWindow::new(23, 23))
        );
    }

    #[test]
    fn one_open_ended_date_is_still_a_range() {
        let f = params(&[("from", "2026-05-01")]).to_filter();
        assert_eq!(
            f.dates,
            DateRange::Between("2026-05-01".into(), "9999-12-31".into()),
            "'everything since the 1st' must not silently become 'only the 1st'"
        );
        let f = params(&[("to", "2026-05-09")]).to_filter();
        assert_eq!(
            f.dates,
            DateRange::Between("0000-01-01".into(), "2026-05-09".into())
        );
    }

    #[test]
    fn one_open_ended_hour_runs_to_the_edge_of_the_day() {
        assert_eq!(
            params(&[("hour_from", "20")]).to_filter().hours,
            Some(HourWindow::new(20, 23))
        );
        assert_eq!(
            params(&[("hour_to", "4")]).to_filter().hours,
            Some(HourWindow::new(0, 4))
        );
    }

    #[test]
    fn a_night_window_survives_the_translation() {
        let h = params(&[("hour_from", "22"), ("hour_to", "4")])
            .to_filter()
            .hours
            .expect("a window");
        assert!(h.wraps(), "22→4 must stay a wrapping window");
    }

    #[test]
    fn every_verdict_and_lock_token_maps_and_junk_does_not() {
        for (token, want) in [
            ("confirmed", VerdictFilter::Confirmed),
            ("rejected", VerdictFilter::Rejected),
            ("unreviewed", VerdictFilter::Unreviewed),
            ("nonsense", VerdictFilter::Any),
        ] {
            assert_eq!(params(&[("verdict", token)]).to_filter().verdict, want);
        }
        for (token, want) in [
            ("locked", LockFilter::Locked),
            ("unlocked", LockFilter::Unlocked),
            ("nonsense", LockFilter::Any),
        ] {
            assert_eq!(params(&[("locked", token)]).to_filter().locked, want);
        }
    }

    #[test]
    fn the_query_string_carries_every_criterion_the_filter_reads() {
        // The gate against paging silently dropping a filter on page two: if a
        // criterion is applied but not carried, "next 50" returns a different
        // search's results.
        let p = params(&[
            ("q", "robin"),
            ("species", "Turdus merula"),
            ("from", "2026-05-01"),
            ("to", "2026-05-09"),
            ("hour_from", "5"),
            ("hour_to", "9"),
            ("conf_min", "40"),
            ("conf_max", "95"),
            ("source", "MIC_1"),
            ("verdict", "confirmed"),
            ("locked", "locked"),
            ("category", "rare"),
            ("sort", "oldest"),
        ]);
        let qs = p.to_query_string();
        for key in [
            "q",
            "species",
            "from",
            "to",
            "hour_from",
            "hour_to",
            "conf_min",
            "conf_max",
            "source",
            "verdict",
            "locked",
            "category",
            "sort",
        ] {
            assert!(
                qs.contains(&format!("&{key}=")),
                "{key} is applied to the filter but not carried in the paging URL: {qs}"
            );
        }
        assert!(!p.to_filter().is_unfiltered());
    }

    #[test]
    fn the_query_string_omits_what_was_not_set() {
        assert_eq!(SearchParams::default().to_query_string(), "");
        let qs = params(&[("q", "robin")]).to_query_string();
        assert_eq!(qs, "&q=robin");
    }

    #[test]
    fn the_query_string_encodes_values() {
        let qs = params(&[("species", "Turdus merula")]).to_query_string();
        assert!(
            !qs.contains(' '),
            "an unencoded space breaks the URL for every two-word species: {qs}"
        );
    }

    #[test]
    fn a_row_key_round_trips() {
        let k = parse_row_key("2026-05-01|06:30:00|Turdus merula").expect("parses");
        assert_eq!(
            k,
            RowKey {
                date: "2026-05-01".into(),
                time: "06:30:00".into(),
                sci_name: "Turdus merula".into(),
            }
        );
    }

    #[test]
    fn a_row_key_keeps_everything_after_the_second_separator() {
        // `split` rather than `splitn(3, …)` would truncate the name here and
        // act on a different row — or, worse, on none while reporting success.
        let k = parse_row_key("2026-05-01|06:30:00|Genus species|odd").expect("parses");
        assert_eq!(k.sci_name, "Genus species|odd");
    }

    #[test]
    fn a_malformed_row_key_is_refused() {
        for bad in [
            "",
            "2026-05-01",
            "2026-05-01|06:30:00",
            "||",
            "a||c",
            "|b|c",
        ] {
            assert!(parse_row_key(bad).is_none(), "{bad:?} should not parse");
        }
    }

    #[test]
    fn a_checkbox_group_posts_one_repeated_key_and_all_of_it_survives() {
        // The bug this exists for: a page of checkboxes posts
        // `selected=a&selected=b`, and `axum::Form` (serde_urlencoded) rejects
        // the whole body because it has no representation for a repeated key.
        // Every other test here builds `BulkForm` directly and cannot see that.
        let form = parse_bulk_form(
            b"action=confirm&selected=2026-05-01%7C06%3A30%3A00%7CTurdus+merula\
              &selected=2026-05-02%7C07%3A00%3A00%7CParus+major",
        );
        assert_eq!(form.action, "confirm");
        assert_eq!(
            form.selected,
            vec![
                "2026-05-01|06:30:00|Turdus merula".to_string(),
                "2026-05-02|07:00:00|Parus major".to_string(),
            ],
            "a repeated key must yield every value, percent- and plus-decoded"
        );
    }

    #[test]
    fn a_single_selection_is_still_a_list() {
        let form = parse_bulk_form(b"action=delete&selected=a%7Cb%7Cc");
        assert_eq!(form.selected, vec!["a|b|c".to_string()]);
    }

    #[test]
    fn a_body_with_no_selection_parses_to_an_empty_list() {
        let form = parse_bulk_form(b"action=delete");
        assert_eq!(form.action, "delete");
        assert!(form.selected.is_empty());
        // And an unknown field is ignored rather than failing the whole body,
        // so a stray CSRF or pagination input cannot break the action.
        let form = parse_bulk_form(b"action=lock&page=2&selected=a%7Cb%7Cc");
        assert_eq!(form.action, "lock");
        assert_eq!(form.selected.len(), 1);
    }

    #[test]
    fn an_unknown_bulk_action_is_refused_rather_than_defaulted() {
        assert_eq!(BulkAction::parse("delete"), Some(BulkAction::Delete));
        assert_eq!(BulkAction::parse("confirm"), Some(BulkAction::Confirm));
        // Built by truncation rather than written out: the literal is a
        // misspelling of `delete`, which is exactly the point of the test and
        // exactly what the repository's spell-check gate flags. Slicing says
        // "one character short of `delete`" more clearly than the literal did.
        assert_eq!(
            BulkAction::parse(&"delete"[.."delete".len() - 1]),
            None,
            "a near-miss must not fall through to any action, least of all delete"
        );
        assert_eq!(BulkAction::parse(""), None);
    }

    #[test]
    fn the_rendered_form_escapes_what_the_operator_typed() {
        let p = params(&[("q", "<script>alert(1)</script>")]);
        let html = render_page(&p, &[]);
        assert!(!html.contains("<script>alert"), "unescaped input: {html}");
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn a_source_name_is_escaped_into_its_option() {
        let html = render_page(&SearchParams::default(), &["a\"><b".to_string()]);
        assert!(!html.contains("a\"><b"), "unescaped source name: {html}");
    }
}
