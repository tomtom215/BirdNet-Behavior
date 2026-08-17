# BirdNet-Behavior — UI Design Brief

A self-contained brief for a future **Claude Design** session to redesign /
overhaul the web UI. It encodes the real design system (so a redesign stays
consistent) and the actual gaps. Paste it (or hand it) to the design session.

> Keep this versioned. Update the **Known problems & goals** and **Current
> screens** sections as the app evolves so the next design pass starts from
> current reality, not this snapshot.

---

## Product context

You are redesigning the web UI of **BirdNet-Behavior** — a single-binary Rust
rewrite of BirdNET-Pi: a real-time acoustic **bird-detection station**
dashboard. It runs on a **Raspberry Pi 4/5** (and x86 Linux) and is viewed over
the **local network** on phones, tablets, and desktops. Treat it as an
at-a-glance "what's singing in my yard right now" appliance for **non-technical
users**, with deep analytics underneath for enthusiasts.

## Hard technical constraints (the redesign must respect these)

- **Server-rendered with axum + HTMX** — no SPA framework, no client-side
  router. Screens are full HTML pages whose live regions are HTMX partials
  (`hx-get` / `hx-trigger="load, every Ns"` / `hx-swap`). Design for
  partial-swap, skeleton-then-content loading.
- **Strict CSP: no inline `style=` and no inline scripts.** All styling is
  classes in one stylesheet; the few dynamic values use a `data-style`
  attribute promoted by a nonce'd `<style>` block. Do not propose inline styles.
- **One hand-written stylesheet** (`crates/birdnet-web/static/css/app.css`,
  ~3.4k lines) and **self-hosted fonts** (offline-capable — air-gapped Pi
  installs are supported). No CDNs, no runtime-fetched web fonts, no new JS
  dependencies.
- **Performance-conscious for a Pi** and **fully responsive** (a phone
  bottom-tab-bar + PWA already exist). **Light & dark themes** via a
  `data-theme` attribute on `<html>`, set pre-paint.
- **Accessibility**: honor `prefers-reduced-motion`, sufficient contrast, focus
  states, ARIA for live regions, keyboard nav (a ⌘K command palette exists).

## Design system to match exactly (OKLCH tokens)

- **Neutrals:** `--bg`, `--bg-2`, `--surface`, `--surface-2`, `--hairline`,
  `--text`, `--text-muted`.
- **Brand / semantic ramps**, each with a `-soft` (pale fill) and `-ink` (dark
  text) variant:
  - `--moss` (calm green) = primary / accent / **success** / "live" / recording.
  - `--dawn` (amber) = **warning** / "today" highlights.
  - `--rare` (red) = **danger** / **rare-bird** accent.
- **Type:** **Inter Tight** (UI, weights 400/500/600) + **Instrument Serif**
  (large *display* headings, italic used for emotional emphasis — e.g. the hero
  "The yard is *singing*"). Monospace for tabular numbers and timestamps.
- **Existing component vocabulary to reuse and extend** (do not reinvent):
  `bnb-card`, `bnb-pill` + `bnb-dot` (status), `bnb-eyebrow` (small-caps section
  label), `stat-tile`, `section-header`, skeleton loaders, OOB **toasts**, a
  **confirm modal**, a **command palette**, **arc gauges** (System page), and a
  family of **hand-rolled SVG visualizations**: activity streamgraph, phenology
  ridgeline, co-occurrence matrix + chord / "acoustic network", circadian /
  dawn-chorus polar, hour × day-of-week heatmap, sparklines, diversity bars, and
  detection **"feed rows"** (avatar + species + scientific name + confidence bar
  + waveform + inline audio player).

## Current screens (inventory to evaluate and improve)

- **Primary nav:** **Dashboard** (live "right now" hero + live-signal
  spectrogram + stat tiles + live feed + top species + today heatmap + **best
  recordings**), **Today** (full day log + timeline), **Species** (+ species
  detail), **Heatmap** (streamgraph / grid / dawn-chorus / hourly / phenology),
  **Migration** (phenology ridgeline + diversity + KPI tiles + editorial cards),
  **Analytics** (behavioral: sessions / retention / funnel / next-species),
  **Life list**, **Quarantine** (rare-bird review), **System** (real CPU / mem /
  temp / disk gauges + DB / audio status).
- **"More" menu:** History, Weekly report, Year in review, **Live audio**
  (listen + test mic), Recordings, Gallery, Dawn chorus, Co-occurrence, **Time
  series** (DuckDB), Notifications, **Admin** (settings / audio sources /
  backups / BirdNET-Pi migration / accounts), Kiosk (wall display), Changelog,
  Help / methodology.
- **Plus:** Onboarding wizard, Login, Detection detail, Share permalinks,
  iCal / RSS feeds.

## Known problems & goals (prioritize these)

1. **Consistency & alignment pass.** The analytics screens grew organically and
   have uneven spacing, alignment, card rhythm, and header patterns. Define a
   coherent **page template** (page-head / eyebrow / segmented range controls /
   card grid) and apply it across *all* analytics pages so they feel like one
   product.
2. **Edge cases & states, everywhere.** Design the full state matrix for every
   data surface: **loading (skeleton)**, **empty / first-run ("quiet yard")**,
   **error / unavailable**, **single data point**, **huge numbers** (1.6M+
   detections), **very long species / common names**, **overflow / truncation**,
   **dense vs sparse data**. These are currently inconsistent.
3. **At-a-glance overview.** Strengthen the Dashboard so a BirdNET-Pi user gets
   everything in one screen (today's totals, most-recent, top species, hourly
   shape, **best recordings**, multi-day trend) without hopping between
   Dashboard / Today / Species / Heatmap / Recordings.
4. **Mobile-first polish.** The bottom tab bar + PWA exist, but individual
   screens (especially the wide SVG analytics, admin forms, and tables) need
   true small-screen layouts.
5. **New: per-source / multi-stream UI.** Multiple RTSP mics/cameras are
   supported and every detection now carries a first-class **`Source`** label
   (`cam1`, `cam2`, `local`). Design source **filtering**, **per-source badges /
   legends**, a **corroboration** display (*"also heard by cam2"* — multiple mics
   confirming a detection), and — as an advanced, off-by-default option — a
   **duplicate-collapse** affordance for explicitly co-located mics. The full,
   corroboration-first design rationale is in
   [`book/field/multistream.md`](book/field/multistream.md); design these as
   one coherent surface.
6. **Admin & onboarding UX.** The settings area is form-dense; make it a guided,
   reassuring experience for non-technical owners (audio source setup, location,
   notifications, backups). Polish the first-run wizard.
7. **System / health screen** as a proper status dashboard now that metrics are
   real (CPU / mem / temp / disk with low/critical states, audio-pipeline
   liveness per source, detection-daemon status).
8. **The "live signal".** Rethink the dashboard's live spectrogram so its
   liveness is honest (it reflects the last captured segment; an idle/flat state
   means no recent audio). Design clear **"live / idle / no audio"**
   affordances.
9. **Accessibility & dark-mode** parity across all of the above.

## What I want from you, per screen (deliverables)

- A short **diagnosis** of the current screen's problems.
- A **redesign** expressed as **static HTML + CSS that uses the existing tokens
  and classes** (so it can drop into the HTMX templates and `app.css` with
  minimal new classes) — *not* a framework mockup. Include **all states**
  (loading / empty / error / data), **mobile and desktop** layouts, and **light
  + dark**.
- Any **new utility classes** you introduce, listed explicitly and additive (no
  overrides of existing classes), CSP-safe.
- Notes on **HTMX wiring** (which regions are partials, swap targets, poll
  intervals) and any **a11y** considerations.

## Ground rules

Reuse the OKLCH tokens and existing components; no inline styles or scripts; no
new runtime JS/CSS dependencies; everything must render server-side and work
offline; keep it gentle on a Raspberry Pi. **Start by proposing a prioritized
screen-by-screen plan**, then design the top screens first (Dashboard, the
shared analytics page template, mobile, and the per-source UI).

---

## Where things live (for the implementer who applies the designs)

| Area | Path |
|------|------|
| Templates (HTML) | `crates/birdnet-web/templates/` |
| Page/partial routes | `crates/birdnet-web/src/routes/pages/` |
| Admin routes | `crates/birdnet-web/src/routes/admin/` |
| Stylesheet | `crates/birdnet-web/static/css/app.css` |
| Self-hosted fonts | `crates/birdnet-web/static/fonts/` |
| Nav manifest (single source of truth) | `crates/birdnet-web/src/routes/pages/nav.rs` |
| SVG viz helpers | `crates/birdnet-web/src/routes/pages/viz.rs` (+ `atoms.rs`) |
| Empty states | `crates/birdnet-web/src/routes/pages/empty_states.rs` |
| Skeletons | `crates/birdnet-web/src/routes/pages/skeletons.rs` |
