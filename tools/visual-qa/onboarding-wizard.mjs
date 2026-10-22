// Behavioural gate for the setup wizard's location step and its last step.
//
// Three things a screenshot cannot show, held by driving the page:
//
//   zone     "Look up my approximate location" fills the coordinates and a hidden
//            time zone. A person who then types DIFFERENT coordinates must not be
//            saved with the looked-up zone: 50.0 / 5.0 was saved as
//            America/Los_Angeles, and Diagnostics then told them to run
//            `timedatectl set-timezone America/Los_Angeles`. The form is read
//            with FormData, the way the browser would send it, without
//            submitting. The look-up is answered by a route fixture (no network),
//            and the gate fails if the fixture was never used.
//   keep     the counterpart: the look-up left alone, or its own coordinates
//            typed again, keeps the zone, so a gate that always cleared it
//            cannot pass.
//   buttons  "Skip the rest and finish" is offered on the steps before the last
//            and not on the last, where "Finish & save" is the one finishing
//            button; and the last step's heading does not say "listening" over
//            a body that says nothing has been saved.
//
// Usage:  BASE=http://127.0.0.1:8581 CHROMIUM_PATH=... node onboarding-wizard.mjs
// MUTATE=zone breaks the page script's clearing of the zone; MUTATE=buttons puts back the
// last step's old heading and the skip button on every step. Each must go red.
import { chromium } from 'playwright';

const BASE = process.env.BASE || 'http://127.0.0.1:8502';
const CHROMIUM_PATH = process.env.CHROMIUM_PATH || '';
const MUTATE = process.env.MUTATE || '';

const browser = await chromium.launch(CHROMIUM_PATH ? { executablePath: CHROMIUM_PATH } : {});
const failures = [];
const check = (ok, what) => { if (!ok) failures.push(what); console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`); };

async function open() {
  const ctx = await browser.newContext({ viewport: { width: 1100, height: 900 } });
  const page = await ctx.newPage();
  let lookups = 0;
  await page.route('**/admin/settings/detect-location', (route) => {
    lookups += 1;
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ lat: 34.0522, lon: -118.2437, timezone: 'America/Los_Angeles', city: 'Los Angeles', country: 'United States' }),
    });
  });
  if (MUTATE) {
    await page.route('**/onboarding', async (route) => {
      const res = await route.fetch();
      let body = await res.text();
      if (MUTATE === 'zone') { body = body.replace('function forgetTimezone() {', 'function forgetTimezone() { return;'); }
      // The pre-fix last step: a heading that claims it, and a skip button on every step.
      if (MUTATE === 'buttons') {
        body = body.replace('Ready to <em>save</em>.', 'You\'re <em>listening</em>.')
          .replace("skip.style.display = step === total ? 'none' : '';", '');
      }
      await route.fulfill({ response: res, body });
    });
  }
  await page.goto(BASE + '/onboarding', { waitUntil: 'networkidle' });
  return { ctx, page, lookups: () => lookups };
}

const formValue = (page, name) =>
  page.evaluate((n) => new FormData(document.getElementById('ob-form')).get(n), name);

async function toStep(page, n) {
  // Forward from wherever the wizard is. Step 1 may hold a password pair; blank
  // is allowed.
  for (;;) {
    const cur = Number(await page.$eval('#ob-cur', (e) => e.textContent));
    if (cur >= n) { return; }
    await page.click('#ob-next');
    await page.waitForFunction((k) => Number(document.getElementById('ob-cur').textContent) > k, cur);
  }
}

// ---- zone: different coordinates typed after a look-up ----------------------
{
  const { ctx, page, lookups } = await open();
  await toStep(page, 3);
  await page.click('#ob-detect');
  await page.waitForFunction(() => document.getElementById('ob-tz').value !== '');
  check((await formValue(page, 'timezone')) === 'America/Los_Angeles', 'the look-up fills the time zone');
  check((await formValue(page, 'timezone_latitude')) === '34.0522', 'and records the latitude it is for');
  await page.fill('#ob-lat', '50.0');
  await page.fill('#ob-lon', '5.0');
  check((await formValue(page, 'latitude')) === '50.0', 'the typed latitude is what the form carries');
  check((await formValue(page, 'timezone')) === '', 'typing other coordinates drops the looked-up zone');
  check((await formValue(page, 'timezone_latitude')) === '', '... and the coordinates it was for');
  check(lookups() === 1, 'the look-up fixture was used exactly once');
  await ctx.close();
}

// ---- keep: the look-up left alone, or typed back, keeps its zone -----------
{
  const { ctx, page } = await open();
  await toStep(page, 3);
  await page.click('#ob-detect');
  await page.waitForFunction(() => document.getElementById('ob-tz').value !== '');
  check((await formValue(page, 'timezone')) === 'America/Los_Angeles', 'untouched look-up keeps its zone');
  await page.fill('#ob-lat', '34.0522');
  await page.fill('#ob-lon', '-118.2437');
  check((await formValue(page, 'timezone')) === 'America/Los_Angeles', 'its own coordinates typed again keep the zone');
  await page.fill('#ob-lat', '34,0522');
  check((await formValue(page, 'timezone')) === 'America/Los_Angeles', 'a decimal comma for the same point keeps the zone');
  await ctx.close();
}

// ---- buttons ---------------------------------------------------------------
{
  const { ctx, page } = await open();
  const total = await page.$$eval('section.ob-step', (n) => n.length);
  const skipShown = () => page.$eval('#ob-skip', (b) => getComputedStyle(b).display !== 'none');
  check(await skipShown(), 'the skip button is offered on step 1');
  check((await page.$eval('#ob-skip', (b) => b.textContent.trim())) === 'Skip the rest and finish', 'and says what it does');
  await toStep(page, total - 1);
  check(await skipShown(), `and on step ${total - 1}`);
  await toStep(page, total);
  check(!(await skipShown()), `but not on the last step (${total}), where Finish & save is the one button`);
  check((await page.$eval('#ob-next', (b) => b.textContent.trim())).startsWith('Finish & save'), 'the last step has Finish & save');
  const heading = await page.$eval('section.ob-step.active h1', (h) => h.textContent.trim());
  check(!/listening/i.test(heading), `the last step's heading does not claim the station is listening (“${heading}”)`);
  await ctx.close();
}

await browser.close();
if (failures.length) {
  console.error(`\n${failures.length} check(s) failed:\n- ${failures.join('\n- ')}`);
  process.exit(1);
}
console.log('\nonboarding wizard gate: all checks passed');
