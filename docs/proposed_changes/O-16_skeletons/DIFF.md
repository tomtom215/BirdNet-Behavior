# O-16 · Skeleton loading states

<!-- BNB:STATUS-HEADER -->
> **Risk:** none · **Priority:** 2 · **Status:** ready for review
> Acceptance: VERIFY.md § O-16 · Rollback: ROLLBACK.md § O-16
<!-- BNB:STATUS-HEADER -->


## What

Every htmx swap target in the codebase currently renders `<p class="bnb-meta">Loading…</p>` as its placeholder. That's fine prose but it tells the user nothing about the shape that's coming, and it forces a visible reflow when the partial finally arrives.

This change adds:

1. A single `.bnb-skel` utility class with a shimmer keyframe (lives in `app.css`).
2. A Rust helper module `pages::skeletons` that emits the right skeleton-shape HTML for every recurring partial type — feed rows, stat tiles, polar plot, ridgeline, day-strip, hourly bars, species-row, hero card, audio scrubber.
3. A drop-in patch list mapping each "Loading…" string in the existing templates to the matching helper.

The skeleton fragments use only existing tokens (`--surface-2`, `--hairline`) plus one new keyframe. Reduced-motion is respected via the existing `@media (prefers-reduced-motion: reduce)` rule already in `app.css` — the shimmer is the only animation it needs to cover.

## Files

| Action | Path |
|---|---|
| Add | `crates/birdnet-web/src/routes/pages/skeletons.rs` |
| Append | `crates/birdnet-web/static/css/app.css` — see `css/app.css.append` |
| Patch | `crates/birdnet-web/src/routes/pages/mod.rs` — `pub(crate) mod skeletons;` |
| Reference | `templates/_skeleton_examples.html` — every shape rendered together, useful for designers and as a regression target |

## Migration: which partial shows which skeleton

The grep target is the literal string `>Loading…<` (and its variants like `Loading detections…`, `Loading chart…`, `Building polar plot…`). Replace each with the matching call:

| Today's template / partial | Today's placeholder | Replacement |
|---|---|---|
| `templates/today.html` · `#today-results` | "Loading detections…" | `skeletons::feed_rows(8)` |
| `templates/today.html` · `#today-daystrip` | "Loading timeline…" | `skeletons::day_strip()` |
| `templates/dawn_chorus.html` · `#dawn-polar` | "Building polar plot…" | `skeletons::polar_plot()` |
| `templates/dawn_chorus.html` · `#dawn-list` | "Loading ribbons…" | `skeletons::species_ribbons(6)` |
| `templates/migration.html` · `#migration-ridgeline` | "Building ridgeline…" | `skeletons::ridgeline()` |
| `templates/migration.html` · `#migration-diversity` | (empty) | `skeletons::diversity_bars()` |
| `templates/migration.html` · stat tiles | `<span class="value">—</span>` | `skeletons::stat_row(4)` |
| `templates/species_detail.html` · `#species-status` | "loading…" pill | `skeletons::pill_row(3)` |
| `templates/species_detail.html` · `#species-hero` | "loading…" caption + meta | `skeletons::hero_card()` |
| `templates/species_detail.html` · `.stat-row` | "—" placeholders | `skeletons::stat_row(4)` |
| `templates/species_detail.html` · circadian / trend / detections cards | "Loading chart…" / "Loading detections…" | `skeletons::hourly_bars(24)` / `skeletons::trend_line()` / `skeletons::list_rows(5)` |
| `pages/dashboard/partials.rs` · live feed first-render | (server emits real rows) | use `skeletons::feed_rows(10)` only on **initial GET** when DB has 0 rows; after that empty state `empty_states::quiet_yard()` is correct |
| `admin/quality.rs` · `#quality-summary`, `#quality-trend` | inline placeholders | `skeletons::stat_row(6)` / `skeletons::trend_line()` |

For partials served as separate HTMX endpoints, the skeleton lives **inline in the parent template** (between the opening tag of the `hx-get` element). HTMX automatically replaces it when the response arrives.

## Behavior

- The shimmer keyframe is a 1.4s cubic-bezier sweep across the element's background. One keyframe shared by every shape.
- The keyframe is wrapped in `@media (prefers-reduced-motion: no-preference)` — under reduced motion the elements still render at the right shape, just without the moving highlight.
- Each shape preserves the **same height** as the loaded content. No layout shift when the swap lands.
- A `[aria-busy="true"]` attribute is set on every skeleton wrapper so screen readers announce the loading state once.

## Risk

Zero — purely additive utility class, additive Rust module, optional per-partial migration. Existing "Loading…" strings keep working until each is migrated.

---

<!-- BNB:CROSSREF-FOOTER -->
## Related

* **Apply order:** ship before O-17, O-18 (they reuse the same shimmer keyframe). See `HANDOFF.md`.
* **Preview:** open `templates/_skeleton_examples.html` in the proposed-changes preview rig.
<!-- BNB:CROSSREF-FOOTER -->
