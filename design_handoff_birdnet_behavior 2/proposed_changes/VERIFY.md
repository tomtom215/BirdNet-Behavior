# VERIFY — Post-merge acceptance per change

The ten changes ship as a single combined PR but are verified independently — run the matching block below after the merge lands.
All checks should pass before announcing or rolling out the public-facing surfaces (O-07, O-11).

Time budget for the full sweep: ~25 minutes.

---

## O-04 · Species detail rebuild

### Automated

```sh
cargo check -p birdnet-web
cargo clippy -p birdnet-web --all-targets
```

### Manual

1. Navigate to `/species/detail?name=Northern%20Cardinal` (or any species with ≥ 1 detection).
2. **Hero**: photo card on the left fills 440px, residency/first-seen pills render under the photo caption, display headline reads at 56px Instrument Serif.
3. **Stats**: the 4-up `stat-mini` row shows Today / All-time / First seen / Mean confidence — no values are blank "—".
4. **Analytics row**: hourly chart is colored in the species hue (not a generic moss), 12-week heat grid shows weekend desaturation, companion species each have a strength bar.
5. **Recordings strip**: 5 clip cards with mini waveforms; play buttons are 32px tall and clickable.
6. **Dark mode**: toggle theme; verify card borders + photo gradient remain readable. No legacy bridge classes should be visible (no `.card` outline collisions).
7. **Empty-state path**: visit a species with no historical detections; confirm the History section degrades gracefully (shows the "All 12 →" link disabled or hidden).

### Accept if

* Page renders without console errors on Chrome, Safari, Firefox.
* All `data-screen-label="Species · <name>"` survive into rendered HTML.
* Lighthouse a11y score ≥ 95.

---

## O-09 · Today · comparative phrase

### Automated

```sh
cargo test -p birdnet-web today_phrase
```

Three unit tests should pass: `percentile_basic`, `tier_boundaries`, and the implicit round-trip.

### Manual

1. Open `/today`. Wait for `#today-phrase` to swap in.
2. Headline should read **A `<verb>` `<time>`.** where verb is one of `quiet`, `calm`, `steady`, `busy`, `loud`, `record` and time is one of `morning`, `midday`, `evening`, `night`.
3. Sub-line should include detection count, species count, and a percentile phrase.
4. Wait 5 minutes; verify the partial re-polls and the phrase updates (or stays the same).
5. **Day strip**: 24-hour bar chart shows period brackets (DAWN, MORNING, MIDDAY, EVENING, NIGHT) above. Tap an hour — confirm the list below filters (if you wired the click handler; the template provides the markup).
6. **Sticky filter row**: segmented control sticks under the topnav on scroll.

### Accept if

* `cargo test` passes.
* Phrase tier matches manual calc against last-30-days SQL: `SELECT COUNT(*) FROM detections WHERE Date = date('now') ... ;` then compute percentile within the prior 30 daily counts.

---

## O-01 · `/migration` ridgeline

### Automated

```sh
cargo test -p birdnet-web migration
```

### Manual

1. Navigate to `/migration`.
2. SVG renders within ~500 ms on a Pi 4. If load > 2 s, drop the species cap from 12 → 8 in `collect_ridges(.., max_species: ..)`.
3. **Visual check**: species ridges sorted left-to-right by peak week; spring band (weeks 8–20) tinted moss; fall band (34–44) tinted dawn; today marker dashed vertical line at current ISO week.
4. **KPI tiles**: first-of-year arrivals, peak diversity week, earliest vs last year (— if no prior-year data), still expected (— if forecast model not wired).
5. **Editorial cards**: "Just arrived", "Currently peaking", "Missing" — each renders with a real species when data is present; degrades to neutral placeholder copy when not.
6. **Old `/admin/migration` route**: confirm a 301 redirect to `/admin/import` is in place (or the new path is live and bookmarks updated).

### Accept if

* Page loads in < 2 s with 12-month database on a Pi 4.
* No `Result::unwrap()` panics in logs.

---

## O-02 · `/analytics/dawn-chorus` polar ribbons

### Manual

1. Navigate to `/analytics/dawn-chorus`.
2. Polar SVG renders 8 species ribbons; ribbons stay inside their own concentric row (no overlap).
3. Sunrise/sunset markers labeled with actual computed times (or fallback 05:30/20:00 if station lat/lon not configured).
4. Current-time hand is a dashed line from center to outer ring at the wall-clock hour.
5. Right rail: 8 species rows, each with avatar + name + peak hour + linear hour-strip + total count.
6. Off-chorus species (peak outside 05–08) get a small "off-chorus" pill on their row.

### Accept if

* `cargo check` clean.
* Page renders without console errors.
* No JS required — SVG is server-rendered.

---

## O-05 · Detection detail page

### Manual

1. From the dashboard or `/today`, click on a detection's timestamp.
2. **Hero**: crumbs (Today › Species › #id), display headline, Latin name italic, species blurb directly under the Latin name.
3. **Spectrogram card**: bounding box drawn over the detected region, scrubber below.
4. **Triage row**: Confirm / Quarantine / Mistake buttons all wired (clicking should hit the matching `POST /pages/detection-{confirm,flag}` endpoint).
5. **Context window**: ±5-min feed rows render with the current detection highlighted as "this one".
6. **History card** (below context in the main column): grid of 3 historical clip cards if this species has prior detections.
7. **Right rail**: Detection facts table only.
8. **Permalink copy**: click "Copy link" → confirms a `/detection/<id>` URL is in the clipboard.

### Accept if

* `detection_reviews` table exists (or all triage buttons are gracefully disabled if you opted out of the schema migration).
* Returning to the dashboard, clicking a feed row's time also navigates here.

---

## O-07 · Rare-bird permalinks

### Manual

1. From the quarantine queue or detection detail, click "Share clip".
2. A URL is copied to clipboard in the form `https://your-host/r/<token>`.
3. Open the URL in an incognito window (no auth). Should render:
   * The bird photo card with attribution footer.
   * Species name + Latin + 3 pills.
   * Spectrogram + scrubber.
   * Two about cards (species + project).
   * Download buttons (wav, spectrogram).
4. **Tamper test**: mutate one character of the token in the URL. Should land on the "This clip is gone" 404 page, not leak the detection.
5. **Expiry test**: in dev, set the TTL to 60 seconds; wait 70 seconds; reload. 404.
6. **OG preview**: paste the URL into Slack or iMessage. Card should render with the spectrogram as the preview image.

### Environment

* Set `BNB_SHARE_SECRET` to a 32+ byte random value before going live. Otherwise tokens invalidate on every restart.

### Accept if

* All three security tests pass.
* HMAC roundtrip test in `share.rs` passes (`cargo test share`).

---

## O-08 · Print stylesheet

### Manual

1. Open `/weekly` or `/year-in-review`.
2. Press `Cmd + P` / `Ctrl + P`.
3. Print preview should show:
   * Light palette (even if you're in dark mode on screen).
   * No topnav, no audio controls, no segmented buttons.
   * External links resolved inline as `… (https://example.com)`.
   * Section page breaks where annotated with `data-print-page-break-after`.
   * Running header on page 2+: "BirdNet-Behavior · `<page title>`".
4. Save as PDF; verify all pages render correctly in a PDF reader.

### Accept if

* Three-page weekly report fits on Letter without truncation.
* No black backgrounds bleeding through.

---

## O-03 · Display preferences

### Manual

1. Navigate to `/system` (or wherever you placed the `_partial_display_prefs.html` snippet).
2. Each of the four rows shows a 3-button segmented control with one button active.
3. **Theme**: click each. Verify `<html data-theme>` flips immediately; reload — theme persists.
4. **Density**: click each. Page reflows; `--density` CSS variable updates.
5. **Motion**: click "Reduced". Verify the dashboard's live-pulse canvas stops animating and `.bnb-dot.live` no longer pulses.
6. **Contrast**: click "High". Borders thicken; `--border` and `--fg-3` shift to high-contrast variants.
7. **Reset**: click. All controls return to defaults (Auto / Regular / Full / Standard).

### Accept if

* No FOUC on reload — the pre-paint guard in `layout.html` handles all four keys.
* OS theme change (`prefers-color-scheme`) propagates when Theme = Auto.

---

## O-11 · iCal + RSS feeds

### Automated

```sh
cargo test -p birdnet-web feeds
```

Five unit tests should pass: `rfc822_known_date`, `ics_datetime_format`, `escape_ics_special_chars`, `build_rss_empty_is_valid`, and the implicit handler integration.

### Manual

1. `curl https://your-host/feeds/rare.rss | xmllint --noout - && echo OK` — should print OK (well-formed XML).
2. `curl https://your-host/feeds/rare.ics | head` — should start with `BEGIN:VCALENDAR`.
3. Paste each feed URL into a real reader (Feedbin, Reeder, Apple Calendar). Should subscribe without complaint.
4. **Discovery**: View source of `/` (or any page using `layout.html`). Confirm `<link rel="alternate" type="application/rss+xml" href="/feeds/rare.rss">` is present in `<head>`.

### Environment

* Set `BNB_BASE_URL=https://birdnet.yourdomain` so the `<link>` and `<guid>` URLs in the feed are absolute. Without it, defaults to `http://localhost:8080`.

### Accept if

* Feed URLs return `Content-Type: application/rss+xml; charset=utf-8` and `Content-Type: text/calendar; charset=utf-8` respectively.
* `Cache-Control: public, max-age=300` (RSS) / `max-age=3600` (iCal).

---

## O-12 · Empty states

### Manual

1. Visit each surface that has been migrated:
   * **Dashboard** live feed on a station with no detections in last 24h → "A quiet yard."
   * **Species** page on a fresh install → "No species heard yet."
   * **Heatmap / Dawn chorus** with < 24h of data → "Chorus hasn't started."
   * **Correlation** with < 2 active species → "Not enough overlap yet."
   * **Quarantine** when queue is empty → "Nothing waiting for review."
   * **Life list** on first run → "Your life list starts here."
2. Each SVG renders inline (no external file). Each carries one helpful sentence below the headline.
3. In dark mode, all six illustrations remain readable (no hard-coded `#000` that disappears).

### Accept if

* All six SVGs render from `empty_states::*()` helpers (no fallback strings reaching production templates).

---

## Cross-cutting checks

After all PRs are merged:

* **Lighthouse**: run on `/`, `/today`, `/species/detail?name=…`, `/migration`, `/analytics/dawn-chorus`. All scores ≥ 90 across Performance / Accessibility / Best Practices.
* **Bundle size**: `static/css/app.css` after O-03 append should be ≤ 38 KB. If higher, you've concatenated twice.
* **Browser matrix**: smoke-test on Chrome, Safari, Firefox (latest two majors) and on a Pi-class touch device.
* **Reduced-motion**: enable OS reduced-motion; verify live-pulse and feed-row entrance animations are gone everywhere, not just on the screens you remembered to wire.
* **Dark mode**: visit every new screen with `data-theme="dark"` and confirm there are no `--surface` / `--fg` references that collapse to white-on-white or black-on-black.
* **No bridge class leakage**: grep production `static/css/app.css` for `Legacy class bridges` and confirm none of the new templates rely on them (`.card`, `.stat-card`, `.btn-primary` — all should use `.bnb-card`, `.stat-tile`, `.bnb-btn.primary`).
