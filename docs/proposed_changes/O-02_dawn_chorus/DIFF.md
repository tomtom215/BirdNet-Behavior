# O-02 · `/analytics/dawn-chorus` — polar species ribbons

<!-- BNB:STATUS-HEADER -->
> **Risk:** none · **Priority:** 3 · **Status:** ready for review
> Acceptance: [VERIFY.md § O-02](../VERIFY.md#o-02--analyticsdawn-chorus-polar-ribbons) · Rollback: [ROLLBACK.md § O-02](../ROLLBACK.md#o-02--analyticsdawn-chorus-polar-ribbons)
<!-- BNB:STATUS-HEADER -->


## What

A new page mounted at `/analytics/dawn-chorus` showing per-species 24-hour activity as stacked polar ribbons around a day/night wedge, sunrise/sunset markers, and a current-time hand. Right rail repeats each species as a linear hour-strip + total count.

## Why

The existing `/timeseries` page already has a polar **hour-of-day** clock (single ring, aggregate). The mockup's `dawn-chorus.jsx` is a different chart — **per-species ribbons stacked radially** — answering "who sings, and when?" at a glance. This is the more useful visualization for a behavior-focused product, and it's been the most-cited absent screen in the drift report.

## Files

| Action | Path |
|---|---|
| Add | `crates/birdnet-web/templates/dawn_chorus.html` |
| Add | `crates/birdnet-web/src/routes/pages/dawn_chorus.rs` |
| Patch | `crates/birdnet-web/src/routes/pages/mod.rs` — register module + merge router |
| Patch | `crates/birdnet-web/templates/layout.html` — optional: add `/analytics/dawn-chorus` to the analytics group (or surface from `/analytics` page header) |

## Data

- Hourly histogram per species over the last 60 days, normalized to its own peak.
- Picks the top 8 species by total count. Adjust the constant in `collect_chorus(.., 60, 8)`.
- One color per species, taken from the shared `pages::atoms::species_color()` so it matches every other page.

## Sunrise / sunset

Currently stubbed at 05:30 / 20:00. To use the real station location:

```rust
fn station_sun_times() -> (f64, f64) {
    let cfg = state.config().location;
    let now = SystemTime::now();
    let (rise, set) = solar::sun_times(cfg.lat, cfg.lon, now);
    (rise.to_decimal_hours(), set.to_decimal_hours())
}
```

You'll need a tiny solar-position helper (the standard algorithm fits in ~80 lines). Or vendor `sun_times` / similar crate.

## Visual choices

- **Outer = highest-total species, inner = next, etc.** Avoids the most-active species being obscured.
- Day/night wedge uses `--night` at 6% opacity — subtle, doesn't compete with the ribbons.
- Current-time hand is dashed so it doesn't read as a chart element.
- Sunrise / sunset markers labeled with the actual computed times. The dawn (warmer) hue distinguishes them from the moss (alive) hue used for species.
- Right-rail strips use the same color per species; "off-chorus" pill flags species whose peak falls outside 05:00–08:00 (the literal dawn-chorus band).

## Performance

- Single SQL query, ~200 rows (species × hour) for typical setups.
- Pure SVG; no client JS beyond a tiny postprocessor that copies the rendered sunrise/sunset times into the page hero.
- Total response size ~12 KB.

## Risk

None. New page, no schema changes, no impact on existing routes.

---

<!-- BNB:CROSSREF-FOOTER -->
## Related

* **Apply order:** shipped in the combined PR — see [HANDOFF.md](../HANDOFF.md#what-ships-in-this-pr) for the full file list.
* **Acceptance criteria:** [VERIFY.md § O-02](../VERIFY.md#o-02--analyticsdawn-chorus-polar-ribbons).
* **Rollback:** [ROLLBACK.md § O-02](../ROLLBACK.md#o-02--analyticsdawn-chorus-polar-ribbons).
* **Preview:** open [`INDEX.html`](../INDEX.html#O-02) for the rendered screen.
<!-- BNB:CROSSREF-FOOTER -->
