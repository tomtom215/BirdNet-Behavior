# Design tokens

The web UI's entire visual language — colour, type, spacing, elevation, corner
radii — is expressed as a small set of **CSS custom properties** ("design
tokens") defined once at the top of
[`crates/birdnet-web/static/css/app.css`](https://github.com/tomtom215/BirdNet-Behavior/blob/main/crates/birdnet-web/static/css/app.css).
Every component reads tokens; nothing hard-codes a colour or a pixel pad. This
page is the reference for what each token means and what value it carries in
each theme.

Two principles govern them:

- **Colours are [OKLCH](https://oklch.com/).** OKLCH is perceptually uniform, so
  a hue stays the same "colour" as its lightness changes, and light/dark themes
  can be derived by moving lightness rather than re-picking hex values. The
  format is `oklch(L C H)` — lightness %, chroma, hue degrees — with an optional
  `/ alpha`.
- **The token set is closed.** New UI is built by composing existing tokens into
  new *classes*, **not** by adding tokens. Adding a token is a deliberate,
  reviewed design decision (see [Adding or changing a token](#adding-or-changing-a-token)).

## Themes

Tokens are defined for two themes, selected by the `data-theme` attribute on the
`:root` element (set early by `static/theme-guard.js` to avoid a flash):

| Theme | Selector | Notes |
|---|---|---|
| Light | `:root`, `:root[data-theme="light"]` | The default. |
| Dark  | `:root[data-theme="dark"]` | Overrides the colour + shadow tokens only; structural tokens (radii, spacing, type) are shared. |

Because components only ever read tokens, switching `data-theme` reskins the
whole UI with no per-component dark-mode CSS.

## Colour — surfaces & lines

The greys that build the page: backgrounds, card surfaces, and dividers.

| Token | Light | Dark | Use |
|---|---|---|---|
| `--bg` | `oklch(98.5% 0.004 80)` | `oklch(12% 0.008 250)` | Page background. |
| `--bg-2` | `oklch(96.5% 0.005 80)` | `oklch(15% 0.008 250)` | Secondary background / inset wells. |
| `--surface` | `oklch(100% 0 0)` | `oklch(18% 0.010 250)` | Card / panel surface. |
| `--surface-2` | `oklch(97.5% 0.004 80)` | `oklch(22% 0.010 250)` | Nested surface, hover fill. |
| `--border` | `oklch(90% 0.006 80)` | `oklch(28% 0.014 250)` | Default border. |
| `--border-2` | `oklch(82% 0.008 80)` | `oklch(40% 0.018 250)` | Stronger border (inputs, focus rings). |
| `--hairline` | `oklch(94% 0.005 80)` | `oklch(24% 0.012 250)` | Faint divider between list rows. |

## Colour — text

A four-step text hierarchy, brightest (most important) to faintest.

| Token | Light | Dark | Use |
|---|---|---|---|
| `--fg` | `oklch(22% 0.008 70)` | `oklch(97% 0.004 240)` | Primary text. |
| `--fg-2` | `oklch(40% 0.008 70)` | `oklch(80% 0.008 240)` | Secondary text. |
| `--fg-3` | `oklch(55% 0.008 70)` | `oklch(60% 0.010 240)` | Muted / metadata. |
| `--fg-4` | `oklch(70% 0.008 70)` | `oklch(42% 0.012 240)` | Faint / disabled. |

## Colour — semantic hues

Three hue families carry meaning across the app. Each has a base, a `-soft`
tint (for fills/badges) and most have an `-ink` (for text/icons on a soft fill).

| Token | Light | Dark | Meaning |
|---|---|---|---|
| `--moss` | `oklch(55% 0.09 150)` | `oklch(78% 0.18 150)` | Primary / healthy / success — the "green" of a thriving yard. |
| `--moss-soft` | `oklch(92% 0.04 150)` | `oklch(26% 0.10 150)` | Soft green fill (e.g. high-confidence background). |
| `--moss-ink` | `oklch(35% 0.09 150)` | `oklch(90% 0.16 150)` | Green text/ink on a soft fill. |
| `--dawn` | `oklch(68% 0.12 60)` | `oklch(82% 0.18 65)` | Morning / dawn chorus / warning — amber. |
| `--dawn-soft` | `oklch(94% 0.05 65)` | `oklch(28% 0.10 60)` | Soft amber fill (mid-confidence background). |
| `--dawn-ink` | `oklch(42% 0.12 55)` | `oklch(92% 0.16 60)` | Amber text/ink. |
| `--rare` | `oklch(58% 0.16 28)` | `oklch(74% 0.20 25)` | Rare bird / alert / danger — red. |
| `--rare-soft` | `oklch(94% 0.05 28)` | `oklch(28% 0.12 25)` | Soft red fill (low-confidence / rare badge background). |

## Colour — special surfaces

| Token | Light | Dark | Use |
|---|---|---|---|
| `--paper` | `oklch(96% 0.012 75)` | `oklch(18% 0.010 250)` | Warm "paper" surface for editorial recaps (Reports). |
| `--night` | `oklch(28% 0.04 270)` | `oklch(8% 0.020 250)` | Deep night surface (e.g. live spectrogram backdrop). |

## Elevation — shadows

Three elevations. Each layers a soft drop shadow with a hairline `0 0 0 0.5px`
ring so a card reads as lifted on both themes; the dark theme deepens the
shadow and switches the ring to a faint light edge.

| Token | Use |
|---|---|
| `--shadow-sm` | Subtle lift (chips, inputs). |
| `--shadow-md` | Cards, popovers. |
| `--shadow-lg` | Modals, the floating now-playing bar. |

## Radii

| Token | Value | Use |
|---|---|---|
| `--r-xs` | `4px` | Checkboxes, tiny chips. |
| `--r-sm` | `6px` | Buttons, inputs. |
| `--r-md` | `10px` | Cards, rows. |
| `--r-lg` | `14px` | Large cards, the head player. |
| `--r-xl` | `20px` | Hero panels. |

## Spacing & density

Padding is derived from a single `--density` multiplier, so the operator's
density preference (Settings → display) rescales the whole UI by changing one
value rather than re-spacing every component.

| Token | Value | Use |
|---|---|---|
| `--density` | `1` | Density multiplier (compact / comfy / regular set this). |
| `--pad-1` | `calc(8px * var(--density))` | Tight padding / small gaps. |
| `--pad-2` | `calc(12px * var(--density))` | Default control padding. |
| `--pad-3` | `calc(18px * var(--density))` | Card padding. |
| `--pad-4` | `calc(28px * var(--density))` | Section padding. |

## Typography

Self-hosted font stacks (no external font CDNs) with broad CJK / Devanagari
fallbacks so localized common names render.

| Token | Stack head | Use |
|---|---|---|
| `--font-display` | Instrument Serif → Source Serif 4 → Georgia | Editorial headlines (hero phrases, report titles). |
| `--font-ui` | Inter Tight → Noto Sans | Body and UI text. |
| `--font-mono` | JetBrains Mono → IBM Plex Mono | Times, codes, numeric tables. |

## Legacy aliases

A handful of tokens from before the v3 redesign are kept as **aliases** of the
current set, so pre-redesign partials stay visually consistent without being
rewritten. Prefer the modern token in new code.

| Legacy alias | Resolves to |
|---|---|
| `--bg-card` | `--surface` |
| `--bg-hover` | `--surface-2` |
| `--text` | `--fg` |
| `--text-muted` | `--fg-3` |
| `--accent` | `--moss` |
| `--accent-dim` | `--moss-ink` |
| `--success` | `--moss` |
| `--warning` | `--dawn-ink` (light) / `--dawn` (dark) |
| `--danger` | `--rare` |
| `--radius` | `--r-md` |
| `--input-bg` | `--surface` |
| `--conf-high-bg` / `--conf-mid-bg` / `--conf-low-bg` | `--moss-soft` / `--dawn-soft` / `--rare-soft` |

`--accent-rgb` (a comma-separated RGB triple) is the one non-OKLCH value, kept
for the few rules that still need `rgba()` with a variable alpha.

## Adding or changing a token

The token set is deliberately closed (see the v3 spine
[protect list](https://github.com/tomtom215/BirdNet-Behavior/blob/main/docs/design/handover/v3_spine/IMPLEMENTATION_PLAN.md) —
"OKLCH token set: zero new tokens"):

- **Build new UI from existing tokens.** Compose them into new CSS *classes*;
  that is the additive path and needs no token change.
- **A genuinely new token is a design decision.** Justify it in the PR, give it
  values for **both** themes (keep light and dark in lockstep), and prefer
  extending a hue family (`-soft` / `-ink`) over inventing a new hue.
- **Never hard-code a colour or pad** in a component — read the token, so a
  future theme or density change stays a one-line edit.
