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
- **Working branch:** `claude/dazzling-wright-60OjQ` — commit here.
- **Integration branch:** `claude/gallant-feynman-bJs95` — open every PR with this as the **base**.
- `main` is the old release branch (stuck at `#86`); **do not** target it.

Per-task cycle:
1. Ensure the working branch is at the integration tip:
   `git fetch origin claude/gallant-feynman-bJs95 && git reset --hard origin/claude/gallant-feynman-bJs95`
2. Commit the change on `claude/dazzling-wright-60OjQ`.
3. `git push --force-with-lease -u origin claude/dazzling-wright-60OjQ`
   (force-with-lease is expected — the working branch is rewritten each cycle after the prior squash-merge).
4. Open a PR: head `claude/dazzling-wright-60OjQ` → base `claude/gallant-feynman-bJs95`.
5. After it squash-merges, go back to step 1.

**Gate (run all before opening a PR — there is _no_ CI on this repo, so this is the only gate):**
```bash
cargo fmt --check --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --lib
# plus any integration test you touched, e.g.:
cargo test --test web_api_admin
```
Note: the repo's historical gate was `--lib` only, which **skips `tests/`** (they link
libonnxruntime). Run the relevant `--test <name>` explicitly so integration-test rot
can't hide (this is how the dead Basic-Auth test in #113 had gone stale).

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
| **BUG-1** | Bird images don't populate gallery/previews | **P1** | M | low–med |
| **P1-1** | Dead "Reset password" button (admin accounts) | **P1** | S | low |
| **P2-1** | Stale `accounts.rs` module doc | P2 | XS | none |
| **P2-2** | CSP still allows `'unsafe-inline'` | P2 | M | med |
| **P2-3** | Extend help links to remaining analytical screens | P2 | S | low |
| **P3-1** | O-13 legacy `--audio-source` retirement | P3 | S + decision | low |
| **P3-2** | No background session pruning | P3 | S | low |
| **P3-3** | O-25 inline-style sweep (unlocks P2-2 style-src) | P3 | L | low (tedious) |
| **P3-4** | Minor cosmetics (uptime pill, migration compare) | P3 | XS | none |

Recommended order: **BUG-1 → P1-1 → P2-1 (fold into P1-1) → P2-3 → P2-2/P3-3 → P3-1 (needs your call) → P3-2 → P3-4.**

---

## BUG-1 — Bird images don't populate the gallery or previews  · **P1**

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

## P1-1 — Dead "Reset password" button in admin accounts  · **P1**

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

## P2-1 — Stale `accounts.rs` module doc  · **P2**

**Evidence.** `accounts.rs:8-13` still says *"until the auth wire is flipped the request-time user is
the seed admin row … see the `TODO(O-15-followup)` comments below for the call sites that need
`require_admin`."* The wire is flipped (#96), RBAC is centralised in the cookie middleware (#112), and a
grep confirms **no such `TODO` comments and no `require_admin` call remain in this file**. Same stale-doc
class #112 reconciled in `session.rs`/`auth_pages.rs`; this file was missed.

**Fix.** Rewrite the header to describe the live state (cookie middleware gates `/admin`, writes are
admin-only via the central RBAC check). **Verify:** `cargo doc` + read-through. **Effort:** XS. **Risk:** none.

---

## P2-2 — CSP still allows `'unsafe-inline'`  · **P2**

**Evidence.** `crates/birdnet-web/src/security.rs:21-31` — `style-src 'unsafe-inline'` and
`script-src 'unsafe-inline'`, with the comment *"Tighten to nonce/hash-based `script-src` in a later
pass."* For a LAN-exposed admin this is the main remaining hardening item.

**Fix.** Move `script-src` to per-response **nonces** (generate a nonce in a layer, thread it into the
few inline `<script>` bootstraps + templates, drop `'unsafe-inline'` for scripts). Dropping it for
**styles** requires removing inline `style=` attributes first ⇒ **depends on P3-3 (O-25)**. Ship the
script-side first; style-side after O-25.

**Verify.** Browser console shows no CSP violations across pages; `curl -I` shows the tightened header;
no inline script executes without the nonce. **Effort:** M. **Risk:** med (a missed inline script breaks UI). **Depends on:** P3-3 (for the style half).

---

## P2-3 — Extend help links to the remaining analytical screens  · **P2**

**Evidence.** `help_link(Topic::…)` is wired on 12 screens (dashboard, today, heatmap, dawn_chorus,
life_list, recordings, quarantine, migration, notification_center, weekly_report, year_in_review, help).
The analytical screens **without** it: `correlation`, `behavioral`, `history`, `species_pages`,
`timeseries_dash`, `system_dashboard` (`crates/birdnet-web/src/routes/pages/`). README_v2 explicitly
deferred these "cross-screen edits" to follow-ups.

**Fix.** Apply the proven #104 pattern: add a `Topic` variant per screen (if missing) and a
`help_link(Topic::…)` in each page header. Same shape as the shipped batch — low-risk. **Verify:** each
screen shows the "How this works" affordance opening the right mdBook section. **Effort:** S. **Risk:** low.

---

## P3-1 — O-13 legacy `--audio-source` retirement  · **P3 (needs a product decision)**

**Status:** _not a bug._ The capture daemon already reads the `audio_sources` table first
(`src/capture.rs:138 resolve_sources_from_db`), with the legacy single-string CLI/env `--audio-source`
as fallback (`capture.rs:135-137`). Fully functional.

**Remaining:** retiring the single-string fallback **deprecates the BirdNET-Pi-compat `--audio-source`
flag** — flagged in #111 as a **product decision**, not code. **Decide first**, then the cleanup is small
(remove `state.audio_source()` readers + the CLI/env flag, update docs/migration). **Effort:** S after the decision. **Risk:** low.

---

## P3-2 — No background session pruning  · **P3**

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

## P3-4 — Minor cosmetics  · **P3**

- **Topnav uptime pill unwired.** `crates/birdnet-web/src/routes/pages/mod.rs:~184` sets
  `{{uptime_short}}` to `""` (hidden via the O-26 `[data-empty-hide]` CSS). Wire it to the real uptime
  from the system snapshot, or remove the pill. Effort XS.
- **Migration "missing species" comparison stub.** Reported by the code sweep at
  `routes/pages/migration.rs:616` (a `missing` field hardcoded `None`, "requires comparative model").
  _Not independently verified — confirm before acting._ Low value.

---

## Out of scope (documented as deliberately deferred)

From `docs/proposed_changes/README_v2.md` — captured here so they aren't mistaken for gaps:
- **Multi-station compare** ("your station vs neighbour's") — needs a storage/shape decision.
- **Web Push** — O-24 shipped PWA bones (manifest, service worker, icons) but not Web Push; needs a
  server-side push store + key-rotation story distinct from the session model.
- **Custom species images** — `BIRDNET_CUSTOM_IMAGE_DIR` (`.env.example:223`) overrides the Wikipedia
  cache; verify this path still works after any BUG-1 changes.

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
