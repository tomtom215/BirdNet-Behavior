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

/** M14: the live feed comes back after the browser restores the page.
 *
 * The socket was stopped on `beforeunload`, which set a flag nothing ever
 * cleared. A page restored from the back/forward cache (`pageshow` with
 * `persisted`) therefore had a dead feed until reloaded, and a `beforeunload`
 * listener is itself one of the things that keeps a page out of that cache.
 * Playwright's Chromium runs with the cache off, so the events the browser
 * fires around it are replayed in order.
 */
async function liveFeedSurvivesBackForward(page) {
  let sockets = 0;
  page.on('websocket', (ws) => {
    if (ws.url().includes('/ws/detections')) sockets += 1;
  });
  await page.goto(`${BASE}/`, { waitUntil: 'networkidle' });
  await page.waitForTimeout(500);
  check('bfcache: the feed connects on load', sockets >= 1, `${sockets} socket(s)`);
  const before = sockets;
  await page.evaluate(() => {
    window.dispatchEvent(new Event('beforeunload'));
    window.dispatchEvent(new PageTransitionEvent('pagehide', { persisted: true }));
    window.dispatchEvent(new PageTransitionEvent('pageshow', { persisted: true }));
  });
  await page.waitForTimeout(1500);
  check(
    'bfcache: a restored page reconnects its live feed',
    sockets > before,
    `no new socket after pageshow (persisted); ${sockets} total`,
  );
}

/** M5: a live detection refreshes Today's feed at once.
 *
 * The listener called `htmx.trigger(feed, 'load')`. htmx fires its `load`
 * trigger itself, once, and does not listen for a `load` event, so the call
 * fetched nothing and a new bird waited for the 15-second poll.
 */
async function liveDetectionRefreshesFeed(page) {
  let fetches = 0;
  page.on('request', (r) => {
    if (r.url().includes('/pages/detections')) fetches += 1;
  });
  await page.goto(`${BASE}/`, { waitUntil: 'networkidle' });
  const before = fetches;
  await page.evaluate(() => {
    document.dispatchEvent(
      new CustomEvent('birdnet:detection', { detail: { event: 'detection' } }),
    );
  });
  await page.waitForTimeout(1500);
  check(
    'feed: a live detection refreshes the feed',
    fetches > before,
    `no /pages/detections request within 1.5 s of the event (${fetches} total)`,
  );
}

/** M5: the command palette loads when it is opened, and fresh each time.
 *
 * Its input carried `hx-trigger="…, load"`, so every page view fetched
 * /pages/cmdk (a database read) for a palette nobody opened; and reopening
 * it called `htmx.trigger(input, 'load')`, which fetches nothing — the input
 * was cleared and the last query's results stayed under it.
 */
async function paletteLoadsOnOpen(page) {
  const queries = [];
  page.on('request', (r) => {
    if (r.url().includes('/pages/cmdk')) queries.push(new URL(r.url()).searchParams.get('q'));
  });
  await page.goto(`${BASE}/`, { waitUntil: 'networkidle' });
  check('palette: a page view does not query it', queries.length === 0, `${queries.length} request(s)`);

  const beforeOpen = queries.length;
  await page.keyboard.press('Control+k');
  await page.waitForTimeout(600);
  check(
    'palette: opening it loads the default list',
    queries.length === beforeOpen + 1,
    `${queries.length - beforeOpen} request(s) caused by opening it`,
  );

  await page.keyboard.type('zzzz');
  await page.waitForTimeout(800);
  await page.keyboard.press('Escape');
  const before = queries.length;
  await page.keyboard.press('Control+k');
  await page.waitForTimeout(800);
  const reloaded = queries.length > before && (queries[queries.length - 1] || '') === '';
  check(
    'palette: reopening it shows the default list, not the last query',
    reloaded,
    `requests after reopen: ${JSON.stringify(queries.slice(before))}`,
  );
}

/** M7: the co-occurrence range the reader picked is the range every part of
 * the tab shows.
 *
 * The companion lookup sent `days-val`, which the server never reads, so it
 * always answered for 30 days; and the two collapsed tables were fetched on
 * first opening with a hard-coded `?days=30`, overwriting whatever range had
 * been chosen before they were opened.
 */
async function correlationRangeHolds(page) {
  const seen = [];
  page.on('request', (r) => {
    const u = new URL(r.url());
    if (/cooccurrence-matrix|correlation-pairs|companion-species/.test(u.pathname)) {
      seen.push({ path: u.pathname, days: u.searchParams.get('days') });
    }
  });
  await page.goto(`${BASE}/patterns?tab=together`, { waitUntil: 'networkidle' });
  await page.click('#range-controls [data-days="90"]');
  await page.waitForTimeout(600);
  const opened = seen.length;
  await page.click('summary:has-text("co-occurrence matrix")');
  await page.waitForTimeout(800);
  const afterOpen = seen.slice(opened).filter((x) => x.path.endsWith('cooccurrence-matrix'));
  check(
    'range: opening the matrix keeps the chosen 90 days',
    afterOpen.every((x) => x.days === '90'),
    `requests on opening: ${JSON.stringify(afterOpen)}`,
  );
  // Typed, not filled: the input listens for `keyup`, which fill() never sends.
  await page.click('#species-input');
  await page.keyboard.type('Robin');
  await page.waitForTimeout(900);
  const companion = seen.filter((x) => x.path.endsWith('companion-species')).pop();
  check(
    'range: the companion lookup asks for the chosen 90 days',
    companion && companion.days === '90',
    `last companion request: ${JSON.stringify(companion)}`,
  );
}

/** M11: the help drawer shows the page it fetched, with working links.
 *
 * `body.querySelector(hash)` throws for an id that starts with a digit —
 * mdBook's `3-sensitivity` — and the throw landed in the catch, which
 * replaced the page it had just loaded with "Couldn't load help". And the
 * docs' links are relative, so inside the host page they resolved against
 * `/admin/...` and led nowhere.
 *
 * The page is served from a fixture, not the mdBook render: the root crate's
 * `build.rs` produces that render and CI builds only `birdnet-web`, so there
 * `/help/*` is a 404 and this gate was grading the drawer's error path. The
 * fixture keeps the mdBook shape the drawer relies on (`<main>`, an id that
 * starts with a digit, relative links).
 */
async function helpDrawerDeepLink(page) {
  await page.route('**/help/guides/tuning', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'text/html; charset=utf-8',
      body:
        '<!DOCTYPE html><html><head><title>Tuning</title></head><body>' +
        '<nav class="sidebar"><a href="../index.html">Home</a></nav>' +
        '<main><h1 id="tuning">Tuning</h1>' +
        '<p>See <a href="../reference/configuration.html">configuration</a> and ' +
        '<a href="first-run.html#1-location">first run</a>.</p>' +
        '<h3 id="3-sensitivity">3. Sensitivity</h3><p>Sensitivity text.</p>' +
        '</main></body></html>',
    }),
  );
  await page.goto(`${BASE}/`, { waitUntil: 'networkidle' });
  await page.evaluate(() => {
    const b = document.createElement('button');
    b.id = 'zz-help';
    b.setAttribute('data-help-drawer', '/help/guides/tuning#3-sensitivity');
    b.textContent = 'help';
    document.body.appendChild(b);
  });
  await page.click('#zz-help');
  await page.waitForFunction(
    () => !/Loading/.test(document.getElementById('bnb-help-drawer-title').textContent),
    null,
    { timeout: 10000 },
  );
  const title = await page.textContent('#bnb-help-drawer-title');
  check('help: a digit-leading anchor still shows the page', !/Couldn/.test(title), `title: ${title}`);
  check(
    'help: the drawer shows the fetched page',
    await page.$('#bnb-help-drawer-body #\\33 -sensitivity') !== null,
    `body: ${(await page.textContent('#bnb-help-drawer-body')).slice(0, 120)}`,
  );
  const links = await page.evaluate(() =>
    Array.from(document.querySelectorAll('#bnb-help-drawer-body a[href]'))
      .map((a) => a.href)
      .filter((h) => /configuration\.html|first-run\.html/.test(h)),
  );
  // Without this the check below passes on an empty list — which is what it
  // did while the page failed to load.
  check('help: the fixture\'s relative links reached the drawer', links.length === 2, JSON.stringify(links));
  const stray = links.filter((h) => !h.startsWith(`${BASE}/help/`));
  check('help: relative links resolve inside /help', stray.length === 0, JSON.stringify(stray));
  await page.unroute('**/help/guides/tuning');
}

/** M13: a hidden tab does not poll; it catches up when shown.
 *
 * Every `hx-trigger="every …"` kept firing in a background tab — a Today page
 * left open all day is a dozen database reads a minute that nobody sees. htmx
 * filters (`[!document.hidden]`) need `eval`, which the CSP refuses.
 */
async function hiddenTabsDoNotPoll(page) {
  let polls = 0;
  page.on('request', (r) => {
    if (r.url().includes('/pages/today-count?bare=1&zz=poll')) polls += 1;
  });
  await page.goto(`${BASE}/`, { waitUntil: 'networkidle' });
  await page.evaluate(() => {
    window.__hidden = false;
    Object.defineProperty(document, 'hidden', { configurable: true, get: () => window.__hidden });
    Object.defineProperty(document, 'visibilityState', {
      configurable: true,
      get: () => (window.__hidden ? 'hidden' : 'visible'),
    });
    const d = document.createElement('div');
    d.id = 'zz-poll';
    d.setAttribute('hx-get', '/pages/today-count?bare=1&zz=poll');
    d.setAttribute('hx-trigger', 'every 1s');
    document.body.appendChild(d);
    window.htmx.process(d);
    window.__hidden = true;
    document.dispatchEvent(new Event('visibilitychange'));
  });
  await page.waitForTimeout(3500);
  check('poll: a hidden tab does not poll', polls === 0, `${polls} poll(s) while hidden`);
  const atShow = polls;
  await page.evaluate(() => {
    window.__hidden = false;
    document.dispatchEvent(new Event('visibilitychange'));
  });
  await page.waitForTimeout(600);
  check('poll: showing the tab refreshes at once', polls > atShow, `${polls - atShow} poll(s) after showing`);
}

// The Display card on /station/settings. A heading above it carried the
// card's own id, so its script bound to the heading and every button was
// inert — the page rendered perfectly and axe was clean. Drive the buttons.
async function displayPrefs(page) {
  await page.goto(`${BASE}/station/settings`, { waitUntil: 'networkidle' });
  const dark = page.locator('#display-prefs [data-prefs-key="theme"] [data-value="dark"]');
  if (!(await dark.count())) {
    check('prefs: theme control exists', false, 'no Dark button in #display-prefs');
    return;
  }
  await dark.click();
  const theme = await page.evaluate(() => document.documentElement.dataset.theme);
  check('prefs: Dark switches the theme', theme === 'dark', `data-theme is "${theme}"`);
  check('prefs: Dark is marked chosen', (await dark.getAttribute('aria-checked')) === 'true', 'aria-checked is not "true"');
  const compact = page.locator('#display-prefs [data-prefs-key="bnb-density"] [data-value="compact"]');
  await compact.click();
  const density = await page.evaluate(() => document.documentElement.style.getPropertyValue('--density'));
  check('prefs: Compact changes the density', density.trim() === '0.78', `--density is "${density}"`);
  await page.click('#display-prefs [data-prefs-reset]');
  const auto = page.locator('#display-prefs [data-prefs-key="theme"] [data-value="auto"]');
  check(
    'prefs: Reset moves the screen-reader state too',
    (await auto.getAttribute('aria-checked')) === 'true' && (await dark.getAttribute('aria-checked')) === 'false',
    `auto aria-checked=${await auto.getAttribute('aria-checked')}, dark=${await dark.getAttribute('aria-checked')}`,
  );
}

// Adding a microphone. The form reset itself through `hx-on::after-request`,
// which the CSP blocks, so it never reset; and a refused add (the same device
// twice) answered 422, whose body htmx discards, so nothing said why.
async function audioSourceAdd(page) {
  await page.addInitScript(() => {
    window.__csp = [];
    document.addEventListener('securitypolicyviolation', (e) => window.__csp.push(e.violatedDirective));
  });
  await page.goto(`${BASE}/admin/audio`, { waitUntil: 'networkidle' });
  if (!(await page.locator('#add-local form').count())) {
    check('audio: add form exists', false, 'no #add-local form on /admin/audio');
    return;
  }
  const dev = `plughw:7,${Date.now() % 100000}`;
  const submit = async () => {
    await page.evaluate(() => { document.getElementById('add-local').open = true; });
    await page.fill('#lt-id', dev);
    const done = page.waitForResponse((r) => r.url().endsWith('/admin/audio/sources') && r.request().method() === 'POST');
    await page.click('#add-local form button[type=submit]');
    await done;
    await page.waitForTimeout(500);
    return page.evaluate(() => ({
      open: document.getElementById('add-local').open,
      value: document.getElementById('lt-id').value,
      toasts: [...document.querySelectorAll('#bnb-toasts .bnb-toast')].map((t) => t.textContent),
      csp: window.__csp.length,
    }));
  };
  const first = await submit();
  check('audio: a saved add closes and clears the form', !first.open && first.value === '', JSON.stringify(first));
  check('audio: no CSP violation', first.csp === 0, `${first.csp} violation(s)`);
  await page.evaluate(() => document.querySelectorAll('#bnb-toasts .bnb-toast').forEach((t) => t.remove()));
  const second = await submit();
  check(
    'audio: a refused add says why',
    second.toasts.some((t) => /already/i.test(t)),
    `toasts: ${JSON.stringify(second.toasts)}`,
  );
  check('audio: a refused add keeps what was typed', second.open && second.value === dev, JSON.stringify(second));
}

// Playing clip A then clip B. `stop()` cleared only A's class, so A kept its
// ⏸ and its "Pause" name: two rows claiming to play, one of them silent.
async function clipGlyphsFollowPlayback(page) {
  await page.addInitScript(STUB_MEDIA);
  await page.goto(`${BASE}/recordings`, { waitUntil: 'domcontentloaded' });
  const rows = page.locator('#rc-clips [data-play-src]');
  if ((await rows.count()) < 2) {
    check('clip glyphs: two playable clips exist', false, `${await rows.count()} on /recordings`);
    return;
  }
  for (const i of [0, 1]) {
    await rows.nth(i).click();
    await page.evaluate(() => window.__media.resolve && window.__media.resolve());
    await page.waitForTimeout(150);
  }
  const state = await page.evaluate(() =>
    [...document.querySelectorAll('#rc-clips [data-play-src]')].slice(0, 2).map((b) => ({
      glyph: b.textContent.trim(),
      label: b.getAttribute('aria-label'),
      name: b.getAttribute('data-clip-name'),
    })),
  );
  check('clip glyphs: only the playing row shows pause', state[0].glyph === '▶' && state[1].glyph === '⏸', JSON.stringify(state));
  check(
    'clip glyphs: each row still names its bird',
    state[0].label === `Play ${state[0].name}` && state[1].label === `Pause ${state[1].name}`,
    JSON.stringify(state),
  );
}

// On a phone the now-playing dock slid up beneath the bottom tab bar, which
// hid its scrub bar and time. Measured, not read from the CSS: a media query
// earlier in the sheet can lose to a later rule (CLAUDE.md).
async function floatDockClearsTheTabBar(page) {
  await page.addInitScript(STUB_MEDIA);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(`${BASE}/recordings`, { waitUntil: 'networkidle' });
  // The dock is built the first time a clip plays.
  const play = page.locator('#rc-clips [data-play-src]').first();
  if (await play.count()) {
    await play.click();
    await page.evaluate(() => window.__media.resolve && window.__media.resolve());
    await page.waitForTimeout(200);
  }
  const geo = await page.evaluate(() => {
    const dock = document.querySelector('.rc-floatdock');
    const bar = document.querySelector('.bnb-tabbar');
    if (!dock || !bar) return null;
    dock.classList.add('show');
    dock.style.transition = 'none';
    const d = dock.getBoundingClientRect();
    const b = bar.getBoundingClientRect();
    return { dockBottom: d.bottom, barTop: b.top, barShown: getComputedStyle(bar).display !== 'none' };
  });
  if (!geo) {
    check('dock: dock and tab bar exist', false, 'no .rc-floatdock or .bnb-tabbar on /recordings');
    return;
  }
  check('dock: the tab bar is shown at phone width', geo.barShown, JSON.stringify(geo));
  check('dock: the dock sits above the tab bar', geo.dockBottom <= geo.barTop + 0.5, JSON.stringify(geo));
}

// With site data blocked, every `localStorage` access throws. The pre-paint
// guard read it unguarded, so it aborted before setting a theme — a dark-OS
// reader got the light page — and the toggle threw before applying, so the
// button did nothing.
async function themeWithStorageBlocked(page) {
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.addInitScript(() => {
    const deny = () => { throw new DOMException('blocked', 'SecurityError'); };
    Storage.prototype.getItem = deny;
    Storage.prototype.setItem = deny;
  });
  await page.goto(`${BASE}/`, { waitUntil: 'domcontentloaded' });
  const first = await page.evaluate(() => document.documentElement.dataset.theme);
  check('storage blocked: the OS theme still applies', first === 'dark', `data-theme is "${first}"`);
  await page.click('#theme-toggle');
  const after = await page.evaluate(() => document.documentElement.dataset.theme);
  check('storage blocked: the theme toggle still works', !!after && after !== first, `was "${first}", is "${after}"`);
}

// The Today source picker navigated to Recordings on `change`, which some
// platforms fire for every arrow key in a closed select (WCAG 3.2.2). It now
// chooses what the card draws and where "Listen live" goes, and stays put.
async function todaySourcePickerStays(page) {
  await page.goto(`${BASE}/`, { waitUntil: 'networkidle' });
  const opts = await page.locator('#td-source option:not([disabled])').evaluateAll((os) => os.map((o) => o.value));
  const pick = opts.find((v) => v);
  if (!pick) {
    check('source picker: a source to pick exists', false, `options: ${JSON.stringify(opts)}`);
    return;
  }
  const before = page.url();
  await page.selectOption('#td-source', pick);
  await page.waitForTimeout(400);
  check('source picker: choosing a source stays on the page', page.url() === before, `navigated to ${page.url()}`);
  const href = await page.getAttribute('.x-listen', 'href');
  check('source picker: Listen live carries the choice', (href || '').includes(`source=${encodeURIComponent(pick)}`), `href ${href}`);
}

// Every client receives every source's spectrogram frames. The Today card
// drew them all, so a two-source station's "live signal" was two inputs
// interleaved. Feed it A, B, A, B and count repaints (paintFrame clears the
// canvas once per frame it draws).
async function todaySignalFollowsOneSource(page) {
  await page.addInitScript(() => {
    window.__sockets = [];
    const Real = window.WebSocket;
    window.WebSocket = function (url) {
      if (!String(url).includes('/ws/spectrogram')) return new Real(url);
      const fake = { readyState: 1, close() {}, send() {} };
      window.__sockets.push(fake);
      setTimeout(() => fake.onopen && fake.onopen({}), 0);
      return fake;
    };
    window.__paints = 0;
    const clear = CanvasRenderingContext2D.prototype.clearRect;
    CanvasRenderingContext2D.prototype.clearRect = function (...a) {
      if (window.__counting && this.canvas && this.canvas.id === 'hero-pulse') window.__paints += 1;
      return clear.apply(this, a);
    };
  });
  await page.goto(`${BASE}/`, { waitUntil: 'networkidle' });
  if (!(await page.locator('#hero-pulse').count())) {
    check('signal: the live signal card is on the page', false, 'no #hero-pulse on /');
    return;
  }
  const paints = await page.evaluate(() => {
    const ws = window.__sockets[window.__sockets.length - 1];
    if (!ws || !ws.onmessage) return -1;
    window.__counting = true;
    for (const source of ['src_a', 'src_b', 'src_a', 'src_b']) {
      ws.onmessage({ data: JSON.stringify({ event: 'spectrogram', source, n_mels: 1, n_frames: 1, data: [1] }) });
    }
    window.__counting = false;
    return window.__paints;
  });
  check('signal: the card draws one source, not every source interleaved', paints === 2, `${paints} repaint(s) for A,B,A,B`);
}

const page404 = [];

// A copy button read its own label when clicked, so a second click inside
// the "Copied!" window took "Copied!" as the label and restored to it for
// good. The clipboard is stubbed: this is about the label, not the platform.
async function copyButtonReturnsToItsLabel(page) {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, 'clipboard', { value: { writeText: () => Promise.resolve() } });
  });
  const r = await page.request.get(`${BASE}/api/v2/detections?limit=1`);
  const d = (await r.json()).detections[0];
  await page.goto(`${BASE}/detections/detail?date=${d.date}&time=${d.time}&name=${encodeURIComponent(d.com_name)}`, { waitUntil: 'domcontentloaded' });
  const btn = page.locator('[data-copy-url]').first();
  if (!(await btn.count())) {
    check('copy button: the detection page has one', false, 'no [data-copy-url] on the page');
    return;
  }
  const label = (await btn.textContent()).trim();
  await btn.click();
  await page.waitForTimeout(200);
  await btn.click();
  await page.waitForTimeout(1900);
  const after = (await btn.textContent()).trim();
  check('copy button: a double click still returns to its label', after === label, `was "${label}", is "${after}"`);
}

// htmx 2 has no `api.onElRemoved`; the SSE extension called it on connect,
// which logged a TypeError on the live log page and closed nothing.
async function liveLogsConnectCleanly(page) {
  const errs = [];
  page.on('pageerror', (e) => errs.push(e.message));
  page.on('console', (m) => { if (m.type() === 'error') errs.push(m.text()); });
  await page.goto(`${BASE}/admin/system/logs/page`, { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => /Connected/.test(document.getElementById('conn-status').textContent), null, { timeout: 5000 }).catch(() => {});
  const status = await page.textContent('#conn-status');
  check('live logs: the stream connects', /Connected/.test(status), `status "${status}"`);
  check('live logs: connecting logs no error', errs.length === 0, JSON.stringify(errs).slice(0, 300));
}

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
    ['clip glyphs follow playback', clipGlyphsFollowPlayback],
    ['float dock clears the tab bar', floatDockClearsTheTabBar],
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
    ['live feed after back/forward', liveFeedSurvivesBackForward],
    ['live detection refreshes the feed', liveDetectionRefreshesFeed],
    ['palette loads on open', paletteLoadsOnOpen],
    ['co-occurrence range holds', correlationRangeHolds],
    ['help drawer deep link', helpDrawerDeepLink],
    ['hidden tabs do not poll', hiddenTabsDoNotPoll],
    ['display preferences', displayPrefs],
    ['theme with storage blocked', themeWithStorageBlocked],
    ['today source picker stays', todaySourcePickerStays],
    ['today signal follows one source', todaySignalFollowsOneSource],
    ['audio source add', audioSourceAdd],
    ['copy button returns to its label', copyButtonReturnsToItsLabel],
    ['live logs connect cleanly', liveLogsConnectCleanly],
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
