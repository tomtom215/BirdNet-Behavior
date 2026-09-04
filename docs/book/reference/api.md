# HTTP & WebSocket API

Everything the UI does is backed by a versioned JSON API under **`/api/v2`**. It's handy for dashboards, scripts, and home-automation pulls. Almost every endpoint is a read-only `GET`; the exceptions are the eight [write endpoints](#changing-a-station), which need a token.

> Base URL in the examples is `http://localhost:8502`. Adjust for your host, and remember any [reverse-proxy auth](../admin/remote-access.md) you've added.

> **Auth:** the built-in session sign-in gates only the `/admin*` UI routes. Every *read* endpoint under `/api/v2/*`, the WebSocket stream, and the health check are open to anyone who can reach the port — restrict them at the network layer (VPN / proxy allow-list) if that matters.
>
> The **write** endpoints, and the settings read, are the exception and do not follow that rule: each needs `Authorization: Bearer <token>`, and a station with no `BNB_API_TOKEN` answers `404` to all of them. See [Changing a station](#changing-a-station).

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
  "detection_writes": "accepted",
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
| `GET /api/v2/system/disk` | Disk usage for the data directory. Answers 503 when the disk is critical (95 % used) and 200 otherwise. Fullness is measured against the space this user can actually reach — `used / (used + available)`, the same figure `df`'s `Use%` column reports — not against the raw device size, which on any ext4 with its default 5 % root reserve, or inside a container quota, is larger than anything the station can use. |

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

## Changing a station

Eight method-and-path pairs across seven routes, and they are the only ones in
`/api/v2` that change anything.
They exist so Home Assistant, Node-RED or a shell script can *act* on a station
rather than only read it — before them, every state change in the product was an
HTMX form post returning HTML, which is not a contract anyone can build on.

An eighth, `GET /api/v2/settings`, is a *read* that lives behind the same token:
a station's configuration is not public even with its credentials taken out.

### Turning them on

They are **off** by default. Set `BNB_API_TOKEN` in the config file or the
environment and restart:

```bash
openssl rand -base64 48
```

Until you do, all eight answer `404`: the write surface does not exist rather
than existing unprotected. Note this is the opposite default from `CADDY_PWD`,
where an unset password leaves `/admin` *open* — an unset token leaves the write
API *closed*. A token shorter than 32 bytes is refused and leaves the API off,
with a warning in the log and a warning from `birdnet-behavior --doctor`.

### Identifying a detection

There is no surrogate id. A detection is identified the way the database
identifies it — by date, time and scientific name:

```json
{ "date": "2026-09-03", "time": "06:12:44", "sci_name": "Erithacus rubecula" }
```

A malformed key is `400`; a well-formed key matching no row is `404`.

### The endpoints

| Method | Path | Does |
|---|---|---|
| `POST` | `/api/v2/detections/review` | Record `confirmed` / `rejected`, or clear a verdict by omitting `status` |
| `POST` | `/api/v2/detections/lock` | Protect the clip from the disk-full purge and retention sweep |
| `POST` | `/api/v2/detections/unlock` | Return it to the ordinary purge rules |
| `POST` | `/api/v2/detections/delete` | Remove the detection row |
| `POST` | `/api/v2/detections/batch` | Apply one of the four above to up to 500 detections |
| `GET` | `/api/v2/settings` | Read every setting, with credentials redacted |
| `PUT` | `/api/v2/settings` | Change one or more settings |
| `POST` | `/api/v2/control/restart` | Restart the station |

```bash
curl -X POST http://localhost:8502/api/v2/detections/review \
  -H "Authorization: Bearer $BNB_API_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"date":"2026-09-03","time":"06:12:44","sci_name":"Erithacus rubecula","status":"confirmed"}'
```

```bash
curl -X POST http://localhost:8502/api/v2/detections/lock \
  -H "Authorization: Bearer $BNB_API_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"date":"2026-09-03","time":"06:12:44","sci_name":"Erithacus rubecula"}'
```

### Batches

One round trip instead of forty, for the case that actually comes up: triaging
a night's false positives.

```bash
curl -X POST http://localhost:8502/api/v2/detections/batch \
  -H "Authorization: Bearer $BNB_API_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"op":"review","status":"rejected","detections":[
        {"date":"2026-09-03","time":"02:14:07","sci_name":"Strix aluco"},
        {"date":"2026-09-03","time":"02:19:51","sci_name":"Strix aluco"}
      ]}'
```

```json
{
  "op": "review",
  "requested": 2,
  "applied": 2,
  "failed": 0,
  "results": [
    { "detection": "2026-09-03 02:14:07 Strix aluco", "applied": true },
    { "detection": "2026-09-03 02:19:51 Strix aluco", "applied": true }
  ]
}
```

`op` is one of `review`, `lock`, `unlock`, `delete`, and applies to every
detection in the list. `status` and `notes` belong to `review` and are refused
with any other `op`: a caller who sent `{"op":"delete","status":"confirmed"}`
meant something by both fields, and silently dropping one would not say which.
At most 500
detections per request; a longer list is refused before anything is written
rather than quietly truncated.

> **Read `failed`, not just the status code.** The response is `200` whenever
> the *request* was well-formed, even if every item failed. A key that matches
> nothing does not stop the batch — a client working from a list a few seconds
> stale would otherwise have forty good deletions refused because three rows had
> already gone — so each detection gets its own entry in `results`, and
> `applied` and `failed` are at the top level so you need not walk the array.

This is **not** a transaction. Each detection takes the same path as the single-detection endpoint, which is what keeps the SQLite
store and the analytics copy in step; a shared transaction would mean a second
implementation of "delete a detection" and is not worth that. If the process
dies mid-batch, the detections already applied stay applied.

Every detection actually changed is written to the audit log individually, under
the same action name a single-detection call uses — so "what happened to that
recording?" is still answerable afterwards. A full batch therefore writes 500
rows, which is exactly what the admin audit view shows in one page before it
says "Showing the most recent 500 matches". That, and the cost of the writes
themselves, is what the limit bounds.

### Settings

`GET /api/v2/settings` returns every persisted setting, plus `redacted` (the
keys whose value was withheld) and `writable_keys` (what `PUT` will accept):

```bash
curl http://localhost:8502/api/v2/settings \
  -H "Authorization: Bearer $BNB_API_TOKEN"
```

```json
{
  "settings": { "confidence_threshold": "0.7", "email_smtp_pass": "***REDACTED***" },
  "redacted": ["email_smtp_pass"],
  "writable_keys": ["alsa_device", "rtsp_url", "…"]
}
```

Credentials are removed two ways: by key name (`email_smtp_pass`,
`birdweather_token`) and by value shape, which catches the credential inside a
URL — `apprise_url` does not *look* like a secret and routinely carries one.
A withheld value is **replaced** rather than omitted, so "you may not read this"
stays distinguishable from "this was never configured".

The by-shape rules are the support bundle's, applied in the same order, and they
are blunt: `ntfy://alice:hunter2@ntfy.example/topic` comes back as
`***@ntfy.example/topic` — the host and path survive, the scheme and username do
not. Treat a by-shape redaction as "the host, roughly", not as a value you can
edit and send back.

> One gap, stated rather than left to be discovered: a URL whose *path segment*
> is the credential — a heartbeat ping URL, for instance — matches neither rule
> and is returned in full.

`PUT` is a partial update; send only the keys you mean to change. Values may be
strings, numbers or booleans, and each goes through the same normalisation the
settings page uses, so `"51,5"` is stored as `51.5`:

```bash
curl -X PUT http://localhost:8502/api/v2/settings \
  -H "Authorization: Bearer $BNB_API_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"confidence_threshold": 0.75, "station_name": "Back Garden"}'
```

```json
{ "updated": 2, "keys": ["confidence_threshold", "station_name"] }
```

A key already holding the value you sent is not rewritten and is not counted in
`updated`. Two things are refused with `400` rather than accepted quietly:

- **An unknown key.** A misspelled `confidence_treshold` answering `200` would
  tell you a change had landed when none had.
- **The literal `***REDACTED***`.** It is what `GET` hands back for a secret, so
  a client that reads the whole object, edits one field and writes it back would
  otherwise overwrite the station's real SMTP password with the placeholder.

Settings that the running process reads at startup need a restart to take
effect, the same as when they are changed from `/admin/settings`.

### Restarting

```bash
curl -X POST http://localhost:8502/api/v2/control/restart \
  -H "Authorization: Bearer $BNB_API_TOKEN"
```

The process sends itself `SIGTERM` after a short delay — long enough for the
response to reach you — and systemd's `Restart=always` starts a fresh instance.
Outside systemd — a bare `cargo run`, a container without an init — nothing
would bring the station back, so the endpoint answers `503` and signals nothing
rather than reporting a restart that would in fact be a shutdown.

### The audit log

Every change is written to the [audit log](../admin/system.md) as
`detection.review` / `detection.lock` / `detection.unlock` / `detection.delete`
/ `settings.update` / `system.restart`, with no user and `via=api` in the
metadata — a batch writes one such row per detection it changed, under the same
names — a token is not a person, and the log says so rather than inventing
one. A settings change records the key *names* only: an entry reading
`birdweather_token=…` would put a credential on the page that renders the log.
The restart entry is written before the decision, so a refused restart is
recorded too.

### Cross-origin calls

These endpoints are exempt from the dashboard's same-origin (CSRF) check,
because a cross-site *form* cannot set an `Authorization` header — that is the
whole premise of the check, so it has nothing to protect here. The exemption is
scoped to these paths; every other write in the product still has it.
