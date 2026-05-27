# O-05 · Detection detail — surface the existing page + wire feed links

<!-- BNB:STATUS-HEADER -->
> **Risk:** low · **Priority:** 2 · **Status:** ready for review
> Acceptance: [VERIFY.md § O-05](../VERIFY.md#o-05--detection-detail-page) · Rollback: [ROLLBACK.md § O-05](../ROLLBACK.md#o-05--detection-detail-page)
<!-- BNB:STATUS-HEADER -->

## What

`crates/birdnet-web/src/routes/pages/detection_detail.rs` is **13 KB of working Rust** but the page doesn't appear in any nav, and feed rows in the dashboard / today page link to *species detail*, not the individual detection. This PR:

1. Adds a proper `detection_detail.html` template (the existing module has partials, but no full-page chrome to host them).
2. Adds a `data-share="link"` permalink-copy button.
3. Patches the dashboard feed row + today list rows so the **time/id field becomes a link** to `/detection/{id}`.
4. Adds a **reference-photo card** between the hero block and the spectrogram (matches the share-page treatment), wired to the workspace's existing `ImageCache` via a new `/pages/species-photo` partial.

## Files

| Action | Path |
|---|---|
| Add | `crates/birdnet-web/templates/detection_detail.html` |
| Add | `crates/birdnet-web/src/routes/pages/species_photo.rs` |
| Patch | `crates/birdnet-web/src/routes/pages/mod.rs` — `pub mod species_photo;` and `.merge(species_photo::router())` |
| Patch | `crates/birdnet-web/src/routes/pages/detection_detail.rs` — `render_page("Detection #{id}", &body, "")` where `body` is the template with placeholders substituted |
| Patch | `crates/birdnet-web/src/routes/pages/dashboard/partials.rs` — feed row time becomes a link |
| Patch | `crates/birdnet-web/src/routes/pages/today.rs` — list row time becomes a link |

## Render call (in `detection_detail.rs`)

```rust
const TEMPLATE: &str = include_str!("../../../templates/detection_detail.html");

fn substitute(det: &DetectionRow, conf_class: &str, ago: &str) -> String {
    TEMPLATE
        .replace("{{detection_id}}",     &det.id.to_string())
        .replace("{{species_name}}",     &escape_html(&det.com_name))
        .replace("{{scientific_name}}",  &escape_html(&det.sci_name))
        .replace("{{species_encoded}}",  &simple_url_encode(&det.com_name))
        .replace("{{date}}",             &escape_html(&det.date))
        .replace("{{time}}",             &escape_html(&det.time))
        .replace("{{confidence}}",       &format!("{:.3}", det.confidence))
        .replace("{{confidence_pct}}",   &format!("{:.0}%", det.confidence * 100.0))
        .replace("{{conf_class}}",       conf_class)
        .replace("{{conf_class_pill}}",  match conf_class { "high" => "moss", "mid" => "dawn", _ => "rare" })
        .replace("{{audio_filename}}",   &escape_html(&det.file_name.clone().unwrap_or_default()))
        .replace("{{spectrogram_url}}",  &format!("/api/v2/spectrogram/{}", det.id))
        .replace("{{ago_phrase}}",       ago)
}
```

## Feed-row link patch

In `dashboard/partials.rs::render_feed_row`, change the `<span class="ago mono">` to:

```rust
write!(
    html,
    r#"<a class="ago mono" href="/detection/{id}" style="color:inherit;text-decoration:none;">{time_short}</a>"#,
    id = d.id,
)?;
```

Add `id: i64` to the `DetectionRow` query if it isn't already selected (it almost certainly is).

Identical change applies to `today.rs` list rows. ~6 lines of code per file.

## Reference photo card — production path

The template's hero now includes a 16:9 photo card directly above the spectrogram, identical in treatment to the public share page. The card is wired to a new partial that reads from the workspace's existing **`ImageCache`** (`birdnet_integrations::species_images::ImageCache`, already held by `AppState::image_cache()` — the same cache `species_info_partial` already uses to embed images on the species-detail page).

### `GET /pages/species-photo?name=<common>[&caption=…&attribution=…]`

`crates/birdnet-web/src/routes/pages/species_photo.rs` ships ready-to-merge:

* Looks up the scientific name from the `detections` table (same single-row pattern as `species_info_partial`).
* Calls `state.image_cache().get_cached(&sci_name)`.
* If the cache has a downloaded image (`cached_path.is_some()`), returns an `<img src="/api/v2/species/image/<sci>/file" loading="lazy" decoding="async">` — that endpoint is already in production.
* Otherwise returns `204 No Content` so the surrounding `.bnb-photo` hatched-pattern placeholder remains visible.
* Caches the response `public, max-age=3600` since the underlying photo rarely changes.

The same partial is used by the share page (O-07). Its placeholder design is the production-canonical `.bnb-photo` diagonal stripe pattern (defined in `app.css`) with a small "photograph pending" centerpiece — never tries to be a generated bird silhouette.

## New partials referenced

The detail page calls a handful of partials. Most exist; the ones below are new and small enough to be co-located in `detection_detail.rs`:

| Endpoint | Purpose | Rough body |
|---|---|---|
| `GET  /pages/species-photo?name=…` | Reference photo (new — see above) | `state.image_cache().get_cached(sci)` |
| `GET  /pages/detection-tags?id=…` | One-pill row: "first today" / "rare" / "confirmed" / etc. | `SELECT … FROM detections WHERE id = ?` |
| `GET  /pages/detection-status?id=…` | "Pending review" / "Confirmed by you" / etc. | reads `detection_reviews` |
| `POST /pages/detection-confirm?id=…` | Marks confirmed; returns the same status pill | insert into `detection_reviews(id, kind='confirm')` |
| `POST /pages/detection-flag?id=…&kind=…` | Marks quarantine / mistake | insert into `detection_reviews(id, kind=?)` |
| `GET  /pages/detection-context?id=…` | Mini live-feed view: this detection ± 5 minutes | window query on `detections` |
| `GET  /pages/detection-spectrogram?id=…` | `<img src="/api/v2/spectrogram/<id>">` wrapped in a placeholder until ready | uses existing `spectrogram` module |
| `GET  /pages/detection-model-info?id=…` | `<tr><td>Model</td><td>BirdNET V2.4</td></tr>` | reads model version from settings table |

**`detection_reviews` table** — if you don't already have one, this is the right time to add it:

```sql
CREATE TABLE IF NOT EXISTS detection_reviews (
  detection_id INTEGER PRIMARY KEY REFERENCES detections(rowid),
  kind TEXT NOT NULL CHECK (kind IN ('confirm','quarantine','mistake')),
  reviewer TEXT,
  reviewed_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  note TEXT
);
CREATE INDEX IF NOT EXISTS idx_reviews_kind ON detection_reviews(kind, reviewed_at);
```

Backwards-compatible: pre-existing detections without a row are treated as "pending".

## Why this page matters

Without it, every feed row dead-ends at species detail. A user who hears something interesting can't:
- Listen to *that exact* clip without going to recordings and scrolling
- Share it with a friend (no permalink)
- Mark it as a mistake or send it to quarantine
- See what else was happening that minute

This is the page that turns the dashboard from a counter into a logbook.

## Risk

Low. New page; only nudges existing partials to add an `id` link. The optional `detection_reviews` table is additive. The reference-photo card degrades gracefully to the canonical `.bnb-photo` placeholder when the image cache hasn't fetched the species yet.

---

<!-- BNB:CROSSREF-FOOTER -->
## Related

* **Apply order:** shipped in the combined PR — see [HANDOFF.md](../HANDOFF.md#what-ships-in-this-pr) for the full file list.
* **Acceptance criteria:** [VERIFY.md § O-05](../VERIFY.md#o-05--detection-detail-page).
* **Rollback:** [ROLLBACK.md § O-05](../ROLLBACK.md#o-05--detection-detail-page).
* **Preview:** open [`INDEX.html`](../INDEX.html#O-05) for the rendered screen.
<!-- BNB:CROSSREF-FOOTER -->
