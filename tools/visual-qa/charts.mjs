// Behavioural gate for the chart kit (static/chart-kit.js): drive the charts the way a
// reader does, with a pointer, a finger and a keyboard, and assert what they do.
//
// A screenshot shows a chart at rest. Everything the kit adds happens when something
// is *done* to it, so nothing that grades a still picture (axe, the legibility and
// layout sweeps) can see it break: a tooltip that never appears, one series that
// does not come forward, a keyboard that cannot reach the chart, an entrance that
// plays for a reader who asked for none. Each check below is one of those.
//
//   tooltip on hover          the streamgraph, the hour bars, the grid, the calendar
//   tooltip on keyboard focus the same four, reached by Tab, read by arrow keys
//   the reading is announced  an aria-live region carries the same words
//   focus-and-dim             hovering a band / a legend entry puts the rest at
//                             --chart-dim; Up/Down do it from the keyboard
//   legend pin                a click pins, aria-pressed says so
//   no layout shift           the chart's box does not move or resize when read
//   reduced motion            the OS asking, and the operator's Motion setting: no
//                             chart animation runs at all (the control run, with
//                             neither asked for, must find one: a probe that finds
//                             none is not looking)
//   no replay on refresh      a chart swapped in again under the same id does not
//                             play its entrance again
//   theme                     the tooltip is repainted by a theme switch, no reload
//
// Run against the fixture server:
//   BASE=http://127.0.0.1:8502 CHROMIUM_PATH=... node charts.mjs
//
// Self-test (each must go red; this is how the gate was seen failing):
//   MUTATE=notip     the kit script is replaced by an empty one
//   MUTATE=nodim     the stylesheet stops dimming
//   MUTATE=motion    reduced motion is switched off for chart animation
//   MUTATE=noreplay  the kit never marks a repeat as `.ck-still`
import { chromium } from 'playwright';

const BASE = process.env.BASE || 'http://127.0.0.1:8502';
const CHROMIUM_PATH = process.env.CHROMIUM_PATH || '';
const MUTATE = process.env.MUTATE || '';
const failures = [];

function check(name, ok, detail = '') {
  if (ok) console.log(`  \x1b[32mPASS\x1b[0m ${name}`);
  else {
    failures.push({ name, detail });
    console.log(`  \x1b[31mFAIL\x1b[0m ${name}\n       ${detail}`);
  }
}

const browser = await chromium.launch(CHROMIUM_PATH ? { executablePath: CHROMIUM_PATH } : {});

async function open(route, { reduced = 'no-preference', motionSetting = null, theme = 'light', width = 1280, touch = false, init = null } = {}) {
  const context = await browser.newContext({
    viewport: { width, height: 900 },
    reducedMotion: reduced,
    colorScheme: theme,
    hasTouch: touch,
  });
  await context.addInitScript(([m, t]) => {
    try {
      if (m) localStorage.setItem('bnb-motion', m);
      localStorage.setItem('theme', t);
    } catch (e) { /* storage blocked */ }
  }, [motionSetting, theme]);
  if (init) await context.addInitScript(init);
  if (MUTATE === 'notip') {
    await context.route('**/static/chart-kit.js*', (r) => r.fulfill({ contentType: 'application/javascript', body: '' }));
  }
  if (MUTATE === 'noreplay') {
    await context.route('**/static/chart-kit.js*', async (r) => {
      const res = await r.fetch();
      const body = (await res.text()).replace('svg.classList.add("ck-still")', '');
      await r.fulfill({ response: res, body });
    });
  }
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', (e) => errors.push(e.message));
  await page.goto(BASE + route, { waitUntil: 'domcontentloaded', timeout: 30000 });
  await page.waitForLoadState('networkidle', { timeout: 10000 }).catch(() => {});
  // Through the CSSOM: the page's CSP forbids a <style> that is not its own.
  const inject = (rule) => page.evaluate((r) => document.styleSheets[0].insertRule(r, document.styleSheets[0].cssRules.length), rule);
  if (MUTATE === 'nodim') await inject('svg.ck-focus [data-ck-s], svg.ck-focus-i [data-ck-i] { opacity: 1 !important; }');
  if (MUTATE === 'motion') await inject('svg.ck-in .ck-grow { animation: ck-grow 3s linear both !important; }');
  return { page, context, errors };
}

const tipState = (page, scope) =>
  page.evaluate((sel) => {
    const root = document.querySelector(sel).closest('.ck');
    const tip = root.querySelector('.ck-tip');
    if (!tip) return { on: false, text: '', live: '' };
    const cs = getComputedStyle(tip);
    return { on: tip.classList.contains('on') && cs.visibility === 'visible' && Number(cs.opacity) > 0.5, text: tip.textContent, live: root.querySelector('.ck-live')?.textContent || '' };
  }, scope);

async function waitChart(page, sel) {
  await page.waitForSelector(`${sel}`, { state: 'attached', timeout: 15000 });
  // On screen: a pointer cannot hover what is below the fold (the Patterns page has a card above these).
  await page.locator(sel).evaluate((e) => e.scrollIntoView({ block: 'center', inline: 'center' }));
  await page.waitForTimeout(1200); // past the entrance
}

// ── the streamgraph: a "cols" chart with series ─────────────────────────────────
{
  console.log('Streamgraph (patterns)');
  const { page, context, errors } = await open('/patterns');
  const S = '#activity-streamgraph svg[data-ck="cols"]';
  await waitChart(page, S);
  const box0 = await page.locator(S).evaluate((e) => { const r = e.closest('.ck').getBoundingClientRect(); return [r.x, r.y, r.width, r.height]; });
  const r = await page.locator(S).boundingBox();
  await page.mouse.move(r.x + r.width * 0.55, r.y + r.height * 0.5);
  await page.waitForTimeout(350);
  let t = await tipState(page, S);
  check('stream: hover shows a tooltip', t.on, JSON.stringify(t));
  check('stream: the tooltip names a day and a total', /\d{1,2} [A-Z][a-z]{2}/.test(t.text) && /All species/.test(t.text), t.text);
  check('stream: the same words are announced (aria-live)', t.live.length > 10 && /All species/.test(t.live), t.live);
  const cursorVisible = await page.locator(`${S} .ck-cursor`).evaluate((e) => e.getAttribute('visibility') === 'visible');
  check('stream: a crosshair marks the day', cursorVisible);
  const box1 = await page.locator(S).evaluate((e) => { const r = e.closest('.ck').getBoundingClientRect(); return [r.x, r.y, r.width, r.height]; });
  check('stream: reading it moves nothing (no layout shift)', JSON.stringify(box0) === JSON.stringify(box1), `${box0} -> ${box1}`);

  // Focus and dim: rest the pointer on one band.
  const band = page.locator(`${S} path[data-ck-s="0"]`);
  const bb = await band.evaluate((e) => { const b = e.getBoundingClientRect(); return [b.x + b.width * 0.5, b.y + b.height * 0.5]; });
  // the middle of a band's box can be another band: find a point inside this band
  const hit = await page.evaluate(([sel, k]) => {
    const svg = document.querySelector(sel); const p = svg.querySelector(`path[data-ck-s="${k}"]`); const b = p.getBoundingClientRect();
    for (let fx = 0.1; fx < 0.95; fx += 0.05) for (let fy = 0.2; fy < 0.9; fy += 0.1) {
      const x = b.x + b.width * fx, y = b.y + b.height * fy; if (document.elementFromPoint(x, y) === p) return [x, y];
    }
    return null;
  }, [S, 0]);
  check('stream: a band can be pointed at', !!hit, 'no point of band 0 is on top');
  if (hit) {
    await page.mouse.move(hit[0], hit[1]);
    await page.waitForTimeout(450);
    const op = await page.evaluate((sel) => {
      const svg = document.querySelector(sel);
      return { cls: svg.classList.contains('ck-focus'), sel: Number(getComputedStyle(svg.querySelector('path[data-ck-s="0"]')).opacity), other: Number(getComputedStyle(svg.querySelector('path[data-ck-s="3"]')).opacity), dim: Number(getComputedStyle(document.documentElement).getPropertyValue('--chart-dim')) };
    }, S);
    check('stream: pointing at a band brings it forward', op.cls && op.sel > 0.95, JSON.stringify(op));
    check('stream: the others fall to --chart-dim', op.other < 0.5 && Math.abs(op.other - op.dim) < 0.08, JSON.stringify(op));
    t = await tipState(page, S);
    const firstRow = await page.locator(`${S}`).evaluate((e) => e.closest('.ck').querySelector('.ck-tip .ck-tip-r')?.textContent || '');
    const legend0 = await page.locator('#activity-streamgraph .ck-leg').first().innerText();
    check('stream: the focused species leads the tooltip', firstRow.includes(legend0.trim()), `${firstRow} vs ${legend0}`);
  }
  await page.mouse.move(2, 2);
  await page.waitForTimeout(400);
  t = await tipState(page, S);
  check('stream: leaving hides the tooltip', !t.on, JSON.stringify(t));

  // Legend: hover previews, click pins.
  const leg = page.locator('#activity-streamgraph .ck-leg').nth(1);
  await leg.hover();
  await page.waitForTimeout(400);
  const preview = await page.locator(S).evaluate((e) => e.classList.contains('ck-focus'));
  check('legend: hovering an entry puts that series forward', preview);
  await leg.click();
  await page.mouse.move(2, 2);
  await page.waitForTimeout(400);
  const pinned = await page.evaluate((sel) => ({ pressed: document.querySelector('#activity-streamgraph .ck-leg[aria-pressed="true"]') !== null, focus: document.querySelector(sel).classList.contains('ck-focus') }), S);
  check('legend: a click pins it (aria-pressed) and it stays after the pointer leaves', pinned.pressed && pinned.focus, JSON.stringify(pinned));
  await leg.click();

  // Keyboard.
  await page.locator(S).focus();
  await page.keyboard.press('ArrowLeft');
  await page.waitForTimeout(300);
  t = await tipState(page, S);
  check('stream: keyboard focus shows the same reading', t.on && /All species/.test(t.text), JSON.stringify(t));
  const before = t.live;
  await page.keyboard.press('ArrowLeft');
  await page.waitForTimeout(300);
  t = await tipState(page, S);
  check('stream: an arrow key moves to another day and announces it', t.live !== before && t.live.length > 10, `${before} -> ${t.live}`);
  await page.keyboard.press('ArrowDown');
  await page.waitForTimeout(300);
  const kdim = await page.locator(S).evaluate((e) => e.classList.contains('ck-focus'));
  check('stream: Down puts one species forward from the keyboard', kdim);
  await page.keyboard.press('Escape');
  await page.waitForTimeout(300);
  t = await tipState(page, S);
  check('stream: Escape lets go', !t.on, JSON.stringify(t));
  check('stream: no script errors', errors.length === 0, errors.join(' | '));
  await context.close();
}

// ── hour bars and the grid: "marks" charts ──────────────────────────────────────
{
  console.log('Hour bars and the grid (patterns)');
  const { page, context, errors } = await open('/patterns');
  const B = '#hourly-totals svg[data-ck="marks"]';
  const G = '#heatmap-grid svg[data-ck="marks"]';
  await waitChart(page, G);
  await waitChart(page, B);

  await page.locator(B).scrollIntoViewIfNeeded();
  const pos = await page.locator(`${B} .ck-hit[data-ck-i="6"]`).boundingBox();
  await page.mouse.move(pos.x + pos.width / 2, pos.y + pos.height * 0.7);
  await page.waitForTimeout(350);
  let t = await tipState(page, B);
  check('hours: hover shows that hour', t.on && /06:00/.test(t.text) && /Detections/.test(t.text), JSON.stringify(t));
  const dim = await page.evaluate((sel) => {
    const svg = document.querySelector(sel);
    const lit = svg.querySelector('path[data-ck-i="6"]'); const other = svg.querySelector('path[data-ck-i="12"]');
    return { cls: svg.classList.contains('ck-focus-i'), lit: Number(getComputedStyle(lit).opacity), other: other ? Number(getComputedStyle(other).opacity) : null };
  }, B);
  check('hours: the hour under the pointer is lit and the others dimmed', dim.cls && dim.lit > 0.95 && (dim.other === null || dim.other < 0.5), JSON.stringify(dim));
  await page.mouse.move(2, 2);
  await page.waitForTimeout(350);

  await page.locator(B).focus();
  await page.keyboard.press('ArrowRight');
  await page.waitForTimeout(300);
  const a = await tipState(page, B);
  await page.keyboard.press('ArrowRight');
  await page.waitForTimeout(300);
  const b = await tipState(page, B);
  check('hours: keyboard steps hour by hour', a.on && b.on && a.text !== b.text, `${a.text} / ${b.text}`);

  // The grid: Down moves a whole day.
  await page.locator(G).evaluate((e) => e.scrollIntoView({ block: 'center' }));
  await page.locator(G).focus();
  await page.waitForTimeout(250);
  const g0 = await tipState(page, G);
  await page.keyboard.press('ArrowDown');
  await page.waitForTimeout(250);
  const g1 = await tipState(page, G);
  const day = (s) => (s.match(/\b(Sun|Mon|Tue|Wed|Thu|Fri|Sat)\b/) || [])[1];
  const hr = (s) => (s.match(/(\d\d):00/) || [])[1];
  check('grid: keyboard focus lands on the busiest cell and reads it', g0.on && !!day(g0.text), JSON.stringify(g0));
  check('grid: Down moves to another weekday at the same hour', day(g0.text) !== day(g1.text) && hr(g0.text) === hr(g1.text), `${g0.text} -> ${g1.text}`);
  const gp = await page.locator(`${G} rect[data-ck-i="30"]`).boundingBox();
  await page.mouse.move(gp.x + gp.width / 2, gp.y + gp.height / 2);
  await page.waitForTimeout(300);
  const g2 = await tipState(page, G);
  check('grid: hover reads a cell', g2.on && /Mon 06:00/.test(g2.text), JSON.stringify(g2));
  check('charts: no script errors', errors.length === 0, errors.join(' | '));
  await context.close();
}

// ── the calendar on the Year in review ──────────────────────────────────────────
{
  console.log('Calendar (reports year)');
  const { page, context } = await open('/reports?tab=year');
  const C = 'svg.cal-svg[data-ck="marks"]';
  await waitChart(page, C);
  const n = await page.locator(`${C} [data-tip]`).count();
  check('calendar: a reading for every day drawn', n >= 28, `${n} cells`);
  await page.locator(C).focus();
  await page.waitForTimeout(250);
  const t0 = await tipState(page, C);
  await page.keyboard.press('ArrowRight');
  await page.waitForTimeout(250);
  const t1 = await tipState(page, C);
  check('calendar: focus reads the busiest day; Right moves a week', t0.on && t1.on && t0.text !== t1.text, `${t0.text} -> ${t1.text}`);
  await context.close();
}

// ── a finger ────────────────────────────────────────────────────────────────────
{
  console.log('Touch (a 390 px phone)');
  const { page, context } = await open('/patterns', { width: 390, touch: true });
  const H = '#hourly-totals svg[data-ck="marks"]';
  await waitChart(page, H);
  await page.locator(H).evaluate((e) => e.scrollIntoView({ block: 'center' }));
  await page.waitForTimeout(300);
  const hit = await page.locator(`${H} .ck-hit[data-ck-i="6"]`).boundingBox();
  await page.touchscreen.tap(hit.x + hit.width / 2, hit.y + hit.height * 0.7);
  await page.waitForTimeout(400);
  const t = await tipState(page, H);
  check('touch: a tap on an hour reads it', t.on && /06:00/.test(t.text), JSON.stringify(t));
  // The charts that scroll sideways inside their card must let a swipe scroll them; the others
  // keep pan-y so a sideways drag moves the reading and a vertical one scrolls the page.
  const ta = await page.evaluate(() => {
    const get = (sel) => { const e = document.querySelector(sel); return e ? getComputedStyle(e).touchAction : 'missing'; };
    return { stream: get('#activity-streamgraph svg[data-ck]'), grid: get('#heatmap-grid svg[data-ck]'), hours: get('#hourly-totals svg[data-ck]') };
  });
  check('touch: the sideways-scrolling streamgraph and grid accept a swipe', ta.stream === 'auto' && ta.grid === 'auto', JSON.stringify(ta));
  check('touch: the hour bars keep pan-y so a drag reads and a swipe down scrolls', ta.hours === 'pan-y', JSON.stringify(ta));
  await context.close();
}

// ── reduced motion, three ways ──────────────────────────────────────────────────
// The probe runs from before the page's first script: every frame it counts the chart
// animations that are running, and keeps the most it ever saw. A snapshot taken after the
// page settled would find none whatever the preference, and prove nothing.
const PROBE = () => {
  window.__ckMax = 0;
  const tick = () => {
    try {
      const n = document.getAnimations().filter((a) => a.playState === 'running' && a.effect && a.effect.target && a.effect.target.closest && a.effect.target.closest('svg.ck-in') && (a.effect.getTiming().duration || 0) > 20).length;
      if (n > window.__ckMax) window.__ckMax = n;
    } catch (e) { /* no animations API */ }
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
};
{
  console.log('Reduced motion');
  for (const [name, opts, expectQuiet] of [
    ['os', { reduced: 'reduce' }, true],
    ['operator', { reduced: 'no-preference', motionSetting: 'reduced' }, true],
    ['control', { reduced: 'no-preference' }, false],
  ]) {
    const { page, context } = await open('/patterns', { ...opts, init: PROBE });
    await page.waitForTimeout(1500);
    const seen = await page.evaluate(() => window.__ckMax);
    if (expectQuiet) check(`motion(${name}): no chart animation ever ran`, seen === 0, `${seen} running at the most`);
    else check('motion(control): the probe finds a chart animation when none is asked to stop', seen > 0, `${seen} at the most: a probe that finds none is not looking`);
    if (expectQuiet) {
      const settled = await page.evaluate(() => { const b = document.querySelector('#hourly-totals .ck-grow'); return b ? getComputedStyle(b).transform : 'none-element'; });
      check(`motion(${name}): the bars are at full height straight away`, settled === 'none' || settled === 'matrix(1, 0, 0, 1, 0, 0)', settled);
    }
    await context.close();
  }
}

// ── no replay on a refresh ──────────────────────────────────────────────────────
{
  console.log('Refresh');
  const { page, context } = await open('/');
  await page.waitForSelector('#today-daystrip svg[data-ck-id]', { timeout: 15000 });
  await page.waitForTimeout(1500);
  await page.evaluate(() => htmx.ajax('GET', '/pages/today-daystrip', '#today-daystrip'));
  await page.waitForTimeout(120);
  const r = await page.evaluate(() => ({ still: !!document.querySelector('#today-daystrip svg.ck-still'), running: document.getAnimations().filter((a) => a.playState === 'running' && a.animationName === 'ck-grow').length }));
  check('refresh: the same chart swapped in again is marked still and plays nothing', r.still && r.running === 0, JSON.stringify(r));
  await context.close();
}

// ── theme without a reload ──────────────────────────────────────────────────────
{
  console.log('Theme');
  const { page, context } = await open('/patterns', { theme: 'light' });
  const S = '#hourly-totals svg[data-ck="marks"]';
  await waitChart(page, S);
  const paint = async () => {
    const r = await page.locator(`${S} .ck-hit[data-ck-i="6"]`).boundingBox();
    await page.mouse.move(r.x + r.width / 2, r.y + r.height * 0.7);
    await page.waitForTimeout(300);
    return page.evaluate((sel) => { const tip = document.querySelector(sel).closest('.ck').querySelector('.ck-tip'); return getComputedStyle(tip).backgroundColor; }, S);
  };
  const light = await paint();
  await page.evaluate(() => document.documentElement.setAttribute('data-theme', 'dark'));
  await page.mouse.move(2, 2);
  await page.waitForTimeout(300);
  const dark = await paint();
  check('theme: the tooltip is repainted by a theme switch, no reload', light !== dark, `${light} / ${dark}`);
  await context.close();
}

await browser.close();
console.log(failures.length ? `\n${failures.length} FAILED` : '\nchart kit: all checks passed');
process.exit(failures.length ? 1 : 0);
