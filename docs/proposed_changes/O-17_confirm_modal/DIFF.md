# O-17 · Themed confirmation modal

<!-- BNB:STATUS-HEADER -->
> **Risk:** none · **Priority:** 2 · **Status:** ready for review
> Acceptance: VERIFY.md § O-17 · Rollback: ROLLBACK.md § O-17
<!-- BNB:STATUS-HEADER -->


## What

Every destructive admin action today routes through `hx-confirm="…"`, which triggers the **browser's native** confirmation dialog. That's a visual cliff — wrong typeface, wrong palette, wrong tone, and on the danger-zone card in `backup_recovery` it makes the most consequential actions in the app look like an alert from a 2003 webmail client.

This change adds a single themed `<dialog>`-based modal partial mounted in `layout.html`. Any button that needs confirmation gains four attributes (`data-confirm-title`, `data-confirm-body`, `data-confirm-action`, `data-confirm-style`); a tiny JS shim intercepts the click, fills the modal, and on confirmation re-issues the original `hx-*` request via `htmx.trigger`.

No new dependencies. `<dialog>` is supported in every browser the project already targets (Chrome 110+, Safari 16.4+, Firefox 113+ — same set required by OKLCH).

## Files

| Action | Path |
|---|---|
| Add | `crates/birdnet-web/templates/_partial_confirm_modal.html` |
| Add | `crates/birdnet-web/src/routes/pages/confirm.rs` — `confirm_button(...)` helper that emits the trigger button HTML |
| Append | `crates/birdnet-web/static/css/app.css` — see `css/app.css.append` |
| Patch | `crates/birdnet-web/templates/layout.html` — one `{{include}}` before `</body>` (or hard-inline the partial) |
| Patch | `crates/birdnet-web/src/routes/pages/mod.rs` — `pub(crate) mod confirm;` |

## How a destructive button looks before/after

Before (uses native dialog):
```html
<button class="btn btn-danger"
        hx-post="/admin/system/factory-reset"
        hx-confirm="This wipes everything. Are you sure?">
  Factory reset
</button>
```

After (uses themed modal):
```html
<button class="bnb-btn danger"
        data-confirm-action="hx-post"
        data-confirm-url="/admin/system/factory-reset"
        data-confirm-title="Factory reset"
        data-confirm-body="Wipes every detection, recording, snapshot and setting on this station. This cannot be undone. The station name and Wikipedia photo cache are preserved."
        data-confirm-confirm-label="Wipe everything"
        data-confirm-style="danger">
  Factory reset
</button>
```

Or from Rust, using the helper:
```rust
use crate::routes::pages::confirm;
let html = confirm::confirm_button(confirm::Confirm {
    label: "Factory reset",
    action: confirm::Action::Post("/admin/system/factory-reset"),
    title: "Factory reset",
    body: "Wipes every detection, recording, snapshot and setting on this station. \
           This cannot be undone. The station name and Wikipedia photo cache are preserved.",
    confirm_label: "Wipe everything",
    style: confirm::Style::Danger,
});
```

## Migration: every existing `hx-confirm` site

Grep target: `hx-confirm="`. The currently-shipped admin code uses native confirm at these sites (read from main on the date of this drift report). Each gets a `data-confirm-*` rewrite plus a tone-appropriate copy lift.

| File | Existing prompt | New tone |
|---|---|---|
| `admin/backup_recovery.rs` — reset settings | "Reset all settings to defaults?" | "**Reset settings.** Restores every configurable setting to its installed default. Detections, recordings, snapshots and the species list are preserved." |
| `admin/backup_recovery.rs` — wipe recordings | "Delete all recordings?" | "**Wipe recordings.** Deletes every audio clip on disk. Detection rows in the database are preserved — only the audio files are removed. Frees ~{used} of storage." |
| `admin/backup_recovery.rs` — factory reset | "Factory reset?" | (see example above) |
| `admin/backup_recovery.rs` — uninstall | "Uninstall?" | "**Uninstall.** Stops the service and removes the binary, configuration and database. **The recordings folder is left in place** so you can rescue clips before deleting it manually." |
| `admin/system_controls/data.rs` — clear data | inline confirm | same as wipe recordings, plus DB rows |
| `pages/quarantine.rs` — reject detection | "Reject this detection?" | "**Reject this detection.** Removes it from the database and the recording from disk. Used to train the false-positive list." |
| `pages/species_pages.rs` — delete species row | (none today — guard once added) | n/a |
| `admin/notifications.rs` — remove channel | (none) | n/a |
| `admin/rules.rs` — delete rule | "Delete this alert rule?" | "**Delete this rule.** New detections matching it will no longer trigger alerts. Past alerts in the notification log are kept." |
| `admin/system/logs.rs` — prune logs | "Prune old log entries?" | "**Prune log entries.** Removes log rows older than {days} days. The current session's logs are kept regardless of age." |

After migration, the original `hx-confirm` attribute can be deleted from each call site. The Rust handler doesn't change at all — the same endpoint receives the same request, just triggered from JS instead of the native confirm path.

## Behavior

- The modal uses native `<dialog>`. Opening calls `dialog.showModal()`, which:
  - traps focus inside the dialog,
  - dims the rest of the page via `::backdrop`,
  - blocks Esc / outside-click dismissal unless `data-confirm-dismissable="true"`.
- The primary action button receives the same `hx-*` attributes the trigger had — so the resulting request goes to the same endpoint, includes the same hidden fields, and targets the same swap location.
- `data-confirm-style="danger"` paints the primary button rare-red; default is `moss`. A third option (`data-confirm-style="warn"`) uses dawn-amber for things like "Restart now to apply" that aren't destructive but need attention.
- A "Don't ask for {n} minutes" checkbox is **deliberately not added** — every site that uses confirm in this codebase is high-consequence (factory reset, wipe recordings, prune logs). The existing pattern of explicit re-confirmation per action is correct.

## Risk

Zero. The fallback path (no JS, ancient browser without `<dialog>`) falls through to the browser's native `confirm()` because the helper sets `hx-confirm` alongside `data-confirm-*` — htmx still gates the request, just without our nicer chrome.

---

<!-- BNB:CROSSREF-FOOTER -->
## Related

* Pairs with O-18 (toast) — successful action → toast. Cancelled action → no toast.
* `pages/skeletons` (O-16) ships first; this opportunity reuses the same shimmer keyframe? No — modals don't shimmer. Listed only because both are layout primitives.
<!-- BNB:CROSSREF-FOOTER -->
