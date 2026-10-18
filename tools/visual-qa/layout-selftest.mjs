// Does the layout gate grade what a reader sees? Two cases, each against the
// running fixture, each the other's counterpart:
//
//   clipped   a species name too long for the Today feed's phone column is cut
//             off by `overflow: hidden` + an ellipsis. The text's own box still
//             runs under the play button beside it (Range.getClientRects reports
//             the whole run), but none of that part is painted. The gate flagged
//             it: "Ruby-throated Hummingbird" x the play button, on the first CI
//             run of the gate, because that day's seeded feed held the name.
//             Must NOT be reported.
//   painted   the same row's scientific name moved up onto the common name with
//             a CSSOM style (allowed by the CSP, unlike a style attribute). Two
//             painted lines over each other. Must be reported, or the clipped
//             case passing only means the gate stopped looking.
//
// Env: BASE (default http://127.0.0.1:8502), CHROMIUM_PATH.
import { chromium } from 'playwright';
import { inPage } from './layout.mjs';

const BASE = process.env.BASE || 'http://127.0.0.1:8502';
const CHROMIUM_PATH = process.env.CHROMIUM_PATH || '';
const NAME = 'Ruby-throated Hummingbird';

const browser = await chromium.launch(CHROMIUM_PATH ? { executablePath: CHROMIUM_PATH } : {});
const context = await browser.newContext({ viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true });
const page = await context.newPage();
let failed = false;
const check = (ok, what) => {
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${what}`);
  if (!ok) failed = true;
};

async function fresh() {
  await page.goto(`${BASE}/`, { waitUntil: 'networkidle' });
  await page.waitForTimeout(900);
}
const overlaps = async () => (await page.evaluate(inPage)).overlap;

await fresh();
const geometry = await page.evaluate((name) => {
  const a = document.querySelector('#detections-table .feed-row .who .name > a');
  const play = a && a.closest('.feed-row').querySelector('button.x-fplay');
  if (!a || !play) return null;
  a.textContent = name;
  const rg = document.createRange();
  rg.selectNodeContents(a.firstChild);
  const t = rg.getBoundingClientRect();
  return { textRight: t.right, boxRight: a.getBoundingClientRect().right, playLeft: play.getBoundingClientRect().left };
}, NAME);
check(geometry !== null, 'the fixture has a Today feed row to put a name in');
if (geometry) {
  // The precondition: without it the case proves nothing about clipping.
  check(
    geometry.textRight > geometry.playLeft && geometry.boxRight <= geometry.playLeft,
    `the long name's text (to ${Math.round(geometry.textRight)}) runs past the play button (from ${Math.round(geometry.playLeft)}) while its clipping box ends at ${Math.round(geometry.boxRight)}`,
  );
  const clipped = await overlaps();
  check(clipped.length === 0, `a name clipped by its own box is not reported as overlapping (${clipped.join(' | ') || 'none'})`);
}

await fresh();
await page.evaluate(() => {
  const who = document.querySelector('#detections-table .feed-row .who');
  const sci = who.querySelector('.sci');
  sci.style.position = 'relative';
  sci.style.top = '-14px';
});
const painted = await overlaps();
check(
  painted.some((f) => f.includes('div.sci')),
  `two painted lines over each other are reported (${painted.join(' | ') || 'none'})`,
);

await browser.close();
if (failed) process.exit(1);
console.log('layout-selftest: all pass');
