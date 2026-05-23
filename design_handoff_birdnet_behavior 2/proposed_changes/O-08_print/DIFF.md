# O-08 · Print stylesheet

<!-- BNB:STATUS-HEADER -->
> **Risk:** none · **Priority:** 1 · **Status:** ready for review
> Acceptance: [VERIFY.md § O-08](../VERIFY.md#o-08--print-stylesheet) · Rollback: [ROLLBACK.md § O-08](../ROLLBACK.md#o-08--print-stylesheet)
<!-- BNB:STATUS-HEADER -->


## What

A dedicated `print.css` that makes any page in the app print well, with bespoke handling for `/weekly` and `/year-in-review` (the editorial layouts that *want* to be printed).

## Files

| Action | Path |
|---|---|
| Add | `crates/birdnet-web/static/css/print.css` |
| Patch | `crates/birdnet-web/templates/layout.html` — add `<link rel="stylesheet" href="/static/css/print.css" media="print">` after the existing app.css link |
| Optional patches | Year-in-review + weekly-report templates: add `data-print-page-break-after` to section wrappers where you want a fresh page |

## Behavior

1. **Forces light palette** — overrides every OKLCH custom property with hex equivalents so dark-mode users get a readable print. (Browsers won't print dark backgrounds by default, but child elements styled with OKLCH custom properties still pick up the wrong colors without the override.)
2. **Strips interactive chrome** — topnav, search, segmented controls, all buttons, audio elements, htmx loaders. Anything tagged `data-print-hide` is also removed.
3. **Resolves links inline** — long-form `(https://example.com)` after every external link, so a paper copy is still actionable.
4. **Avoids breaks mid-card** — `break-inside: avoid` on every `.bnb-card`, `.stat-tile`, `.stat-card`, `.feed-row`, plus `thead`/`tfoot` repeat on multi-page tables.
5. **Print-only page breaks** via `[data-print-page-break-after]`/`[data-print-page-break-before]` markers.
6. **Letter at 0.55–0.7 in margins**, first page gets a deeper top margin so editorial covers breathe.

## Editorial reports — add page-break markers

In `/templates/year_in_review.html` (or wherever the sections are emitted), add:

```html
<section data-print-page-break-after>…the cover hero…</section>
<section data-print-page-break-after>…big-number reel…</section>
<section>…calendar of the year…</section>
```

Same in `/templates/weekly_report.html`.

## How to use

User: opens `/weekly`, presses **Cmd+P**. Print preview shows a clean editorial PDF, no nav. Save as PDF → email to the family.

## Risk

Zero. Print-only stylesheet — never affects screen render.

---

<!-- BNB:CROSSREF-FOOTER -->
## Related

* **Apply order:** shipped in the combined PR — see [HANDOFF.md](../HANDOFF.md#what-ships-in-this-pr) for the full file list.
* **Acceptance criteria:** [VERIFY.md § O-08](../VERIFY.md#o-08--print-stylesheet).
* **Rollback:** [ROLLBACK.md § O-08](../ROLLBACK.md#o-08--print-stylesheet).
* **Preview:** open [`INDEX.html`](../INDEX.html#O-08) for the rendered screen.
<!-- BNB:CROSSREF-FOOTER -->
