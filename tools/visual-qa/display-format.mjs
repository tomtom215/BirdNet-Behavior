// Does the dashboard print clock times and temperatures in the clock and unit
// the reader chose (static/display-format.js, src/display.rs)?
//
//   1. THE TABLE. The browser's formatters are run on every case in
//      crates/birdnet-web/tests/data/display_cases.json, the same file the Rust
//      formatter is held to, so the two implementations cannot drift apart.
//   2. THE PAGES. With the station default 24-hour, every marked time on the
//      detection page is the canonical text; choosing 12-hour on the Display
//      card re-prints them all (and nothing is left in 24-hour); choosing the
//      station default again restores them; the choice survives a reload.
//   3. THE STATION DEFAULT. `time_format=12h` and `temperature_unit=fahrenheit`
//      in the settings table are what a browser with no choice shows, and a
//      browser's own choice beats them. The default is read from
//      /api/v2/display on each page load.
//   4. LATER CONTENT. A row arriving after load (htmx, the live feed) is printed
//      too.
//   5. STORAGE THAT THROWS leaves the canonical text and no page error.
//
// Usage:  BASE=http://127.0.0.1:8516 FIXTURE_DB=/path/to/bnb_screenshots.db \
//         CHROMIUM_PATH=... node display-format.mjs
//         MUTATE=1 node display-format.mjs   (self-test: the 12-hour branch of
//                                             the browser formatter is broken;
//                                             check 1 must go red)
import { chromium } from 'playwright';
import fs from 'node:fs';
import { execFileSync } from 'node:child_process';

const BASE = process.env.BASE || 'http://127.0.0.1:8502';
const CHROMIUM_PATH = process.env.CHROMIUM_PATH || '';
const FIXTURE_DB = process.env.FIXTURE_DB || '';
const MUTATE = Boolean(process.env.MUTATE);

const failures = [];
const ok = (cond, msg) => {
  if (!cond) { failures.push(msg); console.log('  FAIL', msg); }
};

const cases = JSON.parse(
  fs.readFileSync(new URL('../../crates/birdnet-web/tests/data/display_cases.json', import.meta.url), 'utf8'),
);

const setStationDefault = (key, value) => {
  if (!FIXTURE_DB) return false;
  execFileSync('python3', ['-I', '-c', `
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
c.execute("CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL, category TEXT NOT NULL DEFAULT 'general', updated_at TEXT NOT NULL DEFAULT (datetime('now')))")
if sys.argv[3] == '':
    c.execute("DELETE FROM settings WHERE key = ?", (sys.argv[2],))
else:
    c.execute("INSERT OR REPLACE INTO settings(key, value, category) VALUES(?, ?, 'System')", (sys.argv[2], sys.argv[3]))
c.commit()
`, FIXTURE_DB, key, value]);
  return true;
};

const browser = await chromium.launch(CHROMIUM_PATH ? { executablePath: CHROMIUM_PATH } : {});
const newPage = async (init) => {
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  if (init) await ctx.addInitScript(init);
  const page = await ctx.newPage();
  const errors = [];
  page.on('pageerror', (e) => errors.push(String(e)));
  return { ctx, page, errors };
};

// Start from a station with no choices stored.
setStationDefault('time_format', '');
setStationDefault('temperature_unit', '');

// ---- 1: the shared table ----------------------------------------------------

{
  const { ctx, page } = await newPage();
  await page.goto(`${BASE}/station/settings`, { waitUntil: 'networkidle' });
  if (MUTATE) {
    await page.evaluate(() => {
      const real = window.bnbDisplay.formatClock;
      window.bnbDisplay.formatClock = (t, f) => (f === '12h' ? real(t, '24h') : real(t, f));
    });
  }
  const got = await page.evaluate(({ clock, temp }) => {
    const out = { clock: [], temp: [] };
    for (const [input, format] of clock) out.clock.push(window.bnbDisplay.formatClock(input, format));
    for (const [c, dp, unit, withUnit] of temp) out.temp.push(window.bnbDisplay.formatTemperature(c, dp, unit, withUnit));
    return out;
  }, cases);
  cases.clock.forEach((row, i) => ok(got.clock[i] === row[2], `clock ${JSON.stringify(row.slice(0, 2))}: browser ${JSON.stringify(got.clock[i])}, table ${JSON.stringify(row[2])}`));
  cases.temp.forEach((row, i) => ok(got.temp[i] === row[4], `temperature ${JSON.stringify(row.slice(0, 4))}: browser ${JSON.stringify(got.temp[i])}, table ${JSON.stringify(row[4])}`));
  console.log(`table: ${cases.clock.length} clock and ${cases.temp.length} temperature cases`);
  await ctx.close();
}

// ---- 2: the pages --------------------------------------------------------------

const detections = await (await fetch(`${BASE}/api/v2/detections?limit=1`)).json();
const d = detections.detections[0];
const detailUrl = `${BASE}/detections/detail?date=${d.date}&time=${encodeURIComponent(d.time)}&name=${encodeURIComponent(d.com_name)}`;
const clockTexts = (page) => page.evaluate(() => [...document.querySelectorAll('[data-clk]')].map((e) => [e.getAttribute('data-clk'), e.textContent]));
const is12 = (s) => /^\d{1,2}:\d{2}(:\d{2})? (AM|PM)$/.test(s);

{
  const { ctx, page, errors } = await newPage();
  await page.goto(detailUrl, { waitUntil: 'networkidle' });
  let marked = await clockTexts(page);
  console.log(`detail page: ${marked.length} marked times`);
  ok(marked.length >= 2, 'the detection page has marked times (it prints the time twice)');
  ok(marked.every(([c, t]) => c === t), 'with the station default 24-hour every time is canonical');

  await page.evaluate(() => localStorage.setItem('bnb-time', '12h'));
  await page.reload({ waitUntil: 'networkidle' });
  marked = await clockTexts(page);
  ok(marked.length > 0 && marked.every(([, t]) => is12(t)), `a browser that chose 12-hour prints every marked time that way: ${JSON.stringify(marked)}`);

  // Back to the station default through the Display card, with the mouse.
  await page.goto(`${BASE}/station/settings`, { waitUntil: 'networkidle' });
  const clockGroup = page.locator('#display-prefs [data-prefs-key="bnb-time"]');
  ok((await clockGroup.locator('button.active').getAttribute('data-value')) === '12h', 'the Display card shows the stored choice');
  await clockGroup.locator('[data-value="auto"]').click();
  ok((await page.evaluate(() => localStorage.getItem('bnb-time'))) === 'auto', 'choosing the station default is stored');
  await page.goto(detailUrl, { waitUntil: 'networkidle' });
  marked = await clockTexts(page);
  ok(marked.every(([c, t]) => c === t), 'the station default restores the canonical text');
  ok(errors.length === 0, `no page errors: ${errors.join('; ')}`);
  await ctx.close();
}

// ---- 3: the station default and the browser's own choice ----------------------------

if (setStationDefault('time_format', '12h') && setStationDefault('temperature_unit', 'fahrenheit')) {
  const { ctx, page } = await newPage();
  await page.goto(detailUrl, { waitUntil: 'networkidle' });
  const marked = await clockTexts(page);
  ok(marked.length > 0 && marked.every(([, t]) => is12(t)), `a station default of 12-hour reaches a browser with no choice: ${JSON.stringify(marked)}`);
  const served = await (await fetch(`${BASE}/api/v2/display`)).json();
  ok(served.time_format === '12h' && served.temperature_unit === 'fahrenheit', `/api/v2/display: ${JSON.stringify(served)}`);

  // A temperature element, the way the server writes one, in the station's unit.
  const shown = await page.evaluate(() => {
    const el = document.createElement('span');
    el.setAttribute('data-tc', '21.4'); el.setAttribute('data-dp', '0'); el.setAttribute('data-tu', '1');
    el.textContent = '21°C';
    document.body.appendChild(el);   // arrives after load: the observer prints it
    return new Promise((r) => setTimeout(() => r(el.textContent), 100));
  });
  ok(shown === '71°F', `a temperature arriving later is printed in the station's unit: ${shown}`);

  // The browser's own choice beats the station's.
  await page.evaluate(() => { localStorage.setItem('bnb-time', '24h'); localStorage.setItem('bnb-temp', 'celsius'); });
  await page.reload({ waitUntil: 'networkidle' });
  const own = await clockTexts(page);
  ok(own.every(([c, t]) => c === t), 'a browser that chose 24-hour keeps it against a 12-hour station');
  await ctx.close();

  // A time that arrives later (the live feed, an htmx swap).
  const live = await newPage();
  await live.page.goto(detailUrl, { waitUntil: 'networkidle' });
  const late = await live.page.evaluate(() => {
    const el = document.createElement('span');
    el.setAttribute('data-clk', '16:31:37');
    el.textContent = '16:31:37';
    document.body.appendChild(el);
    return new Promise((r) => setTimeout(() => r(el.textContent), 100));
  });
  ok(late === '4:31:37 PM', `a time arriving after load is printed in the station's clock: ${late}`);
  await live.ctx.close();
} else {
  console.log('(no FIXTURE_DB: the station-default checks are skipped)');
}
setStationDefault('time_format', '');
setStationDefault('temperature_unit', '');

// ---- 5: storage that throws ---------------------------------------------------------

{
  const { ctx, page, errors } = await newPage(() => {
    const deny = () => { throw new DOMException('denied', 'SecurityError'); };
    Object.defineProperty(window, 'localStorage', { get: deny });
    Object.defineProperty(window, 'sessionStorage', { get: deny });
  });
  await page.goto(detailUrl, { waitUntil: 'networkidle' });
  const marked = await clockTexts(page);
  ok(marked.length > 0 && marked.every(([c, t]) => c === t), 'with storage blocked the canonical text stands');
  ok(errors.length === 0, `no page error when storage throws: ${errors.join('; ')}`);
  await ctx.close();
}

await browser.close();
if (failures.length) {
  console.log(`\n${failures.length} FAILED`);
  process.exit(1);
}
console.log('\ndisplay-format: all checks passed');
