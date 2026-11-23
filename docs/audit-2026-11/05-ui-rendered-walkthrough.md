# 05 — The UI as rendered: a walkthrough by eye

Audit date 2026-11-22. Judged from screenshots of the running fixture, looked at one by one, from three
seats: a **birder** who has never opened a terminal, a **chart-lover**, and an **academic researcher**.
Source lines are cited only where the cause was located after the defect was seen. Every finding below
says whether it was **SEEN-IN-RENDER** or **READ-ONLY**. All 37 findings are SEEN-IN-RENDER; none
rests on template reading alone.

## What was built and run

| Item | Value |
|---|---|
| Commit | `9d8bb1c43f4e689ba91a2301f2c5397fc31e310f` (2026-11-12), working tree clean except `docs/audit-2026-11/` |
| Build | `CARGO_INCREMENTAL=0 cargo build -p birdnet-web --example screenshot_server --features analytics` — cold, `Finished dev profile in 8m 53s`, exit 0 (the ort cache was already seeded; `setup-onnxruntime.sh` was not needed) |
| Binary | `target/debug/examples/screenshot_server`, mtime 2026-11-22 17:28:11 UTC |
| Populated fixture | `BNB_FIXTURE_DIR=<scratch>/fixture-main/…long-path…`, `127.0.0.1:8502`, pid 15641, started **17:28:16 UTC** (5 s after the binary was written, so it is this build) |
| Empty fixture | same binary, `BNB_FIXTURE_PROFILE=empty`, `127.0.0.1:8504`, pid 15709, started 17:28:23 UTC |
| Start script | `<scratch>/ui-audit/start-fixture.sh` (fresh dir, nohup, pid file, polls `/`) |
| Browser | Playwright 1.63.0 from `/opt/node-tools`, Chromium build 1243 from `/opt/pw-browsers`; `@axe-core/playwright` 4.x installed in the scratch copy only (`tools/visual-qa/` has no `node_modules`; nothing was written there) |
| Capture script | `<scratch>/ui-audit/qa/audit-capture.mjs` — imports `ROUTES` from an unmodified copy of `qa.mjs` |

`<scratch>` is `/tmp/claude-0/-home-user-BirdNet-Behavior/99b13c55-75d7-508d-9e77-66322782cc68/scratchpad`.

Two things about the captures:

* **Reduced motion is on in the audit captures.** A first pass without it produced full-page pictures of
  Today with an empty hour chart: Playwright's `fullPage` resizes the viewport, and resizing replays the
  chart kit's entrance (a probe saw 10 `<g>` animations restart when the width changed to 390 px). The
  viewport-only "fold" picture of the same load had the bars. Every picture listed below was taken with
  `reducedMotion: 'reduce'` so that what is graded is the chart at rest (see UIR-33).
* The fixture clock was the real one (17:28 UTC on a Sunday in November, after sunset at the
  fixture's London coordinates), so Today says "evening" and the kiosk's `?night=` forms were both graded.

### Routes × viewports × themes captured

* **Populated fixture, every gate route plus 3 extras:** the 46 entries of `qa.mjs` `ROUTES` (resolved
  detection and species from the fixture's API: Tufted Titmouse, 2026-11-22 17:26:44) plus
  `/station/logs`, `/help` and `/changelog` (the last three answered 404: `/help/` needs the root crate's
  book render, which the fixture does not have; the real changelog is `/system/changelog`, captured below;
  `/station/logs` does not exist). 49 routes × {1280×900, 390×844 touch} × {light, dark} × {full page,
  fold} = **392 PNGs** in `<scratch>/ui-audit/shots/` named `<route>__<theme>__<desk|phone>[_fold].png`.
  `facts.json` beside them holds title, h1, h2s, page height and overflow per state. No state overflowed
  horizontally.
* **Empty fixture (first run):** dashboard, species, life list, patterns, trends, behavior, recordings,
  reports, year, station, data, onboarding, species-detail, kiosk — 14 × 2 × 2 × 2 = **112 PNGs** in
  `<scratch>/ui-audit/shots-empty/`.
* **Pages no gate visits** (light only): `/system/changelog`, `/admin/accounts`, `/admin/audio`,
  `/admin/backups`, `/admin/migrate`, `/admin/notifications`, `/admin/quality`, `/admin/rules`,
  `/admin/species`, `/search?q=jay`, `/analysis-runs` (404) — 11 × 2 × 2 = **44 PNGs** in
  `<scratch>/ui-audit/shots-extra/`. The `/admin/*` ones redirect into Station tabs (same heights).
* Crops and contact sheets used for close reading: `<scratch>/ui-audit/crops/`.

Total: **548 screenshots**. Every desktop-light full page of the 46 gate routes was opened and read; dark
and phone states were read through contact sheets (fold shots at 0.5–0.8 scale) plus full-resolution crops
where something looked wrong. That is the limit of "looked at": a dark-theme or phone defect on a page
below its fold may have been missed.

### Gate outputs (run here, against the populated fixture, unmodified scripts)

| Gate | Command | Summary line, quoted |
|---|---|---|
| `qa.mjs` | `STRICT=1 OUT=<scratch>/ui-audit/gate-qa-shots node qa.mjs` | `Captured 184 screenshots …` / `=== 0 pages with issues ===` (exit 0) |
| `axe.mjs` | `VPS=desktop,mobile node axe.mjs` | `=== axe: 0 total violation(s); 0 at/above [minor, moderate, serious, critical] ===` / `--- advisory (WCAG 2.2 + best-practice): 0 finding(s), PROMOTED TO BLOCKING ---` / `PASS: no blocking accessibility violations.` (180 states, all "clean"; exit 0) |
| `layout.mjs` | `node layout.mjs` (default: 1440, 390, 360, 320 px, both themes) | `Graded 360 page states.` / `layout: no overlaps, squeezed columns, escaping controls, sub-9px text, colliding chart labels or page wider than the screen.` (exit 0) |

Logs: `<scratch>/ui-audit/gate-{qa,axe,layout}.log`. The other gates (`interactions`, `legibility`,
`tokens`, `focus`, `motion`, …) were not run.

**What the green gates did not see.** Every gate above was green, and the render still showed a rank column
that breaks "10" over two lines (UIR-06), a heat table whose cells are never coloured (UIR-03), literal
Markdown asterisks on a page every footer links to (UIR-07, a route no gate visits), a sentence that names a
different "busiest" weekday from the bars beside it (UIR-02), and copy that says "darker" over a dark-theme
chart where more is lighter (UIR-04). None of those is the kind of thing these gates measure; they are listed
under each finding's acceptance gate.

### Fixture limits that affect what could be judged

* Every species picture is the fixture's flat grey JPEG, so the Photos grid, avatars, life-list portraits and
  the kiosk plates are grey squares or dark blobs. Photo cropping, credits under real photos and the
  engraving were **not judged**.
* The notification history is seeded with times later than "now" and species never detected (Bald Eagle);
  the message format `… detected at 0.79` is the fixture's (`screenshot_server.rs:1064`), not the product's.
* The fixture is a web-only process with seeded model rows; the Capture tab's "Classifier status is not
  available in this process" next to Health's two running models (UIR-12) may be that, not the product.

## Per-page verdicts

"5-second" is what a first-time birder would take away.

| Page (route) | 5-second verdict | Top issues |
|---|---|---|
| Today `/` | Clear and warm: "A calm evening", 24 detections, a feed. | Header "Source stopped", hero green "recording", live card "idle" disagree (UIR-11); per-row "waveform" is decoration (UIR-05); species colours here differ from every other page (UIR-01); temperature in Robin's orange, no unit (UIR-23) |
| Today, empty station | Excellent first-run: "Getting to know your yard", a checklist, next actions. | Checklist ticks "Microphone recording" while 2 of 3 sources are down (UIR-11); "(0 so far)" |
| Species list `/species` | Ranked list, legible. | Rank "10"–"15" break over two lines (UIR-06); two search boxes (header + page); sparklines each scaled to own max with no scale; counts left-aligned |
| Life list | Celebratory, two curves. | Same curve drawn twice; only 4 of 15 portraits named, the rest hover-only (UIR-25); ISO vs "27 Nov 2025" dates (UIR-20) |
| Photos | (fixture grey) | Not judged |
| Occurrence lookup | Genuinely useful answer page. | "Held back" offers no action (UIR-37); week numbers |
| Species detail | Rich; good. | Four numbers shown twice (UIR-24); "CIRCADIAN"; CLI flag `--metadata-labels` in prose (UIR-17); companions defined differently from Patterns (UIR-22) |
| Species detail, unheard | Seven empty cards for a bird never heard. | Should be one explanation pointing at "Why isn't a bird heard here?" (UIR-26) |
| Patterns · When active | Beautiful, dense, four figures. | Weekday sentence contradicts the bars (UIR-02); clock-defined dawn/dusk vs sun-defined dawn on the same page (UIR-18); 12a/6a axis vs 00/03 axis (UIR-19); Fig 4 clipped on a phone (UIR-29) |
| Patterns · Dawn chorus | Striking radial; legible small multiples. | Ring colours ≠ Today's colours (UIR-01); "off-chorus" judged against 05:00–08:59 clock window in November (UIR-18); static pills look like filters (UIR-36); no n per species (UIR-30) |
| Patterns · Trends | Two tidy charts. | The question in the title is never answered; Fig 1 and Fig 2 cover different 30-day windows; "Low 0 · 23 Oct" sits on a bar (UIR-34); "API endpoints" card (UIR-17) |
| Patterns · Who sings together | Gorgeous chord. | Full colour ramp spent on 1–3 %; "darker" is wrong in dark (UIR-04, UIR-22); "e.g. European Robin" on a North American station |
| Patterns · Migration | Clear ridgeline, honest "not yet" hatch. | "amber" is orange-red; spring/fall by hue only; "w39"; "R. Hummingbird" (UIR-36, UIR-23) |
| Patterns · Visits (behavior) | "Research tier" — honest, but heavy. | Retention cells uncoloured despite a colour key (UIR-03); DuckDB/extension/config table on a birder page (UIR-17) |
| Patterns · Sequences | Ambitious; hard to read. | Running-order lines in fragments (ALREADY-TRACKED HANDOFF §5.33); "Nothing stands out" beside a giant disc; truncated names with room to spare; banding codes as headers (UIR-21) |
| Recordings · Clips | A list of spectrograms; where is Play? | Actions hidden until hover (UIR-10); fourth row design (UIR-28) |
| Recordings · Live | "Idle" over an empty white box. | Spectrogram card blank when idle; "— frames"; copy promises "never a fake waveform" above rows that carry one (UIR-05) |
| Reports · Weekly | "A loud week so far" — confident. | "+315 %" against a week with 2 days off (UIR-08); red "0 new" (UIR-23); content column 100 px right of its head (UIR-27) |
| Reports · Year | Delightful; calendar + bump chart. | Blue Jay is red here (UIR-01); spaghetti bump chart; red "10 new" |
| Reports · History | Calendar + day panel. | "3411 sp" collisions, off days look like future days (UIR-09); "darker" wrong in dark (UIR-04) |
| Reports · Day | A list of the day. | No page head, no prev/next day (UIR-27); yet another row design (UIR-28) |
| Settings · Health | The best page in the app for "is it working?" | Hidden under a nav item called "Settings" (UIR-16); uptime strips have no time axis or key; no action from "Needs attention" |
| Settings · Capture | 6,100 px of forms. | "The station's watch folder The station takes…" (UIR-13); a badge on every field; BirdNET-Pi keys in every hint (UIR-17); model card contradicts Health (UIR-12) |
| Settings · Alerts | "Get a nudge" → a rules engine. | Webhooks/structured logs first, push channels last and unset (UIR-32) |
| Settings · Data | Thorough. | Shows `DISK_PURGE_THRESHOLD` instead of 95 %; "copying them elsewhere is on the Data tab" on the Data tab; nested H1 (UIR-14, UIR-27) |
| Settings · General | Good Display card. | Clock and temperature asked twice (device and station) with no explanation of which wins beyond hint text |
| Settings · Access | Clear. | `CADDY_PWD`; "Signed in as Administrator" over "No active sessions" |
| Login | Clean. | — |
| Onboarding | Warm, seven steps. | First thing asked is an admin password (fine), sonar glyph floats |
| Detection detail | Best researcher page: models, bars, review. | Headline 96 % does not name its model (UIR-30); no source/mic shown on a 3-source station; spectrogram without axes |
| Detection reviews / Quarantine | Functional queues. | Nav highlights "Species"; Reject vs Delete (UIR-35) |
| Notifications | Fixture data only | Nav highlights "Settings" (UIR-35) |
| Kiosk (day/night) | A handsome plate; fixture pictures are blobs. | Latin primary, common name small caps; mode links nearly invisible; raw host URL in the credit line; phone opens dark Numbers in the light theme (UIR-31) |
| Picture credits | Clear table. | — |
| Admin (`/admin`, `/admin/*`) | A second settings app. | Parallel IA, duplicated forms, contradictory numbers, emoji icons, half-empty layouts (UIR-15, UIR-27) |
| Changelog `/system/changelog` | Broken. | Literal `**`, every bullet cut to its first line, 37,000 px tall (UIR-07) |
| Search `/search` | Powerful, legacy look. | Sans H1, "Erithacus rubecula" placeholder, US date inputs (UIR-20, UIR-27) |
| 404 | Friendly. | — |

## Findings

### UIR-01: A species changes colour from chart to chart
- Severity: high — Kind: bug — Audience: chart-lover, birder — NEW
- Evidence: SEEN-IN-RENDER. `shots/dashboard__light__desk.png` (Northern Cardinal green, Blue Jay blue,
  Mourning Dove purple); `shots/patterns-dawn__light__desk.png` (Blue Jay green, Northern Cardinal blue,
  Mourning Dove orange); `shots/patterns-together__light__desk.png` (Mourning Dove blue, Northern Cardinal
  orange); `shots/reports-year__light__desk.png` (Blue Jay red/pink); `shots/patterns-behavior__light__desk.png`
  (American Robin purple). Cause: `crates/birdnet-web/src/routes/pages/viz/kit.rs:587` `slot_class(i)` gives
  `ck-c{i+1}` by the series' rank *within that chart*; `docs/design/charts.md:101` says "identity is
  `--chart-1..8` in fixed order", which is fixed per chart, not per bird.
- What the person experiences: having learned "the green one is the Cardinal" on Today, the reader opens
  Dawn chorus and the green ring is the Blue Jay. Every chart needs its legend re-read; cross-chart
  comparison by colour is actively misleading.
- Proposed change: one station-wide species→slot map (the eight most-heard species of, say, the last 90 days
  own slots 1–8 in rank order, recomputed rarely and cached; everyone else `--chart-other` or a neutral
  stroke), passed to every chart builder; a chart whose top eight differ uses the station slots for the ones it
  shares and the remaining slots for the rest. Optionally let the owner pin colours.
- Acceptance gate: a Rust test rendering Today's day strip, the dawn rings, the chord, the bump chart and the
  bursts timeline from one fixture asserts each species carries the same `ck-cN` class in all of them; red
  against `slot_class` by rank.
- Effort M

### UIR-02: "No weekday stands out: the busiest, Thursday" beside bars where Saturday is longest
- Severity: high — Kind: bug — Audience: chart-lover, researcher — NEW
- Evidence: SEEN-IN-RENDER. `crops/pat-fig4.png` from `shots/patterns__light__desk.png` (1280, light): margin
  bars Sun 145, Mon 81, Tue 99, Wed 147, Thu 159, Fri 156, **Sat 163**; sentence names Thursday.
  `crates/birdnet-web/src/routes/pages/insight.rs:118-147` averages per *listened* day of each name (Saturday
  occurs 5 times in the 30 days, Thursday 4; two Mondays/Tuesdays were not listening), while the margin
  bars are raw totals and are not labelled as such.
- What the person experiences: the headline sentence looks wrong; the one place the app "says what it
  found" contradicts its own picture, and a researcher loses trust in every other sentence.
- Proposed change: draw the margins in the unit the sentence uses ("average per listened day") with the day
  count under each ("4 days"), or say "per listened day" in the sentence and add the counts. Hatch or
  annotate a weekday that lost days to an outage.
- Acceptance gate: test that the weekday the sentence names is the argmax of the values drawn in the margin;
  red on today's code with this fixture.
- Effort S

### UIR-03: Retention heat table never shows its colour
- Severity: medium — Kind: bug — Audience: chart-lover — NEW
- Evidence: SEEN-IN-RENDER. `crops/retention.png` (Patterns › Visits, 1280 light): cells are plain text under
  a seven-step green key. Probe: `td.rt-cell.ck-m7` computes `background-color: rgba(0,0,0,0)`. Cause:
  `static/css/app.css:6257` `.rt-table th, .rt-table td { … background: none; }` (specificity 0,1,1) beats
  `:6266` `.rt-cell { background: var(--c, …) }` (0,1,0).
- What the person experiences: a legend for a colour scale nothing uses; the table reads as a wall of
  95 %/100 %.
- Proposed change: `.rt-table td.rt-cell { background: var(--c, var(--chart-track)); }` and check text
  contrast on the darkest step.
- Acceptance gate: `getComputedStyle` of a `.rt-cell.ck-m7` is not transparent, in both themes; plus
  `tokens.mjs`-style 4.5:1 for its text.
- Effort S

### UIR-04: "Darker means more" is written over dark-theme charts where more is lighter
- Severity: medium — Kind: bug — Audience: chart-lover, birder — NEW
- Evidence: SEEN-IN-RENDER. `crops/hist-dark.png` (`shots/reports-history__dark__desk.png`): lede "Darker
  squares were busier", busiest day 19 Nov is the brightest cell. `crops/d1.png` chord in dark: strongest
  ribbons are pale yellow under "A wider, darker ribbon means…". Strings:
  `routes/pages/history.rs:133`, `routes/pages/correlation.rs:74`, `viz/matrix.rs:200`, `viz/radial.rs:277`.
- What the person experiences: in dark mode the instruction tells them to read the chart backwards.
- Proposed change: theme-neutral wording, as the activity grid already uses: "a stronger colour means
  more".
- Acceptance gate: a source test refusing "darker"/"lighter" in chart copy (named exemptions), seen failing
  on the four strings.
- Effort S

### UIR-05: The per-row "waveform" is invented
- Severity: medium — Kind: bug (honesty) — Audience: birder, researcher — NEW
- Evidence: SEEN-IN-RENDER. Every row on Today and Recordings › Live (`shots/recordings-live__light__desk.png`)
  carries a 24-bar glyph; they look alike because they are: `routes/pages/atoms.rs:215-238` draws a bell
  envelope from a pseudo-random seed. The same page says "The signal is honest: … never a fake waveform",
  and `docs/book/guide/today.md:50` calls the glyph "a mini call-waveform".
- What the person experiences: a birder reads shape into it ("that one was a long trill"); a researcher who
  learns it is decoration discounts the page's other honesty claims.
- Proposed change: replace with a real amplitude envelope computed when the clip is extracted (24 RMS values
  stored with the detection), or the spectrogram thumbnail Recordings › Clips already shows, or nothing.
- Acceptance gate: a test that a feed row's glyph heights come from stored clip data (or that no `.waveform`
  is rendered for a detection without it); manual text updated.
- Effort M

### UIR-06: Species list rank "10"–"15" break over two lines on a desktop
- Severity: medium — Kind: bug — Audience: birder — NEW
- Evidence: SEEN-IN-RENDER. `crops/species-rank.png` (`/species`, 1280 light): "1/0", "1/1", "1/2". Cause:
  `static/css/app.css:3870` `.sp-rank { width: 30px }` with `td` padding `var(--space-10)` each side
  (`:3867`).
- What the person experiences: the list looks broken from row 10 on.
- Proposed change: `.sp-rank { width: auto; min-width: 3ch; white-space: nowrap; text-align: right }`.
- Acceptance gate: `layout.mjs` rule: no text node that is only digits wraps (line count > 1); seen red here.
- Effort S

### UIR-07: The changelog prints raw Markdown and cuts every entry to its first line
- Severity: medium — Kind: bug — Audience: all — NEW
- Evidence: SEEN-IN-RENDER. `crops/changelog.png` (`/system/changelog`, linked from every footer):
  "**Which classifiers heard a detection, and a Models board.** A detection's page shows a *Who heard it*
  card:" — and the bullet ends there; "…Perch v2's 14 795 classes as a". `routes/pages/changelog.rs:193`
  reads one line per bullet; `render_inline_md` (`:240`) handles code and links, not emphasis. The page is
  37,233 px tall on desktop, 65,080 on a phone. No gate route visits it.
- What the person experiences: the "What's changed" page looks unfinished and is unreadable.
- Proposed change: join continuation lines into the bullet; render `**`/`*`; show the latest three releases
  with older ones collapsed.
- Acceptance gate: unit test of a two-line bold bullet → `<strong>` and the whole sentence; add
  `/system/changelog` to `ROUTES`.
- Effort S

### UIR-08: "A loud week, +315 %" against a week the station was partly off
- Severity: medium — Kind: bug (claim) — Audience: birder, researcher — NEW (related to ux-audit-2026-10 #3,
  which fixed like-for-like *days*, not effort)
- Evidence: SEEN-IN-RENDER. `shots/reports__light__desk.png`: "315 % busier than the same 6 days last week";
  the Trends tab and the terrain both hatch 9–10 Nov (Mon, Tue of last week) as "not listening".
- What the person experiences: the headline adjective is produced by an outage, and nothing on the page
  says so.
- Proposed change: compare per listened day (from `recording_effort`) and say when the comparison week lost
  days: "last week the station was off for 2 of those 6 days". Suppress the adjective when effort differs by
  more than a set share.
- Acceptance gate: fixture with an effort gap in the previous week; the sentence names the gap and the
  percentage is per listened day.
- Effort M

### UIR-09: History calendar numbers collide, and an off day looks like a future one
- Severity: medium — Kind: bug — Audience: birder, chart-lover — NEW
- Evidence: SEEN-IN-RENDER. `crops/hist-cal.png` (`/reports?tab=history`, 1280): "34" and "11 sp" run
  together as "3411 sp"; three unlabelled numbers per cell; 9 and 10 Nov (not listening) are blank like 23–30
  Nov (future). charts.md honesty rule 3 says off, quiet and not-yet are three things.
- What the person experiences: misreads counts; cannot tell an outage from the future.
- Proposed change: date small top-left, detections as the one big number, species as "11 kinds" on its own
  line or in the tooltip; hatch off days, outline future days, as the activity calendar does.
- Acceptance gate: `layout.mjs` text-overlap already exists — extend it to the calendar's inline runs (it
  missed this); a DOM test that a day with no effort carries the hatch class.
- Effort S

### UIR-10: On Recordings, Play is invisible until you hover
- Severity: medium — Kind: gap — Audience: birder — NEW
- Evidence: SEEN-IN-RENDER. `shots/recordings__light__desk.png` (rows with no controls) vs `crops/rec-hover.png`
  (Play, download, lock, delete appear on hover). Probe: every `Play …` button reports
  `checkVisibility() === false` at rest. On the phone they show (touch).
- What the person experiences: the page called "Listen in" has no visible way to listen; a laptop with a
  touchpad user may never discover hover. Today's feed shows its play button always.
- Proposed change: always show Play (the primary action); keep secondary actions on hover/focus or in a
  "⋯" menu.
- Acceptance gate: at 1280 with no pointer, `button[aria-label^="Play"]` in the first row is visible.
- Effort S

### UIR-11: Three signals disagree about whether the station is recording
- Severity: medium — Kind: bug — Audience: birder — NEW
- Evidence: SEEN-IN-RENDER. `shots/dashboard__light__desk.png`: header pill "Source stopped" (amber), hero
  pill "● recording" (green), live card "idle — No audio in the last 30 seconds"; Health says two of three
  sources stopped. Empty station `shots-empty/dashboard__light__desk.png`: checklist "✓ Microphone
  recording · USB microphone · Backyard mic +2" beside the same header pill. Hero logic
  `routes/pages/today.rs:640-660` treats any live source as "recording". The header pill says "Source" for two.
- What the person experiences: "is it working or not?" has three answers on one screen.
- Proposed change: one capture state with three values (all / some / none) used by header, hero pill and
  checklist: "recording · 1 of 3 sources" in amber, linked to Health.
- Acceptance gate: fixture with one healthy and two faulted sources → hero pill text contains "1 of 3" and
  is not the green class; checklist row not ticked.
- Effort S

### UIR-12: Capture tab says the classifier status is unavailable while Health lists two running models
- Severity: low — Kind: bug (possibly fixture) — Audience: birder — NEW
- Evidence: SEEN-IN-RENDER. `crops/cap-top.png` "Classifier status is not available in this process" vs
  `shots/station__light__desk.png` "birdnet-v3 Running · perch-v2 Running". The fixture is web-only with
  seeded rows, so a real station may not show this; the wording "in this process" is jargon either way.
- Proposed change: Capture reads the same model rows Health reads, or says "the detector is not running in
  this window — see Health".
- Acceptance gate: same fixture, both tabs name the same models.
- Effort S

### UIR-13: "The station's watch folder The station takes…"
- Severity: low — Kind: bug — Audience: birder — NEW
- Evidence: SEEN-IN-RENDER. `crops/cap-watch.png` (Capture tab). `crates/birdnet-web/src/watch_folder.rs:91`
  falls back to a sentence fragment when the folder is unknown.
- Proposed change: fallback "Recordings are read from the station's watch folder." (a sentence).
- Acceptance gate: unit test of `capture_note(None)` ending in a full stop before the next sentence.
- Effort S

### UIR-14: The Data tab prints a setting's name where its value belongs, and refers to itself
- Severity: low — Kind: bug — Audience: birder — NEW
- Evidence: SEEN-IN-RENDER. `crops/station-data-top.png`: Retention "Clips purged at
  `DISK_PURGE_THRESHOLD`" (General says 95 %); the Backups card at the foot: "copying them elsewhere is on
  the Data tab" — on the Data tab. "Snapshots kept 1 — first automatic backup due shortly" beside a
  snapshot taken 2 minutes ago.
- Proposed change: print the value ("when the disk is 95 % full") with a link to change it; drop the
  self-reference.
- Acceptance gate: DOM test that no `UPPER_SNAKE` config key appears as a value on Station tabs.
- Effort S

### UIR-15: Two settings applications that disagree with each other
- Severity: high — Kind: gap (IA) — Audience: birder — ALREADY-TRACKED(DESIGN_BRIEF "Known problems" 6;
  `qa.mjs` "Admin pages that have not folded into a Station tab yet"); the contradictions are NEW
- Evidence: SEEN-IN-RENDER. `crops/admin-settings-top.png`: `/admin/*` has its own 12-item nav (Overview,
  System, Diagnostics, Audio, Species, Rules, Notifications, Backups, Migration, Quality, Settings, Accounts)
  beside the Station tabs; "Detection Settings" appear on Capture and on Admin Settings (11,822 px); Danger
  zone on Data and on Admin System. `shots/admin-system__light__desk.png`: "Recordings 0 audio files · 0 B"
  while Data says "Recordings (20 clips) 4.8 MB"; "uptime 21m" and "Uptime 3m 32s" on one page; CPU 16 %
  (Health), 9 % (System), 6 % (Overview) within a minute. Admin Doctor: "No configuration file path is
  known … run `birdnet-behavior --doctor` on the host" — a dead end.
- What the person experiences: they cannot tell which place is the real one, and the numbers differ.
- Proposed change: finish the fold — every `/admin/*` page either becomes a section of a Station tab or a
  308 to one; one source for each number; label "host uptime" vs "service uptime".
- Acceptance gate: `qa_routes_cover_the_navigation.rs` extended so no route renders the admin nav; a test
  that the recordings count on Data and System come from one function.
- Effort L

### UIR-16: "Is it working?" lives under a nav item called Settings
- Severity: medium — Kind: gap (IA) — Audience: birder — NEW
- Evidence: SEEN-IN-RENDER. Top nav Today · Species · Patterns · Recordings · Reports · Settings; Settings
  opens on "Health — is it working?" (`shots/station__light__desk.png`). The `nav.rs:79` comment records the
  rename as deliberate.
- What the person experiences: a birder whose station has gone quiet does not think "Settings". The header
  pill links there, which helps only when something is already flagged.
- Proposed change: call the sixth item "Station" (health first, configuration after), or move Health's
  summary to Today's "Your station" card with a direct link.
- Acceptance gate: n/a (decision); then `nav.rs` test.
- Effort S

### UIR-17: Engine internals and command lines on pages written for birders
- Severity: medium — Kind: polish (plain language) — Audience: birder — ALREADY-TRACKED(DESIGN_BRIEF item 10,
  the standing rule); these instances NEW
- Evidence: SEEN-IN-RENDER. Patterns › Visits and Sequences open with "Connected · Analytics Engine · DuckDB
  analytics database connected … duckdb-behavioral extension" and end with a "Configuration" figure holding
  the SQLite path and "Analytics Compiled true" (`shots/patterns-behavior__light__desk.png`); Trends: "API
  endpoints — for scripts & integrations"; species page: "For whoever set it up: `--metadata-labels`
  supplies them"; Access: `CADDY_PWD`; Admin System: `sudo install.sh`; every Capture hint ends in a
  BirdNET-Pi key; Alerts: "structured INFO entry".
- Proposed change: move engine status and configuration to Health/Diagnostics; keep BirdNET-Pi key names
  behind a "for BirdNET-Pi users" disclosure; replace CLI flags with what the owner can do in the browser.
- Acceptance gate: extend the plain-language test (if one exists) with a deny-list (`DuckDB`, `--[a-z]`,
  `[A-Z_]{6,}`) on public pages, with named exemptions.
- Effort M

### UIR-18: Dawn and dusk are defined by the clock on some charts and by the sun on others
- Severity: medium — Kind: bug (method) — Audience: researcher, chart-lover — NEW
- Evidence: SEEN-IN-RENDER. `/patterns` Fig 1: "28 % … within two hours either side of sunrise"; Fig 3 on
  the same page: "the dawn hours, 05:00 to 09:00, hold 28 %", bars green for 05–09 and 18–22. Dawn tab:
  "The shaded band is the dawn window, 05:00 to 08:59", "Blue Jay peaks 09:00 · off-chorus", while the same
  page says sunrise 07:28, sunset 16:04. Green also means "Northern Cardinal" on Today.
- What the person experiences: in November, "dusk" bars are two hours after dark; a researcher cannot use
  "off-chorus".
- Proposed change: one definition, sun-relative (civil dawn −1 h to sunrise +2 h, say), stated once in
  the Method note; fall back to clock hours only without coordinates and say so. Use a neutral emphasis, not
  a series colour.
- Acceptance gate: test that the dawn band moves with the fixture's sunrise across two seasons.
- Effort M

### UIR-19: Two hour-axis vocabularies
- Severity: low — Kind: polish — Audience: chart-lover — NEW
- Evidence: SEEN-IN-RENDER. 12a/6a/12p/6p on the terrain, dawn rings, bursts and running order
  (`crops/pat-fig1.png`, `crops/seq-fig1.png`); 00/03/06 on Today, Fig 3, species and day charts. The Display
  card's Clock setting changes neither.
- Proposed change: one axis formatter fed by the Clock setting.
- Acceptance gate: `display-format.mjs` extended to SVG tick labels.
- Effort S

### UIR-20: Five date formats
- Severity: low — Kind: polish — Audience: birder — NEW
- Evidence: SEEN-IN-RENDER. ISO "2026-11-22" (feeds, species, weekly range), "Sun 26 Jul" (calendar note),
  "27 Nov 2025" and "Thu 27 Nov 2025" (life list), "w39" (migration), mm/dd/yyyy (audit log and search date
  inputs, the browser's locale).
- Proposed change: human dates in prose ("Sun 22 Nov"), ISO only in tables meant for export; one formatter.
- Acceptance gate: `display-format.mjs` case table gains dates.
- Effort M

### UIR-21: "Nothing stands out" beside a disc that dominates the figure
- Severity: medium — Kind: polish (honesty) — Audience: chart-lover, researcher — NEW
- Evidence: SEEN-IN-RENDER. `crops/seq-fig4.png` (Sequences Fig 4): one cell of count 3 drawn as a disc
  ~10× the others; row names "American Goldfin…", "Black-capped Chi…" truncated with ~150 px free;
  columns headed BLJA/NOCA/AMGO (banding codes).
- Proposed change: scale disc area against a fixed reference (or the expected count), mute the figure when
  the sentence says nothing stands out; full names (wrap) and common-name column heads with the code on
  hover.
- Acceptance gate: test that with max count < threshold the largest disc ≤ a set radius; `layout.mjs`
  "text truncated while its box has room".
- Effort S

### UIR-22: Co-occurrence: the strongest tie is 3 %, drawn as if it were strong; three definitions of "together"
- Severity: medium — Kind: gap (method) — Audience: researcher, chart-lover — NEW
- Evidence: SEEN-IN-RENDER. `shots/patterns-together__light__desk.png`: key "1 % to 3 % of the blocks either
  was heard in", full seven-step ramp across it; no chance expectation. Fig 2 lookup: "same day"; species
  page: "same days … over the last 30 days"; chord: "same five-minute block". Placeholder "e.g. European
  Robin" on a North American station (Search: "Erithacus rubecula").
- Proposed change: show lift over chance (observed ÷ expected from each species' own block share) or at least
  the expected value in the tooltip; anchor the ramp at 0; one definition named the same everywhere;
  placeholder from the station's own list.
- Acceptance gate: test that the colour step for a pair at the chance level is the lowest step.
- Effort M

### UIR-23: Small encodings that say the wrong thing
- Severity: low — Kind: polish — Audience: birder, chart-lover — NEW
- Evidence: SEEN-IN-RENDER. Red numerals for good news ("10 new to your list" on Year, "0 new" on Weekly);
  "11° / 2°" with no unit; Today's temperature line in the same orange as American Robin directly above its
  key (`crops/temp.png`); "Peak diversity week w39"; "R. Hummingbird"; uptime strips on Health with no time
  axis or key.
- Proposed change: neutral ink for counts; °C/°F; temperature in `--chart-annot`, not a series hue; "late
  Sep (week 39)"; full names; "last 24 h, each bar 30 min" under the strips.
- Acceptance gate: `amber_is_never_a_text_colour`-style test for "rare" red on positive counts.
- Effort S

### UIR-24: Species page shows its four numbers twice and opens with "Circadian"
- Severity: low — Kind: polish — Audience: birder — NEW
- Evidence: SEEN-IN-RENDER. `shots/species-detail__light__desk.png`: pills "1,063 detections / First heard /
  Last heard / avg 84 %" then tiles with the same four; eyebrow "CIRCADIAN"; three link buttons styled like
  tabs with the first filled as if selected.
- Proposed change: keep the tiles, drop the pills; "Daily rhythm"; links as plain text links.
- Effort S

### UIR-25: Life list draws one curve twice and names 4 of 15 birds
- Severity: low — Kind: polish — Audience: birder, chart-lover — NEW
- Evidence: SEEN-IN-RENDER. `shots/species-lifelist__light__desk.png`: "Your growing list" area curve and the
  "Firsts" step curve show the same accumulation; portraits labelled only for 4 birds, the rest by hover; a
  five-line keyboard paragraph.
- Proposed change: one chart; label every portrait that fits, alternate above/below; move the keyboard help
  behind a "?".
- Effort S

### UIR-26: A bird never heard gets a full, empty species page
- Severity: low — Kind: gap — Audience: birder — ALREADY-TRACKED in part(HANDOFF §5.19 empty vocabulary;
  ux-audit-2026-10 #11)
- Evidence: SEEN-IN-RENDER. `shots/species-detail-unheard__light__desk_fold.png`: seven cards each "Not heard",
  "All its detections →" for zero. Patterns on the empty station shows three different empty phrasings on
  one page (`shots-empty/patterns__light__desk.png`).
- Proposed change: one panel: "Not heard here yet" + the occurrence answer for this week + "set an alert for
  it".
- Effort S

### UIR-27: Page heads, H1 faces and content columns differ by page
- Severity: low — Kind: polish — Audience: all — ALREADY-TRACKED in part(ux-audit-2026-10 #1; DESIGN_BRIEF 1)
- Evidence: SEEN-IN-RENDER. Admin Settings and Search: sans-bold H1; Data tab: a nested display H1 "Backups &
  recovery" under the Station tabs, which otherwise have no H1; Day page: no page head and no prev/next day;
  Weekly: content column starts 100 px right of its head; Admin Overview: emoji icons; Admin System/Doctor/
  Audit: fixed-width cards leave half of a 1280 screen empty.
- Proposed change: `page_head` on every route including Station tabs and Day; one container width.
- Acceptance gate: `every_home_opens_with_the_shared_page_head` extended to Station tabs, Day, Search, admin.
- Effort M

### UIR-28: Six designs for "a detection"
- Severity: low — Kind: polish — Audience: birder — NEW
- Evidence: SEEN-IN-RENDER. Today feed (avatar, waveform, bar, play), Recordings clip row (time stack,
  spectrogram, hidden actions), Day page (card per row, bar inline), species-detail table (pill + time + date,
  no play), Quarantine table (player inline, four buttons), Search table (checkbox, source chip).
- Proposed change: one detection row component with density variants; source chip and play everywhere.
- Effort M

### UIR-29: Phone: hidden tabs and clipped charts
- Severity: medium — Kind: bug — Audience: birder — ALREADY-TRACKED in part(HANDOFF §5 "species hourly chart
  and the weekly chart still scale down on a phone")
- Evidence: SEEN-IN-RENDER. `crops/p1.png`, `crops/p2.png`: Patterns and Station sub-tabs scroll sideways
  with the last items cut ("Trends busier or quie…", "Data", "General", "Access" off-screen) and no affordance;
  `crops/ph-grid.png`: the activity grid ends at 13:00, the weekday totals off-screen; keyboard instructions
  ("arrow keys… Shift") printed on touch; tab-bar glyphs ⌬ (Species), ¶ (Reports), ⌗ (Settings) carry no
  meaning.
- Proposed change: a "More ▾" or two-row sub-nav; fade edge on scrollers; draw the grid for 360 units with
  3-hour columns; hide keyboard help on `pointer: coarse`; real icons.
- Acceptance gate: `layout.mjs` rule: a nav scroller whose last item is off-screen without a visible
  affordance.
- Effort M

### UIR-30: Numbers without provenance
- Severity: medium — Kind: gap — Audience: researcher — ALREADY-TRACKED in part(HANDOFF §5.31 Method panels
  on six cards only)
- Evidence: SEEN-IN-RENDER. Detection detail's headline "96 %" names no model (birdnet-v3 is in the card
  below); species list "Avg confidence" does not say over which model on a two-model station; no n per
  species on dawn rings or small multiples; Trends and Reports show no recording effort beyond hatched days;
  model version appears only on Health and detection detail.
- Proposed change: a small "n = …, model …, listened … h" foot on every figure (from the existing Method
  panel), and model name beside any confidence.
- Effort M

### UIR-31: Kiosk details
- Severity: low — Kind: polish — Audience: birder — ALREADY-TRACKED in part(HANDOFF §5.22 small displays)
- Evidence: SEEN-IN-RENDER. `crops/kiosk-tl.png`, `crops/kiosk-bot.png`: Latin name primary, common name in
  small caps under it; legend in Latin only; "Plate · Latest · Numbers" links near-invisible and over the frame
  rule; credit line prints `127.0.0.1:8502/system/credits`; at 390 px the light theme opens a dark Numbers
  view.
- Proposed change: common name primary on a household display (Latin on the plate caption); mode links at
  4.5:1; "Picture credits: Settings → Credits" or a QR code.
- Effort S

### UIR-32: Alerts is a rules engine where a birder expects "tell me when…"
- Severity: medium — Kind: gap — Audience: birder — NEW
- Evidence: SEEN-IN-RENDER. `crops/station-alerts-top.png`: lede "Get a nudge when something special happens",
  then "Alert Rules — webhooks, structured logs, or notification suppression", default action "Log
  (structured INFO entry)", confidence as 0.0–1.0, days as "1=Mon…7=Sun", species pattern shown as `%Owl%`
  in the table but `*Owl*` in the help; push channels at the bottom: "No destination resolved … set
  Notification URLs … in Settings and restart".
- Proposed change: first card "Tell me when" (species picker, "first of the year / rare / any", channel
  picker with phone push set up inline); rules engine below as "Advanced".
- Effort M

### UIR-33: Resizing replays chart entrances (and empties full-page screenshots)
- Severity: low — Kind: footgun — Audience: chart-lover, tooling — NEW
- Evidence: SEEN-IN-RENDER. First capture pass, `fullPage` without reduced motion: Today's hour chart empty;
  probe: after `setViewportSize(390)` 10 `<g>` animations restart. `book.mjs`/`book-mobile.mjs` take
  full-page pictures and could ship an empty chart.
- Proposed change: run the entrance once per element (`data-ck-id` already exists for swaps) and not on
  resize; book scripts set reduced motion.
- Acceptance gate: `charts.mjs`: after a resize, `getAnimations()` on chart groups is empty.
- Effort S

### UIR-34: Trends asks a question and does not answer it
- Severity: medium — Kind: gap — Audience: birder, researcher — NEW
- Evidence: SEEN-IN-RENDER. `crops/trends.png`: title "Is your yard getting busier?", no sentence, no % change;
  Fig 1 "last 30 complete days" (23 Oct–21 Nov) beside Fig 2 "last 30 days" (24 Oct–22 Nov, today partial,
  the comparison line falling to 0 at the right edge); "Low 0 · 23 Oct" drawn on the first bar; the 7-day
  average starts on day one.
- Proposed change: an `insight.rs` sentence per listened day ("12 % busier than the 30 days before, 1 more
  species"), one window for both figures, today excluded or marked partial.
- Effort S

### UIR-35: Queues with no home in the navigation
- Severity: low — Kind: gap (IA) — Audience: birder — NEW
- Evidence: SEEN-IN-RENDER. Detection reviews and Quarantine highlight "Species"; Notifications highlights
  "Settings"; reachable from a Today banner and links only. Quarantine rows carry Approve / Reject / Delete /
  Share with no explanation of Reject vs Delete.
- Proposed change: an inbox badge in the header ("6 to review") opening one review page with two tabs;
  "Not this bird (keep the recording)" vs "Delete recording".
- Effort M

### UIR-36: Migration and Dawn: pills that look like filters, colour that names a season
- Severity: low — Kind: polish — Audience: chart-lover — NEW
- Evidence: SEEN-IN-RENDER. `shots/patterns-migration__light__desk.png`: "Calendar year 2026", "Seasonal
  birds only", "vs. last year" and Dawn's "Last 60 days", "Per-species normalized" are `<span class="bnb-pill">`
  styled like the range buttons; text says "amber in the fall" over orange-red; spring/fall told by hue
  (green vs orange-red) and band shading only.
- Proposed change: render static facts as plain caption text; label the peak markers "spring"/"fall"
  directly; check the pair under deuteranopia with `chart-ramps.mjs`.
- Effort S

### UIR-37: The occurrence answer stops at "Held back"
- Severity: low — Kind: gap — Audience: birder — NEW
- Evidence: SEEN-IN-RENDER. `shots/species-occurrence-answer__light__desk.png`: "Held back: the range model
  gives it 0.8 % here in week 22; this station admits species at 3 %" — no button to add the bird to Always
  allowed, which the same paragraph mentions. (Health says "Species filter off"; the fixture seeds the
  occurrence table separately, so this may be a fixture inconsistency.)
- Proposed change: "Allow Painted Bunting anyway" button (admin) beside the answer.
- Effort S

## Top 15 polish wins (value ÷ effort)

1. UIR-01 stable species colours across every chart (M, high).
2. UIR-02 weekday sentence and margins in the same unit (S, high).
3. UIR-03 retention cells coloured — one CSS selector (S).
4. UIR-06 rank column no longer breaks "10" (S).
5. UIR-07 changelog renders bold and whole bullets (S).
6. UIR-04 theme-neutral "stronger colour" wording (S).
7. UIR-10 Play visible on Recordings (S).
8. UIR-11 one recording state: "1 of 3 sources" (S).
9. UIR-09 history calendar: one number per cell, off days hatched (S).
10. UIR-34 Trends answers its own question; one window (S).
11. UIR-08 weekly comparison per listened day, gaps named (M).
12. UIR-05 remove or make real the per-row waveform (M).
13. UIR-13 + UIR-14 watch-folder sentence, `DISK_PURGE_THRESHOLD` value, self-reference (S).
14. UIR-23 neutral ink for good-news counts, units on temperature, temperature off the series palette (S).
15. UIR-16 rename "Settings" → "Station" (S, decision).

## Not examined

* Real photographs, credits under real photos, the engraving and plate quality (fixture pictures are grey).
* Help pages (`/help/*` is 404 on the fixture; the root build renders them) and `/r/{token}` share pages.
* The onboarding wizard beyond step 1 (no step was driven), and every form's error states after a bad submit.
* Interactions: tooltips, hover focus, legend pinning, the 3D terrain, keyboard navigation of charts,
  the command palette, live audio playback and the live spectrogram with audio arriving.
* `BNB_FIXTURE_PROFILE=one` and `stress` (long names, 60 species, 1.6 M in an hour) — only `empty` was run.
* Dark and phone states were read through contact sheets and crops, not every full page at full resolution.
* Widths other than 1280 and 390 (the layout gate covered 1440/390/360/320 mechanically).
* High-contrast, compact/comfy density, Plate chart style, 12-hour clock, Fahrenheit.
* Firefox, WebKit, a real phone, a Raspberry Pi screen, a screen reader.
* Gates other than qa, axe and layout.

Teardown: both fixtures were stopped by their pid files (15641, 15709), and both pids were gone a second
later. `df -h /` afterwards: 252G size, 14G used, 55G available (21 %). Before the cold build it read 10G
used and 59G available.
