# RSS & Calendar Feeds

The station publishes lightweight, public, read-only feeds so you can follow your yard from a feed reader or a calendar app without leaving a browser tab open.

| Feed | Path | Format |
|------|------|--------|
| Rare / first-of-station | `/feeds/rare.rss` | RSS 2.0 |
| Rare / first-of-station | `/feeds/rare.ics` | iCalendar |
| Everything today | `/feeds/today.rss` | RSS 2.0 |

The **rare** feeds list first-ever-at-this-station detections with confidence above 0.85 (strictly `> 0.85`) — a low-noise stream of genuinely new birds. The **today** feed is every detection from the current day (chatty by design).

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

If unset it defaults to `http://localhost:8502` (the server's own port). A request can name its own base with `?base=…`, which is how a reverse proxy gets the scheme and path prefix right, but only for an address the station already knows: the host the request was addressed to, or the host of `BNB_PUBLIC_URL` or `BNB_BASE_URL`. Any other `?base=` is ignored and the default is used, so a feed address cannot be made to point at someone else's site. If a proxy rewrites `Host` and your readers pass `?base=`, set `BNB_BASE_URL`. Cap item counts with `?limit=…`.

## Caching

RSS responds with `Cache-Control: private, max-age=300` (5 minutes); iCal with `private, max-age=3600` (one hour), since calendar clients repoll slowly. Both say `private`, so a shared cache or CDN in front of the station will not keep a copy. An empty station still returns valid, well-formed XML/iCalendar.

> These endpoints are unauthenticated and expose the same information as the public dashboard. If you gate the UI behind a reverse proxy, decide whether to expose `/feeds/*` alongside it.
