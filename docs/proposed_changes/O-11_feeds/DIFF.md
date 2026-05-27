# O-11 · iCal + RSS feeds

<!-- BNB:STATUS-HEADER -->
> **Risk:** medium · **Priority:** 4 · **Status:** ready for review
> Acceptance: [VERIFY.md § O-11](../VERIFY.md#o-11--ical--rss-feeds) · Rollback: [ROLLBACK.md § O-11](../ROLLBACK.md#o-11--ical--rss-feeds)
<!-- BNB:STATUS-HEADER -->


## What

Three pure-read endpoints that let users subscribe to their station from any reader or calendar app.

| Endpoint | Format | Content |
|---|---|---|
| `/feeds/rare.rss` | RSS 2.0 | First-station-detection events, conf ≥ 0.85 |
| `/feeds/today.rss` | RSS 2.0 | Every detection today (chatty — for power users) |
| `/feeds/rare.ics` | iCalendar | Same rare set as `rare.rss`, as 3-min `VEVENT`s |

## Files

| Action | Path |
|---|---|
| Add | `crates/birdnet-web/src/routes/feeds.rs` |
| Patch | `crates/birdnet-web/src/routes/mod.rs` — `pub mod feeds;` and `.merge(feeds::router())` |
| Optional | discovery `<link rel="alternate" type="application/rss+xml" href="/feeds/rare.rss">` in `layout.html` `<head>` so browsers offer to subscribe |

## Query params

- `?limit=200` — max items (RSS clamped to 1..=500; iCal to 1..=1000)
- `?base=https://yourdomain` — override the base URL used in `<link>` and `<guid>` when the device is behind a reverse proxy. Or set the env var `BNB_BASE_URL`.

## Caching

- RSS: `Cache-Control: public, max-age=300` (5 min — matches typical reader poll cadence)
- iCal: `public, max-age=3600` (calendar apps poll slowly anyway)

## Tests

`feeds.rs` ships with unit tests for date formatting (`rfc822`, `ics_datetime`), ICS escaping, and that empty inputs still produce valid feed envelopes:

```sh
cargo test -p birdnet-web feeds
```

## Calendar UX

`iCalendar` events are emitted as 3-minute durations so they show up as a visible block in day/week views rather than collapsing to a single line. `CATEGORIES:rare-bird` lets users color them in Apple Calendar / Google Calendar.

## Risk

Zero. Pure read endpoints; no schema changes. The only network surface added is two URLs that anyone with the device IP can already hit via the existing API — feeds just give them a nicer envelope.

---

<!-- BNB:CROSSREF-FOOTER -->
## Related

* **Apply order:** shipped in the combined PR — see [HANDOFF.md](../HANDOFF.md#what-ships-in-this-pr) for the full file list.
* **Acceptance criteria:** [VERIFY.md § O-11](../VERIFY.md#o-11--ical--rss-feeds).
* **Rollback:** [ROLLBACK.md § O-11](../ROLLBACK.md#o-11--ical--rss-feeds).
* **Preview:** open [`INDEX.html`](../INDEX.html#O-11) for the rendered screen.
<!-- BNB:CROSSREF-FOOTER -->
