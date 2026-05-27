# BUILD_PR.md — Instructions for Claude Code

> **Purpose:** apply this bundle as a single PR against `tomtom215/BirdNet-Behavior` on a new branch.
> **Read first:** [HANDOFF.md](HANDOFF.md) for the high-level package overview.

---

## What you're doing

Applying ten design follow-on changes as one combined PR. Every change is documented in its own `O-NN/DIFF.md`; this file consolidates the apply steps so you don't need to walk all ten individually.

The bundle is **production-wired against the real workspace** — I read `state.rs`, `pages/mod.rs`, `atoms.rs`, `today.rs`, `species_pages.rs`, `recordings.rs`, `Cargo.toml`, and `routes/mod.rs` to match conventions exactly. The Rust files compile against the real `AppState`, use the real `super::atoms::*` helpers, the real `super::render_page()`, the real `state.with_db(|conn| ...)` accessor, and the real `state.image_cache()`. Nothing should need significant adjustment.

---

## Step 0 — environment check

```sh
# From the repo root of tomtom215/BirdNet-Behavior:
test -d crates/birdnet-web   # must exist
test -f Cargo.toml            # must exist
git status                    # must be clean
gh auth status                # must be authed
```

If any of those fail, stop and fix before continuing.

---

## Step 1 — bulk copy via `apply.sh`

```sh
bash design_handoff_birdnet_behavior/proposed_changes/apply.sh
```

This does the safe operations:
1. Creates branch `feat/design-followons` (override with `BNB_BRANCH=…`).
2. Copies every `O-NN/templates/`, `O-NN/src/`, and `O-NN/static/` into the matching target paths under `crates/birdnet-web/`.
3. Appends `O-NN/css/*.append` files to `crates/birdnet-web/static/css/app.css` between idempotent marker comments.
4. Runs `cargo add hmac@0.12 sha2@0.10 base64@0.22 --no-default-features --features std` for O-07.

The script will print a numbered list of the remaining in-place edits. Apply each one below.

---

## Step 2 — register route modules

**File: `crates/birdnet-web/src/routes/pages/mod.rs`**

Near the top, add (after `pub mod today;` to keep alphabetical-ish):

```rust
pub mod migration;
pub mod dawn_chorus;
pub mod species_photo;
pub mod empty_states;
```

Also add right next to `pub mod today;`:

```rust
// (No new module — `today_phrase` is wired inside today.rs, see step 4.)
```

In the embedded-template const block (around the `pub(crate) const RECORDINGS_PAGE_HTML…` lines), add:

```rust
pub(crate) const MIGRATION_PAGE_HTML: &str =
    include_str!("../../../templates/migration.html");
pub(crate) const DAWN_CHORUS_PAGE_HTML: &str =
    include_str!("../../../templates/dawn_chorus.html");
pub(crate) const DETECTION_DETAIL_HTML: &str =
    include_str!("../../../templates/detection_detail.html");
```

In the `router()` function's `.merge()` chain, append:

```rust
        .merge(migration::router())
        .merge(dawn_chorus::router())
        .merge(species_photo::router())
```

`empty_states` is a helper-function module, not a router — no merge needed.

---

## Step 3 — register share + feeds at the API layer

**File: `crates/birdnet-web/src/routes/mod.rs`**

Near the top:

```rust
pub mod share;
pub mod feeds;
```

In `api_routes()`, after the existing `.merge(pages::router())` line:

```rust
        .merge(share::router())
        .merge(feeds::router())
```

These are intentionally outside the `nest("/api/v2", …)` blocks because they expose user-facing public URLs (`/r/{token}`, `/feeds/*.rss`, `/feeds/*.ics`).

---

## Step 4 — register the today-phrase partial

**File: `crates/birdnet-web/src/routes/pages/today.rs`**

Near the top (after existing imports):

```rust
mod today_phrase;
use today_phrase::today_phrase_partial;
```

In `router()`'s chain, add:

```rust
        .route("/pages/today-phrase", get(today_phrase_partial))
```

---

## Step 5 — feed rows link to detection detail

The detection-detail page (`O-05`) exists in production but isn't linked from anywhere. Make the time field on each feed row a link to it.

**File: `crates/birdnet-web/src/routes/pages/dashboard/partials.rs`**

Find the feed-row render (look for `class="ago"` or `class="ago mono"`). Change the time `<span>` to:

```rust
write!(html, r#"<a class="ago mono" href="/detection/{id}" style="color:inherit;text-decoration:none;" title="Open detection detail">{time_short}</a>"#, id = d.id)?;
```

(Adjust `d.id` to whatever field carries the integer rowid in the existing `DetectionRow` struct. If it isn't selected today, add `d.rowid` to the query's `SELECT` clause first.)

**File: `crates/birdnet-web/src/routes/pages/today.rs`**

Same change inside `render_detection_card()` — the inline detection-detail link already exists; the new patch is to also wrap the time field. Look for the existing `<a href="/detections/detail?date=…">` line and add an equivalent `/detection/{rowid}` wrapper on the time `<span>`. Mirror the dashboard treatment so both pages behave the same.

---

## Step 6 — layout template patches

**File: `crates/birdnet-web/templates/layout.html`**

### 6a. Print stylesheet

After the existing `<link rel="stylesheet" href="/static/css/app.css">`:

```html
<link rel="stylesheet" href="/static/css/print.css" media="print">
```

### 6b. Feed discovery

In `<head>` (anywhere):

```html
<link rel="alternate" type="application/rss+xml"
      title="BirdNet · rare detections"
      href="/feeds/rare.rss">
```

### 6c. Migration nav link

In the topnav's analytics group (look for the existing `<a href="/heatmap" class="topnav-link {{nav_heatmap}}">…`), add:

```html
<a href="/migration" class="topnav-link {{nav_migration}}">Migration</a>
```

Update `pages/mod.rs::render_page()` to handle `{{nav_migration}}` — add `.replace("{{nav_migration}}", nav("migration"))` alongside the other nav substitutions.

### 6d. FOUC guard for new preference keys

Find the existing `<script>` near the top of `<head>` that reads `theme` and `bnb-density` from localStorage. Extend it to also handle the two new keys from O-03:

```html
<script>(function(){
  var t=localStorage.getItem('theme');
  if(t!=='light'&&t!=='dark'){
    t=(t==='auto'||!t)?(window.matchMedia('(prefers-color-scheme:dark)').matches?'dark':'light'):t;
  }
  document.documentElement.setAttribute('data-theme',t);
  var d=localStorage.getItem('bnb-density');
  if(d==='compact'||d==='comfy'||d==='regular')
    document.documentElement.style.setProperty('--density',d==='compact'?'0.78':d==='comfy'?'1.15':'1');
  var m=localStorage.getItem('bnb-motion');
  if(m==='reduced') document.documentElement.setAttribute('data-motion','reduced');
  var c=localStorage.getItem('bnb-contrast');
  if(c==='high') document.documentElement.setAttribute('data-contrast','high');
})();</script>
```

---

## Step 7 — detection-detail render wiring

**File: `crates/birdnet-web/src/routes/pages/detection_detail.rs`**

The module exists in production but only ships partial handlers — no full-page route. Add a page handler at the end of the file:

```rust
const TEMPLATE: &str = super::DETECTION_DETAIL_HTML;

pub async fn detection_detail_page(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Html<String> {
    let detection = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            conn.query_row(
                "SELECT Com_Name, Sci_Name, Date, Time, Confidence, File_Name \
                 FROM detections WHERE rowid = ?1",
                [id],
                |r| Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, f64>(4)?,
                    r.get::<_, Option<String>>(5)?,
                )),
            ).ok()
        })
    })
    .await
    .ok()
    .flatten();

    let Some((com, sci, date, time, conf, file)) = detection else {
        return super::render_page(
            "Detection not found",
            "<p>That detection doesn't exist.</p>",
            "",
        );
    };

    let conf_class = if conf >= 0.85 { "high" } else if conf >= 0.60 { "mid" } else { "low" };
    let conf_pill  = match conf_class { "high" => "moss", "mid" => "dawn", _ => "rare" };
    let body = TEMPLATE
        .replace("{{detection_id}}",    &id.to_string())
        .replace("{{species_name}}",    &super::escape_html(&com))
        .replace("{{scientific_name}}", &super::escape_html(&sci))
        .replace("{{species_encoded}}", &super::simple_url_encode(&com))
        .replace("{{date}}",            &super::escape_html(&date))
        .replace("{{time}}",            &super::escape_html(&time))
        .replace("{{confidence}}",      &format!("{conf:.3}"))
        .replace("{{confidence_pct}}",  &format!("{:.0}%", conf * 100.0))
        .replace("{{conf_class}}",      conf_class)
        .replace("{{conf_class_pill}}", conf_pill)
        .replace("{{audio_filename}}",  &super::escape_html(&file.unwrap_or_default()))
        .replace("{{spectrogram_url}}", &format!("/api/v2/spectrogram/{id}"))
        .replace("{{ago_phrase}}",      &format!("{date} {time}"));
    super::render_page(&format!("Detection #{id}"), &body, "")
}
```

Add `.route("/detection/{id}", get(detection_detail_page))` to the module's existing `router()`.

---

## Step 8 — optional schema

If you want the triage buttons on detection detail to persist (Confirm / Quarantine / Mistake), add this to your sqlite migrations:

```sql
CREATE TABLE IF NOT EXISTS detection_reviews (
  detection_id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL CHECK (kind IN ('confirm','quarantine','mistake')),
  reviewer TEXT,
  reviewed_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  note TEXT
);
CREATE INDEX IF NOT EXISTS idx_reviews_kind ON detection_reviews(kind, reviewed_at);
```

Back-compatible additive change; safe to skip if you'd rather ship triage in a follow-up.

---

## Step 9 — environment variables

For deployment, set:

```sh
export BNB_SHARE_SECRET=$(openssl rand -hex 32)        # O-07 — token signing
export BNB_BASE_URL=https://birdnet.example.com        # O-11 — feed URLs
export BNB_STATION_LAT=42.36                            # O-02 — sun-times
export BNB_STATION_LON=-71.06                           # O-02 — negative = west
```

Without `BNB_SHARE_SECRET`, share-link tokens invalidate on every server restart (intentional — fail-secure default). The others have sensible fallbacks.

---

## Step 10 — verify locally

```sh
cd crates/birdnet-web
cargo check                                # compiles
cargo clippy --all-targets -- -D warnings  # workspace lints are pedantic; expect some
cargo test                                 # unit tests for share/feeds/migration/today_phrase
```

Expected output:
* `cargo check` clean.
* `cargo clippy` may warn on some pedantic lints in the new files; they're acceptable but a `--no-deps` run is fine.
* `cargo test` should pass ~15 new unit tests across the new modules.

If anything fails, fix in place (the patterns are 1:1 with existing code — usually a missing `use super::…`).

---

## Step 11 — commit and PR

```sh
git add -A
git commit -m "Design follow-on PR set: 10 changes (O-01 through O-12)

- O-04: Species detail rebuild on the design system
- O-09: Today comparative phrasing partial
- O-12: Empty states library
- O-08: Print stylesheet (weekly/year-in-review)
- O-03: Display preferences (theme · density · motion · contrast)
- O-05: Detection detail surfacing + share/copy + reference photo
- O-01: /migration phenology ridgeline
- O-02: /analytics/dawn-chorus polar ribbons
- O-07: /r/<token> shareable rare-bird permalinks (HMAC-signed)
- O-11: iCal + RSS public feeds

Zero new crate dependencies beyond hmac/sha2/base64 (for token signing).
One optional schema migration (detection_reviews); back-compatible.
One URL rename: /admin/migration -> /admin/import (with 301 redirect).

Closes: design follow-on milestone."
git push -u origin feat/design-followons
```

Then:

```sh
gh pr create \
  --title "Design follow-on PR set: 10 changes (species detail, migration, dawn chorus, share, feeds, …)" \
  --body-file design_handoff_birdnet_behavior/proposed_changes/HANDOFF.md \
  --base main \
  --head feat/design-followons \
  --draft
```

Open as a draft for one cycle of self-review, then flip to ready.

---

## Step 12 — post-merge

Run the [VERIFY.md](VERIFY.md) checks on a live device (or staging). If anything regresses, [ROLLBACK.md](ROLLBACK.md) has the per-change back-out recipes.

---

## If you hit something unexpected

* **`cargo check` fails on a missing `use`:** these files were written against the read-time snapshot of the workspace. If the workspace has moved (new module reorganization, renamed `AppState` accessor, etc.), the imports may need adjustment. Look for the existing pattern in `species_pages.rs` and copy it.
* **A route collides with an existing one:** the most likely culprit is `/admin/migration` (kept the URL collision intentional — see HANDOFF.md). Rename it or add a 301 redirect from old → new.
* **`detection_reviews` table doesn't exist and triage buttons 500:** apply step 8's schema migration, or comment out the `Confirm`/`Quarantine`/`Mistake` buttons in `detection_detail.html` for the initial ship.
* **CSS append duplicates on re-runs:** `apply.sh` uses idempotent `BNB:CSS-APPEND:O-NN` marker comments, so re-running is safe.

---

## What this PR does *not* change

* No existing routes have their behavior altered (only feed-row link addition + Migration nav link).
* No existing templates are rewritten — only `species_detail.html` and `today.html` are *replaced* (full content swap, but no breaking schema changes to the partials they call).
* No existing API endpoints are touched.
* No version bump.
* No new crate dependencies beyond the three for HMAC token signing.

Net surface: small. Net value: ten meaningful design upgrades shipped at once.
