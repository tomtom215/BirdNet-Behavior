// Base-path gate: served under a prefix, nothing the pages do leaves it (M8).
//
// The server rewrites the URLs in the HTML it sends and in its redirect
// headers. What it cannot rewrite is a URL a script builds in the browser —
// `location.href = '/recordings…'`, `htmx.ajax('GET', '/pages/…')`, a
// `fetch('/api/…')`. Under a reverse-proxy prefix each of those is a request
// outside the application: it 404s, or reaches something else entirely on a
// shared host. So this drives the pages and their scripted controls and fails
// on every same-origin request or socket whose path is not under the prefix.
//
// Run against a second fixture served under a prefix:
//   BIRDNET_BASE_PATH=/bn BNB_FIXTURE_ADDR=127.0.0.1:8503 BNB_FIXTURE_DIR=/tmp/bnb-bp \
//     cargo run -p birdnet-web --example screenshot_server --features analytics &
//   node base-path.mjs
//
// Env: BASE (default http://127.0.0.1:8503/bn), CHROMIUM_PATH.
import { chromium } from 'playwright';

const BASE = process.env.BASE || 'http://127.0.0.1:8503/bn';
const CHROMIUM_PATH = process.env.CHROMIUM_PATH || '';
const { origin, pathname: PREFIX } = new URL(BASE);
const escapes = new Map();

function note(url, where) {
  let u;
  try { u = new URL(url); } catch { return; }
  if (u.origin.replace(/^ws/, 'http') !== origin) return;
  if (u.pathname === PREFIX || u.pathname.startsWith(`${PREFIX}/`)) return;
  const key = u.pathname;
  if (!escapes.has(key)) escapes.set(key, where);
}

const PAGES = [
  '/', '/species', '/patterns', '/patterns?tab=together', '/patterns?tab=when',
  '/patterns?tab=dawn', '/patterns?tab=migration', '/patterns?tab=trends',
  '/recordings', '/recordings?view=clips', '/recordings?view=live', '/reports',
  '/search', '/station', '/notifications', '/quarantine', '/admin/settings',
];

async function drive(page, path) {
  await page.goto(`${BASE}${path}`, { waitUntil: 'networkidle' }).catch(() => {});
  await page.waitForTimeout(300);
}

async function main() {
  const browser = await chromium.launch({
    headless: true,
    ...(CHROMIUM_PATH ? { executablePath: CHROMIUM_PATH } : {}),
  });
  const page = await (await browser.newContext()).newPage();
  let current = '';
  page.on('request', (r) => note(r.url(), current));
  page.on('websocket', (ws) => note(ws.url(), current));

  for (const path of PAGES) {
    current = path;
    await drive(page, path);
  }

  // Scripted controls: each builds its URL in the browser.
  current = 'palette';
  await drive(page, '/');
  await page.keyboard.press('Control+k');
  await page.keyboard.type('robin');
  await page.waitForTimeout(700);
  await page.keyboard.press('ArrowDown');
  await Promise.all([page.waitForLoadState('networkidle').catch(() => {}), page.keyboard.press('Enter')]);

  current = 'today: expand, play, live detection';
  await drive(page, '/');
  const expand = page.locator('.x-expander button').first();
  if (await expand.count()) await expand.click().catch(() => {});
  await page.waitForTimeout(800);
  const play = page.locator('[data-play-src]').first();
  if (await play.count()) await play.click().catch(() => {});
  await page.evaluate(() => document.dispatchEvent(new CustomEvent('birdnet:detection', { detail: {} })));
  await page.waitForTimeout(800);

  current = 'recordings: play, lock';
  await drive(page, '/recordings?view=clips');
  const rplay = page.locator('[data-play-src]').first();
  if (await rplay.count()) await rplay.click().catch(() => {});
  await page.waitForTimeout(800);

  current = 'recordings: live listen';
  await drive(page, '/recordings?view=live');
  const listen = page.locator('#rc-listen-btn');
  if (await listen.count()) await listen.click().catch(() => {});
  await page.waitForTimeout(800);

  current = 'together: range, lookup, open tables';
  await drive(page, '/patterns?tab=together');
  await page.click('#range-controls [data-days="90"]').catch(() => {});
  await page.click('#species-input').catch(() => {});
  await page.keyboard.type('Robin');
  for (const s of await page.locator('summary').all()) await s.click().catch(() => {});
  await page.waitForTimeout(1000);

  current = 'when active: range';
  await drive(page, '/patterns?tab=when');
  for (const b of await page.locator('[data-days]').all()) await b.click().catch(() => {});
  await page.waitForTimeout(800);

  current = 'help drawer';
  await drive(page, '/');
  const help = page.locator('[data-help-drawer]').first();
  if (await help.count()) await help.click().catch(() => {});
  await page.waitForTimeout(800);

  await browser.close();

  if (escapes.size) {
    console.log(`\x1b[31mFAIL\x1b[0m ${escapes.size} request path(s) left ${PREFIX}:`);
    for (const [path, where] of escapes) console.log(`  ${path}   (during: ${where})`);
    process.exit(1);
  }
  console.log(`\x1b[32mPASS\x1b[0m every same-origin request stayed under ${PREFIX}`);
}

main().catch((e) => { console.error(e); process.exit(1); });
