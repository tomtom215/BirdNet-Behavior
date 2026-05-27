# O-26 · IA fix — stop using the footer as the secondary nav

<!-- BNB:STATUS-HEADER -->
> **Risk:** none (template + CSS only) · **Priority:** 1 (everyone hits this on every page) · **Status:** ready for review
> Acceptance: VERIFY.md § O-26 · Rollback: ROLLBACK.md § O-26
<!-- BNB:STATUS-HEADER -->


## What

`layout.html`'s footer currently carries twelve links in a `flex-wrap` row:

> *Today · History · Weekly report · Year in review · Recordings · Gallery · Time series · Co-occurrence · Notifications · System · Admin · Kiosk*

Of those, **ten don't appear in the topnav**: History, Weekly, Year-in-review, Recordings, Gallery, Time series, Co-occurrence, Notifications, Admin, Kiosk. That makes the footer the *primary secondary navigation* — and it's a particularly bad place for it:

1. **Below the fold on every page.** Year-in-review and Trends are tall scroll surfaces. A user who scrolls down has to scroll all the way past the content to find another page; the footer is effectively invisible from the top of the screen.
2. **No grouping or hierarchy.** Twelve sibling links with no section headers and no active-state styling. A user navigating from `/correlation` can't tell at a glance which sibling section they're in.
3. **Wrong visual vocabulary for the role.** The footer is small, muted, centred, hairline-bordered — every cue says *"page closure"*. Putting destinations there fights the cue.
4. **Discoverability collapses on tall surfaces.** Kiosk mode (which is in the list!) is hardest to reach precisely because kiosk presets are tall.
5. **Duplicates the command palette without its grouping.** Once O-19 lands, ⌘K covers every destination — but a footer flex-wrap of plain anchors gives users no signal that the keyboard route exists.

This change:

1. **Adds an overflow `⌗ More ▾` button** to the right end of the topnav. Click opens a grouped dropdown (desktop) / bottom sheet (phone — already shipped in O-24) listing the destinations not in the primary nav.
2. **Strips the destination links out of the footer** entirely. The footer becomes a real footer: brand, version, uptime, About / Help / Changelog / RSS / iCal links, copyright.
3. **Adds nothing to the primary topnav.** The current nine primary destinations stay in place — they're the right set, and the More menu absorbs everything else without crowding.
4. **Surfaces ⌘K** prominently in both the More menu and the footer.

## Files

| Action | Path |
|---|---|
| Patch | `crates/birdnet-web/templates/layout.html` — see "Patched layout.html" below |
| Add | `crates/birdnet-web/templates/_partial_topnav_more.html` — the More button + dropdown |
| Add | `crates/birdnet-web/templates/_partial_footer.html` — the real footer |
| Append | `crates/birdnet-web/static/css/app.css` — see `css/app.css.append` |

## The IA, made explicit

Primary topnav (already shipped, unchanged):

> Dashboard · Today · Species · Heatmap · Migration · Analytics · Life list · Quarantine · System

Right rail of the topnav: existing search + status pill + theme toggle, **plus** a new `⌗ More ▾` button immediately before the theme toggle.

Dropdown contents (one disclosure, grouped):

| Group | Items |
|---|---|
| **Reports** | History · Weekly report · Year-in-review |
| **Audio & images** | Recordings · Gallery |
| **Analytics — deep dives** | Dawn chorus · Co-occurrence · Time series · *(Heatmap and Migration are already in the topnav)* |
| **Operations** | Notifications · Admin · Kiosk mode |
| **Help** | Changelog · Help docs · *⌘K — open the command palette* |

Each row in the dropdown is a real `<a>` with a stable href, a 22 px glyph, the label, and an optional one-line sub-caption ("Sunday recap", "Last 60 days of co-occurrence ρ"). Active state highlights the row when the user is already on that page. Keyboard: `↑ ↓ ↵ Esc`. Closes on outside-click or Esc.

The same dropdown contents are already wired into the mobile *More* sheet from **O-24** — this PR refactors that sheet's data into a shared partial (`_partial_topnav_more.html`) so both surfaces stay synchronised. One source of truth.

## Patched layout.html

Two narrow patches. (1) Replace the footer block, (2) inject the topnav-more partial just before the theme toggle.

### Topnav — one-line addition

```html
<!-- before the theme toggle button: -->
{{include _partial_topnav_more.html}}
<button class="icon-btn" id="theme-toggle" ...>...</button>
```

### Footer — full replacement

Replace the existing `<footer role="contentinfo">…</footer>` block with `{{include _partial_footer.html}}`. The new footer is in this PR.

## The new footer — what's in scope

The footer's *real* role is page closure. It carries:

- **Brand line** — wordmark + version + a small live uptime pill (`listening · 14d 02h`).
- **A tiny set of site-meta links** — five at most: About this station · Help · Changelog · RSS (rare detections) · iCal (rare confirmations).
- **Copyright + provenance** — `© BirdNet-Behavior · self-hosted on this device`.

That's it. No "Today", no "Year in review", no "Admin". Every destination has a home above the fold (either in the topnav, in the More dropdown, or in the command palette).

## How users get to every destination after this change

| Destination | Topnav | More | Cmd-K | Footer |
|---|---|---|---|---|
| Dashboard | ✓ | — | ✓ | brand link |
| Today | ✓ | — | ✓ | — |
| Species | ✓ | — | ✓ | — |
| Heatmap | ✓ | — | ✓ | — |
| Migration | ✓ | — | ✓ | — |
| Analytics | ✓ | — | ✓ | — |
| Life list | ✓ | — | ✓ | — |
| Quarantine | ✓ | — | ✓ | — |
| System | ✓ | — | ✓ | — |
| History | — | Reports | ✓ | — |
| Weekly report | — | Reports | ✓ | — |
| Year-in-review | — | Reports | ✓ | — |
| Recordings | — | Audio | ✓ | — |
| Gallery | — | Audio | ✓ | — |
| Dawn chorus | — | Analytics | ✓ | — |
| Co-occurrence | — | Analytics | ✓ | — |
| Time series | — | Analytics | ✓ | — |
| Notifications | — | Operations | ✓ | — |
| Admin | — | Operations | ✓ | — |
| Kiosk | — | Operations | ✓ | — |
| Changelog | — | Help | ✓ | meta link |
| Help docs | — | Help | ✓ | meta link |
| About | — | — | — | meta link |
| RSS / iCal | — | — | — | meta link |

**Every destination keeps multiple discoverable entry points.** The change is from "twelve plain links pasted at the bottom" to "grouped, labeled, active-state-aware secondary menu at the top, with the footer for closure."

## Risk

Zero. Pure template + CSS change. No Rust touched. Rolling back is a single revert of `layout.html` plus deleting the two partials.

---

<!-- BNB:CROSSREF-FOOTER -->
## Related

* Reuses the `<dialog>` primitive from O-17.
* The dropdown's contents share a partial with O-24's mobile *More* sheet — one source of truth, two presentations.
* The command-palette hint in the dropdown's Help group ties to O-19.
<!-- BNB:CROSSREF-FOOTER -->
