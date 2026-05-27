# Proposed PR set · BirdNet-Behavior · v2.0

**Status: ready for review** — fourteen changes targeting `tomtom215/BirdNet-Behavior@main` (commit `6a32dd82f692`).

The v1.0 set in this folder (O-01 … O-12) has shipped — every opportunity from that wave is now in production templates, partials, or `app.css`. This v2 set picks up where that left off, with thirteen new opportunities derived from a fresh repo read.

## What's here

| # | Change | Risk | Priority | Depends on | Type |
|---|---|---|---|---|---|
| O-13 | **Audio Sources** — first-class CRUD replaces the stubbed `/admin/audio` | medium | 1 | O-17, O-18 | template + Rust rewrite + SQLite migration |
| O-14 | **Branded login** + cookie-session migration | medium | 2 | — | template + auth-model RFC |
| O-15 | **Accounts & sessions** surface (users, sessions, audit log) | medium | 4 | O-14 | template + Rust + SQLite migration |
| O-16 | **Skeleton loading** states across every htmx swap target | none | 2 | — | utility class + Rust helper |
| O-17 | **Themed confirmation modal** replaces every `hx-confirm` site | none | 2 | — | utility class + Rust helper + partial |
| O-18 | **Toast / snackbar** region with OOB-swap helper | none | 2 | — | utility class + Rust helper + partial |
| O-19 | **Command palette** (⌘K / Ctrl-K / `/`) | none | 3 | — | utility class + Rust handler + partial |
| O-20 | **Help / methodology** surface — links + drawer into the embedded mdBook | low | 2 | — | utility class + Rust helper + partial |
| O-21 | **Changelog viewer** + post-upgrade banner | none | 3 | — | utility class + Rust handler + partial |
| O-22 | **Quality dashboard** audit — two model-trust panels + skin pass | low | 3 | — | DIFF only + utility class |
| O-23 | **Signal-context overlay** — weather / moon / SPL on time-axis charts | low (weather/moon) · medium (SPL) | 4 | — | utility class + Rust helper |
| O-24 | **Mobile / PWA** — bottom tab bar + manifest + service worker | low | 3 | — | manifest + sw + utility class + partial |
| O-25 | **Inline-style audit** — promote recurring shapes to utility classes | none | 4 (cosmetic) | All other O-NN | utility class additions only |
| O-26 | **IA fix** — strip the destination flex-wrap out of the footer; add a grouped *More* dropdown to the topnav | none | 1 | — | template + CSS only |

**Three foundations (O-16, O-17, O-18) unblock everything else** and should land first. **Five drop-ins (O-19, O-20, O-21, O-22, O-23) are independent**. **Two scoped RFCs (O-14, O-15) need engineering sign-off** on the session model before shipping. **O-13 is the headline feature change** — it replaces the audio.rs stub with a real entity model. **O-24 (Mobile/PWA) and O-25 (style audit) close the package.**

## Folder layout

```
proposed_changes/
├── README.md               ← v1.0 — already shipped to main
├── README_v2.md            ← this file
├── HANDOFF_v2.md           ← apply order + cheat sheet
├── INDEX_v2.html           ← visual previews (matches the existing INDEX.html shape)
├── O-13_audio_sources/
├── O-14_login/
├── O-15_accounts/
├── O-16_skeletons/
├── O-17_confirm_modal/
├── O-18_toast/
├── O-19_cmdk/
├── O-20_help/
├── O-21_changelog/
├── O-22_quality/
├── O-23_signal_overlay/
├── O-24_mobile_pwa/
├── O-25_inline_styles/
└── O-26_nav_ia/
```

Each `O-NN_*/` folder mirrors target paths in `crates/birdnet-web/` (and `crates/birdnet-db/` for the migrations), so a bulk copy works. See `HANDOFF_v2.md` for the apply script.

## Apply order

Five waves. Each wave is independent; opportunities inside a wave are also independent of each other.

| Wave | Opportunities | Why this order |
|---|---|---|
| **A · foundations** | O-16, O-17, O-18, O-25, **O-26** | First three add visual primitives O-13/O-14/O-15 use; O-25 introduces the layout utility classes; **O-26 rebuilds the IA so every later opportunity routes through the right surface, not the footer.** |
| **B · access** | O-14, O-15 | RFC sign-off on the session model lands these together. |
| **C · primary feature** | O-13 | Headline change — the audio-sources rewrite. |
| **D · independent surfaces** | O-19, O-20, O-21, O-22 | Cmdk, help, changelog, quality audit. Each ships in isolation. |
| **E · context + reach** | O-23, O-24 | Signal overlays + mobile/PWA wrap up the round. |

## Built against

`tomtom215/BirdNet-Behavior@6a32dd82f692` (the commit at the time of this report). Detailed source-read findings are in `../DRIFT_REPORT.md` (v1, against an earlier commit) and the in-chat review threaded above.

## Common shape (every opportunity)

Each `O-NN/` folder contains, at minimum:

- `DIFF.md` — single source of truth for files touched, query / schema deltas, sequencing, and risk.
- A `templates/` or `src/` or `css/` subtree mirroring the repo layout.
- An `app.css.append` where relevant — designed to be `cat >> static/css/app.css` cleanly, no overrides or duplications.

Where an opportunity is primarily an RFC (O-14, O-15, parts of O-23), the DIFF.md spells out the schema, the routing pattern, and the rollback. Where an opportunity ships drop-in code (everything else), the folder mirrors the target path; one bulk copy lands the files.

## What we deliberately did **not** include

- **Cross-screen edits.** Each O-NN ships its drop-ins. Tying every existing analytical screen to (say) the new help-link or the new skeleton helpers is left to follow-up PRs so the v2 package stays surgical.
- **Multi-station compare.** Still single-station; a "your station vs. neighbour's" feature is out of scope until storage / shape is decided.
- **Push notifications.** O-24 ships PWA bones but not Web Push — that needs a server-side push store and a key-rotation story that's distinct from the session model in O-14.
- **PWA icons.** O-24 references the three icon sizes but does not commit binaries to this folder — generate them from the existing `BrandMark` SVG with whatever pipeline the project prefers.

— *Reach out to the design author for context on any decision; the chat transcript that produced this set is available on request.*
