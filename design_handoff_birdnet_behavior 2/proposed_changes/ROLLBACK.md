# ROLLBACK — Per-PR back-out plan

Every PR in this package is independently revertable. None depend on another; all dependencies on existing code are additive (new endpoints, new files) except where explicitly noted.

For each PR below:

* **`git revert`** — the safe default. Reverts the commit; old behavior returns.
* **Manual back-out** — if you've cherry-picked files instead of using a single commit per PR. Lists every file to delete and patch to undo.
* **Data** — what (if any) data the PR persists and whether reverting orphans it.
* **External callers** — what URLs / feeds you may have published that will start 404-ing after revert.

---

## O-04 · Species detail rebuild

**git revert**: one commit.

**Manual back-out**:

```sh
git checkout HEAD~1 -- crates/birdnet-web/templates/species_detail.html
```

(restores the pre-redesign template — uses bridge classes so it'll still render.)

**Data**: none persisted.

**External callers**: none. The URL `/species/detail?name=…` is unchanged.

---

## O-09 · Today · comparative phrase

**git revert**: one commit.

**Manual back-out**:

```sh
git checkout HEAD~1 -- crates/birdnet-web/templates/today.html
rm crates/birdnet-web/src/routes/pages/today_phrase.rs
# in pages/today.rs::router(): remove the `.route("/pages/today-phrase", …)` line
# in pages/today.rs: remove `mod today_phrase;`
```

**Data**: none.

**External callers**: `GET /pages/today-phrase` becomes a 404. No public clients should be calling this — it's htmx-only inside the page.

---

## O-12 · Empty states

**git revert**: one commit.

**Manual back-out**:

```sh
rm crates/birdnet-web/src/routes/pages/empty_states.rs
# in pages/mod.rs: remove `pub(crate) mod empty_states;`
# undo any call sites that switched from string fallbacks to empty_states::*()
```

The string fallbacks (`"No detections yet"`) still work in the old code — there's no requirement to leave the helper in place.

**Data**: none.

**External callers**: none.

---

## O-08 · Print stylesheet

**git revert**: one commit.

**Manual back-out**:

```sh
rm crates/birdnet-web/static/css/print.css
# in layout.html: remove the <link rel="stylesheet" href="/static/css/print.css" media="print">
```

**Data**: none.

**External callers**: none. Users who had bookmarked a PDF copy of `/weekly` still have their PDF; printing reverts to browser default.

---

## O-03 · Display preferences

**git revert**: one commit.

**Manual back-out**:

```sh
rm crates/birdnet-web/templates/_partial_display_prefs.html
# in static/css/app.css: remove everything from the `O-03 · BEGIN` marker
#   to the `O-03 · END` marker (or undo the appended block if you didn't use markers).
# in layout.html: revert the FOUC guard back to handling only `theme` and `bnb-density`.
```

**Data**: user's localStorage keys `bnb-motion` and `bnb-contrast` become orphaned. They're harmless (~20 bytes each); ignore.

**External callers**: none.

---

## O-05 · Detection detail page

**git revert**: one commit.

**Manual back-out**:

```sh
rm crates/birdnet-web/templates/detection_detail.html
# in pages/detection_detail.rs: revert the render_page wiring + new partials
# in pages/dashboard/partials.rs: revert the feed-row time → <a href="/detection/{id}"> patch
# in pages/today.rs: revert the same patch
```

**Data**:

* If you ran the optional `detection_reviews` table migration, **leave the table in place** even after revert. Re-applying the PR will reuse existing rows; dropping the table loses any human triage already done. Schema:
  ```sql
  -- safe to leave indefinitely; back-compatible with old code
  CREATE TABLE detection_reviews (
    detection_id INTEGER PRIMARY KEY, kind TEXT, reviewer TEXT, reviewed_at TIMESTAMP, note TEXT
  );
  ```

**External callers**: `/detection/<id>` 404s after revert. Anyone who shared a permalink (e.g. via O-07) gets a 404 there too — but the share-page route (`/r/<token>`) keeps working since it has its own renderer.

---

## O-01 · `/migration` ridgeline

**git revert**: one commit, **plus the URL rename**.

**Manual back-out**:

```sh
rm crates/birdnet-web/templates/migration.html
rm crates/birdnet-web/src/routes/pages/migration.rs
# in pages/mod.rs: remove `pub mod migration;` and the `.merge(migration::router())`
# in layout.html: remove the `/migration` nav link
```

**URL rename rollback**: if you renamed `/admin/migration` → `/admin/import` as part of this PR, revert that too. Add a 301 redirect in the opposite direction to soften the round-trip for any bookmarks made during the brief renamed window.

**Data**: none.

**External callers**: none — `/migration` was a new URL.

---

## O-02 · `/analytics/dawn-chorus` polar ribbons

**git revert**: one commit.

**Manual back-out**:

```sh
rm crates/birdnet-web/templates/dawn_chorus.html
rm crates/birdnet-web/src/routes/pages/dawn_chorus.rs
# in pages/mod.rs: remove `pub mod dawn_chorus;` and the `.merge(dawn_chorus::router())`
# in /analytics page header: remove the link to /analytics/dawn-chorus if you added it
```

**Data**: none.

**External callers**: none — new URL.

---

## O-07 · Rare-bird permalinks

**git revert**: one commit.

**Manual back-out**:

```sh
rm crates/birdnet-web/templates/share_rare.html
rm crates/birdnet-web/src/routes/share.rs
# in routes/mod.rs: remove `pub mod share;` and the `.merge(share::router())`
# in quarantine + detection_detail templates: remove the "Share clip" button if surfaced
# unset BNB_SHARE_SECRET in the deployment environment
```

**Data**: none persisted server-side (tokens are HMAC-encoded; no storage).

**External callers**: **all previously-shared URLs start 404-ing.** If you've broadcast any share links externally, consider:

1. Leaving the `share.rs` route module in place but disabling new token issuance.
2. Or pre-revert, generate a static HTML archive of each previously-shared detection and serve from `/r-archive/<id>.html`.

---

## O-11 · iCal + RSS feeds

**git revert**: one commit.

**Manual back-out**:

```sh
rm crates/birdnet-web/src/routes/feeds.rs
# in routes/mod.rs: remove `pub mod feeds;` and the `.merge(feeds::router())`
# in layout.html: remove the <link rel="alternate" type="application/rss+xml" ...>
# unset BNB_BASE_URL if you set it just for the feeds
```

**Data**: none.

**External callers**: **anyone subscribed in a reader/calendar will see stale entries fail to refresh.** Most readers cache the last successful fetch indefinitely on 404 (silent fail). Calendar clients (Apple, Google) typically show a "could not refresh" badge.

If you must keep external clients happy through the revert window, leave the routes returning a minimal `<rss><channel></channel></rss>` envelope rather than 404 — graceful empty.

---

## Full-package rollback

If you need to back out the entire package in one shot:

```sh
git revert <range>   # 10 commits, one per PR
```

Order of revert doesn't matter; the PRs are independent. After revert, run:

```sh
cargo check -p birdnet-web
cargo test -p birdnet-web
```

Both should pass cleanly against the pre-merge baseline.

---

## Communications template

If you have to roll back a public-facing PR (O-07 or O-11), a one-line notice helps:

> Heads-up — we temporarily reverted the rare-bird share links. Existing links will resume working in `~N` days. Your subscriptions are preserved.
