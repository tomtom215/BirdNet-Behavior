# 06 — UI code: information architecture, charts, copy, states, front-end

Audit of the **source** of the server-rendered UI (`crates/birdnet-web`), read at `9d8bb1c` (2026-11-12) on
2026-11-22. Read-only: no code was edited and no build or browser was run (another agent owns the build and
the rendered screenshots). Every claim cites a file and line that was opened. "VERIFIED-BY-READING" means the
cited code was read and says what is claimed; "CONJECTURED" names the probe that would settle it.

Audiences: **B** = a non-technical birder, **C** = a chart lover, **R** = an academic researcher.

## Counts at a glance

| What | Count | How counted |
|---|---|---|
| Distinct route paths registered with `.route(` | 382 | multiline regex over `src/routes/**` (scratch list, not committed) |
| … admin (`/admin/*`, gated) | 113 | prefix |
| … `/pages/*` HTMX partials | 92 | prefix |
| … static assets | 34 | `static_files.rs:189-240` |
| … other (REST under `/api/v2`, feeds, share, login) | ~120 | remainder |
| Full HTML page routes a person can open | 29 | table below |
| Addressable tab/view states inside those pages | 15 | Patterns 7, Reports 3, Species 3, Recordings 2 |
| Legacy 308 redirects | 16 | `routes/redirects.rs:24-48` |
| Top-level nav items | 6 | `routes/pages/nav.rs:46-88` |
| `/pages/*` partials with no caller in `src/`, `templates/`, `static/` | 15 (10 in TD-6, **5 new**) | grep per path, excluding the `.route(` line, comments and tests |
| Full pages with no in-app link | 1 (`/player/{filename}`) | grep |
| Charts / visualisations inventoried | 47 (41 live, 6 orphaned) | chart table below |
| Rendering systems | 7 | server SVG kit; hand-written SVG; HTML/CSS bar and grid charts; client-side JS SVG; Canvas 2D; WebGL (three.js r186); server PNG spectrograms |

---

## 1. Route inventory

### 1.1 Full pages

| # | Route | Handler | Nav home (`active_nav`) | How a person reaches it | Public? | Notes |
|---|---|---|---|---|---|---|
| 1 | `/` | `pages/today.rs` | Today | top nav, tab bar | yes | Today home; polls 9 regions (`templates/today.html:26,32,49,90,147,163,185,195,202`) |
| 2 | `/species` (`?view=list\|photos\|lifelist`) | `pages/species_pages.rs:149-178` | Species | top nav; header search submits `?q=` here (`templates/layout.html:81`) | yes | 3 views |
| 3 | `/species/detail?name=` | `species_pages.rs` + `templates/species_detail.html` | Species | any species link | yes | 9 HTMX cards (`species_detail.html:30-169`) |
| 4 | `/species/occurrence` | `pages/occurrence_page.rs:46` | Species | one button on `/species` (`species_pages.rs:190`) | yes | "Why isn't a bird heard here?" |
| 5 | `/patterns` (`?tab=when\|dawn\|trends\|together\|migration\|behavior\|sequences`) | `pages/homes/patterns.rs:36-133` | Patterns | top nav | yes | 7 tabs in 3 groups (When/Who/How) |
| 6 | `/recordings` (`?view=clips\|live`) | `pages/recordings.rs:51-97` | Recordings | top nav | yes | |
| 7 | `/reports` (`?tab=weekly\|year\|history`) | `pages/homes/reports.rs:23-71` | Reports | top nav | yes | |
| 8 | `/reports/day?date=` | `pages/history.rs:380-412` | Reports | "Open day →" in History (`history.rs:446`) | yes | renders every detection of the day, unpaged (UIC-21) |
| 9 | `/station` | `pages/homes/station.rs:117` | **Settings** (label) | top nav; header health badge | yes | Health tab |
| 10-14 | `/station/{capture,alerts,data,settings,access}` | `homes/station_tabs.rs` | Settings | Station sub-tabs | **gated** | admin pages regrouped |
| 15 | `/detections/detail?date&time&name&file` | `pages/detection_detail.rs` | — | time link on every row | yes | only place a share link is made (`detection_detail.rs:224-227`) |
| 16 | `/detection-reviews` | `pages/detection_reviews.rs:43` | — | one link in the review widget (`detection_reviews.rs:781`) and in Quarantine's text (`quarantine.rs:130`); cmdk does **not** list it | yes | overlaps `/quarantine` (UIC-08) |
| 17 | `/quarantine` | `pages/quarantine.rs` | — | Today nudge (`today.rs:758`), cmdk (`cmdk.rs:325`) | yes | kept out of redirects on purpose (`redirects.rs:12-14`) |
| 18 | `/search` | `pages/search.rs` | — | cmdk (`cmdk.rs:332`), a few links | yes | the detection log; no export (UIC-10) |
| 19 | `/notifications` | `pages/notification_center.rs:23` | — | **cmdk only** (`cmdk.rs:352,947`) | yes | |
| 20 | `/kiosk` | `pages/dashboard/mod.rs:27` | — | cmdk (`cmdk.rs:347`), links | yes | plate / numbers modes |
| 21-23 | `/onboarding`, `/onboarding/done`, `/onboarding/mic-test` | `pages/onboarding.rs` | — | first run, cmdk | mixed | `/onboarding/save` is POST |
| 24 | `/system/changelog` | `pages/changelog.rs` | — | cmdk, footer | yes | `/system/changelog/latest` is a fragment fetched by the update banner (`_partial_update_banner.html:113`) |
| 25 | `/system/credits` | `pages/credits.rs` | — | footer | yes | |
| 26 | `/help/*` | `pages/help.rs:90-93` (mdBook `ServeDir`) | — | `?` button, help links | yes | 12 of 24 topics have no call site (TD-2) |
| 27 | `/login` | `routes/auth_pages.rs:43` | — | gate redirects | yes | |
| 28 | `/r/{token}` | `routes/share.rs:57` | — | copied share link | yes | 30-day TTL (`share.rs` `DEFAULT_TTL_SECS`) |
| 29 | `/player/{filename}` | `pages/audio_player.rs:30` | — | **nothing links to it** | yes | orphan page (UIC-05) |
| — | fallback | `pages/mod.rs` `not_found` | — | mistyped URL | yes | branded 404; JSON under `/api/` |

Feeds (`/feeds/rare.rss`, `/feeds/rare.ics`, `/feeds/today.rss`, `routes/feeds.rs:30-32`) and `/api/v2/openapi.json`
are machine surfaces; `openapi.json` is linked from nowhere in the UI (grep: 0 hits outside its route).

### 1.2 Top-level navigation

Six items, one table generates desktop nav, phone tab bar and active state (`nav.rs:46-88,111-138`): **Today ·
Species · Patterns · Recordings · Reports · Settings** (the last is `/station`, relabelled on purpose,
`nav.rs:77-86`). Station sub-tabs: Health · Capture · Alerts · Data · General · Access (`homes/station.rs:24-58`).
The command palette (`pages/cmdk.rs:153-493`) lists the long tail (Quarantine, Search, Kiosk, Notifications,
Changelog, Help, Onboarding and 14 admin anchors).

### 1.3 Orphaned `/pages/*` partials (TD-6 and beyond)

| Partial | Registered | In TD-6? | What it is |
|---|---|---|---|
| `/pages/stats`, `/pages/hero-status`, `/pages/species-list`, `/pages/hourly-chart`, `/pages/daily-chart`, `/pages/most-recent` | `dashboard/mod.rs:28-44` | yes | the pre-spine dashboard |
| `/pages/disk-status` | `health.rs` | yes | |
| `/pages/history-dates` | `history.rs:413` | yes | "legacy date browser" |
| `/pages/today-lock`, `/pages/today-unlock` | `today.rs:57-58` | yes | dead: Today rows post to `/pages/recordings-lock` via `recordings::lock_button` (`today.rs:1247-1254`) |
| **`/pages/today-relabel`** | `today.rs:53-56` | **no** | POST, nothing in the tree posts to it |
| **`/pages/confidence-chart`** | `dashboard/mod.rs:40-42` | **no** | a confidence histogram (`partials.rs:526-541`, `bars::distribution`) |
| **`/pages/activity-heatmap`** | `dashboard/mod.rs:46-48` | **no** | species × hour grid for today, with its own linear ramp (`heatmap.rs:436-443`) |
| **`/pages/life-accumulation`** | `life_list.rs:21` | **no** | the lifelist view computes the same curve inline (`species_pages.rs:529`) |
| **`/pages/ts-heatmap`** | `timeseries_dash.rs:36` | **no** | removed from `templates/timeseries.html:44-46`; route kept ("The route still answers", `timeseries_dash.rs:602-604`) |

### 1.4 Depth to common birder tasks (clicks from `/`)

| Task | Path | Depth | Note |
|---|---|---|---|
| What did I hear today? | Today feed (`today.html:147`) | 0 | |
| Is that a new species? | first-ever badge on the row; "first" filter on Today | 0 | |
| Play the sound | ▶ on the Today row (`today.rs:1241`) | 0 | button's accessible name is "Play clip" for every row (UIC-12) |
| Is my station working? | header badge (every page, links to `/station`, `health.rs:136-146`) → Health | 0 to glance, 1 for reasons | the home is labelled **Settings** (UIC-07) |
| Share this | row time → detection detail → "Share clip" | 2 | per detection only; no share for a species, a day or a chart (UIC-09) |
| Take my data away | Settings → Data (login) → export buttons (`admin/backup_recovery.rs:437-461`) | 2 + sign-in | detection log `/search` has no export (UIC-10) |
| Why is a bird never heard? | Species → "Why isn't a bird heard here?" | 2 | |
| Review doubtful detections | Today nudge → `/quarantine`; or detail → `/detection-reviews` | 1–2 | two queues (UIC-08) |

---

## 2. Chart inventory

Legend. **Renderer**: *kit* = server SVG via `routes/pages/viz/*` with `static/chart-kit.js` (pointer, touch and
keyboard readings, `aria-live` announcement, `svg_a11y` `<title>/<desc>`, colours only from `--chart-*`/`--seq-*`
tokens so dark mode is automatic and the categorical palette is graded by
`tests/the_chart_palette_can_be_told_apart.rs`, per `docs/design/charts.md`). *hand SVG* = server SVG written in
the page module. *HTML* = `<div>`/`<table>` with `data-style` widths or backgrounds. *JS SVG* / *canvas* / *WebGL*
= drawn in the browser. **Effort**: whether `recording_effort` (via `pages/listening.rs`) is consulted; in every
"yes" it only **hatches** a day as "not listening", it never scales a count (`listening.rs:12-16`). **Alt** =
table or readout carrying every value. **Export**: CSV/JSON next to the chart. "—" = absent.

| # | Where | Question | Renderer (file) | Data | n / effort | Empty | Alt | Export |
|---|---|---|---|---|---|---|---|---|
| 1 | Today | when were birds heard today? (stacked columns, temp strip, sun rail) | kit `viz/timeline.rs:660` → `bars::hourly` | `/pages/today-daystrip` | totals in head / n.a. | skeleton `skeletons.rs:68`; "Not enough" | — (`table: false`, `timeline.rs:691`) | — |
| 2 | Today ("Every song") | every song on the day's sky, by sureness | JS SVG `static/skysong.js` | `/pages/today-songs` JSON (`today_songs.rs:122-160`), capped, `total` disclosed | yes (total) / — | "The songs are not available just now" (`skysong.js:812`) | readout + key | — |
| 3 | Today hero | live signal, last 30 s | canvas `#hero-pulse` (`today.rs:251`) | spectrogram socket | — | note "Waiting for the first audio segment…" | — (aria-label only) | — |
| 4 | Today "Top species" rail | 14-day shape per species | hand SVG `atoms::sparkline` (`atoms.rs:260-305`), `aria-hidden` | `/pages/top-species` | — | "Nothing heard yet today." | numbers in row | — |
| 5 | Today rows | how sure was the station? | HTML `conf_bar` | row | — | — | % text | — |
| 6 | Patterns · When | soundscape: days × hours | canvas 2D, WebGL on request (`static/terrain.js`, `templates/terrain.html:45-48`) | `/pages/terrain-data` | — / **yes** (`terrain.rs:387-389`) | "(loading)" label | table (`terrain.rs:428`) | — |
| 7 | Patterns · When | who's singing over time | kit streamgraph `viz/timeline.rs:177` | `/pages/activity-streamgraph` | — / **yes** (`heatmap.rs:340-346`) | "Not enough data" | table (`timeline.rs:450`) | — |
| 8 | Patterns · When | detections by hour | kit `bars::hourly` (`heatmap.rs:480-498`) | `/pages/hourly-totals` | — / — (sun only ≤14 d, `heatmap.rs:254-258`) | "No data available…" | table (`heatmap.rs:496`) | — |
| 9 | Patterns · When | hour × weekday | kit `viz/heat.rs:133` | `/pages/heatmap-grid` | per listened weekday / **yes** (`heatmap.rs:180-206`) | "Not enough data" | marginal totals only | — |
| 10 | Patterns · Dawn | who sings when (rings) | hand SVG on kit attrs `dawn_chorus.rs:435` | `/pages/dawn-polar` | "Last 60 days", "Per-species normalized" pills (`dawn_chorus.rs:69`) / — | `no_chorus()` | table (`dawn_chorus.rs:895`) | — |
| 11 | Patterns · Dawn | per-species hour strips | hand SVG `dawn_chorus.rs:750` | `/pages/dawn-list` | — / — | skeleton | via #10 | — |
| 12 | Patterns · Trends | detections per day vs previous 30 | kit `viz/trend.rs:74` | `/pages/ts-daily` (DuckDB `MovingAverage`) | — / hatch yes (`timeseries_dash.rs:300-306`), average **not** effort-aware (UIC-03) | "No trend data yet." | table | — |
| 13 | Patterns · Trends | species richness vs previous 30 | kit `viz/trend.rs` | `/pages/ts-diversity` | — / hatch yes | "No diversity data yet." | table with Shannon H′, evenness | — |
| 14 | Trends (folded) | peak 15-minute windows | table | `/pages/ts-peak` | — | "No detections to rank yet." | itself | — |
| 15 | Trends (folded) | activity sessions | table | `/pages/ts-sessions` | — | "No activity sessions found." | itself | — |
| 16 | Trends (folded) | anomalous days | table (z-score) | `/pages/ts-anomalies` | — / **no** (UIC-03) | "✓ No anomalies…" | itself | — |
| 17 | Patterns · Together | who sings together (chord) | kit `viz/radial.rs:223` via `viz/matrix.rs:216` | `/pages/acoustic-network` | shared 5-min blocks + Jaccard in tooltip/table / — | `no_co_signal()` | pairs table (`correlation.rs:305-330`) | — |
| 18 | Patterns · Together (phone) | same, as a ladder | kit `viz/matrix.rs:51` | `/pages/cooccurrence-matrix` | same | same | "See the numbers" (`correlation.rs:89`) | — |
| 19 | Patterns · Together | companions of one species | HTML table + bars (`correlation.rs:386-445`) | `/pages/companion-species` | raw shared **days**, bar relative to top (UIC-04) | hint | itself | — |
| 20 | Patterns · Migration | when each species is here (ridgeline + species/week lane) | kit `viz/ridge.rs:200` | `/pages/migration-ridgeline` | per-species normalised (stated, `ridge.rs:6-10`) / — ; **current year only** (`migration.rs:197`) | `no_migrants()` | table (`ridge.rs:492`) | — |
| 21 | Patterns · Visits | bursts of singing | kit `viz/bursts.rs:113` | `/pages/analytics-sessions` | yes / — | `EMPTY` | table (`behavioral.rs:299`) | **Method + CSV + JSON** |
| 22 | Patterns · Visits | who keeps coming back | HTML table heat (`behavioral.rs:434-526`) | `/pages/analytics-retention` | weeks heard "x of y" / — | "No retention data yet." | itself | **Method + CSV + JSON** |
| 23 | Patterns · Sequences | before and after one bird | kit `viz/flow.rs:91` | `/pages/analytics-next` | denominator stated (HANDOFF §5.32) | `EMPTY` | table (`behavioral.rs:722-749`) | **Method** |
| 24 | Patterns · Sequences | morning running order | kit `viz/funnel.rs:260` | `/pages/analytics-dawn-sequence` | — | `EMPTY` | "Recent mornings" table (`behavioral.rs:1317`) | **Method** |
| 25 | Patterns · Sequences | who answers whom | hand SVG `behavioral_answers.rs:138` | `/pages/analytics-answers` | floor `MIN_EXCHANGES = 3`, cells under it outlined (`behavioral_answers.rs:12,27,270`) | sentence (`:264`) | table (`:286`) | **Method + JSON** |
| 26 | Patterns · Sequences | chorus assembling | kit `viz/assembly.rs:278` | `/pages/analytics-assembly` | — | `EMPTY` | table (`behavioral_assembly.rs:128`) | **Method + CSV + JSON** |
| 27 | Species list | 14-day shape per species | `atoms::sparkline` (`species_pages.rs:373-376`) | page | — | — | count column | — |
| 28 | Species · Life list | list size over time | kit `viz/timeline.rs:501` | inline (`species_pages.rs:529`) | — / — ; no estimator | "Not enough data" | — | — |
| 29 | Species · Life list | when each bird joined | JS SVG `static/discovery.js` | JSON in attribute (`life_timeline.rs:3,96`) | — | — | phone list + aria | — |
| 30 | Species detail | when it's most active | kit `bars::hourly` via `charts::render_hourly_chart` (`species_pages.rs:765`) | `/pages/species-hourly` | — / — | "No detections today yet." (wording is Today's, `charts.rs:22`) | — | — |
| 31 | Species detail | last 14 days | kit `bars::daily` pair (`species_pages.rs:814`) | `/pages/species-daily` | — / **yes** | — | — (`charts.rs:153`) | — |
| 32 | Species detail | through the year | kit `viz/heat.rs:396` calendar (`species_pages.rs:867`) | `/pages/species-year` | — / **yes** | "Not heard in the last year." | — | — |
| 33 | Species detail | heard the same days | HTML table (`species_pages.rs:1330-1344`) | `/pages/species-companions` | raw shared days | — | itself | — |
| 34 | Recordings · Live | live spectrogram | canvas `#rc-spectrogram` (`recordings.rs:602`) | `/api/v2/ws/spectrogram` | — | — | — | — |
| 35 | Recordings · Clips; detail | clip spectrogram | server PNG (`routes/spectrogram`) | `/api/v2/spectrogram/{file}` | — | — | alt "Spectrogram of {name}" | PNG via URL |
| 36 | Reports · Weekly | this week by day | kit `bars::daily` (`weekly_report.rs:291-320`) | `/pages/weekly-content` | like-for-like days (ux-audit #3) / — | — | — (`table: false`, `:320`) | Save as PDF |
| 37 | Reports · Year | the year in activity | kit `viz/heat.rs:396` (`year_in_review.rs:182`) | page | — / **yes** | — | — | Save as PDF |
| 38 | Reports · Year | who led each month (bump chart + bars) | JS SVG `static/seasons.js` | JSON in attribute (`year_in_review.rs:400-450`) | — | — | table (`year_in_review.rs:430`) | Save as PDF |
| 39 | Reports · History | month calendar | **HTML grid, own linear ramp** (`history.rs:193-300,329-336`) | `/pages/history-calendar` | count + species per cell / **no** | inline error | numbers in cells | — |
| 40 | Reports · History / `/reports/day` | one day by hour | kit via `render_hourly_chart` (`history.rs:462,512`) | `/pages/history-chart` | — / — | "No detections today yet." (for a past day) | — | — |
| 41 | Kiosk | today as an engraved plate / numbers | hand SVG + canvas (`dashboard/plate/render.rs`, `static/plate.js`) | `/pages/kiosk-content` | — | — | numbers mode | — |
| 42 | Station · Capture (gated) | EQ curve | hand SVG `admin/eq_curve.rs:129` | form | — | none for a bad spec | aria-label summary | — |
| 43-47 | **orphans** | confidence histogram (`partials.rs:526`), species×hour widget (`heatmap_widget.rs`), ts hour heatmap table, dashboard hourly/daily charts (`partials.rs:473`), stat-tile sparkline (`stats.rs:84`) | mixed | orphan routes | — | — | — | — |
| — | decorative | empty-state illustrations (`empty_states.rs`), `/player` playhead canvas (`audio_player.rs:63`, orphan) | hand SVG / canvas | — | — | — | — | — |

What the table shows, in one paragraph: the **kit** charts are consistently good (zero baselines, monotone
smoothing, labelled extremes, keyboard readings, token colours), and the docs' honesty rules are real. The
weaknesses are around the kit, not in it: (a) **effort** is used to hatch days, never to normalise, while an
effort-corrected API sits unused (UIC-02); (b) **export and method** exist on 6 of ~41 live charts (UIC-10);
(c) **table alternatives** are missing on the most-seen charts (Today's strip, species hourly/daily, weekly,
history day) (UIC-27); (d) **three heat-scale implementations** disagree (kit sqrt steps, History linear
`color-mix`, the orphan widget linear `--seq`) (UIC-18).

---

## 3. Findings

### UIC-01: Year in review says "days of listening" and "≈ N a day" for days on which something was *detected*
- Severity: high — Kind: bug — Audience: B, R — NEW
- Evidence: `routes/pages/year_in_review.rs:109` `let active_days = daily.len();` where `daily` is
  `range_daily_counts` (`birdnet-db/src/sqlite/queries/analytics.rs:251-271`, `GROUP BY Date`, so only days with a
  detection). `:128-131` `per_day = total / active_days`; `:161` prints `≈ {per_day} a day`; `:290`
  "`{total} detections across {days} days of listening in {year}`" with `days = active_days` (`:295`). The
  effort ledger is read on the same page (`:57`) and passed to the calendar only (`:80`).
  Status: VERIFIED-BY-READING
- Impact: a station that listened 320 days and heard birds on 250 says "250 days of listening", and its daily
  average is inflated by the silent days left out. It is the page people print and share ("Save as PDF").
- Proposed change: count listening days from `effort` (days with ≥ `MIN_LISTENED_SECS`, `listening.rs:26`) when
  the ledger has rows; otherwise say "days with a detection". Divide the average by the same denominator and
  say which it is.
- Acceptance gate: a unit test with a fixture of 10 listened days, 6 with detections: the closing line says
  10 days of listening and the average divides by 10; the counterpart without a ledger says "6 days with a
  detection". Observe it red against the current `active_days`.
- Effort: S

### UIC-02: An effort-corrected abundance API exists and no chart uses it; every Patterns chart draws raw counts
- Severity: high — Kind: gap — Audience: R, C — NEW
- Evidence: `routes/analytics.rs:948-973` ("Detections per hour of listening, per species per week… the analytic a
  raw count cannot substitute for"), `:975-1000` (phenology timing per species-year). `grep` for
  `analytics/abundance|analytics/phenology` in `src/routes/pages`, `templates`, `static`: 0 hits.
  `pages/listening.rs:12-16`: effort "is only ever used to *add* a statement … never to scale or correct a
  count". The ridgeline uses raw weekly counts for the current year only (`migration.rs:196-216`).
  Status: VERIFIED-BY-READING
- Impact: a day with 3 hours of audio (power cut, SD-card full) reads as a quiet day on every chart except the
  binary "not listening" hatch (threshold 60 s, `listening.rs:26`); seasonal comparisons conflate day length and
  uptime with bird activity. The API's own doc names this as the standard correction.
- Proposed change: add a "per listening hour" toggle to the ridgeline, daily trend and hour-by-weekday grid
  (data: `/api/v2/analytics/abundance`, `recording_effort`), defaulting to counts for B and rates for R; draw
  effort as a thin lane under each chart so low-effort weeks are visible.
- Acceptance gate: a fixture where week 10 has half the listening seconds of week 9 and the same detections:
  the rate view draws week 10 twice as tall; the count view unchanged.
- Effort: M

### UIC-03: Trend average and anomaly detection count an outage as a quiet day
- Severity: medium — Kind: bug — Audience: R, B — NEW
- Evidence: `birdnet-timeseries/src/queries/trend.rs:293-304` builds a zero-filled date spine and `:321-326`
  flags `detections < rolling_mean - z*stddev` as `low`; `grep recording_effort|effort` in
  `crates/birdnet-timeseries/src`: 0 hits. The `MovingAverage` doc says an average "is only reported when all of
  those days are ones the station was running" (`trend.rs:10-12`), but "running" is only "after the first
  detection" (`station_start`, `:60`). The page renders the flagged rows as "Type: low" with a "Z-Score" column
  (`pages/timeseries_dash.rs:525-552`).
  Status: VERIFIED-BY-READING
- Impact: every outage becomes an "anomaly", and depresses the baseline the next 30 days are judged against.
  The researcher reading "Anomaly detection" sees instrument faults labelled as bird events.
- Proposed change: join the spine to `recording_effort` (DuckDB already reads `detections_ts`; mirror the
  ledger or pass listened dates in), exclude non-listening days from both the baseline and the flagged set, and
  label any day with partial effort. Correct the `MovingAverage` doc.
- Acceptance gate: a timeseries test with a 3-day outage in a steady series: no `low` rows on the outage days,
  and the day after the outage is `normal`. Observe red first.
- Effort: M

### UIC-04: "Companion species" ranks by raw shared days — the defect the pair view was fixed for
- Severity: medium — Kind: bug (statistical) — Audience: B, R — NEW
- Evidence: `birdnet-db/src/sqlite/queries/correlation.rs:148-171` (`COUNT(DISTINCT d.Date) AS shared_days …
  ORDER BY shared_days DESC`); the same file's pair query says counting shared days "ranks a dawn resident with
  a dusk one as the strongest pair" and moved to Jaccard (`:42-49`). The table's bar is relative to the top
  companion (`pages/correlation.rs:397,419`) under the header "Co-occurrence" (`:406`); the species page uses
  the same query (`species_pages.rs:1330`, header "Co-occurrence Days", `:1344`). `FollowOn` (`birdnet-db/src/sqlite/queries/correlation.rs:31-34`) is documented as
  "seen *after*" (`correlation.rs` struct doc) but the query is same-day.
  Status: VERIFIED-BY-READING
- Impact: the most common residents are every species' "companions"; the answer barely depends on the species
  asked about. Rare species look associated with everything.
- Proposed change: rank by Jaccard on shared days (or 5-minute blocks, like the chord) and print "x of the
  trigger's y days"; fix the struct doc.
- Acceptance gate: a fixture with one ubiquitous resident and a true pair: the pair ranks first; counterpart:
  the resident still appears, below it.
- Effort: S

### UIC-05: Six more routes are orphaned than TD-6 lists, including a full page
- Severity: medium — Kind: gap — Audience: maintainers — ALREADY-TRACKED(TD-6) for 10; NEW for 6
- Evidence: §1.3 table. `/pages/today-relabel` (`today.rs:53-56`), `/pages/confidence-chart`
  (`dashboard/mod.rs:40-42`), `/pages/activity-heatmap` (`:46-48`), `/pages/life-accumulation` (`life_list.rs:21`),
  `/pages/ts-heatmap` (`timeseries_dash.rs:36`, kept per `:602-604`), and the page `/player/{filename}`
  (`audio_player.rs:30`). `today-lock`/`today-unlock` are dead because Today rows use
  `recordings::lock_button` (`today.rs:1247-1254`).
  Status: VERIFIED-BY-READING (grep per path; a URL built from pieces would be invisible, as TD-6 notes)
- Impact: each orphan is maintained, tested and pre-warmed (`ts-heatmap` was removed from prewarm only) for no
  reader; `today-relabel` is a write endpoint behind auth with no UI. Two of them (confidence histogram,
  species×hour widget) are charts a chart lover would want (UIC-25).
- Proposed change: add the five to TD-6 now; then either link (confidence histogram on the species page, see
  UIC-25) or delete. Extend TD-6's scan to full-page routes.
- Acceptance gate: a test that lists every `GET` route under `/pages/` and full pages and fails on any with no
  caller outside tests, with a named exemption list (CLAUDE.md: "prefer a named exemption list").
- Effort: S

### UIC-06: The "is it working?" home is labelled "Settings"
- Severity: medium — Kind: gap (IA) — Audience: B — NEW (the relabel itself was deliberate)
- Evidence: `nav.rs:77-86` (label "Settings", path `/station`); its first and only public tab is Health, "is it
  working?" (`homes/station.rs:24-29`). Mitigation: the header badge links there (`health.rs:136-146`).
- Impact: a birder whose badge is amber is taken to a page whose nav entry says Settings, and a visitor without
  a login sees a "Settings" tab of which 5 of 6 sub-tabs demand sign-in.
- Proposed change: nav label "Station" (subtitle in the tab bar: "health & settings"), or move a Health summary
  card to Today and keep Settings for the gated tabs. Decide with the IA proposal in UIC-30.
- Acceptance gate: `nav.rs::spine_is_the_six_homes_in_order` updated deliberately; a11y name of the tab bar
  slot includes "health".
- Effort: S

### UIC-07: Two "Activity sessions", two review queues, three searches, four hour-of-day pictures
- Severity: medium — Kind: anti-pattern (IA overlap) — Audience: B, R — ALREADY-TRACKED(docs/design/insights-and-organisation.md §2, "the same question is answered in several homes"); NEW for the session duplicate and the review queues
- Evidence: Trends' folded "Activity sessions" from `birdnet-timeseries` (`templates/timeseries.html:57-66`,
  `timeseries_dash.rs:190-196`, gap 30) and Visits' "Activity sessions / Bursts of singing" from
  `birdnet-behavioral` `sessionize` (`templates/analytics.html:5-15`, gap default 30, `birdnet-behavioral/src/types.rs:278-285`).
  `/quarantine` (rare holds) and `/detection-reviews` (verdicts) link to each other in prose
  (`quarantine.rs:130`); only the first is in cmdk. Header search → `/species?q=` (`layout.html:81`), `/search`
  (detection log), cmdk. Hour-of-day: Today strip, When/hourly totals, Dawn rings, species hourly.
  Status: VERIFIED-BY-READING
- Impact: two engines can give two different session counts for the same days under the same name; a birder
  cannot tell which review page is "the" one.
- Proposed change: delete the Trends sessions table (Visits owns sessions); make `/detection-reviews` a tab of
  one "Review" page with Quarantine; label the header box "Find a species" and put "Search all detections →"
  under its results.
- Acceptance gate: a route-level test that no two pages render a card titled "Activity sessions".
- Effort: S–M

### UIC-08: The Patterns range is JavaScript-only, not in the URL, not remembered, and not shareable
- Severity: medium — Kind: gap / anti-pattern (progressive enhancement) — Audience: R, C — NEW
- Evidence: `heatmap.rs:61-75` renders `<button type="button" data-days>`; `:146-165` the only wiring is an
  inline script calling `htmx.ajax`. Only `search.rs:315,660` and `occurrence_page.rs` push URL state (grep of
  `hx-push-url|replaceState|pushState`). Contrast Today's view switch, hidden until its script runs
  (`today.html:83`). Same shape: companion lookup (`correlation.rs:106-138`).
  Status: VERIFIED-BY-READING
- Impact: a researcher cannot link "the last 365 days of the When tab"; reload loses the range; with scripting
  off the buttons do nothing.
- Proposed change: make the control a `GET` form (`/patterns?tab=when&days=365`), server-render the partials at
  that range, and let the script upgrade it with `hx-push-url`. Apply the same `days`, and later `source`, to
  every Patterns tab.
- Acceptance gate: `GET /patterns?days=365` renders all four When figures with `days=365` in their `hx-get`;
  a Playwright step reloads after choosing a range and finds it kept.
- Effort: M

### UIC-09: Sharing exists only for one detection
- Severity: low — Kind: gap — Audience: B, C — NEW
- Evidence: the only issuer is `detection_detail.rs:224-227`; `share.rs` signs `(date, time, com_name)` only.
- Impact: "look at this year" or "this species' calendar" cannot be shared without exposing the station.
- Proposed change: after UIC-08, a "Copy link" on each figure (LAN), and a signed static snapshot for Reports
  (the print stylesheet already exists).
- Acceptance gate: a figure's link reproduces its range and tab.
- Effort: M

### UIC-10: Data export is a date range behind a login; the detection log cannot export what it found
- Severity: medium — Kind: gap — Audience: R — ALREADY-TRACKED(HANDOFF §5.31) for Method/export on charts; NEW for export filters
- Evidence: `routes/export/csv.rs:15-22` (`format`, `from`, `to` only); UI links only on gated pages
  (`admin/backup_recovery.rs:437-461`, `admin/overview.rs:226`); `pages/search.rs` has no CSV/export (0 matches),
  although it composes nine criteria (`search.rs` module doc). Method panels on 6 cards (`behavioral.rs:152,384,626,832,1057`,
  `behavioral_assembly.rs:220`). No export carries `recording_effort` (grep: no route reads it except
  `analytics.rs`).
  Status: VERIFIED-BY-READING
- Impact: a researcher can filter "this species, 22:00–04:00, below 40 %" on screen and cannot take that set
  away; no export lets them compute rates.
- Proposed change: "Download these rows (CSV)" on `/search`, reusing `DetectionFilter` for the export query;
  add `/api/v2/effort/export` (date, source, seconds). Method panel + JSON link for each Patterns figure.
- Acceptance gate: the search CSV for a filter equals the rows the page lists (count and keys).
- Effort: M

### UIC-11: No source, model or confidence filter on any Patterns chart
- Severity: medium — Kind: gap — Audience: R — ALREADY-TRACKED(docs/DESIGN_BRIEF.md "Known problems" #5)
- Evidence: query structs accept `days` only: `heatmap.rs:38-40`, `terrain.rs:57-59`; `correlation.rs:47-53`
  adds `species`, `links`. Status: VERIFIED-BY-READING
- Impact: multi-microphone stations see merged counts; a two-classifier station cannot compare models.
- Proposed change: one filter bar (range, source, model, min confidence) in the URL (UIC-08), honoured by every
  partial; index exists (migration 52, per DESIGN_BRIEF).
- Effort: L

### UIC-12: Every play button on Today is named "Play clip"
- Severity: medium — Kind: bug (a11y, WCAG 2.4.6) — Audience: B — NEW
- Evidence: `today.rs:1241-1242` (`aria-label=\"Play clip\"`), `dashboard/partials.rs:154` (feed row), `:257`
  (best recordings). Recordings does it right: `aria-label="Play {com_name}"` (`recordings.rs` row template,
  `recordings.rs:476`).
  Status: VERIFIED-BY-READING
- Impact: a screen-reader user tabbing the feed hears "Play clip" thirty times.
- Proposed change: `aria-label="Play {shown name}, {time}"` (escaped), as Recordings.
- Acceptance gate: a page test that every `.x-fplay`/`.x-play` on Today has a distinct accessible name.
- Effort: S

### UIC-13: Today's headline lives in a region swapped every 5 minutes, named by `aria-label` on a `div`
- Severity: low — Kind: polish (a11y) — Audience: B — NEW
- Evidence: `templates/today.html:23-28` (`<div id="today-phrase" … hx-trigger="load, every 5m" … aria-label="Day summary">`),
  the `<h1>` comes from `today_phrase.rs:137-167`. ARIA does not name a generic `div`.
  Status: VERIFIED-BY-READING; whether a screen reader re-announces is CONJECTURED (probe: NVDA/VoiceOver on Today for 5 min)
- Proposed change: keep a static `<h1>` outside the swap and swap only the sentence; drop the `aria-label`.
- Effort: S

### UIC-14: A birder tab shows the database's file path and build instructions
- Severity: low — Kind: anti-pattern (IA, minor disclosure) — Audience: B — NEW
- Evidence: Visits tab card "The analytics engine" (`templates/analytics.html:32-42`) loads
  `/pages/analytics-config` which prints `SQLite Database <code>{db_path}</code>`, DuckDB and extension versions,
  `--refresh-extension`, `BIRDNET_BUNDLED_EXTENSION_FILE` (`behavioral.rs:1335-1398`). The page is in the public
  router (`routes/mod.rs` `public_routes` merges `pages::router()`).
  Status: VERIFIED-BY-READING
- Impact: noise for B; an absolute server path on a public page.
- Proposed change: move the card to Settings → Health (or the Method panel's "Computed with"), drop the path.
- Effort: S

### UIC-15: Jargon a birder will not understand (with proposed wording)
- Severity: medium — Kind: polish (copy) — Audience: B — NEW
- Evidence and proposals (all VERIFIED-BY-READING):

| String | Where | Proposed |
|---|---|---|
| "Anomaly detection", "z &gt; 2.0", "deviate &gt; 2 standard deviations from the 30-day rolling mean", column "Z-Score" | `templates/timeseries.html:71-73`, `timeseries_dash.rs:531` | "Unusual days" · "Days much busier or quieter than the month before"; column "How unusual" (keep z in Method) |
| "Shannon H′", "Evenness", "Richness" | `timeseries_dash.rs:480`, `timeseries.html:16` | "Kinds of birds" · "Balance (0–1)"; H′ in a tooltip/Method |
| "Activity sessions", "Continuous-presence windows separated by inactivity gaps" | `timeseries.html:58-60` | remove (UIC-07) or "Busy spells" |
| "Per-species normalized" | `dawn_chorus.rs:69` | "Each bird scaled to its own busiest hour" |
| "Avg Confidence" / "Co-occurrence" / "Co-occurrence Days" | `correlation.rs:405-406`, `species_pages.rs:1344` | "How sure (average)" / "Days heard together" |
| "Species retention" | `templates/analytics.html:21` | the h2 already says "Who keeps coming back"; drop the eyebrow term |
| `window_funnel`, `window_funnel_events` in visible text | `frame.rs:253` (test), `behavioral_assembly.rs:235` | keep inside "Method and export" only |
| "Hourly/Daily/Weekly tumbling windows", "hopping windows" | `timeseries.html:101-110` | fine for the collapsed API table; label it "for scripts" (it is) |
| "Detections above 0.95 confidence skip this queue automatically." | `empty_states.rs:125` | "Detections the station was at least 95 % sure of skip this queue." (and read the real setting rather than a literal) |
| "Analytics Compiled / Analytics Active / DuckDB" | `behavioral.rs:1370-1380` | move (UIC-14) |

- Acceptance gate: a copy test listing banned terms outside `<details class="bh-method">` and `/admin`.
- Effort: S

### UIC-16: The retention lede describes a table that is no longer there
- Severity: low — Kind: bug (copy) — Audience: B, R — NEW
- Evidence: `templates/analytics.html:24` "how often it returned the **next** day"; the table shows 1 d, 2 d,
  3 d, 1 wk, 2 wk, 1 mo (`behavioral.rs:413-420`) and its key says "that much later" (`:524`).
  Status: VERIFIED-BY-READING
- Proposed change: "Of the days a species was heard, how often it was heard again a day, a week or a month later."
- Effort: S

### UIC-17: Localisation stops at species names; chart times ignore the 12-hour setting; numbers always use commas
- Severity: medium — Kind: gap — Audience: B — ALREADY-TRACKED(docs/design/tier-c-proposals.md §4) for UI strings; NEW for the clock and number items
- Evidence: species names only (`birdnet-core/src/i18n.rs:1-31`). `display.rs:1-31` promises one formatter
  marked with `data-clk` so `display-format.js` can reprint; chart captions and tooltip headings use private
  formatters with no marker: `viz/bars.rs:106-124` (`clock`, `fmt_clock`, e.g. "Sunrise 6:12", `:534-536`),
  `dawn_chorus.rs:935-940` (`fmt_hour`), `viz/bursts.rs:82`, `viz/funnel.rs:235`. `group_thousands` always
  inserts "," (`pages/mod.rs:602-612`). Species-name coverage is good where checked (correlation, migration,
  heatmap, behavioral, recordings all go through `names.*`; the companion box maps typed names,
  `correlation.rs:354-356`).
  Status: VERIFIED-BY-READING (that `display-format.js` leaves unmarked SVG text alone is CONJECTURED; probe: set 12-hour, open Today, read the sun rail)
- Proposed change: route chart clock text through `display::clock_attr`; a `NumberFormat` keyed on the station
  language for thousands separators.
- Effort: S–M

### UIC-18: Duplicated chart helpers, one with its arguments swapped; three heat scales
- Severity: low — Kind: anti-pattern — Audience: maintainers, C — NEW
- Evidence: `viz/radial.rs:61` `polar(cx, cy, r, ang)` vs `dawn_chorus.rs:931` `polar(cx, cy, angle, r)`;
  `clock` in `bars.rs:106`, `bursts.rs:82`, `funnel.rs:235`; `fmt_hour` (`dawn_chorus.rs:935`) duplicates
  `fmt_clock` (`bars.rs:117`); `pct` ×3 (`radial.rs:156`, `model_evidence.rs:272`, `admin/species/learned.rs:35`).
  Heat scales: kit square-root steps (`docs/design/charts.md` "Colour follows the job"), History's linear
  `color-mix` (`history.rs:329-336`), the orphan widget's linear `--seq` (`heatmap.rs:436-443`). History's
  calendar is a third calendar renderer beside `viz::heat::calendar`, without the effort hatch.
  Status: VERIFIED-BY-READING
- Impact: a later edit that calls the wrong `polar` draws a plausible, wrong chart; one detection beside 200
  disappears on History's linear scale (the kit's stated reason for sqrt).
- Proposed change: move `dawn_chorus` onto `radial::polar`; one `kit::clock`; render History's month with
  `viz::heat::calendar` (gains effort hatch and keyboard).
- Effort: S–M

### UIC-19: Hashed species colours on lists that show many species together
- Severity: low — Kind: polish — Audience: C — NEW
- Evidence: `atoms.rs:62-84` (doc: hashed hue "is wrong wherever several species are shown together"; measured
  2° collisions) vs calls on lists: `dashboard/partials.rs:342,426`, `species_pages.rs:373,442`.
  Status: VERIFIED-BY-READING
- Proposed change: rows are labelled, so use one neutral `--chart-1` stroke for list sparklines.
- Effort: S

### UIC-20: A dead template with inline styles and a retired link
- Severity: low — Kind: anti-pattern — NEW
- Evidence: `templates/_empty_states.html` is `include_str!`'d nowhere (only a doc mention, `empty_states.rs:4`);
  it has `style="…"` (`:46-114`) and `href="/system"` (`:75`). Status: VERIFIED-BY-READING
- Proposed change: delete it; `empty_states.rs` is the source.
- Effort: S

### UIC-21: History scans the whole table on every month click; a day page lists every detection
- Severity: medium — Kind: gap (performance) — Audience: B on a Pi — ALREADY-TRACKED(HANDOFF §5 item 9) in general; NEW in these specifics
- Evidence: `history.rs:79-82` and `:160-168` both call `detections_per_day` (no date bound,
  `birdnet-db/.../detections/read.rs:981-997`), uncached; measured at 1 271 ms on a 3.3 M-row desktop copy
  (`db_pool.rs:11-17`). `/reports/day` uses `detections_by_date` with no `LIMIT` (`read.rs:161-168`,
  `history.rs:394`). No Pi timing exists here.
  Status: VERIFIED-BY-READING (no speed claim beyond the cited desktop measurement)
- Proposed change: a `detections_per_day_between(from, to)` for the visible month (and cache the full scan for the
  first render); page the day list (the search filter already pages).
- Acceptance gate: month navigation issues a query bounded by the month (EXPLAIN uses the date index).
- Effort: S

### UIC-22: Loading vocabulary is uneven; most Patterns and Reports cards say "Loading…" in text
- Severity: low — Kind: polish — Audience: B — ALREADY-TRACKED(HANDOFF §5.19; ux-audit-2026-10 #11; CLS in HANDOFF §5.26)
- Evidence: skeletons used only by Today, Species, Migration, Dawn, Recordings and admin Quality
  (`skeletons::*` callers); text placeholders in `heatmap.rs:79-84` (When), `timeseries.html:11,20`,
  `analytics.html:14,28,40`.
- Effort: S

### UIC-23: Cached analytic figures do not say how old they are
- Severity: low — Kind: gap — Audience: R — NEW
- Evidence: `analytics_cache.rs:37` (TTL 600 s), `cached_fragment` returns a cached HTML string with its "now"
  marks baked in (`analytics_cache.rs:193-223`; e.g. heat grid's current hour, `heatmap.rs:210-219`).
  Errors are not cached (good, `:216-221`). Status: VERIFIED-BY-READING
- Proposed change: stamp "as of HH:MM" in the figure's Method/caption.
- Effort: S

### UIC-24: Delete and lock controls are drawn for visitors who cannot use them
- Severity: low — Kind: polish (IA) — Audience: B — NEW
- Evidence: `today.rs:1286-1296` renders Delete on every row with no signed-in check (no `signed_in` in
  `today.rs`); the POST is gated and answers 401 with `HX-Redirect` to login (`auth_middleware.rs:545-558`).
  Status: VERIFIED-BY-READING
- Proposed change: pass `looks_signed_in` into the row renderer; show actions to signed-in users only (or a
  single "Sign in to manage" link).
- Effort: S

### UIC-25: Graphics the stored data supports and no page draws (for chart lovers and researchers)
- Severity: medium — Kind: opportunity — Audience: C, R — NEW
- Evidence of the data: `recording_effort` (migration 27, `listening.rs:1-16`), detections with `Source`
  (index from migration 52, DESIGN_BRIEF #5), `model_id`/`model_agreement` (`model_board.rs:10-15`), clips +
  `/api/v2/spectrogram/{file}`, weather samples (Today's temp strip), solar times (`solar_times_local`). Unused
  endpoints: `/api/v2/analytics/abundance`, `/analytics/phenology` (`analytics.rs:948-1000`),
  `/api/v2/timeseries/year-over-year` (listed only in the API table, `timeseries.html:107`).
- Proposals, each with its table:
  1. **Effort-corrected phenology ridgeline**, one ridge per species, detections per listening hour by week,
     with an effort lane (detections + `recording_effort`; endpoint exists).
  2. **Arrival/departure dot plot across years** (first and last date per species per year; `phenology`
     endpoint) — the migration question as small multiples by year, which the current-year ridgeline cannot ask.
  3. **Year-over-year weekly comparison** (endpoint exists).
  4. **Species accumulation with an estimator**: the life-list curve plus a rarefaction/Chao1 band from
     per-day species incidence (detections only); label it as an estimate.
  5. **Spectrogram gallery** per species: best N clips by confidence (`best_detections_for_date` pattern) as
     small spectrogram tiles with play.
  6. **Polar activity clock relative to sunrise** per species (hours offset from local sunrise; detections +
     solar), small multiples for the top 12 — answers "is this bird early or late for the season"
     (insights-and-organisation §4 "Your bird's day in one line").
  7. **Confidence distribution per species** (the orphan `/pages/confidence-chart` revived on the species
     page, `bars::distribution`) — directly useful for judging a species threshold.
  8. **Per-source small multiples** (`Source`) and **model agreement** (`model_agreement`) once UIC-11 lands.
  9. **Weather overlay as its own strip** under daily trends (weather samples), per charts.md rule 5.
- Acceptance gate per chart: charts.md's "Adding a chart" checklist and `charts-edge.mjs`.
- Effort: M each

### UIC-26: Dawn and dusk colouring on "Detections by hour" uses fixed clock hours
- Severity: low — Kind: polish (statistical framing) — Audience: C, R — NEW
- Evidence: caption "Dawn (05:00 to 09:00) and dusk (18:00 to 22:00) bars are green" (`heatmap.rs:117`),
  `Fill::DawnDusk` (`heatmap.rs:470-488`); the sky (real sunrise) is drawn only for ranges ≤ 14 days
  (`heatmap.rs:251-256`). `frame.rs:31-39` documents three dawn definitions on purpose.
  Status: VERIFIED-BY-READING
- Impact: at 60° N in June, "dusk" bars are mid-afternoon; in December "dawn" is night.
- Proposed change: colour by hours relative to each day's sunrise/sunset when a location is set; otherwise say
  "05:00–09:00" without the word dawn.
- Effort: S

### UIC-27: The most-seen charts have no table alternative
- Severity: medium — Kind: gap (a11y, R) — Audience: B (screen readers), R — NEW
- Evidence: `table: false` in `viz/timeline.rs:691` (Today strip), `charts.rs:39,153` (species hourly/daily,
  history day, dashboard), `weekly_report.rs:320`; `viz::heat::calendar` (species year, year in review) has
  none; `accumulation_curve` has none. The kit offers it (`viz/bars.rs:596,922`).
  Status: VERIFIED-BY-READING
- Proposed change: turn `table: true` on (it is a closed `<details>`, zero visual cost); add one to the calendar
  and accumulation renderers. charts.md step 3 already requires it.
- Acceptance gate: a test enumerating kit chart call sites (`bars::hourly|daily`, `heat::calendar`,
  `accumulation_curve`) asserting a table or readout per site.
- Effort: S

### UIC-28: History's month cells are `href='#'` links driven only by htmx
- Severity: low — Kind: anti-pattern (progressive enhancement, a11y) — Audience: B — NEW
- Evidence: `history.rs:284` `<a class="rp-day…" href='#' hx-get="/pages/history-chart?date=…">`; the real page
  `/reports/day?date=` exists (`history.rs:446`). The cell's colour is the only encoding of intensity apart from
  the printed count (`:277-281`).
  Status: VERIFIED-BY-READING
- Proposed change: `href="/reports/day?date=…"` with the `hx-get` as enhancement; `aria-label="12 May, 140
  detections, 23 species"`.
- Effort: S

### UIC-29: Focus after HTMX swaps is left to chance outside a few scripts
- Severity: low — Kind: gap (a11y) — Audience: B — NEW
- Evidence: focus is managed in `layout.html:284` and in `discovery.js:2121,2231`, `skysong.js:950`. Swaps that
  replace the focused control: the review queue's pager links inside `#dr-queue` (`detection_reviews.rs:123,400`),
  History's calendar month buttons, Quarantine paging (`quarantine.rs:225,480`).
  Status: CONJECTURED (probe: Playwright, Tab to "Next" in `/detection-reviews`, press Enter, read
  `document.activeElement` after `htmx:afterSettle`; expect `body`)
- Proposed change: a shared `htmx:afterSettle` handler that moves focus to `[data-focus-after-swap]` in the new
  content.
- Effort: S

### UIC-30: Proposed simplified IA — one switch, two depths, same URLs
- Severity: medium — Kind: opportunity — Audience: B, R — NEW (builds on insights-and-organisation §3, which rejected a per-page depth switch)
- Evidence for the need: Patterns opens with research vocabulary even in the "When" group (UIC-15); the
  researcher's controls are missing (UIC-08, 10, 11); the Visits tab mixes diagnostics (UIC-14);
  insights-and-organisation §2: "a first-time reader and a researcher get the same sequence". §3 rejected a
  **per-page** Glance/Explore/Research switch ("a reader never finds what a switch hides").
- Proposal (station-wide preference, not per page, stored like Display prefs):
  * **Birder mode** (default): nav Today · Species · Recordings · Patterns · Reports · Station. Patterns shows
    When active, Dawn chorus, Migration, Who sings together; Trends' folded dashboard, Visits and Sequences
    move behind "More patterns"; every figure leads with its `insight.rs` sentence; jargon only in Method.
  * **Researcher mode**: same URLs; adds the filter bar (range, source, model, min confidence) to every
    Patterns tab, Method and export on every figure, "per listening hour" toggles (UIC-02), `/search` in the
    nav as "Detections", and the API table.
  * Both modes: Review becomes one page (UIC-07); "Settings" → "Station" (UIC-06).
- Acceptance gate: `nav.rs` tests updated deliberately; every figure in researcher mode has Method + export
  (a sweep test over `frame::figure` call sites).
- Effort: L

### UIC-31: Escaping discipline holds where sampled; it is per-call-site, not structural
- Severity: low — Kind: footgun — Audience: maintainers — NEW (observation; no defect found)
- Evidence: HTML is built by `format!`/`write!` and a chained `.replace` layout (`pages/mod.rs:247-292`, which
  orders `{{content}}` last for the reason in its comment). Sampled sinks all escape: species names
  (`recordings.rs:406-426`, `today.rs:1257-1262`, `correlation.rs:438`), JSON in single-quoted `hx-vals` via
  `escape_html` that escapes `'` (`pages/mod.rs:370-376`, `recordings.rs:491-501`), tooltips via `kit::tip`
  (charts.md), JS draws names with `textContent` (`live-detections.js:76`, `seasons.js` header). `innerHTML` in
  JS is used only on same-origin server HTML (`setup-controls.js:52,105`) and a static template
  (`listen-fx.js:321`). Inline handlers: none (grep of ` on[a-z]+="` outside tests: 0). Inline `style=` only
  in the dead template (UIC-20).
  Status: VERIFIED-BY-READING (sample, not exhaustive)
- Impact: correct today; one forgotten `escape_html` among ~640 `format!` sites (tier-c §4) is a reflection
  the CSP blunts but does not remove (HANDOFF §5.9).
- Proposed change: newtype `Html(String)` vs `Text(String)` for the render helpers so an unescaped `String`
  cannot reach a markup slot without a call.
- Effort: L

### UIC-32: Polling and the live socket overlap on Today
- Severity: low — Kind: polish — Audience: B on a Pi — NEW
- Evidence: Today polls nine regions (`today.html:26-202`: 5 min, 60 s ×6, 30 s, 15 s) while
  `live-detections.js` also delivers detections over the socket, and the feed listens for
  `birdnet:detection` (`today.html:147`). `poll-visibility.js` pauses polls in hidden tabs.
  Status: VERIFIED-BY-READING; no load measured
- Proposed change: drive count, pills and top-species refresh from the `birdnet:detection` event (throttled)
  and drop their timers to 5 min as a safety net.
- Effort: S

### UIC-33: The failed-read-as-zero sweep: gate exists and holds; blind spots named
- Severity: low — Kind: gap — Audience: B, R — ALREADY-TRACKED(tests/a_failed_read_is_not_defaulted_into_a_fact.rs; TD-3, TD-4)
- Evidence: the gate parses `src/` and `crates/birdnet-web/src/` (`tests/a_failed_read_is_not_defaulted_into_a_fact.rs:1-33`)
  with 247 classified rows (`tests/data/read_default_sweep.tsv`: handled 109, decoration 29, omitted 30,
  setting-default 30, fail-closed 18, task-panic 14, not-a-read 4, write-outcome 4, tracked TD-4 5, TD-3 2,
  legit-zero 1, cache 1). A manual grep of `unwrap_or(0)|unwrap_or_default()` in `routes/pages/**` found only
  query-parameter defaults, conversions and classified sites (e.g. corroboration, `detection_detail.rs:90-100`,
  classed decoration, row 118). Library crates outside the gate (`birdnet-db`, `birdnet-timeseries`,
  `birdnet-behavioral`): every hit is a numeric conversion or clock (`resilience.rs:493,841`,
  `analytics.rs:671,745`, etc.), none a defaulted read. The remaining *class* of this defect in my area is
  semantic, not syntactic: UIC-01 (a real count with the wrong denominator) and UIC-03 (an outage as a zero),
  which no adaptor sweep can see.
  Status: VERIFIED-BY-READING
- Proposed change: a second sweep for "a count printed with a per-day or per-listening claim" (grep `a day|per
  day|of listening|listening days`) and classify.
- Effort: S

### UIC-34: Stale module docs
- Severity: low — Kind: polish — NEW
- Evidence: `dawn_chorus.rs:4` lists `GET /analytics/dawn-chorus full page` (now a 308, `redirects.rs:33`);
  `homes/mod.rs:5-7` says Patterns folds six surfaces (seven tabs, `patterns.rs:36-72`);
  `routes/pages/mod.rs:5-18` module table omits most current modules; `species_detail` hourly empty text "No
  detections today yet." is reused for species and past days (`charts.rs:22`).
- Effort: S

---

## Not examined

- Rendered output in a browser (owned by the other agent): no claim here about pixels, overlap, contrast or CLS
  except where quoting HANDOFF.
- `/admin/*` pages and their copy, beyond the EQ curve and export links.
- The internals of `static/terrain.js`, `skysong.js`, `discovery.js`, `seasons.js`, `plate.js`, `chart-kit.js`
  beyond their header contracts and fetch-failure paths.
- A full CSS specificity/dead-override sweep of `static/css/app.css` (7 006 lines, 113 `@media` blocks); only
  counts were taken.
- Localisation of species names in notifications (Apprise, MQTT, email) and feeds' bodies beyond `share.rs:292`.
- The mdBook help content under `/help/*` and onboarding copy (2 768 lines).
- Every one of the ~640 `format!` sites for escaping; UIC-31 is a sample.
- Query plans and Pi timings: no query was run; the only timing cited is `db_pool.rs:11-17`.
- Whether `display-format.js` reformats unmarked SVG text (UIC-17) and whether swaps drop focus (UIC-29): probes named.
