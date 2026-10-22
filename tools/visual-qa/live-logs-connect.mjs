// Cross-engine gate: the live log viewer says "Connected" at once, in every engine.
//
// `/admin/system/logs` is a Server-Sent Events stream and the viewer's status
// line is driven by `EventSource.onopen`. Chromium and WebKit fire `open` when
// the response headers arrive; Firefox fires it on the first byte of the BODY.
// On a quiet log the first body byte used to be the 15-second keep-alive
// comment, so in Firefox only the viewer read "Connecting…" for up to 15 s over
// a stream that was open and healthy. `interactions.mjs` has the same check
// ("live logs: the stream connects") but runs Chromium only, and waits 5 s.
//
// This measures the time to "Connected" in each engine and fails over
// `LIMIT_MS` (default 3000: far inside the keep-alive, far outside a normal
// connect). The log is quiet on a fresh fixture, which is the case that
// matters; a fixture whose log already holds lines would pass on the old code
// because the replayed backlog is itself a body byte.
//
//   node live-logs-connect.mjs
//
// Env: BASE (default http://127.0.0.1:8502), CHROMIUM_PATH, ENGINES (default
// `chromium,firefox,webkit`; an engine that is asked for and cannot launch is a
// failure, not a skip), LIMIT_MS, PLAYWRIGHT_BROWSERS_PATH for the other two.
import { chromium, firefox, webkit } from 'playwright';

const BASE = process.env.BASE || 'http://127.0.0.1:8502';
const LIMIT_MS = Number(process.env.LIMIT_MS || 3000);
const CHROMIUM_PATH = process.env.CHROMIUM_PATH || '';
const WANT = (process.env.ENGINES || 'chromium,firefox,webkit').split(',').map((s) => s.trim()).filter(Boolean);
const TYPES = { chromium, firefox, webkit };

const failures = [];
const check = (name, ok, detail) => {
  console.log(`  ${ok ? '\x1b[32mPASS\x1b[0m' : '\x1b[31mFAIL\x1b[0m'} ${name}${ok ? '' : `\n       ${detail}`}`);
  if (!ok) failures.push(name);
};

for (const name of WANT) {
  const type = TYPES[name];
  if (!type) { check(`${name}: known engine`, false, 'expected chromium, firefox or webkit'); continue; }
  let browser;
  try {
    browser = await type.launch(name === 'chromium' && CHROMIUM_PATH ? { executablePath: CHROMIUM_PATH } : {});
  } catch (e) {
    check(`${name}: launched`, false, String(e.message || e).split('\n')[0]);
    continue;
  }
  try {
    const page = await browser.newPage();
    const errors = [];
    page.on('pageerror', (e) => errors.push(String(e)));
    const t0 = Date.now();
    await page.goto(`${BASE}/admin/system/logs/page`, { waitUntil: 'domcontentloaded' });
    let ms = null;
    try {
      await page.waitForFunction(
        () => /Connected/.test(document.getElementById('conn-status')?.textContent || ''),
        null, { timeout: 20000, polling: 50 },
      );
      ms = Date.now() - t0;
    } catch { /* reported below with the status text */ }
    const status = await page.textContent('#conn-status').catch(() => '(no #conn-status)');
    console.log(`\n${name} ${browser.version()}: "${status}"${ms === null ? ' (never connected in 20 s)' : ` after ${ms} ms`}`);
    check(`${name}: the live log connects within ${LIMIT_MS} ms`, ms !== null && ms <= LIMIT_MS,
      `status "${status}", ${ms === null ? 'never' : `${ms} ms`}`);
    check(`${name}: connecting raises no page error`, errors.length === 0, errors.join(' | ').slice(0, 300));
  } finally {
    await browser.close();
  }
}

if (failures.length) { console.log(`\n${failures.length} failed: ${failures.join('; ')}`); process.exit(1); }
console.log('\nall passed');
