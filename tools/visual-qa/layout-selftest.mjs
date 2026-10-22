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
//   widen     the document wider than the screen (counterparts, each the others'):
//             a 600px unbreakable line in `main` must be reported and named; the
//             same line inside an `overflow-x: auto` box that fits must not be (a
//             table that scrolls is the page behaving); and a long identifier in a
//             `.hint`, the original defect, must not widen the page at all, which
//             is the stylesheet's job and fails on the CSS before the fix.
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
// Which Today row to put the long line in. A row whose common name wraps to two lines on a
// phone has its scientific name a line lower, so a long one passes under the play button's
// glyph with only a sliver of vertical overlap, which the gate rightly does not call a
// collision. The case needs a row where the two lines share a band; the fixture's first row
// depends on the time of day it was seeded, so this finds one rather than assuming row 0.
const ROW = await page.evaluate((name) => {
  const rows = [...document.querySelectorAll('#detections-table .feed-row')];
  for (let i = 0; i < rows.length; i++) {
    const a = rows[i].querySelector('.who .sci');
    const play = rows[i].querySelector('button.x-fplay');
    if (!a || !play || !a.firstChild || !play.firstChild) continue;
    const band = (node) => { const g = document.createRange(); g.selectNodeContents(node); const r = g.getBoundingClientRect(); return [r.top, r.bottom]; };
    const [t0, t1] = band(a.firstChild);
    const [p0, p1] = band(play.firstChild);
    if (Math.min(t1, p1) - Math.max(t0, p0) >= 6) return i;
  }
  return -1;
}, NAME);
check(ROW >= 0, `the fixture has a Today row whose scientific name shares a line with its play button (row ${ROW})`);
const geometry = await page.evaluate(([name, row]) => {
  const a = document.querySelectorAll('#detections-table .feed-row')[row]?.querySelector('.who .sci');
  const play = a && a.closest('.feed-row').querySelector('button.x-fplay');
  if (!a || !play) return null;
  a.textContent = name;
  const rg = document.createRange();
  rg.selectNodeContents(a.firstChild);
  const t = rg.getBoundingClientRect();
  return { textRight: t.right, boxRight: a.getBoundingClientRect().right, playLeft: play.getBoundingClientRect().left };
}, [NAME, ROW]);
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
await page.evaluate(([name, row]) => {
  const sci = document.querySelectorAll('#detections-table .feed-row')[row].querySelector('.who .sci');
  sci.textContent = name;
  sci.style.overflow = 'visible';
  sci.style.textOverflow = 'clip';
}, [NAME, ROW]);
const unclipped = await overlaps();
check(
  unclipped.length > 0,
  `the same line left unclipped is reported (${unclipped.slice(0, 2).join(' | ') || 'none'})`,
);

const widen = async () => (await page.evaluate(inPage)).widen;
// Built with the DOM and CSSOM, not markup: the CSP refuses a style attribute.
const plant = (spec) =>
  page.evaluate((sp) => {
    const host = document.querySelector('main');
    const make = (n) => {
      const el = document.createElement(n.tag);
      if (n.id) el.id = n.id;
      if (n.cls) el.className = n.cls;
      if (n.text) el.textContent = n.text;
      Object.assign(el.style, n.style || {});
      for (const k of n.kids || []) el.appendChild(make(k));
      return el;
    };
    host.appendChild(make(sp));
  }, spec);
const WORD = 'APPRISE_MINIMUM_SECONDS_BETWEEN_NOTIFICATIONS_PER_SPECIES_AND_SOME_MORE_OF_IT_BEYOND_ANY_PHONE';

await fresh();
check((await widen()).length === 0, 'precondition: the fresh Today page is not wider than the screen');
await plant({ tag: 'p', id: 'zz-wide', text: `${WORD} ${WORD}`, style: { whiteSpace: 'nowrap', margin: '0' } });
const wide = await widen();
check(
  wide.length === 1 && wide[0].includes('p#zz-wide'),
  `a line that cannot wrap and widens the page is reported and named (${wide.join(' | ') || 'none'})`,
);

await fresh();
await plant({
  tag: 'div',
  style: { overflowX: 'auto', width: '300px' },
  kids: [{ tag: 'p', text: `${WORD} ${WORD}`, style: { whiteSpace: 'nowrap', margin: '0' } }],
});
const scrolled = await widen();
check(scrolled.length === 0, `the same line inside a box that scrolls sideways is not reported (${scrolled.join(' | ') || 'none'})`);

await fresh();
await plant({ tag: 'div', cls: 'card', kids: [{ tag: 'p', cls: 'hint', id: 'zz-hint', text: WORD }] });
const hinted = await widen();
check(hinted.length === 0, `a long identifier in a hint wraps instead of widening the page (${hinted.join(' | ') || 'none'})`);

await browser.close();
if (failed) process.exit(1);
console.log('layout-selftest: all pass');
