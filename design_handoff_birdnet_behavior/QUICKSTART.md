# Quick-start for Claude Code

You're picking up a polished design handoff for **BirdNet-Behavior** — a Rust acoustic-bird-classification system on Raspberry Pi.

## Read first

1. **`README.md`** — the full handoff. Covers all 26 screens, design tokens, interaction rules, state needs, and implementation options.
2. **`source/lib/tokens.css`** — every color, spacing, typography, shadow, radius value. Treat as canonical.
3. **`source/BirdNet-Behavior.html`** — load locally (`python3 -m http.server 8000`) to see every screen scroll past.

## Then

4. Decide the integration approach (HTMX-enhanced templates vs. SPA — see "Implementation guidance" in README).
5. Reproduce design tokens in the target codebase first. Don't start screens until the palette + type + spacing scale exist.
6. Build atoms second (`<Sparkline>`, `<Stat>`, `<SpeciesAvatar>`, `<BirdPhoto>`, etc. — listed in README "Shared atoms").
7. Then screens, in the order they're delivered in the section structure.

## Don't forget

- **`prefers-reduced-motion`** disables: feed pulse, kiosk fade, aurora, phone-rise, sonar rings.
- **Dark mode is not an invert** — see README. Hue 250, cool neutral. Accent chroma bumps from 0.09 → 0.18.
- **Co-occurrence matrix labels** are 4-letter codes, NOT rotated common names. (Earlier iteration; common names collide.)
- **Kiosk** auto-rotates every 9s. **Night Mode** is a separate display, gated by quiet hours.
- **Wikipedia CC BY-SA** attribution is mandatory on bird photos.
- **The prototype is the spec.** When in doubt, match it pixel-for-pixel.
