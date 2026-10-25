// The charts on the stations a lived-in fixture never is.
//
// A chart that is only ever drawn from a tidy year of fifteen species hides the
// defects that matter on the day a station is new, or crowded, or busy: a label
// that cannot hold a seven-digit count, a legend that has to wrap sixty names, a
// name too long for its band, an empty chart that prints "NaN", a single point that
// draws nothing. This drives the charts on a fixture started with
// `BNB_FIXTURE_PROFILE` and fails on any of those:
//
//   empty    nothing heard yet: every chart says so in words, none draws NaN
//   one      a single detection: every chart draws that one point
//   stress   sixty species with names up to 70 characters, a year of days, and one
//            hour with 1.6 million detections: no two labels overlap, none leaves
//            its drawing, a tooltip stays inside the window, and the ranges the
//            page offers (7, 14, 30, 90 days, and a 365-day request) all draw
//
// Usage (one fixture per profile, each on its own port and directory):
//   BNB_FIXTURE_PROFILE=stress BNB_FIXTURE_ADDR=127.0.0.1:8611 BNB_FIXTURE_DIR=... screenshot_server &
//   BASE=http://127.0.0.1:8611 PROFILE=stress CHROMIUM_PATH=... node charts-edge.mjs
//
// MUTATE=overlap puts two labels on top of each other in the first chart it finds, to
// watch the overlap check go red.
import { chromium } from 'playwright';

const BASE = process.env.BASE || 'http://127.0.0.1:8502';
const PROFILE = process.env.PROFILE || 'stress';
const CHROMIUM_PATH = process.env.CHROMIUM_PATH || '';
const MUTATE = process.env.MUTATE || '';
const failures = [];
let graded = 0;

function check(name, ok, detail = '') {
  if (ok) return;
  failures.push({ name, detail });
  console.log(`  \x1b[31mFAIL\x1b[0m ${name}\n       ${detail}`);
}

const browser = await chromium.launch(CHROMIUM_PATH ? { executablePath: CHROMIUM_PATH } : {});

/** Everything that can be measured about the charts on the page as it stands. */
const GRADE = () => {
  const out = [];
  for (const svg of document.querySelectorAll('svg[data-ck], svg.cal-svg, svg.viz-stream, svg.hm-svg')) {
    const r = svg.getBoundingClientRect();
    if (!r.width || !r.height) continue;
    const id = svg.getAttribute('data-ck-id') || svg.getAttribute('class');
    const texts = [...svg.querySelectorAll('text')]
      .filter((t) => getComputedStyle(t).visibility !== 'hidden')
      .map((t) => ({ el: t, b: t.getBoundingClientRect(), s: t.textContent }))
      .filter((t) => t.b.width > 0 && t.b.height > 0);
    const overlaps = [];
    for (let i = 0; i < texts.length; i++) for (let j = i + 1; j < texts.length; j++) {
      const a = texts[i].b, b = texts[j].b, pad = 1;
      if (a.left + pad < b.right && b.left + pad < a.right && a.top + pad < b.bottom && b.top + pad < a.bottom) overlaps.push([texts[i].s, texts[j].s]);
    }
    // A label may not leave its own drawing: chart-text.js grows a label that would be too
    // small and shortens one that then runs past the edge, so what is left is inside. (Not
    // "inside the window": a chart on a phone scrolls sideways inside its card, and the part
    // of it beyond the screen is still inside the drawing.)
    const outside = texts.filter((t) => t.b.left < r.left - 2 || t.b.right > r.right + 2).map((t) => t.s);
    const html = svg.outerHTML;
    out.push({
      id,
      labels: texts.length,
      overlaps: overlaps.slice(0, 4),
      outside: outside.slice(0, 4),
      nan: /NaN|Infinity|undefined/.test(html.replace(/<title>[\s\S]*?<\/title>|<desc>[\s\S]*?<\/desc>/g, '')),
      width: r.width,
    });
  }
  return out;
};

const MUTATE_OVERLAP = () => {
  const svg = document.querySelector('svg[data-ck]');
  const ts = [...svg.querySelectorAll('text')];
  if (ts.length > 2) { ts[1].setAttribute('x', ts[0].getAttribute('x')); ts[1].setAttribute('y', ts[0].getAttribute('y')); ts[1].style.visibility = 'visible'; }
};

async function visit(page, label, route, after) {
  const errors = [];
  const onErr = (e) => errors.push(e.message);
  page.on('pageerror', onErr);
  await page.goto(BASE + route, { waitUntil: 'domcontentloaded', timeout: 60000 });
  await page.waitForLoadState('networkidle', { timeout: 15000 }).catch(() => {});
  await page.waitForTimeout(1500);
  if (after) await after(page);
  if (MUTATE === 'overlap' && !globalThis.__mutated) { globalThis.__mutated = true; await page.evaluate(MUTATE_OVERLAP); await page.waitForTimeout(100); }
  const charts = await page.evaluate(GRADE);
  page.off('pageerror', onErr);
  graded += charts.length;
  check(`${label}: no script errors`, errors.length === 0, errors.join(' | '));
  for (const c of charts) {
    check(`${label} [${c.id}]: no two labels overlap`, c.overlaps.length === 0, JSON.stringify(c.overlaps));
    check(`${label} [${c.id}]: no label leaves its drawing`, c.outside.length === 0, JSON.stringify(c.outside));
    check(`${label} [${c.id}]: no NaN or Infinity in the drawing`, !c.nan);
  }
  const body = await page.evaluate(() => document.body.innerText);
  check(`${label}: the page does not print NaN`, !/\bNaN\b/.test(body), body.match(/.{30}\bNaN\b.{30}/)?.[0] || '');
  return charts;
}

const legs = [
  { name: 'desktop light', ctx: { viewport: { width: 1440, height: 900 }, colorScheme: 'light' } },
  { name: 'phone dark', ctx: { viewport: { width: 390, height: 844 }, colorScheme: 'dark', hasTouch: true } },
];

for (const leg of legs) {
  console.log(`${PROFILE}: ${leg.name}`);
  const context = await browser.newContext(leg.ctx);
  await context.addInitScript((t) => { try { localStorage.setItem('theme', t); } catch (e) {} }, leg.ctx.colorScheme);
  const page = await context.newPage();
  const L = `${PROFILE}/${leg.name}`;

  const today = await visit(page, `${L} Today`, '/');
  const pat = await visit(page, `${L} Patterns`, '/patterns');
  for (const days of [14, 30, 90]) {
    await visit(page, `${L} Patterns ${days} days`, '/patterns', async (p) => {
      await p.locator(`#range-controls button[data-days="${days}"]`).click();
      await p.waitForTimeout(2200);
    });
  }
  // The page offers four ranges; the endpoint takes up to a year.
  await visit(page, `${L} Patterns 365 days`, '/patterns', async (p) => {
    await p.evaluate(() => htmx.ajax('GET', '/pages/activity-streamgraph?days=365', '#activity-streamgraph'));
    await p.evaluate(() => htmx.ajax('GET', '/pages/heatmap-grid?days=365', '#heatmap-grid'));
    await p.evaluate(() => htmx.ajax('GET', '/pages/hourly-totals?days=365', '#hourly-totals'));
    await p.waitForTimeout(2500);
  });
  await visit(page, `${L} Trends`, '/patterns?tab=trends');
  await visit(page, `${L} Year in review`, '/reports?tab=year');
  await visit(page, `${L} Weekly`, '/reports');
  await visit(page, `${L} Life list`, '/species?view=lifelist');
  const first = await (await fetch(`${BASE}/api/v2/detections?limit=1`)).json().catch(() => ({ detections: [] }));
  const name = first.detections?.[0]?.com_name;
  if (name) await visit(page, `${L} Species`, `/species/detail?name=${encodeURIComponent(name)}`);

  if (PROFILE === 'empty') {
    // Nothing heard: every chart slot says so in words rather than drawing a chart of nothing.
    check(`${L}: no chart is drawn for nothing`, pat.length === 0 && today.length === 0, `${pat.length + today.length} charts drawn on an empty station`);
  }
  if (PROFILE === 'one') {
    check(`${L}: the single detection draws a chart on Today`, today.length >= 1, 'no chart on Today');
  }
  if (PROFILE === 'stress') {
    check(`${L}: the charts were found`, pat.length >= 3, `${pat.length} on Patterns`);
    // The seven-digit hour: its number is compact on the axis and exact in the tooltip.
    await page.goto(BASE + '/patterns', { waitUntil: 'domcontentloaded' });
    await page.waitForSelector('#hourly-totals svg[data-ck]', { timeout: 20000 });
    await page.locator('#range-controls button[data-days="14"]').click();
    await page.waitForTimeout(2200);
    const axis = await page.evaluate(() => [...document.querySelectorAll('#hourly-totals svg text')].map((t) => t.textContent));
    check(`${L}: a 1.6 million hour is compacted on the axis`, axis.some((t) => /^[12](\.\d)?M$/.test(t)), axis.join(' '));
    const hit = page.locator('#hourly-totals .ck-hit[data-ck-i="6"]');
    await hit.evaluate((e) => e.scrollIntoView({ block: 'center', inline: 'center' }));
    await page.waitForTimeout(300);
    const b = await hit.boundingBox();
    if (b) {
      await page.mouse.move(b.x + b.width / 2, b.y + b.height * 0.8);
      await page.waitForTimeout(400);
      const tip = await page.evaluate(() => { const t = document.querySelector('#hourly-totals .ck-tip'); const r = t.getBoundingClientRect(); return { text: t.textContent, l: r.left, r: r.right, w: innerWidth, on: t.classList.contains('on') }; });
      check(`${L}: the tooltip holds the exact seven-digit count`, /1,6\d\d,\d{3}/.test(tip.text), tip.text);
      check(`${L}: the tooltip stays inside the window`, tip.l >= -1 && tip.r <= tip.w + 1, JSON.stringify(tip));
    }
    // Sixty species fold into eight bands and one more; the legend wraps and nothing overflows sideways.
    const legend = await page.evaluate(() => document.querySelectorAll('#activity-streamgraph .ck-leg').length);
    check(`${L}: sixty species are eight bands and "other species"`, legend === 9, `${legend} legend entries`);
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - innerWidth);
    check(`${L}: the page does not scroll sideways`, overflow <= 1, `${overflow}px`);
  }
  await context.close();
}

await browser.close();
if (graded < (PROFILE === 'empty' ? 0 : 20)) {
  failures.push({ name: 'coverage', detail: `graded only ${graded} charts: a run that found none cannot pass` });
  console.log(`  \x1b[31mFAIL\x1b[0m graded only ${graded} charts`);
}
console.log(failures.length ? `\n${failures.length} FAILED (${graded} charts graded)` : `\nchart edge cases (${PROFILE}): ${graded} charts graded, all checks passed`);
process.exit(failures.length ? 1 : 0);
