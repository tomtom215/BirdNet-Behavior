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
| `share.mjs` | Capture the `/r/{token}` share page, the tampered "gone" page, and the 404. |
| `book.mjs` | Regenerate `docs/book/images/*.png` at a consistent 1440 width, height-clipped for docs. |
| `seed.py` | Deterministic demo-data generator (the community used in the screenshots). |
| `montage.mjs` | Build a contact sheet from a shots dir for quick eyeballing. |
| `probe.mjs` / `onboarding.mjs` | Font/console probe; onboarding step capture. |

## Regenerate the book screenshots

```bash
BASE=http://127.0.0.1:8502 node book.mjs      # -> docs/book/images/*.png
BASE=http://127.0.0.1:8502 node share.mjs     # share-page__light__desktop.png
cp shots/share-page__light__desktop.png ../../docs/book/images/share-page.png
```

## Full QA sweep

```bash
# light+dark desktop, with the per-page diagnostics report
BASE=http://127.0.0.1:8502 OUT=shots THEMES=light,dark VPS=xl node qa.mjs
# responsive overflow check
BASE=http://127.0.0.1:8502 OUT=shots-resp THEMES=light VPS=lg,md,sm,mobile node qa.mjs
```
