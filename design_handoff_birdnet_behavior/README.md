# Handoff: BirdNet-Behavior Dashboard & Admin UI Redesign

> **A complete design package for the BirdNet-Behavior Rust application** — a Raspberry Pi acoustic bird classification system with behavioral analytics. This handoff covers a full dashboard redesign plus first-run, kiosk, analytics, history, and admin surfaces.

---

## Repository

Target repo: **`tomtom215/BirdNet-Behavior`** — a Rust single-binary application using:

- **`axum`** web server with HTMX templates (current implementation)
- **`birdnet-web`** crate owns the HTTP/WebSocket layer
- **SQLite (rusqlite)** for detections, **DuckDB** for behavioral analytics
- Built static binary deployed to `aarch64-unknown-linux-gnu` (Raspberry Pi 5/4B) and `x86_64-unknown-linux-gnu`

This redesign should be implemented as a progressive enhancement of the existing HTMX templates in `crates/birdnet-web/templates/` (or migrated to a richer interactive layer if the team chooses) — see the **Implementation guidance** section below.

---

## About the Design Files

The `source/` folder contains a **React + JSX prototype** of every screen. **These are design references — not production code to ship.** They were authored as inline-Babel HTML to make iteration fast, and they intentionally over-fit to look correct in a browser.

Your task is to **recreate these designs in the target codebase's environment** — the existing HTMX + Tera/Askama template stack, or a richer interactive layer (HTMX + Alpine.js, or a Yew/Leptos SPA, or a separate React/Svelte frontend served by `axum::Router`) — using the project's established Rust patterns. Pick what fits the team's roadmap.

Where the prototype uses inline styles, **port those values into the codebase's design system** (CSS custom properties, Tailwind config, or a stylesheet — whatever already exists). Where the prototype uses `window.BNB.SPECIES` mock data, wire to the real `birdnet-db` queries.

---

## Fidelity

**High-fidelity.** Pixel-perfect mockups with final colors (in OKLCH), typography, spacing, and interactions. The developer should recreate the UI faithfully — every spacing value, hue, font size, radius, and shadow is intentional.

The design tokens are documented exhaustively in `source/lib/tokens.css`. **Treat this file as the canonical reference** for the visual system.

---

## What's in this package

```
design_handoff_birdnet_behavior/
├── README.md                       ← this file
├── source/                         ← the design prototype
│   ├── BirdNet-Behavior.html       ← entry point, loads everything
│   ├── lib/
│   │   ├── tokens.css              ← THE design system (read first)
│   │   ├── components.jsx          ← shared atoms (Sparkline, Stat, BirdPhoto, etc.)
│   │   ├── data.jsx                ← mock species / heatmap / migration data
│   │   └── app.jsx                 ← root + section/frame scaffolding + TOC
│   ├── screens/                    ← one .jsx per screen — 26 total
│   │   ├── onboarding.jsx
│   │   ├── dashboard.jsx
│   │   ├── today.jsx
│   │   ├── heatmap.jsx
│   │   ├── dawn-chorus.jsx
│   │   ├── co-occurrence.jsx
│   │   ├── migration.jsx
│   │   ├── spectrogram.jsx
│   │   ├── species-list.jsx
│   │   ├── species-detail.jsx
│   │   ├── gallery.jsx
│   │   ├── recordings.jsx
│   │   ├── life-list.jsx
│   │   ├── history.jsx
│   │   ├── trends.jsx
│   │   ├── year-in-review.jsx
│   │   ├── weekly-report.jsx
│   │   ├── system.jsx              ← system health + admin/detection settings
│   │   ├── audio-settings.jsx      ← RTSP + USB microphone management
│   │   ├── quarantine.jsx
│   │   ├── notifications.jsx
│   │   ├── migrate.jsx             ← BirdNET-Pi import wizard
│   │   ├── backup-recovery.jsx
│   │   ├── kiosk.jsx               ← wall display, 7 stations + night mode
│   │   └── mobile.jsx              ← phone showcase (3 phone screens)
│   ├── design-canvas.jsx           ← (unused — kept for reference)
│   ├── tweaks-panel.jsx            ← live theme/density/demo controls
│   ├── ios-frame.jsx               ← (unused — kept for reference)
│   └── image-slot.js               ← drag-drop photo upload web component
```

To inspect the prototype locally:
```bash
cd source/
python3 -m http.server 8000
# open http://localhost:8000/BirdNet-Behavior.html
```

---

## Design tokens — the visual system

All values are defined in `source/lib/tokens.css` as CSS custom properties on `:root` (light) and `.theme-dark`. **Reproduce this token map in the target codebase before building any screen.**

### Color philosophy

The palette is built on **OKLCH** for perceptually uniform color. Two semantic accents:

- **Moss** (hue 150) — "alive / detected / present / OK" — every active/live state, every primary action
- **Dawn** (hue 60) — "time-of-day / activity / warm" — sunrise/sunset, daytime activity, warning states
- **Rare** (hue 28) — "first-of-station / alert" — rare-bird detections only

Plus a deep blue-neutral **Night** for after-hours kiosk mode.

### Light theme

```css
--bg:        oklch(98.5% 0.004 80);   /* warm off-white, never pure white */
--bg-2:      oklch(96.5% 0.005 80);
--surface:   oklch(100% 0 0);
--surface-2: oklch(97.5% 0.004 80);
--border:    oklch(90% 0.006 80);
--border-2:  oklch(82% 0.008 80);
--hairline:  oklch(94% 0.005 80);

--fg:   oklch(22% 0.008 70);
--fg-2: oklch(40% 0.008 70);
--fg-3: oklch(55% 0.008 70);
--fg-4: oklch(70% 0.008 70);

--moss:      oklch(55% 0.09 150);
--moss-soft: oklch(92% 0.04 150);
--moss-ink:  oklch(35% 0.09 150);

--dawn:      oklch(68% 0.12 60);
--dawn-soft: oklch(94% 0.05 65);
--dawn-ink:  oklch(42% 0.12 55);

--rare:      oklch(58% 0.16 28);
--rare-soft: oklch(94% 0.05 28);
```

### Dark theme

**Important:** dark mode is NOT a desaturated invert of light. It's a cool observatory black (hue 250, very low chroma) with **brighter, more saturated accents** that glow against the neutral surface. Earlier brown-tinted dark modes felt muddy on screen — this palette was tuned after multiple iterations.

```css
--bg:        oklch(12% 0.008 250);
--bg-2:      oklch(15% 0.008 250);
--surface:   oklch(18% 0.010 250);
--surface-2: oklch(22% 0.010 250);
--border:    oklch(28% 0.014 250);
--hairline:  oklch(24% 0.012 250);

--fg:   oklch(97% 0.004 240);
--fg-2: oklch(80% 0.008 240);
--fg-3: oklch(60% 0.010 240);

/* Accents bump from chroma 0.09 → 0.18 — they need more punch on dark */
--moss:      oklch(78% 0.18 150);
--moss-ink:  oklch(90% 0.16 150);

--dawn:      oklch(82% 0.18 65);
--dawn-ink:  oklch(92% 0.16 60);

--rare:      oklch(74% 0.20 25);
```

Dark cards get a subtle linear gradient: `linear-gradient(180deg, oklch(20% 0.010 250) 0%, oklch(17% 0.010 250) 100%)`. Streamgraphs / ridges / chord ribbons get a CSS `filter: saturate(1.25) brightness(1.30)` boost in dark mode.

### Typography

Three families, loaded from Google Fonts:

- **Display** — `"Instrument Serif"` (fallback Source Serif 4, Newsreader, Georgia, serif) — used for big numbers, species names, statement headlines. Italics carry editorial emphasis.
- **UI** — `"Inter Tight"` — all body, labels, controls. Letter-spacing `-0.005em`.
- **Mono** — `"JetBrains Mono"` (fallback IBM Plex Mono, ui-monospace) — timestamps, frequencies, device IDs, technical metadata. Always `font-variant-numeric: tabular-nums`.

Display headlines use `letter-spacing: -0.025em` to `-0.04em` depending on size, and `font-weight: 400` (the serif's elegance comes from the cut, not weight). Body is `font-weight: 400`, labels `500`, big emphasis `600`.

Mandatory: `font-feature-settings: "ss01", "cv11"` on the UI font and `"ss02"` on mono — these are the variants that look intentional.

Headings: `text-wrap: balance` on display, `text-wrap: pretty` on body paragraphs.

### Spacing & density

The CSS custom property `--density` (default `1`, compact `0.78`, comfy `1.15`) multiplies every padding value:

```css
--pad-1: calc(8px  * var(--density));
--pad-2: calc(12px * var(--density));
--pad-3: calc(18px * var(--density));
--pad-4: calc(28px * var(--density));
```

Build the codebase's spacing scale to honor this. Users should be able to flip a Compact / Regular / Comfy toggle in admin settings.

### Radii & shadows

```css
--r-xs: 4px;  --r-sm: 6px;  --r-md: 10px;  --r-lg: 14px;  --r-xl: 20px;

--shadow-sm: 0 1px 2px oklch(0% 0 0 / .04), 0 0 0 0.5px oklch(0% 0 0 / .04);
--shadow-md: 0 4px 14px oklch(0% 0 0 / .06), 0 0 0 0.5px oklch(0% 0 0 / .04);
--shadow-lg: 0 18px 40px oklch(0% 0 0 / .08), 0 0 0 0.5px oklch(0% 0 0 / .05);
```

The "hairline" shadow component (`0 0 0 0.5px`) replaces traditional borders on cards — it gives a crisper edge at any zoom level.

---

## Audience principles

Every screen was designed to serve **two users in one interface**:

1. **Hobbyist** — set up a Pi once, look at the dashboard, get a delight reaction
2. **Researcher** — PhD-level ornithology, needs methodological rigor

These conflict less than you'd think. The patterns:

- **One plain-English headline at the top.** Every screen leads with a statement: "The yard is singing." "When the yard is alive." "You're listening." The dense numbers/charts sit underneath.
- **Progressive disclosure.** Advanced controls are gated behind `<details>` accordions with `adv` pills. SF threshold, sensitivity, quality filter — all collapsible. Tooltips (`<HelpDot>` in `components.jsx`) explain methodology without crowding the layout.
- **Eyebrows on everything.** 10.5px uppercase letter-spaced 0.10em labels orient the user before they parse the data.
- **Plain-English reads.** Every analytical screen offers a paragraph translating the chart ("When you hear Cardinal, there's a 71% chance you'll hear Chickadee within five minutes. They share habitat and feeding times.").

---

## Screens / Views

The 26 screens are grouped into 8 sections in `lib/app.jsx`. Below, every screen with its purpose, layout, and key components.

### 00 — Onboarding (`screens/onboarding.jsx`)

**Purpose:** First-run wizard. 90 seconds, five steps, foolproof for a non-technical user.

**Layout:** Full-bleed, no chrome. Sticky stepper at top. Two-column content area (text left, illustration/preview right). Bottom navigation row (← Back · Continue →).

**Steps:**
1. **Welcome** — animated sonar SVG with concentric rings and bird-glyph silhouettes orbiting a Pi outline. Three bullets ("No accounts · Set once · Always tweakable").
2. **Where** — lat/lon entry with `auto-detect (ipapi.co)` and manual options; right-side mock map with topographic contours, station bullseye marker, dashed 100 km radius. Success state in moss-soft: `✓ Boston, MA · 247 species expected this time of year · sunrise 5:21 AM`.
3. **How it hears** — four audio options as large radio cards: auto-detected USB device (recommended pill), second USB, "Add an RTSP camera", "Watch a folder". Each card shows live mic level as 24-bar VU meter.
4. **Who gets notified** — four mode cards (None · Rare only [recommended] · Daily digest · Everything) plus collapsible "Pick channels now" with 12 channel chips.
5. **Done** — "You're listening." headline, station summary card (4 rows: Location / Microphone / Notifications / Dashboard URL), preview card showing the "warming up" dashboard with calibrating waveform and BirdNET+ V3.0 badge.

**Animations:** Sonar rings use SVG `<animate>` with `r` and `stroke-opacity` over 4–6s loops. Step transition: 200ms ease fade.

### 01 — Dashboard (`screens/dashboard.jsx`)

**Purpose:** The right-now view. Live detections, today's pulse.

**Layout:**
- TopNav (Dashboard active)
- Hero strip — 1.25fr / 1fr grid:
  - **Left:** eyebrow ("Right now · Thursday May 22"), 64px display headline "The yard is **singing**." (italic moss-ink emphasis on "singing"), 15px body paragraph, four pills below (recording + duration · sunrise · sunset · coordinates)
  - **Right:** HeroPulse card — animated live waveform (canvas, 80px tall, 90 bars with bell-curve envelope), readout row (SNR / sample rate / inference time), plus a small DayClock SVG (80px circle with night wedge, dawn band, current-time hand) and "Dawn chorus window · 1h 18m left" copy
- Stat row — 4 columns inside a rounded card: **Detections 24h · Species 24h (with first-of-year chips) · Rare today (with awaiting-review subline, rare-colored sparkline) · Listening (with 48-segment recording stripe instead of a sparkline)**
- Two-column work area (1.35fr / 1fr):
  - **Live feed card** — section header + filter pills + 10 rows. Each row: 62px timestamp / 32px species avatar / species name+sci+rare-pill / MiniWaveform / ConfBar / Play button. Fresh row gets moss-tinted background + border, rises in with a 320ms cubic-bezier animation.
  - **Right rail (top):** Top species mini-list (6 rows with sparklines per row)
  - **Right rail (bottom):** Compact 7-day hour×day heatmap

**Live behavior:** Every 1.8–6.5s (depending on Tweaks "demo" state quiet/busy/dawn), a new detection prepends to the feed, the prior rows shift down, and the freshly-prepended row pulses moss for ~600ms before settling.

### 02 — Today (`screens/today.jsx`)

**Purpose:** Searchable, filterable detection log for today.

**Layout:**
- TopNav (Today active) + header with search input, ≥0.80 / All species / Range pills, Export CSV
- **DayStrip** — full-width 120px-tall SVG showing every detection as a colored dot on a 24-hour timeline. Background shows sunrise/sunset bands (`var(--night)` 0.05 alpha), per-hour histogram bars (`var(--moss-soft)`), sun/moon markers with timestamps, and a black "now" pill at current hour
- Stat strip in header — "06:47 peak hour · 34 in dawn chorus · 1 rare"
- Table — 7-column grid: Time · Avatar · Species name+sci · MiniSpecRow (stylized spectrogram strip) · Confidence bar · Play (with duration) · Actions (Lock/Re-label/Delete)
- Pagination footer

### 03 — Heatmap "When the yard is alive" (`screens/heatmap.jsx`)

**Purpose:** Behavioral analytics — when activity peaks and which species dominate.

**Two stacked visualizations:**

1. **Streamgraph** (top) — full-width SVG, today's half-hour species composition with **centered-baseline** wiggle. Each species is a colored band. Sunrise/sunset dashed lines with labels. Legend in the section header shows the 7 contributing species with color swatches.

2. **Hour × day-of-week mosaic** (bottom-left, 7 rows × 24 cols) — **single dominant-species color per cell**, not stacked bars. Background interpolates from `var(--surface-2)` to `color-mix(in oklch, dominant.color 90%, var(--surface))` as intensity climbs. Right-edge ticks show co-occurring species count. Peak intensity (=5) shows a small dot in the top-left corner.

   - Hover any cell → right-side rail updates: shows day + hour, intensity rating, top 3 species with share percentages
   - Below the mosaic: horizontal histogram of hourly totals across all 7 days

3. **Side rail** (right of mosaic) — CellDetail panel + "Weekly totals" bar list by day

4. **Insight cards row** (bottom) — 4 cards: Peak hour · Quietest day · Loudest species · Anomaly (with rare-toned styling)

### 04 — Dawn Chorus (`screens/dawn-chorus.jsx`)

**Purpose:** Circadian polar plot showing when each species sings across 24 hours.

**Layout:** Two columns. Left card holds the polar SVG (max 540px square): concentric ribbons per species, each centered on a baseline circle, with night-wedge background fill, sunrise/sunset sun-markers, hour-tick marks every 3h, and a dashed "current time hand". Right card has per-species rows with `CircadianRing` mini gauges.

### 05 — Co-occurrence Matrix (`screens/co-occurrence.jsx`)

**Purpose:** Spearman ρ between every species pair within a 5-minute rolling window.

**Important:** column headers use **4-letter codes only** (not rotated common names) with colored species dots above them. Row headers use full common name + avatar. Earlier iterations rotated full names — they collided. **Do not bring rotated text back.**

Hovered pair lights up; the right-side panel shows the pair detail with overlay hourly chart, stat row (ρ / co-detections / median Δt), and a plain-English read paragraph.

### 06 — Acoustic Network (`screens/co-occurrence.jsx` — `AcousticNetwork`)

**Purpose:** Same data as the matrix, drawn as a chord diagram.

**Layout:** 720×720 SVG. Each species gets an outer arc proportional to its total connectedness. Ribbons connect pairs (only ρ ≥ 0.20), with gradient fills between the two species' colors. Labels ride along the arc (tangent-rotated, auto-flip on the left half) — never collide. Right rail lists strongest pairs with a hover→matrix-light interaction.

### 07 — Migration Phenology (`screens/migration.jsx`)

**Purpose:** Year-long ridgeline of weekly abundance for migratory species.

**Layout:** Headline + 4 stat tiles (First-of-year arrivals · Peak diversity · Earliest vs. 2024 · Still expected) above the chart (stat-first, not buried below). Chart: 1240×360 SVG with one ridge per migratory species, each normalized to its own peak, fills are gradients (saturated near curve fading toward baseline), peak markers with dashed stems, spring/fall season bands shaded behind. Today line cuts across with a pill. Below: weekly-diversity stacked bar chart (52 weeks, color-banded moss/dawn/fg-3 for spring/fall/baseline). 3-card row at bottom: Just arrived · Currently peaking · Missing.

### 08 — Live Spectrogram (`screens/spectrogram.jsx`)

**Purpose:** The 30-second window — what the Pi hears right now.

**Layout:** Main canvas (240×80, scaled, pixelated rendering) with frequency axis on the left (0/3/6/9/12 kHz with tick lines and rotated "kHz" unit label). **Detection boxes travel with the scrolling spectrogram** — they appear at the right edge, grow as the chirp emits, then drift left in lockstep with the audio band. Each box is colored by species (rare = rare hue). Below the spectrogram: live waveform canvas (44px tall). Bottom: time axis (-30s → now). Side panel: live detection tally + per-species color-dot list + quality filter readout.

**Implementation note:** the spectrogram is `<canvas>` with `imageRendering: pixelated`. Frame loop: shift left by 1 column (`ctx.getImageData(1, 0, W-1, H)` + `putImageData(prev, 0, 0)`), wipe right column, render any in-flight chirp pixels into the new right column, advance boxes' `right` offset. Boxes are absolute-positioned DOM divs over the canvas — translates via `left` percentage as `right` grows.

### 09 — Species List (`screens/species-list.jsx`)

**Purpose:** Searchable, sortable browse view for every detected species.

**Layout:** Search input · sort picker (Most heard / A→Z / Newest) · filter pills (All / Today / Rare) · Export CSV. Stat strip with 5 metrics (Total species · Active today · First-of-year · Rare · Median confidence). Then a 9-column table: rank / avatar / common+sci / All-time count / 14-day sparkline / Conf bar / First seen / Last heard / Status pill (active/rare/—).

### 10 — Species Detail (`screens/species-detail.jsx`)

**Purpose:** Single-species deep view.

**Hero:** 440px-wide full-bleed photo column on the left (via `<BirdPhoto>`), info on the right (status pills, 56px display common name, italic sci, 14px description from Wikipedia, 4-tile stat row). Floating frosted-glass "Last heard 14 minutes ago" badge in the photo's bottom-left.

**Below hero (3 columns):** Hourly activity bars · 12-week activity grid (replaces the earlier sparkline — a GitHub-contribution-style grid colored by species hue) · Companion species list (top 3 by ρ).

**Bottom:** Recordings strip — 5 clip cards with stylized spectrogram thumbs.

### 11 — Gallery (`screens/gallery.jsx`)

**Purpose:** Photo card grid.

5-column responsive grid of cards. Each card: 4:3 photo (via `<BirdPhoto>`), rare/active pill in top-left, 4-letter code in top-right, common+sci name + count + sparkline below.

### 12 — Recordings (`screens/recordings.jsx`)

**Purpose:** Listen to detection clips.

Two-pane: clips list (avatar / common name / time+duration+conf / mini-spectrogram / file size, selected row highlighted with moss left-border) and a player pane with large spectrogram (detection box overlay, frequency labels), waveform, scrubber + 0.5×/1×/2×/loop transport controls, and a file-path footer.

### 13 — Life List (`screens/life-list.jsx`)

**Purpose:** Birding journal — every species, once.

**Year Tape** — full-width 1380×130 SVG. Every lifer is a colored dot with stem on a 365-day axis. Spring (DOY 60–135) and fall (DOY 240–310) bands are tinted moss/dawn. Today dashed line. Stem height = species' detection count. Rare lifers get a halo ring. Plus a per-month counts bar strip in the section header.

**Two columns below:** Journal entries grouped by month (dense rows with italic quote notes) + a right column with "Latest lifer" photo, milestones tile, and "Likely next" species list with eBird probabilities.

### 14 — History calendar (`screens/history.jsx`)

**Purpose:** 8-week calendar browser.

Calendar grid: 8 rows of weeks, each with weekday-of-month label, then 7 day-tiles. Day tiles are color-intensity-coded by detection count; selected day gets `outline: 2px solid var(--fg)`. Each tile shows the day number, the count, and a row of tiny species-color dashes representing the day's diversity. Right column: DayDetail pane with hourly bars + top-5 species for the selected day.

### 15 — Trends & Comparisons (`screens/trends.jsx`)

**Purpose:** Week / Month / Year period-over-period analytics.

Period picker (Week · Month · Year segmented control). Three header cards:
- **Big comparison** — current period number (56px display), big delta pill with arrow + percentage, mini before/after dual-line chart (current solid moss, previous dashed fg-3)
- **Diversity compare** — species count vs. prior period with stacked bars
- **Novelty** — gained (moss pill chips with avatars) and lost (dashed-border dim chips)

**Year-on-year overlay chart** — 1340×240 SVG with three lines: current (solid moss with filled area), prior (dashed fg-3), 3-year average (thin dawn). Today marker for the year view.

Two cards below: **Species cohort** (per-species dual-bar with delta %) and **On This Day** (entries from prior years with italic note quote).

**Long-term trends** — 10 species in a 2-column grid, each with avatar + 60-week colored sparkbar + recent-trend sparkline. "This year start" and "today" vertical markers on each row.

### 16 — Year in Review (`screens/year-in-review.jsx`)

**Purpose:** Editorial annual recap. Celebratory.

Cinematic 96px display headline ("A year of *listening*.") with eyebrow "YEAR IN REVIEW · 2025 · STATION #001". Four big-number tiles (Detections / Species / Hours listening / Rare confirmed). Full **52-week year tape** — every day as a moss-intensity cell. Top species leaderboard. Lifers list. **Milestone strip** with 4 specific dates (Day one · Earliest warbler · Best dawn chorus · First Barred Owl). Seasonal donut + dawn chorus length chart. Closing card with "Likely still to come" species chips and Export-as-PDF action.

### 17 — Weekly Report (`screens/weekly-report.jsx`)

**Purpose:** Newspaper-style Sunday recap.

Centered masthead ("The Backyard Bulletin · Issue No. 32") with a 76px headline. Two-column body: left has lead story (2-column body text inside), 4-cell big-number row, leaderboard. Right has First-of-year sidebar, daily activity bars, an editorial note pushing to the quarantine queue, and a methodology block.

### 18 — System Health (`screens/system.jsx` — `SystemHealth`)

**Purpose:** Pi health dashboard.

Top row: **SystemPulse** (gradient card with "Pi 5 · 8 GB · running cool" headline, 60-min CPU sparkline) + **4 large 3/4-arc gauges** (CPU / Memory / Temperature / Disk) — 120×100 SVG each with 28px display number in the center. Below: ResourceChart (24h CPU+Memory line/area) and self-check probe list (12 rows with pass/warn/err glyphs).

### 19 — Audio Settings (`screens/audio-settings.jsx`)

**Purpose:** Microphone management. THIS IS THE PRIMARY SUPPORT BURDEN — make it bulletproof.

**3-column shell:** sidebar (Sections list with Audio highlighted) / main / right rail.

**Main:** Source list — three rows showing USB and RTSP microphones. Each row is a 7-column grid: USB/RTSP icon · name+device path+detail · **Level meter (with SNR)** · **Uptime (e.g. "14 d 02 h · stable")** · **Last detection (e.g. "14 s ago · Northern Cardinal" in moss when recent)** · 24h detection count · actions including `▸ tune`.

**Click `▸ tune`** to expand into a full audio control panel with:
- **Gain slider** -12 → +24 dB with a zero-mark indicator
- **Sample rate** picker (8/16/22.05/44.1/48 kHz)
- **Channels** picker (mono/left/right/stereo)
- **Bit depth** picker (16/24-bit PCM)
- **Pipeline toggles** — high-pass filter, DC offset removal, auto-gain control, RTSP keepalive
- **Name + position** editable fields, schedule, Discard/Apply

**Below the list:** AddRtspCard — three-step horizontal wizard (URL → Auth → Label) with a locked `rtsp://` prefix on the URL input, live reachability pill + sniffed audio properties, plus a live preview row with waveform + "Listen for 10s" + "Add to sources".

**Bottom:** collapsible `<details>` for researcher options (transport tcp/udp/auto, reconnect backoff, parallel vs round-robin, drift correction).

**Right rail:** Combined input total · per-source level mini-bars · three plain-English "Common pitfalls" tips.

### 20 — Admin Settings (`screens/system.jsx` — `AdminSettings`)

**Purpose:** Detection thresholds & rules.

3-column layout: sections sidebar (with status dots for sections needing attention — e.g. MQTT has a dawn dot when disconnected) / main form / right rail. Main form has 5 fields: confidence threshold (slider), sensitivity (slider, `adv` pill), species frequency filter (toggle + read-only SF threshold), quality pre-filter (toggle + select), rare-bird quarantine (toggle + queue status). Each field has a label, helper text in `bnb-meta`, and the control. Right rail: **threshold preview** (estimated detections/day at each cutoff as horizontal bars, current highlighted) + **birdnet.conf code preview** showing the persisted values.

### 21 — Quarantine (`screens/quarantine.jsx`)

**Purpose:** Rare-bird review queue.

Two-column. **Queue list** (left, 320px wide): one row per pending detection with avatar, species name, time, priority pill, note, confidence bar. Selected row gets moss left-border. **Review pane** (right): species identity at top, info pills (first-ever-at-station/eBird code/range), then a 2-column evidence area — left has a custom-drawn owl-call spectrogram + waveform + audio scrubber transport, right has reference recording from Macaulay Library, "Top alternative ID" list (Great Horned Owl 0.34, Screech-Owl 0.22, etc.) with mini bars, and a context block (weather + last detection). **Decision strip** at bottom: Reject · Re-label as… · Save notes · Approve.

### 22 — Notifications (`screens/notifications.jsx`)

**Purpose:** Channel management.

Stat strip (4 metrics) above. Two-pane: **Channels** (6 rows with kind-icon, name, target, rule, send count + last sent, Test/Edit/⋯ actions) and **Recent events** (7 rows with kind-icon, subject, channel+when, delivered pill). Each channel kind has a colored glyph: telegram (paper-plane blue), email (envelope warm red), MQTT (≋ moss), webhook (`{}` neutral), slack (# purple), discord (◉ violet).

### 23 — Migrate from BirdNET-Pi (`screens/migrate.jsx`)

**Purpose:** Safe read-only import of legacy SQLite database.

4-step stepper. Main pane has detected file row, **moss-toned Safety Bullets** ("read-only / transactional / dedupe-safe" + dawn-toned "stop BirdNET-Pi first" warning), **Schema validation rows** mapping each legacy column to its new location with pass/skip glyphs, and top-5 source species preview. Right rail: data quality report + merge explanation + help links.

### 24 — Backups, Restore & System Admin (`screens/backup-recovery.jsx`)

**Purpose:** The full sysadmin surface.

Stat strip (Last backup / Retained / Backup size / Restore tested). **Manual upload + export section** (2 columns) — left has dashed-border drop zone for `.bnb-backup` files with signature verification footer; right has 6 export options as labeled rows (Full bundle · Database · CSV · WAV · Settings JSON · Logs). **Snapshot list** — 8 rows with auto/manual dots, today highlighted in moss, pre-upgrade snapshots tagged. **Right rail:** Backup destinations (Local/S3/SMB/Email toggles) · Restore card · System update card. **Storage breakdown** — 4 tiles (SQLite/DuckDB/Recordings/Wikipedia cache) with progress bars; recordings tile uses dawn tone since it grows. **Retention controls** — 3 fields with chip-style values and a "change" affordance. **Operations log viewer** with color-coded info/warn/error rows in a monospace pane. **Danger zone card** (red border) — 4 destructive actions, each confirmation-gated: Reset settings / Wipe recordings / Factory reset / Uninstall.

### 25 — Kiosk Mode (`screens/kiosk.jsx`)

**Purpose:** Wall-mounted display. Auto-rotates every 9 seconds between **seven stations**, plus a **Night Mode** for after-hours.

**Day stations:**
1. **Now Detection** — 144px display species name + portrait + confidence stats
2. **Daily Pulse** — "912 calls heard from 15 species" big headline + 4 big-stat tiles
3. **Circadian Sky** — sun-arc SVG with detection-density blobs underneath, chorus-ends countdown headline
4. **Constellation** — species as glowing stars in clusters (Residents / Visitors / Rare), Delaunay-ish connections, twinkling background stars
5. **Soundscape Bloom** — radial flower-style chart, each petal = species
6. **Living Spectrum** — fullscreen flowing canvas spectrogram with floating species labels
7. **Feed Ticker** — last 10 detections scrolling

**Aurora** — SVG animated wave fills behind everything (moss + dawn hues, 14–18s loops).

**Night Mode** — single station: moon SVG with craters and pulsing glow, 96px clock, one-line editorial sentence, three small stat tiles, "Display dimmed · will resume at 06:00" footer.

**Controls panel** — `⚙ controls` button top-right opens a glass-backed side panel with: Night-mode toggle, quiet hours (22:00–06:00), auto-advance rate, jump-to-station picker, output settings (resolution / brightness / burn-in protect: pixel shift).

### 26 — Mobile Showcase (`screens/mobile.jsx`)

**Purpose:** Phone companion app — three states shown together.

Three iPhone-style frames on a radial-gradient dark canvas with sonar background rings. Each phone is a 320×690 rounded device with deep gradient bezel + dynamic-island bump, displaying scaled-down `PhoneDashboard`, `PhoneSpecies`, or `PhoneAlert`. Phones animate in with staggered rise-up + slight tilt. Captions below.

---

## Shared atoms (`lib/components.jsx`)

These are the building blocks used everywhere. Reproduce in the target codebase:

- **`<Sparkline data width height accent>`** — line + area mini-chart, max-normalized
- **`<MiniBars data width height accent>`** — opacity-graded bar chart
- **`<SpeciesAvatar sp size>`** — circular 4-letter-code chip in the species' color (color-mix at 22% over surface)
- **`<ConfBar value width>`** — confidence bar 0–1, color thresholds: >0.9 moss, >0.75 dawn, else fg-3
- **`<CircadianRing data size accent>`** — 24-segment ring with opacity-mapped activity
- **`<Screen children padded>`** — root scaffold for every screen
- **`<TopNav active>`** — sticky nav with Dashboard / Today / Species / Heatmap / Analytics / Life list / System anchors, right-side status pills
- **`<BrandMark size>`** — sound-wave circle SVG (the logo)
- **`<Stat label value sub accent size>`** — stat block; sizes sm/md/lg drive the display number from 20 / 28 / 36 px
- **`<SectionHeader eyebrow title action>`** — inside-card heading
- **`<HelpDot>`** — `?` glyph that reveals a dark tooltip on hover (the methodology-disclosure mechanism)
- **`<BirdPhoto sp idx slotId height attribution>`** — renders Wikipedia photo with hover-reveal `<image-slot>` overlay for user-uploaded override, plus decorative silhouette fallback if photo fails to load

---

## Interactions & Behavior

### Live data behaviors

- **Dashboard feed** — new detection prepends every 1.8/3.2/6.5s (dawn/busy/quiet). Fresh row pulses moss background for 600ms then transitions back. `bnb-rise` keyframe: 320ms translateY(-6px) → 0 with cubic-bezier(.2,.7,.2,1).
- **Spectrogram** — `requestAnimationFrame` loop shifts canvas left by 1 column, paints new data into the rightmost column. Detection boxes are DOM elements that grow during chirps and drift left after. Every ~30 frames, re-read theme to pick light/dark background.
- **Kiosk** — `setInterval(9000)` advances station index. New station mounts under a `kiosk-fade` keyframe (600ms scale .99 → 1).
- **Phone Showcase** — phone-rise keyframe (800ms, delays 0/120/240ms).
- **Aurora** — SVG `<animate>` on `d` attribute, 14s and 18s loops.

### Hover/focus rules

- All hovers fade in 120–150ms ease
- Card hover: no shadow change in this design — keep cards static
- Pills clickable: hover shifts background tone
- Co-occurrence cells: hovered cell gets 1.75px inset outline `var(--fg)`; non-hovered row/column dims to opacity 0.55

### Tweaks panel (`tweaks-panel.jsx`)

Bottom-right floating glass panel. Controls: theme (light/dark), accent (moss/ocean/heath/ember — these rotate the hue of `--moss` and `--dawn` while preserving chroma), display font, density, demo state (quiet/busy/dawn). Values persist via `window.parent.postMessage` to the host. **For production, replace this with admin/settings/personalization controls** — the panel itself is an authoring tool.

### Theme switching

When `.theme-dark` is toggled on `<html>`:
1. All CSS custom properties flip
2. The dark-mode `filter: saturate(1.25) brightness(1.30)` SVG enhancement kicks in for streamgraph/ridge/mosaic-bar elements
3. Cards switch to a gradient background
4. Display numerals get a subtle `text-shadow` glow

**Implement system-preference detection:** `prefers-color-scheme: dark` should set the default. Users can override and persist.

---

## State Management

The prototype keeps everything in component state. For production, **wire to the existing crate boundaries:**

- `birdnet-db` → SQLite OLTP queries (detection feeds, species lists, today's log, calendar data)
- `birdnet-behavioral` → DuckDB OLAP queries (co-occurrence ρ, dawn chorus aggregates, migration weekly index, year-on-year comparisons)
- `birdnet-timeseries` → trends, peaks, gaps, sessions
- `birdnet-integrations` → notification channel state, BirdWeather sync
- `birdnet-scheduler` → sunrise/sunset, recording windows
- `birdnet-migrate` → the migration import wizard

WebSocket stream at `/api/v2/ws` (already in the crate) feeds the live dashboard, spectrogram, and kiosk. Server-sent events at `/admin/system/logs/page` feed the operations log viewer.

---

## Implementation guidance

### Approach options

1. **Enhance HTMX templates** — fastest path. Render server-side; use Alpine.js for the small interactive bits (Tweaks-style panels, expand/collapse, live feed prepending). Canvas-heavy screens (spectrogram, living-spectrum kiosk station) use small islands of Web Components.
2. **Yew or Leptos SPA** — keeps everything Rust. Higher fidelity for the interactive screens; HTMX simplicity disappears.
3. **Separate frontend** — Vite + Svelte/Solid app served by `axum::Router` static-files. Best for the team if they want to iterate quickly on JS-side features.

The team should pick by what's already in `crates/birdnet-web/templates/`. If it's pure HTMX, **option 1** is the lift. If they've been considering a rewrite anyway, **option 3** has the smoothest path for everything except the chord diagram and spectrogram (those need careful Rust-side data shaping regardless).

### Asset checklist

- **Fonts** — already self-host from Google Fonts CDN in prototype; switch to `fonts.bunny.net` or self-hosted WOFF2 files in `crates/birdnet-web/static/fonts/` for offline-on-Pi support. Required families: Instrument Serif, Inter Tight, JetBrains Mono.
- **Bird photos** — prototype uses Wikipedia Commons CC BY-SA. Cache these via `birdnet-integrations::wikipedia` (already implemented). User-uploaded overrides live in `/data/photos/<sp.short>/`. Attribution badge is required by CC BY-SA.
- **No icon library** — every icon is inline SVG. Don't pull in Lucide / Phosphor / etc. for these.

### Quality bar

- **All numeric data** uses `font-variant-numeric: tabular-nums`
- **Every interactive element** has a focus-visible state with 2px moss outline + 2px offset
- **WCAG AA** contrast on body text (the `--fg-2` against `--surface` lands at 7.1:1 in light, 11:1 in dark)
- **`prefers-reduced-motion`** must disable: feed-prepend pulse, kiosk fade, aurora, sonar rings, phone-rise. Replace with instant transitions
- **Keyboard nav** through all rows in lists, tables, queue, day-calendar tiles
- **`hidden` content** must use `aria-hidden="true"` (SVG decorations); **icon-only buttons** need `aria-label`

### Testing matrix

- Browsers: Chrome 110+, Safari 16.4+ (OKLCH needs 16.4), Firefox 113+
- Pi 5 / Pi 4B rendering perf for kiosk mode (60fps target on the streaming spectrogram canvas)
- Mobile viewport 390×844 (iPhone 14) for the phone screens
- Print stylesheet for Weekly Report PDF export — newspaper layout collapses to single column, headings retain weight

---

## Source data shape

The prototype mock at `source/lib/data.jsx` has the shape every screen consumes. Real queries should produce equivalents:

```ts
// Species (from SQLite + Wikipedia cache)
{ sci, common, short, color, count, conf, rare, photo, trend: number[24] }

// Heatmap (7×24)
HEATMAP: number[7][24]  // 0..5 intensity buckets

// Feed (latest N detections)
FEED: { id, sp_idx, conf, lat /*seconds*/, ago, rare? }[]

// Co-occurrence (top-N species × top-N matrix)
COOC: number[N][N]  // Spearman ρ in [0,1]
COOC_SPECIES: number[N]  // indexes into SPECIES

// Migration phenology
MIGRATION: { sp_idx, curve: number[52] /*weekly index*/ }[]

// Dawn chorus per-species
CHORUS: { sp_idx, hours: number[24] }[]

// Life list (chronological)
LIFE_LIST: { sp_idx, first /*ISO date*/, note: string }[]
```

The `color` field is OKLCH — generate from species taxonomy at first detection and persist alongside the species row.

---

## Asset list

- **`source/`** — the entire HTML prototype, run-able locally
- **Wikipedia photo URLs** are referenced in `source/lib/data.jsx` (each species has a `photo` field pointing to Commons)
- **No proprietary assets** — every icon, illustration, and SVG is inline and freely modifiable

---

## Open questions for the engineering team

These came up during design but need engineering input to resolve:

1. **Live update frequency** — the prototype animates new detections every few seconds. In production, do we push every detection or batch on a 1s tick? WebSocket frame budget at the kiosk's `Living Spectrum` station is the constraint.
2. **DuckDB query latency** — co-occurrence, year-on-year, and trends all hit the OLAP layer. Are sub-300ms queries achievable on Pi 5, or do we need to materialize nightly?
3. **Photo licensing** — Wikipedia CC BY-SA attribution is shown on every photo. If anyone wants to disable attribution for kiosk mode, that's a license violation. Confirm we keep it.
4. **Quarantine ML** — the "Top alternative ID" list in Quarantine requires the model to expose its top-K predictions, not just the argmax. Currently `birdnet-core` exposes only the best label. Is exposing top-3 a small change or a model-format change?
5. **Recording retention** — the prototype shows "auto-purge at 95% disk." We should add **per-species lock** affordances on the recordings list (already designed) so researchers can pin clips they care about before the purge runs.

---

## Final note

This design package is the result of a long, careful iteration loop. Every decision has rationale — most of which is documented above. **When tempted to "simplify" by removing structure, please re-read the relevant section first.** Many of the patterns that look ornamental (the eyebrow labels, the dual-baseline ridges, the dominant-species-color cell encoding, the night mode for kiosk) are answers to specific user-research observations from the conversation.

Reach out to the design author for context on any decision — the chat transcript is available on request.
