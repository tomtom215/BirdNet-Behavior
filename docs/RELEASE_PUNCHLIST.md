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

**Conventions:** see `CLAUDE.md` — `unsafe` is denied workspace-wide (so no `std::env::set_var`
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
| **P2-2** | CSP `script-src` hardening — ✅ **DONE** (per-request nonce + `'strict-dynamic'`, browser-verified); `style-src` half tracked by P3-3 | P2 | M | med |
| ~~**P2-3**~~ | Extend help links to remaining analytical screens — ✅ **DONE** (6 screens) | ~~P2~~ | S | low |
| **P3-1** | O-13 legacy `--audio-source` retirement — ✅ **DONE** | P3 | S | low |
| ~~**P3-2**~~ | No background session pruning — ✅ **DONE** (daily maintenance tick) | ~~P3~~ | S | low |
| **P3-3** | O-25 inline-style sweep (unlocks P2-2 style-src) | P3 | L | low (tedious) |
| **P3-4** | Minor cosmetics — uptime pill ✅ **wired**; migration-missing out of scope | P3 | XS | none |
| ~~**P3-5**~~ | Image blacklist enforcement on read path — ✅ **DONE** (serve-check + purge-on-blacklist) | ~~P3~~ | S | low |

Recommended order: ~~BUG-1 → P1-1 → P2-1 → P2-3 → P3-2 → P3-5 → P3-1 → P2-2 (script half)~~ (shipped) → **P3-3** (inline-style sweep, then drop `style-src 'unsafe-inline'` to finish P2-2). _Also shipped: ONNX offline build tooling (SessionStart hook). Remaining low-value/deferred: P3-4 cosmetics._

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

## P2-2 — CSP `script-src` hardening  · **P2** — ✅ DONE (script half; style half tracked by P3-3)

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
- `style-src 'unsafe-inline'` is intentionally unchanged — dropping it still depends on the **P3-3**
  inline-style sweep.

**Verified (headless Chromium via Playwright).** Swept all 32 screens + interactions (command palette,
theme toggle, live admin stats): **0 CSP violations**, header nonce == body nonce per request, static
JS/JSON passed through byte-identical, every page renders intact. `curl` shows the tightened header.
**Risk retired:** the "a missed inline script breaks UI" worry is gone — the injector is exhaustive and
browser-verified. **Still depends on:** P3-3 (for the style half only).

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

**Evidence.** ~**991** `style="…"` occurrences across **88** files (`rg -c 'style="' crates/birdnet-web/`).
Cosmetic on its own, but removing them is the prerequisite for dropping `style-src 'unsafe-inline'` (P2-2).
Reference design: `docs/proposed_changes/O-25_inline_styles/` (utility classes in `css/app.css.append`).

**Fix.** **Do not** attempt in one PR. Agree a utility-class vocabulary, then sweep **one area per PR**
(e.g. admin settings renderers first), substituting inline `style=` for classes. Track coverage so the
final PR can drop `'unsafe-inline'` from `style-src`. **Verify per PR:** visual diff unchanged; occurrence
count drops. **Effort:** L (many small PRs). **Risk:** low but tedious.

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

From `docs/proposed_changes/README_v2.md` — captured here so they aren't mistaken for gaps:
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
