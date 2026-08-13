// Behavioural gate: drive real controls in a real browser and assert what they
// do — not how they look.
//
// This exists because of a bug the rest of the suite could not have caught.
// 0.13.0 shipped a Listen -> Live button that cancelled its own stream: the
// server streamed correct MP3 throughout, the page rendered perfectly, axe was
// clean and every screenshot looked right. The defect lived entirely in what
// happened on the *second* click, and nothing anywhere drove a control twice.
//
// So the checks here are all of one shape: perform the interaction a real,
// slightly impatient operator performs, and assert the control does not undo
// its own slow work. Media playback and network requests are stubbed or held
// open deliberately, because the bug window is precisely the interval before
// they complete — a test that waits for them to succeed cannot see it.
//
// Run against the fixture server:
//   cargo run -p birdnet-web --example screenshot_server --features analytics &
//   node interactions.mjs
//
// Env:
//   BASE          base url (default http://127.0.0.1:8502)
//   HEADED        set to 1 to watch it run
//   CHROMIUM_PATH explicit browser binary, for sandboxes that ship their own
//                 Chromium rather than the build this playwright pinned
import { chromium } from 'playwright';

const BASE = process.env.BASE || 'http://127.0.0.1:8502';
const CHROMIUM_PATH = process.env.CHROMIUM_PATH || '';
const failures = [];
const passes = [];

function check(name, ok, detail) {
  if (ok) {
    passes.push(name);
    console.log(`  \x1b[32mPASS\x1b[0m ${name}`);
  } else {
    failures.push({ name, detail });
    console.log(`  \x1b[31mFAIL\x1b[0m ${name}\n       ${detail}`);
  }
}

/** Replace media playback with a promise the test controls.
 *
 * The real bug lives between the click and the moment `play()` resolves, so the
 * test has to own that interval rather than wait it out. Also records every
 * pause() — a second click reaching pause() *is* the regression.
 */
const STUB_MEDIA = () => {
  window.__media = { plays: 0, pauses: 0, resolve: null };
  HTMLMediaElement.prototype.play = function () {
    window.__media.plays += 1;
    return new Promise((resolve) => {
      window.__media.resolve = () => {
        Object.defineProperty(this, 'paused', { value: false, configurable: true });
        resolve();
      };
      // Mirror the browser: paused flips false synchronously, long before the
      // promise settles. This is the exact asymmetry the bug turned on.
      Object.defineProperty(this, 'paused', { value: false, configurable: true });
    });
  };
  HTMLMediaElement.prototype.pause = function () {
    window.__media.pauses += 1;
    Object.defineProperty(this, 'paused', { value: true, configurable: true });
  };
};

async function liveAudioButton(page) {
  await page.addInitScript(STUB_MEDIA);
  await page.goto(`${BASE}/recordings?view=live`, { waitUntil: 'domcontentloaded' });

  const btn = page.locator('#rc-listen-btn');
  const label = page.locator('#rc-listen-label');
  if (!(await btn.count())) {
    check('live: listen button exists', false, 'no #rc-listen-btn on /recordings?view=live');
    return;
  }

  await btn.click();
  const connectingLabel = (await label.textContent())?.trim();
  check(
    'live: shows progress while connecting',
    connectingLabel === 'Connecting…',
    `expected "Connecting…" while play() is pending, got "${connectingLabel}". ` +
      'Without visible progress an operator clicks again, which is how this broke.',
  );

  // The regression: a second click during the connect window must not stop the
  // stream that is still starting.
  await btn.click();
  const media = await page.evaluate(() => window.__media);
  check(
    'live: a second click does not cancel the connecting stream',
    media.pauses === 0,
    `pause() was called ${media.pauses}x during connect — the second click killed ` +
      'the stream the operator was waiting for (the 0.13.0 bug).',
  );
  check(
    'live: a second click does not open a duplicate stream',
    media.plays === 1,
    `play() was called ${media.plays}x; a duplicate /stream burns one of the ` +
      'station\'s few concurrent stream slots.',
  );

  // And once playback genuinely starts, the button must say so.
  await page.evaluate(() => window.__media.resolve());
  await page.waitForFunction(
    () => document.getElementById('rc-listen-label')?.textContent.trim() === 'Stop',
    null,
    { timeout: 2000 },
  ).catch(() => {});
  const playingLabel = (await label.textContent())?.trim();
  check(
    'live: reports playing once play() resolves',
    playingLabel === 'Stop',
    `expected "Stop" after playback began, got "${playingLabel}"`,
  );
}

async function clipPlayer(page) {
  await page.addInitScript(STUB_MEDIA);
  await page.goto(`${BASE}/recordings`, { waitUntil: 'domcontentloaded' });

  const play = page.locator('[data-play-src]').first();
  if (!(await play.count())) {
    check('clips: a playable clip exists', false, 'no [data-play-src] control on /recordings');
    return;
  }
  await play.click();
  await play.click();
  const media = await page.evaluate(() => window.__media);
  check(
    'clips: a second click does not stop a clip that is still starting',
    media.pauses === 0,
    `pause() was called ${media.pauses}x while the clip was still loading`,
  );
}

async function bulkActions(page) {
  const posts = [];
  await page.route('**/pages/recordings-lock', async (route) => {
    posts.push(route.request().url());
    // Hold the batch open: the second click has to land while the first is
    // still in flight, which is the only moment the guard matters.
    await new Promise((r) => setTimeout(r, 400));
    await route.fulfill({ status: 200, body: '' });
  });
  await page.goto(`${BASE}/recordings`, { waitUntil: 'domcontentloaded' });

  const selMode = page.locator('#rc-selmode');
  if (!(await selMode.count())) {
    check('bulk: select mode exists', false, 'no #rc-selmode on /recordings');
    return;
  }
  await selMode.click();
  const rows = page.locator('.rc-row');
  const n = Math.min(await rows.count(), 3);
  if (n === 0) {
    check('bulk: selectable rows exist', false, 'no .rc-row to select');
    return;
  }
  for (let i = 0; i < n; i += 1) await rows.nth(i).click();

  const lock = page.locator('#rc-bulk-lock');
  if (!(await lock.count())) {
    check('bulk: lock button exists', false, 'no #rc-bulk-lock');
    return;
  }
  await lock.click();
  await lock.click();          // impatient second click, batch still in flight
  await page.waitForTimeout(900);

  check(
    'bulk: a second click does not re-send the whole batch',
    posts.length === n,
    `expected ${n} POSTs (one per selected clip), saw ${posts.length} — the ` +
      'batch was sent twice.',
  );
}

async function destructiveControlDisables(page) {
  let released;
  const held = new Promise((r) => { released = r; });
  await page.route('**/admin/system/clear-detections', async (route) => {
    await held;
    await route.fulfill({ status: 200, body: '<p>done</p>' });
  });
  const resp = await page.goto(`${BASE}/station/data`, { waitUntil: 'domcontentloaded' });
  if (!resp || resp.status() >= 400) {
    check('destructive: data page reachable', false, `GET /station/data -> ${resp?.status()}`);
    released();
    return;
  }
  const btn = page.locator('[hx-post="/admin/system/clear-detections"]').first();
  if (!(await btn.count())) {
    check('destructive: clear-detections control exists', false, 'control not found on /station/data');
    released();
    return;
  }
  const guarded = await btn.getAttribute('hx-disabled-elt');
  check(
    'destructive: clear-detections disables itself in flight',
    guarded !== null,
    'no hx-disabled-elt: htmx 2.x does not dedupe in-flight requests by ' +
      'default, so this destructive control can be fired twice.',
  );
  released();
}

const page404 = [];

async function main() {
  const browser = await chromium.launch({
    headless: process.env.HEADED !== '1',
    ...(CHROMIUM_PATH ? { executablePath: CHROMIUM_PATH } : {}),
  });
  const ctx = await browser.newContext();
  ctx.on('response', (r) => {
    if (r.status() >= 500) page404.push(`${r.status()} ${r.url()}`);
  });

  for (const [name, fn] of [
    ['live audio button', liveAudioButton],
    ['clip player', clipPlayer],
    ['bulk actions', bulkActions],
    ['destructive controls', destructiveControlDisables],
  ]) {
    console.log(`\n${name}`);
    const page = await ctx.newPage();
    try {
      await fn(page);
    } catch (e) {
      check(`${name}: harness ran`, false, String(e && e.message ? e.message : e));
    }
    await page.close();
  }

  await browser.close();

  console.log(`\n${passes.length} passed, ${failures.length} failed`);
  if (page404.length) console.log(`server 5xx during run:\n  ${page404.join('\n  ')}`);
  if (failures.length) {
    console.log('\nfailures:');
    for (const f of failures) console.log(`  - ${f.name}: ${f.detail}`);
    process.exit(1);
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
