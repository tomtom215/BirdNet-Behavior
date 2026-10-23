// Cross-engine gate: the service worker is dormant, and stays so on purpose.
//
// `static/sw.js` is written to make the dashboard instant and to keep a wall
// display up through an outage. It was registered with no `scope`, which makes
// the scope the script's own directory (`/static/`); `Service-Worker-Allowed: /`
// only permits a wider one. A worker controls only pages inside its scope, so
// for every release it has controlled nothing: `navigator.serviceWorker.controller`
// is null on every page in Chromium, Firefox and WebKit, and none of its
// offline behaviour has ever run. Nothing in the repository noticed, because
// its unit harness (`crates/birdnet-web/tests/sw_harness.mjs`) feeds the fetch
// handler requests itself and a browser never did.
//
// Turning it on is a decision with privacy and staleness consequences, not a
// side effect (sw.js lists what it needs). So this gate pins the state the
// comments describe, in each engine:
//
//   1. a worker registers and activates, with a scope inside `<base>/static/`
//      (under BASE=…/bn the scope carries the prefix: the registration used to
//      be `/static/sw.js` at the host's root, which is not the station's, and
//      registered nothing);
//   2. after a reload no dashboard page (`/`, `/kiosk`, `/station`) is controlled;
//   3. Cache Storage holds only `bnb-static-*`, and no HTML document in it
//      (`Cache.addAll` ignores `Cache-Control: no-store`, so a precached `/`
//      kept the signed-in dashboard of a private station after sign-out);
//   4. the counterpart, so 2 cannot pass because the probe is blind: a worker
//      registered here with scope `/` DOES control the page after a reload,
//      in the same engine.
//
//   node sw-state.mjs
//
// Env: BASE (default http://127.0.0.1:8502; also run it with the base-path
// fixture, e.g. http://127.0.0.1:8503/bn), CHROMIUM_PATH, ENGINES (default
// `chromium,firefox,webkit`; one that is asked for and cannot launch fails),
// PLAYWRIGHT_BROWSERS_PATH for the other two.
import { chromium, firefox, webkit } from 'playwright';

const BASE = (process.env.BASE || 'http://127.0.0.1:8502').replace(/\/+$/, '');
const { origin, pathname: PREFIX0 } = new URL(BASE);
const PREFIX = PREFIX0.replace(/\/+$/, '');
// The scope the counterpart gives its worker: the whole origin. Run at the
// root only. Under a prefix, Firefox 157 never let a worker control the
// dashboard (`/bn/species`) whatever scope it was given (`/`, `/bn`, `/bn/`),
// while Chromium and WebKit did, so a "controlled" probe there would fail for a
// reason that is not the code under test. That also means enabling the worker
// under a base path needs its own investigation, which the root leg cannot do.
const WIDE = '/';
const CHROMIUM_PATH = process.env.CHROMIUM_PATH || '';
const WANT = (process.env.ENGINES || 'chromium,firefox,webkit').split(',').map((s) => s.trim()).filter(Boolean);
const TYPES = { chromium, firefox, webkit };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const failures = [];
const check = (name, ok, detail) => {
  console.log(`  ${ok ? '\x1b[32mPASS\x1b[0m' : '\x1b[31mFAIL\x1b[0m'} ${name}${ok ? '' : `\n       ${detail}`}`);
  if (!ok) failures.push(name);
};

/** Registrations (scope path, whether active) and the page's controller. */
const snapshot = (page) => page.evaluate(async () => {
  const regs = await navigator.serviceWorker.getRegistrations();
  return {
    controlled: !!navigator.serviceWorker.controller,
    regs: regs.map((r) => ({ scope: new URL(r.scope).pathname, active: !!r.active })),
  };
});

/** Every Cache Storage entry as {cache, path, type}. */
const cacheEntries = (page) => page.evaluate(async () => {
  const out = [];
  for (const name of await caches.keys()) {
    const cache = await caches.open(name);
    for (const req of await cache.keys()) {
      const res = await cache.match(req);
      out.push({ cache: name, path: new URL(req.url).pathname, type: (res && res.headers.get('content-type')) || '' });
    }
  }
  return out;
});

async function untilActive(page, want = (s) => s.regs.some((r) => r.active)) {
  for (let i = 0; i < 60; i++) {
    const s = await snapshot(page);
    if (want(s)) return s;
    await sleep(250);
  }
  return snapshot(page);
}

async function oneEngine(name) {
  const type = TYPES[name];
  if (!type) return check(`${name}: known engine`, false, 'expected chromium, firefox or webkit');
  let browser;
  try {
    browser = await type.launch(name === 'chromium' && CHROMIUM_PATH ? { executablePath: CHROMIUM_PATH } : {});
  } catch (e) {
    return check(`${name}: launched`, false, String(e.message || e).split('\n')[0]);
  }
  try {
    console.log(`\n${name} ${browser.version()}`);

    // --- as shipped -------------------------------------------------------
    const ctx = await browser.newContext();
    const page = await ctx.newPage();
    // Every same-origin request the context makes, the worker's own included
    // where the engine reports them (Chromium and WebKit do; Firefox may not).
    const outside = new Set();
    ctx.on('request', (r) => {
      const u = new URL(r.url());
      if (u.origin !== origin || !PREFIX) return;
      if (u.pathname !== PREFIX && !u.pathname.startsWith(`${PREFIX}/`)) outside.add(u.pathname);
    });
    await page.goto(`${BASE}/`, { waitUntil: 'load' });
    const reg = await untilActive(page);
    check(`${name}: a worker registers and activates`, reg.regs.some((r) => r.active), `registrations: ${JSON.stringify(reg.regs)}`);
    const inScope = reg.regs.length > 0 && reg.regs.every((r) => r.scope === `${PREFIX}/static/`);
    check(`${name}: its scope is ${PREFIX}/static/ and no wider`, inScope, `scopes: ${JSON.stringify(reg.regs.map((r) => r.scope))}`);
    await sleep(1500); // let the install finish its precache
    for (const path of ['/', '/kiosk', '/station']) {
      await page.goto(`${BASE}${path}`, { waitUntil: 'load' });
      await page.reload({ waitUntil: 'load' });
      const s = await snapshot(page);
      check(`${name}: ${path} is not controlled by it`, s.controlled === false,
        'navigator.serviceWorker.controller is set: the worker is live on a dashboard page. Enabling it needs the list at the top of static/sw.js first; this gate pins the dormant state until then.');
    }
    const entries = await cacheEntries(page);
    const caches = [...new Set(entries.map((e) => e.cache))];
    check(`${name}: Cache Storage holds only bnb-static-* (${caches.join(', ') || 'nothing'})`,
      caches.every((c) => c.startsWith('bnb-static-')), `caches: ${JSON.stringify(caches)}`);
    const html = entries.filter((e) => /text\/html/i.test(e.type));
    check(`${name}: no HTML document is kept in Cache Storage`, html.length === 0,
      `kept: ${JSON.stringify(html.map((e) => `${e.cache} ${e.path}`))}`);
    check(`${name}: the install did precache something (${entries.length} entries)`, entries.length > 0,
      'an empty cache would make the two checks above vacuous');

    if (PREFIX) {
      check(`${name}: nothing it does leaves ${PREFIX}/ (the host's own paths belong to something else)`,
        outside.size === 0, `requests outside the prefix: ${JSON.stringify([...outside])}`);
    }

    // --- the counterpart: a probe that can see a controlled page -----------
    if (PREFIX) {
      console.log(`  (counterpart not run under ${PREFIX}: see WIDE above; the root leg grades it)`);
    } else {
      const ctx2 = await browser.newContext();
      const page2 = await ctx2.newPage();
      await page2.goto(`${BASE}/`, { waitUntil: 'load' });
      const outcome = await page2.evaluate(async (args) => {
        try {
          await navigator.serviceWorker.register(args.url, { scope: args.scope });
          return 'registered';
        } catch (e) { return `register failed: ${e.message}`; }
      }, { url: '/static/sw.js?v=gate', scope: WIDE });
      const wide = await untilActive(page2, (s) => s.regs.some((r) => r.active && r.scope === WIDE));
      await sleep(1000);
      await page2.reload({ waitUntil: 'load' });
      const after = await snapshot(page2);
      check(`${name}: counterpart — a worker given scope ${WIDE} does control the page (${outcome})`,
        after.controlled === true, `controlled=${after.controlled}, registrations ${JSON.stringify(wide.regs)}`);
      await ctx2.close();
    }
    await ctx.close();
  } finally {
    await browser.close();
  }
}

for (const name of WANT) await oneEngine(name);
if (failures.length) { console.log(`\n${failures.length} failed:\n  - ${failures.join('\n  - ')}`); process.exit(1); }
console.log('\nall passed');
