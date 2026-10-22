// Does qa.mjs's "stuck Loading…" check grade what a reader sees? Four cases against the
// running fixture, each the others' counterpart:
//
//   closed     a "Loading…" placeholder inside a CLOSED <details> is not on screen. Chromium 141
//              leaves `offsetParent` set for such content, so the old test reported the
//              /patterns "See the numbers" disclosure as stuck in all four states. Must NOT be
//              reported.
//   visible    the same placeholder in the page itself: a loader that never finished. Must be
//              reported, or the closed case passing only means the check stopped looking.
//   open       the placeholder in an OPEN <details>: on screen, so reported.
//   hidden     the placeholder in an element with the `hidden` attribute: not on screen, not
//              reported.
//
// Env: BASE (default http://127.0.0.1:8502), CHROMIUM_PATH.
import { chromium } from 'playwright';
import { diagnose } from './qa.mjs';

const BASE = process.env.BASE || 'http://127.0.0.1:8502';
const CHROMIUM_PATH = process.env.CHROMIUM_PATH || '';
const browser = await chromium.launch(CHROMIUM_PATH ? { executablePath: CHROMIUM_PATH } : {});
const page = await (await browser.newContext({ viewport: { width: 1440, height: 900 } })).newPage();
let failed = false;
const check = (ok, what) => {
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${what}`);
  if (!ok) failed = true;
};

async function stuckWith(build) {
  await page.goto(`${BASE}/`, { waitUntil: 'networkidle' });
  await page.waitForTimeout(500);
  await page.evaluate(build);
  return (await diagnose(page)).stuck;
}

check((await stuckWith(() => {})).length === 0, 'precondition: the fresh Today page has no stuck loader');

const closed = await stuckWith(() => {
  const d = document.createElement('details');
  d.innerHTML = '<summary>See the numbers</summary><div><span class="bnb-meta">Loading…</span></div>';
  document.querySelector('main').appendChild(d);
});
check(closed.length === 0, `a placeholder inside a closed disclosure is not reported (${closed.join(' | ') || 'none'})`);

const visible = await stuckWith(() => {
  const s = document.createElement('span');
  s.className = 'bnb-meta';
  s.textContent = 'Loading…';
  document.querySelector('main').appendChild(s);
});
check(visible.length === 1, `a placeholder on the page is reported (${visible.join(' | ') || 'none'})`);

const open = await stuckWith(() => {
  const d = document.createElement('details');
  d.open = true;
  d.innerHTML = '<summary>See the numbers</summary><div><span class="bnb-meta">Loading…</span></div>';
  document.querySelector('main').appendChild(d);
});
check(open.length === 1, `a placeholder inside an open disclosure is reported (${open.join(' | ') || 'none'})`);

const hidden = await stuckWith(() => {
  const d = document.createElement('div');
  d.hidden = true;
  d.innerHTML = '<span class="bnb-meta">Loading…</span>';
  document.querySelector('main').appendChild(d);
});
check(hidden.length === 0, `a placeholder in a hidden element is not reported (${hidden.join(' | ') || 'none'})`);

await browser.close();
if (failed) process.exit(1);
console.log('qa-selftest: all pass');
