# Proposed PR set · BirdNet-Behavior · v1.0

**Status: ready for review** — ten changes packaged as a single PR against `tomtom215/BirdNet-Behavior@main`.

## Start here

| Document | Purpose |
|---|---|
| [**HANDOFF.md**](HANDOFF.md) | Canonical entry point — package overview, wave-by-wave apply order, cheat-sheet bulk copy, environment knobs |
| [**INDEX.html**](INDEX.html) | Visual preview of all 10 PRs rendered against the production stylesheet |
| [**VERIFY.md**](VERIFY.md) | Post-merge acceptance criteria per PR (~25 min total) |
| [**ROLLBACK.md**](ROLLBACK.md) | Independent per-PR back-out plan |
| `O-NN/DIFF.md` | Single source of truth for each PR (files, route patches, risk) |

## What's here

| # | Change | Type |
|---|---|---|
| O-04 | Species detail rebuild | template-only |
| O-09 | Today · comparative phrase | template + 1 partial |
| O-12 | Empty states library | helper module |
| O-08 | Print stylesheet | CSS-only |
| O-03 | Display preferences | partial + CSS append |
| O-05 | Detection detail surfacing | template + 6 small partials |
| O-01 | `/migration` ridgeline | new page |
| O-02 | `/analytics/dawn-chorus` polar ribbons | new page |
| O-07 | `/r/<token>` share permalinks | new public route |
| O-11 | iCal + RSS feeds | new public routes |

**Zero new crate dependencies. One optional schema migration. One URL rename (with redirect plan). Shipping as one combined PR.**

## Folder layout

```
proposed_changes/
├── HANDOFF.md  VERIFY.md  ROLLBACK.md  README.md
├── INDEX.html
├── O-01_migration/      O-02_dawn_chorus/     O-03_display_prefs/
├── O-04_species_detail/ O-05_detection_detail/ O-07_share_links/
├── O-08_print/          O-09_today_comparative/ O-11_feeds/
├── O-12_empty_states/
└── _preview_assets/     (production CSS + self-hosted fonts — preview only)
```

Each `O-NN/` folder mirrors target paths in `crates/birdnet-web/`, so a bulk copy works (see HANDOFF.md's cheat-sheet).

## Built against

`tomtom215/BirdNet-Behavior@c1a3dcd538a7` (main). Full drift analysis in [`../DRIFT_REPORT.md`](../DRIFT_REPORT.md).
