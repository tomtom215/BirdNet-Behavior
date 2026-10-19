// The one book image book.mjs cannot take: the first-run wizard's Accuracy step
// (docs/book/getting-started/first-steps.md). It is reached by clicking Next to
// the step whose heading names accuracy, so the script finds the step by its
// label rather than counting, and fails if there is none — an image of the
// wrong step is worse than no image.
import { chromium } from 'playwright';
import path from 'node:path';

const BASE = process.env.BASE || 'http://127.0.0.1:8502';
const OUT = process.env.OUT || '/home/user/BirdNet-Behavior/docs/book/images';
const browser = await chromium.launch(process.env.CHROMIUM_PATH ? { executablePath: process.env.CHROMIUM_PATH } : {});
const ctx = await browser.newContext({ viewport: { width: 1440, height: 1000 }, deviceScaleFactor: 1, colorScheme: 'light' });
await ctx.addInitScript(() => { try { localStorage.setItem('theme', 'light'); } catch (e) {} });
const page = await ctx.newPage();
await page.goto(BASE + '/onboarding', { waitUntil: 'networkidle' });
const total = await page.$$eval('section.ob-step', (n) => n.length);
if (!total) throw new Error('no onboarding steps found — did the markup change?');
let found = false;
for (let step = 1; step <= total; step++) {
  await page.waitForTimeout(400);
  const visible = await page.$$eval('section.ob-step', (nodes) =>
    nodes.filter((n) => n.offsetParent !== null).map((n) => n.textContent || ''));
  if (visible.some((t) => /how picky should it be/i.test(t))) { found = true; break; }
  if (step < total) await page.click('#ob-next');
}
if (!found) throw new Error('the Accuracy step was not found among ' + total + ' steps');
await page.screenshot({ path: path.join(OUT, 'onboarding-accuracy.png'), clip: { x: 0, y: 0, width: 1440, height: 600 } });
console.log('. onboarding-accuracy.png');
await browser.close();
