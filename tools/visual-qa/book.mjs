// Capture the curated book/README screenshots at a consistent 1440 width and a
// doc-friendly height (full page, clipped to MAXH so long list pages don't
// become unusably tall in the rendered book). Light theme, plus a dark
// dashboard. Output names match docs/book/images/. The share-page image is
// produced separately by share.mjs (it needs the signed-token flow).
import { chromium } from 'playwright';
import path from 'node:path';

const BASE = process.env.BASE || 'http://127.0.0.1:8502';
const OUT = process.env.OUT || '/home/user/BirdNet-Behavior/docs/book/images';
const W = 1440;
const MAXH = parseInt(process.env.MAXH || '1500', 10);
// ONLY=kiosk.png captures just the images whose name contains it (default: all), so one figure can be
// refreshed without touching the rest.
const ONLY = process.env.ONLY || '';
const enc = encodeURIComponent;
const T = new Date();
const TODAY = `${T.getUTCFullYear()}-${String(T.getUTCMonth() + 1).padStart(2, '0')}-${String(T.getUTCDate()).padStart(2, '0')}`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// Detail-page targets come from the live fixture (the regen wrapper reads them
// from the seeded DB), so the species-detail and detection-detail shots always
// land on a real species/detection rather than a hardcoded one that may not
// exist in the dataset.
const DETAIL_NAME = process.env.DETAIL_NAME || 'Northern Cardinal';
const DETAIL_DATE = process.env.DETAIL_DATE || TODAY;
const DETAIL_TIME = process.env.DETAIL_TIME || '06:00:00';

// A detection whose classifiers disagreed, found through the API the page itself documents: the newest
// one that one classifier heard and another scored below its own bar. Falls back to the newest one that
// more than one classifier answered. `null` when the station stores no per-classifier answers.
async function splitDetection() {
  try {
    const list = (await (await fetch(`${BASE}/api/v2/detections?limit=30`)).json()).detections || [];
    let fallback = null;
    for (const d of list) {
      const m = await (await fetch(`${BASE}/api/v2/detections/models?date=${d.date}&time=${enc(d.time)}&sci_name=${enc(d.sci_name)}`)).json();
      const states = (m.models || []).map((x) => x.state);
      if (states.includes('heard') && states.includes('below_bar')) return d;
      if (!fallback && states.length > 1) fallback = d;
    }
    return fallback;
  } catch (e) {
    return null;
  }
}
const SPLIT = await splitDetection();

// [outputName, route, theme, selector?] — with a selector the image is that element alone, not the page.
// "Who heard it" is drawn from the station's own API data, not a hard-coded detection.
// (A route of null is skipped, so a station with no stored per-classifier answers draws nothing.)
// Routes are the canonical v3-spine addresses (six homes + query-param views),
// not the pre-spine paths — the old `/heatmap`, `/gallery`, `/system` … still
// 308-redirect here, but the book should picture the real URLs. Output names are
// kept stable so the Markdown `![…](images/NAME.png)` references don't churn.
const PAGES = [
  ['dashboard.png', '/', 'light'],
  ['dashboard-dark.png', '/', 'dark'],
  ['today.png', '/', 'light'],
  ['species-list.png', '/species', 'light'],
  ['species-detail.png', `/species/detail?name=${enc(DETAIL_NAME)}`, 'light'],
  ['detection-detail.png', `/detections/detail?date=${DETAIL_DATE}&time=${DETAIL_TIME}&name=${enc(DETAIL_NAME)}`, 'light'],
  ['detection-reviews.png', '/detection-reviews', 'light'],
  [
    'detection-who-heard-it.png',
    SPLIT && `/detections/detail?date=${SPLIT.date}&time=${enc(SPLIT.time)}&name=${enc(SPLIT.com_name)}`,
    'light',
    '.ws-card',
  ],
  ['heatmap.png', '/patterns', 'light'],
  ['migration.png', '/patterns?tab=migration', 'light'],
  ['dawn-chorus.png', '/patterns?tab=dawn', 'light'],
  ['correlation.png', '/patterns?tab=together', 'light'],
  ['behavior.png', '/patterns?tab=behavior', 'light'],
  ['sequences.png', '/patterns?tab=sequences', 'light'],
  ['life-list.png', '/species?view=lifelist', 'light'],
  ['recordings.png', '/recordings', 'light'],
  ['gallery.png', '/species?view=photos', 'light'],
  ['weekly-report.png', '/reports', 'light'],
  ['year-in-review.png', '/reports?tab=year', 'light'],
  ['history.png', '/reports?tab=history', 'light'],
  ['notifications.png', '/notifications', 'light'],
  ['quarantine.png', '/quarantine', 'light'],
  ['system-health.png', '/station', 'light'],
  ['kiosk.png', '/kiosk', 'light'],
  ['onboarding.png', '/onboarding', 'light'],
  ['admin-audio.png', '/station/capture', 'light'],
  ['admin-backups.png', '/station/data', 'light'],
];

// Same knob as qa.mjs / axe.mjs: sandboxes ship their own Chromium rather than the build this playwright pins.
const browser = await chromium.launch(process.env.CHROMIUM_PATH ? { executablePath: process.env.CHROMIUM_PATH } : {});
for (const [name, route, theme, selector] of PAGES) {
  if (ONLY && !name.includes(ONLY)) continue;
  if (!route) {
    console.log(`- ${name}: skipped, nothing to draw on this station`);
    continue;
  }
  const ctx = await browser.newContext({
    viewport: { width: W, height: 1000 },
    deviceScaleFactor: 1,
    colorScheme: theme,
  });
  await ctx.addInitScript((t) => {
    try { localStorage.setItem('theme', t); localStorage.setItem('bnb-density', 'regular'); } catch (e) {}
  }, theme);
  const page = await ctx.newPage();
  try {
    await page.goto(BASE + route, { waitUntil: 'domcontentloaded', timeout: 25000 });
    await page.waitForLoadState('networkidle', { timeout: 9000 }).catch(() => {});
    await page.waitForFunction(() => [...document.images].every((i) => i.complete), { timeout: 8000 }).catch(() => {});
    await sleep(1400);
    if (selector) {
      const box = page.locator(selector).first();
      await box.waitFor({ state: 'visible', timeout: 8000 });
      await box.screenshot({ path: path.join(OUT, name) });
      console.log(`. ${name} (${selector})`);
      await ctx.close();
      continue;
    }
    const h = await page.evaluate(() => document.documentElement.scrollHeight);
    const clipH = Math.min(h, MAXH);
    // Playwright's `clip` can't exceed the viewport, so a clip taller than the
    // initial 1000px height was silently truncated. Grow the viewport to the
    // intended clip height (re-waiting for any reflowed images) so MAXH is the
    // real cap on tall list pages rather than the viewport.
    await page.setViewportSize({ width: W, height: clipH });
    await page.waitForFunction(() => [...document.images].every((i) => i.complete), { timeout: 8000 }).catch(() => {});
    await sleep(400);
    await page.screenshot({ path: path.join(OUT, name), clip: { x: 0, y: 0, width: W, height: clipH } });
    console.log(`. ${name} (${W}x${clipH}${h > MAXH ? ` clipped from ${h}` : ''})`);
  } catch (e) {
    console.log(`x ${name}: ${e.message}`);
  }
  await ctx.close();
}
await browser.close();
console.log('book images captured');
