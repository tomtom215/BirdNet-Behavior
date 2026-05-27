# Handoff · v2.0 · BirdNet-Behavior PR package

Apply order, cheat-sheet bulk-copy, and one-pass acceptance checks for the thirteen-PR set documented in `README_v2.md`. Each opportunity is independently revertable — the goal is a clean, surgical merge wave by wave.

## What ships in this package

Wave A — foundations (no Rust changes that block elsewhere; unblocks everything else):

- `O-16` skeletons — `pages/skeletons.rs` + CSS shimmer
- `O-17` confirm modal — `pages/confirm.rs` + `_partial_confirm_modal.html` + CSS
- `O-18` toast — `pages/toast.rs` + `_partial_toast_region.html` + CSS
- `O-25` inline-style audit — layout utility classes
- `O-26` IA fix — More-dropdown in topnav, real footer (replaces the 12-link flex-wrap)

Wave B — access (RFC sign-off required before merge):

- `O-14` branded login + cookie session
- `O-15` accounts & sessions surface

Wave C — primary feature:

- `O-13` audio sources CRUD (replaces the existing `/admin/audio` stub)

Wave D — independent surfaces:

- `O-19` command palette
- `O-20` help / methodology
- `O-21` changelog viewer + post-upgrade banner
- `O-22` quality dashboard audit

Wave E — context + reach:

- `O-23` signal overlay (weather + moon; SPL design-only)
- `O-24` mobile / PWA

## Bulk apply — cheat sheet

The folder layout under each `O-NN/` mirrors the target paths in `crates/birdnet-web/` and `crates/birdnet-db/`, so a recursive copy lands files in the right place. The patterns:

```bash
# Drop one opportunity's drop-in files into the repo (run from the repo root):
WAVE=O-17_confirm_modal
SRC=path/to/proposed_changes/${WAVE}

# Templates and Rust modules — drop straight in.
cp -R ${SRC}/templates/.   crates/birdnet-web/templates/
cp -R ${SRC}/src/.         crates/birdnet-web/src/routes/pages/

# CSS appends — cat onto the live stylesheet (idempotent because every block is
# clearly commented; re-running is safe but unnecessary).
cat ${SRC}/css/app.css.append >> crates/birdnet-web/static/css/app.css

# Migrations (O-13, O-15) — copy into birdnet-db's migrations dir.
[ -d ${SRC}/migrations ] && cp -R ${SRC}/migrations/. crates/birdnet-db/migrations/
```

A full-wave apply is the same loop over each `O-NN` in the wave's list.

After copying, the only manual edits left are the layout / mod.rs patches called out in each DIFF.md (one-liners — see the `## Patch` rows in each opportunity's table).

## Wave A · foundations

These four land first because every subsequent opportunity references them.

1. **O-16 skeletons.** New file `crates/birdnet-web/src/routes/pages/skeletons.rs`. Append the CSS. Add `pub(crate) mod skeletons;` to `pages/mod.rs`. Then start replacing `<p class="bnb-meta">Loading…</p>` placeholders — see the migration table in `O-16/DIFF.md`.
2. **O-17 confirm modal.** Drop `_partial_confirm_modal.html` into `templates/`, `confirm.rs` into `pages/`. Include the partial near the end of `layout.html` (one line). Then sweep every `hx-confirm="…"` and rewrite the call site as documented in `O-17/DIFF.md`. The native confirm path stays as a fallback — partial migrations are fine.
3. **O-18 toast.** Same shape: partial + Rust helper + CSS + include in `layout.html`. Then attach toasts to successful admin POSTs at the sites listed in `O-18/DIFF.md`.
4. **O-25 inline-style audit.** Append the utility classes to `app.css`. Then the listed Rust handlers can substitute their inline `style="…"` strings for `class="bnb-row between"` etc. Per-file substitution; revertable.
5. **O-26 IA fix.** Drop the two new partials into `templates/`. In `layout.html` replace the existing `<footer>` block with `{{include _partial_footer.html}}`, and inject `{{include _partial_topnav_more.html}}` immediately before the theme-toggle button inside `.topnav-right`. Append the CSS. The twelve destination links currently in the footer are now grouped inside the More dropdown.

## Wave B · access

These need an engineering decision before merge: the session-cookie shape in `O-14/DIFF.md`, then `O-15` builds on it. Once the model is locked:

1. **O-14 login.** Drop in `login.html`, write the new `auth_pages.rs`, update `auth.rs` to the cookie path. Switch `BNB_SESSION_SECRET` env var on. Verify the login flow end-to-end against the existing rate limiter.
2. **O-15 accounts.** Apply the SQLite migration (009). Add the new template and Rust module. The new `/admin/accounts` link goes in the `admin_shell` nav (already shipped).

## Wave C · primary feature

1. **O-13 audio sources.** Apply migration 008. Wire `birdnet-db::audio_sources::AudioSourceStore` (the trait stub in `src/routes/admin/audio.rs` shows what the real impl needs to expose). Replace `audio.rs` with the proposed version. The page now reads from the new table; existing `state.audio_source()` continues to return the device id of the first non-disabled row for backwards-compat with the audio daemon.

## Wave D · independent surfaces

These four can land in any order:

- **O-19 cmdk** — `pages/cmdk.rs` + partial + CSS, plus `include` in `layout.html`. New `/pages/cmdk` route.
- **O-20 help** — `pages/help.rs` + drawer partial + CSS, plus a `build.rs` step to build the mdBook into `target/help/` and serve it from `/help/`.
- **O-21 changelog** — `pages/changelog.rs` + banner partial + CSS, plus the `include` in `layout.html`.
- **O-22 quality** — apply the skin pass to `admin/quality.rs` and add the two new model-trust queries.

## Wave E · context + reach

- **O-23 signal overlay** — `pages/overlays.rs` + CSS. Weather table migration (010). Open-Meteo client in `birdnet-integrations`. SPL design-only — wait for the audio daemon column.
- **O-24 mobile / PWA** — manifest + service worker + tab-bar partial + CSS. Generate the three PNG icons from `BrandMark` SVG.

## Acceptance — what to verify per wave

A full set lives in `VERIFY.md` (TBD per opportunity). The shortest version per wave:

- **Wave A.** Every previously-confirming destructive action now opens the themed modal AND still works if JS is off. Skeletons swap to real content on htmx response. Toasts auto-dismiss success in 4s, stay sticky on error.
- **Wave B.** `/admin/*` requires a cookie. `/r/<token>` and `/feeds/*` still work without one. Sign-out clears the cookie. Two roles render different `/admin/*` access sets.
- **Wave C.** `/admin/audio` lists rows from `audio_sources` table. Add / Edit / Remove round-trip through the new endpoints. Existing audio daemon still gets a string from `state.audio_source()` matching the seed row.
- **Wave D.** ⌘K opens cmdk on every page. "How this works" links open the right mdBook section. Changelog renders the embedded `CHANGELOG.md`. Quality page has the two new panels.
- **Wave E.** Day-strip and dawn-chorus carry the weather/moon overlay strip on devices with internet. Phone widths show the bottom tab bar.

## Rollback

Each opportunity is a clean git revert. The two with migrations (O-13 sources, O-15 accounts, O-23 weather) need their down-migrations applied first. Every other opportunity is additive HTML / CSS / Rust modules with no schema impact.

---

*Built against `tomtom215/BirdNet-Behavior@6a32dd82f692`. Re-run the drift check after each wave merges.*
