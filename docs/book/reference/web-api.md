# Web UI & URLs

A map of every page and admin URL the dashboard serves. For the JSON/WebSocket API, see [HTTP & WebSocket API](./api.md).

## Pages

| URL | Description |
|---|---|
| `/` | Dashboard — live detections, top species, activity heatmap |
| `/today` | Today's detections — searchable, paginated; delete / lock / re-label |
| `/history` | Detection history — date browser with hourly bar charts |
| `/weekly` | Weekly report — top species, new discoveries, 7-day chart |
| `/year-in-review` | Editorial annual recap — 52-week tape, leaderboard, milestones |
| `/species` | Species list — search, counts, sparklines |
| `/species/detail?name=…` | Species detail — hourly chart, trend, companions, photo |
| `/gallery` | Species photo gallery — card grid with search and sort |
| `/life-list` | Life list — every species ever detected, with a growth curve |
| `/recordings` | Recording browser with inline audio player |
| `/heatmap` | Activity heatmap, circadian polar, migration ridgeline |
| `/correlation` | Co-occurrence matrix + acoustic-network chord diagram |
| `/analytics` | Behavioral analytics (analytics build) |
| `/timeseries` | Time-series analytics (activity, diversity, trends, peaks) |
| `/quarantine` | Rare-bird quarantine — review, approve, reject |
| `/notifications` | Notification center — history and channel stats |
| `/system` | System health — CPU / memory / temp gauges, database, disk |
| `/kiosk` | Kiosk mode — auto-refreshing display for dedicated screens |
| `/onboarding` | First-run setup wizard |
| `/live` | Live audio stream |

## Admin

| URL | Description |
|---|---|
| `/admin/settings` | Audio, location, detection, notifications, email, MQTT, species, system |
| `/admin/audio` | Microphone & RTSP source management |
| `/admin/quality` | Data-quality metrics dashboard |
| `/admin/rules` | Conditional alert-rule engine |
| `/admin/migrate` | BirdNET-Pi database import |
| `/admin/backups` | Backups, restore, storage, danger zone |
| `/admin/system` | CPU / memory / temperature / disk |
| `/admin/system/logs/page` | Live log viewer (SSE, level filtering) |
| `/admin/update/check` | Check for and apply binary updates |

## API

The JSON API is versioned under `/api/v2`, with a WebSocket live stream at `/api/v2/ws/detections`. It has its own page: **[HTTP & WebSocket API](./api.md)** — endpoints, query parameters, response shapes and examples.
