# HTTP & WebSocket API

Everything the UI does is backed by a versioned JSON API under **`/api/v2`**. It's handy for dashboards, scripts, and home-automation pulls. All endpoints are read-only `GET`s unless noted.

> Base URL in the examples is `http://localhost:8502`. Adjust for your host, and remember any [reverse-proxy auth](../admin/remote-access.md) you've added.

> **Auth:** the built-in HTTP Basic Auth gates only the `/admin*` UI routes. Every `/api/v2/*` endpoint, the WebSocket stream, and the health check are open to anyone who can reach the port — restrict them at the network layer (VPN / proxy allow-list) if that matters.

> **OpenAPI:** a complete, machine-readable **OpenAPI 3.1** description of this API is served at [`GET /api/v2/openapi.json`](http://localhost:8502/api/v2/openapi.json) (and committed at [`crates/birdnet-web/openapi.json`](https://github.com/tomtom215/BirdNet-Behavior/blob/main/crates/birdnet-web/openapi.json)). Load it into Swagger UI, Redoc, Postman, or `openapi-generator` to explore the endpoints and generate clients.

## Health & metrics

```bash
curl http://localhost:8502/api/v2/health
```

```json
{
  "status": "healthy",
  "version": "0.15.0",
  "database": "ok",
  "analytics": true,
  "detection_daemon": "running",
  "detection_silence_secs": 142
}
```

`status` is `"healthy"` (HTTP `200`) or `"degraded"` (HTTP `503` — the database
is unreachable, or the last recorded integrity check failed), so monitoring can
alert on the status code alone.

`database` is `"ok"`, `"unchecked"` or `"error"`. It reports the verdict of the
**daily maintenance integrity check**, not a check run at request time: that
check reads every page of the database file, and this endpoint is polled by the
container health check every 30 seconds. On a multi-year station a per-request
check could not finish inside the health check's own timeout. `"unchecked"`
means no verdict is on record yet — normal for the first few minutes after a
fresh install, and reported `healthy`/`200`, because "not yet verified" is not
"broken". A failure stays reported until it is fixed, rather than depending on
which request happened to catch it.
`detection_daemon` is `"running"` or `"stopped"` — `"stopped"` means web-only
mode or an unconfigured model/labels/watch-dir, i.e. the UI is up but nothing is
being analysed. `analytics` reports whether the DuckDB engine is active.
`detection_silence_secs` is the end-to-end freshness signal: seconds since the
most recent stored detection (the deadman watchdog's measurement), or `null`
before the first measurement / on a station that has never detected anything.
A value that climbs past your expected quiet period means the chain from
microphone to database has stopped producing rows even if every component
looks healthy.

| Endpoint | Purpose |
|---|---|
| `GET /api/v2/health` | JSON liveness/health check (use for monitoring). |
| `GET /api/v2/metrics` | Prometheus metrics (text format) — see [Integrations](./integrations.md#prometheus-metrics). |
| `GET /api/v2/stats` | Summary counts (detections, species, today). |
| `GET /api/v2/system/disk` | Disk usage for the data directory. |

## Detections

```bash
curl 'http://localhost:8502/api/v2/detections/recent?limit=2'
```

```json
{
  "detections": [
    {
      "date": "2026-05-22", "time": "21:10:31",
      "com_name": "Song Sparrow", "sci_name": "Melospiza melodia",
      "confidence": 0.83, "cutoff": 0.7,
      "lat": 42.3601, "lon": -71.0589,
      "week": 21, "sens": 1.25, "overlap": 0.0,
      "file_name": "Song_Sparrow-83-2026-05-22-...wav"
    }
  ]
}
```

| Endpoint | Common query params |
|---|---|
| `GET /api/v2/detections` | `date`, `species`, `limit`, `offset` — paginated list |
| `GET /api/v2/detections/recent` | `limit` — most recent first |
| `GET /api/v2/detections/daily` | `days` — per-day counts over the last N days |

## Species

| Endpoint | Common query params | Purpose |
|---|---|---|
| `GET /api/v2/species/top` | `limit` | Most-detected species with counts |
| `GET /api/v2/species/search` | `q`, `limit` | Search by name |
| `GET /api/v2/species/detail` | `name` | Per-species stats |
| `GET /api/v2/species/activity` | `date` | Hourly activity (all species) on a given day |
| `GET /api/v2/species/image/{scientific_name}` | — | Cached Wikipedia image (redirect/bytes) |
| `GET /api/v2/recordings/{filename}` | — | Stream a recorded clip (WAV) |

## Time-series & analytics

These power the charts on the [Analytics](../guide/analytics.md) and time-series pages. The richer ones use the DuckDB analytics engine, which is **built in and on by default** ([Configuration](../getting-started/configuration.md)); they return a "not available" status only if you've disabled it.

| Endpoint | Purpose |
|---|---|
| `GET /api/v2/timeseries/daily` · `weekly` · `hourly` | Counts over time |
| `GET /api/v2/timeseries/diversity` | Species diversity over time |
| `GET /api/v2/timeseries/trend` · `year-over-year` | Trends and year-on-year comparison |
| `GET /api/v2/timeseries/peak-windows` · `gaps` · `anomalies` | Peak activity, quiet gaps, outliers |
| `GET /api/v2/timeseries/accumulation` | Life-list accumulation curve |
| `GET /api/v2/timeseries/heatmap` · `sessions` | Hour×day heatmap, activity sessions |
| `GET /api/v2/timeseries/status` | Whether the time-series engine is available |
| `GET /api/v2/analytics/sessions` · `retention` · `funnel` · `next-species` · `patterns` | DuckDB behavioral analytics |
| `GET /api/v2/analytics/status` | Analytics build flags **and** the state of the store |

### Diagnosing empty analytics dashboards

`GET /api/v2/analytics/status` is the first thing to check when the analytics
screens are blank. Its `analytics_compiled` and `analytics_configured` fields
describe *intent* — that the binary has DuckDB in it and a database was wired
up — and both stay `true` in every case where the dashboards are actually
broken. The `store` object is the part that differs:

```json
{
  "analytics_compiled": true,
  "analytics_configured": true,
  "store": {
    "extension_loaded": true,
    "detections": 412903,
    "unplaceable_detections": 3,
    "detections_placeable": 412900,
    "engine_duckdb_version": "v1.5.5",
    "engine_platform": "linux_arm64",
    "embedded_extension": {
      "version": "v0.9.1",
      "duckdb_version": "v1.5.5",
      "platform": "linux_arm64",
      "mismatch": null
    }
  }
}
```

- `extension_loaded: false` — the behavioural functions (`sessionize`,
  `retention`, `window_funnel`, `sequence_*`) will fail while the time-series
  screens keep working. Check `embedded_extension.mismatch`: a non-null value
  means the copy built into this binary can never load, which leaves an offline
  station with no behavioural analytics. Its `property` says whether the
  `DuckDB version`, the `platform`, or both disagree — compare against
  `engine_duckdb_version` and `engine_platform`. An extension is locked to
  both, and both fail the same way at `LOAD`. Rebuild against
  `community-extensions.duckdb.org/<engine_duckdb_version>/<engine_platform>/`.
- `detections: 0` against a station with history — the SQLite → DuckDB sync has
  not run or did not complete.
- `unplaceable_detections` above zero — that many rows carry a `Date`/`Time`
  naming no point in time (usually from a BirdNET-Pi import). They count toward
  the station's detection total but cannot appear in any date- or time-based
  analytic, which is why a dashboard total can sit below the raw count.
- `store: null` — this binary was built without analytics.

## WebSocket — live detections

```text
ws://localhost:8502/api/v2/ws/detections
```

Connect and you receive a JSON message for each new detection as it happens — the same stream that drives the dashboard live feed, the spectrogram and kiosk mode. Behind a reverse proxy, make sure the `Upgrade`/`Connection` headers are forwarded ([Remote Access](../admin/remote-access.md#reverse-proxy-with-https)).

```js
const ws = new WebSocket("ws://localhost:8502/api/v2/ws/detections");
ws.onmessage = (e) => console.log(JSON.parse(e.data));
```

## Errors

An unmatched path under `/api/` returns a machine-readable JSON `404` (not the
HTML "page not found" the browser UI serves), so a script that mistypes a route
or hits a removed endpoint sees the failure instead of silently parsing a web
page:

```json
{ "error": "not found", "path": "/api/v2/nope" }
```

## Export

CSV/JSON/eBird export of the full detection history is available from the [Backups](../admin/backups.md#export) page (and a BirdNET-Pi-compatible CSV for tooling that expects that format).
