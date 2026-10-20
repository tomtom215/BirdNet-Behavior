// What does a sticky, blurred bar cost every frame of a scroll?
//
// `backdrop-filter` re-samples and blurs whatever is behind the element on every
// frame the content under it moves, so a bar that is `position: sticky` or
// `fixed` pays for it for as long as the page scrolls. On a laptop that is
// invisible; on the phones and the old tablets that read a Raspberry Pi's
// dashboard it is the difference between a page that follows the finger and one
// that trails it. This measures it instead of arguing about it: the same page,
// the same scroll, the same browser, with the page's own CSS and with the blur
// switched off through the CSSOM, interleaved so drift in the machine cancels.
//
// Reports, per viewport and variant, the median and 95th-percentile frame
// interval over a scripted scroll, and the count of frames over 20 ms.
// Headless Chromium rasterises in software, which is the right proxy for a
// device with a weak GPU; the numbers are only meaningful as a ratio between
// the two variants on one machine.
//
// Usage:  BASE=... CHROMIUM_PATH=... node scroll-cost.mjs [route]
//         ROUNDS=6 (default)
import { chromium } from 'playwright';

const BASE = process.env.BASE || 'http://127.0.0.1:8502';
const CHROMIUM_PATH = process.env.CHROMIUM_PATH || '';
const ROUTE = process.argv[2] || '/today';
const ROUNDS = Number(process.env.ROUNDS || 6);

const browser = await chromium.launch(CHROMIUM_PATH ? { executablePath: CHROMIUM_PATH } : {});

const VIEWS = [
  { name: 'desktop 1440', opts: { viewport: { width: 1440, height: 900 } } },
  { name: 'phone 390', opts: { viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true } },
];

const median = (a) => [...a].sort((x, y) => x - y)[Math.floor(a.length / 2)];
const pct = (a, p) => [...a].sort((x, y) => x - y)[Math.min(a.length - 1, Math.floor(a.length * p))];

// Compositor and raster work does not show up in main-thread frame intervals
// until it blows the 16.7 ms budget, so the interval above can read identical for
// two pages that do very different amounts of drawing. The trace is where that
// work is counted: the time spent in raster tasks and GPU tasks over the same
// scripted scroll.
async function traced(browser, page, fn) {
  await browser.startTracing(page, { categories: ['cc', 'gpu', 'viz', 'disabled-by-default-devtools.timeline', 'devtools.timeline'] });
  await fn();
  const buf = await browser.stopTracing();
  const events = JSON.parse(buf.toString()).traceEvents || [];
  let raster = 0, gpu = 0, draws = 0;
  for (const e of events) {
    if (e.ph === 'X' && e.dur) {
      if (e.name === 'RasterTask') raster += e.dur;
      else if (e.name === 'GPUTask') gpu += e.dur;
    }
    if (e.name === 'DrawFrame' || e.name === 'Display::DrawAndSwap') draws++;
  }
  return { raster: raster / 1000, gpu: gpu / 1000, draws };
}

async function scrollRun(page) {
  return page.evaluate(() => new Promise((resolve) => {
    const frames = [];
    let last = performance.now();
    let n = 0;
    let dir = 1;
    const max = document.documentElement.scrollHeight - innerHeight;
    window.scrollTo(0, 0);
    const tick = (t) => {
      frames.push(t - last);
      last = t;
      const y = window.scrollY + dir * 24;
      if (y >= max) dir = -1;
      if (y <= 0) dir = 1;
      window.scrollTo(0, Math.max(0, Math.min(max, y)));
      if (++n < 120) requestAnimationFrame(tick);
      else resolve(frames.slice(5)); // the first frames pay for layout, not for the scroll
    };
    requestAnimationFrame(tick);
  }));
}

const rows = [];
for (const view of VIEWS) {
  const ctx = await browser.newContext(view.opts);
  const page = await ctx.newPage();
  await page.goto(BASE + ROUTE, { waitUntil: 'domcontentloaded' });
  await page.waitForLoadState('networkidle', { timeout: 9000 }).catch(() => {});
  await page.waitForTimeout(800);
  const doc = await page.evaluate(() => document.documentElement.scrollHeight - innerHeight);
  const blurred = await page.evaluate(() => {
    const out = [];
    for (const el of document.querySelectorAll('*')) {
      const b = getComputedStyle(el).backdropFilter;
      if (b && b !== 'none' && getComputedStyle(el).display !== 'none') out.push(`${el.tagName.toLowerCase()}.${String(el.className).split(' ')[0]} ${b}`);
    }
    return out;
  });
  const own = [];
  const flat = [];
  const trOwn = [];
  const trFlat = [];
  for (let i = 0; i < ROUNDS; i++) {
    trOwn.push(await traced(browser, page, async () => { own.push(...(await scrollRun(page))); }));
    await page.evaluate(() => {
      const s = document.styleSheets[0];
      if (!window.__flat) {
        s.insertRule('*, *::before, *::after { backdrop-filter: none !important; -webkit-backdrop-filter: none !important; }', s.cssRules.length);
        window.__flat = s.cssRules.length - 1;
      }
    });
    trFlat.push(await traced(browser, page, async () => { flat.push(...(await scrollRun(page))); }));
    await page.evaluate(() => {
      if (window.__flat !== undefined) {
        document.styleSheets[0].deleteRule(window.__flat);
        window.__flat = undefined;
      }
    });
  }
  rows.push({ view: view.name, scrollable: doc, blurred, own, flat, trOwn, trFlat });
  await ctx.close();
}
await browser.close();

for (const r of rows) {
  const f = (a) => `median ${median(a).toFixed(1)} ms, p95 ${pct(a, 0.95).toFixed(1)} ms, ${a.filter((x) => x > 20).length}/${a.length} frames over 20 ms`;
  console.log(`${r.view}  (${ROUTE}, ${r.scrollable}px of scroll)`);
  console.log(`  elements with a backdrop-filter: ${r.blurred.length ? r.blurred.join('; ') : 'none'}`);
  console.log(`  page as shipped : ${f(r.own)}`);
  console.log(`  blur switched off: ${f(r.flat)}`);
  const sum = (a, k) => a.reduce((x, y) => x + y[k], 0) / a.length;
  const t = (a) => `raster ${sum(a, 'raster').toFixed(1)} ms, gpu tasks ${sum(a, 'gpu').toFixed(1)} ms, ${sum(a, 'draws').toFixed(0)} frames drawn (mean of ${a.length} scrolls)`;
  console.log(`  trace, as shipped : ${t(r.trOwn)}`);
  console.log(`  trace, blur off   : ${t(r.trFlat)}`);
}
