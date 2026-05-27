# O-24 · Mobile / PWA — bottom tab bar, manifest, service worker

<!-- BNB:STATUS-HEADER -->
> **Risk:** low (additive surface) · **Priority:** 3 · **Status:** ready for review
> Acceptance: VERIFY.md § O-24 · Rollback: ROLLBACK.md § O-24
<!-- BNB:STATUS-HEADER -->


## What

The shipped responsive CSS handles layout collapse (`@media (max-width: 720px)`) for feed rows, the topnav, and the stat row. That's enough to *survive* a phone but not enough to feel native on one. Three orthogonal additions:

1. **Phone bottom tab bar.** A 6-slot navigation pinned to the bottom of the viewport at widths ≤ 720 px, replacing the wrapped topnav links. Five primary destinations (Dashboard, Today, Species, Heatmap, Migration) plus a `⌗ More` slot that opens a sheet to the remainder of the topnav.
2. **PWA manifest + icons.** A `manifest.webmanifest` declares the app name, icons, theme colour, and `display: standalone`. The dashboard becomes home-screen-installable; the kiosk room TV can `Add to Home Screen` and pin the kiosk URL as a kiosk app.
3. **Service worker.** Caches the design system + static assets + a "last-known dashboard" so a kiosk-mode TV that loses its router for 90 seconds still shows the last dashboard with a quiet `offline` banner. **Public read-only endpoints only** — the worker never caches `/admin/*` or any signed share link.

Larger tap targets on existing rows (Today list, Species list, recordings) come for free with the bottom-bar opportunity since the `@media (pointer: coarse)` query is added in the same change.

## Files

| Action | Path |
|---|---|
| Add | `crates/birdnet-web/templates/_partial_tabbar.html` — phone-only bottom nav, included in `layout.html` |
| Add | `crates/birdnet-web/static/manifest.webmanifest` |
| Add | `crates/birdnet-web/static/sw.js` — service worker (precache + runtime cache rules) |
| Add | `crates/birdnet-web/static/icon-192.png`, `icon-512.png`, `icon-maskable-512.png` — three icons derived from the existing `BrandMark` SVG (sound-wave-in-circle) |
| Append | `crates/birdnet-web/static/css/app.css` — see `css/app.css.append` |
| Patch | `crates/birdnet-web/templates/layout.html` — add `<link rel="manifest" …>`, `<meta name="theme-color" …>`, the worker bootstrap, and the tab-bar partial |

## Phone bottom bar

```
┌────────────────────────────────────────────────────────────┐
│                                                            │
│             page contents scroll up here                   │
│                                                            │
├────────────────────────────────────────────────────────────┤
│   ⌂        ⊙       ⌬       ▦        ∿        ⌗            │
│ Dashbd   Today  Species Heatmap Migration  More            │
└────────────────────────────────────────────────────────────┘
```

- Sits as a `<nav class="bnb-tabbar">` pinned `position: fixed; bottom: 0`.
- Visible only when `(max-width: 720px) and (pointer: coarse)`. Mouse users on small windows keep the topnav.
- Each slot is a 56×56 px tap target with a 22 px glyph and 10 px label.
- Active state mirrors the existing `.topnav-link.active` styling (moss-ink text + soft moss background).
- The `⌗ More` slot opens a `<dialog>`-as-bottom-sheet with the secondary destinations (Life list, Quarantine, Notifications, System, Admin, Sign out). The sheet reuses the help-drawer mobile slide-in keyframe.
- The topnav remains in the DOM (for desktop) but its **links list** is hidden under `(max-width: 720px) and (pointer: coarse)` since the tabbar covers it.

`<main>` gains `padding-bottom: 76px` only when the tab bar is visible, so content never hides beneath it.

## Manifest

```json
{
  "name": "BirdNet-Behavior",
  "short_name": "BirdNet",
  "description": "Acoustic bird-monitoring station",
  "id": "/?source=pwa",
  "start_url": "/",
  "scope": "/",
  "display": "standalone",
  "orientation": "any",
  "theme_color": "oklch(98.5% 0.004 80)",
  "background_color": "oklch(98.5% 0.004 80)",
  "icons": [
    { "src": "/static/icon-192.png",          "sizes": "192x192", "type": "image/png" },
    { "src": "/static/icon-512.png",          "sizes": "512x512", "type": "image/png" },
    { "src": "/static/icon-maskable-512.png", "sizes": "512x512", "type": "image/png", "purpose": "maskable" }
  ],
  "shortcuts": [
    { "name": "Today",       "url": "/today",       "description": "Detection log for today" },
    { "name": "Migration",   "url": "/migration",   "description": "Arrivals and departures this year" },
    { "name": "Quarantine",  "url": "/quarantine",  "description": "Rare-bird review queue" }
  ]
}
```

Two theme-color meta tags ride along — one for light, one for dark — so iOS Safari and Chrome paint the address bar correctly under both themes.

## Service worker — caching policy

| Route pattern | Policy |
|---|---|
| `/static/css/app.css`, `/static/fonts/*`, `/static/htmx.min.js` | **Cache-first.** Versioned by the build-time `?v=` query param. New build → new cache key. |
| `/static/icon-*.png`, `/static/manifest.webmanifest` | **Cache-first.** |
| `/` (the dashboard) | **Stale-while-revalidate.** Last successful response is served on instant-load, then a fresh fetch updates the cache in the background. Lets a kiosk TV keep showing the last dashboard during a brief router outage. |
| `/today`, `/species`, `/heatmap`, `/migration` | Same as `/`. |
| `/api/v2/*`, `/pages/*` (htmx partials), `/api/v2/ws/*` | **Network-only.** Never cached — these are live data. |
| `/admin/*`, `/login`, `/logout`, `/r/*`, `/feeds/*` | **Network-only and bypass the worker entirely** — auth and signed share links must never be cached. |

`navigator.serviceWorker.register('/static/sw.js')` is the only client-side hook; failures are logged silently (no console clutter on browsers that don't support SW). A `<meta name="bnb-build">` value lets the worker detect a new build and invalidate the cache; the `bnb-toasts` region is reused to surface "Update available — reload" when a new build is in cache and ready.

## Mobile-first row patterns

In addition to the tab bar, the responsive CSS gains:

- `@media (pointer: coarse)` rules that bump the minimum row height on feed / species / recordings list rows from current 40-ish px to **52 px** (Apple HIG minimum is 44 px; we go slightly above so a confidently mistyped tap still works).
- Heatmap mosaic cells go from 12 × 12 to 16 × 16 on coarse pointers — still legible, easier to touch.
- The `topnav-right` collapses entirely on phones (live pill, station handle, theme toggle hide); their functions live on the More sheet or in O-19's command palette.

## Risk

Low. The tab bar is a phone-only overlay. The manifest + worker are opt-in by the browser. The worker is configured **fail-closed** — any `fetch` error inside the worker yields control back to the network, never a cached stale auth response.

---

<!-- BNB:CROSSREF-FOOTER -->
## Related

* Uses the O-17 `<dialog>` primitive for the "More" sheet.
* Uses the O-18 toast region for the "Update available — reload" prompt.
* The PWA shortcuts surface deep links the same way O-19 (cmdk) does for desktop.
<!-- BNB:CROSSREF-FOOTER -->
