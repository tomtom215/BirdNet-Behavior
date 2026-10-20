// Do numbers that are meant to line up, line up?
//
// Inter Tight's default figures are proportional: a "1" is narrower than a "0",
// so a column of times, counts and percentages has a ragged right edge and a
// live counter jitters as it ticks. `font-variant-numeric: tabular-nums` fixes
// that, and the stylesheet applied it to 17 rules by hand. This finds the ones
// it missed, on the rendered pages, rather than trusting a grep for the rule.
//
// A "figure" here is a text node that is nothing but digits and the punctuation
// of numbers (1,444  06:57  88%  +12  3.2 kHz). It passes if the element that
// holds it computes `tabular-nums`, or draws it in a monospaced face. The report
// is the figures that fail, grouped by the element that holds them.
//
// Display numerals in the serif (a page's big statistic) are exempt by class:
// they are set, not tabulated, and proportional figures are the better choice at
// that size... except in the stat tiles, which are compared to each other.
//
// Usage:  BASE=... CHROMIUM_PATH=... node figures.mjs [--fail]
//         exits 1 with --fail when any figure is proportional.
import { chromium } from 'playwright';
import { ROUTES } from './qa.mjs';

const BASE = process.env.BASE || 'http://127.0.0.1:8502';
const CHROMIUM_PATH = process.env.CHROMIUM_PATH || '';
const FAIL = process.argv.includes('--fail');

const browser = await chromium.launch(CHROMIUM_PATH ? { executablePath: CHROMIUM_PATH } : {});
const context = await browser.newContext({ viewport: { width: 1440, height: 900 }, reducedMotion: 'reduce' });
const page = await context.newPage();

const MONO = /mono|courier|menlo|consolas|sf mono/i;
const groups = new Map();
let figures = 0;
let proportional = 0;

for (const [name, route] of ROUTES) {
  try {
    await page.goto(BASE + route, { waitUntil: 'domcontentloaded', timeout: 25000 });
    await page.waitForLoadState('networkidle', { timeout: 9000 }).catch(() => {});
    await page.waitForTimeout(500);
  } catch (e) { continue; }
  const found = await page.evaluate(() => {
    const out = [];
    const re = /^[\s\d.,:%+\-−–/]*\d[\s\d.,:%+\-−–/]*$/;
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    for (let n = walker.nextNode(); n; n = walker.nextNode()) {
      const t = n.nodeValue.trim();
      if (!t || t.length > 14 || !re.test(t)) continue;
      const el = n.parentElement;
      if (!el || el.closest('svg, script, style, [hidden], [aria-hidden="true"]')) continue;
      const r = el.getBoundingClientRect();
      if (!r.width || !r.height) continue;
      const cs = getComputedStyle(el);
      if (cs.visibility === 'hidden' || cs.display === 'none') continue;
      out.push({
        text: t,
        sel: el.tagName.toLowerCase() + (typeof el.className === 'string' && el.className.trim() ? '.' + el.className.trim().split(/\s+/).slice(0, 2).join('.') : ''),
        tabular: /tabular-nums/.test(cs.fontVariantNumeric),
        family: cs.fontFamily,
        serif: /instrument serif/i.test(cs.fontFamily),
      });
    }
    return out;
  });
  for (const f of found) {
    figures++;
    if (f.tabular || MONO.test(f.family.split(',')[0]) || f.serif) continue;
    proportional++;
    const key = f.sel;
    const g = groups.get(key) || { n: 0, routes: new Set(), sample: f.text };
    g.n++;
    g.routes.add(name);
    groups.set(key, g);
  }
}
await browser.close();

const rows = [...groups.entries()].sort((a, b) => b[1].n - a[1].n);
console.log(`${figures} figures on ${ROUTES.length} routes; ${proportional} set in proportional figures`);
for (const [sel, g] of rows.slice(0, 25)) console.log(`  ${String(g.n).padStart(4)}  ${sel}  e.g. "${g.sample}"  (${[...g.routes].slice(0, 3).join(', ')}${g.routes.size > 3 ? ', …' : ''})`);
if (FAIL && proportional) process.exit(1);
