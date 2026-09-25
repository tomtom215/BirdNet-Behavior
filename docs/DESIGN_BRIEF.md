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
  ~4.6k lines) and **self-hosted fonts** (offline-capable — air-gapped Pi
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
- **Brand / semantic ramps**, each with a `-soft` (pale fill) variant, and
  `--moss` / `--dawn` with an `-ink` (dark text) variant too:
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

- **Primary nav** (`crates/birdnet-web/src/routes/pages/nav.rs`, six
  entries, identical on desktop and the mobile tab bar): **Today** (`/`: live
  "right now" hero + live-signal spectrogram + stat tiles + live feed + day
  log), **Species** (`/species`, views: list / photos / life list, + species
  detail), **Patterns** (`/patterns`, tabs: when active / dawn chorus /
  migration / who sings together (co-occurrence) / trends (time series) /
  behavior (sessions / retention / funnel / next-species)), **Recordings** (`/recordings`: clips +
  live audio), **Reports** (`/reports`: weekly / year in review / history),
  **Settings** (`/station`: CPU / mem / temp / disk gauges, DB / audio status;
  tabs: health / capture / alerts / data / settings / access).
- **Former standalone pages** (**Dashboard**, **Heatmap**, **Migration**,
  **Analytics**, **Life list**, **Gallery**, **Co-occurrence**, **Time
  series**, **Dawn chorus**, **System**, **History**, **Weekly report**, **Year
  in review**, **Live audio**) are permanent redirects into those homes and
  tabs — `crates/birdnet-web/src/routes/redirects.rs` holds the map.
- **Still standalone:** Quarantine (rare-bird review), Notifications, **Admin**
  (settings / audio sources / backups / BirdNET-Pi migration / accounts /
  doctor), Kiosk (wall display), Changelog, Help / methodology.
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

   *Partly addressed.* `routes/pages/error_states.rs` now gives **failure** its
   own vocabulary beside `empty_states.rs`, because the two were being
   conflated: History told an operator with three years of data "No detection
   history yet", Recordings answered a failed query with "No saved clips yet"
   (which reads as *the purge ate them*), and the Migration tab reported a
   database error as a quiet year. Those four surfaces, the species list and
   the dashboard heatmap are converted;
   `tests/a_failed_fragment_never_renders_as_an_empty_one.rs` stops the
   `cached_fragment` shape regressing. Still inconsistent elsewhere — there are
   13 different empty-state class vocabularies and three loading idioms.
   **Single data point** and **huge numbers** are done for the sparkline, the
   species-detail day chart, the weekly chart and the stat tiles; the rest of
   the numeric slots have not been swept.
3. **At-a-glance overview.** Strengthen the Dashboard so a BirdNET-Pi user gets
   everything in one screen (today's totals, most-recent, top species, hourly
   shape, **best recordings**, multi-day trend) without hopping between
   Dashboard / Today / Species / Heatmap / Recordings.
4. **Mobile-first polish.** The bottom tab bar + PWA exist, but individual
   screens (especially the wide SVG analytics, admin forms, and tables) need
   true small-screen layouts.
5. **Per-source / multi-stream UI.** Multiple RTSP mics/cameras are
   supported and every detection now carries a first-class **`Source`** label
   (`cam1`, `cam2`, `local`). A **corroboration** display (*"also heard by
   cam2"*) already ships on the detection detail page
   (`routes/pages/detection_detail.rs`, backed by
   `birdnet_db::sqlite::concurrent_detections_from_other_sources`) — it needs
   a design pass and a home on the surfaces that do not yet show it, not
   inventing. Still to design: source **filtering**, **per-source badges /
   legends**, and — as an advanced, off-by-default option — a
   **duplicate-collapse** affordance for explicitly co-located mics. The index
   filtering needs is back: migration 52 adds
   `idx_detections_source_datetime (Source, Date, Time)`, after migration 33
   had dropped the Source index on the reasoning that nothing filtered on it.
   Two more pieces are in place: every page write now names a detection by
   its clip as well as date, time and species, so acting on one source's
   detection no longer reaches another source's detection of the same bird in
   the same second; and the Today signal card follows the chosen source
   instead of drawing every source's frames interleaved. The full,
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

   *Addressed.* Three states rather than two — `ws.onerror`/`onclose` used to
   fall through to the same flat line under the word "idle", so a blocked
   WebSocket looked exactly like a silent microphone. There is now a
   **"no signal"** state and a caption saying which of the two you are looking
   at. The flat baseline itself was invisible: a 1px stroke on an integer `y`
   straddles two device rows and painted at alpha 35/255, so the card read as a
   blank box. Remaining design work: what the card should show on a station
   with several sources, and whether the source picker belongs there.
9. **Accessibility & dark-mode** parity across all of the above.

   *Substantially addressed, and the gate was the problem.* `axe.mjs` gated on
   four WCAG tags, so 36 of axe's rules never executed, and ran at one desktop
   viewport, so the phone layout was never graded. Both are fixed and both
   tiers are blocking; the work that surfaced is in the commit log. What
   remains: nothing, on the two items that were open. `link-in-text-block` —
   the one rule this gate deferred — is now enforced: it was estimated at ~36
   in-text link sites and measured at **five**, because everywhere else a link
   sits alone in its own box. Nothing is excluded from the gate any more.

   The amber **status dot** is resolved too, and the resolution is worth
   recording because it was a design choice with three candidates. The dot
   measured 2.45:1 against the `--dawn-soft` pill it actually sits on — worse
   than the 2.85:1 against `--bg` first reported. Recolouring
   `.bnb-dot.dawn` alone would have fixed the number and broken the system,
   leaving the dot the one member of the moss/dawn/rare ramp that does not
   match its own pill. Moving `--dawn` would have fixed one 6px dot by shifting
   every amber surface — hour bars, temperature line, buttons, meters, chips —
   none of which is failing. Instead the **whole dot family** gains a contrast
   ring mixed from its own hue toward `--fg`, which darkens in light and
   lightens in dark from a single declaration; `--dot` is now the one property
   a variant sets, so the fill and the ring cannot disagree. Measured after:
   every dot clears 3:1 on its own background in both themes, the amber one at
   5.65 via its ring, with no layout change. See the "Status dots" block in
   `app.css` and `tests/a_status_dot_and_its_ring_cannot_disagree.rs`.

10. **Plain language, and the reader who has never opened a terminal.** The
    product is for someone who bought a Pi kit to find out which birds are in
    their garden. Every string that reaches the browser is written for them.

    *A first pass is done; the standing rule is the point.* Three tests, in
    order of how much they caught:

    - **Does the word name the thing, or the mechanism?** `Backing off` is the
      retry algorithm; `Reconnecting` is what is happening. `quick_check` is
      SQLite's pragma; "no damage found" is the finding. `usb-alsa` is the
      storage identifier; "USB microphone" is the object on the windowsill.
      `0.87` is the model's output; `87%` is how sure it was.
    - **Does it name which one?** "An audio source is down" is true and
      useless on a station with three. Name it, with the label its owner typed
      — which means the screen showing status has to know the names the screen
      doing configuration collects.
    - **Can the reader act on it?** A badge that states a fault must be a link
      to the screen that explains it. An error must distinguish "your input was
      wrong" from "the station failed", because the reader's next action is
      completely different — and the station must never claim the former when
      it means the latter.

    Two structural rules fall out and are enforced by gates rather than by
    review:

    - **A number a person types is validated where they type it.** Checking it
      somewhere upstream is not checking it. `validate()` ran on the config
      file and the settings overlay was applied afterwards, so the one field
      most likely to be got wrong was the one field nothing checked. The five
      ranges `validate()` enforces live in
      `birdnet_core::config::validate::NUMERIC_RANGES`, and a test holds the
      form's copy equal to it; the two alert thresholds are bounded by the form
      alone, because a `validate()` error reverts the whole configuration file
      and a mistyped notification threshold must not do that.
    - **A message that refuses input says what to type instead.** Restating the
      rule is not enough when the mistake is predictable: someone entering
      `75` in a 0–1 field means 75%, and the refusal says so.

    Hover is not a channel. Anything living only in a `title` attribute does
    not exist on a phone, and a `title` on a control is not a reliable
    accessible name.

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
| SVG viz helpers | `crates/birdnet-web/src/routes/pages/viz/` — a directory (`mod.rs`, `funnel.rs`, `matrix.rs`, `radial.rs`, `timeline.rs`), plus `atoms.rs` beside it |
| Empty states | `crates/birdnet-web/src/routes/pages/empty_states.rs` |
| Skeletons | `crates/birdnet-web/src/routes/pages/skeletons.rs` |
