// DOM-sink gate: a URL a script reads back from the page is a URL to follow,
// not one to trust (CodeQL js/xss-through-dom, js/insecure-randomness).
//
// Five places in the templates read a URL out of a `data-*` attribute and hand
// it to a sink: the now-playing panel's download link and the bulk download
// (`recordings.html`), the help drawer's "Open full page" link and its
// fallback link (`_partial_help_drawer.html`), and the command palette's
// `location.href` (`_partial_cmdk.html`). The server only ever writes a
// constant, escaped path into those attributes, so nothing is exploitable
// today; this gate sets each attribute to a hostile value in the browser, as a
// future template change or an injected element would, and fails when the
// value reaches its sink. A `javascript:` URL and another origin are hostile;
// a markup-bearing path is hostile to the one sink that used to build HTML.
//
// Every hostile case has its counterpart: the ordinary same-origin path must
// still arrive, or a gate that refused everything would pass.
//
// The HLS listener id (recordings.html) is checked here too: it is made from
// `crypto.getRandomValues` when `crypto.randomUUID` is missing, which it is on
// a station served over plain http. Not a secret, but it must not be
// `Math.random()`.
//
// Run against the fixture server:
//   BASE=http://127.0.0.1:8541 CHROMIUM_PATH=... node dom-sinks.mjs
import { chromium } from 'playwright';

const BASE = (process.env.BASE || 'http://127.0.0.1:8502').replace(/\/$/, '');
const CHROMIUM_PATH = process.env.CHROMIUM_PATH || '';
const ORIGIN = new URL(BASE).origin;
const EVIL = 'https://evil.test/pwn.wav';
const JS = 'javascript:window.__pwned=1';
const MARKUP = '/help/nope"><img id="pwned-img" src="x">';

const failures = [];
const passes = [];
function check(name, ok, detail = '') {
  (ok ? passes : failures).push(ok ? name : `${name}${detail ? `  (${detail})` : ''}`);
}

// Resolves against the page, as the browser does when it follows the value.
const sameOrigin = (v, base) => {
  try { return new URL(v, base).origin === ORIGIN; } catch { return false; }
};

async function newPage(browser, init, arg) {
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  const page = await ctx.newPage();
  const seen = { evil: [], pageErrors: [] };
  // Nothing may reach the hostile host; anything that tries is recorded and refused.
  await ctx.route(/^https?:\/\/evil\.test\//, (r) => { seen.evil.push(r.request().url()); return r.abort(); });
  page.on('pageerror', (e) => seen.pageErrors.push(String(e)));
  if (init) await page.addInitScript(init, arg);
  return { ctx, page, seen };
}

async function recordings(browser) {
  const t = await newPage(browser, () => {
    // Record every programmatic anchor click instead of performing it.
    window.__clicks = [];
    // The panel is filled when playback begins; whether a hostile source can
    // actually play is not the question, so playback always "begins".
    HTMLMediaElement.prototype.play = () => Promise.resolve();
    HTMLAnchorElement.prototype.click = function () {
      window.__clicks.push({ href: this.getAttribute('href'), download: this.hasAttribute('download') });
    };
  });
  await t.page.goto(`${BASE}/recordings?view=clips`, { waitUntil: 'networkidle' });
  return t;
}

async function nowPlayingAndBulk(browser) {
  // One legitimate row, one hostile row each, so the counterpart sits beside the case.
  const { ctx, page, seen } = await recordings(browser);
  const n = await page.locator('#rc-clips [data-play-src]').count();
  check('recordings: the fixture has at least 3 playable rows', n >= 3, `found ${n}`);
  await page.evaluate(([a, b]) => {
    const els = document.querySelectorAll('#rc-clips [data-play-src]');
    els[1].setAttribute('data-play-src', a);
    els[2].setAttribute('data-play-src', b);
  }, [JS, EVIL]);

  // 1. Now-playing panel: the download link (recordings.html, `dl.setAttribute('href', src)`).
  for (const [label, i, hostile] of [['legitimate path', 0, false], ['javascript: URL', 1, true], ['another origin', 2, true]]) {
    await page.locator('#rc-clips [data-play-src]').nth(i).click({ force: true }).catch(() => {});
    await page.waitForTimeout(250);
    const hrefs = await page.evaluate(() =>
      Array.from(document.querySelectorAll('.rc-hp-dl')).map((a) => a.getAttribute('href')));
    if (!hostile) {
      check(`download link: ${label} reaches the link`,
        hrefs.length > 0 && hrefs.every((h) => h && sameOrigin(h, page.url()) && h.includes('/api/v2/recordings/')),
        JSON.stringify(hrefs));
    } else {
      check(`download link: ${label} is refused`,
        hrefs.length > 0 && hrefs.every((h) => h === null || sameOrigin(h, page.url())),
        JSON.stringify(hrefs));
    }
  }
  check('download link: nothing was requested from the hostile host', seen.evil.length === 0, seen.evil.join(' '));

  // 2. Bulk download (recordings.html, `a.href = p.getAttribute('data-play-src')`).
  await page.evaluate(() => { window.__clicks.length = 0; });
  await page.locator('#rc-selmode').click();
  const rows = page.locator('#rc-clips .rc-row');
  for (let i = 0; i < 3; i++) await rows.nth(i).locator('.rc-check').click({ force: true }).catch(() => {});
  const selected = await page.locator('#rc-clips .rc-row.sel').count();
  check('bulk download: three rows were selected', selected === 3, `selected ${selected}`);
  await page.locator('#rc-bulk-dl').click();
  const clicks = await page.evaluate(() => window.__clicks);
  const downloads = clicks.filter((c) => c.download);
  check('bulk download: the legitimate clip is offered',
    downloads.some((c) => c.href && sameOrigin(c.href, page.url()) && c.href.includes('/api/v2/recordings/')),
    JSON.stringify(clicks));
  check('bulk download: no javascript: or foreign URL is offered',
    downloads.every((c) => c.href && sameOrigin(c.href, page.url())), JSON.stringify(clicks));
  await ctx.close();
}

async function helpDrawer(browser) {
  const doc = '<!doctype html><html><body><main><h1>Gate page</h1><p id="gate-body">ok</p></main></body></html>';
  for (const [label, trigger, hostile] of [
    ['legitimate page', '/help/gate-legit.html#frag', false],
    ['legitimate path that fails to load', '/help/gate-missing.html', false],
    ['javascript: URL', JS, true],
    ['another origin', EVIL, true],
    ['markup in the path', MARKUP, true],
  ]) {
    const { ctx, page, seen } = await newPage(browser);
    let fixtureUsed = 0;
    // The manual is built into docs/book/_generated and is absent from a CI
    // checkout, so the page this case needs is served here, and counted.
    await ctx.route(`${ORIGIN}/help/gate-legit.html`, (r) => { fixtureUsed++; return r.fulfill({ status: 200, contentType: 'text/html', body: doc }); });
    await ctx.route(`${ORIGIN}/help/gate-missing.html`, (r) => r.fulfill({ status: 404, contentType: 'text/plain', body: 'no' }));
    await page.goto(`${BASE}/`, { waitUntil: 'networkidle' });
    await page.evaluate((v) => document.querySelector('[data-help-drawer]').setAttribute('data-help-drawer', v), trigger);
    await page.locator('[data-help-drawer]').first().click();
    await page.waitForFunction(() => {
      const t = document.getElementById('bnb-help-drawer-title');
      return t && t.textContent !== 'Loading…';
    }, null, { timeout: 8000 }).catch(() => {});
    const r = await page.evaluate(() => {
      const open = document.getElementById('bnb-help-drawer-openfull');
      const body = document.getElementById('bnb-help-drawer-body');
      return {
        title: document.getElementById('bnb-help-drawer-title').textContent,
        open: open.getAttribute('href'),
        links: Array.from(body.querySelectorAll('a')).map((a) => a.getAttribute('href')),
        injected: !!document.getElementById('pwned-img') || body.querySelector('img') !== null,
        text: body.textContent,
        pwned: window.__pwned === 1,
      };
    });
    const where = `help drawer, ${label}`;
    check(`${where}: "Open full page" stays on this origin`, r.open !== null && sameOrigin(r.open, page.url()), String(r.open));
    check(`${where}: no link in the drawer leaves this origin`, r.links.every((h) => h !== null && sameOrigin(h, page.url())), JSON.stringify(r.links));
    check(`${where}: no element is injected and nothing ran`, !r.injected && !r.pwned);
    check(`${where}: nothing was requested from the hostile host`, seen.evil.length === 0, seen.evil.join(' '));
    if (label === 'legitimate page') {
      check(`${where}: the page loaded, from the fixture`, fixtureUsed === 1 && r.text.includes('ok') && /gate-legit\.html#frag$/.test(r.open), `${r.title} used=${fixtureUsed} open=${r.open}`);
    } else if (label === 'javascript: URL' || label === 'another origin') {
      check(`${where}: the drawer says so and "Open full page" goes to the manual's front page`, /help\/index\.html$/.test(r.open) && r.text.includes('not one of this station'), `${r.open} / ${r.text}`);
    } else if (label === 'legitimate path that fails to load') {
      check(`${where}: the fallback still links to the page`, r.links.length === 1 && /gate-missing\.html$/.test(r.links[0]) && r.text.includes('Open in a new tab'), JSON.stringify(r.links));
    }
    await ctx.close();
  }
}

async function palette(browser) {
  for (const key of ['click', 'Enter']) {
    for (const [label, value, hostile] of [['legitimate path', null, false], ['another origin', EVIL, true], ['javascript: URL', JS, true]]) {
      const { ctx, page, seen } = await newPage(browser, () => {
        window.__csp = [];
        document.addEventListener('securitypolicyviolation', (e) => window.__csp.push(e.violatedDirective));
      });
      await page.goto(`${BASE}/`, { waitUntil: 'networkidle' });
      await page.keyboard.press('Control+k');
      await page.keyboard.type('robin');
      await page.waitForSelector('#bnb-cmdk-results li[data-href]', { timeout: 8000 }).catch(() => {});
      // Let the search settle: a later swap would replace the row edited below.
      await page.waitForLoadState('networkidle').catch(() => {});
      await page.waitForTimeout(600);
      const lis = await page.locator('#bnb-cmdk-results li[data-href]').count();
      check(`palette (${key}), ${label}: the palette has results`, lis > 0, `${lis}`);
      if (!lis) { await ctx.close(); continue; }
      const real = await page.locator('#bnb-cmdk-results li[data-href]').first().getAttribute('data-href');
      if (value) await page.locator('#bnb-cmdk-results li[data-href]').first().evaluate((li, v) => li.setAttribute('data-href', v), value);
      const startUrl = page.url();
      if (key === 'click') {
        await page.locator('#bnb-cmdk-results li[data-href]').first().click();
      } else {
        await page.keyboard.press('ArrowDown');
        await page.keyboard.press('Enter');
      }
      await page.waitForTimeout(hostile ? 700 : 100);
      if (!hostile) {
        await page.waitForURL((u) => u.href !== startUrl, { timeout: 8000 }).catch(() => {});
        check(`palette (${key}), ${label}: follows the result`, page.url() !== startUrl && sameOrigin(page.url(), startUrl), `${startUrl} -> ${page.url()} (wanted ${real})`);
      } else {
        const csp = await page.evaluate(() => window.__csp || []).catch(() => []);
        const pwned = await page.evaluate(() => window.__pwned === 1).catch(() => false);
        check(`palette (${key}), ${label}: not followed`,
          page.url() === startUrl && seen.evil.length === 0 && !pwned && csp.length === 0,
          `url=${page.url()} evil=${seen.evil.length} pwned=${pwned} csp=${csp}`);
      }
      await ctx.close();
    }
  }
}

async function listenerId(browser) {
  for (const secureContextUuid of [false, true]) {
    const label = secureContextUuid ? 'randomUUID present' : 'randomUUID absent (plain http)';
    const { ctx, page } = await newPage(browser, (hasUuid) => {
      if (!hasUuid) Object.defineProperty(Crypto.prototype, 'randomUUID', { value: undefined, configurable: true });
      HTMLMediaElement.prototype.canPlayType = () => 'maybe';
      window.__randomFrom = [];
      const real = Math.random;
      Math.random = function () {
        // The page's inline scripts are attributed to the document URL.
        if (/\/recordings\?view=live/.test(new Error().stack || '')) window.__randomFrom.push('recordings');
        return real.call(Math);
      };
    }, secureContextUuid);
    const bodies = [];
    await ctx.route('**/api/v2/streams/hls/**', (r) => {
      if (r.request().method() === 'POST') bodies.push(r.request().postData() || '');
      return r.fulfill({ status: 503, contentType: 'application/json', body: '{}' });
    });
    await page.goto(`${BASE}/recordings?view=live`, { waitUntil: 'networkidle' });
    const btn = page.locator('#rc-listen-btn');
    check(`listener id (${label}): the listen button exists`, (await btn.count()) === 1);
    await btn.click().catch(() => {});
    await page.waitForTimeout(800);
    const ids = bodies.map((b) => { try { return JSON.parse(b).session_id; } catch { return undefined; } });
    const want = secureContextUuid ? /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/ : /^[0-9a-f]{32}$/;
    check(`listener id (${label}): the start request carries a well-formed id`, ids.length > 0 && ids.every((i) => typeof i === 'string' && want.test(i)), JSON.stringify(ids));
    const from = await page.evaluate(() => window.__randomFrom);
    check(`listener id (${label}): Math.random is not used by the page`, from.length === 0, JSON.stringify(from));
    await ctx.close();
  }
}

async function main() {
  const browser = await chromium.launch({
    headless: true,
    ...(CHROMIUM_PATH ? { executablePath: CHROMIUM_PATH } : {}),
  });
  try {
    await nowPlayingAndBulk(browser);
    await helpDrawer(browser);
    await palette(browser);
    await listenerId(browser);
  } finally {
    await browser.close();
  }
  for (const p of passes) console.log(`  ok   ${p}`);
  if (failures.length) {
    console.log(`\x1b[31mFAIL\x1b[0m ${failures.length} of ${failures.length + passes.length} checks:`);
    for (const f of failures) console.log(`  FAIL ${f}`);
    process.exit(1);
  }
  // A gate that graded few cases is not a pass.
  if (passes.length < 40) { console.log(`\x1b[31mFAIL\x1b[0m only ${passes.length} checks ran`); process.exit(1); }
  console.log(`\x1b[32mPASS\x1b[0m ${passes.length} checks`);
}

main().catch((e) => { console.error(e); process.exit(1); });
