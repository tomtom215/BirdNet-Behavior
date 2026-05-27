# O-12 · Empty states

<!-- BNB:STATUS-HEADER -->
> **Risk:** none · **Priority:** 1 · **Status:** ready for review
> Acceptance: [VERIFY.md § O-12](../VERIFY.md#o-12--empty-states) · Rollback: [ROLLBACK.md § O-12](../ROLLBACK.md#o-12--empty-states)
<!-- BNB:STATUS-HEADER -->


## What

Six hand-rolled SVG empty states matched to specific scenarios, drawn entirely with design-system tokens (no stock illustration, no external file). Each replaces a generic `<p class="bnb-meta">No detections yet…</p>` fallback.

## Files

| Action | Path |
|---|---|
| Add | `crates/birdnet-web/src/routes/pages/empty_states.rs` |
| Reference | `templates/_empty_states.html` — the same six fragments as `<template>` tags, useful if you'd rather inline-render than call a function |
| Patch | `crates/birdnet-web/src/routes/pages/mod.rs` — `pub(crate) mod empty_states;` |

## The six states

| Function | Triggers in | One-liner |
|---|---|---|
| `quiet_yard()` | dashboard live feed when no detections returned | *"A quiet yard."* |
| `no_species()` | `/species` partial when species count = 0 | *"No species heard yet."* (links to /system) |
| `no_chorus()` | `/analytics/dawn-chorus` & `/heatmap` when < 24h of data | *"The chorus hasn't started."* |
| `no_co_signal()` | `/correlation` when fewer than 2 species have ≥10 detections | *"Not enough overlap yet."* |
| `no_rare_yet()` | `/quarantine` when queue is empty | *"Nothing waiting for review."* |
| `no_life_list()` | `/life-list` when life list is empty | *"Your life list starts here."* |

## Where to wire them

Search the existing partials for the literal `"No detections yet"` / `"No species"` / `"No data"` strings and replace each with the matching helper. Approximate hit list (from spot-grepping the route tree):

- `pages/dashboard/partials.rs` — `quiet_yard()` for the live feed and most-recent card
- `pages/species_pages.rs` — `no_species()` 
- `pages/correlation.rs` — `no_co_signal()`
- `pages/quarantine.rs` — `no_rare_yet()`
- `pages/life_list.rs` — `no_life_list()`
- `pages/heatmap.rs` + `pages/timeseries_dash.rs` + the new dawn-chorus partial — `no_chorus()`

## Visual choices

- **Stays on-system.** Every SVG uses only the existing OKLCH tokens (`--moss`, `--moss-soft`, `--dawn`, `--rare`, `--fg-3`, `--hairline`, `--surface-2`). No new colors introduced.
- **Quiet, not cute.** The illustrations are abstract — a fading spectrogram, an empty list, an open journal — not cartoon birds. Matches the editorial tone of the rest of the app.
- **Always paired with one helpful sentence.** *Quiet yard* doesn't just say "no data"; it tells the user the mic is working. *No species* links to `/system` so they can diagnose. *No chorus* sets the right expectation (need 24h). This is the empty-state pattern that earns its keep.

## Risk

Zero. Pure additive helper module; existing string fallbacks keep working until each partial is migrated.

---

<!-- BNB:CROSSREF-FOOTER -->
## Related

* **Apply order:** shipped in the combined PR — see [HANDOFF.md](../HANDOFF.md#what-ships-in-this-pr) for the full file list.
* **Acceptance criteria:** [VERIFY.md § O-12](../VERIFY.md#o-12--empty-states).
* **Rollback:** [ROLLBACK.md § O-12](../ROLLBACK.md#o-12--empty-states).
* **Preview:** open [`INDEX.html`](../INDEX.html#O-12) for the rendered screen.
<!-- BNB:CROSSREF-FOOTER -->
