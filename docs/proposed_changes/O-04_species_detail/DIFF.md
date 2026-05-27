# O-04 · Species detail — full rebuild

<!-- BNB:STATUS-HEADER -->
> **Risk:** none · **Priority:** 1 · **Status:** ready for review
> Acceptance: [VERIFY.md § O-04](../VERIFY.md#o-04--species-detail-rebuild) · Rollback: [ROLLBACK.md § O-04](../ROLLBACK.md#o-04--species-detail-rebuild)
<!-- BNB:STATUS-HEADER -->


## What

Replaces `crates/birdnet-web/templates/species_detail.html` with a layout that uses the design system instead of legacy bridge classes (`.card`/`.stat-card`), surfaces the best recording at the top, and adds cross-links to heatmap & co-occurrence.

## Files

| Old path | New content | Action |
|---|---|---|
| `crates/birdnet-web/templates/species_detail.html` | `templates/species_detail.html` (in this folder) | **Replace** |

No Rust changes required for the existing experience to work. **Two optional new partials** unlock the photo+scrubber hero rail and the residency pills — endpoints listed below.

## Why

The current page falls back to `<h1 class="display">` + a 2-card stat block + 4 stacked legacy `.card` panels in a single column. Mockup intent was a hero with photo + scrubber, residency metadata pills, two-column body, and explicit deep links to other analytics surfaces (heatmap / co-occurrence). This is one of the most-visited screens — visual upgrade has the biggest ROI in the report.

## New partial endpoints (optional, with graceful fallback)

Both endpoints already have placeholders in the template — the page renders cleanly without them, falling back to `bnb-photo` placeholder and a loading pill.

### `GET /pages/species-hero?name=…`

Return HTML for the right-side rail:

```html
<div class="bnb-eyebrow" style="margin-bottom:8px;">Best detection</div>
<div class="bnb-photo"
     data-caption="2026-03-12 06:14 · 0.97 conf"
     style="aspect-ratio:4/3;border-radius:var(--r-md);">
  <!-- optional <img src="…"> overlay, sourced from species_images table -->
</div>
<audio controls preload="metadata"
       src="/api/v2/recordings/2026-03-12-06-14-12_Cyanocitta_cristata.wav"
       style="width:100%;margin-top:10px;height:32px;"></audio>
<div class="bnb-meta mono" style="margin-top:6px;">peak 4.2 kHz · clip 3.0 s</div>
```

Suggested Rust signature (drop into `pages/species_pages.rs`):

```rust
async fn species_hero_partial(
    State(state): State<AppState>,
    Query(q): Query<NameQuery>,
) -> impl IntoResponse {
    // 1. Pick the highest-confidence detection for q.name in the last 30 days.
    // 2. Look up species_images.local_path (existing images.rs module).
    // 3. Render with the snippet above; missing image → keep .bnb-photo placeholder.
}
```

### `GET /pages/species-status?name=…`

Return a row of pills for the hero:

```html
<span class="bnb-pill moss"><span class="bnb-dot"></span> Resident</span>
<span class="bnb-pill">First heard 2025-04-12</span>
<span class="bnb-pill">347 days at this station</span>
<span class="bnb-pill dawn"><span class="bnb-dot dawn"></span> Peak hour 06:00</span>
```

Residency classification (Resident / Regular / Migrant / Rarity) already exists in `birdnet_behavioral::types::ResidencyType` — wired in `behavioral.rs`. Reuse it.

## Tweaks worth doing alongside

- Pass `?focus=<species>` through `/heatmap` and `/correlation` so the cross-link actually highlights the species on arrival. Both pages already accept query params from `pages/heatmap.rs` and `pages/correlation.rs`; small parser change.
- Add a `data-screen-label="Species · {{species_name}}"` attribute on the outer wrapper if you want consistent comment anchoring across screens.

## Risk

Zero — old page reads `.card` etc which are still defined as bridge classes in `app.css`. New page reads the canonical atoms (`.bnb-card`, `.stat-tile`, `.bnb-eyebrow`). Existing endpoints' return payloads are unchanged.

---

<!-- BNB:CROSSREF-FOOTER -->
## Related

* **Apply order:** shipped in the combined PR — see [HANDOFF.md](../HANDOFF.md#what-ships-in-this-pr) for the full file list.
* **Acceptance criteria:** [VERIFY.md § O-04](../VERIFY.md#o-04--species-detail-rebuild).
* **Rollback:** [ROLLBACK.md § O-04](../ROLLBACK.md#o-04--species-detail-rebuild).
* **Preview:** open [`INDEX.html`](../INDEX.html#O-04) for the rendered screen.
<!-- BNB:CROSSREF-FOOTER -->
