# O-19 · Command palette (⌘K / Ctrl-K)

<!-- BNB:STATUS-HEADER -->
> **Risk:** none · **Priority:** 3 · **Status:** ready for review
> Acceptance: VERIFY.md § O-19 · Rollback: ROLLBACK.md § O-19
<!-- BNB:STATUS-HEADER -->


## What

A keyboard-first navigator overlaid on any page. ⌘K (mac) / Ctrl-K (everywhere else) opens it; `/` does too when no input is focused. Types into one input, results stream in via HTMX from `/pages/cmdk?q=…` grouped by source:

- **Pages** — every section from the topnav, plus a curated set of deep links ("Yesterday's detections", "Last week's report", "Quarantine queue").
- **Species** — fuzzy-matched against the species table (common name + scientific + 4-letter banding code).
- **Dates** — natural strings (`today`, `yesterday`, `last sunday`, `2025-04`) that resolve to `/history?date=…` or `/today?date=…`.
- **Settings** — every settings sub-page indexed by name + synonyms (`mic`, `microphone`, `RTSP` → Audio settings).

Results are server-rendered. The client owns only: the overlay markup, the `⌘K` shortcut, debounced input handling, and arrow-key navigation. Total client JS: ~80 lines.

## Files

| Action | Path |
|---|---|
| Add | `crates/birdnet-web/templates/_partial_cmdk.html` — overlay + script |
| Add | `crates/birdnet-web/src/routes/pages/cmdk.rs` — `GET /pages/cmdk?q=…` handler + index |
| Append | `crates/birdnet-web/static/css/app.css` — see `css/app.css.append` |
| Patch | `crates/birdnet-web/templates/layout.html` — one line: include the partial before `</body>` |
| Patch | `crates/birdnet-web/src/routes/pages/mod.rs` — `pub(crate) mod cmdk;` + `.merge(cmdk::router())` |

## Endpoint

```
GET /pages/cmdk?q=cardin           → HTML fragment (10 rows max), targeted at #cmdk-results
GET /pages/cmdk                     → empty default with curated "Jump to…" rows
```

Response is a `<ul>` of grouped `<li>` rows, each with `data-href` and a server-decided rank. The client navigates on Enter or Click; no SPA, no history state — every result is a real link.

## Index sources

| Source | Where queried | Cached |
|---|---|---|
| Pages (≈20 entries) | hard-coded vec in `cmdk.rs` | const |
| Species | `birdnet-db::species_list_alpha()` already called by `/species` | per-request, capped at 200 rows |
| Dates | parsed in Rust (no DB) — see "Date strings" below | n/a |
| Settings | hard-coded vec in `cmdk.rs` | const |
| Recent detections | `birdnet-db::recent_detections(8)` — only when q is empty | per-request |

## Date strings

The handler interprets the query as a date when it matches any of:

- `today`, `yesterday`, `tomorrow` (UTC, station tz applied)
- `last <weekday>` / `this <weekday>` / a bare weekday name
- A `YYYY-MM` or `YYYY-MM-DD` literal
- `last week`, `this week`, `this month`, `last month`

A matching query injects a **Dates** group with a single result that jumps to the appropriate page (`/today?date=…`, `/history?date=…`, `/weekly?week=…`).

## Visual

A vertically-centred glass card, 540px wide, blurred backdrop. Top: a single input with a `⌘K` chip and an animated `…` indicator. Middle: scrollable results, grouped by source with sticky group headers. Bottom: a hint footer (`↑ ↓ navigate · ↵ open · esc close`).

Uses only existing tokens: `--surface`, `--border`, `--moss-soft`, `--hairline`. The backdrop is `color-mix(in oklch, oklch(0% 0 0) 28%, transparent)` with a 6px blur.

## Risk

Zero. Keyboard shortcut is namespaced (won't trigger inside `<input>`/`<textarea>`/`contenteditable`), the overlay is a plain `<dialog>` (graceful no-JS fallback: a small "Search" link in the topnav still goes to `/species`).

---

<!-- BNB:CROSSREF-FOOTER -->
## Related

* Reuses the search-input pattern from the existing topnav `.nav-search`.
* The dialog primitive is the same `<dialog>` element O-17 uses; both share `bnb-modal__form` if we extract the chrome — but that extraction is **not** needed for this PR.
<!-- BNB:CROSSREF-FOOTER -->
