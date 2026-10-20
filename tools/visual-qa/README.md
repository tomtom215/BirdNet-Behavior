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
| `legibility.mjs` | **Measured gate.** The on-screen size of every SVG chart label on every route at 1440 and 390 px (computed font size × the SVG's screen scale), failing below 8.5 px. Charts are fixed-canvas SVGs scaled to their card, so labels authored at 8 px rendered at 2.7–5 px on a phone while every other gate was green; `static/chart-text.js` is what keeps them readable. Fails when it grades fewer than 200 labels, so a run that found no charts cannot pass. Runs in CI after `interactions.mjs`. |
| `layout.mjs` | **Measured gate.** The defects a screenshot shows and neither axe nor `qa.mjs` can: text boxes that overlap other text (a hint struck through by the input above it, a heading drawn on its eyebrow), a block of 4+ lines squeezed into a column under 130px inside a wide card, a control that ends past the card holding it, HTML text under 9px, and two chart labels (SVG `<text>`) whose turned boxes overlap — the first four checks skip every `<svg>`, so the chord diagram's names ran into each other on the manual's own picture with the gate green. Per route x theme x viewport (1440 and 390). Fails when it grades fewer than 30 routes per leg, so a run that could not reach the fixture cannot pass. Not yet wired into `a11y.yml`. |
| `base-path.mjs` | **Behavioural gate.** Against a fixture served under `BIRDNET_BASE_PATH=/bn` (`BNB_FIXTURE_ADDR`, `BNB_FIXTURE_DIR` run it beside the first), drives the pages and their scripted controls and fails on any same-origin request or socket outside the prefix. Runs in CI after `interactions.mjs`. |
| `share.mjs` | Capture the `/r/{token}` share page, the tampered "gone" page, and the 404. |
| `layout-selftest.mjs` | Holds `layout.mjs` to what a reader sees, in both directions, against the running fixture: a line cut off by `overflow: hidden` is not reported as overlapping the play button beside it, the same line left unclipped is, and two painted lines over each other are. |
| `tokens.mjs` | **Measured gate.** Reads the colours the browser *paints* (after `color-mix()`, `var()` and the theme cascade) and grades them: confidence tiers (3:1 bar on track, 4.5:1 pill text); that a tint keeps the hue of the colour it is a tint of (toasts, status discs, the waveform: a 16 % amber over white came out pink, a 16 % moss brown, until the mixes moved to OKLab); the browser's own bar (`<meta name="theme-color">` equals `--bg` in each theme, and follows the page when theme and OS disagree); the sign-in fields (3:1 edge against the card). `MUTATE='--token=value'` overrides a token through the CSSOM before grading, to watch a check go red; `CSS_OVERRIDE=old.css` grades an earlier stylesheet on the running fixture. Not wired into `a11y.yml`. |
| `motion.mjs` | **Measured gate.** With the OS asking for reduced motion, and with the operator's Motion setting on Reduced, no animation longer than a frame may be running on any route (`document.getAnimations()`); a control run with neither must find some, or the probe is not looking. `MUTATE=1` puts a 5 s animation on `<body>` to watch it fail. Not wired into `a11y.yml`. |
| `focus.mjs` | **Measured gate.** Tabs through every route in both themes (30 stops each) and reads what the browser computes on the focused element: it keeps its own border radius (a `border-radius` written onto `:focus-visible` squared off 948 of 1,156 stops), and the ring is at least 2 px and 3:1 against what is behind it. Fails when it reached fewer than 300 stops. The stops inside a browser's own media control bar (`<audio controls>` takes one per button) are counted and not graded: CSS cannot reach them. Not wired into `a11y.yml`. |
| `cls.mjs` | Cumulative Layout Shift per route, desktop and 390 px, from navigation until the network has been quiet for 1.5 s, worst first (`--fail` exits 1 over `THRESHOLD`, default 0.10). Each score is named after the first node the browser lists as moved, which is the first in document order, not the cause: the header was named first in 11 of the 12 worst loads and giving it room moved the total from 4.168 to 3.866 and no load out of the 15 over 0.1. What did was the cards under it: a one-line placeholder replaced by a four-card strip, a column that sized itself to its contents, a calendar arriving into a line. |
| `figures.mjs` | Every text node that is only a number, on every route, and whether the element holding it computes tabular figures or a monospaced face; reports the ones that do not (`--fail` exits 1). |
| `scroll-cost.mjs` | Frame interval and trace-measured raster time of a scripted scroll with the page's own CSS and with `backdrop-filter` switched off through the CSSOM, interleaved on the same page. A ratio on one machine, not an absolute; in headless Chromium's software compositor it found no difference (raster 39.9 vs 38.6 ms), which is why the sticky bars still blur. |
| `style-snapshot.mjs` | Proof that a stylesheet refactor changed nothing: the computed value of 34 properties on every element of every route x theme x viewport, taken twice from the **same running fixture** (`CSS_OVERRIDE=old.css` swaps the old stylesheet in through request interception), then diffed. |
| `book.mjs` | Regenerate `docs/book/images/*.png` at a consistent 1440 width, height-clipped for docs. |
| `book-mobile.mjs` | Regenerate `docs/book/images/mobile/*.png` (iPhone-13 class — 390 CSS px @ DPR 3 → 1170×1992). |
| `seed.py` | Deterministic demo-data generator (the community used in the screenshots). |
| `montage.mjs` | Build a contact sheet from a shots dir for quick eyeballing. |
| `probe.mjs` / `onboarding.mjs` | Font/console probe; onboarding step capture. |

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
