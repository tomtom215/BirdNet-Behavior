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
| `/station` | **Settings** | health (public) + the gated admin task groups |

### Searching the whole log

`/search` is the detection log across all time rather than one day. Every filter
lives in the query string, so a search worth repeating is a bookmark:

```text
/search?q=Turdus&from=2026-03-01&to=2026-03-31&conf_min=80&sort=confidence
```

| Parameter | Takes | Notes |
|---|---|---|
| `q` | free text | matches common **and** scientific name; a leading `NOT ` inverts it |
| `species` | one name | exact, rather than `q`'s substring match |
| `from`, `to` | `YYYY-MM-DD` | inclusive both ends |
| `hour_from`, `hour_to` | `0`–`23` | `hour_from` above `hour_to` asks for a window *through* midnight |
| `conf_min`, `conf_max` | `0`–`100` | a percentage, because that is what the UI shows everywhere else |
| `source` | audio source label | which microphone or stream |
| `verdict` | `confirmed` · `rejected` · `unreviewed` | |
| `locked` | `locked` · `unlocked` | whether the clip is protected from retention |
| `category` | the Today log's four shortcuts | rare, new, verified, quarantined |
| `sort` | `oldest` · `confidence` · `confidence-asc` · `species` · `species-desc` | newest first when absent |
| `offset` | a row number | 50 rows per page |

With an admin session it also carries **bulk review**: tick rows and confirm,
reject or delete up to 100 at once. Those actions are `POST`s on the gated
router, so a logged-out visitor sees the same search results and none of the
buttons.

Tabs within a home are selected by a query parameter, e.g.
`/patterns?tab=dawn`, `/reports?tab=history`, `/species?view=lifelist`,
`/recordings?view=live`.

### Legacy routes (permanent redirects)

Older addresses and BirdNET-Pi muscle-memory still work — each one
`308`-redirects to its place in the new structure, so no bookmark ever 404s.

| Old URL | → New |
|---|---|
| `/today` | `/` |
| `/gallery` | `/species?view=photos` |
| `/life-list` | `/species?view=lifelist` |
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
| `/quarantine` | Rare-bird review — approve, reject (also surfaced as the Today review nudge) |
| `/notifications` | Notification center — history and channel stats |
| `/kiosk` | Kiosk mode — auto-refreshing display for dedicated screens |
| `/onboarding` | First-run setup wizard |
| `/detections/detail?date=&time=&name=` | Single-detection detail — spectrogram, audio, correlation id, share |

## Admin

Sign-in (a session cookie) is required, when an admin password is set — a
fresh bare-metal install sets one automatically — for the `/admin*` routes, the
five Station management tabs and every page action that changes something (the
Today delete/relabel/lock posts, detection reviews, recordings lock/unlock/delete,
quarantine approve/reject/delete, the onboarding save and bulk review from
Search). The Station home's **Health** tab (`/station`) is public — the
operator-grade vital-signs surface that replaced the read-only `/system` page,
which now redirects to it. The management tabs are the gated pages; the old
`/admin/audio`, `/admin/species`, `/admin/notifications`, `/admin/rules`,
`/admin/quality`, `/admin/migrate`, `/admin/backups`, `/admin/accounts` and
`/admin` itself `308`-redirect into them. Every home above, the read-only
JSON/WebSocket API below and the feeds are open. See
[Remote Access & Security](../admin/remote-access.md).

| URL | Description |
|---|---|
| `/station/capture` | Audio sources (`#audio`) and species lists (`#species`) — the old `/admin/audio` and `/admin/species` redirect here |
| `/station/alerts` | Notifications (`#notifications`) and the alert-rule engine (`#rules`) — was `/admin/notifications`, `/admin/rules` |
| `/station/data` | Data quality (`#quality`), BirdNET-Pi import (`#import`), backups / restore / danger zone (`#backups`) — was `/admin/quality`, `/admin/migrate`, `/admin/backups` |
| `/station/settings` | General settings (the tab is labelled **General**) |
| `/station/access` | Accounts and access (`#accounts`) — was `/admin/accounts` |
| `/admin/settings` | The all-in-one form: audio, location, detection, notifications, email, MQTT, species, system |
| `/admin/system` | CPU / memory / temperature / disk |
| `/admin/overview` | Health-detail overview |
| `/admin/doctor` | The `--doctor` report rendered in the browser; `/admin/doctor.json` and `/admin/support-bundle` beside it |
| `/admin/audit` | Audit log (180 days) |
| `/admin/system/logs/page` | Live log viewer (SSE, level filtering) |
| `/admin/update/check` | Check GitHub Releases for a newer version (JSON). Applying is a separate `POST /admin/update/apply` with no UI button; the panel's **Check for Updates** button points you at `install.sh` instead |

## Public links & feeds

These are unauthenticated, read-only endpoints. See [Detection Detail & Sharing](../guide/sharing.md) and [RSS & Calendar Feeds](../guide/feeds.md).

| URL | Description |
|---|---|
| `/r/<token>` | Public share page for one detection (HMAC-signed, 30-day expiry) |
| `/r/<token>/audio.wav` | Redirect to the shared clip's audio |
| `/r/<token>/spectrogram.png` | Redirect to the shared clip's spectrogram |
| `/feeds/rare.rss` | RSS 2.0 — first-of-station detections (confidence > 0.85) |
| `/feeds/rare.ics` | iCalendar — the same rare detections as calendar events |
| `/feeds/today.rss` | RSS 2.0 — every detection today |

## API

The JSON API is versioned under `/api/v2`, with a WebSocket live stream at `/api/v2/ws/detections`. It has its own page: **[HTTP & WebSocket API](./api.md)** — endpoints, query parameters, response shapes and examples.
