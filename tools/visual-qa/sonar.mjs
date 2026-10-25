// The setup wizard's Welcome animation must run without tripping the page's CSP.
//
// The three "sonar" rings were SVG SMIL <animate> elements. Firefox applies SMIL
// to a presentation attribute as a style change, which the CSP's style-src-attr
// refuses: about 94 securitypolicyviolation reports a second ("Prevented too many
// CSP reports"), and rings that never faded (stroke-opacity stayed 1, where
// Chromium and WebKit went 0.63 to 0.42). Chromium does not report it at all, so a
// Chromium-only run cannot see the cause; it can see its effect on the markup, and
// both are held:
//
//   markup     there is no <animate>/<animateTransform>/<animateMotion> in the page
//   csp        zero `securitypolicyviolation` events over three seconds after load
//   moves      three CSS animations are running on the rings, and the computed
//              stroke-opacity of a ring changes between two samples (a probe that
//              sees a constant would pass a ring that does not fade)
//   reduced    with prefers-reduced-motion, and with the operator's Motion setting
//              (`bnb-motion` = reduced), no ring animation is running
//
// Browsers: Chromium (CHROMIUM_PATH) always; Firefox and WebKit when
// PLAYWRIGHT_BROWSERS_PATH holds them (BROWSERS=chromium,firefox,webkit chooses).
// MUTATE=smil puts the old SMIL markup back before the page loads and must go red.
//
// Usage:  BASE=http://127.0.0.1:8581 CHROMIUM_PATH=... node sonar.mjs
import { chromium, firefox, webkit } from 'playwright';

const BASE = process.env.BASE || 'http://127.0.0.1:8502';
const CHROMIUM_PATH = process.env.CHROMIUM_PATH || '';
const MUTATE = process.env.MUTATE || '';
const WANT = (process.env.BROWSERS || 'chromium').split(',');
const ENGINES = { chromium, firefox, webkit };

const OLD_SMIL = ['0', '1.6s', '3.2s'].map((b) => {
  const begin = b === '0' ? '' : ` begin="${b}"`;
  return `<circle cx="120" cy="120" r="30"><animate attributeName="r" values="30;110" dur="5s"${begin} repeatCount="indefinite"/><animate attributeName="stroke-opacity" values="0.7;0" dur="5s"${begin} repeatCount="indefinite"/></circle>`;
}).join('');

const failures = [];
const check = (ok, what) => { if (!ok) failures.push(what); console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); };

async function load(browser, ctxOpts, store) {
  const ctx = await browser.newContext({ viewport: { width: 1100, height: 820 }, ...ctxOpts });
  await ctx.addInitScript((m) => {
    window.__csp = 0;
    document.addEventListener('securitypolicyviolation', () => { window.__csp += 1; });
    try { if (m) localStorage.setItem('bnb-motion', m); } catch (e) { /* blocked */ }
  }, store);
  const page = await ctx.newPage();
  if (MUTATE === 'smil') {
    await page.route('**/onboarding', async (route) => {
      const res = await route.fetch();
      const body = (await res.text()).replace(/(<g class="sonar-rings"[^>]*>)[\s\S]*?(<\/g>)/, `$1${OLD_SMIL}$2`);
      await route.fulfill({ response: res, body });
    });
  }
  await page.goto(BASE + '/onboarding', { waitUntil: 'networkidle' });
  return { ctx, page };
}

const ringAnimations = (page) => page.evaluate(() =>
  document.getAnimations().filter((a) => a.effect && a.effect.target && a.effect.target.closest && a.effect.target.closest('.sonar')
    && a.playState === 'running' && (a.effect.getTiming().duration || 0) > 20).length);

for (const name of WANT) {
  const engine = ENGINES[name];
  if (!engine) { failures.push(`unknown browser ${name}`); continue; }
  console.log(`\n== ${name}`);
  const browser = await engine.launch(name === 'chromium' && CHROMIUM_PATH ? { executablePath: CHROMIUM_PATH } : {});

  {
    const { ctx, page } = await load(browser, {}, null);
    check((await page.$$eval('animate, animateTransform, animateMotion', (n) => n.length)) === 0,
      `${name}: no SMIL animation element in the page`);
    const opacityOf = () => page.$eval('.sonar-rings circle:nth-child(1)', (c) => getComputedStyle(c).strokeOpacity);
    const samples = [];
    for (let i = 0; i < 6; i++) { samples.push(parseFloat(await opacityOf())); await page.waitForTimeout(500); }
    check(new Set(samples.map((v) => v.toFixed(2))).size > 2, `${name}: the ring's stroke-opacity changes (${samples.map((v) => v.toFixed(2)).join(' ')})`);
    check((await ringAnimations(page)) === 3, `${name}: three ring animations are running`);
    // Counted from now, after the page has loaded: WebKit reports one
    // style-src-attr violation while it first renders the language <select> (on any
    // page with a select, e.g. /), which is not the animation. The SMIL rings
    // report continuously, hundreds in three seconds, so a window after load tells
    // the two apart.
    await page.evaluate(() => { window.__csp = 0; });
    await page.waitForTimeout(3000);
    const violations = await page.evaluate(() => window.__csp);
    check(violations === 0, `${name}: ${violations} securitypolicyviolation events over 3 s (expect 0)`);
    await ctx.close();
  }
  for (const [label, opts, store] of [
    ['OS reduced motion', { reducedMotion: 'reduce' }, null],
    ['operator Motion = Reduced', { reducedMotion: 'no-preference' }, 'reduced'],
  ]) {
    const { ctx, page } = await load(browser, opts, store);
    await page.waitForTimeout(500);
    check((await ringAnimations(page)) === 0, `${name}: no ring animation running under ${label}`);
    await ctx.close();
  }
  await browser.close();
}

if (failures.length) {
  console.error(`\n${failures.length} check(s) failed:\n- ${failures.join('\n- ')}`);
  process.exit(1);
}
console.log('\nsonar gate: all checks passed');
