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

/** UX-1: the wizard's preference cards must work without a mouse.
 *
 * They were bare <div>s with a click handler: no role, no tabindex, so Tab
 * skipped them and a keyboard-only operator could set neither the threshold
 * nor the alert mode during first-run setup. axe passed the route clean —
 * it grades what is there, not what is missing — so the check is behavioural:
 * walk to the threshold step with the page's own button, then, using nothing
 * but the keyboard, reach a card and change the selection.
 */
async function wizardCardsByKeyboard(page) {
  await page.goto(`${BASE}/onboarding`, { waitUntil: 'domcontentloaded' });
  if (!(await page.locator('#ob-next').count())) {
    check('wizard: page exists', false, 'no #ob-next on /onboarding');
    return;
  }
  // The threshold cards are step 4; the password step accepts two blank
  // fields as "not now".
  for (let i = 0; i < 3; i++) await page.click('#ob-next');
  const active = await page.$eval('.ob-step.active', (s) => s.dataset.step);
  check('wizard: reached the threshold step', active === '4', `active step is ${active}`);
  if (active !== '4') return;

  const before = await page.inputValue('#ob-conf');
  // Keyboard only from here. Tab forward until focus is inside a threshold
  // card; the loop wraps through the page chrome, so it is generous.
  let landed = false;
  for (let i = 0; i < 80 && !landed; i++) {
    await page.keyboard.press('Tab');
    landed = await page.evaluate(() => {
      const el = document.activeElement;
      return Boolean(el && el.closest && el.closest('[data-radio="conf"]'));
    });
  }
  check('wizard: Tab reaches a threshold card', landed, 'after 80 Tabs focus never entered a [data-radio="conf"] card');
  if (!landed) return;

  await page.keyboard.press('ArrowDown');
  const after = await page.inputValue('#ob-conf');
  check('wizard: ArrowDown changes the threshold', after !== '' && after !== before, `#ob-conf was "${before}", is "${after}"`);
  const highlighted = await page.$eval('[data-radio="conf"].sel', (c) => c.dataset.value).catch(() => null);
  check('wizard: the highlighted card is the chosen one', highlighted === after, `highlighted ${highlighted}, input ${after}`);

  // Counterpart: the mouse path still works, and lands the same way.
  await page.click('[data-radio="conf"][data-value="0.9"]');
  const clicked = await page.inputValue('#ob-conf');
  const clickedCard = await page.$eval('[data-radio="conf"].sel', (c) => c.dataset.value).catch(() => null);
  check('wizard: clicking a card still selects it', clicked === '0.9' && clickedCard === '0.9', `input ${clicked}, highlighted ${clickedCard}`);
}

/** A toast the server sends out-of-band can be dismissed, and goes by itself.
 *
 * htmx fires `htmx:oobAfterSwap` on the main swap's target, and the region
 * listened for its own id on `e.target`, so no server toast was ever bound:
 * the × did nothing and "Settings saved" stayed on screen for good.
 */
async function serverToasts(page) {
  await page.goto(`${BASE}/admin/settings`, { waitUntil: 'networkidle' });
  await page.click('button.btn-primary[type=submit]');
  const toast = await page.waitForSelector('#bnb-toasts .bnb-toast', { timeout: 10000 }).catch(() => null);
  check('toasts: saving settings shows a toast', !!toast, 'no toast appeared');
  if (!toast) return;
  const bound = await toast.evaluate((t) => t.dataset.bound === '1');
  check('toasts: the server toast is wired up', bound, 'data-bound was never set');
  await page.click('#bnb-toasts .bnb-toast [data-toast-close]');
  const gone = await page
    .waitForFunction(() => document.querySelectorAll('#bnb-toasts .bnb-toast').length === 0, null, { timeout: 2000 })
    .then(() => true, () => false);
  check('toasts: the × dismisses it', gone, 'the toast is still on screen after its × was clicked');

  await page.click('button.btn-primary[type=submit]');
  await page.waitForSelector('#bnb-toasts .bnb-toast', { timeout: 10000 });
  const timedOut = await page
    .waitForFunction(() => document.querySelectorAll('#bnb-toasts .bnb-toast').length === 0, null, { timeout: 9000 })
    .then(() => true, () => false);
  check('toasts: a success toast goes by itself', timedOut, 'still on screen 9 s later');
}

/** The polar activity clock draws the station's data, not 24 zeros.
 *
 * Its script read `json.data` / `hour` / `avg_detections`; the endpoint answers
 * `heatmap` / `hour_of_day` / `avg_detections_per_day`, so every station got a
 * flat clock labelled "90d avg" and nothing said anything was wrong.
 */
async function polarClock(page) {
  await page.goto(`${BASE}/patterns?tab=trends`, { waitUntil: 'networkidle' });
  const found = await page.$('#polar-clock');
  check('clock: the trends tab has the polar clock', !!found, 'no #polar-clock');
  if (!found) return;
  await page.evaluate(() => {
    let d = document.getElementById('polar-clock').closest('details');
    while (d) { d.open = true; d = d.parentElement && d.parentElement.closest('details'); }
  });
  const radii = await page
    .waitForFunction(() => {
      const paths = [...document.querySelectorAll('#polar-clock path')];
      return paths.length === 24 ? new Set(paths.map((p) => p.getAttribute('d').split(' ')[7])).size : 0;
    }, null, { timeout: 8000 })
    .then((h) => h.jsonValue(), () => 0);
  check('clock: the wedges follow the data', radii > 1, `${radii} distinct wedge radii — a flat clock`);
}

/** Searching leaves the page's own URL in the address bar, and it reloads.
 *
 * The form pushed the fragment's URL (`/pages/search-results?…`), so a reload,
 * a bookmark or Back showed a bare, unstyled list with no page around it.
 */
async function searchAddressBar(page) {
  await page.goto(`${BASE}/search`, { waitUntil: 'networkidle' });
  await page.fill('#sr-form input[name=q]', 'robin');
  await page.press('#sr-form input[name=q]', 'Enter');
  await page.waitForFunction(() => location.search.includes('q=robin'), null, { timeout: 8000 }).catch(() => {});
  const path = await page.evaluate(() => location.pathname + location.search);
  check('search: the address bar carries the page, not the fragment', path.startsWith('/search?') && path.includes('q=robin'), path);
  await page.reload({ waitUntil: 'networkidle' });
  const whole = await page.evaluate(() => !!document.querySelector('#sr-form') && document.title.length > 0);
  check('search: reloading that URL gives the whole page back', whole, `title="${await page.title()}"`);
}

/** Bulk "Apply" asks first, then acts and says so.
 *
 * It targeted `#toast-region`, an id no page has, so htmx refused to send it:
 * confirm, reject, lock, unlock and delete in bulk all did nothing. And its
 * confirmation was never wired, so once it did send, Delete would not ask.
 */
async function searchBulkApply(page) {
  await page.goto(`${BASE}/search?q=robin`, { waitUntil: 'networkidle' });
  await page.waitForSelector('form.sr-bulk input[type=checkbox][name]', { timeout: 8000 });
  await page.check('form.sr-bulk input[type=checkbox][name]');
  await page.selectOption('#sr-bulk-action', 'lock');
  const sent = [];
  page.on('request', (r) => { if (r.url().includes('/pages/search-bulk')) sent.push(r.method()); });
  await page.click('form.sr-bulk button[type=submit]');
  const asked = await page.waitForSelector('#bnb-confirm[open]', { timeout: 3000 }).then(() => true, () => false);
  check('bulk: Apply asks before acting', asked, 'no confirmation dialog opened');
  check('bulk: nothing is sent before the answer', sent.length === 0, `${sent.length} request(s) already sent`);
  if (!asked) return;
  await page.click('#bnb-confirm [data-confirm-ok]');
  const toast = await page.waitForSelector('#bnb-toasts .bnb-toast', { timeout: 8000 }).then(() => true, () => false);
  check('bulk: confirming sends it', sent.length === 1, `${sent.length} request(s)`);
  check('bulk: the outcome is shown', toast, 'no toast after the bulk action');
}

/** Every ▶ plays — including the Live view's feed, which had no player. */
async function livePlayButtons(page) {
  await page.addInitScript(() => {
    window.__plays = 0;
    HTMLMediaElement.prototype.play = function () { window.__plays += 1; return Promise.resolve(); };
  });
  await page.goto(`${BASE}/recordings?view=live`, { waitUntil: 'networkidle' });
  const btn = await page.waitForSelector('[data-play-src]', { timeout: 8000 }).catch(() => null);
  check('live feed: rows carry a play button', !!btn, 'no [data-play-src] on the Live view');
  if (!btn) return;
  await btn.click();
  await page.waitForTimeout(300);
  const plays = await page.evaluate(() => window.__plays);
  check('live feed: ▶ plays the clip', plays === 1, `play() called ${plays} time(s)`);
}

/** Enter in Today's search filters in place instead of reloading the page. */
async function todaySearchEnter(page) {
  await page.goto(`${BASE}/`, { waitUntil: 'networkidle' });
  let navigations = 0;
  page.on('framenavigated', (f) => { if (f === page.mainFrame()) navigations += 1; });
  await page.fill('#today-search', 'Cardinal');
  await page.press('#today-search', 'Enter');
  await page.waitForTimeout(1500);
  check('today: Enter does not reload the page', navigations === 0, `${navigations} navigation(s), now at ${page.url()}`);
}

/** "Load more" adds to the list rather than replacing it. */
async function loadMoreAppends(page) {
  await page.goto(`${BASE}/`, { waitUntil: 'networkidle' });
  // Open the day the way a reader does, then load it five at a time so the
  // demo station's day is long enough to need a second page.
  await page.click('button:has-text("Show the full day")');
  await page.waitForTimeout(500);
  await page.evaluate(() => {
    window.htmx.ajax('GET', '/pages/today-list?limit=5', { target: '#today-full', swap: 'innerHTML' });
  });
  await page.waitForSelector('#today-full .tdl-more-btn', { timeout: 8000 });
  const before = await page.$$eval('#today-full .tdl-card', (e) => e.length);
  await page.click('#today-full .tdl-more-btn');
  await page.waitForFunction((n) => document.querySelectorAll('#today-full .tdl-card').length !== n, before, { timeout: 8000 }).catch(() => {});
  const after = await page.$$eval('#today-full .tdl-card', (e) => e.length);
  check('today: Load more adds to the list', before === 5 && after === 10, `${before} rows, then ${after}`);
}

/** A stream this browser cannot reach says "no signal", and keeps trying.
 *
 * The idle poll overwrote "no signal" with "idle" within a second — so a
 * blocked socket read as a quiet yard — and the socket was opened once and
 * never again, leaving the card dead after any blip until a reload.
 */
async function liveSignal(page) {
  let attempts = 0;
  await page.routeWebSocket(/\/api\/v2\/ws\/spectrogram/, (ws) => { attempts += 1; ws.close(); });
  await page.goto(`${BASE}/`, { waitUntil: 'domcontentloaded' });
  await page.waitForTimeout(3500);
  const pill = await page.$eval('.db-live-pill', (p) => p.textContent.trim()).catch(() => '(no pill)');
  check('live signal: an unreachable stream reads "no signal", not "idle"', /no signal/.test(pill), `pill says "${pill}"`);
  check('live signal: it tries again after losing the stream', attempts >= 2, `${attempts} connection attempt(s) in 3.5 s`);
}

// The rare-sightings nudge is said once, not every time it is re-fetched.
async function nudgeAnnouncesOnce(page) {
  await page.goto(`${BASE}/`, { waitUntil: 'networkidle' });
  const first = await page.$eval('#td-nudge-status', (e) => e.textContent.trim()).catch(() => '');
  check('nudge: the waiting sightings are announced', /waiting for review/.test(first), `status says "${first}"`);
  const writes = await page.evaluate(async () => {
    const out = document.getElementById('td-nudge-status');
    let n = 0;
    new MutationObserver(() => { n += 1; }).observe(out, { childList: true, characterData: true, subtree: true });
    for (let i = 0; i < 2; i += 1) {
      await new Promise((resolve) => {
        document.body.addEventListener('htmx:afterSettle', resolve, { once: true });
        window.htmx.ajax('GET', '/pages/today-nudge', { target: '#today-nudge', swap: 'innerHTML' });
      });
    }
    return n;
  });
  check('nudge: an unchanged nudge is not re-announced', writes === 0, `${writes} status write(s) over 2 re-fetches`);
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
    ['wizard cards by keyboard', wizardCardsByKeyboard],
    ['server toasts', serverToasts],
    ['activity clock', polarClock],
    ['search address bar', searchAddressBar],
    ['search bulk apply', searchBulkApply],
    ['live feed play', livePlayButtons],
    ['today search enter', todaySearchEnter],
    ['load more', loadMoreAppends],
    ['live signal', liveSignal],
    ['nudge announces once', nudgeAnnouncesOnce],
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
