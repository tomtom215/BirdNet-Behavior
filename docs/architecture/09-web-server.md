# Web Server

> axum-based REST API, Server-Sent Events, HTMX pages, and admin panel.

## Table of Contents

- [Overview](#overview)
- [Server Architecture](#server-architecture)
- [Application State](#application-state)
- [REST API Endpoints](#rest-api-endpoints)
- [HTMX Pages](#htmx-pages)
- [Admin Panel](#admin-panel)
- [Live streams](#live-streams)
- [Design Decisions](#design-decisions)

---

**Status: ✅ Fully implemented** in `crates/birdnet-web/`

## Overview

The web server replaces the Python FastAPI application with an axum server
embedded in the single binary. It serves:

- **REST API** (`/api/v2/*`) for programmatic access and HTMX partial updates
- **WebSocket** (`/api/v2/ws/detections`, `/api/v2/ws/spectrogram`) for live detection and spectrogram streams
- **Server-Sent Events** (`/admin/system/logs`) for the live admin log viewer
- **HTMX pages** — fully server-rendered homes (Today, Species, Patterns, Recordings, Reports, Station) plus admin
- **Admin panel** — settings editor, system info, backup management, log viewer, update check
- **Static assets** — species images cached from Wikipedia, embedded HTMX JS

## Server Architecture

```
axum Router
├── Middleware Stack
│   ├── TraceLayer (structured request/response logging)
│   ├── CorsLayer (permissive for development)
│   ├── security_headers_middleware (CSP, security headers, base-path rewrite)
│   └── CompressionLayer (skips text/event-stream, gRPC and image/*)
│
├── /api/v2/                           REST API
│   ├── GET /                          API info
│   ├── GET /health                    Health check
│   ├── GET /stats                     Detection/species counts
│   ├── GET /detections                List detections (date?, limit?)
│   ├── GET /detections/recent         Recent N detections
│   ├── POST /detections/delete        Delete a detection (bearer-gated write API)
│   ├── GET /ws/detections             WebSocket live detection feed
│   ├── GET /species/top               Top species by count
│   ├── GET /species/activity          Hourly activity for date
│   ├── GET /species/image/{scientific_name}  Species image (cached)
│   ├── GET /recordings/{filename}     Serve WAV recording file
│   ├── GET /analytics/*               DuckDB analytics (see below)
│   └── GET /timeseries/*              Time-series analytics (see below)
│
├── (top level)                        HTMX server-rendered homes
│   ├── GET /                          Today — detection table + live updates
│   ├── GET /species                   Species home (list, photos, life list)
│   ├── GET /patterns                  Patterns home (heatmap, trends, behaviour)
│   ├── GET /recordings                Recordings home (clips, live audio)
│   ├── GET /reports                   Reports home (weekly, year, history)
│   └── GET /station                   Station health
│
├── /pages/*                           HTMX fragments for those homes
│
└── /admin/                            Admin panel
    ├── GET  /admin/settings           Settings form
    ├── POST /admin/settings           Save settings
    ├── GET  /admin/system             System info + backup controls
    ├── POST /admin/system/backup      Trigger backup now
    ├── GET  /admin/system/backups     List backup files
    ├── GET  /admin/system/backups/{name} Download backup file
    ├── DELETE /admin/system/backups/{name} Delete backup (HTMX)
    ├── GET  /admin/system/logs        SSE live log feed
    └── GET  /admin/system/logs/page   Admin log viewer
```

## Application State

Shared state using `Arc<AppState>`:

```rust
pub struct AppState {
    inner: Arc<AppStateInner>,
}

struct AppStateInner {
    db: Mutex<rusqlite::Connection>,     // the single SQLite writer
    readers: Option<ReaderPool>,
    recording_dir: PathBuf,
    analytics_db: …,
    image_cache: …,
    detection_broadcast: …,
    log_broadcaster: …,
    spectrogram_broadcast: …,
    // …and roughly twenty more, all private
}
```

`AppState` is a cheap-to-clone newtype over `Arc<AppStateInner>`; every
field is private and reached through accessor methods, so handlers cannot
reach past the invariants the accessors enforce. The backup directory is
derived from the database path rather than stored.

- Migrations run automatically on startup
- All SQLite access goes through `with_db()` closure
- Handlers use `tokio::task::spawn_blocking` for DB queries
- `tokio::sync::broadcast` ring buffers feed the live streams: detections 256 slots (`DEFAULT_BROADCAST_CAPACITY`), spectrogram 16 (`SPECTROGRAM_BROADCAST_CAPACITY`), log lines 512 (`LOG_CHANNEL_CAPACITY`)

## REST API Endpoints

Every path in this section is under `/api/v2` (the routers are nested with
`.nest("/api/v2", …)` in `routes/mod.rs`), and the tables name the main routes
by the group they belong to; they are not the whole list. The whole list, with
parameters and response shapes, is the manual's
[HTTP & WebSocket API](../book/reference/api.md) page, and
`crates/birdnet-web/openapi.json` is the machine-readable form. Two
gates hold it to the router in both directions: `every_documented_path_is_routed`
(every documented path is served) and
`every_route_the_api_serves_is_in_the_openapi_document` (every served
`/api/v2` method and path is documented, read from the router's own `Debug`
output); the same file checks the document's structure (references, path
parameters, operation ids, response descriptions). A route added without its
entry fails by name.

### System

| Endpoint | Response |
|----------|----------|
| `GET /api/v2/` | `{ name, version, status }` |
| `GET /api/v2/health` | `{ status: "healthy"/"degraded", version, database, … }`: the full body is on the manual's API page |
| `GET /api/v2/stats` | Summary counts (detections, species, today) |
| `GET /api/v2/metrics` | Prometheus text format |

### Detections

| Endpoint | Query Params | Response |
|----------|-------------|----------|
| `GET /api/v2/detections` | `date?`, `species?`, `source?`, `limit?`, `offset?` | `{ detections: [...], total_count }` |
| `GET /api/v2/detections/recent` | `limit?` | `{ detections: [...] }` |
| `GET /api/v2/detections/export` (and `/export/birddb`, `/export/ebird`, `/export/raven`) | `from?`, `to?`, `format?` | CSV / JSON / BirdDB.txt / eBird / Raven downloads |
| `POST /api/v2/detections/delete` | body: detection id | Bearer-gated write API (`routes/api_write.rs`); the Today page's HTMX rows post to `/pages/today-delete` instead |
| `GET /api/v2/ws/detections` | — | WebSocket live detection feed (there is no SSE detection stream) |

### Species

| Endpoint | Query Params | Response |
|----------|-------------|----------|
| `GET /api/v2/species/top` | `limit?` (default 20) | `{ species: [{ name, count, avg_confidence }] }` |
| `GET /api/v2/species/activity` | `date` (required) | `{ activity: [{ hour, count }], date }` |
| `GET /api/v2/species/image/{scientific_name}` | — | Image metadata JSON or 404; the `/file` suffix serves the cached image |

### Recordings & Logs

| Endpoint | Response |
|----------|----------|
| `GET /api/v2/recordings` | The files in the recordings directory (`species?`, `limit?`, `offset?`) |
| `GET /api/v2/recordings/{filename}` | The clip, streamed, honouring `Range` (`206`); the type follows the extension |
| `GET /admin/system/logs` | `text/event-stream` SSE of log lines (admin session required; not under `/api/v2`) |

### Analytics

Behavioural analytics (DuckDB) live under `/api/v2/analytics/*`:
`sessions`, `retention`, `funnel`, `funnel-events`, `patterns`,
`sequence-count`, `sequence-match-events`, `next-species`, `abundance`,
`phenology`, `status`.

Trend, heatmap and seasonal work is a separate surface under
`/api/v2/timeseries/*`: `daily`, `weekly`, `hourly`, `diversity`,
`trend`, `year-over-year`, `peak-windows`, `gaps`, `anomalies`,
`accumulation`, `heatmap`, `sessions`, `status`.

See [08-behavioral-analytics.md](08-behavioral-analytics.md) for the
query semantics behind each.

## HTMX Pages

All pages use server-side HTML rendering with inline styles (Tailwind-like utility
classes) and HTMX for dynamic updates. No client-side JavaScript framework.

| Page | Route | Key Features |
|------|-------|-------------|
| Today | `/` | Detection table with audio player, live updates, review controls |
| Species | `/species` | All detected species with image thumbnails, counts, confidence; photo and life-list views |
| Patterns | `/patterns` | Hour×weekday activity chart, trends, co-occurrence, behaviour tabs |
| Recordings | `/recordings` | Clip browser and live audio |
| Reports | `/reports` | Weekly, year-in-review and history look-backs |
| Station | `/station` | Station health and vitals |
| Live Logs | `/admin/system/logs/page` | Real-time log stream via SSE |

Each home composes HTMX fragments served under `/pages/*`. The retired
paths (`/heatmap`, `/analytics`, `/timeseries`, `/correlation`, `/today`,
`/gallery`, `/life-list`, …) are permanent redirects into these homes;
`crates/birdnet-web/src/routes/redirects.rs` holds the whole map.

### Dashboard Audio Player

Each detection row that has a recording file shows an inline `<audio>` element:

```html
<audio controls preload="none" style="height:24px;max-width:160px;">
  <source src="/api/v2/recordings/BirdNET_20260314_063045.wav" type="audio/wav">
</audio>
```

### HTMX Patterns Used

- `hx-get` + `hx-trigger="every 30s"` — polling refresh for detection table
- `hx-delete` + `hx-confirm` — detection deletion with confirmation dialog
- `hx-swap="outerHTML"` — row removal on delete
- `hx-include` — species filter form submission without page reload

## Admin Panel

### Settings Editor (`/admin/settings`)

Full settings form covering all configuration categories:

| Category | Fields |
|----------|--------|
| Station | Latitude, longitude, location name |
| Audio | Microphone device, recording length, overlap, sensitivity |
| Detection | Minimum confidence, occurrence threshold |
| BirdWeather | API key, enabled toggle |
| Email Alerts | SMTP host/port/user/pass, from/to, STARTTLS, min confidence, cooldown |
| Apprise | Enabled, URL |
| System | Backup count, disk limit |

Settings are stored in the SQLite `settings` table and read at startup.
Changed settings take effect on the next detection cycle (no restart needed
for integration settings; audio settings require restart).

### System Info (`/admin/system`)

Displays via `sysinfo` 0.39:
- CPU model + load (1/5/15 min averages)
- Memory: used / total
- Disk: used / total / percentage
- Temperature: CPU thermal zone
- Uptime
- Active recording status

Buttons: "Create Backup Now" → `POST /admin/system/backup`, "Manage Backups →"

### Backup Management (`/admin/system/backups`)

| Action | Endpoint |
|--------|----------|
| List backups | `GET /admin/system/backups` |
| Download `.db` | `GET /admin/system/backups/{name}` |
| Delete backup | `DELETE /admin/system/backups/{name}` (HTMX row removal) |

Security: canonical path check prevents directory traversal; only `.db` files
with safe names are served. Files streamed via `tokio-util::ReaderStream`.

### Live Admin Logs (`/admin/system/logs/page`)

SSE-based live log viewer wired to a tokio broadcast channel. All
`tracing` events are forwarded to the channel for real-time display. New
subscribers are warmed up with the last 200 retained lines
(`RECENT_LOG_CAPACITY`) before the live tail begins.

## Live streams

| Stream | Endpoint | Transport | Payload |
|--------|----------|-----------|---------|
| Live detections | `/api/v2/ws/detections` | WebSocket | JSON `Detection` objects |
| Live spectrogram | `/api/v2/ws/spectrogram` | WebSocket | Spectrogram frames |
| Live log lines | `/admin/system/logs` | SSE | JSON `LogLine` records |

All three use `tokio::sync::broadcast` channels: detections 256 slots, the
spectrogram 16, the log stream 512. Subscribers receive events from when
they connect; missed events are not replayed (ring buffer only). The Today
page's WebSocket client (`static/live-detections.js`) reconnects on
disconnect; the log viewer relies on `EventSource`'s built-in retry.

## Design Decisions

**Why axum over actix-web:** axum uses Tower middleware which composes better,
has a smaller API surface, and integrates naturally with tokio. It's lighter
weight for an embedded application.

**Why one writer plus an in-house `ReaderPool`, not a pool crate:** `AppState`
originally held a single `Mutex<Connection>` and every path took it — each
page render, the health-badge poll on every open tab, `/api/v2/metrics`, the live
feed and the detection writer. WAL was enabled the whole time, and a single
connection uses none of what WAL is for (`db_pool.rs` records a 1.3 s
lock hold on the history calendar at 3.3 M rows). `birdnet-web::db_pool`
now opens a few read-only connections for queries and keeps the single
writer for inserts; no `r2d2`/`deadpool`.

**Why a heavy-read gate on top of the pool:** the routes that scan the whole
detection history (the RSS feeds, the exports, the insights routes, the
migration cards, the analytics fragments) would otherwise take every reader
at once, and a cheap page — `/api/v2/health`, the species list — then waits
behind them (measured: 22 s with the pool alone). `AppState` holds a
semaphore of half the pool, at least one slot; a heavy route takes a slot
(`run_blocking_heavy`, `heavy_read_permit`) before it takes a reader. The feeds
use `try_heavy_read_permit` and answer `503` with `Retry-After: 15` rather than
queueing. The gate bounds readers, not time: a single scan is as slow as its
query. `heavy_reads_leave_readers_for_the_cheap_requests` is the gate on the
width.

**Why the CSP nonce goes onto sealed tags, not onto every `<script`:** the nonce
used to be stamped onto every `<script ` and `<script>` in the finished body, which
cannot tell the server's script from a visitor's text that reached the page
unescaped. `seal.rs` marks the tags the server wrote (a per-process random token,
put there where the authored text is defined, never on a string that holds visitor
text) and `security_headers_middleware` redeems exactly those for the request's nonce.
Every other `<script` becomes `&lt;script`, because a script htmx re-creates from a
swapped fragment is admitted by `'strict-dynamic'` without a nonce, so a bare tag is
not a harmless one. `authored_markup_is_sealed.rs` reads the source and
`every_script_on_every_page_carries_the_nonce.rs` renders the pages: a new script
that is not sealed fails one of them, and in the browser would not run.

**Why `spawn_blocking` for DB:** SQLite operations block the calling thread.
Running them on tokio's blocking thread pool keeps the async runtime responsive
for WebSocket and HTTP handling.

**Why HTMX over React/Vue:** HTMX allows full server-rendered HTML without
a JavaScript build pipeline. The binary embeds everything. Pages work
without client-side JS except for the EventSource SSE connection.

**Why SSE for the log tail and WebSocket for detections and the spectrogram:**
the admin log viewer is a one-directional text stream, and SSE with
`EventSource`'s built-in retry is the simplest fit. The detection feed and
the live spectrogram use WebSocket (`/api/v2/ws/detections`,
`/api/v2/ws/spectrogram`); both answer WebSocket ping with pong so dead
peers are detected and dropped.

**Why the base path is applied by rewriting responses, not by a `url_for()`:**
`BIRDNET_BASE_PATH` mounts the station under a prefix. `Router::nest` fixes
*incoming* requests; every URL the station **emits** is an absolute path from
`/` — 234 literal `href`/`src`/`hx-*` attributes across 47 Rust files, 88 more
in the templates, plus every `Location` header and the session cookie's `Path`.

A `url_for()` helper at each of those 322 sites would depend on every future
line of HTML remembering to call it, and the failure mode is not a clean one:
the page renders and then *some* links 404 while others work, which reads like
a caching bug. So the prefix is applied centrally, in the pass
`security_headers_middleware` already makes over every `text/html` body to
stamp CSP nonces. That pass buffers and walks the body regardless, so the
rewrite is free, and it covers markup written after the change as well as
before it.

The scan matches an attribute *name* from a fixed list followed by `="/`, not
a bare `/` — matching values alone would rewrite paths in prose and in embedded
JSON. It does not track whether it is inside a tag, and `base_path`'s tests say
why: doing so means toggling on `<` and `>`, which appear inside the inline
scripts this application ships, and toggling wrongly there produces a *missed*
link, which breaks navigation rather than displaying one word oddly.

Three things sit outside that pass and are handled explicitly: `Location`
headers (not HTML), the session cookie's `Path` (which keeps a trailing slash,
because RFC 6265's path-match is a prefix rule and `Path=/birdnet` also matches
`/birdnetsomethingelse`), and the WebSocket URLs the live pages assemble in the
browser — those read `<body data-base-path>`, which the same middleware stamps.

Two routes sit beside the nest, both found by probing axum 0.8.9 rather than
assumed: `/` (the host root, otherwise a bare 404 that reads as "the station is
down") and `{base}/` — `nest("/b", …)` matches `/b` and `/b/x` but **not**
`/b/`, which is the URL a browser shows after any link to the station root.

---

[← Behavioral Analytics](08-behavioral-analytics.md) | [Back to Index](../RUST_ARCHITECTURE_PLAN.md) | [Next: Deployment →](10-deployment.md)
