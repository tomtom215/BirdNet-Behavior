# O-21 · Changelog viewer + post-upgrade banner

<!-- BNB:STATUS-HEADER -->
> **Risk:** none · **Priority:** 3 · **Status:** ready for review
> Acceptance: VERIFY.md § O-21 · Rollback: ROLLBACK.md § O-21
<!-- BNB:STATUS-HEADER -->


## What

`CHANGELOG.md` is 66 KB at the repo root with detailed entries for every release. The running app shows the version in the footer (`BirdNet-Behavior v{{version}}`) but offers no link to find out what changed.

This change ships two surfaces:

1. **`/system/changelog`** — a page that renders the embedded `CHANGELOG.md` with the design system applied. Reuses the `bnb-help-drawer__body` typography rules (sub-CSS, no new tokens). Anchored headings so a release tag is a permalink (`/system/changelog#v1-4-2`).
2. **Post-upgrade banner** on `/system` and (on first navigation after upgrade) `/` — a single `bnb-banner` row reading *"You're on v1.5.0 — see what changed →"* with a dismiss button. The dismiss writes the current version to `localStorage.bnb-last-seen-version`; the banner stays hidden until the version moves up.

No new dependencies. `CHANGELOG.md` is `include_str!`-ed at build time alongside the templates, parsed in pure Rust (the parser only needs to recognise `## [vX.Y.Z]` headings, dates, and the bullet blocks under them — the file follows a *Keep a Changelog* structure).

## Files

| Action | Path |
|---|---|
| Add | `crates/birdnet-web/src/routes/pages/changelog.rs` — parser + `/system/changelog` page + `/system/changelog/latest` partial |
| Add | `crates/birdnet-web/templates/_partial_update_banner.html` — banner markup + dismiss script |
| Append | `crates/birdnet-web/static/css/app.css` — see `css/app.css.append` |
| Patch | `crates/birdnet-web/src/routes/pages/mod.rs` — `pub(crate) mod changelog;` + `.merge(changelog::router())` |
| Patch | `crates/birdnet-web/templates/layout.html` — include `_partial_update_banner.html` immediately after `<main id="main-content">` (it's empty unless the FOUC guard adds `data-update-banner-pending` to `<html>`) |

## Parser scope

The `Keep a Changelog` shape it expects:

```
## [1.5.0] - 2026-04-22

### Added
- Themed confirmation modal with sticky-toast follow-up
- /analytics/dawn-chorus polar plot per species

### Fixed
- Day-strip now respects station time zone

## [1.4.2] - 2026-04-12
...
```

The parser produces a `Vec<Release { version, date, sections: [(heading, [bullet])] }>` with no allocation per bullet beyond the bullet's own string slice (uses `&str` references into the embedded `CHANGELOG.md`). The output is rendered to HTML with stable anchor ids derived from the version (`v1-5-0`).

If the file ever drifts from that shape, the parser falls back to "render the raw markdown as a `<pre>` block" — never blank, never panicking.

## Banner trigger

The post-upgrade banner is **client-side gated** to keep the server stateless:

```html
<!-- in layout.html, right after the opening <main> -->
<div id="bnb-update-banner-mount" data-current-version="{{version}}"></div>
```

The script in the banner partial reads `localStorage.bnb-last-seen-version`. If the stored value parses to a semver less than `data-current-version`, the banner is rendered in place. Dismissal writes the current version into localStorage and removes the banner. New install (no stored value) — banner stays hidden; we don't celebrate a first-launch as an upgrade.

The banner's *Subject* and *body* are derived from the **latest release block** in `CHANGELOG.md` — fetched via `/system/changelog/latest` (returns just the top `<article>`). This avoids hard-coding any release copy into the layout.

## Visual

A wide `.bnb-banner` strip across the top of `<main>`:

- Tinted background (`color-mix(in oklch, var(--moss) 8%, var(--surface))`).
- A `bnb-pill moss` on the left with the new version number.
- A serif headline ("New in v1.5.0") and a one-line summary ("Confirmation modals · skeletons · command palette").
- A `bnb-btn ghost` "See all changes →" link.
- A close × button (right edge).

No reflow above the page contents — the banner replaces the top spacing, doesn't push everything down. `prefers-reduced-motion` skips the fade-in.

## Risk

Zero. Banner is a no-op when localStorage matches the current version (every subsequent page load after dismissal). Changelog page is purely additive.

---

<!-- BNB:CROSSREF-FOOTER -->
## Related

* Reuses the `_partial_help_drawer.html` body typography for the changelog page.
* The "See all changes" link is also surfaced by the O-19 command palette under the *Settings* group (`changelog`).
<!-- BNB:CROSSREF-FOOTER -->
