# Visual QA & screenshot pipeline

Headless-Chromium tooling for QA'ing the `birdnet-web` UI and regenerating the
documentation screenshots in `docs/book/images/` from one consistent demo seed.

## Setup

```bash
cd tools/visual-qa
npm i playwright && npx playwright install chromium
```

## Generate a demo database

Start the binary once so it migrates a fresh SQLite DB, then seed it:

```bash
# in birdnet.conf: DB_PATH=/path/demo/birds.db, ANALYTICS_DB_PATH=..., IMAGE_CACHE_DIR=...
birdnet-behavior -c demo.conf --web-only --listen 127.0.0.1:8502   # migrates, then ^C
python3 seed.py /path/demo/birds.db                                # ~30k detections
```

Set `BNB_SHARE_SECRET`, `BNB_STATION_LAT=51.48`, `BNB_STATION_LON=-0.13` in the
environment, point `File_Name`s at real WAVs in `<db parent>/recordings/` for
playable audio/spectrograms, then restart the server (the SQLite → DuckDB sync
runs on boot). Warm species photos with
`GET /api/v2/species/image/<name>` for each species (common + scientific name).

## Scripts

| Script | Purpose |
|--------|---------|
| `qa.mjs` | Capture every route × `THEMES` × `VPS`; writes `report.json` flagging overflow, console errors, broken images, stuck "loading…". |
| `interactions.mjs` | **Behavioural gate.** Drives controls the way an impatient operator does — clicking twice while the first action is still in flight — and asserts they do not undo or duplicate their own work. Exits non-zero on regression; runs in CI after `axe.mjs`. |
| `onboarding-wizard.mjs` | **Behavioural gate** for the setup wizard. After "Look up my approximate location" (answered by a route fixture; the gate fails if it was never used) typing different coordinates must drop the looked-up time zone from the form the browser would send (`50.0, 5.0` was saved as `America/Los_Angeles`), while the look-up left alone, its own coordinates typed again, or the same point with a decimal comma keeps it; "Skip the rest and finish" is offered before the last step and not on it, and the last step's heading does not say "listening" over a body that says nothing is saved. `MUTATE=zone\|buttons` goes red on purpose. Needs the fixture. |
| `sonar.mjs` | **Behavioural gate** for the setup wizard's Welcome animation, in Chromium and (with `PLAYWRIGHT_BROWSERS_PATH`, `BROWSERS=chromium,firefox,webkit`) Firefox and WebKit: no SMIL `<animate>` element in the page, zero `securitypolicyviolation` events in the three seconds after load (the old SMIL rings made Firefox report ~180 a second and never fade), three CSS animations running on the rings with a `stroke-opacity` that changes between samples, and none running under the OS preference or the operator's Motion setting. `MUTATE=smil` puts the old markup back and must go red. Needs the fixture. |
| `charts.mjs` | **Behavioural gate** for the chart kit (`static/chart-kit.js`). A screenshot shows a chart at rest; everything the kit adds happens when something is *done* to it, so nothing that grades a still picture can see it break. Hover and keyboard focus each show a tooltip (streamgraph, hour bars, weekday grid, calendar) and announce it in an `aria-live` region; pointing at a band or a legend entry brings that series forward and drops the rest to `--chart-dim`; a legend click pins; reading a chart moves nothing (layout shift 0); the entrance runs for nobody who asked for reduced motion, by the OS or by the operator's Motion setting, and a control run with neither must find an animation (a probe that finds none is not looking); a chart swapped in again under the same id does not replay; a theme switch repaints the tooltip. `MUTATE=notip\|nodim\|motion\|noreplay` breaks one thing each, to watch the matching check go red. |
| `relational.mjs` | **Behavioural gate** for the chord diagram and its phone ladder, the dawn rings, the migration ridgeline and the Behavior tab: pointing at a species or a ribbon leaves exactly the marks that belong to it in the foreground, a ring-hour reads, the ridgeline's future is hatched, the "Earliest vs last year" tile does not print a number the station cannot know. `MUTATE=nofocus\|allthere\|nodim` goes red on purpose. |
| `terrain.mjs` | **Behavioural + measured gate** for the soundscape terrain and the vendored three.js: the mesh as pure geometry under node (cannot overshoot, a void day is flat), then in a browser the lazy load (not requested until 3D is chosen, hashed URL, immutable), non-blank pixels read back from the GPU, drag/keys/wheel/Home, the reading, context loss, no WebGL, reduced motion, hidden tab, pixel ratio, dispose, CSP and frame time on software GL. The camera is fitted to the canvas (`MUTATE=nofit` restores the one fixed distance): no lit pixel on the edge of the canvas and no axis label outside the stage, at 1280 and at 390 px. `MUTATE=eager\|noorbit\|nofallback\|noreading\|leak\|nofit` goes red on purpose. |
| `charts-edge.mjs` | **Measured gate** for the stations a lived-in fixture never is. Run against a fixture started with `BNB_FIXTURE_PROFILE=empty\|one\|stress` (see `screenshot_server.rs`): nothing heard; one detection; sixty species with names to 70 characters, a year of days and an hour holding 1.6 million detections. Every chart on Today, Patterns (7, 14, 30, 90 and a 365-day request), Trends, Year in review, Weekly, the life list and a species page must have no two labels overlapping, none outside the window, no `NaN`, no script error; a seven-digit hour is compact on its axis and exact in its tooltip, which stays inside the window; sixty species are eight bands and one more. Fails when it graded fewer than 20 charts (`stress`, `one`). `MUTATE=overlap` puts two labels on each other to watch it fail. |
| `legibility.mjs` | **Measured gate.** The on-screen size of every SVG chart label on every route at 1440 and 390 px (computed font size × the SVG's screen scale), failing below 8.5 px. Charts are fixed-canvas SVGs scaled to their card, so labels authored at 8 px rendered at 2.7–5 px on a phone while every other gate was green; `static/chart-text.js` is what keeps them readable. Fails when it grades fewer than 200 labels, so a run that found no charts cannot pass. Runs in CI after `interactions.mjs`. |
| `layout.mjs` | **Measured gate.** The defects a screenshot shows and neither axe nor `qa.mjs` can: text boxes that overlap other text (a hint struck through by the input above it, a heading drawn on its eyebrow), a block of 4+ lines squeezed into a column under 130px inside a wide card, a control that ends past the card holding it or is cut off by an `overflow: hidden` ancestor (the Live view's source menu, sliced at its card's edge at 320 px with the gate green), HTML text under 9px, and two chart labels (SVG `<text>`) whose turned boxes overlap — the first four checks skip every `<svg>`, so the chord diagram's names ran into each other on the manual's own picture with the gate green. Per route x theme x viewport (1440, 390, 360 and 320 px wide; `VPS=` picks a subset). Fails when it grades fewer than 30 routes per leg, so a run that could not reach the fixture cannot pass. Run by `a11y.yml`. A sixth check, `widen`, grades the page as a whole: `scrollingElement.scrollWidth > clientWidth` is reported with the widest element no scrolling ancestor holds (a 385 px identifier in a hint widened `/station/alerts` to 434 px on a 390 px phone and dragged the fixed tab bar with it, and none of the checks above looks at the document). `CSS_OVERRIDE=old.css` grades an earlier stylesheet, to watch a check go red against the CSS it was written for. |
| `base-path.mjs` | **Behavioural gate.** Against a fixture served under `BIRDNET_BASE_PATH=/bn` (`BNB_FIXTURE_ADDR`, `BNB_FIXTURE_DIR` run it beside the first), drives the pages and their scripted controls and fails on any same-origin request or socket outside the prefix. Runs in CI after `interactions.mjs`. |
| `live-logs-connect.mjs` | **Behavioural gate, three engines.** The live log viewer reads "Connected" within `LIMIT_MS` (3 s) in Chromium, Firefox and WebKit (`ENGINES=` picks; an engine asked for that cannot launch is a failure). Firefox fires an `EventSource`'s `open` on the first byte of the *body*, the others on the headers, and on a quiet log the stream's first body byte was the 15-second keep-alive: Firefox alone said "Connecting…" for up to 15 s (measured 15.4 s on the old build, 0.6 s on the fix). Needs the fixture (`BASE`). |
| `sw-state.mjs` | **Behavioural gate, three engines.** The service worker (`static/sw.js`) is dormant, and this holds it so. It was registered with no `scope`, so its scope was `/static/` and `navigator.serviceWorker.controller` was null on every dashboard page in Chromium, Firefox and WebKit: its offline behaviour has never run. Pins, per engine: a worker registers and activates in `<base>/static/` and no wider (run it with `BASE=…/bn` against the base-path fixture: the registration used to ask the host's root for `/static/sw.js` and register nothing); `/`, `/kiosk` and `/station` are not controlled after a reload; Cache Storage holds only `bnb-static-*` and no HTML document (`Cache.addAll` ignores `no-store`, so a precached `/` kept a private station's dashboard after sign-out); nothing leaves the prefix; and the counterpart, a worker given scope `/` in the same engine does control the page, so the "not controlled" checks cannot pass because the probe is blind. Enabling the worker is a decision with a list (top of `sw.js`), not a side effect; this fails when it happens without one. Needs the fixture (`BASE`). |
| `dom-sinks.mjs` | **Behavioural gate.** Plants hostile values (a `javascript:` URL, another origin, a quote-bearing path) in the `data-*` attributes that five scripts read back into a sink (`recordings.html` download link and bulk download, the help drawer's "Open full page" and its fallback link, the command palette's `location.href`) and fails if one reaches it; the ordinary path is the counterpart for each, so a gate that refused everything cannot pass. Also: the HLS listener id is a UUID, or 32 hex characters from `crypto.getRandomValues` where `randomUUID` is missing (plain http), and the page never calls `Math.random` for it. Needs the fixture (`BASE`, default `http://127.0.0.1:8502`). Run by `a11y.yml`. |
| `listen-fx.mjs` | **Measured gate.** The listening aids (`static/listen-fx.js`: boost, rumble cut, compressor) in a real browser: real clips (`LISTEN_FX_CLIPS`, plus the fixture's own) are rendered through the page's own graph on an `OfflineAudioContext` at settings up to +24 dB with the compressor, and the output peak must stay under the limiter's ceiling; the same settings without the limiter must go over full scale on some clip, or the check could not see a clip; neutral settings must leave the level alone and a played clip unrouted; a cross-origin source is never routed; the controls work from the keyboard, remember their value and survive `localStorage` throwing. `MUTATE=1` removes the limiter and must go red. `SHOTS=dir` writes light/dark, 1440/390 pictures. Not in CI yet: needs the fixture and a clip. |
| `display-format.mjs` | **Measured gate.** The browser half of the clock and temperature formatter (`static/display-format.js`): its output on every case of `crates/birdnet-web/tests/data/display_cases.json`, the table the Rust formatter is held to; a 12-hour choice on the Display card re-prints every marked time on the detection page and the station default restores it; a station default of `12h` / `fahrenheit` (`FIXTURE_DB` points at the fixture's database) reaches a browser with no choice and a browser's own choice beats it; a time or temperature that arrives after load is printed; blocked `localStorage` leaves the canonical text and no page error. `MUTATE=1` breaks the 12-hour branch and must go red. |
| `share.mjs` | Capture the `/r/{token}` share page, the tampered "gone" page, and the 404. |
| `layout-selftest.mjs` | Holds `layout.mjs` to what a reader sees, in both directions, against the running fixture: a line cut off by `overflow: hidden` is not reported as overlapping the play button beside it, the same line left unclipped is, and two painted lines over each other are. Also the `widen` check in both directions: a line that cannot wrap is reported and named, the same line in a box that scrolls sideways is not, and a long identifier in a `.hint` must not widen the page. And the cut-off form of `escape`: a control wider than its `overflow: hidden` box is reported, the same control in a box that scrolls or one that fits is not. |
| `qa-selftest.mjs` | Holds `qa.mjs`'s stuck-"Loading…" check to what a reader sees, in both directions, against the running fixture: a placeholder inside a closed `<details>` (or a `hidden` element) is not reported (Chromium 141 leaves `offsetParent` set for it, so `/patterns` was reported stuck in all four states), the same text on the page or in an open disclosure is. |
| `tokens.mjs` | **Measured gate.** Reads the colours the browser *paints* (after `color-mix()`, `var()` and the theme cascade) and grades them: confidence tiers (3:1 bar on track, 4.5:1 pill text); that a tint keeps the hue of the colour it is a tint of (toasts, status discs, the waveform: a 16 % amber over white came out pink, a 16 % moss brown, until the mixes moved to OKLab); the browser's own bar (`<meta name="theme-color">` equals `--bg` in each theme, and follows the page when theme and OS disagree); the sign-in fields (3:1 edge against the card). `MUTATE='--token=value'` overrides a token through the CSSOM before grading, to watch a check go red; `CSS_OVERRIDE=old.css` grades an earlier stylesheet on the running fixture. Run by `a11y.yml`. Also every visible placeholder on five real pages, graded at 4.5:1 as the browser paints it (`::placeholder`'s computed colour and opacity over the field's fill): the engines' own defaults measured 2.35:1 (WebKit), 3.8:1 (Firefox) and 4.08:1 (Chromium dark). |
| `motion.mjs` | **Measured gate.** With the OS asking for reduced motion, and with the operator's Motion setting on Reduced, no animation longer than a frame may be running on any route (`document.getAnimations()`); a control run with neither must find some, or the probe is not looking. `MUTATE=1` puts a 5 s animation on `<body>` to watch it fail. Run by `a11y.yml`. |
| `focus.mjs` | **Measured gate.** Tabs through every route in both themes (30 stops each) and reads what the browser computes on the focused element: it keeps its own border radius (a `border-radius` written onto `:focus-visible` squared off 948 of 1,156 stops), and the ring is at least 2 px and 3:1 against what is behind it. Fails when it reached fewer than 300 stops. The stops inside a browser's own media control bar (`<audio controls>` takes one per button) are counted and not graded: CSS cannot reach them. Run by `a11y.yml`. |
| `cls.mjs` | Cumulative Layout Shift per route, desktop and 390 px, from navigation until the network has been quiet for 1.5 s, worst first (`--fail` exits 1 over `THRESHOLD`, default 0.10). Each score is named after the first node the browser lists as moved, which is the first in document order, not the cause: the header was named first in 11 of the 12 worst loads and giving it room moved the total from 4.168 to 3.866 and no load out of the 15 over 0.1. What did was the cards under it: a one-line placeholder replaced by a four-card strip, a column that sized itself to its contents, a calendar arriving into a line. |
| `figures.mjs` | Every text node that is only a number, on every route, and whether the element holding it computes tabular figures or a monospaced face; reports the ones that do not (`--fail` exits 1). |
| `scroll-cost.mjs` | Frame interval and trace-measured raster time of a scripted scroll with the page's own CSS and with `backdrop-filter` switched off through the CSSOM, interleaved on the same page. A ratio on one machine, not an absolute; in headless Chromium's software compositor it found no difference (raster 39.9 vs 38.6 ms), which is why the sticky bars still blur. |
| `chart-ramps.mjs` | **Measured gate, no browser.** Reads the sequential, diverging and data-ink tokens from `app.css` and grades them in both themes: lightness moves one way, neighbours are 7-13 dE (sequential) or 10-24 dE (diverging arms) apart, the first step is 8 dE clear of an empty cell, the last is 3:1 on the card, the two diverging poles are 15 dE apart under protanopia and deuteranopia, the diverging middle is neutral, and the grid, axis, label, annotation and tooltip ink clear the contrast the token page states. `MUTATE='--seq-3=oklch(30% 0.1 60)'` overrides a token before grading, to watch it fail. The categorical set is graded by `the_chart_palette_can_be_told_apart.rs`. Run by `a11y.yml`. |
| `surface-layers.mjs` | **Measured gate, no browser.** The layers around the content: the page's two washes leave `--fg-3`, `--fg-2` and the link colour at 4.5:1 where they are strongest, the translucent nav keeps `--fg-3` at 4.5:1 over the strongest tint scrolled under it, dark surfaces step up in the direction of "nearer", and the focus ring is 3:1 against the card and the page and 2 px wide. `MUTATE` as above. Run by `a11y.yml`. |
| `token-doc.mjs` | Writes (`--write`) or checks (`--check`) the generated tables and the two token counts in `docs/book/reference/design-tokens.md` from `app.css`. The description of each token lives in `GROUPS` beside its name; the value is never typed. `--check` is run by `a11y.yml`. |
| `token-swatches.mjs` | Draws every token in `app.css` as a swatch, shadow, radius, space and type sample, in both themes, from the stylesheet the fixture serves: `docs/book/images/design-tokens__light.png` and `__dark.png`. |
| `colour-math.mjs` | OKLCH to sRGB, WCAG contrast, OKLab dE, Machado 2009 colour-blind simulation and compositing, shared by the two gates above. |
| `style-snapshot.mjs` | Proof that a stylesheet refactor changed nothing: the computed value of 34 properties on every element of every route x theme x viewport, taken twice from the **same running fixture** (`CSS_OVERRIDE=old.css` swaps the old stylesheet in through request interception), then diffed. |
| `book.mjs` | Regenerate `docs/book/images/*.png` at a consistent 1440 width, height-clipped for docs. |
| `book-mobile.mjs` | Regenerate `docs/book/images/mobile/*.png` (iPhone-13 class — 390 CSS px @ DPR 3 → 1170×1992). |
| `seed.py` | Deterministic demo-data generator (the community used in the screenshots). |
| `montage.mjs` | Build a contact sheet from a shots dir for quick eyeballing. |
| `probe.mjs` / `onboarding.mjs` | Font/console probe; onboarding step capture. |

## The fixture's clock

The fixture seeds today only up to the time of day, and never fewer than 24 detections (the Today page pages its feed, and `interactions.mjs` needs more than one page of it): when fewer have happened by the clock, the first of the rest are spread over the hours that have, over at least three. `BNB_FIXTURE_NOW=<unix seconds>` replaces the clock the **seeding** reads, so a gate can be run as if it were 02:00 or 23:00 without waiting (`BNB_FIXTURE_NOW=$(( $(date -u -d 'today 02:00' +%s) )) cargo run -p birdnet-web --example screenshot_server`); the server itself still reads the real clock, so choose a time on the real UTC date.

## Interaction gate

```bash
cargo run -p birdnet-web --example screenshot_server --features analytics &
node interactions.mjs                  # CHROMIUM_PATH=... to use a preinstalled browser
```

Every check here exists because the rest of the suite could not have caught the
bug that prompted it: 0.13.0 shipped a Listen → Live button that cancelled its
own stream on the second click. The server streamed correct MP3 throughout, the
page rendered perfectly, axe was clean and every screenshot looked right — the
defect lived entirely in what the *second* click did, and nothing drove a
control twice. Media playback and in-flight requests are stubbed or held open on
purpose: the bug window is the interval before they complete, so a check that
waits for success cannot see it.

When adding a control that starts slow work, add a case here.

## Regenerate the book screenshots

```bash
BASE=http://127.0.0.1:8502 node book.mjs         # -> docs/book/images/*.png (desktop)
BASE=http://127.0.0.1:8502 node book-mobile.mjs  # -> docs/book/images/mobile/*.png
BASE=http://127.0.0.1:8502 node book-onboarding.mjs   # -> docs/book/images/onboarding-accuracy.png
BASE=http://127.0.0.1:8502 node share.mjs        # share-page__light__desktop.png
cp shots/share-page__light__desktop.png ../../docs/book/images/share-page.png
```

`DETAIL_NAME`, `DETAIL_DATE` and `DETAIL_TIME` pick the detection that `book.mjs`,
`book-mobile.mjs` and `share.mjs` open. It has to be one the fixture holds *and*
one whose clip exists (the fixture writes audio for its newest 20): the old
`detection-detail.png` was the "Detection not found" page for a time the
fixture never seeded, and nothing said so.

## Full QA sweep

```bash
# light+dark desktop, with the per-page diagnostics report
BASE=http://127.0.0.1:8502 OUT=shots THEMES=light,dark VPS=xl node qa.mjs
# responsive overflow check
BASE=http://127.0.0.1:8502 OUT=shots-resp THEMES=light VPS=lg,md,sm,mobile node qa.mjs
```
