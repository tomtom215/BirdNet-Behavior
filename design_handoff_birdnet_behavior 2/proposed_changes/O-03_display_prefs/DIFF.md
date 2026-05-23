# O-03 · Display preferences (theme · density · motion · contrast)

<!-- BNB:STATUS-HEADER -->
> **Risk:** none · **Priority:** 2 · **Status:** ready for review
> Acceptance: [VERIFY.md § O-03](../VERIFY.md#o-03--display-preferences) · Rollback: [ROLLBACK.md § O-03](../ROLLBACK.md#o-03--display-preferences)
<!-- BNB:STATUS-HEADER -->


## What

The current build has `localStorage`-backed *density* and *theme* wired into `layout.html`'s FOUC guard but **no UI to change them.** This adds a Display card with four segmented controls and two new keys (`bnb-motion`, `bnb-contrast`), reusing the existing tokens and the FOUC guard.

## Files

| Action | Path |
|---|---|
| Add | `crates/birdnet-web/templates/_partial_display_prefs.html` (or inline into `system.html`) |
| Append | `crates/birdnet-web/static/css/app.css` — see `css/app.css.append` |
| Patch | `crates/birdnet-web/templates/layout.html` — extend the FOUC guard to also read `bnb-motion` and `bnb-contrast` (one-line change, see below) |

## Layout.html patch

Replace the existing FOUC guard `<script>` at the top of `<head>` with:

```html
<script>(function(){
  var t=localStorage.getItem('theme');
  if(t!=='light'&&t!=='dark'){
    t=(t==='auto'||!t) ? (window.matchMedia('(prefers-color-scheme:dark)').matches?'dark':'light') : t;
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

(Same shape as the current guard — just three extra lines for `data-motion`, `data-contrast`, and the `auto` theme fallback.)

## Integration

The card is self-contained — drop the file's HTML into wherever you want it. Two recommended placements:

1. **Inline in `system.html`** as the first card on the System page — the natural home for user prefs.
2. **As a partial endpoint** `/pages/display-prefs` so it can also be popped open from a settings icon in the topnav. Endpoint signature:

```rust
async fn display_prefs_partial() -> Html<&'static str> {
    Html(include_str!("../../../templates/_partial_display_prefs.html"))
}
```

## Behavior

- All keys persist immediately on click (no Save button — that's the better pattern for this kind of UI).
- "Auto" theme tracks OS via `matchMedia('(prefers-color-scheme: dark)')` change events.
- `[data-motion="reduced"]` adds an explicit motion override layered *on top of* the existing `@media (prefers-reduced-motion: reduce)` rule, so a user can force-disable motion even on systems that lie about it.
- `[data-contrast="high"]` thickens borders and ink — values picked to remain on the OKLCH palette.
- Reset button restores every key to its default.

## Risk

Zero. The keys are additive; the existing guard logic and density resolver are unchanged for anyone who never opens the card.

---

<!-- BNB:CROSSREF-FOOTER -->
## Related

* **Apply order:** shipped in the combined PR — see [HANDOFF.md](../HANDOFF.md#what-ships-in-this-pr) for the full file list.
* **Acceptance criteria:** [VERIFY.md § O-03](../VERIFY.md#o-03--display-preferences).
* **Rollback:** [ROLLBACK.md § O-03](../ROLLBACK.md#o-03--display-preferences).
* **Preview:** open [`INDEX.html`](../INDEX.html#O-03) for the rendered screen.
<!-- BNB:CROSSREF-FOOTER -->
