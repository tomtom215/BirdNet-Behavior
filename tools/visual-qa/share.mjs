// Capture the share-link flow: a real /r/{token} page (issued from the
// detection-detail "Share clip" button), the tampered-token "gone" page, and
// the branded 404 — in light + dark. Set BNB_SHARE_SECRET on the server so the
// token is stable. Requires a detection at the date/time/species below.
import { chromium } from 'playwright';
import fs from 'node:fs';
import path from 'node:path';

const BASE = process.env.BASE || 'http://127.0.0.1:8502';
const OUT = process.env.OUT || 'shots';
const enc = encodeURIComponent;
const T = new Date();
const TODAY = `${T.getUTCFullYear()}-${String(T.getUTCMonth() + 1).padStart(2, '0')}-${String(T.getUTCDate()).padStart(2, '0')}`;
const DETAIL = `/detections/detail?date=${TODAY}&time=05:14:08&name=${enc('Eurasian Magpie')}`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

fs.mkdirSync(OUT, { recursive: true });
const browser = await chromium.launch();

for (const theme of ['light', 'dark']) {
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 }, colorScheme: theme });
  await ctx.addInitScript((t) => { try { localStorage.setItem('theme', t); } catch (e) {} }, theme);
  const page = await ctx.newPage();

  // 1) load detail page, extract the real token from the Share clip button
  await page.goto(BASE + DETAIL, { waitUntil: 'domcontentloaded', timeout: 20000 });
  const token = await page.evaluate(() => {
    const m = document.body.innerHTML.match(/\/r\/([A-Za-z0-9_\-.~%]+)/);
    return m ? m[1] : null;
  });
  if (!token) {
    console.error(`[${theme}] could not extract share token from detail page`);
  } else {
    console.log(`[${theme}] token=${token.slice(0, 24)}…`);
    // 2) the real share page — wait for the (large) spectrogram img to paint
    await page.goto(`${BASE}/r/${token}`, { waitUntil: 'domcontentloaded', timeout: 20000 });
    await page
      .waitForFunction(() => {
        const i = document.querySelector('img');
        return i && i.complete && i.naturalWidth > 0;
      }, { timeout: 12000 })
      .catch(() => {});
    await sleep(800);
    await page.screenshot({ path: path.join(OUT, `share-page__${theme}__desktop.png`), fullPage: true });
    // 3) tampered token -> gone page
    await page.goto(`${BASE}/r/${token}XX`, { waitUntil: 'domcontentloaded', timeout: 20000 });
    await sleep(600);
    await page.screenshot({ path: path.join(OUT, `share-gone__${theme}__desktop.png`), fullPage: true });
  }

  // 4) branded 404
  await page.goto(`${BASE}/no-such-page-here`, { waitUntil: 'domcontentloaded', timeout: 20000 });
  await sleep(500);
  await page.screenshot({ path: path.join(OUT, `notfound__${theme}__desktop.png`), fullPage: true });

  await ctx.close();
}
await browser.close();
console.log('share/gone/404 captured');
