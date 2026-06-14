# v3 Spine — Implementation Plan

> **Status.** Done, tested and documented: Wave A (the six-home nav spine +
> redirects + the Patterns/Reports/Station tab shells), Wave B1 (the
> Dashboard+Today merge), the **Recordings** rebuild of Wave C (the Clips
> browser + the folded Live view, with `/listen`·`/livestream`·`/live` →
> `/recordings?view=live`), and the first parts of **Wave B2** — the
> operator-grade **Station Health** surface (status banner · per-source
> *activity* panel · vitals · pipeline · diagnostics) and the **`admin/nav.rs`
> regroup** into the six labelled task groups (Health · Capture · Alerts · Data
> · Settings · Access), and the gated **Station management tabs** —
> `/station/{capture,alerts,data,settings,access}` fold the `/admin/*` render
> bodies into the `st-*` card treatment, rendered through the main shell with the
> shared Station sub-tab row but gated inside the admin router; the real forms are
> reused verbatim and keep posting to their existing `/admin/...` endpoints. The
> **301 cut** has landed too: the eight folded `/admin/*` management pages (and
> the `/admin` landing) permanently redirect to their Station tab, while the
> Health-detail pages (`overview`·`system`·`doctor`) and the all-in-one
> `/admin/settings` form stay reachable as gated fallbacks. The **Species** home
> rebuild has landed too — the List/Photos/Life-list view switcher at
> `/species?view=…` (with `/gallery`·`/life-list` redirecting in), the filter +
> search, and the `sd-*` detail page. The **Patterns** reskin has landed too —
> the full `pt-*` vocabulary is ported, every tab opens with a plain-English
> `bnb-lede`, leads with one picture, and tucks its tables/numbers behind a
> "see the numbers" `pt-disc` disclosure (chord→matrix, polar→ribbons,
> trends→full-dashboard), Behavior is a defined-in-place `pt-masonry`, and the
> When tab drops the now-duplicated dawn/phenology panels. The **Reports** reskin
> has landed too — the full `rp-*` vocabulary is ported, Weekly + Year open with
> an editorial `rp-hero` over a four-up `rp-stats` band (with the leaderboard and
> first-ever/milestone columns), and History is a month heat-calendar
> (`rp-cal`) keyed by a new `detections_per_day` query, with a day-detail panel
> (hourly bars + top species) and month navigation. Still open (Wave D): the
> per-source live state-chip/uptime/retry once the capture supervisor exposes
> status to the web layer; an honest per-day "Open day" landing for the History
> detail panel; and the remaining Wave D backlog (a11y titles, export wizard,
> doc screenshot refresh, OpenAPI).
>
> **Recordings — deliberate honest omissions (Wave D).** The mock's per-clip
> spectrogram thumbnail and clip-duration columns have no cheap, honest
> server-side source today (no stored thumbnail; duration isn't persisted), so
> the Clips grid omits both rather than render a decorative fake — tracked as
> Wave D (a cached thumbnail at extraction time; a persisted duration). Per-row
> "rare/first" badges are likewise deferred; the Rare filter chip already
> surfaces those clips.

Source of truth: the Claude Design handover packet in this directory
(`HANDOFF_v3.html` + six `*_home.html` hi-fi mockups), grounded against
`tomtom215/BirdNet-Behavior@8aec5b4` (v0.8.0) — the exact base of this branch.
The packet's `_assets/app.css` is byte-identical to production
`crates/birdnet-web/static/css/app.css`, so every unprefixed class in the
mockups already exists; the prefixed blocks (`x-`, `sp-`, `sd-`, `pt-`,
`rc-`, `rp-`, `st-`) are the documented **net-new** CSS.

The IA change: 9 primary tabs + 14 "More" destinations → **6 homes**
(Today · Species · Patterns · Recordings · Reports · Station), driven by one
rewrite of the `nav.rs` manifest, with permanent redirects so no bookmark 404s.

## Protect list (from the handoff §05 — non-negotiable)

- Nav manifest + parity tests: every screen enters through `nav.rs`.
- CSP discipline: no inline `style=`/scripts outside the nonce'd pattern;
  `data-style` + classes only (`inline_style_guard` stays green).
- Server-rendered inline-SVG charts: extend `viz/`, never a JS chart runtime.
- Honest liveness: flat idle baseline, never a fake waveform.
- OKLCH token set: **zero new tokens**; new classes are additive only.

## Deliberate deviations from the literal mockups

The mockups are static prototypes whose tabs are client-side JS and whose
deep links use `#fragments`. Fragments are not sent to the server, so they
cannot drive server-rendered, per-tab gated, Pi-cheap pages. Translation:

1. **Tabs are query params / sub-routes, not hashes.**
   `/patterns?tab=when|dawn|migration|together|trends|behavior`,
   `/reports?tab=weekly|year|history`, `/recordings?view=clips|live`,
   `/species?view=list|photos|lifelist` (the packet itself already uses
   `?view=` for Species and Recordings). Redirects target these URLs.
2. **`/quarantine` is not redirected.** The handoff table maps it to
   `/?review` but also says "page still reachable for bulk triage" — a 301
   would make that impossible. The tab disappears; Today's Review nudge links
   to `/quarantine` (renamed "Review", `active_nav = today`).
3. **Station auth split.** `/station` (Health tab) is public — it inherits
   the public `/system` page's job ("check from the field"). The five
   management tabs (`/station/capture|alerts|data|settings|access`) live in
   the gated admin router, exactly as `/admin/*` is gated today. Old admin
   page GETs 301 to their new tab; **admin POST/action/partial endpoints keep
   their `/admin/...` paths** so forms and HTMX wiring don't churn.
4. **Mobile tab bar: six slots, no "More".** With the MORE table empty and
   all six homes on the bar, the sheet would be empty — the hard-coded More
   slot and sheet are removed from `_partial_tabbar.html`.
5. **Mock chrome is not product.** `.x-statebar` (state switcher),
   `.x-notes` (design-notes panels), Google-Fonts links and the JS data
   tables exist only to make the prototypes self-contained. Production keeps
   self-hosted fonts and Rust renderers.
6. **Exact-duplicate CSS is consolidated.** The four tab-shell blocks
   (`pt-tab`/`rc-tab`/`rp-tab`/`st-tab`) and ledes are character-identical
   except for prefix → one shared `bnb-subtabs` / `bnb-lede` component
   (identical rendered pixels). `sp-thumb` (three identical copies) lands
   once. Everything visually distinct keeps its mock name and values.
7. **Never fake a backend.** Mock elements without real data behind them
   (e.g. the Access tab's session list / audit log if no store exists) are
   omitted and listed under Deferred, not stubbed with placeholder data.

## Wave A — the spine (`nav.rs` + redirects + shell)

- `PRIMARY` → six entries, all with mobile slots (glyphs from the handoff):
  `/` Today ⌂ · `/species` Species ⌬ · `/patterns` Patterns ▦ ·
  `/recordings` Recordings ♪ · `/reports` Reports ¶ · `/station` Station ⌗.
  Quarantine badge special-case retires with the tab.
- `MORE` → empty; `more_groups`/`sheet_rows` render nothing. The desktop
  "More ▾" dropdown is removed from the layout; a `?` Help icon-button joins
  the topnav right rail (opens the existing help drawer); ⌘K unchanged.
- `cmdk.rs` page table updates to the new vocabulary; long-tail destinations
  (Review, Kiosk, Changelog, Help, detail pages) stay reachable there.
- Redirect module (301): `/today→/`, `/gallery→/species?view=photos`,
  `/life-list→/species?view=lifelist`, `/heatmap→/patterns`,
  `/migration→/patterns?tab=migration`, `/correlation→/patterns?tab=together`,
  `/timeseries→/patterns?tab=trends`, `/analytics→/patterns?tab=behavior`,
  `/analytics/dawn-chorus→/patterns?tab=dawn`, `/listen→/recordings?view=live`,
  `/livestream→/recordings?view=live`, `/live→/recordings?view=live`,
  `/weekly→/reports`, `/year-in-review→/reports?tab=year`,
  `/history→/reports?tab=history`, `/system→/station`, plus per-page admin
  GETs → their Station tab. Until a home ships its new shell (Waves B/C),
  the old page renders at the new path so the site never half-breaks:
  Wave A lands the manifest + redirects in the same commit as thin
  route moves.
- Tests: rewrite nav parity tests to the new truth; add a redirect table
  test (each legacy path → expected `Location`); router-collision and
  cmdk-coverage tests updated; `tests/web_api_pages.rs` follows the moves.

## Wave B1 — Today (merge Dashboard + Today at `/`)

One template (`templates/today.html`, replacing `dashboard.html` too), five
layers, real partials:

- **Hero**: existing `/pages/today-phrase` headline (busy/quiet/record
  tiering) replaces the static "The yard is singing"; pill-row gains weather
  (new `/pages/today-weather-pill` from the cached weather table; hidden when
  empty), sunrise/sunset (scheduler solar calc), station name + coords
  (settings). Live-signal card keeps the real canvas + WS consumer and gains
  the `.x-sig-row` source picker (from configured audio sources) + "Listen
  live →" link to `/recordings?view=live`.
- **Review nudge / outage banner** (`.x-nudge`): new conditional partial —
  pending quarantine count > 0 → review strip; stale capture (deadman-style
  last-audio age) → outage variant. Absent when healthy and quiet.
- **Day strip**: `viz::day_strip` modified per the mock's documented a11y
  decision — per-detection hue dots removed; amber in-strip temperature line
  (from the weather samples already fetched) with now-dot + temp label;
  labelled sunrise/sunset lines; "now" pill; stats (peak · dawn · total)
  move into the section-header (`.x-daystats`); caption explains encodings +
  moon badge. Separate weather band retires from this page.
- **Unified log**: search + segmented filters (from old Today) above the
  live feed (`/pages/detections`, live-prepend, ~8 rows) + "Show the full
  day (N)" disclosure that lazy-loads `/pages/today-list` (tdl cards with
  Lock/Delete + Load more). `feed-row` gets the mock's fixed grid
  (one-line ellipsised names, `.x-fplay` 30px play affordance) so rows align.
- **Right rail**: top species (`.x-top` rows: avatar · name · banding code ·
  count · sparkline), best recordings (`.x-best` compact rows + first/rare
  tags), station card (new one-line health partial + "Looking back" links
  into Reports).
- **First-run**: onboarding bounce unchanged; an onboarded station with zero
  detections ever gets the hero checklist (mic/model/disk from doctor +
  health data) + illustrated empty states ("the empty hour").
- One-time "Coming from BirdNET-Pi?" ribbon mapping old vocabulary → new,
  localStorage-dismissed like the update banner.
- `/kiosk` keeps working off the same partials.

## Wave B2 — Station

- `admin/nav.rs` regroups 12 destinations → 6 task groups; the Station shell
  renders the `.st-tabs` row from it.
- **Health** (`/station`, public): overall status banner; per-source panel
  (`.st-source` cards: state chip, 24h uptime strip, last-audio freshness,
  detections today, retry/backoff line) from the capture supervisor's
  per-source status; vitals row (CPU/mem/temp/disk with meters, df-correct);
  pipeline row (last detection, queued uploads from store-and-forward,
  model, service uptime); doctor checklist. Composed from the existing
  `/system` + `/admin/overview` + doctor renderers.
- **Capture** (gated): audio sources list (+ add), recording schedule &
  retention toggles, species filter (with Preview dry-run link), thresholds
  with plain-English hints (single canonical home for detection threshold —
  the mock's "de-dup"), location.
- **Alerts**: rules list with toggles + channels with Send-test + recent
  sends table.
- **Data**: backups (now/nightly/download/restore), BirdNET-Pi import,
  quality summary + quarantine toggle, export.
- **Settings**: display prefs (theme/density/motion/contrast — folds
  `_partial_display_prefs.html`), station identity, integrations
  (BirdWeather/MQTT/feeds), kiosk launcher.
- **Access**: accounts; danger zone with the mock's lockout warning around
  the real network-bind/auth controls that exist today.
- Existing `/admin/*` GET pages 301 to their tab; POST/partial endpoints
  unchanged; settings save semantics verified before any form is split.

## Wave C — tab-shell homes

- **Species** (`/species?view=…`): controls row (view switcher + filter
  chips + search); List = existing species-list data in the `sp-table`
  treatment (rank · thumb · 14-day sparkline · count · **Avg confidence**);
  Photos = gallery content; Life list = big counters + accumulation curve +
  "New to the list"; species detail page aligned to the `sd-*` treatment.
  Wikipedia thumbs via the existing image pipeline with the gradient
  banding-code fallback.
- **Patterns** (`/patterns?tab=…`): one picture per tab + plain-English
  `bnb-lede`, numbers behind `<details>` disclosures. when = heatmap grid +
  hourly bars; dawn = circadian polar; migration = KPI trio + ridgeline;
  together = chord + matrix disclosure; trends = weekly detections +
  richness lines; behavior = sessions/retention/follow-on/diversity cards,
  every term defined in place. All charts stay the existing Rust `viz`
  renderers (composition/copy change only).
- **Recordings** (`/recordings?view=…`): Clips = `rc-row` browser
  (spectrogram thumb, badges, conf, duration, hover actions) + Select-mode
  bulk bar (lock/download/delete via existing per-item endpoints) +
  now-playing head player that docks to a floating bar on scroll
  (IntersectionObserver, reduced-motion safe). Live = honest scrolling
  sonogram (real WS frames; flat baseline when idle) + source picker +
  live-detection trickle.
- **Reports** (`/reports?tab=…`): Weekly + Year keep their editorial heroes
  with the `rp-stats` band, leaderboards, milestones, and a "Save as PDF"
  print affordance; History becomes the heat calendar (`rp-cal`) with a
  day-detail panel (hourly bars + top species + "Open day").

## CSS plan

All additions land in `app.css` under a single bannered "v3 spine" section,
sub-bannered per home, values copied from the mock `<style>` blocks
verbatim (minus mock chrome, minus the consolidations in Deviation 6).
No existing selector is modified except the two the mocks explicitly
re-spec: the unified-log `feed-row` grid (scoped under the log container)
and `main#main-content` page width — both ported as written. `print.css`
gains the Reports rules.

## Test & quality gates (every wave)

`cargo fmt --check` · `cargo clippy --workspace --all-targets` (pedantic +
nursery clean) · `cargo test --workspace` · `inline_style_guard` green ·
nav/cmdk/admin parity tests rewritten to the new truth (guards updated,
never deleted) · new redirect-table test · `tests/web_api_pages.rs` updated
to the new routes. Commit per wave; push per milestone.

## Wave D — deferred backlog (tracked, not in this branch)

From the handoff §05: axe + visual-regression CI, export wizard UI,
first-detection celebration moment, WS-reconnect polish, tokens doc,
OpenAPI, docs truth-sweep (web-api.md Basic-Auth drift). Plus anything from
the mocks that lacks a backend today (noted inline during implementation).

**Landed (Wave D).**

- **Behavior tab — the dawn "running order" card.** Surfaces v0.8.0's
  `sequence_count` + `window_funnel_events` in the Patterns → Behavior
  `pt-masonry`. A new `/pages/analytics-dawn-sequence` partial derives the
  station's leading dawn voices from its own data (top dawn-window species,
  ordered by mean time-of-day — so the card reads honestly anywhere, not just
  for the European REST defaults), then shows how *often* they sing in that
  order and the step timing of a recent morning. Reuses `pt-tbl` / `pt-disc` /
  `bh-*` — zero new CSS. `sequence_match_events` remains the one unadopted
  v0.8.0 function.

> Note: the handoff references companion docs `AUDIT_v3.html` and
> `IA_REIMAGINING.html` that were not part of the upload; the Wave D summary
> above is reconstructed from the handoff's own §05 list.
