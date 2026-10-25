// Measured gate: the layers around the content, graded from the values app.css
// ships, in both themes. No browser.
//
//   node surface-layers.mjs
//   MUTATE='--wash-a=oklch(60% 0.2 150 / .9)' node surface-layers.mjs     watch it go red
//
// What it holds:
//   wash      the page's two washes (--wash-a, --wash-b) are painted behind text.
//             At their strongest point (the gradient's first stop) the lowest text
//             ink in use, --fg-3, still clears 4.5:1 on the page colour with the
//             wash over it, and so does --moss-ink, the link colour.
//   order     surfaces step up in the direction of "nearer": in dark, lighter
//             (--bg < --surface <= --surface-raised < --surface-overlay); in
//             light, never darker than the page. A raised thing must be told
//             from the page (>= 2 dE in dark, where there is no shadow to do it).
//   focus     the keyboard ring colour is 3:1 against --surface and --bg
//             (WCAG 1.4.11 / 2.4.11) and the ring is at least 2 px wide.
//   glass     the sticky bars paint --bg at 80 % over whatever scrolls under
//             them; the worst case is a full-strength --moss-soft or --rare-soft
//             card behind it. --fg-3 still clears 4.5:1 there.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parse, over, contrastRgb, toSrgb, dE } from './colour-math.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const CSS = process.env.CSS || path.join(here, '../../crates/birdnet-web/static/css/app.css');
const strip = (s) => s.replace(/\/\*[\s\S]*?\*\//g, '');
function block(css, header) {
  const i = css.indexOf(header + ' {');
  if (i < 0) throw new Error('no block ' + header);
  const start = i + header.length + 2, end = css.indexOf('\n}', start);
  const out = new Map(); let depth = 0, cur = '';
  const flush = () => { const d = cur.trim(); cur = ''; if (d.startsWith('--')) { const k = d.indexOf(':'); out.set(d.slice(0, k).trim(), d.slice(k + 1).trim().replace(/\s+/g, ' ')); } };
  for (const ch of css.slice(start, end)) { if (ch === '(') depth++; if (ch === ')') depth--; if (ch === ';' && depth === 0) { flush(); continue; } cur += ch; }
  flush(); return out;
}
const css = strip(fs.readFileSync(CSS, 'utf8'));
const light = block(css, ':root,\n:root[data-theme="light"]');
const dark = block(css, ':root[data-theme="dark"]');
for (const m of (process.env.MUTATE || '').split(';').filter(Boolean)) { const [k, v] = m.split('='); light.set(k, v); dark.set(k, v); }
const raw = (theme, n) => { let v = theme === 'dark' ? dark.get(n) ?? light.get(n) : light.get(n); for (let i = 0; i < 8 && v && v.startsWith('var('); i++) v = raw(theme, v.slice(4, -1).trim()); if (v === undefined) throw new Error(n + ' is not declared'); return v; };
const col = (theme, n) => parse(raw(theme, n));
const fails = []; let graded = 0;
const check = (name, ok, msg) => { graded++; if (!ok) fails.push(`${name}: ${msg}`); };

for (const theme of ['light', 'dark']) {
  const bg = col(theme, '--bg'), surface = col(theme, '--surface');
  for (const w of ['--wash-a', '--wash-b']) {
    const under = toSrgb(bg); const washed = over(col(theme, w), bg);
    for (const ink of ['--fg-3', '--moss-ink', '--fg-2']) {
      const c = contrastRgb(toSrgb(col(theme, ink)), washed);
      check('wash', c >= 4.5, `${theme} ${ink} on ${w} over the page is ${c.toFixed(2)}:1 (the page alone: ${contrastRgb(toSrgb(col(theme, ink)), under).toFixed(2)}:1)`);
    }
  }
  const L = (n) => col(theme, n).L;
  if (theme === 'dark') {
    check('order', L('--bg') < L('--surface') && L('--surface') <= L('--surface-raised') && L('--surface-raised') < L('--surface-overlay'), `dark surfaces do not step up: bg ${L('--bg')}, surface ${L('--surface')}, raised ${L('--surface-raised')}, overlay ${L('--surface-overlay')}`);
    check('order', dE(col(theme, '--surface-raised'), bg) >= 2, `dark raised is ${dE(col(theme, '--surface-raised'), bg).toFixed(1)} dE from the page`);
    check('order', dE(col(theme, '--surface-overlay'), col(theme, '--surface-raised')) >= 2, 'dark overlay is not told from raised');
  } else {
    check('order', L('--surface-overlay') >= L('--bg') && L('--surface-raised') >= L('--bg'), 'light raised/overlay is darker than the page');
  }
  for (const [n, against] of [['--focus-color', '--surface'], ['--focus-color', '--bg']]) {
    const c = contrastRgb(toSrgb(col(theme, n)), toSrgb(col(theme, against)));
    check('focus', c >= 3, `${theme} ${n} on ${against} is ${c.toFixed(2)}:1`);
  }
  check('focus', parseFloat(raw(theme, '--focus-width')) >= 2, `${theme} focus ring is ${raw(theme, '--focus-width')} wide`);
  for (const tint of ['--moss-soft', '--rare-soft', '--dawn-soft', '--info-soft']) {
    const glass = over({ ...bg, A: 0.8 }, col(theme, tint));
    const c = contrastRgb(toSrgb(col(theme, '--fg-3')), glass);
    check('glass', c >= 4.5, `${theme} --fg-3 on the nav over ${tint} is ${c.toFixed(2)}:1`);
  }
}
if (graded < 30) { console.error(`surface-layers: graded only ${graded}`); process.exit(1); }
if (fails.length) { console.error(`surface-layers: ${fails.length} failure(s) of ${graded}:\n  ` + fails.join('\n  ')); process.exit(1); }
console.log(`surface-layers: ${graded} checks, 0 failures`);
