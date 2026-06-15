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

// [outputName, route, theme]
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
  ['heatmap.png', '/patterns', 'light'],
  ['migration.png', '/patterns?tab=migration', 'light'],
  ['dawn-chorus.png', '/patterns?tab=dawn', 'light'],
  ['correlation.png', '/patterns?tab=together', 'light'],
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

const browser = await chromium.launch();
for (const [name, route, theme] of PAGES) {
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
