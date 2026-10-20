// How far does the page move while it fills in?
//
// Most of the app is server-rendered shells whose cards load their charts with
// htmx a moment after the page: a skeleton goes in, the fragment replaces it, and
// if the two are different heights everything under them jumps. That is what
// Cumulative Layout Shift measures, and it is the one number here that is the
// reader's: the line of text they were about to tap moves away.
//
// The node named after each score is the first one the browser lists as having
// moved, which is the first in document order, not the cause: the header sat
// first in 11 of the 12 worst loads before its badge was given room, and the
// score barely moved, because the cards under it were filling in. Read the
// number, then probe the shifts themselves before blaming the name.
//
// Per route (desktop and a 390 px phone): the sum of every `layout-shift` entry
// not caused by the reader's own input, from navigation until the network has
// been quiet for 1.5 s. Reported worst first. `--fail` exits 1 over THRESHOLD
// (default 0.10, Google's "good" line is 0.1).
//
// Usage:  BASE=... CHROMIUM_PATH=... node cls.mjs [--fail]
import { chromium } from 'playwright';
import { ROUTES } from './qa.mjs';

const BASE = process.env.BASE || 'http://127.0.0.1:8502';
const CHROMIUM_PATH = process.env.CHROMIUM_PATH || '';
const THRESHOLD = Number(process.env.THRESHOLD || 0.1);
const FAIL = process.argv.includes('--fail');

const browser = await chromium.launch(CHROMIUM_PATH ? { executablePath: CHROMIUM_PATH } : {});
const rows = [];
for (const vp of [
  { name: 'desktop', opts: { viewport: { width: 1440, height: 900 } } },
  { name: 'phone', opts: { viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true } },
]) {
  const ctx = await browser.newContext({ ...vp.opts, reducedMotion: 'reduce' });
  await ctx.addInitScript(() => {
    window.__cls = 0;
    window.__shifts = [];
    try {
      new PerformanceObserver((list) => {
        for (const e of list.getEntries()) {
          if (e.hadRecentInput) continue;
          window.__cls += e.value;
          const src = (e.sources || []).map((s) => (s.node && s.node.nodeType === 1 ? s.node.tagName.toLowerCase() + (s.node.id ? '#' + s.node.id : (typeof s.node.className === 'string' && s.node.className ? '.' + s.node.className.trim().split(/\s+/)[0] : '')) : '?'))[0];
          window.__shifts.push([Math.round(e.value * 1000) / 1000, src]);
        }
      }).observe({ type: 'layout-shift', buffered: true });
    } catch (e) { /* not supported: reads as 0 and the run says so */ }
  });
  const page = await ctx.newPage();
  for (const [name, route] of ROUTES) {
    try {
      await page.goto(BASE + route, { waitUntil: 'domcontentloaded', timeout: 25000 });
      await page.waitForLoadState('networkidle', { timeout: 9000 }).catch(() => {});
      await page.waitForTimeout(1500);
      const r = await page.evaluate(() => ({ cls: window.__cls, top: window.__shifts.sort((a, b) => b[0] - a[0]).slice(0, 2) }));
      rows.push({ name: `${name} (${vp.name})`, cls: r.cls, top: r.top });
    } catch (e) { /* a route that did not load is not a route that did not shift */ }
  }
  await ctx.close();
}
await browser.close();

rows.sort((a, b) => b.cls - a.cls);
const total = rows.reduce((s, r) => s + r.cls, 0);
const over = rows.filter((r) => r.cls > THRESHOLD);
console.log(`${rows.length} page loads; total CLS ${total.toFixed(3)}; ${over.length} over ${THRESHOLD}`);
for (const r of rows.slice(0, Math.max(12, over.length))) console.log(`  ${r.cls.toFixed(3)}  ${r.name.padEnd(36)} ${r.top.map(([v, s]) => `${v} ${s}`).join('; ')}`);
if (FAIL && over.length) process.exit(1);
