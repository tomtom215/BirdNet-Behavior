# O-25 · Inline-style audit — promote utilities into `app.css`

<!-- BNB:STATUS-HEADER -->
> **Risk:** none · **Priority:** 4 (cosmetic / maintainability) · **Status:** ready for review
> Acceptance: VERIFY.md § O-25 · Rollback: ROLLBACK.md § O-25
<!-- BNB:STATUS-HEADER -->


## What

A lot of the admin Rust handlers (`audio.rs`, `backup_recovery.rs`, `notifications.rs`, `quality.rs`, several settings render modules) build HTML with literal `style="…"` strings inside `format!(...)`. Works today, but every layout tweak is a Rust edit + rebuild rather than a CSS edit. Designers can't iterate, theme overrides can't reach the inlined values, and the same grid shape gets re-typed in five files with subtle differences.

This change introduces a small set of **layout utility classes** that absorb the recurring inline-style patterns, then ships a hit-list of files where the substitution should land. The classes are deliberately *layout-only* — no colours, no typography, since the design system already handles those.

Risk is low: each substitution is mechanical and the existing inline-style versions remain valid HTML for any partial that's not migrated this cycle.

## New utility classes

```css
/* Flex / grid primitives (additive — never override existing rules) */
.bnb-row              { display: flex; align-items: center; gap: var(--pad-2); }
.bnb-row.between      { justify-content: space-between; }
.bnb-row.tight        { gap: 8px; }
.bnb-row.wide         { gap: var(--pad-3); }
.bnb-col              { display: flex; flex-direction: column; gap: var(--pad-2); }
.bnb-col.tight        { gap: 8px; }
.bnb-col.wide         { gap: var(--pad-3); }
.bnb-stack            { display: grid; gap: var(--pad-2); }
.bnb-stack.tight      { gap: 8px; }
.bnb-spread           { display: flex; align-items: flex-start; justify-content: space-between; gap: var(--pad-2); }

/* Grid templates for the recurring shapes */
.bnb-grid-2           { display: grid; grid-template-columns: 1fr 1fr; gap: var(--pad-3); }
.bnb-grid-3           { display: grid; grid-template-columns: repeat(3, 1fr); gap: var(--pad-3); }
.bnb-grid-4           { display: grid; grid-template-columns: repeat(4, 1fr); gap: var(--pad-3); }
.bnb-grid-side        { display: grid; grid-template-columns: 200px minmax(0, 1fr) 280px; gap: var(--pad-3); align-items: start; }
.bnb-grid-side.wide   { grid-template-columns: 240px minmax(0, 1fr) 320px; }
@media (max-width: 980px) { .bnb-grid-side, .bnb-grid-side.wide { grid-template-columns: 1fr; } }

/* Sticky aside (settings sidebars) */
.bnb-side { position: sticky; top: 16px; }

/* Form rows used by the admin settings render modules */
.bnb-form-row { display: grid; grid-template-columns: 220px 1fr; gap: 14px; align-items: center; padding: 12px 0; border-top: 0.5px solid var(--hairline); }
.bnb-form-row:first-child { border-top: 0; }
.bnb-form-row > label { font-size: 13px; color: var(--fg-2); }
.bnb-form-row > .hint { grid-column: 2; color: var(--fg-3); font-size: 11.5px; }

/* Range slider polish (replaces inline `accent-color`) */
.bnb-range { width: 100%; accent-color: var(--moss); }

/* Inline label + value (settings preview rows) */
.bnb-kv { display: inline-flex; gap: 6px; align-items: baseline; font-size: 12.5px; }
.bnb-kv .k { color: var(--fg-3); font-family: var(--font-mono); font-size: 10.5px; letter-spacing: 0.04em; text-transform: uppercase; }
.bnb-kv .v { color: var(--fg); font-variant-numeric: tabular-nums; }
```

These mirror the inline shapes already used in the codebase — nothing new visually. The point is they live in CSS instead of in `format!()` strings.

## Hit list — files with the most inline-style debt

Counted from a `grep -rn 'style="'` pass across `crates/birdnet-web/src/` at the date of this report. **Bold** entries are where the substitution has the highest payoff (≥10 inline-style strings).

| File | Inline shapes | Substitution |
|---|---|---|
| **`admin/audio.rs`** | ~25 (whole-page layout) | covered by O-13 rewrite |
| **`admin/backup_recovery.rs`** | ~22 | `bnb-grid-2`, `bnb-grid-side`, `bnb-spread`, `bnb-form-row` |
| **`admin/quality.rs`** | ~14 (its own `<style>` block plus inline overrides) | covered by O-22 skin pass |
| **`admin/notifications.rs`** | ~12 | `bnb-grid-side`, `bnb-row`, `bnb-form-row` |
| `admin/notification_test.rs` | ~9 | `bnb-col.tight`, `bnb-row` |
| `admin/system.rs` | ~14 | `bnb-stack`, `bnb-grid-2`, `bnb-range` |
| `admin/rules.rs` | ~9 | `bnb-grid-2`, `bnb-form-row` |
| `admin/migration.rs` | ~6 | `bnb-stack`, `bnb-row` |
| `admin/system_controls/{backup,data,service,update}.rs` | ~6 each | `bnb-row.between`, `bnb-grid-2` |
| `admin/settings/render/{audio,detection,email,location,notifications,species,system}.rs` | ~30 across all of them | `bnb-form-row`, `bnb-kv`, `bnb-row` (the form-row class alone replaces the most copy-pasted shape) |
| `admin/doctor.rs` | ~5 | `bnb-stack` |
| `admin/overview.rs` | ~7 | `bnb-grid-3`, `bnb-stack` |
| `admin/species/render.rs` | ~10 | `bnb-form-row`, `bnb-row.between` |
| `admin/species_tester.rs` | ~6 | `bnb-form-row` |
| `pages/dashboard/partials.rs` | ~4 | `bnb-row` |
| `pages/year_in_review.rs` | ~3 | `bnb-grid-4`, `bnb-stack` |

**Topnav, layout.html, dashboard.html, today.html, species_detail.html, species.html, dawn_chorus.html, migration.html, share_rare.html, recordings.html, timeseries.html, analytics.html** — all of these are already on-system (very few inline `style=` attributes). No work needed.

## Migration pattern — one worked example

`admin/backup_recovery.rs` builds a 3-column inline grid for the "Manual upload + export" section:

```rust
// Before
write!(html, r#"<div style="display:grid;grid-template-columns:1fr 1fr;gap:24px;">{drop_zone}{exports}</div>"#)?;
```

after introducing `bnb-grid-2`:

```rust
// After
write!(html, r#"<div class="bnb-grid-2">{drop_zone}{exports}</div>"#)?;
```

Multiply that by ~150 substitutions across the listed files and the inline-style footprint drops ~80%. The remaining inline `style=` strings are dynamic ones (computed widths in progress bars, computed `--sp:` colours on species avatars) which are appropriate places to keep them.

## Sequencing

This opportunity is **last** in the apply order. Every other O-NN here may add new inline styles (badges, drawers, etc.) — landing the utilities first would leave them in a half-migrated state. Doing it after all visual work is settled means one cohesive sweep through the listed files.

## Risk

Zero. The new classes are additive. The substitution is mechanical and reversible per-file.

---

<!-- BNB:CROSSREF-FOOTER -->
## Related

* O-13 (audio sources) already ships its own utility classes (`.audio-source__*`, `.bnb-add-form__*`, `.bnb-type-picker`). The classes proposed here are the generic layout primitives that everything else falls back to.
* O-22 (quality dashboard) skin pass uses these primitives as part of its rewrite.
<!-- BNB:CROSSREF-FOOTER -->
