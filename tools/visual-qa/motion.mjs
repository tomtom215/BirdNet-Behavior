// Does "reduce motion" reduce motion, on every page?
//
// Three ways to ask for it, none of which a screenshot can show:
//
//   os        the browser reports `prefers-reduced-motion: reduce`
//   operator  Settings > Display > Motion = Reduced (`bnb-motion` in storage,
//             stamped as `data-motion="reduced"` before first paint)
//   neither   the control: the same pages with nothing asked for
//
// For each, on every route in qa.mjs, after the page has settled, it lists the
// animations that are still *running* (`document.getAnimations()`) and the
// cross-document view transition if one was started, and fails on any whose
// duration is longer than a frame in the first two cases.
//
// The control exists because a gate that reports "0 running animations" proves
// nothing until it has been seen to count some: with no preference, the live
// dot pulses and the skeletons shimmer, and the run must find them. A probe
// that finds none in the control is not looking, and exits 1 for that.
//
// A fourth case (UIR-33): a chart's entrance plays once. With nothing asked for, each chart
// route is opened at 1280 px, left to finish arriving, then resized to a phone and back; no
// chart entrance (ck-grow, ck-reveal, ck-draw, ck-fade, dc-bloom) may be running after either
// resize. Its control: at least one of those routes must have been seen running an entrance
// on load, or the probe is not looking.
//
// Usage:  BASE=http://127.0.0.1:8611 CHROMIUM_PATH=... node motion.mjs
//         MUTATE=1 node motion.mjs   (self-test: puts a 5 s animation on <body>
//                                     through the CSSOM, which no preference
//                                     the page reads may excuse; must go red)
import { chromium } from 'playwright';
import { ROUTES } from './qa.mjs';

const BASE = process.env.BASE || 'http://127.0.0.1:8502';
const CHROMIUM_PATH = process.env.CHROMIUM_PATH || '';
const MUTATE = Boolean(process.env.MUTATE);

const FRAME_MS = 20; // anything this short is a state change, not motion

const browser = await chromium.launch(CHROMIUM_PATH ? { executablePath: CHROMIUM_PATH } : {});

const MODES = [
  { name: 'os', ctx: { reducedMotion: 'reduce' }, store: null, expectQuiet: true },
  { name: 'operator', ctx: { reducedMotion: 'no-preference' }, store: 'reduced', expectQuiet: true },
  { name: 'control', ctx: { reducedMotion: 'no-preference' }, store: null, expectQuiet: false },
];

const failures = [];
const control = { pages: 0, withMotion: 0, animations: 0 };
const summary = [];

for (const mode of MODES) {
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 }, ...mode.ctx });
  await context.addInitScript((m) => {
    try {
      if (m) localStorage.setItem('bnb-motion', m);
    } catch (e) { /* storage blocked: the OS case still runs */ }
  }, mode.store);
  const page = await context.newPage();
  let pages = 0;
  let loud = 0;
  for (const [name, route] of ROUTES) {
    try {
      await page.goto(BASE + route, { waitUntil: 'domcontentloaded', timeout: 25000 });
      await page.waitForLoadState('networkidle', { timeout: 9000 }).catch(() => {});
      await page.waitForTimeout(700);
      if (MUTATE) {
        await page.evaluate(() => {
          const sheet = document.styleSheets[0];
          sheet.insertRule('@keyframes zz-spin { from { opacity: 1 } to { opacity: .5 } }', sheet.cssRules.length);
          sheet.insertRule('body { animation: zz-spin 5s linear infinite !important; }', sheet.cssRules.length);
        });
        await page.waitForTimeout(50);
      }
      const running = await page.evaluate((frame) => document.getAnimations()
        .filter((a) => a.playState === 'running')
        .map((a) => {
          const t = a.effect && a.effect.getTiming ? a.effect.getTiming() : {};
          const target = a.effect && a.effect.target;
          return {
            name: a.animationName || a.transitionProperty || 'anim',
            ms: Number(t.duration) || 0,
            iterations: t.iterations,
            where: target ? (target.tagName.toLowerCase() + (target.className && typeof target.className === 'string' ? '.' + target.className.trim().split(/\s+/).slice(0, 2).join('.') : '')) : '?',
          };
        })
        .filter((a) => a.ms > frame), FRAME_MS);
      pages++;
      if (running.length) loud++;
      if (mode.name === 'control') {
        control.pages++;
        if (running.length) control.withMotion++;
        control.animations += running.length;
      } else if (mode.expectQuiet && running.length) {
        failures.push(`${mode.name} ${name}: ${running.length} still running: ${running.slice(0, 3).map((a) => `${a.name} ${a.ms}ms on ${a.where}`).join('; ')}`);
      }
    } catch (e) {
      failures.push(`${mode.name} ${name}: ${String(e).slice(0, 100)}`);
    }
  }
  summary.push(`${mode.name.padEnd(9)} ${pages} pages, ${loud} with a running animation`);
  await context.close();
}
// ---- a resize does not replay a chart's entrance (UIR-33) ----------------------------
const ENTRANCES = ['ck-grow', 'ck-reveal', 'ck-draw', 'ck-fade', 'dc-bloom'];
const CHART_ROUTES = ['/', '/patterns', '/patterns?tab=dawn', '/patterns?tab=migration', '/reports?tab=year', '/species?view=lifelist'];
let entrancesSeen = 0;
{
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 }, reducedMotion: 'no-preference' });
  const page = await context.newPage();
  const entrances = () => page.evaluate((names) => document.getAnimations()
    .filter((a) => a.playState === 'running' && names.includes(a.animationName))
    .map((a) => {
      const t = a.effect && a.effect.target;
      return `${a.animationName} on ${t ? t.tagName.toLowerCase() + '.' + String(t.className && t.className.baseVal !== undefined ? t.className.baseVal : t.className).trim().split(/\s+/).slice(0, 2).join('.') : '?'}`;
    }), ENTRANCES);
  for (const route of CHART_ROUTES) {
    try {
      await page.setViewportSize({ width: 1280, height: 900 });
      await page.goto(BASE + route, { waitUntil: 'domcontentloaded', timeout: 25000 });
      // Count the entrance while it plays, for the control, then let it finish.
      for (let i = 0; i < 20 && !entrancesSeen; i++) {
        if ((await entrances()).length) entrancesSeen++;
        await page.waitForTimeout(50);
      }
      await page.waitForLoadState('networkidle', { timeout: 9000 }).catch(() => {});
      await page.waitForTimeout(2500);
      for (const width of [390, 1280]) {
        await page.setViewportSize({ width, height: width === 390 ? 844 : 900 });
        await page.waitForTimeout(120);
        const replay = await entrances();
        if (replay.length) {
          failures.push(`resize ${route} to ${width} px: ${replay.length} chart entrances play again: ${replay.slice(0, 3).join('; ')}`);
        }
      }
    } catch (e) {
      failures.push(`resize ${route}: ${String(e).slice(0, 100)}`);
    }
  }
  await context.close();
  summary.push(`resize    ${CHART_ROUTES.length} chart pages resized twice each`);
}
if (!entrancesSeen && !MUTATE) {
  failures.push('the resize check never saw a chart entrance play on load: it is not looking, so a quiet resize means nothing');
}

await browser.close();

console.log(summary.join('\n'));
if (control.animations === 0 && !MUTATE) {
  failures.push('the control found no running animation on any page: the probe is not seeing motion, so a quiet result above means nothing');
}
console.log(`control: ${control.animations} running animations across ${control.withMotion} of ${control.pages} pages`);
if (failures.length) {
  console.error(`\nmotion: ${failures.length} failure(s)\n  ${failures.slice(0, 30).join('\n  ')}`);
  process.exit(1);
}
console.log('motion: reduced motion is honoured on every page, by both routes');
