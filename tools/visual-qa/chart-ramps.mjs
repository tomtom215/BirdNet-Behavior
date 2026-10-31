// Measured gate: the chart ramps and the data ink, graded from the values app.css
// ships (not from a copy), in both themes.
//
//   node chart-ramps.mjs               grade; exit 1 on any failure
//   MUTATE='--seq-3=oklch(30% 0.1 60)' node chart-ramps.mjs     watch a check go red
//
// What it holds (each line is one check name in the output):
//   monotone    a sequential ramp's lightness only moves one way; a diverging arm's
//               lightness only moves away from the middle
//   step        neighbours are 7-13 dE (sequential) or 10-24 dE (diverging) apart
//               in OKLab x100, so the eye reads one step as one step
//   empty       a sequential ramp's first step is >= 8 dE from an empty cell
//   gamut       every step is a colour an sRGB screen can show without clipping
//   hi-contrast a sequential ramp's last step is >= 3:1 on the card
//   poles       the two diverging poles are >= 15 dE apart for a reader with
//               protanopia and for one with deuteranopia
//   mid         the diverging middle is neutral (chroma < 0.012)
//   sky         the four washes behind a 24-hour axis: label and annotation ink >= 4.5:1 and the
//               axis >= 3:1 on every one, so no chart text is ever drawn on a wash it cannot be read on
//   ink         grid 1.1-1.6:1, axis >= 3:1, label and annotation >= 4.5:1,
//               tooltip text >= 7:1 on its tooltip, tooltip edge >= 1.5:1 on the card
//
// The categorical set is graded by crates/birdnet-web/tests/the_chart_palette_can_be_told_apart.rs.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parse, inGamut, contrast, dE, dECvd } from './colour-math.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const CSS = process.env.CSS || path.join(here, '../../crates/birdnet-web/static/css/app.css');

const strip = (s) => s.replace(/\/\*[\s\S]*?\*\//g, '');
function block(css, header) {
  const i = css.indexOf(header + ' {');
  if (i < 0) throw new Error('no block ' + header);
  const start = i + header.length + 2;
  const end = css.indexOf('\n}', start);
  const out = new Map();
  let depth = 0, cur = '';
  const flush = () => { const d = cur.trim(); cur = ''; if (d.startsWith('--')) { const k = d.indexOf(':'); out.set(d.slice(0, k).trim(), d.slice(k + 1).trim().replace(/\s+/g, ' ')); } };
  for (const ch of css.slice(start, end)) { if (ch === '(') depth++; if (ch === ')') depth--; if (ch === ';' && depth === 0) { flush(); continue; } cur += ch; }
  flush();
  return out;
}
const css = strip(fs.readFileSync(CSS, 'utf8'));
const light = block(css, ':root,\n:root[data-theme="light"]');
const dark = block(css, ':root[data-theme="dark"]');
for (const m of (process.env.MUTATE || '').split(';').filter(Boolean)) {
  const [k, v] = m.split('=');
  light.set(k, v); dark.set(k, v);
}
const val = (theme, name) => {
  let v = (theme === 'dark' ? dark.get(name) ?? light.get(name) : light.get(name));
  for (let i = 0; i < 8 && v && v.startsWith('var('); i++) v = val(theme, v.slice(4, -1).trim());
  if (v === undefined) throw new Error(`${name} is not declared`);
  return v;
};
const col = (theme, name) => parse(val(theme, name));

const fails = [];
let graded = 0;
const check = (name, ok, msg) => { graded++; if (!ok) fails.push(`${name}: ${msg}`); };
const rampOf = (theme, prefix, n) => Array.from({ length: n }, (_, i) => col(theme, `${prefix}-${i + 1}`));

for (const theme of ['light', 'dark']) {
  const surface = col(theme, '--surface');
  const empty = col(theme, '--surface-2');
  for (const [label, prefix] of [['sun', '--seq'], ['moss', '--seq-moss']]) {
    const r = rampOf(theme, prefix, 7);
    const dirs = r.slice(1).map((c, i) => Math.sign(c.L - r[i].L));
    check('monotone', dirs.every((d) => d === dirs[0] && d !== 0), `${theme} ${label}: lightness does not move one way`);
    r.slice(1).forEach((c, i) => {
      const d = dE(r[i], c);
      check('step', d >= 7 && d <= 13, `${theme} ${label} step ${i + 1}->${i + 2} is ${d.toFixed(1)} dE`);
    });
    check('empty', dE(r[0], empty) >= 8, `${theme} ${label} step 1 is ${dE(r[0], empty).toFixed(1)} dE from an empty cell`);
    r.forEach((c, i) => check('gamut', inGamut(c), `${theme} ${label} step ${i + 1} is outside sRGB`));
    check('hi-contrast', contrast(r[6], surface) >= 3, `${theme} ${label} step 7 is ${contrast(r[6], surface).toFixed(2)}:1 on the card`);
  }
  const names = ['--div-neg-3', '--div-neg-2', '--div-neg-1', '--div-mid', '--div-pos-1', '--div-pos-2', '--div-pos-3'];
  const d = names.map((n) => col(theme, n));
  const negArm = [d[0], d[1], d[2], d[3]], posArm = [d[3], d[4], d[5], d[6]];
  for (const [arm, label] of [[negArm, 'negative'], [posArm.slice().reverse(), 'positive']]) {
    // arm runs pole -> mid; lightness must move one way along it
    const dirs = arm.slice(1).map((c, i) => Math.sign(c.L - arm[i].L));
    check('monotone', dirs.every((x) => x === dirs[0] && x !== 0), `${theme} diverging ${label} arm: lightness does not move one way away from the middle`);
    arm.slice(1).forEach((c, i) => { const s = dE(arm[i], c); check('step', s >= 10 && s <= 24, `${theme} diverging ${label} arm step ${i + 1} is ${s.toFixed(1)} dE`); });
  }
  d.forEach((c, i) => check('gamut', inGamut(c), `${theme} ${names[i]} is outside sRGB`));
  for (const kind of ['protan', 'deutan']) {
    const s = dECvd(d[0], d[6], kind);
    check('poles', s >= 15, `${theme} poles are ${s.toFixed(1)} dE under ${kind}`);
  }
  check('mid', d[3].C < 0.012, `${theme} --div-mid has chroma ${d[3].C}`);

  const c = (n) => col(theme, n);
  const r = (n) => contrast(c(n), surface);
  check('ink', r('--chart-grid') >= 1.1 && r('--chart-grid') <= 1.6, `${theme} grid is ${r('--chart-grid').toFixed(2)}:1`);
  check('ink', r('--chart-grid-strong') >= 1.3 && r('--chart-grid-strong') <= 2.2, `${theme} strong grid is ${r('--chart-grid-strong').toFixed(2)}:1`);
  check('ink', r('--chart-axis') >= 3, `${theme} axis is ${r('--chart-axis').toFixed(2)}:1`);
  check('ink', r('--chart-label') >= 4.5, `${theme} label is ${r('--chart-label').toFixed(2)}:1`);
  check('ink', r('--chart-annot') >= 4.5, `${theme} annotation is ${r('--chart-annot').toFixed(2)}:1`);
  check('ink', contrast(c('--chart-tip-fg'), c('--chart-tip-bg')) >= 7, `${theme} tooltip text is ${contrast(c('--chart-tip-fg'), c('--chart-tip-bg')).toFixed(2)}:1`);
  for (const wash of ['--sky-night', '--sky-dawn', '--sky-day', '--sky-dusk']) {
    const on = (n) => contrast(c(n), c(wash));
    check('sky', on('--chart-label') >= 4.5, `${theme} label is ${on('--chart-label').toFixed(2)}:1 on ${wash}`);
    check('sky', on('--chart-annot') >= 4.5, `${theme} annotation is ${on('--chart-annot').toFixed(2)}:1 on ${wash}`);
    check('sky', on('--chart-axis') >= 3, `${theme} axis is ${on('--chart-axis').toFixed(2)}:1 on ${wash}`);
    check('gamut', inGamut(c(wash)), `${theme} ${wash} is outside sRGB`);
  }
  check('ink', r('--chart-tip-border') >= 1.5, `${theme} tooltip edge is ${r('--chart-tip-border').toFixed(2)}:1 on the card`);
}

if (graded < 100) { console.error(`chart-ramps: graded only ${graded} values; the reader has stopped finding them`); process.exit(1); }
if (fails.length) { console.error(`chart-ramps: ${fails.length} failure(s) of ${graded}:\n  ` + fails.join('\n  ')); process.exit(1); }
console.log(`chart-ramps: ${graded} checks, 0 failures`);
