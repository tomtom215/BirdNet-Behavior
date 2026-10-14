// Chart-label legibility gate for birdnet-web.
//
// The charts are fixed-canvas SVGs scaled to their card, and their text scales
// with them: measured on a 390 px phone, labels that were authored at 8 px
// rendered at 2.7-5 px on the dashboard, Patterns, Migration, History and the
// species page, and the visual-QA sweep, axe and every screenshot called those
// pages clean. `static/chart-text.js` now enlarges any label that would render
// under its floor; this gate measures what is actually on screen after it ran,
// on every route and both viewports, and fails below MIN_PX.
//
// Measured, not asserted: effective size is the computed font size times the
// SVG's screen scale (getScreenCTM), for every <text> that is rendered and not
// hidden. A label the fitter hid because it collided with a larger one is not
// graded — it is not on screen.
//
// Counterpart: a gate that finds no labels to grade is not a pass, so it fails
// when fewer than MIN_LABELS were measured across the run.
//
// Env:
//   BASE         base url                  (default http://127.0.0.1:8502)
//   VPS          csv of desktop,mobile     (default desktop,mobile)
//   MIN_PX       smallest allowed on-screen size   (default 8.5)
//   MIN_LABELS   fewest labels that must be graded (default 200)
//   ONLY         substring filter on route name
//   NO_FITTER    set to 1 to block chart-text.js, i.e. measure the pages as they
//                were before the fitter — the case this gate exists to fail
import { chromium } from 'playwright';
import { ROUTES } from './qa.mjs';

const BASE = process.env.BASE || 'http://127.0.0.1:8502';
const CHROMIUM_PATH = process.env.CHROMIUM_PATH || '';
const MIN_PX = Number(process.env.MIN_PX || 8.5);
const MIN_LABELS = Number(process.env.MIN_LABELS || 200);
const ONLY = process.env.ONLY || '';
const NO_FITTER = process.env.NO_FITTER === '1';
const VP = {
  desktop: { width: 1440, height: 900 },
  mobile: { width: 390, height: 844, touch: true },
};
const VPS = (process.env.VPS || 'desktop,mobile').split(',').filter(Boolean);

const browser = await chromium.launch(CHROMIUM_PATH ? { executablePath: CHROMIUM_PATH } : {});
const failures = [];
let graded = 0;

for (const vpName of VPS) {
  const vp = VP[vpName];
  const ctx = await browser.newContext({
    viewport: { width: vp.width, height: vp.height },
    hasTouch: Boolean(vp.touch),
    isMobile: Boolean(vp.touch),
  });
  const page = await ctx.newPage();
  if (NO_FITTER) await page.route('**/static/chart-text.js*', (r) => r.abort());
  for (const [name, route] of ROUTES) {
    if (name === 'notfound' || (ONLY && !name.includes(ONLY))) continue;
    await page.goto(BASE + route, { waitUntil: 'domcontentloaded', timeout: 25000 });
    await page.waitForLoadState('networkidle', { timeout: 9000 }).catch(() => {});
    // The fitter runs on a frame after layout and after each htmx swap.
    await page.waitForTimeout(1500);
    const r = await page.evaluate((minPx) => {
      const small = [];
      let n = 0;
      for (const t of document.querySelectorAll('svg text')) {
        const box = t.getBoundingClientRect();
        if (box.width === 0 || box.height === 0) continue;
        if (getComputedStyle(t).visibility === 'hidden') continue;
        const m = t.getScreenCTM();
        if (!m) continue;
        const px = parseFloat(getComputedStyle(t).fontSize) * Math.min(Math.hypot(m.a, m.b), Math.hypot(m.c, m.d));
        n += 1;
        if (px < minPx) small.push(`${t.textContent.trim().slice(0, 16)} ${px.toFixed(1)}px`);
      }
      return { n, small };
    }, MIN_PX);
    graded += r.n;
    if (r.small.length) {
      failures.push(`${name} @${vpName}: ${r.small.length} label(s) under ${MIN_PX}px, e.g. ${r.small.slice(0, 3).join(', ')}`);
    }
  }
  await ctx.close();
}
await browser.close();

console.log(`graded ${graded} chart labels across ${ROUTES.length} routes x ${VPS.join('+')}`);
if (graded < MIN_LABELS) {
  failures.push(`only ${graded} labels were graded (< ${MIN_LABELS}); the gate is not looking at the charts`);
}
if (failures.length) {
  console.error(`\n${failures.length} legibility failure(s):`);
  for (const f of failures) console.error(`  - ${f}`);
  process.exit(1);
}
console.log(`ok: every chart label is at least ${MIN_PX}px on screen`);
