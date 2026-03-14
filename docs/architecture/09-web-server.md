# Web Server

> axum-based REST API, Server-Sent Events, HTMX pages, and admin panel.

## Table of Contents

- [Overview](#overview)
- [Server Architecture](#server-architecture)
- [Application State](#application-state)
- [REST API Endpoints](#rest-api-endpoints)
- [HTMX Pages](#htmx-pages)
- [Admin Panel](#admin-panel)
- [Server-Sent Events (SSE)](#server-sent-events-sse)
- [Design Decisions](#design-decisions)

---

**Status: ✅ Fully implemented** in `crates/birdnet-web/`

## Overview

The web server replaces the Python FastAPI application with an axum server
embedded in the single binary. It serves:

- **REST API** (`/api/v2/*`) for programmatic access and HTMX partial updates
- **Server-Sent Events** (`/api/v2/detections/stream`, `/api/v2/logs/stream`) for live updates
- **HTMX pages** — full server-side rendered dashboard, species, heatmap, analytics, logs
- **Admin panel** — settings editor, system info, backup management, live logs
- **Static files** — species images cached from Flickr/Wikipedia

## Server Architecture

```
axum Router
├── Middleware Stack
│   ├── TraceLayer (structured request/response logging)
│   └── CorsLayer (permissive for development)
│
├── /api/v2/                           REST API
│   ├── GET /                          API info
│   ├── GET /health                    Health check
│   ├── GET /stats                     Detection/species counts
│   ├── GET /detections                List detections (date?, limit?)
│   ├── GET /detections/recent         Recent N detections
│   ├── DELETE /detections/{id}        Delete a detection (HTMX)
│   ├── GET /detections/stream         SSE live detection feed
│   ├── GET /species/top               Top species by count
│   ├── GET /species/activity          Hourly activity for date
│   ├── GET /species/{name}/image      Species image (cached)
│   ├── GET /recordings/{filename}     Serve WAV recording file
│   ├── GET /logs/stream               SSE live log feed
│   └── GET /analytics/*               DuckDB analytics (see below)
│
├── /pages/                            HTMX server-rendered pages
│   ├── GET /pages/dashboard           Detection table + live SSE
│   ├── GET /pages/species             Species summary page
│   ├── GET /pages/heatmap             Hour×weekday activity SVG
│   ├── GET /pages/analytics           Trends, co-occurrence, seasonal
│   └── GET /pages/logs                Live log stream viewer
│
└── /admin/                            Admin panel
    ├── GET  /admin/settings           Settings form
    ├── POST /admin/settings           Save settings
    ├── GET  /admin/system             System info + backup controls
    ├── POST /admin/system/backup      Trigger backup now
    ├── GET  /admin/system/backups     List backup files
    ├── GET  /admin/system/backups/{n} Download backup file
    ├── DELETE /admin/system/backups/{n} Delete backup (HTMX)
    └── GET  /admin/logs               Admin log viewer
```

## Application State

Shared state using `Arc<AppState>`:

```rust
pub struct AppState {
    pub db: Arc<Mutex<rusqlite::Connection>>,
    pub db_path: PathBuf,
    pub backup_dir: PathBuf,
    pub detection_tx: tokio::sync::broadcast::Sender<Detection>,
    pub log_tx: tokio::sync::broadcast::Sender<String>,
}
```

- Migrations run automatically on startup
- All SQLite access goes through `with_db()` closure
- Handlers use `tokio::task::spawn_blocking` for DB queries
- Broadcast channels for SSE: ring buffer (capacity 256) prevents dropped events

## REST API Endpoints

### System

| Endpoint | Response |
|----------|----------|
| `GET /api/v2/` | `{ name, version, status }` |
| `GET /api/v2/health` | `{ status: "healthy"/"degraded", database: bool }` |
| `GET /api/v2/stats` | `{ total_detections, unique_species, today_count }` |

### Detections

| Endpoint | Query Params | Response |
|----------|-------------|----------|
| `GET /detections` | `date?`, `limit?` (default 100) | `{ detections: [...], total }` |
| `GET /detections/recent` | `limit?` (default 20) | `{ detections: [...], total }` |
| `DELETE /detections/{id}` | — | 200 + HTMX `HX-Trigger: detection-deleted` |
| `GET /detections/stream` | — | `text/event-stream` SSE |

### Species

| Endpoint | Query Params | Response |
|----------|-------------|----------|
| `GET /species/top` | `limit?` (default 20) | `{ species: [{ name, count, avg_confidence }] }` |
| `GET /species/activity` | `date` (required) | `{ activity: [{ hour, count }], date }` |
| `GET /species/{name}/image` | — | Image redirect or 404 |

### Recordings & Logs

| Endpoint | Response |
|----------|----------|
| `GET /recordings/{filename}` | `audio/wav` file stream (chunked) |
| `GET /logs/stream` | `text/event-stream` SSE of log lines |

### Analytics

| Endpoint | Query Params | Response |
|----------|-------------|----------|
| `GET /analytics/trends` | `days?` (default 30) | `{ dates, counts, moving_avg }` |
| `GET /analytics/heatmap` | `species?` | `{ data: [[hour, weekday, count]] }` |
| `GET /analytics/top-species` | `period?` (week/month/year) | `{ species: [...] }` |
| `GET /analytics/correlation` | `min_days?` | `{ pairs: [{ a, b, days }] }` |
| `GET /analytics/seasonal` | `species?` | `{ months, species, matrix }` |

## HTMX Pages

All pages use server-side HTML rendering with inline styles (Tailwind-like utility
classes) and HTMX for dynamic updates. No client-side JavaScript framework.

| Page | Route | Key Features |
|------|-------|-------------|
| Dashboard | `/pages/dashboard` | Detection table with audio player, SSE live updates, delete buttons |
| Species | `/pages/species` | All detected species with image thumbnails, counts, confidence |
| Heatmap | `/pages/heatmap` | SVG hour×weekday activity chart, species filter |
| Analytics | `/pages/analytics` | Trends, co-occurrence table, seasonal grid |
| Live Logs | `/pages/logs` | Real-time log stream via SSE |

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

Displays via `sysinfo` 0.32:
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

### Live Admin Logs (`/admin/logs`)

SSE-based live log viewer wired to a tokio broadcast channel. All
`tracing` events are forwarded to the channel for real-time display.

## Server-Sent Events (SSE)

Two SSE streams:

| Stream | Endpoint | Payload |
|--------|----------|---------|
| Live detections | `/api/v2/detections/stream` | JSON `Detection` objects |
| Live log lines | `/api/v2/logs/stream` | Plain text log lines |

Both streams use `tokio::sync::broadcast` channels with `capacity = 256`.
Subscribers receive events from when they connect; missed events are not
replayed (ring buffer only). The dashboard page reconnects automatically
on disconnect via `EventSource` built-in retry.

## Design Decisions

**Why axum over actix-web:** axum uses Tower middleware which composes better,
has a smaller API surface, and integrates naturally with tokio. It's lighter
weight for an embedded application.

**Why `Arc<Mutex>` over connection pool:** This is a single-binary embedded
application, not a multi-tenant web service. One connection with WAL mode
provides sufficient concurrency. A pool adds complexity without benefit.

**Why `spawn_blocking` for DB:** SQLite operations block the calling thread.
Running them on tokio's blocking thread pool keeps the async runtime responsive
for WebSocket and HTTP handling.

**Why HTMX over React/Vue:** HTMX allows full server-rendered HTML without
a JavaScript build pipeline. The binary embeds everything. Pages work
without client-side JS except for the EventSource SSE connection.

**Why SSE over WebSocket:** SSE is one-directional (server → client), simpler
to implement, and sufficient for live detection/log updates. WebSocket adds
complexity without benefit for this use case.

---

*Last updated: 2026-03-14*

[← Behavioral Analytics](08-behavioral-analytics.md) | [Back to Index](../RUST_ARCHITECTURE_PLAN.md) | [Next: Deployment →](10-deployment.md)
