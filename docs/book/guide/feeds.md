# RSS & Calendar Feeds

The station publishes lightweight, public, read-only feeds so you can follow your yard from a feed reader or a calendar app without leaving a browser tab open.

| Feed | Path | Format |
|------|------|--------|
| Rare / first-of-station | `/feeds/rare.rss` | RSS 2.0 |
| Rare / first-of-station | `/feeds/rare.ics` | iCalendar |
| Everything today | `/feeds/today.rss` | RSS 2.0 |

The **rare** feeds list first-ever-at-this-station detections with confidence ≥ 0.85 — a low-noise stream of genuinely new birds. The **today** feed is every detection from the current day (chatty by design).

## Discovery

The dashboard advertises the rare RSS feed in its `<head>`, so feed readers offered the page will find it automatically:

```html
<link rel="alternate" type="application/rss+xml"
      title="BirdNet · rare detections" href="/feeds/rare.rss">
```

Each feed item links back to the [detection detail page](./sharing.md) for that bird, and the iCal events carry a stable `UID`, a 3-minute duration, and the same detail link.

## `BNB_BASE_URL` — make the links absolute

Feed items need absolute URLs so they work in an external reader. Set `BNB_BASE_URL` to how the station is reached from outside:

```bash
BNB_BASE_URL=http://birdnet.local        # mDNS on the LAN
# or
BNB_BASE_URL=https://birds.example.com    # behind a reverse proxy
```

If unset it defaults to `http://localhost:8502` (the server's own port). You can also override per-request with `?base=…`, and cap item counts with `?limit=…`.

## Caching

RSS responds with `Cache-Control: public, max-age=300` (5 minutes); iCal with one hour, since calendar clients repoll slowly. An empty station still returns valid, well-formed XML/iCalendar.

> These endpoints are unauthenticated and expose the same information as the public dashboard. If you gate the UI behind a reverse proxy, decide whether to expose `/feeds/*` alongside it.
