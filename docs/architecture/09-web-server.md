# Web Server

> axum-based REST API, WebSocket, and HTMX serving.

## Overview

The web server replaces the Python FastAPI application with an axum server
embedded in the single binary. It serves:

- REST API (`/api/v2/*`) for programmatic access
- WebSocket (`/ws/detections`) for live detection streaming
- HTMX partials for the web dashboard
- Static files embedded in the binary

**Status: Core server and 4 route groups implemented** in `crates/birdnet-web/`

## Server Architecture

```
axum Router
├── Middleware Stack
│   ├── TraceLayer (request/response logging)
│   └── CorsLayer (permissive for development)
│
├── /api/v2/
│   ├── / (GET)                    → API info
│   ├── /health (GET)              → Health check
│   ├── /stats (GET)               → Detection/species counts
│   ├── /detections (GET)          → List detections (by date, with limit)
│   ├── /detections/recent (GET)   → Recent detections
│   ├── /species/top (GET)         → Top species by count
│   ├── /species/activity (GET)    → Hourly activity for date
│   └── /analytics/* (GET)         → DuckDB behavioral analytics (planned)
│
└── Graceful Shutdown
    └── Listens for SIGTERM / SIGINT
```

## Application State

Shared state pattern using `Arc<Mutex<Connection>>`:

```rust
pub struct AppState {
    inner: Arc<Mutex<AppStateInner>>,
}

struct AppStateInner {
    db: rusqlite::Connection,
    db_path: PathBuf,
}
```

- Migrations run automatically on startup
- All DB access goes through `with_db()` closure
- Handlers use `tokio::task::spawn_blocking` for DB queries

## Implemented Endpoints

### System (`/api/v2/`)

| Endpoint | Response |
|----------|----------|
| `GET /` | `{ name, version, status }` |
| `GET /health` | `{ status: "healthy"/"degraded", database: bool }` |
| `GET /stats` | `{ total_detections, unique_species }` |

### Detections (`/api/v2/detections/`)

| Endpoint | Query Params | Response |
|----------|-------------|----------|
| `GET /detections` | `date?`, `limit?` (default 100) | `{ detections: [...], total }` |
| `GET /detections/recent` | `limit?` (default 20) | `{ detections: [...], total }` |

### Species (`/api/v2/species/`)

| Endpoint | Query Params | Response |
|----------|-------------|----------|
| `GET /species/top` | `limit?` (default 20) | `{ species: [{ name, count, avg_confidence }] }` |
| `GET /species/activity` | `date` (required) | `{ activity: [{ hour, count }], date }` |

### Analytics (`/api/v2/analytics/`) -- Planned

| Endpoint | Status |
|----------|--------|
| `GET /analytics/sessions` | Placeholder |
| `GET /analytics/retention` | Placeholder |
| `GET /analytics/funnel` | Placeholder |
| `GET /analytics/patterns` | Placeholder |
| `GET /analytics/next-species` | Placeholder |

## Remaining Work

- [ ] WebSocket endpoint for live detection streaming (tokio broadcast channel)
- [ ] HTMX template rendering for dashboard pages
- [ ] Static file serving (embedded via `rust-embed`)
- [ ] Authentication (matching current Caddy basic auth setup)
- [ ] DuckDB-powered analytics endpoint implementations
- [ ] Export endpoints (CSV, JSON bulk export)
- [ ] Internal notification endpoint (`/api/v2/internal/notify`)

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

---

[← Behavioral Analytics](08-behavioral-analytics.md) | [Back to Index](../RUST_ARCHITECTURE_PLAN.md) | [Next: Deployment →](10-deployment.md)
