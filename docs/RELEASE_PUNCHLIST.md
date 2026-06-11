# Release-Readiness Punch-List

**Purpose.** A self-contained backlog of the work remaining before a clean release,
written so it can be picked up **cold in a fresh session** — every item carries its
evidence (`file:line`), root cause, a concrete fix plan, effort/risk, dependencies,
and how to verify it. Nothing here is started; pick an item and go.

_Last audited: 2026-05-29, against integration tip `claude/gallant-feynman-bJs95`
(`032553e`, after PR #113). Re-run the audit greps below if the tree has moved._

---

## 0. How to work this repo (read first if resuming cold)

**Branch model (squash-loop).** Two long-lived branches:
- **Working branch:** harness-assigned each session (e.g. `claude/sleepy-brown-de7jU` this cycle) — commit here; use it as-is, do not rename.
- **Integration branch:** `claude/gallant-feynman-bJs95` — open every PR with this as the **base**.
- `main` is the old release branch (stuck at `#86`); **do not** target it.

Per-task cycle (`$WORK` = your assigned working branch):
1. Ensure the working branch is at the integration tip:
   `git fetch origin claude/gallant-feynman-bJs95 && git reset --hard origin/claude/gallant-feynman-bJs95`
2. Commit the change on `$WORK`.
3. `git push --force-with-lease -u origin $WORK`
   (force-with-lease is expected — the working branch is rewritten each cycle after the prior squash-merge).
4. Open a PR: head `$WORK` → base `claude/gallant-feynman-bJs95`.
5. After it squash-merges, go back to step 1.

**Gate (run all before opening a PR — there is _no_ CI on this repo, so this is the only gate):**
```bash
cargo fmt --check --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --lib
cargo test -p birdnet-behavior --bins   # the root crate is bin-only; --lib skips its ~290 unit tests
# plus any integration test you touched, e.g.:
cargo test --test web_api_admin
```
Note: `--lib` **skips `tests/`** (integration tests link libonnxruntime) **and skips the root
binary crate's own unit tests** (it has no lib target — e.g. the `helpers::state` tests). Run
`cargo test -p birdnet-behavior --bins` and the relevant `--test <name>` explicitly so neither
unit- nor integration-test rot can hide (this is how the dead Basic-Auth test in #113 had gone stale).

**ONNX Runtime offline note (now automated).** `ort`/`ort-sys` downloads a prebuilt ONNX Runtime
via a bundled rustls client that does **not** trust a TLS-intercepting sandbox proxy, so a cold
build fails with `invalid peer certificate: UnknownIssuer`. `curl` (system CA) reaches the CDN fine.
This is now handled automatically: a **SessionStart hook** (`.claude/hooks/session-start.sh`, wired
in `.claude/settings.json`) runs `cargo fetch` then **`scripts/setup-onnxruntime.sh`**, which reads
the URL+sha256 from ort-sys's own `dist.txt` (falling back to the `.crate` when `registry/src` isn't
extracted yet), curls the artifact, verifies the sha256, and unpacks `libonnxruntime.a` into the
cache ort-sys checks before downloading. It is idempotent — run it by hand any time the cold build
fails: `bash scripts/setup-onnxruntime.sh` (pass a target triple, e.g.
`aarch64-unknown-linux-gnu`, when cross-compiling for a Pi). The hook only fires in the remote
(web) environment and is non-fatal.

**Conventions:** see `CLAUDE.md` — `unsafe` is forbidden workspace-wide (so no `std::env::set_var`
in tests), no `anyhow`/`thiserror` in library crates, library crates are sync (tokio owned by
the binary/`birdnet-web`), clippy pedantic+nursery on.

**Status:** the entire **O-01…O-26 roadmap has shipped** (Waves A–E, PRs #87–#104 plus the
multi-stage O-13/O-14/O-15 follow-ups through #113). The code sweep found **no
`todo!()`/`unimplemented!()`/`unreachable!()` and no bare panics in production paths**. What
remains is the finish-off work below.

---

## Priority summary

| ID | Item | Priority | Effort | Risk |
|----|------|----------|--------|------|
| ~~**BUG-1**~~ | Bird images don't populate gallery/previews — ✅ **DONE** (fetch-on-miss `/file` + default-on cache) | ~~P1~~ | M | low–med |
| ~~**P1-1**~~ | Dead "Reset password" button — ✅ **DONE** (wired to live `set_password`) | ~~P1~~ | S | low |
| ~~**P2-1**~~ | Stale `accounts.rs` module doc — ✅ **DONE** (folded into P1-1) | ~~P2~~ | XS | none |
| ~~**P2-2**~~ | CSP hardening — ✅ **DONE (both halves)**. `script-src 'nonce' 'strict-dynamic'` **and** `style-src 'self' 'nonce'` — `'unsafe-inline'` fully dropped. Computed styles ride a `data-style` CSSOM applier (middleware-injected, re-runs on `htmx:afterSwap`), statics are app.css classes. Browser-verified **0** CSP violations across 20 pages + HTMX fragments. | ~~P2~~ | M | med |
| ~~**P2-3**~~ | Extend help links to remaining analytical screens — ✅ **DONE** (6 screens) | ~~P2~~ | S | low |
| **P3-1** | O-13 legacy `--audio-source` retirement — ✅ **DONE** | P3 | S | low |
| ~~**P3-2**~~ | No background session pruning — ✅ **DONE** (daily maintenance tick) | ~~P3~~ | S | low |
| ~~**P3-3**~~ | O-25 inline-style sweep (unlocked P2-2 style-src) — ✅ **DONE**. Every served `style="…"` attribute eliminated: static shapes → app.css classes, computed values → a `data-style` attribute applied via CSSOM. The guard now scans the *whole* crate and forbids **any** inline style. The endgame (slice 18) added `inject_style_nonce` + the `data-style` applier and dropped `style-src 'unsafe-inline'`. | ~~P3~~ | L | low (tedious) |
| **P3-4** | Minor cosmetics — uptime pill ✅ **wired**; migration-missing out of scope | P3 | XS | none |
| ~~**P3-5**~~ | Image blacklist enforcement on read path — ✅ **DONE** (serve-check + purge-on-blacklist) | ~~P3~~ | S | low |

Recommended order: ~~BUG-1 → P1-1 → P2-1 → P2-3 → P3-2 → P3-5 → P3-1 → P2-2 (script half) → P3-3 (inline-style sweep) → P2-2 (style-src drop)~~ — **all shipped; `style-src 'unsafe-inline'` is gone**. _Also shipped: ONNX offline build tooling (SessionStart hook). Remaining low-value/deferred: P3-4 cosmetics._

---

## BUG-1 — Bird images don't populate the gallery or previews  · **P1** — ✅ DONE

**✅ Shipped (this PR).** Both ranked root causes fixed:
1. **`/file` is now fetch-on-miss.** `species_image_file` (`crates/birdnet-web/src/routes/images.rs`)
   calls `cache.get_image()` on a cache miss (mirroring `species_image_info`), so every
   `<img src=".../file">` self-heals on first view; 404 only when no cache is configured or the
   species genuinely has no image. The gallery warmer stays as a pre-warm optimisation.
2. **Image cache defaults on.** `init_image_cache` (`src/helpers/state.rs`) now defaults the cache to
   `<db_dir>/images` when unset (mirrors `default_analytics_path`); an empty `--image-cache-dir ""`
   or `IMAGE_CACHE_DIR=` opts out. Bare-metal installs now show photos like Docker already did.
   _Privacy/egress: a stock install now reaches Wikipedia on demand — documented in `cli.rs` help and
   `.env.example`; consistent with analytics default-on and BirdNET-Pi showing images by default._

Regression test: `tests/web_api_images.rs` — a stubbed `ImageProvider` + throwaway localhost server
proves `/file` returns `200 image/*` on a **cold** cache (red before, green after). Gate green
(`fmt`, `clippy -D warnings`, `--bins` unit tests, `--test web_api_images`, `--test web_api_species`).

**Follow-ups discovered (not blockers, filed below as P3-5):** the `image_blacklist` is admin-managed
but **not consulted on any read/fetch path** today (gallery warmer, `species_image_info`, and now
`/file` all skip it) — so it was correct to mirror `species_image_info` here rather than half-wire it
into one endpoint. Also, `doctor::paths` only reports the image cache when explicitly set, not the new
default. See **P3-5**.

The original investigation notes are kept below for context.

**Symptom (reported).** Bird images are not fetched from the web to fill the image
gallery and species/detection previews.

**Pipeline.**
- Fetch+cache: `birdnet-integrations::species_images` — `ImageCache` + `WikipediaClient`
  (`ImageProvider`). `ImageCache::get_image()` (`crates/birdnet-integrations/src/species_images/mod.rs:127`)
  is the only fetch-on-miss path; it queries the MediaWiki API, downloads the thumbnail bytes
  with a User-Agent'd client (`mod.rs:142`, Wikimedia rejects anonymous requests — see the
  T400119 note at `mod.rs:45`), and writes them to the on-disk cache.
- Wiring: `src/helpers/state.rs:51 init_image_cache()` installs the cache into `AppState`.
- Serve: `crates/birdnet-web/src/routes/images.rs` mounts `/species/image/{sci}` (`species_image_info`,
  JSON, **does** fetch-on-miss) and `/species/image/{sci}/file` (`species_image_file`, the bytes),
  nested under `/api/v2` (`routes/mod.rs:74`).
- Render: gallery (`routes/pages/gallery.rs:139`), species detail (`pages/species_pages.rs:277,287`),
  detection detail (`pages/detection_detail.rs`) emit `<img src="/api/v2/species/image/{sci}/file">`.

**Candidate root causes — ranked. Diagnose in this order:**

1. **`/file` is cache-only (no fetch-on-miss) — the core code defect.**
   `species_image_file` (`routes/images.rs:126`) comment: *"Only serve from cache (no network
   fetch for file serving)"*; on a miss it returns `404 {"error":"image not cached"}`
   (`images.rs:130`). The `<img>` tags therefore **never trigger a fetch**. The only thing that
   populates the cache is the gallery's background warmer (`gallery.rs:~86`, `tokio::spawn`,
   paced **800 ms per species**) and the JSON `species_image_info` endpoint. Consequences:
   - Gallery on first paint shows broken images and fills in slowly (200 species ≈ 2.5 min).
   - Species/detection previews `<img .../file>` **404 until** a gallery visit happened to warm
     that species — so previews look permanently broken if the user never opened the gallery.
   - **Fix:** make `species_image_file` fetch-on-miss — on cache miss, call `cache.get_image(&sci).await`
     (respecting the `image_blacklist`, a bounded timeout, and a placeholder/`404` on genuine
     "no image"), then serve the bytes. Keep the gallery warmer as an optimisation. This makes every
     `<img>` self-heal on first view. _This is the most likely fix regardless of setup._

2. **Cache disabled because no cache dir is configured (bare-metal installs).**
   `init_image_cache` (`src/helpers/state.rs:61`) **no-ops** unless `--image-cache-dir` (CLI) or
   `IMAGE_CACHE_DIR` (config) is set; then `state.image_cache()` is `None` and `/file` returns
   `404 {"error":"image cache not configured"}` (`images.rs:115`). Contrast: analytics **defaults on**
   with a sensible path (`state.rs:40 default_analytics_path` — *"so installs that never explicitly
   enable analytics still get the full feature set out of the box"*). Docker sets
   `BIRDNET_IMAGE_CACHE_DIR=/data/cache` in `.env.example:234`, so **Docker installs are fine**;
   **bare-metal installs that didn't set it get no images at all.**
   - **Fix:** mirror the analytics default — when neither CLI nor config sets it, default the cache
     dir alongside the DB (e.g. `<db_dir>/images`) so images work out of the box. Weigh the
     privacy/bandwidth angle (this makes a stock install reach Wikipedia on demand); the analytics
     precedent is "default on with an opt-out", and BirdNET-Pi shows images by default, so default-on
     with an opt-out (empty `IMAGE_CACHE_DIR` = disabled) is the consistent choice.

3. **Cache dir set but construction failed.** `ImageCache::with_wikipedia` errors (dir not writable,
   volume not mounted) are swallowed as a non-fatal `warn!` (`state.rs:74`) → cache `None`. Check the
   logs for `"species image cache not available (non-fatal)"`. Confirm `/data/cache` is mounted/writable.

4. **Outbound fetch to Wikipedia/Wikimedia is blocked or rejected.** Needs egress to
   `en.wikipedia.org` (`wikipedia.rs:28`) and `upload.wikimedia.org`. The API client sets a UA
   (`wikipedia.rs:60`); confirm the **download** client (`mod.rs image_download_client()`) sends the
   UA Wikimedia now requires (T400119). Diagnose: `curl -A 'BirdNet-Behavior/0.1' '<the API URL from
   query_page>'` from the host, and watch for `ImageError::Http`/`Api` in logs.

**Diagnosis quick-path.** Hit `GET /api/v2/species/image/Turdus%20merula` (the JSON endpoint, which
*does* fetch): `"disabled"` ⇒ cause #2/#3; an `Http`/`Api` error ⇒ cause #4; `"cached"` with a working
URL while `<img .../file>` still 404s ⇒ cause #1.

**Verify the fix.** With a cache dir set and network up: load a species detail page for a never-warmed
species → image renders on first view (proves fetch-on-miss). Bare-metal with nothing configured →
images still render (proves the default). Add an integration test that, with a stubbed `ImageProvider`,
asserts `/api/v2/species/image/{sci}/file` returns `200 image/*` on a cold cache.

**Effort:** M. **Risk:** low–med (touches the serve path + a default; mind the privacy/egress decision in #2).

---

## P1-1 — Dead "Reset password" button in admin accounts  · **P1** — ✅ DONE

**✅ Shipped.** `password_reset_form(id)` (`accounts.rs`) renders an inline password form posting to the live `set_password` (`POST /admin/accounts/users/{id}`); added to the seed-admin row and every non-admin row. `web_api_admin` test asserts a valid rotation changes the argon2 hash and a <10-char one does not.

**Evidence.** `crates/birdnet-web/src/routes/admin/accounts.rs:176` — the seed admin's button does
`hx-post="/admin/accounts/users/0/password-reset-stub"`, a URL with **no route** (the real route is
`POST /admin/accounts/users/{id}`, `accounts.rs:46`). Clicking it is a 404/no-op.

**Not a backend gap.** The `set_password` handler (`accounts.rs:361`) is fully implemented (10-char
min, argon2 via `accounts::hash_password`, `conn.set_password`). The user-create form already posts a
`password` field (`templates/admin_accounts.html:92`). Only the per-user **rotate** affordance is unwired,
and the handler's `// (rotate password — stub)` banner (`accounts.rs:353`) is stale.

**Fix.** Point the admin's "Reset password" button at a confirm-modal/inline form that `POST`s
`{password}` to `/admin/accounts/users/{admin_id}` (the live `set_password`); add the same control to
the non-admin user rows (they currently only have "Remove"). Drop the stale "stub" comment.

**Verify.** Rotate the admin password via the UI → success toast; old sessions invalidated (secret is
derived from the password). Add a `web_api_admin` case: admin `POST /admin/accounts/users/{id}` with a
valid password → 200 + hash changes; <10 chars → error toast.

**Effort:** S. **Risk:** low. _Fold in **P2-1** while you're in this file._

---

## P2-1 — Stale `accounts.rs` module doc  · **P2** — ✅ DONE

**✅ Shipped** (with P1-1): module doc rewritten to describe the live central RBAC (cookie middleware + admin-only writes); the stale "(rotate password — stub)" banner dropped.

**Evidence.** `accounts.rs:8-13` still says *"until the auth wire is flipped the request-time user is
the seed admin row … see the `TODO(O-15-followup)` comments below for the call sites that need
`require_admin`."* The wire is flipped (#96), RBAC is centralised in the cookie middleware (#112), and a
grep confirms **no such `TODO` comments and no `require_admin` call remain in this file**. Same stale-doc
class #112 reconciled in `session.rs`/`auth_pages.rs`; this file was missed.

**Fix.** Rewrite the header to describe the live state (cookie middleware gates `/admin`, writes are
admin-only via the central RBAC check). **Verify:** `cargo doc` + read-through. **Effort:** XS. **Risk:** none.

---

## P2-2 — CSP hardening (`script-src` + `style-src`)  · **P2** — ✅ DONE (both halves)

**✅ Shipped.** `script-src` is now **`'nonce-{random}' 'strict-dynamic'`** — `'unsafe-inline'` and
host-allowlisting are both gone for scripts. One security-middleware pass owns the whole mechanism, so
there is no per-render-path threading to get wrong:

- **One per-request CSPRNG nonce** (`security.rs`, `OsRng` + base64) is minted in
  `security_headers_middleware`, stamped onto every parser-inserted `<script>` of each `text/html`
  response body, and mirrored into that response's `script-src`. Non-HTML responses — the audio
  `/stream`, the live WebSocket upgrade, JSON, images, static assets — are skipped by content-type and
  never buffered.
- **No render-path threading.** Injecting in the single middleware covers the shared layout, the ~8
  bespoke admin `<head>` shells, onboarding, kiosk, login and share uniformly — a new page or inline
  script can't silently ship un-nonced. `'strict-dynamic'` lets htmx (itself nonced) inject fragment
  scripts without a nonce-mismatch.
- **`style-src` is now `'self' 'nonce-{random}'` too** (P3-3 endgame, slice 18) — `'unsafe-inline'` is
  gone for styles as well. The same middleware stamps the nonce onto every `<style>` *element*; inline
  `style="…"` **attributes** were eliminated entirely (they can't carry a nonce, and a per-request-nonced
  `<style>` couldn't match the host page's nonce inside an HTMX-swapped fragment anyway). Computed values
  ride a `data-style` attribute applied by a tiny CSSOM writer the middleware injects into every full
  document and re-runs on `htmx:afterSwap`; static values are `app.css` classes. htmx's own auto-injected
  indicator `<style>` is disabled via the `htmx-config` meta (its rules already live in `app.css`).

**Verified (headless Chromium via Playwright).** **Script half:** all 32 screens + interactions (command
palette, theme toggle, live admin stats) — 0 violations, header nonce == body nonce, static JS/JSON
byte-identical. **Style half:** a `securitypolicyviolation` sweep of 20 pages **plus** their HTMX fragment
swaps (feed, heatmap grid, gallery grid, recordings tabs, range buttons) reports **0** CSP violations
under `style-src 'self' 'nonce'`, and the `data-style` applier is confirmed to style pages, standalone
admin shells, and swapped-in fragments (avatar `--sp`, conf-bar widths, gallery swatches, system usage
bars all apply); gallery/migration/dashboard pixel-checked for parity. **Risk retired:** a missed inline
script *or style* can no longer silently break the UI — both injectors are exhaustive and browser-verified.

---

## P2-3 — Extend help links to the remaining analytical screens  · **P2** — ✅ DONE

**✅ Shipped.** `help_link(Topic::…)` added to correlation, behavioral, history, species (list), timeseries, and system-dashboard headers via the `{{help_link}}` placeholder + handler `.replace` pattern. Targets: Analytics ×4, Species, AdminSystem — all existing mdBook pages.

**Evidence.** `help_link(Topic::…)` is wired on 12 screens (dashboard, today, heatmap, dawn_chorus,
life_list, recordings, quarantine, migration, notification_center, weekly_report, year_in_review, help).
The analytical screens **without** it: `correlation`, `behavioral`, `history`, `species_pages`,
`timeseries_dash`, `system_dashboard` (`crates/birdnet-web/src/routes/pages/`). README_v2 explicitly
deferred these "cross-screen edits" to follow-ups.

**Fix.** Apply the proven #104 pattern: add a `Topic` variant per screen (if missing) and a
`help_link(Topic::…)` in each page header. Same shape as the shipped batch — low-risk. **Verify:** each
screen shows the "How this works" affordance opening the right mdBook section. **Effort:** S. **Risk:** low.

---

## P3-1 — O-13 legacy `--audio-source` retirement  · **P3** — ✅ DONE

**✅ Shipped (seed-then-retire).** The `audio_sources` table (managed via `/admin/audio`) is now the
single source of truth for capture **and** the web surface (live `/stream`, Listen, `/admin/audio`). The
legacy single-string `state.audio_source()` live-stream fallback is retired with **no regression** for
CLI/env-configured stations:

- **Seed on startup:** when the `audio_sources` table is empty, `start_capture_manager` seeds it from the
  CLI/config sources (reusing `resolve_sources`), so `--rtsp-url` / `--alsa-device` / `--pipewire-device`
  / `RTSP_URL` / `ALSA_CARDS` / `ALSA_CARD` keep working — they now populate the table. Idempotent: only
  an empty table is seeded, so admin-UI edits/deletes are never re-seeded, and migration 15's
  `settings.audio_source` seed still wins on upgrade.
- **Retired the legacy readers:** removed `init_audio_source`, the `with_audio_source` builder, the
  `audio_source` AppState field/getter, and the three web fallbacks (`/stream` default resolver, Listen
  selector, `/admin/audio` daemon-status heuristic). `/stream` now `503`s only when the table is *truly*
  empty (no CLI/config sources either); the Listen selector then shows a disabled "no audio sources
  configured" placeholder.
- The capture supervisor's own CLI/config fallback remains as a safety net (state-less invocations or a
  seed failure) — redundant once seeding runs, but harmless.

---

## P3-2 — No background session pruning  · **P3** — ✅ DONE

**✅ Shipped.** `run_session_prune` folded into the existing daily maintenance tick (`src/maintenance.rs`) — opens the DB, calls `prune_expired_sessions`, logs the count; best-effort/non-fatal. Test: an expired row is pruned, a live row survives.

**Evidence.** `birdnet_db::accounts::prune_expired_sessions` exists but a workspace grep shows it is
**never called** — the `sessions` table grows until manually pruned.

**Fix.** Call it from a periodic background task in the binary (alongside the existing daily auto-update
loop in `src/app.rs`), e.g. once daily. **Verify:** unit-test that an expired row is removed; confirm the
task is spawned at startup. **Effort:** S. **Risk:** low.

---

## P3-3 — O-25 inline-style sweep  · **P3 (tedious; unlocks P2-2 style-src)**

**Evidence.** ~**1.1k** inline `style="…"` attributes across ~**88** files (`rg -c 'style="' crates/birdnet-web/`).
Cosmetic on their own, but inline style *attributes* can't carry a CSP nonce, so eliminating them is the
prerequisite for dropping `style-src 'unsafe-inline'` (P2-2). The utility-class vocabulary is already landed
in `app.css` (`bnb-row` / `bnb-grid-*` / `bnb-form-row` / `bnb-kv` / …); the O-25
inline-style audit that specified it has shipped and was removed from the repo.

**Fix.** **Do not** attempt in one PR. Sweep **one area per PR**, substituting inline `style=` for the existing
utility classes (reusable shapes) or page-scoped `<style>` rules (page-specific styling), with a per-page
render guard so new fields can't silently reintroduce an inline style. Track coverage; the **final** PR
(a) handles the remaining *dynamic* inline styles (computed bar widths, `--sp:` avatar colours) by moving them
into **nonce'd `<style>` blocks** — a symmetric extension of the shipped script-nonce middleware — and then
(b) drops `'unsafe-inline'` from `style-src`. **Verify per PR:** visual diff unchanged; occurrence count drops.
**Effort:** L (many small PRs). **Risk:** low but tedious.

**Sequencing (verification-aware).** Two kinds of slice. *Faithful* extractions — old-style standalone admin
pages whose inline styles are static or *enumerable* (fold into the page's own `<style>` block; status colours →
variant classes) — are **zero visual change** and fully unit-verifiable, so they ship first. *Harmonization /
dynamic* files (`backup_recovery`, `skeletons`, charts, spectrogram, heatmap — bespoke values that only match the
shared `bnb-*` classes *approximately*, or computed `width/height:%`) change pixels and want a **visual diff**, so
they batch for a Playwright-verified pass; the dynamic ones fold into the endgame `<style>`-nonce slice.

**Progress.**
- **Slice 1 — `admin/settings/render/*` (this PR).** 27 inline style attributes removed across the 8 settings
  section modules; 3 faithful width utilities added (`.bnb-w-num` / `.bnb-w-num-xs` / `.bnb-w-select`);
  page-specific bits folded into the settings page's own `<style>` block. Count **1115 → 1089**. Added the
  `settings_page_has_no_inline_style_attributes` render guard. No CSP change yet (the page still ships a
  `<style>` block, so `'unsafe-inline'` stays until the endgame).
- **Slice 2 — `admin/notifications.rs` + `admin/notification_test.rs`.** 26 inline style attributes removed; both
  old-style standalone pages reach **zero** inline styles via their own `<style>` blocks. The test-result banner
  and the notification stat/status colours were *enumerable*, so they became `.result-banner.ok/.err` and
  `.value.moss/.rare/.dawn` variant classes rather than computed inline styles. Count **1089 → 1063**. Two render
  guards added; still no CSP change. (Deferred: the shared confirm-modal component still emits 2 inline styles.)
- **Slice 3 — `admin/overview.rs` + `admin/logs.rs`.** 11 inline style attributes removed; both old-style pages
  reach **zero** inline styles via their own `<style>` blocks. The overview stat-card colour
  (moss-ink/moss/dawn/rare) was enumerable → a `.value.<tone>` class. Count **1063 → 1052**. Two render guards.
  _Counting caveat:_ `rg 'style="'` also matches `data-confirm-style="…"` data-attributes (e.g. in `rules.rs`) —
  those are **not** inline styles, so the true remaining attribute count is a little lower than the raw `rg` total.
- **Slice 4 (batch) — `admin/species/render.rs` + `admin/migration/render.rs` + `admin/system.rs`.** One larger
  consolidated PR, per the "fewer, larger PRs" steer.
  - `species/render.rs` (both standalone pages + their HTMX fragments) → **zero** inline styles; Pass/Blocked
    badges and filter stats became enumerable variant classes.
  - `migration/render.rs` folds all static + enumerable styles (nav/h1/cards/steps; validation check icons,
    result-card tone, preview table; progress message/track/fill colour) into the page `<style>` block; the only
    remaining inline style is the live progress bar's computed `width:{pct}%`.
  - `system.rs` folds nav/h1/leads/button-rows/result-slots/danger-card + disk/CPU/memory meter chrome into the
    page `<style>` block; badge/temperature colours become enumerable `.meter-*` / `.temp-val` tone classes; the
    only remaining inline styles are the 3 live usage-bar widths.

  Those remaining `width:{pct}%` fills are the **documented dynamic exception** — they move into a nonce'd
  `<style>` block in the endgame. (Raw `rg 'style="'` over the crate still counts both those and the
  `data-confirm-style=` data-attributes, which aren't inline styles.) Workspace raw `style="` total **1052 → 924**.
  Four new render guards (species ×1 covering 3 surfaces; migration ×3). `system.rs` has no guard — both its render
  fns are async over a full `AppState`, disproportionate to mock; covered by the sibling guards + visual review.
  Original render APIs preserved exactly.
- **Slice 5 (batch) — admin harmonization onto `app.css` utility classes.** The admin pages built via the shared
  `admin_shell` (no page `<style>` block) — so, per the O-25 reference design, their target is `app.css` classes,
  not a `<style>` block. Four pages, screenshot-verified:
  - `admin/backup_recovery.rs` → only the 4 computed storage-bar widths remain inline. Reuses the pre-existing
    `bnb-dropzone`/`bnb-danger-zone`/`bnb-logblock` + a scoped `bkr-*` block.
  - `admin/doctor.rs` → **zero** inline styles (`doc-*`).
  - `admin/quality.rs` → only the computed chart bar height/width remain inline (`q-*`); stat-card/empty-state
    tones became enumerable classes.
  - `admin/accounts.rs` + `templates/admin_accounts.html` + the `/admin/audit` page → **zero** inline styles
    (`acct-*`/`audit-*`).

  **Self-verified in-sandbox** using the repo's own visual-QA harness — `cargo run -p birdnet-web --example
  screenshot_server` (seeds ~9.9k synthetic detections) + Playwright (`tools/visual-qa`, chromium at
  `/opt/pw-browsers`). Captured light/dark × desktop/mobile before/after for each page: all pixel-faithful. The
  harness earned its keep — it caught a **mobile-only regression** on the backups page (its grids had been inline
  `style=` and so were collapsed by the global `[style*="grid-template-columns"]{…!important}` reset at
  `app.css:656`; moving them to classes escaped that selector, so the new grids carry their own
  `@media(max-width:520px)` breakpoint). Three render guards on `backup_recovery`. Workspace raw `style="`
  **924 → 787**. fmt + clippy + 311 lib tests green.

  Original render APIs preserved exactly.
- **Slice 6 (batch) — public-page harmonization, pixel-diff verified.** `pages/year_in_review.rs` (→ only the
  computed week-tape colour + leaderboard bar width inline), `pages/weekly_report.rs` and `pages/history.rs`
  (→ zero inline except the computed top-species/SVG bars). The two legacy-token pages (weekly, history) keep
  their `--bg-card`/`--radius`/`--text-muted`/`--accent`/`--success`/`--warning` tokens verbatim — restyling to
  the modern palette is a separate decision, deliberately out of scope for a faithful sweep.

  **Verification bar raised from eyeball to measurement.** Added two durable tools: `tools/visual-qa/shot.mjs`
  (capture one route, light/dark × desktop/mobile) and `tools/visual-qa/diff_pair.mjs` (quantitative before/after
  RGBA pixel diff, exit-coded). Every page verified to **0 differing pixels in its content region**; the only
  residual whole-image diffs are the shared topnav live-status pulse dot and the footer uptime ticker (isolated
  by region-splitting the diff and confirmed against a same-build self-diff that shows 0 content px). The diff
  caught **two real regressions** unit tests never would: (a) year-in-review's leaderboard grid silently
  un-collapsing on mobile (the inline grid had relied on the global phone reset; fixed with a matching
  `@media(max-width:520px)` rule); (b) weekly's disabled "Next" button gaining a border when routed through the
  shared button class, shifting the flex-centred week label ~2400 px across the nav row (fixed with a borderless
  `.wk-nav-next-off` mirroring the original bare span). Workspace raw `style="` **787 → 701**. fmt + clippy +
  311 lib tests green.

- **Slice 7 (batch) — more public pages, batch pixel-diff verified.** `pages/life_list.rs`, `pages/recordings.rs`,
  `pages/dawn_chorus.rs` swept onto scoped `ll-*`/`rec-*`/`dc-*` classes (legacy tokens preserved). Promoted the
  ad-hoc per-page diffing into a reusable runner — `tools/visual-qa/vqa.mjs` (`snap <label> name=/route …` /
  `diff <before> <after>`), content-vs-chrome aware (the topnav pulse-dot + footer ticker are reported separately
  and don't fail the gate). All three pages 0 content-pixel diff on desktop; residuals are the shared chrome and
  the dawn-chorus polar clock (a time-varying SVG, not touched — confirmed by region-splitting away from the
  species rail + a same-build self-diff). Workspace raw `style="` **701 → 661**.

  The runner caught **three** real regressions pre-merge: (a) life-list's search `<input>`/`<select>` lost to the
  global `input[type=…]`/`select` rule on specificity once moved off the inline style (padding 6.4→6 px, font
  14.4→13 px) — fixed by scoping as `input.ll-search`/`select.ll-sort`; (b) dawn-chorus species rows and
  (c) the same inline-grid-vs-phone-reset trap as year-in-review — fixed with matching `@media(max-width:520px)`
  single-column collapses. **Lesson now generalised:** any inline `style=` that was a multi-column grid, or an
  `<input>/<select>`/element the global element-selectors style, must reproduce the original specificity/breakpoint
  when moved to a class — the pixel-diff is what makes that reliably catchable.

- **Slice 8 — `pages/quarantine.rs` + a permanent regression guard.** Swept the inline-heaviest public page
  (filter tabs, 4 stat cards, the review table, the approve/reject/delete/share button group, load-more, the
  pending-count nav badge) onto a scoped `qz-*` block; legacy tokens preserved; **zero** inline styles. Pixel-diff
  verified 0 content px on desktop + light-mobile. The runner caught a **nested-flex** regression (the inner
  button group, originally `flex` *without* `align-items:center`, inherited centering when routed through the
  outer `.qz-actions` — fixed with a distinct `.qz-btn-group`). The dark-mobile residual traced to the browser's
  **native `<audio>` control** rendering its volume widget non-deterministically (inside `<audio controls>`,
  unstyled here) — confirmed by a 0-px same-build self-diff; not a CSS regression.

  **Bar raised — the sweep now defends itself.** New crate test `tests/inline_style_guard.rs` scans all **27**
  swept files and fails if a bare static inline `style="…"` reappears, with a documented allowlist for the genuine
  dynamic exceptions (computed bar width/height, data-driven background/fill, `--sp:` avatar, SVG text fills) and
  `data-confirm-style=` data-attributes. Wiring it surfaced one straggler — dawn-chorus's polar `<svg>` root
  static sizing → a `.dc-polar-svg` class. Workspace raw `style="` **661 → 657** (the remaining quarantine matches
  are `data-confirm-style` attributes).

- **Slice 9 (batch) — the analytics screens, + analytics made default-active in the QA harness.** Swept
  `pages/behavioral.rs` (`/analytics` partials), `pages/correlation.rs`, `pages/timeseries_dash.rs`
  (`/timeseries` partials), `pages/species_pages.rs` (`/species` + `/species/detail`) and
  `pages/detection_detail.rs` onto scoped `bh-*`/`co-*`/`tsd-*`/`spp-*`/`dd-*` classes. Legacy tokens
  (`--text-muted`/`--accent`/`--radius`) preserved verbatim; the eBird/AllAboutBirds + companion links scoped
  `a.spp-link`/`a.spp-inherit` to beat the global `a{color}` rule; the lookup box scoped
  `input.co-species-input`. The only inline styles left are the **3 computed data-bars** (correlation ×2,
  timeseries heatmap ×1 — the documented `width:{pct}%` exception). Also swept two single-quoted
  `style='color:var(--rare)'` error strings the `style="` scan never counted. Workspace raw `style="`
  **657 → 588**; guard grows **27 → 32 files**.

  **Verified with analytics actually running.** The screenshot QA server was analytics-*compiled* but never
  analytics-*active* — it built `AppState` via `from_connection()` (the SQLite-only path), so the gated
  analytics tables never rendered (`Active: false`). Fixed so the swept screens are verified as users see
  them: `birdnet-web` now enables `analytics` **by default** (embedded + invisible, matching the shipped
  binary; the binary depends on it with `default-features = false` and re-enables via its own `analytics`
  feature, so a `--no-default-features` slim build stays DuckDB-free — verified **0 vs 3** duckdb crates in
  the feature graph), and the example reopens through `new_with_analytics()` so DuckDB is opened + synced.
  With analytics live, `correlation`/`species`/`detection-detail` (real SQLite data) and `behavioral` (real
  sessionize/retention/next tables) pixel-diff to **0 content px** on the deterministic variants. The residual
  analytics-light-desktop + all-timeseries diffs trace to **non-deterministic DuckDB row ordering** — the
  next-species/timeseries queries reshuffle tied rows per request (confirmed by a same-build self-diff and by
  highlighting the diff to differing *species names*, not CSS); a pre-existing data-layer wart, **out of P3-3
  scope** (a follow-up should add stable `ORDER BY` tiebreaks). fmt + clippy (`--all-targets`, analytics now
  default) + 311 lib tests + guard all green.

- **Slice 10 — onboarding wizard, + the analytics query-ordering fix folded in.** Swept
  `pages/onboarding.rs` (the standalone `/onboarding` setup flow, which has its own `<style>` block) onto
  scoped `ob-*` classes; the per-`<i>` staggered VU/calibration `animation-delay`s became clean `:nth-child`
  rules, and the lat/lon + notify grids carry their own `@media(max-width:520px)` single-column stacks (they
  had relied on the global `[style*="grid-template-columns"]` reset). **Zero** inline styles; guard **32 → 33**.
  Workspace raw `style="` **588 → 543**.

  **Folded-in fix — analytics tables no longer reshuffle on refresh** (the wart slice 9 surfaced). Now that
  the QA harness runs analytics-active, the timeseries `peak` and behavioral `next-species` tables visibly
  reshuffled on every request. Root causes + fixes:
  - **Peak windows were anchored to wall-clock `CURRENT_TIMESTAMP`** (`birdnet-timeseries`), so every window
    boundary drifted by the seconds elapsed between requests. Now anchored to `max(detection_timestamp)` —
    deterministic *and* more meaningful ("the busiest windows in the last N days of recorded activity") — plus
    an `ORDER BY detection_count DESC, window_start` tiebreak.
  - **`next_species` tied predictions** (`birdnet-behavioral`) came back in non-deterministic order →
    `ORDER BY frequency DESC, predicted_species` makes the top-N selection *and* order stable.

  Verified against the live analytics QA server: all **9** analytics/timeseries partials are now hash-stable
  across repeated requests (previously `ts-peak` + `analytics-next` reshuffled). The one intermittent
  `ts-heatmap` blip was the startup SQLite→DuckDB sync race, not query non-determinism — its
  `COUNT(*) · 1.0 / COUNT(DISTINCT date)` is exact. fmt + clippy (`birdnet-web` / `birdnet-timeseries` /
  `birdnet-behavioral`, `--all-targets`) + lib tests + guard all green.

  **Next:** `pages/skeletons.rs` (~45, pure computed placeholders), the un-swept admin pages
  (`system_controls/*`, `backup`, `rules`, `audio`), and `templates/*`, same runner + guard; then the endgame
  `<style>`-nonce middleware extension that lets the remaining computed widths/colours carry a nonce, after
  which `style-src 'unsafe-inline'` is finally dropped.

- **Slice 11 (batch) — served HTML page templates, pixel-diff verified.** Swept the six biggest served
  `templates/*.html` shells — `today`, `dawn_chorus`, `species`, `species_detail`, `analytics`, `timeseries`
  — onto scoped `td-`/`dc-`/`sp-`/`sd-`/`an-`/`tsh-` classes in `app.css` (`species_detail` folds into its own
  `<style>` block; the `dc-`/`tsh-` prefixes avoid the already-swept `dawn_chorus.rs` `dc-*` and
  `timeseries_dash.rs` `tsd-*` partial classes). Legacy `--text-muted`/`--accent`/`--pad-3` tokens preserved
  verbatim; the two `.grid-2` column overrides (dawn `1.05fr 0.95fr`, species-detail `1.5fr 1fr`) that had
  relied on the global `[style*="grid-template-columns"]` ≤520px reset now carry their own breakpoint; the two
  flex-column stacks in `species_detail` reuse the existing `bnb-col wide` utility (exact match). **Zero**
  inline styles in all six; guard **33 → 39 files**. Workspace raw `style="` **543 → 479**.

  **Verified (analytics-active QA server + Playwright, light/dark × desktop/mobile).** The four deterministic
  pages (`analytics`/`species`/`species_detail`/`timeseries`) pixel-diff to **0 content px** on desktop. The
  `today` and `dawn_chorus` residuals are the documented dynamic content — `today`'s live detection feed
  (`/pages/today-list`, 15-s refresh on time-relative seed data) and `dawn_chorus`'s polar clock + ribbon
  strips (`/pages/dawn-polar`/`dawn-list`) — confirmed by a same-build self-diff (equally large) and a +4-min
  same-build diff (today then drops to ~1.1k px, symmetric light/dark; an unlucky feed-boundary crossing had
  ballooned one dark capture to 178k px), plus a side-by-side image check showing the styled header/grid
  regions pixel-identical and only the dynamic SVG/feed differing. The persistent ~45 px mobile diff is the
  shared topnav/chrome the runner's band doesn't fully exclude at 390 px — identical in the self-diff, not CSS.
  `_empty_states.html` is **not** `include_str!`-served (a dead file) so it is skipped; `share_rare.html` is
  deferred (its `/r/{token}` route needs a signed token the QA server doesn't seed). fmt + clippy
  (`-p birdnet-web --all-targets`, analytics default) + 311 lib tests + guard all green.

  **Next:** the remaining `templates/*` (`share_rare` once the QA server mints a token; `migration`,
  `admin_audio_sources`, `login`, `dashboard`, `listen`; the JS-toggle `recordings`; the cross-cutting
  `layout`/`_partial_*`), the un-swept admin pages (`system_controls/*`, `backup`, `rules` incl. its two
  single-quoted `style='…'`, `audio`), and `pages/skeletons.rs`/`viz.rs`/`charts.rs` (mostly computed, fold
  into the endgame); then the endgame `<style>`-nonce middleware extension after which
  `style-src 'unsafe-inline'` is finally dropped.

- **Slice 12 (batch) — standalone admin & system pages, pixel-diff verified.** Swept eight more screens onto
  scoped classes: `pages/changelog.rs` (`cl-`), `pages/heatmap.rs` (`hm-`), `pages/notification_center.rs`
  (`nc-`), `pages/system_dashboard.rs` (`sys-`, distinct from the slice-11 `sd-*`), `admin/rules.rs` (folded
  into its own `<style>` block — `nav a.here`/`.rule-success`/`.toggle-state.on|off`/`.nowrap`/`.any`/…), and
  the `login.html` / `migration.html` / `admin_audio_sources.html` templates (`login-*`/`mig-`/`aas-`). The
  toggle-state and notification-stat colours became enumerable classes; the migration 3-col `.grid-2` override
  carries its own ≤520px collapse. Legacy `--text-muted`/`--accent`/`--success`/`--danger`/`--warning`/
  `--bg-hover` tokens preserved verbatim. **This also retired the last two single-quoted `style='…'`** (in
  `rules.rs`), so the whole crate is now free of single-quoted inline styles (`rg "style='"` → **0**). Guard
  **39 → 47 files**; workspace raw `style="` **479 → 391** (the residual matches are the documented dynamic
  exceptions in already-swept files + `data-confirm-style=` data-attributes).

  **Verified (analytics-active QA server + Playwright, light/dark × desktop/mobile).** `audio` / `rules` /
  `changelog` / `login` / `notifications` pixel-diff to **0 content px** on every variant; `migration` 0 except
  the shared ~45 px dark-mobile chrome. `heatmap` and `system` residuals are the documented dynamic content —
  heatmap loads the `/pages/dawn-chorus` polar (a wall-clock "now" hand) + many AA'd SVG cells, system shows
  live CPU/memory/temperature gauges — confirmed non-CSS by a same-build self-diff of equal magnitude and a
  side-by-side image check (the styled hero/grid/table regions are pixel-identical). The pixel-diff caught
  **three real specificity regressions** unit tests never would, all the same `.container element`-beats-`.class`
  trap the inline styles used to win on: (a) changelog section headers reverted to `.bnb-help-drawer__body h2/h3`
  (15px + 22px margins ×~70 sections → **+598 px** page height) — fixed by scoping `.cl-body .cl-rel-h2`/
  `.cl-sec-h3`; (b) login's `<h1>` lost to the mobile `.login-card__head .display{font-size:32px}` rule, and the
  centred `.login-main` amplified the 6 px shrink into a ~119 k-px vertical shift — fixed with
  `.login-card__head .login-h1`; (c) the notification stat values lost their colour to `.stat-card .value{color}`
  — fixed with `.stat-card .value.nc-v-*`. fmt + clippy (`-p birdnet-web --all-targets`) + 311 lib tests + guard
  all green.

  **Next:** the remaining admin control fragments (`system_controls/{service,update,data,backup}.rs`,
  `admin/backup.rs`, `admin/audio.rs` row/probe) — mostly POST-triggered result banners, faithfully extractable
  onto a shared `.ctl-*` vocabulary like slices 2–3; the remaining templates (`share_rare` once the QA server
  mints a token, `dashboard`, `listen`, JS-toggle `recordings`, cross-cutting `layout`/`_partial_*`); and the
  computed-heavy `pages/{skeletons,viz,charts,gallery}.rs` (per-element `{…}`/`--sp:` values that fold into the
  endgame). Then the endgame `<style>`-nonce middleware extension after which `style-src 'unsafe-inline'` is
  dropped.

- **Slice 13 (batch) — guard hardened for escaped quotes + the home/today ecosystem swept.** Two parts:

  **(a) Closed a guard blind spot.** Both `rg 'style="'` and the regression guard only matched the *literal*
  `style="`, so inline styles emitted by ordinary Rust `write!`/`format!` string literals — which appear in
  source as the **escaped** `style=\"…\"` — were invisible to both. The guard now scans for the escaped form
  too, with a colon-gate (`payload.contains(':')`) so it skips the legitimate non-styles that contain the
  characters `style=\"`: render-guard search-strings (`html.split("style=\"")`) and escaped data-attribute
  values (`data-confirm-style=\"danger\"`). **Audited every previously-swept file under the hardened guard —
  all 47 still pass**, confirming the earlier slices left no escaped static styles (their `style=\"` hits were
  exactly those two false-positive kinds). Added unit cases for the escaped form. This also surfaced two
  un-inventoried surfaces for later — `pages/detection_reviews.rs` (29) and `admin/images.rs` (24).

  **(b) Swept the home (`/`) + today (`/today`) ecosystem** (the highest-traffic pages): `pages/today.rs`
  (`tdl-`), `pages/today_phrase.rs` (reuses slice-11 `td-h1`/`td-sub`; the per-tier `<em>` colour became the
  enumerable `tp-c-*` set), `pages/health.rs` (`he-`; the computed disk-bar `width:{pct}%;background:{c}` stays
  inline — the documented dynamic exception), `pages/dashboard/stats.rs` (`ds-`; the last-hour value scoped
  `.stat-tile .value.ds-last-hour` to beat `.stat-tile .value`), `pages/dashboard/partials.rs` (`dp-`; feed
  rows / top-species / species-list / most-recent — the `<a>` colour-inherit links scoped `a.dp-*`, the
  list-row column override carries its own ≤520px collapse), and `dashboard.html` (`db-`; reuses `bnb-col wide`).
  Legacy `--text-muted`/`--bg-hover`/`--border`/`--text`/`--radius`/`--fg-2`/`--fg-4`/`--dawn-ink` tokens
  preserved verbatim. Guard **47 → 53 files**; raw `style="` **391 → 354** (escaped forms in these files all
  removed too — verified with `rg -F 'style=\"'`).

  **Verified (analytics-active QA server + Playwright, light/dark × desktop/mobile).** Both pages are
  **visually pixel-identical** before/after. The residual pixel diffs are the documented dynamic content —
  the live detection feeds (`/pages/detections`, `/pages/today-list`, time-relative seed data) plus the home
  hero's **animated live-signal canvas** (a wall-clock breathing baseline, now under pixel-test for the first
  time) — confirmed non-CSS by a same-build self-diff and a longer-gap self-diff whose magnitude scales with
  the capture interval (today desktop's 1218 px before→after ≈ its 991 px same-build feed drift; home's larger
  number tracks the feed+canvas drift over the multi-minute edit gap, maxΔ matching the canvas not a regression).
  fmt + clippy (`-p birdnet-web --all-targets`) + 311 lib tests + hardened guard all green.

  **Next:** `detection_reviews.rs`/`images.rs` (newly surfaced), the admin control fragments
  (`system_controls/*`, `admin/backup.rs`, `admin/audio.rs`), the remaining templates (`share_rare`, `listen`,
  JS-toggle `recordings`, `layout`/`_partial_*`), and the computed-heavy `skeletons`/`viz`/`charts`/`gallery`
  that fold into the endgame; then the `<style>`-nonce middleware extension after which `style-src
  'unsafe-inline'` is dropped.

- **Slice 14 — the escaped-quote files the hardened guard surfaced.** Swept the two screens that slice 13's
  guard hardening exposed (their inline styles were emitted only as escaped `style=\"…\"`, so every prior
  inventory missed them): `pages/detection_reviews.rs` (`dr-*`; `/detection-reviews` + the inline review widget
  on `/detections/detail`) and `admin/images.rs` (`img-*`; `/admin/images` — a standalone old-style page with
  no `<style>` block, so its inline styles fold onto `app.css` like `share_rare`). The `<a>` colour links
  (`--primary`/`--fg`) were scoped `a.dr-link`/`a.dr-row-name` and the form inputs `input.img-input*`
  preemptively (from the slice-12 specificity lessons); enumerable verdict/badge tones reuse the `bnb-pill`
  variants. Legacy `--fg`/`--fg-2`/`--fg-3`/`--primary`/`--danger`/`--moss`/`--bg`/`--hairline`/`--text-muted`/
  `--card-bg`/`--border`/`--radius`/`--accent`/`--bg-hover` tokens preserved verbatim. Guard **53 → 55 files**.
  This removed **52 escaped inline styles** (`rg -F 'style=\"'` over `src` **95 → 43**) — the raw `style="`
  count is *unchanged* at 354, which is exactly the point: these files were invisible to the raw scan, and the
  hardened guard is what made them findable and keeps them swept.

  **Verified — the cleanest result of the sweep so far.** Both screens pixel-diff to **0 content px on all four
  variants** (light/dark × desktop/mobile) — no dynamic-content caveat needed (the only non-zero numbers are
  the excluded topnav/footer chrome). The preemptive `a.`/`input.` scoping meant zero specificity regressions
  this round. fmt + clippy (`-p birdnet-web --all-targets`) + 311 lib tests + hardened guard all green.

  **Next:** the admin control fragments (`system_controls/{service,update,data,backup}.rs`, `admin/backup.rs`,
  `admin/audio.rs`, `admin/mod.rs`) — a coherent `.ctl-*` result-banner batch; then the remaining templates
  (`share_rare`, `listen`, `recordings`, `layout`/`_partial_*`) and the computed-heavy
  `skeletons`/`viz`/`charts`/`gallery`. With those done the static sweep is essentially complete and the
  endgame (`<style>`-nonce middleware + drop `style-src 'unsafe-inline'`) is ready for design sign-off.

- **Slice 15 (large batch) — the complete admin control surface + cross-cutting layout/partials + listen.**
  18 files in one PR. The 7 admin files (`system_controls/{service,update,data,backup}.rs` + `admin/backup.rs`
  + `admin/audio.rs` + `admin/mod.rs`) were swept by a **delegated sub-agent** following the established
  conventions (shared `.ctl-*` result-banner vocabulary; `bk-*` backup list; `aud-*` edit form scoped
  `input.`; `a.am-nav-active` scoped to beat the global `a{}`; appended at app.css end so `.aud-edit-*`/
  `.am-signout-btn` win their cascade over `.audio-source__*`/`.bnb-btn`); I integrated its CSS and
  centrally verified. The remaining 11 templates I swept directly: `listen.html` (`ls-*`; the animated
  spectrogram canvas, `select.ls-select` scoped, the always-hidden `<audio>`), the shared `layout.html`
  (`lay-*` — logo/vrule/inline-flex pills, **on every page**), and the 9 `_partial_*` (`ft-`/`tb-`/`tm-`/
  `ck-`/`ub-`/`hd-`/`cm-`/`asr-`/`dpf-` — footer dot, the `<dialog>` headings, the display-prefs labels,
  the audio-source-row remove button). Legacy tokens preserved verbatim. Guard **55 → 73 files**; raw
  `style="` **354 → 270**.

  **Verified (analytics-active QA server + Playwright, light/dark × desktop/mobile; before-baseline captured
  via `git stash`).** `/admin/audio` and `/admin/system/backups` pixel-diff to **0 content px on all four
  variants** — and since they render through the same swept `layout.html` + partials, that *proves the
  cross-cutting sweep is faithful* (a layout regression would hit every page). The non-zero pages all carry
  live content — `/admin/system`'s service-status table (PID/uptime/memory), `/system`'s CPU/mem/temp gauges,
  `/today`'s feed, `/listen`'s animated canvas — confirmed non-CSS by a same-build self-diff of equal
  magnitude, plus an image check of `/admin/system` showing the `ctl-*` status table + `am-nav-active` shell
  nav render correctly. fmt + clippy (`-p birdnet-web --all-targets`) + 311 lib tests + hardened guard green.

  **Next (the static sweep's last mile):** the remaining page render-modules — `pages/migration.rs`,
  `audio_player.rs`, `dashboard/kiosk.rs`, `confirm.rs`, `pages/mod.rs`, `empty_states.rs`, `share.rs` — plus
  the JS-toggle `recordings.html` and `share_rare.html` (needs the QA server to mint a share token). After
  that the only inline styles left are the genuinely-computed ones in `skeletons`/`viz`/`charts`/`gallery`/
  `atoms` + the allowlisted bar widths in already-swept files — which is exactly the input to the **endgame**:
  emit them as per-request nonce'd `<style>` blocks, add the `<style>`-nonce injector sibling to
  `inject_script_nonce`, mirror the nonce into `style-src`, and drop `'unsafe-inline'`.

- **Slice 16 — the static sweep's last mile (sub-agent + 2 templates) + a significant CSP finding.** Swept
  the 5 remaining clean page render-modules via a delegated sub-agent — `pages/audio_player.rs` (`ap-*`),
  `dashboard/kiosk.rs` (`ki-*`), `pages/mod.rs` (`pm-*`; sign-out + 404 page), `empty_states.rs` (`es-*`, one
  heading/subtext class reused ×6), `share.rs` (`sh-*`; the "clip is gone" error page) — plus, by hand, the two
  tricky templates: `recordings.html` (`rt-*`; the `display:none`/`switchTab` toggling rewritten to a
  `.rt-hidden`/`.active` **class toggle**, and recordings.rs's `onclick` `style.display=` → `classList`) and
  `share_rare.html` (`sr-*`; standalone share page, the about-grid carrying its own ≤520px collapse). The
  `audio_player` play/pause icon was likewise converted from `.style.display=` to a `.ap-hidden` class toggle
  so the file is inline-style-free. Guard **73 → 80 files**; raw `style="` **270 → 203**. **The static sweep is
  now essentially complete** — every remaining inline style is genuinely computed (`viz`/`charts`/`gallery`/
  `skeletons`/`atoms`/`migration` SVG fills + `--sp` colours, `confirm.rs`'s interpolated style) or trivial
  (the `security.rs` CSP doc-comment; dead un-served `_empty_states.html`).

  **Verified (QA server + Playwright; before-baseline via `git stash` of just the rendering files; a temporary
  `/r/<token>` mint in the example, reverted before commit, + a fixed `BNB_SHARE_SECRET` for a stable URL).**
  `/recordings`, the `/r/<token>` **share page** (the 24-style `share_rare`), the share **error** page, and the
  **404** page all pixel-diff to **0 content px on all four variants**.

  **⚠️ Significant finding (pre-existing, NOT this sweep) — P2-2's `script-src` blocks inline `on*=` handlers.**
  A click-test of the recordings tabs failed; the browser console shows *"Refused to execute inline event
  handler because it violates ... script-src 'nonce-…' 'strict-dynamic'."* The shipped P2-2 script-half
  (`script-src 'nonce' 'strict-dynamic'`, no `'unsafe-inline'`) blocks **every inline event handler**
  (`onclick`/`onchange`/`onmouseover`/`onerror`/`oninput`), so interactions on ~15 files — recordings tabs,
  the audio player, heatmap day-switcher, gallery, rules' action select, admin system controls, quarantine,
  etc. — silently no-op under CSP. P2-2's spot-check passed because the interactions it exercised (theme
  toggle, command palette) use `addEventListener` in nonced `<script>`s, which work. The fix is to convert
  inline `on*=` handlers to `addEventListener` (or nonced inline scripts). **Scope/approach is a decision for
  the maintainer** — tracked separately from the style-src endgame.

- **Slice 17 — inline `on*=` handler conversion (resolves slice 16's CSP finding; its own PR).** Converted
  **every** inline DOM event handler the slice-16 finding flagged — `onclick`/`onchange`/`oninput`/
  `onmouseover`/`onmouseout`/`onerror` — to `addEventListener` / CSS / `data-*` delegation across **16 files**,
  so the shipped `script-src 'nonce' 'strict-dynamic'` no longer silently no-ops interactions. A broad grep
  (`[[:space:]]on[a-z]+=`, excluding HTMX `hx-on`) now returns **zero** matches across `crates/birdnet-web/src`
  + `templates`. Patterns:
  - **Two global delegated listeners in `layout.html`** (one nonced `<script>`, present on every page): a
    capture-phase `error` listener that hides any `img[data-hide-on-error]` that fails to load (replaces four
    `onerror="this.style.display='none'"`), and a `click` listener for copy-to-clipboard buttons keyed by
    `data-copy-url` (origin + path) or `data-copy-from` (`#id` textContent) with an optional
    `data-copied-label` — replacing the three bespoke clipboard IIFEs (detection-detail share + correlation-id,
    quarantine row share).
  - **Range day-switchers** (`heatmap.rs`, `correlation.rs`): `onclick="loadDays(N,this)"` → `data-days="N"`
    + one delegated listener on the existing `#range-controls`.
  - **Tabs / rows**: `recordings.html` wires `#tab-by-species`/`#tab-by-date` via `addEventListener` plus a
    delegated `.rec-item` listener that un-hides `#recordings-detail` (replacing recordings.rs's two row
    `onclick`s); `migration/render.rs` `.tabs` → `data-tab` + delegated listener (the `switchTab` body's
    `event.target` swapped for a `[data-tab]` query so it no longer needs the implicit global `event`).
  - **Media players** (`audio_player.rs`, `livestream.rs`): play/seek/volume/speed/download wired by id at the
    end of the existing scripts (`oninput`/`onchange` preserved via `this.value` wrappers).
  - **Form toggles**: `rules.rs` action-type `<select>` → `change` listener (CSSOM `.style.display`, allowed
    under CSP); `admin_audio_sources.html` four `<details>` open/cancel buttons via a `wireAddForm` helper;
    `admin_accounts.html` cancel button.
  - **Gallery hover** (`gallery.rs`): `onmouseover`/`onmouseout` transform/shadow → a `.ga-hover:hover` rule in
    `app.css` (the card keeps its inline transition; gallery's *computed* `{color}` styles stay deferred to the
    endgame); its broken-image `onerror` → `data-hide-on-error`.

  **Verified (QA server + Playwright with a `securitypolicyviolation` listener installed pre-navigation).** A
  CSP sweep of **15 pages** (`/`, `/today`, `/species`, `/heatmap`, `/migration`, `/analytics`, `/life-list`,
  `/quarantine`, `/system`, `/recordings`, `/correlation`, `/listen`, `/admin/rules`, `/admin/migrate`,
  `/admin/system/logs`) reports **zero** CSP violations. Behavioural click-tests pass: the recordings **tab
  switch + row click** (the exact interaction that failed in slice 16) now toggles the views and un-hides the
  detail panel; the heatmap/correlation range buttons fire; the detection-detail **share** button flips to
  "Link copied" with a real `/r/<token>` URL on the clipboard; the quarantine share button flips to "Copied";
  an injected `img[data-hide-on-error]` resolves to `display:none`. fmt + clippy (`--all-targets`) + **311**
  lib tests + the inline-style guard all green. No CSP directive change in this PR (handlers only) — the
  `style-src 'unsafe-inline'` drop remains the endgame's job, now unblocked of its last behavioural hazard.

- **Slice 18 — the endgame: drop `style-src 'unsafe-inline'`. ✅ This completes P2-2 and P3-3.** Flipped
  `style-src` to **`'self' 'nonce-{random}'`** and eliminated *every* remaining inline `style="…"` attribute.
  The decisive constraint, verified in-browser first: per-request nonces mean a nonced `<style>` works in a
  full-page render but is **blocked** when it rides an HTMX-swapped fragment (the fragment response mints its
  own nonce ≠ the host page's, and there is no `'strict-dynamic'` for styles). So rather than nonce'd `<style>`
  blocks, **computed** styles move to a `data-style` attribute applied via CSSOM — `el.style.setProperty(…)`,
  which CSP does not police — by a tiny applier the **security middleware injects into every full document**
  (so none of the ~20 page shells can forget it) that also re-runs on `htmx:afterSwap`; **static** styles
  become `app.css` classes.
  - **Mechanism** (`security.rs`): `inject_style_nonce` (sibling of `inject_script_nonce`) stamps the nonce on
    `<style>` *elements*; `inject_dyn_style_applier` adds the CSSOM applier before `</body>`;
    `inject_htmx_config` adds `<meta name="htmx-config" content='{"includeIndicatorStyles":false}'>` so htmx
    stops appending its own un-nonceable indicator `<style>` (those rules already live in `app.css`).
  - **Conversions**: `atoms.rs` (avatar `--sp`, conf-bar `width`, waveform `height` → `data-style` — these
    three alone cover most feed/list dynamic styles); `viz.rs`/`charts.rs` (static SVG-wrapper layout → `viz-`/
    `cht-` classes, legend swatch colours → `data-style`; SVG `fill=`/`stroke=` are presentation attributes,
    untouched); `gallery.rs` (full `ga-*` card vocabulary + computed `color-mix` swatch / code colour →
    `data-style`); `migration.rs` (`mig-*` SVG `<text>` label classes + computed eyebrow colour); `skeletons.rs`
    (uniform `data-style` with a `.bnb-skel-bars>span{height:45%}` FOUC default); `heatmap_widget.rs` (its
    `<style>` block **moved to `app.css`** — a fragment can't nonce one); plus ~16 smaller renderers via two
    delegated sub-agents. Workspace served-HTML inline `style="` count → **0**.
  - **Guard rewritten** (`tests/inline_style_guard.rs`): from an allowlist of swept files to a **crate-wide**
    scan of every `src/**/*.rs` + `templates/**/*.html` that forbids **any** real inline `style=` (the dynamic
    allowlist is gone — `data-style`/`data-confirm-style`/search-strings are skipped; the dead, never-served
    `_empty_states.html` is excluded).
  - **QA**: wired a Wikipedia-backed `ImageCache` into the `screenshot_server` example so the gallery/species
    photos actually populate during visual QA (previously the endpoint returned "image cache not configured"
    and only code placeholders showed) — unrelated to CSP, but it surfaced while verifying the gallery card.

  **Verified (headless Chromium + Playwright, `securitypolicyviolation` listener).** **0** CSP violations
  across **20 pages** *and* their HTMX fragment swaps (feed, heatmap grid, gallery grid, recordings tabs,
  range buttons). The `data-style` applier is confirmed on regular pages, **standalone admin shells** (via the
  middleware injection), and **swapped-in fragments** (avatar `--sp`, conf-bar widths, gallery swatch + code
  colours, `/admin/system` usage bars all apply). Gallery (grid + tinted swatches + real photos), migration
  (ridgeline + all SVG labels), dashboard (avatars/conf-bars/waveforms) and `/admin/system` (usage bars)
  screenshot-checked for parity. `fmt` + `clippy --all-targets` (0 warnings) + **314** lib tests + the
  crate-wide guard all green.

---

## P3-4 — Minor cosmetics  · **P3** _(uptime pill ✅ wired; migration-missing out of scope)_

- **Topnav uptime pill** — ✅ **wired.** `render_page_inner` now fills `{{uptime_short}}` from
  `system_info::process_uptime_secs()` formatted by `format_uptime`. No state-plumbing was needed after
  all: the `/proc/self/stat` process-uptime logic already lived in `health.rs`, so it was promoted to
  `system_info` (returning `Option<u64>`) and reused — which also DRY'd `health.rs` and fixed its latent
  fallback (it previously returned wall-clock epoch seconds, now `0`/`None`). Empty when unavailable
  (non-Linux / `/proc` unreadable), so the O-26 `[data-empty-hide]` rule still hides the pill.
- **Migration "missing species" comparison stub.** **Verified** at `routes/pages/migration.rs:616`
  (`"missing" => None, // requires comparative model — stubbed.`): predicting which species *should* be
  present but are absent genuinely needs a baseline/forecast model that does not exist — **out of
  scope**, correctly deferred (the widget already falls back to "Forecast model pending").

---

## P3-5 — Image blacklist enforcement on the read path  · **P3** — ✅ DONE

**✅ Shipped** (chose layer (a): enforce in the web handlers, keeping `birdnet-integrations`
network-/DB-agnostic). The admin blacklist was inert — `/file`, `species_image_info`, and the gallery
warmer all fetched/served without consulting it. Now:
- `species_image_file` checks the resolved URL against `is_image_blacklisted`; a blacklisted hit 404s
  **and** evicts the cached file. The check only runs when a URL is known (image fetched/warmed this
  session), so warm disk-cached gallery loads add **zero** DB queries.
- `add_blacklist` purges the species' cached file on insert, so the next `/file` re-fetches and is
  refused while the URL stays blacklisted (covers images cached before the blacklist entry).
- New `ImageCache::remove` / `DiskCache::remove`.
- Tests: `DiskCache::remove` unit test + `web_api_images::species_image_file_respects_blacklist`
  (fetch → blacklist → 404 + cached file evicted).

**Remaining (minor follow-ups, filed for later):**
- **URL not persisted across restart.** `DiskCache::scan` rebuilds the index with `url=""`, so an image
  cached *before* a blacklist entry and never re-fetched can't be matched by URL at serve time — handled
  for the realistic flow by purge-on-blacklist, but a `{key}.url` sidecar (written in `update_metadata`,
  read in `scan`) would make serve-time enforcement airtight. Low value.
- **`doctor::paths` default image path.** Still reports the image cache only when explicitly configured;
  after BUG-1's default-on it should also report the resolved `<db_dir>/images`. XS; needs `db_path`
  threaded into `check_paths` (+ a one-line test update).
- **`species_image_info`** (JSON metadata) isn't blacklist-aware; the display path (`/file`) is, which
  is what gates what users see.

---

## Out of scope (documented as deliberately deferred)

From the v2 design proposals (since shipped and removed from the repo — see git history) — captured here so they aren't mistaken for gaps:
- **Multi-station compare** ("your station vs neighbour's") — needs a storage/shape decision.
- **Web Push** — O-24 shipped PWA bones (manifest, service worker, icons) but not Web Push; needs a
  server-side push store + key-rotation story distinct from the session model.
- **Custom species images** — `BIRDNET_CUSTOM_IMAGE_DIR` (`.env.example:223`) overrides the Wikipedia
  cache; still works after BUG-1 — `species_image_file` checks the custom dir first (unchanged), then
  falls through to the now-fetch-on-miss Wikipedia cache.

---

## Audit greps (re-run to refresh this doc)

```bash
# Shipped history (integration branch):
git log origin/claude/gallant-feynman-bJs95 --oneline | head -60
# Deferred-work markers:
rg -n -i 'TODO|FIXME|stub|not wired|later pass|follow-?up|O-15-followup|O-13-followup' crates/ src/ -g '*.rs'
# Help-link coverage:
rg -l 'help_link\(' crates/birdnet-web/src/routes/pages/
# Inline-style size (O-25):
rg -c 'style="' crates/birdnet-web/ -g '*.rs' -g '*.html'
# Image-cache wiring:
rg -n 'image_cache|with_wikipedia|IMAGE_CACHE_DIR' src/ crates/birdnet-web/src/ -g '*.rs'
```
