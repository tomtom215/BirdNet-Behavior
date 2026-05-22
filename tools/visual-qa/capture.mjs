// Capture every BirdNet-Behavior screen in light+dark × desktop+mobile.
// Usage: node capture.mjs [out_dir] [only_substring]
import { chromium } from 'playwright';
import fs from 'node:fs';
import path from 'node:path';

const BASE = process.env.BASE || 'http://127.0.0.1:8502';
const OUT = process.argv[2] || 'shots';
const ONLY = process.argv[3] || '';

const VIEWPORTS = [
  { name: 'desktop', width: 1440, height: 900 },
  { name: 'mobile', width: 390, height: 844 },
];
const THEMES = ['light', 'dark'];

const SONG = encodeURIComponent('Song Sparrow');
const CARD = encodeURIComponent('Northern Cardinal');

const ROUTES = [
  ['dashboard', '/'],
  ['today', '/today'],
  ['species', '/species'],
  ['species-detail', `/species/detail?name=${CARD}`],
  ['detection-detail', `/detections/detail?date=2026-05-22&time=21:10:31&name=${SONG}`],
  ['heatmap', '/heatmap'],
  ['correlation', '/correlation'],
  ['analytics', '/analytics'],
  ['timeseries', '/timeseries'],
  ['life-list', '/life-list'],
  ['recordings', '/recordings'],
  ['gallery', '/gallery'],
  ['weekly', '/weekly'],
  ['history', '/history'],
  ['notifications', '/notifications'],
  ['quarantine', '/quarantine'],
  ['system', '/system'],
  ['kiosk', '/kiosk'],
  ['live', '/live'],
  ['admin-overview', '/admin/overview'],
  ['admin-settings', '/admin/settings'],
  ['admin-species', '/admin/species'],
  ['admin-quality', '/admin/quality'],
  ['admin-rules', '/admin/rules'],
  ['admin-notifications', '/admin/notifications'],
  ['admin-system', '/admin/system'],
];

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function main() {
  fs.mkdirSync(OUT, { recursive: true });
  const browser = await chromium.launch();
  let n = 0;
  const failures = [];

  for (const vp of VIEWPORTS) {
    for (const theme of THEMES) {
      const context = await browser.newContext({
        viewport: { width: vp.width, height: vp.height },
        deviceScaleFactor: 1,
        colorScheme: theme === 'dark' ? 'dark' : 'light',
      });
      await context.addInitScript((t) => {
        try {
          localStorage.setItem('theme', t);
          localStorage.setItem('bnb-density', 'regular');
        } catch (e) {}
      }, theme);
      const page = await context.newPage();

      for (const [name, route] of ROUTES) {
        if (ONLY && !name.includes(ONLY)) continue;
        const file = path.join(OUT, `${name}__${theme}__${vp.name}.png`);
        try {
          await page.goto(BASE + route, { waitUntil: 'domcontentloaded', timeout: 20000 });
          await page.waitForLoadState('networkidle', { timeout: 8000 }).catch(() => {});
          await sleep(1300); // let HTMX partials + SVG settle
          await page.screenshot({ path: file, fullPage: true });
          n++;
          process.stdout.write(`. ${name} ${theme}/${vp.name}\n`);
        } catch (err) {
          failures.push(`${name} ${theme}/${vp.name}: ${err.message}`);
          process.stdout.write(`x ${name} ${theme}/${vp.name}: ${err.message}\n`);
        }
      }
      await context.close();
    }
  }

  await browser.close();
  console.log(`\nCaptured ${n} screenshots into ${OUT}/`);
  if (failures.length) {
    console.log(`\n${failures.length} FAILURES:`);
    failures.forEach((f) => console.log('  ' + f));
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
