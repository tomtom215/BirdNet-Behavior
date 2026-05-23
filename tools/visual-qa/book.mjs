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

// [outputName, route, theme]
const PAGES = [
  ['dashboard.png', '/', 'light'],
  ['dashboard-dark.png', '/', 'dark'],
  ['today.png', '/today', 'light'],
  ['species-list.png', '/species', 'light'],
  ['species-detail.png', `/species/detail?name=${enc('European Robin')}`, 'light'],
  ['detection-detail.png', `/detections/detail?date=${TODAY}&time=05:14:08&name=${enc('Eurasian Magpie')}`, 'light'],
  ['heatmap.png', '/heatmap', 'light'],
  ['migration.png', '/migration', 'light'],
  ['dawn-chorus.png', '/analytics/dawn-chorus', 'light'],
  ['correlation.png', '/correlation', 'light'],
  ['life-list.png', '/life-list', 'light'],
  ['recordings.png', '/recordings', 'light'],
  ['gallery.png', '/gallery', 'light'],
  ['weekly-report.png', '/weekly', 'light'],
  ['year-in-review.png', '/year-in-review', 'light'],
  ['history.png', '/history', 'light'],
  ['notifications.png', '/notifications', 'light'],
  ['quarantine.png', '/quarantine', 'light'],
  ['system-health.png', '/system', 'light'],
  ['kiosk.png', '/kiosk', 'light'],
  ['onboarding.png', '/onboarding', 'light'],
  ['admin-audio.png', '/admin/audio', 'light'],
  ['admin-backups.png', '/admin/backups', 'light'],
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
    await page.screenshot({ path: path.join(OUT, name), clip: { x: 0, y: 0, width: W, height: clipH } });
    console.log(`. ${name} (${W}x${clipH}${h > MAXH ? ` clipped from ${h}` : ''})`);
  } catch (e) {
    console.log(`x ${name}: ${e.message}`);
  }
  await ctx.close();
}
await browser.close();
console.log('book images captured');
