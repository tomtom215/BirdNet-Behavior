# Web UI & URLs

A map of every page and admin URL the dashboard serves. For the JSON/WebSocket API, see [HTTP & WebSocket API](./api.md).

## The six homes

The UI is organized into six top-level homes — the tabs across the top of every
page (and the phone bottom bar).

| URL | Home | What it gathers |
|---|---|---|
| `/` | **Today** | the merged "right now" view + the full searchable day (live signal, day strip, unified log, top species, best recordings, station health) |
| `/species` | **Species** | species list · photos · life list, plus per-species detail |
| `/patterns` | **Patterns** | when-active heatmap · dawn chorus · migration · co-occurrence · trends · behavioral analytics, one tab each |
| `/recordings` | **Recordings** | the Clips browser (`?view=clips`) and the live audio + spectrogram (`?view=live`) |
| `/reports` | **Reports** | weekly recap · year in review · history |
| `/station` | **Station** | health (public) + the gated admin task groups |

Tabs within a home are selected by a query parameter, e.g.
`/patterns?tab=dawn`, `/reports?tab=history`, `/species?view=lifelist`,
`/recordings?view=live`.

### Legacy routes (permanent redirects)

Older addresses and BirdNET-Pi muscle-memory still work — each one
`308`-redirects to its place in the new structure, so no bookmark ever 404s.

| Old URL | → New |
|---|---|
| `/today` | `/` |
| `/heatmap` | `/patterns` |
| `/analytics/dawn-chorus` | `/patterns?tab=dawn` |
| `/migration` | `/patterns?tab=migration` |
| `/correlation` | `/patterns?tab=together` |
| `/timeseries` | `/patterns?tab=trends` |
| `/analytics` | `/patterns?tab=behavior` |
| `/weekly` | `/reports` |
| `/year-in-review` | `/reports?tab=year` |
| `/history` | `/reports?tab=history` |
| `/system` | `/station` |
| `/listen` | `/recordings?view=live` |
| `/livestream` | `/recordings?view=live` |
| `/live` | `/recordings?view=live` |

### Other pages

| URL | Description |
|---|---|
| `/species/detail?name=…` | Species detail — hourly chart, trend, companions, photo |
| `/gallery` | Species photo gallery (also reachable as the Species → Photos view) |
| `/life-list` | Life list — every species ever detected, with a growth curve |
| `/quarantine` | Rare-bird review — approve, reject (also surfaced as the Today review nudge) |
| `/notifications` | Notification center — history and channel stats |
| `/kiosk` | Kiosk mode — auto-refreshing display for dedicated screens |
| `/onboarding` | First-run setup wizard |
| `/detections/detail?date=&time=&name=` | Single-detection detail — spectrogram, audio, correlation id, share |

## Admin

The `/admin*` routes are the **only** password-gated part of the UI — they
require sign-in (a session cookie) when an admin password is set; a fresh
bare-metal install sets one automatically. The Station home's **Health** tab is
public (it's the read-only `/system` view); the other Station task groups link
into these gated pages. Every home above and the JSON/WebSocket API below are
open. See [Remote Access & Security](../admin/remote-access.md).

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

## Public links & feeds

These are unauthenticated, read-only endpoints. See [Detection Detail & Sharing](../guide/sharing.md) and [RSS & Calendar Feeds](../guide/feeds.md).

| URL | Description |
|---|---|
| `/r/<token>` | Public share page for one detection (HMAC-signed, 30-day expiry) |
| `/r/<token>/audio.wav` | Redirect to the shared clip's audio |
| `/r/<token>/spectrogram.png` | Redirect to the shared clip's spectrogram |
| `/feeds/rare.rss` | RSS 2.0 — first-of-station detections (confidence ≥ 0.85) |
| `/feeds/rare.ics` | iCalendar — the same rare detections as calendar events |
| `/feeds/today.rss` | RSS 2.0 — every detection today |

## API

The JSON API is versioned under `/api/v2`, with a WebSocket live stream at `/api/v2/ws/detections`. It has its own page: **[HTTP & WebSocket API](./api.md)** — endpoints, query parameters, response shapes and examples.
