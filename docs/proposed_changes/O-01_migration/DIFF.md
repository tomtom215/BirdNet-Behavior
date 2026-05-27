# O-01 · `/migration` — bird-migration phenology ridgeline

<!-- BNB:STATUS-HEADER -->
> **Risk:** low · **Priority:** 3 · **Status:** ready for review
> Acceptance: [VERIFY.md § O-01](../VERIFY.md#o-01--migration-ridgeline) · Rollback: [ROLLBACK.md § O-01](../ROLLBACK.md#o-01--migration-ridgeline)
<!-- BNB:STATUS-HEADER -->


## What

A new full page at `/migration` showing per-species weekly abundance as a stacked ridgeline plot over the calendar year, with KPI tiles, a diversity strip underneath, and three editorial cards (just arrived / peaking / missing).

## Naming collision

The existing `/admin/migration` route is **database migration from BirdNET-Pi** — totally unrelated to bird migration. Rename it to free up the URL:

```diff
- pub mod migration;
+ pub mod import;  // renamed from migration
```

with corresponding route move:

```diff
-  .route("/admin/migration", get(migration_page))
+  .route("/admin/import", get(import_page))
```

The mockup is `screens/migrate.jsx` and its heading is *"Bring your history with you"* — *import* matches its intent better and removes the ambiguity. Any existing bookmarks redirect cleanly.

## Files

| Action | Path |
|---|---|
| Add | `crates/birdnet-web/templates/migration.html` |
| Add | `crates/birdnet-web/src/routes/pages/migration.rs` |
| Patch | `crates/birdnet-web/src/routes/pages/mod.rs` — register `pub mod migration;` and `.merge(migration::router())` |
| Patch | `crates/birdnet-web/templates/layout.html` — add `<a href="/migration" class="topnav-link {{nav_migration}}">Migration</a>` in the analytics group |
| Patch | `crates/birdnet-web/src/routes/pages/mod.rs::render_page` — add `.replace("{{nav_migration}}", nav("migration"))` |

## Data

Pure SQL against the existing `detections` table — no schema changes, no behavioral extension required.

- **Ridges:** weekly count per species, normalized to its own peak. Migratory filter is a heuristic (peak / median > 3, at least 4 weeks with detections, peak count ≥ 5). When `--features analytics` is on, swap for `ResidencyType::Migrant`.
- **Today indicator:** dashed vertical line at the current SQLite `strftime('%W', 'now')` week.
- **KPIs:** first-of-year arrivals (count of species whose `MIN(Date)` falls in current year), peak diversity (week with the most `DISTINCT Com_Name`).
- **Editorial cards:** "just arrived" = species with most recent first-detection-in-year. "Peaking" = top species in last 7 days. "Missing" is stubbed — needs comparative prior-year data; render as `—`.

## Visual choices

- One color per species, sourced from the existing `pages::atoms::species_color()` (deterministic hash → OKLCH hue), so the same species reads the same on every page.
- Spring window (weeks 8–20) tinted with `--moss-soft`, fall window (34–44) with `--dawn-soft`. Quiet bands stay neutral.
- Ridges sorted by peak week → naturally reads left-to-right.
- Each ridge has its peak marker (dashed line + dot) so even thin curves stay readable.

## Performance

- Single SQL query for the ridgeline (one row per `species × week`); ~52 × N rows where N is migratory species. With 12 species cap, that's ~600 rows — negligible.
- The SVG is rendered once per page load; htmx polls `every 1h` on the KPI tiles and `every 30m` on the editorial cards, no auto-refresh on the heavy ridge SVG.
- Total response size ~25 KB uncompressed for the SVG.

## Risks

None to the existing app. The rename of `/admin/migration` → `/admin/import` is the only breaking change; add a 301 redirect to soften it for any cached bookmarks.

---

<!-- BNB:CROSSREF-FOOTER -->
## Related

* **Apply order:** shipped in the combined PR — see [HANDOFF.md](../HANDOFF.md#what-ships-in-this-pr) for the full file list.
* **Acceptance criteria:** [VERIFY.md § O-01](../VERIFY.md#o-01--migration-ridgeline).
* **Rollback:** [ROLLBACK.md § O-01](../ROLLBACK.md#o-01--migration-ridgeline).
* **Preview:** open [`INDEX.html`](../INDEX.html#O-01) for the rendered screen.
<!-- BNB:CROSSREF-FOOTER -->
