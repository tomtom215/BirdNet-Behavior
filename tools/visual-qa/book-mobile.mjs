// Mobile (iPhone-13-class) companion to book.mjs: regenerates
// docs/book/images/mobile/*.png from the same demo seed. Each page is captured
// at 390x664 CSS px with deviceScaleFactor 3 → 1170x1992, matching the existing
// mobile docs set (a top-of-page phone view, bottom tab bar visible). Run after
// book.mjs against the same server:
//   BASE=http://127.0.0.1:8502 node book-mobile.mjs
import { chromium } from 'playwright';
import path from 'node:path';

const BASE = process.env.BASE || 'http://127.0.0.1:8502';
const OUT = process.env.OUT || '/home/user/BirdNet-Behavior/docs/book/images/mobile';
const W = 390;
const H = parseInt(process.env.MH || '664', 10); // 664 * DPR 3 = 1992 tall
const DPR = 3;
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

// Mirrors book.mjs PAGES (the desktop set), minus desktop-only chrome that has
// no phone layout. [outputName, route, theme]
const PAGES = [
  ['dashboard.png', '/', 'light'],
  ['dashboard-dark.png', '/', 'dark'],
  ['today.png', '/today', 'light'],
  ['species-list.png', '/species', 'light'],
  ['species-detail.png', `/species/detail?name=${enc(DETAIL_NAME)}`, 'light'],
  ['detection-detail.png', `/detections/detail?date=${DETAIL_DATE}&time=${DETAIL_TIME}&name=${enc(DETAIL_NAME)}`, 'light'],
  ['detection-reviews.png', '/detection-reviews', 'light'],
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
    viewport: { width: W, height: H },
    deviceScaleFactor: DPR,
    isMobile: true,
    hasTouch: true,
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
    await page.screenshot({ path: path.join(OUT, name) }); // viewport-sized: 1170x1992
    console.log(`. mobile/${name}`);
  } catch (e) {
    console.log(`x ${name}: ${e.message}`);
  }
  await ctx.close();
}
await browser.close();
console.log('mobile book images captured');
