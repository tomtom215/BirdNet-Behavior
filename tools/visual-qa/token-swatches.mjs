// Renders every design token as a swatch sheet, from the stylesheet the fixture
// serves, so the picture on the token page is the tokens and not a drawing of
// them. One PNG per theme.
//
//   BASE=http://127.0.0.1:8502 OUT=docs/book/images node token-swatches.mjs
//
// Output: <OUT>/design-tokens__light.png and design-tokens__dark.png (1440 wide,
// full height). The page it draws is built from the names in app.css's light
// block, grouped by prefix, and read back from the browser's computed values,
// so a token added to app.css appears here without this script changing.
import { chromium } from 'playwright';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const BASE = process.env.BASE || 'http://127.0.0.1:8502';
const OUT = path.resolve(process.env.OUT || path.join(here, '../../docs/book/images'));
const CHROMIUM_PATH = process.env.CHROMIUM_PATH || '';
const CSS = path.join(here, '../../crates/birdnet-web/static/css/app.css');

const strip = (s) => s.replace(/\/\*[\s\S]*?\*\//g, '');
function names(css) {
  const h = ':root,\n:root[data-theme="light"] {';
  const i = css.indexOf(h); const e = css.indexOf('\n}', i);
  const out = []; let d = 0, cur = '';
  for (const ch of css.slice(i + h.length, e)) {
    if (ch === '(') d++; if (ch === ')') d--;
    if (ch === ';' && d === 0) { const t = cur.trim(); if (t.startsWith('--')) out.push(t.slice(0, t.indexOf(':')).trim()); cur = ''; continue; }
    cur += ch;
  }
  return out;
}
const toks = names(strip(fs.readFileSync(CSS, 'utf8')));
const kind = (n) => {
  if (/^--(accent-bar|halo-w|icon-|shadow-key|shadow-up)/.test(n)) return 'other';
  if (/^--(shadow-(xs|sm|md|lg|xl|hover|press|inset|field|glow)|ring|ring-soft|ring-danger)$/.test(n)) return 'shadow';
  if (/^--r-/.test(n)) return 'radius';
  if (/^--space-/.test(n)) return 'space';
  if (/^--fs-/.test(n)) return 'type';
  if (/^--(dur|ease|stagger|lift|press-scale)/.test(n)) return 'motion';
  if (/^--(z-|bw|edge|focus|lh-|fw-|track-|measure|font-|density|pad-|control-h|row-h|glass-blur|chart-(stroke|gap-w|bar-r|dim)|text-glow|accent-rgb|scrim)/.test(n)) return 'other';
  if (/^--(shadow-tint|bg|surface|border|hairline|fg|moss|dawn|rare|info|conf|on-|paper|night|card-|sheen|glow|wash|sp-ink|chart|seq|div|heat|accent|success|warning|danger|input-bg|bg-|text|switch|dawn)/.test(n)) return 'colour';
  return 'other';
};
const groups = { colour: [], shadow: [], radius: [], space: [], type: [], motion: [], other: [] };
for (const n of toks) groups[kind(n)].push(n);

function page() {
  const sw = (n) => `<div class="sw"><div class="chip" style-var="${n}"></div><code>${n}</code><span class="v" data-v="${n}"></span></div>`;
  const colourGroups = [
    ['Surfaces and lines', /^--(bg|surface|border|hairline|card-|sheen|glow|wash|paper|night|shadow-tint)/],
    ['Text', /^--(fg|text|on-|sp-ink|accent-rgb)/],
    ['Semantic', /^--(moss|dawn|rare|info|accent|success|warning|danger|switch)/],
    ['Confidence', /^--conf/],
    ['Chart: categorical', /^--chart-(\d|other)/],
    ['Chart: sequential, sun', /^--(seq-\d|heat)/],
    ['Chart: sequential, moss', /^--seq-moss/],
    ['Chart: diverging', /^--div/],
    ['Chart: ink', /^--chart-/],
  ];
  const used = new Set();
  const colourHtml = colourGroups.map(([t, re]) => {
    const list = groups.colour.filter((n) => !used.has(n) && re.test(n));
    list.forEach((n) => used.add(n));
    return `<h2>${t}</h2><div class="grid">${list.map(sw).join('')}</div>`;
  }).join('') + (() => { const rest = groups.colour.filter((n) => !used.has(n)); return rest.length ? `<h2>Other colours</h2><div class="grid">${rest.map(sw).join('')}</div>` : ''; })();
  const shadows = groups.shadow.map((n) => `<div class="sh"><div class="box" data-shadow="${n}"></div><code>${n}</code></div>`).join('');
  const radii = groups.radius.map((n) => `<div class="sh"><div class="rbox" data-radius="${n}"></div><code>${n}</code><span class="v" data-v="${n}"></span></div>`).join('');
  const space = groups.space.map((n) => `<div class="sp"><code>${n}</code><div class="bar" data-w="${n}"></div><span class="v" data-v="${n}"></span></div>`).join('');
  const type = groups.type.map((n) => `<div class="ty"><code>${n}</code><span class="aa" data-fs="${n}">Aa Bird</span><span class="v" data-v="${n}"></span></div>`).join('');
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><link rel="stylesheet" href="/static/css/app.css"><style>
  body{padding:32px 40px;max-width:1440px;margin:0 auto}
  h1{font-family:var(--font-display);font-weight:400;font-size:40px;margin-bottom:4px}
  h2{font-size:11px;letter-spacing:.1em;text-transform:uppercase;color:var(--fg-3);margin:28px 0 10px}
  .grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(168px,1fr));gap:12px}
  .sw{display:flex;flex-direction:column;gap:4px}.chip{height:44px;border-radius:8px;border:.5px solid var(--border);box-shadow:var(--shadow-xs)}
  code{font-family:var(--font-mono);font-size:10.5px;color:var(--fg)}.v{font-family:var(--font-mono);font-size:9.5px;color:var(--fg-3);overflow-wrap:anywhere}
  .row{display:flex;flex-wrap:wrap;gap:22px}.sh{display:flex;flex-direction:column;gap:8px;align-items:flex-start}
  .box{width:120px;height:64px;border-radius:12px;background:var(--surface-raised)}.rbox{width:64px;height:48px;background:var(--surface-2);border:.5px solid var(--border-2)}
  .sp{display:grid;grid-template-columns:110px 1fr 70px;align-items:center;gap:10px}.bar{height:10px;background:var(--moss);border-radius:2px}
  .ty{display:grid;grid-template-columns:130px 1fr 150px;align-items:baseline;gap:10px;border-bottom:.5px solid var(--hairline);padding:4px 0}.aa{font-family:var(--font-display)}
  .cols{display:grid;grid-template-columns:1fr 1fr;gap:40px}
  </style></head><body><h1>Design tokens</h1><div class="bnb-meta" id="theme"></div>${colourHtml}
  <h2>Elevation</h2><div class="row" style="padding:8px 4px 20px">${shadows}</div>
  <h2>Radius</h2><div class="row">${radii}</div>
  <div class="cols"><div><h2>Spacing</h2>${space}</div><div><h2>Type</h2>${type}</div></div></body></html>`;
}

const browser = await chromium.launch(CHROMIUM_PATH ? { executablePath: CHROMIUM_PATH } : {});
fs.mkdirSync(OUT, { recursive: true });
for (const theme of ['light', 'dark']) {
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 }, colorScheme: theme, serviceWorkers: 'block' });
  await ctx.addInitScript((t) => { try { localStorage.setItem('theme', t); } catch (e) {} }, theme);
  if (process.env.CSS_OVERRIDE) await ctx.route(/\/static\/css\/app\.css(\?.*)?$/, (r) => r.fulfill({ status: 200, contentType: 'text/css', body: fs.readFileSync(process.env.CSS_OVERRIDE, 'utf8') }));
  await ctx.route('**/__swatches', (r) => r.fulfill({ status: 200, contentType: 'text/html', body: page() }));
  const p = await ctx.newPage();
  await p.goto(BASE + '/__swatches', { waitUntil: 'networkidle' });
  const n = await p.evaluate((t) => {
    document.documentElement.setAttribute('data-theme', t);
    const cs = getComputedStyle(document.documentElement);
    document.getElementById('theme').textContent = `${document.documentElement.dataset.theme} theme · values as the browser computes them`;
    let count = 0;
    document.querySelectorAll('[style-var]').forEach((e) => { e.style.background = `var(${e.getAttribute('style-var')})`; count++; });
    document.querySelectorAll('[data-v]').forEach((e) => { e.textContent = cs.getPropertyValue(e.dataset.v).trim().replace(/\s+/g, ' ').slice(0, 60); });
    document.querySelectorAll('[data-shadow]').forEach((e) => { e.style.boxShadow = `var(${e.dataset.shadow})`; });
    document.querySelectorAll('[data-radius]').forEach((e) => { e.style.borderRadius = `var(${e.dataset.radius})`; });
    document.querySelectorAll('[data-w]').forEach((e) => { e.style.width = `var(${e.dataset.w})`; });
    document.querySelectorAll('[data-fs]').forEach((e) => { e.style.fontSize = `var(${e.dataset.fs})`; });
    return count;
  }, theme);
  if (n < 100) throw new Error(`only ${n} colour swatches drawn; the token list was not read`);
  await p.waitForTimeout(300);
  await p.screenshot({ path: path.join(OUT, `design-tokens__${theme}.png`), fullPage: true });
  console.log(`${theme}: ${n} colour swatches`);
  await ctx.close();
}
await browser.close();
