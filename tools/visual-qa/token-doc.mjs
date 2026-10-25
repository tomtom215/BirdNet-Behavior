// Generates the token tables in docs/book/reference/design-tokens.md that are
// marked as generated, from the values in app.css, so the page cannot drift
// from the stylesheet for those groups.
//
//   node token-doc.mjs --write     rewrite every generated region in place
//   node token-doc.mjs --check     exit 1 if a generated region is out of date
//
// A generated region looks like
//
//   <!--tokens:begin chart-ink-->
//   | `--chart-grid` | `oklch(...)` | `oklch(...)` | Use. |
//   <!--tokens:end-->
//
// Hand-written tables elsewhere on the page stay hand-written; the Rust test
// tests/design_tokens_doc_matches_the_stylesheet.rs checks all of them against
// the stylesheet, generated or not. What this adds is that a *generated* row
// is never typed, so the only thing a person writes for it is the `use` text,
// and that lives here, in GROUPS, beside the token's name.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, '../..');
const CSS = path.join(root, 'crates/birdnet-web/static/css/app.css');
const DOC = path.join(root, 'docs/book/reference/design-tokens.md');

export const GROUPS = {
  'seq-sun': [
    ['--seq-1', 'The least a cell can hold and still be drawn (held 8 dE from an empty cell).'],
    ['--seq-2', 'Sequential sun ramp, step 2.'],
    ['--seq-3', 'Step 3.'],
    ['--seq-4', 'The middle of the ramp.'],
    ['--seq-5', 'Step 5.'],
    ['--seq-6', 'Step 6.'],
    ['--seq-7', 'The most. 9.8:1 on a light card, 13:1 on a dark one.'],
  ],
  'seq-moss': [
    ['--seq-moss-1', 'Moss ramp, the least.'],
    ['--seq-moss-2', 'Step 2.'],
    ['--seq-moss-3', 'Step 3.'],
    ['--seq-moss-4', 'Step 4.'],
    ['--seq-moss-5', 'Step 5.'],
    ['--seq-moss-6', 'Step 6.'],
    ['--seq-moss-7', 'The most.'],
  ],
  diverging: [
    ['--div-neg-3', 'Far below the baseline (strong blue).'],
    ['--div-neg-2', 'Below.'],
    ['--div-neg-1', 'Slightly below.'],
    ['--div-mid', 'The baseline: a neutral grey, never a hue.'],
    ['--div-pos-1', 'Slightly above.'],
    ['--div-pos-2', 'Above.'],
    ['--div-pos-3', 'Far above the baseline (strong brick).'],
  ],
  'chart-ink': [
    ['--chart-grid', 'A gridline: 1.2:1 against the card on purpose, a guide and not information.'],
    ['--chart-grid-strong', 'The gridline that marks a boundary (midnight, the zero line of a diverging chart).'],
    ['--chart-axis', 'An axis line and its ticks: 3:1 against the card.'],
    ['--chart-tip-bg', 'A chart tooltip: a dark card on the light theme, a lifted one on dark.'],
    ['--chart-tip-fg', 'Tooltip text, 13:1 or better on the tooltip.'],
    ['--chart-tip-border', 'Tooltip edge, so it never merges with the card behind it.'],
  ],
  'chart-ink-shared': [
    ['--chart-label', 'An axis label or a mark label: text, 4.5:1.'],
    ['--chart-annot', 'An annotation drawn on the plot ("peak", "now"): text, stronger than a label.'],
    ['--chart-track', 'The empty part of a bar: the track a fill sits on.'],
    ['--chart-gap', 'The colour of the gap between touching fills: the card, so a stack reads as separate marks.'],
    ['--chart-crosshair', 'The hover crosshair line.'],
    ['--chart-stroke', 'A line mark\'s width.'],
    ['--chart-gap-w', 'The width of the gap between touching fills.'],
    ['--chart-bar-r', 'The round on the data end of a bar, anchored square at the baseline.'],
    ['--chart-dim', 'The opacity of the marks that are not hovered.'],
  ],
  'surfaces-themed': [
    ['--surface-raised', 'A card. Equal to --surface in light; one step lighter than the page in dark, where nearer is lighter.'],
    ['--surface-overlay', 'A popover, a menu, a dialog: lifted again in dark.'],
    ['--wash-a', 'The green wash behind the top left of the page.'],
    ['--wash-b', 'The warm wash behind the top right.'],
  ],
  'surfaces-shared': [
    ['--surface-sunken', 'A well cut into a card: an inset field, a track, a code block.'],
    ['--surface-glass', 'The translucent surface of a sticky bar, drawn with --glass-blur.'],
    ['--glass-blur', 'The `backdrop-filter` of a glass surface.'],
  ],
  'shadows-themed': [
    ['--shadow-tint', 'What every shadow is mixed from: warm brown on paper, cool near-black on night. Never pure black.'],
    ['--shadow-xs', 'A hairline of depth: a chip, a segmented control.'],
    ['--shadow-hover', 'A card the pointer is on: between --shadow-md and --shadow-lg.'],
    ['--shadow-xl', 'A command palette, a drawer, the biggest thing on the screen.'],
    ['--shadow-press', 'A control being pressed: it sinks into the page.'],
    ['--shadow-inset', 'The one-pixel light along the top edge of a raised surface.'],
    ['--text-glow', 'The halo behind display type on a dark page; none on paper.'],
    ['--shadow-field', 'The inner shadow of a text field or a segmented track: it is cut into the page, not set on it.'],
  ],
  'shadows-shared': [
    ['--ring-danger', 'The faint red ring around a destructive dialog.'],
    ['--accent-bar', 'The width of the accent bar down the left edge of a banner or a toast (an inset shadow in the banner\'s own colour).'],
    ['--accent-bar-sm', 'The same on a list row.'],
    ['--halo-w', 'The width of the halo round a status dot in a toast.'],
    ['--shadow-key', 'A one-pixel highlight along the top of a filled button, in that button\'s own text colour.'],
    ['--shadow-up', 'The faint shadow cast upward by a bar fixed to the bottom of the screen.'],
    ['--shadow-glow', 'A moss ring and a soft green halo: a live or selected thing that should be noticed.'],
  ],
  'radius-extra': [
    ['--r-3xs', 'The smallest round: a meter, a tick.'],
    ['--r-base', 'A hover highlight on a list row.'],
    ['--r-2xl', 'A sheet.'],
    ['--r-control', 'A button or a segmented control. Today the same as --r-sm.'],
    ['--r-field', 'A text field or a select.'],
    ['--r-card', 'A card.'],
    ['--r-popover', 'A menu or a tooltip.'],
    ['--r-dialog', 'A dialog or a drawer.'],
  ],
  edge: [
    ['--edge', 'A card\'s edge: the hairline in --border.'],
    ['--edge-2', 'A control\'s edge: the hairline in --border-2.'],
    ['--edge-hair', 'A divider: the hairline in --hairline.'],
  ],
  'space-extra': [
    ['--space-28', 'Between a page\'s sections on a desk.'],
    ['--space-40', 'Above a page\'s first heading.'],
    ['--space-48', 'The padding of an empty state.'],
    ['--space-64', 'The room under a page.'],
  ],
  'type-extra': [
    ['--fs-display', 'The oversize numeral or opening line of a page with one thing to say. Fluid, 48 to 84 px.'],
    ['--fs-section', 'A section title in the serif. Fluid, 22 to 28 px.'],
    ['--lh-none', 'A single line of display type.'],
    ['--lh-tight', 'Serif display lines that wrap.'],
    ['--lh-snug', 'Card titles and short labels.'],
    ['--lh-base', 'Body text.'],
    ['--lh-loose', 'Long-form prose (the manual, the changelog).'],
    ['--measure', 'The longest a line of prose should run.'],
  ],
  'motion-extra': [
    ['--dur-instant', 'A press: the control has to feel it at once.'],
    ['--dur-slower', 'A panel or a sheet travelling a long way.'],
    ['--dur-loop', 'One sweep of a skeleton\'s shimmer.'],
    ['--dur-pulse', 'One beat of a live dot.'],
    ['--dur-blink', 'One blink of a starting indicator.'],
    ['--dur-glow', 'A highlight fading out: a fresh row settling into the list.'],
    ['--ease-in', 'Something leaving.'],
    ['--ease-emphasized', 'Something arriving with intent: a dialog, a toast.'],
    ['--ease-spring', 'A control that settles: it overshoots by a few per cent and comes back.'],
    ['--stagger', 'The delay between one row of a list and the next as they reveal.'],
    ['--lift', 'How far a hovered card rises. Zero under reduced motion.'],
    ['--press-scale', 'How far a pressed button shrinks. One under reduced motion.'],
  ],
  'z-extra': [
    ['--z-base', 'The page.'],
    ['--z-dropdown', 'A menu under the top bar.'],
    ['--z-drawer', 'The help drawer.'],
    ['--z-modal', 'A dialog that is not in the top layer.'],
    ['--z-tooltip', 'A tooltip, above everything but the skip link.'],
  ],
  'focus-extra': [
    ['--focus-color', 'The keyboard focus ring.'],
    ['--focus-width', 'Its thickness (WCAG 2.4.13 asks for 2 px).'],
    ['--focus-offset', 'The gap between the control and its ring.'],
  ],
  icons: [
    ['--icon-moon', 'The moon on the theme button, drawn as a mask so it takes the button\'s ink.'],
    ['--icon-sun', 'The sun, shown on a dark page.'],
  ],
  density: [
    ['--control-h', 'The height of a button or a field. Follows density, floored at 28 px.'],
    ['--control-h-sm', 'The height of a small control: a chip, a segment.'],
    ['--row-h', 'The height of a list row.'],
  ],
};

const strip = (s) => s.replace(/\/\*[\s\S]*?\*\//g, '');
function block(css, header) {
  const i = css.indexOf(header + ' {');
  if (i < 0) throw new Error('no block ' + header);
  const start = i + header.length + 2;
  const end = css.indexOf('\n}', start);
  const out = new Map();
  let depth = 0, cur = '';
  const flush = () => {
    const d = cur.trim(); cur = '';
    if (!d.startsWith('--')) return;
    const k = d.indexOf(':');
    out.set(d.slice(0, k).trim(), d.slice(k + 1).trim().replace(/\s+/g, ' '));
  };
  for (const ch of css.slice(start, end)) {
    if (ch === '(') depth++;
    if (ch === ')') depth--;
    if (ch === ';' && depth === 0) { flush(); continue; }
    cur += ch;
  }
  flush();
  return out;
}

export function tables() {
  const css = strip(fs.readFileSync(CSS, 'utf8'));
  const light = block(css, ':root,\n:root[data-theme="light"]');
  const dark = block(css, ':root[data-theme="dark"]');
  const out = {};
  for (const [name, rows] of Object.entries(GROUPS)) {
    const lines = rows.map(([t, use]) => {
      const l = light.get(t) ?? dark.get(t);
      if (l === undefined) throw new Error(`${t} (group ${name}) is not declared in app.css`);
      const d = dark.get(t);
      return d !== undefined && d !== l
        ? `| \`${t}\` | \`${l}\` | \`${d}\` | ${use} |`
        : `| \`${t}\` | \`${l}\` | ${use} |`;
    });
    const four = lines.map((x) => x.split(' | ').length === 4);
    if (four.some((x) => x !== four[0])) throw new Error(`group ${name} mixes themed and shared tokens; split it`);
    const head = four[0] ? ['| Token | Light | Dark | Use |', '|---|---|---|---|'] : ['| Token | Value | Use |', '|---|---|---|'];
    out[name] = [...head, ...lines];
  }
  return out;
}

// The two numbers the page opens with ("N tokens in the light block, M overrides
// in the dark block") are read from the stylesheet too; the Rust test compares
// them with the same counts.
export function counts() {
  const css = strip(fs.readFileSync(CSS, 'utf8'));
  return {
    light: block(css, ':root,\n:root[data-theme="light"]').size,
    dark: block(css, ':root[data-theme="dark"]').size,
  };
}

// Every hand-written row keeps its description and loses its typed values: the
// value cells are rewritten from app.css, so the *only* thing a person writes on
// the page is what a token is for. A row whose shape (one value or two) no longer
// fits the stylesheet (a token gained or lost a dark override) is an error, not
// something to guess at.
function syncRows(md) {
  const css = strip(fs.readFileSync(CSS, 'utf8'));
  const light = block(css, ':root,\n:root[data-theme="light"]');
  const dark = block(css, ':root[data-theme="dark"]');
  return md.split('\n').map((line) => {
    const m = line.match(/^\| `(--[\w-]+)` \| (.*)$/);
    if (!m) return line;
    const tok = m[1];
    const cells = line.replace(/^\| /, '').replace(/ \|$/, '').split(' | ');
    const l = light.get(tok) ?? dark.get(tok);
    if (l === undefined) return line; // the Rust test reports a documented token that is not declared
    const d = dark.get(tok);
    const themed = d !== undefined && d !== l;
    if (cells.length === 4 && themed) return `| \`${tok}\` | \`${l}\` | \`${d}\` | ${cells[3]} |`;
    if (cells.length === 3 && !themed) return `| \`${tok}\` | \`${l}\` | ${cells[2]} |`;
    if (cells.length === 2) return line; // an alias row: `--x` | `--y`
    throw new Error(`${tok}: the row has ${cells.length} cells but the stylesheet ${themed ? 'overrides it in dark' : 'has one value'}`);
  }).join('\n');
}

export function render(md) {
  const t = tables();
  let out = syncRows(md);
  const c = counts();
  for (const k of ['light', 'dark']) {
    const re = new RegExp(`(<!--count-${k}-->)\\d+(<!--/count-->)`);
    if (!re.test(out)) throw new Error(`design-tokens.md has no count-${k} marker`);
    out = out.replace(re, `$1${c[k]}$2`);
  }
  for (const [name, rows] of Object.entries(t)) {
    const re = new RegExp(`(<!--tokens:begin ${name}-->\\n)[\\s\\S]*?(\\n<!--tokens:end-->)`);
    if (!re.test(out)) throw new Error(`design-tokens.md has no generated region "${name}"`);
    out = out.replace(re, (_, a, b) => a + rows.join('\n') + b);
  }
  return out;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const mode = process.argv[2];
  const md = fs.readFileSync(DOC, 'utf8');
  const next = render(md);
  if (mode === '--write') { fs.writeFileSync(DOC + '.tmp', next); fs.renameSync(DOC + '.tmp', DOC); console.log('rewrote generated token regions'); }
  else if (mode === '--check') {
    if (next !== md) { console.error('design-tokens.md generated regions are out of date; run: node tools/visual-qa/token-doc.mjs --write'); process.exit(1); }
    console.log('design-tokens.md generated regions are current');
  } else { console.error('usage: token-doc.mjs --write | --check'); process.exit(2); }
}
