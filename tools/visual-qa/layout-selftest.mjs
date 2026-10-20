// Does the layout gate grade what a reader sees? Two cases, each against the
// running fixture, each the other's counterpart:
//
//   clipped   a line too long for the Today feed's phone column is cut off by
//             `overflow: hidden` + an ellipsis. The text's own box still runs
//             under the play button beside it (Range.getClientRects reports the
//             whole run), but none of that part is painted. The gate flagged it
//             once: "Ruby-throated Hummingbird" x the play button, on the first
//             CI run of the gate, because that day's seeded feed held the name.
//             Must NOT be reported. The common name no longer clips on a phone
//             (it wraps to a second line, so the commonest birds are readable);
//             the scientific name beneath it still does, so the case now puts a
//             long binomial there. Moving the case to a line that really clips
//             is the point: the original, left as it was, stopped meeting its
//             own precondition the day the name started to wrap.
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
const NAME = 'Pseudoscopelus lavalaetus ruby-throated hummingbirdus';

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
  const a = document.querySelector('#detections-table .feed-row .who .sci');
  const play = a && a.closest('.feed-row').querySelector('button.x-fplay');
  if (!a || !play) return null;
  a.textContent = name;
  const rg = document.createRange();
  rg.selectNodeContents(a.firstChild);
  const t = rg.getBoundingClientRect();
  return { textRight: t.right, boxRight: a.getBoundingClientRect().right, playLeft: play.getBoundingClientRect().left };
}, NAME);
check(geometry !== null, 'the fixture has a Today feed row to put a long line in');
if (geometry) {
  // The precondition: without it the case proves nothing about clipping.
  check(
    geometry.textRight > geometry.playLeft && geometry.boxRight <= geometry.playLeft,
    `the long line's text (to ${Math.round(geometry.textRight)}) runs past the play button (from ${Math.round(geometry.playLeft)}) while its clipping box ends at ${Math.round(geometry.boxRight)}`,
  );
  const clipped = await overlaps();
  check(clipped.length === 0, `a line clipped by its own box is not reported as overlapping (${clipped.join(' | ') || 'none'})`);
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

// The long line again, this time not clipped: it is painted under the play
// button, and the gate that stays silent about the clipped one must speak here.
await fresh();
await page.evaluate((name) => {
  const sci = document.querySelector('#detections-table .feed-row .who .sci');
  sci.textContent = name;
  sci.style.overflow = 'visible';
  sci.style.textOverflow = 'clip';
}, NAME);
const unclipped = await overlaps();
check(
  unclipped.length > 0,
  `the same line left unclipped is reported (${unclipped.slice(0, 2).join(' | ') || 'none'})`,
);

await browser.close();
if (failed) process.exit(1);
console.log('layout-selftest: all pass');
