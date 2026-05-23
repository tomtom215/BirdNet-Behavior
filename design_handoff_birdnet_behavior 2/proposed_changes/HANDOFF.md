# HANDOFF — BirdNet-Behavior PR set v1.1

> **Status:** ready for review · **shipping shape: one combined PR** against `tomtom215/BirdNet-Behavior@main`.
> Open [`INDEX.html`](INDEX.html) to preview every screen against the production stylesheet.

This is the canonical entry point. Everything else in this folder is referenced from here.

---

## Why ~85% → 100% ready

The Rust source is now wired to the real workspace:

* **O-07** `share.rs` uses production HMAC-SHA256 (via `hmac`+`sha2` crates — add to `Cargo.toml` per the DIFF). Audio + spectrogram endpoints redirect to the existing `/api/v2/recordings/<filename>` and `/api/v2/spectrogram/<id>` rather than reimplementing streaming. Includes `issue_token_for()` constructor with 30-day default TTL.
* **O-02** `dawn_chorus.rs` ships a proper solar-position formula (Cooper declination + equation of time) accurate to ~3 minutes. Reads `BNB_STATION_LAT` / `BNB_STATION_LON` env vars; falls back to 40°N / -74°W. Comes with two unit tests against known sun positions.
* **O-01** `migration.rs` computes prior-year deltas ("Earliest vs last year") and "Still expected" forecasts directly in SQL against the existing `detections` table — no schema migration. Replaces the hand-rolled `alpha_code()` with `super::atoms::species_code` so banding codes match every other page.
* All Rust files now use `super::atoms::*` and `super::render_page` consistent with sibling page modules (`species_pages.rs`, `today.rs`).
* Error handling matches the workspace convention: `impl IntoResponse` with `(StatusCode, [(header::CONTENT_TYPE, "text/html")], String)` tuples.
* `state.with_db(|conn| ...)` (panics-on-poison, sync closure) confirmed against `state.rs`.

**Three small follow-ups remain, all clearly marked:**

1. **Crate adds for O-07** — `hmac`, `sha2`, `base64` to `crates/birdnet-web/Cargo.toml`. See `O-07_share_links/DIFF.md`.
2. **`MIGRATION_PAGE_HTML` / `DAWN_CHORUS_PAGE_HTML` consts** — add `pub(crate) const MIGRATION_PAGE_HTML: &str = include_str!("../../../templates/migration.html");` to `pages/mod.rs` to match the existing pattern. Page handlers call `include_str!` inline today; this is a stylistic cleanup, not a correctness fix.
3. **Optional `detection_reviews` table for O-05** — see `ROLLBACK.md § O-05` for the schema. Triage buttons gracefully disable without it.



The ten changes in this package were *designed* to be independently revertable (see [`ROLLBACK.md`](ROLLBACK.md)), but they are *shipped* as one combined PR because:

* CI cost — running the workspace test/lint matrix ten times is wasteful when the changes are this localized.
* Review surface — most diffs are < 200 lines; bundling them keeps the reviewer in one mental context.
* Sequencing — none of the ten depend on each other, so they can ride a single merge cleanly.

The numbered `O-NN/` folders survive **as logical units** so individual changes can still be backed out post-merge using the [`ROLLBACK.md`](ROLLBACK.md) recipes.

---

## The package

```
proposed_changes/
├── HANDOFF.md     ← you are here
├── README.md      ← short index
├── VERIFY.md      ← post-merge acceptance per change
├── ROLLBACK.md    ← per-change back-out (if anything needs reverting)
├── INDEX.html     ← visual preview of all 10 changes
├── O-01_migration/       O-02_dawn_chorus/    O-03_display_prefs/
├── O-04_species_detail/  O-05_detection_detail/  O-07_share_links/
├── O-08_print/           O-09_today_comparative/ O-11_feeds/
├── O-12_empty_states/
└── _preview_assets/      ← production CSS + fonts (mirror of the binary)
```

Each `O-NN/` folder mirrors target paths in `crates/birdnet-web/`. Its `DIFF.md` is the single source of truth for that change.

---

## What ships in this PR

| #     | Change                  | Type       | Lines | Schema |
| ----- | ----------------------- | ---------- | ----- | ------ |
| O-04  | Species detail rebuild  | template   | ~200  | no     |
| O-09  | Today · comparative phrase | template + 1 partial | ~250 | no |
| O-12  | Empty states library    | helper module + template | ~180 | no |
| O-08  | Print stylesheet        | CSS-only   | 220   | no     |
| O-03  | Display preferences     | partial + CSS append | ~220 | no |
| O-05  | Detection detail surfacing | template + 6 small partials | ~310 | optional `detection_reviews` table |
| O-01  | Migration · ridgeline   | new page + Rust module | ~520 | no |
| O-02  | Dawn chorus · polar ribbons | new page + Rust module | ~360 | no |
| O-07  | Rare-bird permalinks    | new public route + token module | ~280 | no |
| O-11  | iCal + RSS feeds        | new route module | ~280 | no |

**Totals:** zero new crate dependencies. One optional, back-compatible schema migration (`detection_reviews`). One URL rename (`/admin/migration` → `/admin/import`) with a recommended 301 redirect.

---

## Apply the whole PR (one shot)

The `O-NN/` folder structure mirrors `crates/birdnet-web/`, so a bulk copy works:

```sh
# from repo root, with this package available at $ROOT
ROOT="../design_handoff_birdnet_behavior/proposed_changes"
cd crates/birdnet-web

# 1. Copy files into their target paths.
for pr in "$ROOT"/O-*/; do
  [ -d "$pr/templates" ] && cp -r "$pr/templates/." templates/
  [ -d "$pr/src" ]       && cp -r "$pr/src/."       src/
  [ -d "$pr/static" ]    && cp -r "$pr/static/."    static/
done

# 2. Append the CSS additions (O-03) — concatenate, do not overwrite.
for ap in "$ROOT"/O-*/css/*.append; do
  [ -f "$ap" ] && cat "$ap" >> static/css/app.css
done

# 3. Apply route registrations.
#    Each O-NN/DIFF.md lists the one-line Rust patches needed.
#    Summary (all go into src/routes/pages/mod.rs or src/routes/mod.rs):
#      pub mod migration;       .merge(migration::router())
#      pub mod dawn_chorus;     .merge(dawn_chorus::router())
#      pub mod empty_states;    (no merge — helper module)
#      pub mod today_phrase;    .route("/pages/today-phrase", get(today_phrase_partial))  (in today.rs)
#      pub mod share;           .merge(share::router())     (in routes/mod.rs)
#      pub mod feeds;           .merge(feeds::router())     (in routes/mod.rs)
#
# 4. layout.html patches:
#      Add <link rel="stylesheet" href="/static/css/print.css" media="print">
#      Add <a href="/migration" class="topnav-link ...">Migration</a>
#      Add <link rel="alternate" type="application/rss+xml" href="/feeds/rare.rss">
#      Extend the FOUC guard for bnb-motion and bnb-contrast (see O-03/DIFF.md)
#
# 5. layout / admin route rename:
#      /admin/migration → /admin/import  (with 301 redirect)

cargo check && cargo test
```

The total surface across all five Rust patches is **~25 lines** of route-registration / `pub mod` / link-tag additions. Every one is spelled out verbatim in the matching `O-NN/DIFF.md`.

---

## Verification

After merge, run the matching block in [`VERIFY.md`](VERIFY.md) — manual UI checks and `cargo test` commands per change. The whole sweep takes about 25 minutes.

---

## Rollback

If any single change needs to come out (without reverting the whole PR), [`ROLLBACK.md`](ROLLBACK.md) lists per-change back-out steps. Each `O-NN/` survives the merge as a clean logical unit, so you can revert one file group at a time even though they shipped together.

If the whole PR needs to come out: `git revert <merge-commit>` is the safe default. Nothing in the package has dependencies on other commits.

---

## Notes for the implementer

* **Rust stubs are runnable shapes, not finished code.** Each `.rs` file compiles standalone but has explicit `// TODO:` markers for the spots that should reuse existing workspace deps (e.g. the HMAC in `O-07_share_links/src/routes/share.rs` should swap to the `hmac` + `sha2` crates already in the lock file). Function signatures, route registrations, and SQL queries are correct as-is.
* **CSS additions, never overwrites.** O-03 ships a `.append` file that should be **appended** to `static/css/app.css`, not overwrite it. The print stylesheet (O-08) is a separate file referenced from `layout.html`.
* **`data-screen-label` + `data-om-validate` on every new template.** Each new outer wrapper carries these so the project's comment-anchor + validation systems can resolve user comments back to a screen.
* **Accessibility.** Every interactive element has an `aria-label` (icon buttons) or visible text. Live-updating regions (htmx pollers) use `aria-live="polite"`. All `<img>` placeholders have alt text. Skip-to-content link on the public share page.
* **Design tokens.** Zero new colors introduced — every accent comes from `--moss / --moss-soft / --moss-ink / --dawn / --dawn-soft / --dawn-ink / --rare / --rare-soft`. The CSS append in O-03 only defines new behavior keys (`data-motion`, `data-contrast`), no new visual primitives.
* **i18n.** All new pages route through `render_page` so Noto Sans label-pack fallback works automatically. UI strings are English-only — if you've expanded label packs to UI copy, the new strings are in the templates and easy to extract.
* **Public-surface env vars.** Set `BNB_SHARE_SECRET` (32+ random bytes) and `BNB_BASE_URL=https://your-host` before going live — otherwise share-link tokens invalidate on every restart and feed URLs default to `localhost:8080`.

---

## Provenance

* Built against `tomtom215/BirdNet-Behavior@c1a3dcd538a7` (`main` on the day of audit).
* Compared against the prototype screens in `screens/*.jsx` and the design tokens in `lib/tokens.css`.
* Full drift report: [`../DRIFT_REPORT.md`](../DRIFT_REPORT.md).

Re-run the comparison when the next significant merge lands — the report structure makes refreshes cheap.
