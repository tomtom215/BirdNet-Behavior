# O-20 · Help / methodology surface

<!-- BNB:STATUS-HEADER -->
> **Risk:** low · **Priority:** 2 · **Status:** ready for review
> Acceptance: VERIFY.md § O-20 · Rollback: ROLLBACK.md § O-20
<!-- BNB:STATUS-HEADER -->


## What

The repo ships a full mdBook at `docs/book/` covering every screen in the running app — *Dashboard*, *Today*, *Behavioral Analytics*, *Migration*, *Dawn Chorus*, *Tuning Detection Accuracy*, *Glossary*, *FAQ*. The running app exposes none of it. The only methodology surface today is `<HelpDot>` tooltips, which can't carry a paragraph and can't be deep-linked.

This change ships three things:

1. A small "How this works →" affordance, server-rendered at the top of every analytical screen, deep-linked to the right mdBook page.
2. A `/help` route that serves the mdBook build output as static files (or rebuilt fragments embedded into the app shell so the docs read like the rest of the product, not a separate site).
3. Two Rust helpers in `pages::help`: `help_link(topic)` and `help_drawer(topic)` — the second swaps the docs page into a right-side `<dialog>` drawer for in-place reading.

No docs are written or duplicated here. We **point at** what's already in `docs/book/`.

## Files

| Action | Path |
|---|---|
| Add | `crates/birdnet-web/src/routes/pages/help.rs` — `help_link(topic)` helper + `/help/*` route serving `docs/book/_generated/{html}` |
| Add | `crates/birdnet-web/templates/_partial_help_drawer.html` — right-side `<dialog>` that loads `/help/{topic}` inline |
| Append | `crates/birdnet-web/static/css/app.css` — see `css/app.css.append` |
| Patch | `crates/birdnet-web/build.rs` — invoke `mdbook build docs/book` and embed the output under `OUT_DIR/help/` (build-time, no runtime mdbook dep) |
| Patch | every analytical template — one line each, see "Site list" below |

## Helper signature

```rust
// pages/help.rs
pub fn help_link(topic: Topic) -> &'static str;     // returns <a> markup with deep link
pub fn help_drawer(topic: Topic) -> &'static str;   // returns <button> markup that opens the drawer

pub enum Topic {
    Dashboard, Today, Sharing, Reviews, Species,
    Analytics, Phenology, DawnChorus, Recordings,
    Feeds, Reports, DisplayPrefs, Kiosk,
    AdminSettings, AdminAudio, AdminRecording,
    AdminNotifications, AdminRemoteAccess, AdminBackups, AdminSystem,
    Tuning, Glossary, FAQ, Troubleshooting,
}
```

Each `Topic` maps 1:1 to a `docs/book/<section>/<page>.md` and a stable URL like `/help/guide/analytics`.

## Site list — where each link goes

| Screen / template | Topic | Section heading text the link sits under |
|---|---|---|
| `templates/dashboard.html` | `Topic::Dashboard` | (eyebrow row) |
| `templates/today.html` | `Topic::Today` | "Detection log · today" eyebrow |
| `templates/species_detail.html` | `Topic::Species` | next to the "Companions" / "About" cards |
| `templates/dawn_chorus.html` | `Topic::DawnChorus` | "Behavioral analytics · circadian" eyebrow row |
| `templates/migration.html` | `Topic::Phenology` | "Behavioral analytics · phenology" eyebrow row |
| `templates/analytics.html` | `Topic::Analytics` | first card |
| `pages/heatmap.rs` (Rust-rendered) | `Topic::Analytics` | next to the streamgraph legend |
| `pages/correlation.rs` (Rust-rendered) | `Topic::Analytics` | matrix subtitle |
| `pages/year_in_review.rs` | `Topic::Reports` | masthead |
| `pages/weekly_report.rs` | `Topic::Reports` | masthead |
| `pages/quarantine.rs` | `Topic::Reviews` | header bar |
| `pages/life_list.rs` | `Topic::Species` | year-tape header |
| `pages/recordings.rs` | `Topic::Recordings` | header bar |
| `pages/gallery.rs` | `Topic::Recordings` | header bar |
| `pages/notification_center.rs` | `Topic::AdminNotifications` | header bar |
| `pages/system_dashboard.rs` | `Topic::AdminSystem` | next to "self-check" panel |
| `admin/quality.rs` | `Topic::Tuning` | next to "Low-confidence species" — explicitly recommends *Tuning Detection Accuracy* |
| `admin/audio.rs` (post-O-13) | `Topic::AdminAudio` | header subtitle, next to the "primary support surface" eyebrow |
| `admin/settings/render/detection.rs` | `Topic::AdminSettings` | each settings sub-card |
| `admin/notifications.rs` | `Topic::AdminNotifications` | header bar |
| `admin/backup_recovery.rs` | `Topic::AdminBackups` | header bar AND inside the danger zone |
| `admin/migration.rs` (BirdNET-Pi importer) | `Topic::AdminBackups → guides/migration` | first step |
| `admin/doctor.rs` | `Topic::Troubleshooting` | top |
| `admin/rules.rs` | `Topic::AdminNotifications` | rule-builder hint |

## Two modes — link or drawer

Most analytical screens get the **link** variant: a small `bnb-help-link` next to the eyebrow that opens `/help/...` in a new tab. The **drawer** variant is reserved for first-encounter teaching moments — Quarantine's "what does ρ mean?" link, the audio-settings "what does +6 dB do?" link — where pulling the user fully out of the screen would lose context. Both share the same `Topic` enum.

```html
<!-- Link variant (most screens) -->
<a class="bnb-help-link" href="/help/guide/dawn-chorus" target="_blank" rel="noopener">
  How this works
  <span aria-hidden="true">→</span>
</a>

<!-- Drawer variant (single-page contextual) -->
<button class="bnb-help-link" type="button"
        data-help-drawer="/help/guides/tuning#sf-threshold">
  Why per-species frequency matters
  <span aria-hidden="true">↗</span>
</button>
```

## Build-time wiring

`mdbook` is the only sensible source of these docs in production. Two viable shapes:

**A. Build-time embed (recommended).** `build.rs` calls `mdbook build docs/book --dest-dir target/help`, then `include_dir!` (or a manual `walkdir` + `include_bytes!`) inlines the HTML into the binary alongside the existing self-hosted fonts. Runtime: serve them under `/help/` via `axum::routing::get_service`. Zero runtime deps; the Pi never touches mdBook at runtime.

**B. Runtime fetch.** Skip the embed; assume the host has `docs/book/book/` next to the binary. Lighter binary, but breaks the existing "single static binary" principle. **Not recommended.**

This DIFF.md assumes option A.

## Risk

Low. Help is **additive** everywhere — every screen still works unchanged if the link is removed. The build-step adds one mdbook dependency at build time (already required by the project to ship `docs/book/`).

---

<!-- BNB:CROSSREF-FOOTER -->
## Related

* Reuses O-17's `<dialog>` primitive for the drawer variant — different CSS class (`bnb-help-drawer`) so the docs read inline rather than centred.
* Pairs with O-19 (cmdk): `?` is a synonym for the help index — typing `?dawn` in the command palette jumps to the dawn-chorus help page.
<!-- BNB:CROSSREF-FOOTER -->
