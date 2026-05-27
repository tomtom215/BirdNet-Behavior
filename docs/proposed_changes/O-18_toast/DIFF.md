# O-18 · Toast / snackbar region

<!-- BNB:STATUS-HEADER -->
> **Risk:** none · **Priority:** 2 · **Status:** ready for review
> Acceptance: VERIFY.md § O-18 · Rollback: ROLLBACK.md § O-18
<!-- BNB:STATUS-HEADER -->


## What

When a user clicks **Save settings**, **Trigger backup**, **Test channel**, **Approve detection**, or any other admin action, today the feedback is whatever the partial swap renders inline. That's fine for actions that visibly modify the swap target — but **Test channel** swaps nothing, **Save settings** swaps a small form fragment, and **Trigger backup** redirects, so the user is left wondering whether anything happened.

This change introduces a single global toast region (top-right, theme-aware) that receives one-off success / warning / error messages via **htmx OOB swaps**. The handler returns its normal response plus an extra `<div id="bnb-toasts" hx-swap-oob="beforeend">…</div>` fragment, and htmx prepends the toast into the live region. The toast auto-dismisses after a typed-default delay (success 4s, warn 6s, error sticky), with a manual close button.

No new dependencies; htmx's OOB mechanism is already in the binary.

## Files

| Action | Path |
|---|---|
| Add | `crates/birdnet-web/templates/_partial_toast_region.html` — the live region + the auto-dismiss JS |
| Add | `crates/birdnet-web/src/routes/pages/toast.rs` — `toast::success("Saved.")` / `toast::warn(...)` / `toast::error(...)` helpers + an `IntoResponse` extension to attach a toast to any response |
| Append | `crates/birdnet-web/static/css/app.css` — see `css/app.css.append` |
| Patch | `crates/birdnet-web/templates/layout.html` — one line: `{{include _partial_toast_region.html}}` before `</body>` (or inline-paste) |
| Patch | `crates/birdnet-web/src/routes/pages/mod.rs` — `pub(crate) mod toast;` |
| Patch | `crates/birdnet-web/templates/share_rare.html` — same toast region (public pages don't need full layout but they do need confirmation when "Copy permalink" succeeds; the existing inline JS can be replaced with `toast::success` calls dispatched from a tiny `birdnet:toast` CustomEvent) |

## Wiring on the server

```rust
use crate::routes::pages::toast;

// 1. Plain string helper — returns just the OOB toast HTML.
let oob = toast::success("Settings saved.");

// 2. Attach to any existing Html<String> response — extends body with the OOB fragment.
let body = render_some_partial();
let response = toast::with(Html(body), toast::Toast::success("Settings saved."));

// 3. Empty OOB-only response (used when there's nothing to swap, only a notice).
let response = toast::oob_only(toast::Toast::warn("Restart required for changes to take effect.").with_action("/admin/system/restart", "Restart now"));
```

The helper always emits the OOB fragment with `hx-swap-oob="beforeend:#bnb-toasts"` so multiple toasts stack inside the region (newest at the bottom).

## Where it should be wired

Same source-grep methodology as O-17. Existing endpoints whose successful response should attach a toast:

| Endpoint | Toast |
|---|---|
| `POST /admin/settings` (every category) | success: "{category} saved." |
| `POST /admin/system/backup` | success: "Backup started — {filename}." |
| `POST /admin/system/restore/{name}` | warn (sticky): "Restoring… do not close this tab." |
| `POST /admin/notifications/test/{kind}` | success: "Test sent to {channel}." / error: "Channel responded: {status}." |
| `POST /admin/rules` (create) | success: "Rule '{name}' enabled." |
| `POST /admin/rules/{id}/toggle` | success: "Rule {enabled or disabled}." |
| `DELETE /admin/rules/{id}` | success: "Rule deleted." |
| `POST /admin/species/test` (no, this is read-only) | n/a |
| `POST /admin/migrate/run` | warn (sticky): "Import running — see progress below." |
| `POST /pages/quarantine/{id}/approve` | success: "Approved — {species} added to species list." |
| `POST /pages/quarantine/{id}/reject` | success: "Rejected." |
| `POST /admin/system_controls/...` (every destructive action) | warn / success depending on outcome |
| Generic 5xx handlers (`log_internal`) | error (sticky): "Something failed — see logs." |

Existing inline "saved" indicators in the rendered fragments can stay; the toast is **additive**, not a replacement for in-context feedback.

## Behavior

- Toast region: `position: fixed; top: calc(56px + 12px); right: 18px;` (clear of sticky top nav).
- `aria-live="polite"`, `aria-atomic="true"` — screen readers announce each toast on append.
- Each toast carries `role="status"` (success/warn) or `role="alert"` (error).
- Each renders with a `bnb-rise` entry animation (already in `app.css`).
- Auto-dismiss timer paused on hover and on focus (keyboard users can read at their pace).
- Max stack: 5; the oldest is removed when a sixth lands.
- Manual close button (`×`) on every toast.

## Sticky-toast escape hatch

For long-running operations (Restore, Migrate run, Update), `toast::warn(...).sticky()` omits the auto-dismiss timer. Pair with a manual close so the user can dismiss when they've read it.

## Risk

Zero. The region is empty by default — if no handler attaches an OOB toast, nothing renders. JS-disabled users still see whatever inline feedback the partial already returns.

---

<!-- BNB:CROSSREF-FOOTER -->
## Related

* **Pairs with O-17** — confirmed action → fire request → response includes `toast::success` OOB. The `birdnet:toast` CustomEvent makes the toast dispatchable from any inline script (e.g. the share-page Copy-permalink button).
<!-- BNB:CROSSREF-FOOTER -->
