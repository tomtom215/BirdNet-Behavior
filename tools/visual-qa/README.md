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
| `share.mjs` | Capture the `/r/{token}` share page, the tampered "gone" page, and the 404. |
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
BASE=http://127.0.0.1:8502 node share.mjs        # share-page__light__desktop.png (needs BNB_SHARE_SECRET)
cp shots/share-page__light__desktop.png ../../docs/book/images/share-page.png
```

## Full QA sweep

```bash
# light+dark desktop, with the per-page diagnostics report
BASE=http://127.0.0.1:8502 OUT=shots THEMES=light,dark VPS=xl node qa.mjs
# responsive overflow check
BASE=http://127.0.0.1:8502 OUT=shots-resp THEMES=light VPS=lg,md,sm,mobile node qa.mjs
```
