# Design tokens

The web UI's entire visual language — colour, type, spacing, elevation, corner
radii, motion — is expressed as a small set of **CSS custom properties**
("design tokens") defined once at the top of
[`crates/birdnet-web/static/css/app.css`](https://github.com/tomtom215/BirdNet-Behavior/blob/main/crates/birdnet-web/static/css/app.css).
Components read tokens; a colour or a pixel pad written into a rule is a defect
unless this page says why it is allowed (see [What stays literal](#what-stays-literal)).
This page is the reference for what each token means and what value it carries
in each theme.

Two principles govern them:

- **Colours are [OKLCH](https://oklch.com/).** OKLCH is perceptually uniform, so
  a hue stays the same "colour" as its lightness changes, and light/dark themes
  can be derived by moving lightness rather than re-picking hex values. The
  format is `oklch(L C H)` — lightness %, chroma, hue degrees — with an optional
  `/ alpha`. The one family written as hex is the categorical chart palette,
  because a colour-blind-safety check can only be run on the exact values that
  ship (see [Charts](#colour--charts)).
- **The token set is closed.** New UI is built by composing existing tokens into
  new *classes*, **not** by adding tokens. Adding a token is a deliberate,
  reviewed design decision (see [Adding or changing a token](#adding-or-changing-a-token)).

## At a glance

The stylesheet's light block declares
<!--count-light-->123<!--/count--> tokens, and the dark block overrides
<!--count-dark-->39<!--/count--> of them. Those two numbers are checked against
`app.css` by the test below, so they cannot go stale; every other number on this
page that describes the present is read from the stylesheet the same way.

| What | Enforced by |
|---|---|
| Every token declared here, with the value it has in each theme | `tests/design_tokens_doc_matches_the_stylesheet.rs` |
| Every `var(--x)` names a token that exists | `tests/css_variables_are_defined.rs` |
| Every token that flips between themes is forced to a light value on paper | `tests/print_forces_every_theme_flipped_token.rs` |
| No raw colour literal outside the token blocks, and no raw weight, hairline or bar radius where a token exists | `tests/design_tokens_are_the_only_literals.rs` |
| Every colour token has a value in both themes, or is named as theme-invariant | `tests/design_tokens_are_the_only_literals.rs` |
| The contrast ratios the tokens claim, measured in a browser in both themes | `tools/visual-qa/tokens.mjs` |
| A token refactor changes no element's computed style | `tools/visual-qa/style-snapshot.mjs` |

## Themes

Tokens are defined for two themes, selected by the `data-theme` attribute on the
`:root` element (set early by `static/theme-guard.js` to avoid a flash):

| Theme | Selector | Notes |
|---|---|---|
| Light | `:root`, `:root[data-theme="light"]` | The default. |
| Dark  | `:root[data-theme="dark"]` | Overrides the colour + shadow tokens only; structural tokens (radii, type, motion, layers, spacing) are shared. |

Because components only ever read tokens, switching `data-theme` reskins the
whole UI with no per-component dark-mode CSS. A token that differs between the
two blocks must also be listed in `static/css/print.css`, which forces the light
values on paper (`print_forces_every_theme_flipped_token.rs` enforces it).

## Colour — surfaces & lines

The greys that build the page: backgrounds, card surfaces, and dividers.

| Token | Light | Dark | Use |
|---|---|---|---|
| `--bg` | `oklch(98.5% 0.004 80)` | `oklch(12% 0.008 250)` | Page background. |
| `--bg-2` | `oklch(96.5% 0.005 80)` | `oklch(15% 0.008 250)` | Secondary background / inset wells. |
| `--surface` | `oklch(100% 0 0)` | `oklch(18% 0.010 250)` | Card / panel surface. |
| `--surface-2` | `oklch(97.5% 0.004 80)` | `oklch(22% 0.010 250)` | Nested surface, hover fill. |
| `--border` | `oklch(90% 0.006 80)` | `oklch(28% 0.014 250)` | Default border. |
| `--border-2` | `oklch(82% 0.008 80)` | `oklch(40% 0.018 250)` | Stronger border (inputs, buttons). |
| `--hairline` | `oklch(94% 0.005 80)` | `oklch(24% 0.012 250)` | Faint divider between list rows. |

## Colour — text

A four-step text hierarchy, brightest (most important) to faintest.

| Token | Light | Dark | Use |
|---|---|---|---|
| `--fg` | `oklch(22% 0.008 70)` | `oklch(97% 0.004 240)` | Primary text. |
| `--fg-2` | `oklch(40% 0.008 70)` | `oklch(80% 0.008 240)` | Secondary text. |
| `--fg-3` | `oklch(49% 0.008 70)` | `oklch(64% 0.010 240)` | Muted / metadata. |
| `--fg-4` | `oklch(55% 0.012 70)` | `oklch(62% 0.014 240)` | Faint / disabled. |

## Colour — semantic hues

Four hue families carry meaning across the app, and the legacy names `--success`,
`--warning`, `--danger` (below) say which: **success** is moss, **warning** is
dawn, **danger** is rare, and **information** is its own cool blue so that "for
your information" never reads as either "all is well" or "look at this". Each has
a base, a `-soft` tint (for fills/badges) and an `-ink` (for text/icons on a soft
fill; `--rare` is dark enough to be its own ink).

| Token | Light | Dark | Meaning |
|---|---|---|---|
| `--moss` | `oklch(52% 0.09 150)` | `oklch(78% 0.18 150)` | Primary / healthy / success — the "green" of a thriving yard. |
| `--moss-soft` | `oklch(92% 0.04 150)` | `oklch(26% 0.10 150)` | Soft green fill (a positive pill, the "live" tint, a very-sure confidence pill). |
| `--moss-ink` | `oklch(35% 0.09 150)` | `oklch(90% 0.16 150)` | Green text/ink on a soft fill. |
| `--dawn` | `oklch(68% 0.12 60)` | `oklch(82% 0.18 65)` | Morning / dawn chorus / warning — amber. A fill, never text. |
| `--dawn-soft` | `oklch(94% 0.05 65)` | `oklch(28% 0.10 60)` | Soft amber fill (a warning pill, a borderline-confidence pill). |
| `--dawn-ink` | `oklch(42% 0.12 55)` | `oklch(92% 0.16 60)` | Amber text/ink. |
| `--rare` | `oklch(50% 0.16 28)` | `oklch(74% 0.20 25)` | Rare bird / alert / danger — red. |
| `--rare-soft` | `oklch(94% 0.05 28)` | `oklch(28% 0.12 25)` | Soft red fill (rare-bird and alert badges). Never a confidence. |
| `--info` | `oklch(48% 0.10 245)` | `oklch(76% 0.11 245)` | Information — a cool blue. A fill or an icon; the text on it is `--info-ink`. |
| `--info-soft` | `oklch(94% 0.03 245)` | `oklch(27% 0.06 245)` | Soft blue fill (an informational pill or callout). |
| `--info-ink` | `oklch(36% 0.10 245)` | `oklch(88% 0.07 245)` | Blue text/ink on a soft fill. |

## Colour — confidence tiers

How sure the identifier was, painted the same way on every surface that shows it
(the Today feed, species, recordings, the review and quarantine queues, the share
page). **One rule picks the tier** — `crates/birdnet-web/src/routes/pages/confidence.rs`
— and these tokens only say what each tier looks like.

| Tier | Whole percent | Reads as |
|---|---|---|
| high | 90 and up | very sure |
| mid | from the borderline edge to 89 | sure |
| low | below the borderline edge | borderline |

The *borderline edge* follows the station's own confidence cut-off: the cut-off
plus ten points, never above 90. At a 70 % cut-off, 80–89 is mid and 70–79 is low;
a strict station (85 % or more) has no mid tier. The scale never uses `--rare`:
a detection the station accepted at 72 % is not a fault.

| Token | Light | Dark | Use |
|---|---|---|---|
| `--conf-high` | `var(--moss)` | `oklch(70% 0.12 150)` | Bar fill for a very sure detection. |
| `--conf-high-soft` | `var(--moss-soft)` | Pill background, high. |
| `--conf-high-ink` | `var(--moss-ink)` | Pill text, high. |
| `--conf-mid` | `oklch(58% 0.025 150)` | `oklch(68% 0.025 150)` | Bar fill for a sure detection. |
| `--conf-mid-soft` | `var(--surface-2)` | Pill background, mid. |
| `--conf-mid-ink` | `var(--fg-2)` | Pill text, mid. |
| `--conf-low` | `oklch(62% 0.125 74)` | `oklch(80% 0.14 75)` | Bar fill for a borderline detection. |
| `--conf-low-soft` | `var(--dawn-soft)` | Pill background, low. |
| `--conf-low-ink` | `var(--dawn-ink)` | Pill text, low. |

## Colour — special surfaces

| Token | Light | Dark | Use |
|---|---|---|---|
| `--paper` | `oklch(96% 0.012 75)` | `oklch(18% 0.010 250)` | Warm "paper" surface for editorial recaps (Reports). |
| `--night` | `oklch(28% 0.04 270)` | `oklch(8% 0.020 250)` | Deep night surface (e.g. live spectrogram backdrop). |
| `--card-top` | `var(--surface)` | `oklch(20% 0.010 250)` | The top of a dark card's gradient. Light cards are flat, so light only names it. |
| `--card-bottom` | `var(--surface)` | `oklch(17% 0.010 250)` | The bottom of a dark card's gradient. |
| `--sheen` | `transparent` | `oklch(100% 0 0 / .04)` | A faint light along a dark card's upper edge. |
| `--glow` | `transparent` | `oklch(60% 0.04 240 / .10)` | The soft halo behind display type on a dark page. |

## Colour — text on a filled surface

| Token | Light | Dark | Use |
|---|---|---|---|
| `--on-moss` | `oklch(100% 0 0)` | `oklch(18% 0.02 150)` | Text on a `--moss` fill (primary buttons). |
| `--on-fill` | `#fff` | `oklch(15% 0.02 150)` | Text on any saturated fill (badges, hovered amber buttons). |
| `--sp-ink` | `oklch(10% 0 0)` | `oklch(98% 0 0)` | What a species colour is mixed toward for its own text, so the hue stays and the contrast holds. |
| `--accent-rgb` | `60, 120, 80` | `120, 200, 150` | The accent as an `r, g, b` triple, for `rgba()` tints. |

Two inks are fixed because the fill they sit on does not change between themes:

| Token | Value | Use |
|---|---|---|
| `--on-dawn` | `oklch(20% 0.04 60)` | Ink on a `--dawn` fill (the amber button): amber is a bright fill in both themes, so its ink is dark in both. |
| `--switch-thumb` | `oklch(100% 0 0)` | A toggle's thumb, white on its track in both themes. |

## Elevation — shadows

Three elevations. Each layers a soft drop shadow with a hairline `0 0 0 0.5px`
ring so a card reads as lifted on both themes; the dark theme deepens the
shadow and switches the ring to a faint light edge.

| Token | Light | Dark | Use |
|---|---|---|---|
| `--shadow-sm` | `0 1px 2px oklch(0% 0 0 / .04), 0 0 0 0.5px oklch(0% 0 0 / .04)` | `0 1px 2px oklch(0% 0 0 / .50), 0 0 0 0.5px oklch(100% 0 0 / .04)` | Subtle lift (cards, chips, inputs). |
| `--shadow-md` | `0 4px 14px oklch(0% 0 0 / .06), 0 0 0 0.5px oklch(0% 0 0 / .04)` | `0 8px 24px oklch(0% 0 0 / .55), 0 0 0 0.5px oklch(100% 0 0 / .06)` | Popovers, hovered cards. |
| `--shadow-lg` | `0 18px 40px oklch(0% 0 0 / .08), 0 0 0 0.5px oklch(0% 0 0 / .05)` | `0 24px 60px oklch(0% 0 0 / .65), 0 0 0 0.5px oklch(100% 0 0 / .07)` | Modals, the floating now-playing bar. |

## Focus & scrim

| Token | Value | Use |
|---|---|---|
| `--ring` | `0 0 0 2px var(--surface), 0 0 0 4px var(--moss)` | The keyboard focus ring for a field: a `--surface` gap, then `--moss`. One token so a high-contrast theme changes it once. |
| `--ring-soft` | `0 0 0 3px color-mix(in oklch, var(--moss) 22%, transparent)` | The softer halo a pointer-focused field keeps. |
| `--scrim` | `oklch(0% 0 0 / .32)` | The dimming layer behind a dialog, a drawer or a sheet. Five rules had three different opacities (22, 32 and 38 %) before this existed. |

## Radii

| Token | Value | Use |
|---|---|---|
| `--r-2xs` | `3px` | The ends of a bar, a tile's chip, a small mark. |
| `--r-xs` | `4px` | Checkboxes, tiny chips. |
| `--r-sm` | `6px` | Buttons, inputs. |
| `--r-md` | `10px` | Cards, rows. |
| `--r-lg` | `14px` | Large cards, the head player. |
| `--r-xl` | `20px` | Hero panels. |
| `--r-pill` | `999px` | A fully rounded pill (chips, badges, round buttons). |

## Borders

| Token | Value | Use |
|---|---|---|
| `--bw` | `0.5px` | The hairline: every card edge, row divider and control edge. High contrast (the Display preference, or the OS asking for more) changes this one value to `--bw-strong`, and every line in the product thickens at once. |
| `--bw-strong` | `1px` | A line that must read as a line: what `--bw` becomes under high contrast. |

## Spacing & density

Padding is derived from a single `--density` multiplier, so the operator's
density preference (**Settings → General → Display**) rescales the whole UI by changing one
value rather than re-spacing every component.

| Token | Value | Use |
|---|---|---|
| `--density` | `1` | Density multiplier (compact / comfy / regular set this). |
| `--pad-1` | `calc(8px * var(--density))` | Tight padding / small gaps. |
| `--pad-2` | `calc(12px * var(--density))` | Default control padding. |
| `--pad-3` | `calc(18px * var(--density))` | Card padding; the gap between blocks on a page. |
| `--pad-4` | `calc(28px * var(--density))` | Section padding. |

`--pad-*` is the rhythm *between* blocks and *inside* cards, and it moves with
density. The small gaps *within* a component — an icon beside its label, the
cells of a calendar, the gap under an eyebrow — are optical adjustments to that
component's own size and must not move with density, so they have their own,
fixed ladder. `--space-N` is N pixels. It is the set of values the stylesheet
used most, not a modular scale. The odd in-between gaps (3, 5, 7, 9, 11, 13 px)
are still written as literals where a component's own optical balance depends
on them; a new rule should use the nearer rung.

| Token | Value | Use |
|---|---|---|
| `--space-2` | `2px` | The gap between a number and the bar under it. |
| `--space-4` | `4px` | Tight pairs: a label and its value. |
| `--space-6` | `6px` | An icon and its label; chips in a row. |
| `--space-8` | `8px` | Rows of controls; a heading and its first line. |
| `--space-10` | `10px` | The default gap inside a list row or a card header. |
| `--space-12` | `12px` | Between a card's header and its body. |
| `--space-14` | `14px` | Between small cards in a grid. |
| `--space-16` | `16px` | Between groups inside a card. |
| `--space-18` | `18px` | Between a detail page's blocks: a card, then its actions or the next card. |
| `--space-20` | `20px` | Between a page's blocks on a phone. |
| `--space-24` | `24px` | Between a page's blocks on a desktop. |
| `--space-32` | `32px` | Between a page's major parts: an identity strip and what follows it. |

## Typography — families

Self-hosted font stacks (no external font CDNs) with broad CJK / Devanagari
fallbacks so localized common names render.

| Token | Value | Use |
|---|---|---|
| `--font-display` | `"Instrument Serif", "Source Serif 4", Georgia, "Noto Serif CJK SC", "Noto Serif CJK JP", serif` | Editorial headlines, card titles and statistic numerals. |
| `--font-ui` | `"Inter Tight", "Noto Sans", "Noto Sans CJK SC", "Noto Sans CJK JP", "Noto Sans CJK KR", "Noto Sans Devanagari", -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif` | Body and UI text. |
| `--font-mono` | `"JetBrains Mono", "IBM Plex Mono", ui-monospace, SFMono-Regular, monospace` | Times, codes, numeric tables. |

## Typography — sizes

The stylesheet used to carry dozens of `font-size` values, half-pixel steps
included. They are a scale now. The first six steps are UI text; the rest are the
serif display steps. Sizes at or below 16 px were snapped to the nearest step
(never by more than 0.8 px); larger one-off sizes (an icon glyph, a gauge
numeral, a 404 code) stay literal and are listed under
[What stays literal](#what-stays-literal).

| Token | Value | Use |
|---|---|---|
| `--fs-2xs` | `10px` | Axis ticks, banding codes, the smallest mono tags. |
| `--fs-xs` | `11px` | Eyebrows, table heads, captions, chips. |
| `--fs-sm` | `12px` | Hints, meta lines, secondary controls. |
| `--fs-md` | `13px` | Buttons, form controls, table cells. |
| `--fs-base` | `14px` | Body text (set on `<body>`). |
| `--fs-lg` | `16px` | Lede paragraphs and sub-headings. 16 px also keeps iOS from zooming a focused field. |
| `--fs-lead` | `20px` | A headline sentence under a title; a small numeral. |
| `--fs-title` | `22px` | A card or section title, in the serif. |
| `--fs-h3` | `24px` | A sub-section title, or a numeral between a title and a statistic. |
| `--fs-stat` | `32px` | A statistic's numeral, in the serif. |
| `--fs-stat-lg` | `44px` | The large numeral of a four-up stat tile. |
| `--fs-h1` | `clamp(34px, 9vw, 46px)` | A page title. Fluid, so a phone gets a title that fits rather than one that wraps four times. |
| `--fs-hero` | `clamp(40px, 11vw, 60px)` | An editorial headline: Today, the species name, the weekly and yearly recaps. |

## Typography — weights and tracking

The stylesheet writes `font-weight` and `letter-spacing` as these, never as a
number. Inter Tight ships 400, 500 and 600; 700 is the browser's synthetic bold
and is asked for in a handful of places only.

| Token | Value | Use |
|---|---|---|
| `--fw-regular` | `400` | Body text; the serif display faces have one weight. |
| `--fw-medium` | `500` | UI labels, buttons, table heads, names. |
| `--fw-semibold` | `600` | Emphasis inside a sentence; a selected tab. |
| `--fw-bold` | `700` | Rare: a count in a badge. |
| `--track-tight` | `-0.01em` | UI headings. |
| `--track-display` | `-0.02em` | The serif at display sizes, where its default spacing opens up. |
| `--track-caps-sm` | `0.08em` | Small-caps table heads and tile labels. |
| `--track-caps` | `0.10em` | Eyebrows. |

## Motion

Three durations and two curves. `prefers-reduced-motion` (and the operator's
Motion setting) flatten every transition and animation at once, so none of these
needs its own opt-out.

| Token | Value | Use |
|---|---|---|
| `--dur-fast` | `.12s` | Hover and press feedback (background, border, colour). |
| `--dur-base` | `.2s` | Disclosures, popovers, dialogs and drawers appearing. |
| `--dur-slow` | `.32s` | Something moving a distance: a progress bar, the floating dock, a row rising in. |
| `--ease-out` | `cubic-bezier(.2, .7, .2, 1)` | Things arriving. |
| `--ease-in-out` | `cubic-bezier(.4, 0, .2, 1)` | Things that loop or travel and settle (the skeleton shimmer). |

## Layers

Only the stacking layers that must agree across components are tokens. A
`z-index` of 0 or 1 inside a component's own stacking context is local and stays
a literal.

| Token | Value | Use |
|---|---|---|
| `--z-raised` | `5` | A sticky bar inside a page (the Today search row). |
| `--z-sticky` | `50` | The top navigation and the floating recordings dock. |
| `--z-tabbar` | `55` | The phone's bottom tab bar (above the dock, below toasts). |
| `--z-toast` | `60` | The toast region. |
| `--z-skip` | `100` | The skip-to-content link, above everything when focused. |

## Legacy aliases

A handful of tokens from before the v3 redesign are kept as **aliases** of the
current set, so pre-redesign partials stay visually consistent without being
rewritten. Prefer the modern token in new code.

| Legacy alias | Resolves to |
|---|---|
| `--bg-card` | `var(--surface)` |
| `--bg-hover` | `var(--surface-2)` |
| `--text` | `var(--fg)` |
| `--text-muted` | `var(--fg-3)` |
| `--accent` | `var(--moss)` |
| `--accent-dim` | `var(--moss-ink)` |
| `--success` | `var(--moss)` |
| `--danger` | `var(--rare)` |
| `--radius` | `var(--r-md)` |
| `--input-bg` | `var(--surface)` |
| `--conf-high-bg` | `var(--conf-high-soft)` |
| `--conf-mid-bg` | `var(--conf-mid-soft)` |
| `--conf-low-bg` | `var(--conf-low-soft)` |

One alias differs by theme, because amber is a fill and not a text colour:

| Legacy alias | Light | Dark | Use |
|---|---|---|---|
| `--warning` | `var(--dawn-ink)` | `var(--dawn)` | Warning text: the dark amber ink on paper, the bright amber on night. |

`--accent-rgb` (a comma-separated RGB triple, listed above) is the one non-OKLCH
value, kept for the few rules that still need `rgba()` with a variable alpha.

## What stays literal

A literal is allowed when it is not a design choice that a theme or a density
change should move. The clusters that remain, and why:

| Cluster | Why it is not a token |
|---|---|
| Species identity hues (`--sp`, set per bird) | Data, computed per species at render time. |
| Chart ramps (heatmap cells, the streamgraph and ridgeline bands, the chord ribbons) | A colour encoding values; each is derived from `--moss` / `--dawn` with `color-mix()` or set per data point. |
| Loops: skeleton shimmer 1.4 s, pulse 1.6 s, blink 1.2 s, the 0.6 s fresh-row fade | Ambient animation with its own tempo, not interaction feedback. |
| `black` in a `mask-image` | An alpha mask has no colour of its own; only its alpha is read. |
| Heading sizes above 16 px other than the steps above (gauge and tile numerals, an icon glyph, a 404 code) | Per-surface display type, sized to its one container. |
| Padding and the 2 px, 5 px, 7 px and 8 px bar radii | Optical adjustments to one component. A `gap` or a vertical margin that matches a `--space-N` rung uses it. |
| Colours inside the admin pages' own `<style>` blocks, and in the SVG a chart renderer writes | Counted, not forbidden: `tests/design_tokens_are_the_only_literals.rs` holds each file to the number it has now, so none can grow. |

## Adding or changing a token

The token set is deliberately closed (see the v3 spine
[protect list](https://github.com/tomtom215/BirdNet-Behavior/blob/main/docs/design/handover/v3_spine/IMPLEMENTATION_PLAN.md) —
"OKLCH token set: zero new tokens"). The structural tokens on this page — type,
weights, tracking, the hairline, the fixed gaps, motion, layers, focus, scrim and
the pill radius — were each added in a reviewed pass because each replaced a
cluster of ad-hoc literals rather than introducing a new idea; a refactor of that
kind is proven pure with `tools/visual-qa/style-snapshot.mjs` (snapshot every
element's computed style before, and after, and diff).

- **Build new UI from existing tokens.** Compose them into new CSS *classes*;
  that is the additive path and needs no token change.
- **A genuinely new token is a design decision.** Justify it in the PR, give it
  values for **both** themes when it is a colour (keep light and dark in
  lockstep), add it to this page in the same commit, and prefer extending a hue
  family (`-soft` / `-ink`) over inventing a new hue.
- **A token whose value differs between the themes also goes in `print.css`.**
- **Never hard-code a colour or pad** in a component — read the token, so a
  future theme or density change stays a one-line edit.
