// Does focusing a control change the control?
//
// The focus indicator is the one piece of chrome a keyboard user reads every
// time they press Tab, and it is the one a screenshot never shows: nothing has
// focus when the page is captured. Two things are held, by tabbing through every
// route the way a person does and reading what the browser computes:
//
//   shape     the focused element keeps its own border radius. A rule that
//             writes `border-radius` onto `:focus-visible` squares off every pill
//             and round button the moment it is focused, so the control changes
//             shape under the finger that just reached it.
//   ring      the indicator is at least 2px wide and 3:1 against what is behind
//             the element (WCAG 2.4.11 and 1.4.11), where "what is behind" is the
//             nearest ancestor background that is painted.
//
// A browser media control bar (<audio controls>) takes one Tab stop per button
// inside it and draws its own indicator there; those stops are counted and
// reported but not graded, since CSS cannot reach them.
//
// Per route, up to TABS focus stops (default 30). Fails when it focused fewer
// than 300 elements in all, so a run that could not reach the fixture, or whose
// keyboard never moved, cannot pass.
//
// Usage:  BASE=... CHROMIUM_PATH=... node focus.mjs
import { chromium } from 'playwright';
import { ROUTES } from './qa.mjs';

const BASE = process.env.BASE || 'http://127.0.0.1:8502';
const CHROMIUM_PATH = process.env.CHROMIUM_PATH || '';
const TABS = Number(process.env.TABS || 30);

const lin = (c) => { c /= 255; return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4; };
const lum = ([r, g, b]) => 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
const contrast = (a, b) => { const [hi, lo] = [lum(a), lum(b)].sort((x, y) => y - x); return (hi + 0.05) / (lo + 0.05); };

const browser = await chromium.launch(CHROMIUM_PATH ? { executablePath: CHROMIUM_PATH } : {});
const failures = [];
let stops = 0;
let changedShape = 0;
let weakRing = 0;
let nativeStops = 0;

for (const theme of ['light', 'dark']) {
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 900 }, colorScheme: theme, reducedMotion: 'reduce' });
  await ctx.addInitScript((t) => { try { localStorage.setItem('theme', t); } catch (e) {} }, theme);
  const page = await ctx.newPage();
  for (const [name, route] of ROUTES) {
    try {
      await page.goto(BASE + route, { waitUntil: 'domcontentloaded', timeout: 25000 });
      await page.waitForLoadState('networkidle', { timeout: 9000 }).catch(() => {});
      await page.waitForTimeout(300);
    } catch (e) { continue; }
    // What every focusable element's shape is while nothing has focus.
    await page.evaluate(() => {
      window.__radius = new WeakMap();
      for (const el of document.querySelectorAll('a[href], button, input, select, textarea, summary, [tabindex]')) {
        window.__radius.set(el, getComputedStyle(el).borderRadius);
      }
      // A canvas to resolve any CSS colour to sRGB.
      const c = document.createElement('canvas'); c.width = c.height = 1;
      const ctx2 = c.getContext('2d', { willReadFrequently: true });
      window.__rgb = (css) => { ctx2.clearRect(0, 0, 1, 1); ctx2.fillStyle = '#000'; ctx2.fillStyle = css; ctx2.fillRect(0, 0, 1, 1); const d = ctx2.getImageData(0, 0, 1, 1).data; return [d[0], d[1], d[2], d[3]]; };
    });
    await page.evaluate(() => { document.body.focus && document.body.focus(); window.scrollTo(0, 0); });
    for (let i = 0; i < TABS; i++) {
      await page.keyboard.press('Tab');
      const r = await page.evaluate(() => {
        const el = document.activeElement;
        if (!el || el === document.body) return null;
        const cs = getComputedStyle(el);
        const before = window.__radius.get(el);
        // The nearest painted background behind the element.
        let bg = [255, 255, 255, 255];
        // Behind the ring, not behind the element: the ring is drawn outside it.
        for (let n = el.parentElement; n; n = n.parentElement) {
          const c = window.__rgb(getComputedStyle(n).backgroundColor);
          if (c[3] === 255) { bg = c; break; }
        }
        // A <audio controls> takes several Tab stops: the element itself, then each
        // button inside the browser's own control bar. While focus is inside, the
        // element is not :focus-visible and `activeElement` still names it; the
        // indicator then belongs to the browser and CSS cannot reach it.
        const native = (el.tagName === 'AUDIO' || el.tagName === 'VIDEO') && !el.matches(':focus-visible');
        const outlineOn = cs.outlineStyle !== 'none' && parseFloat(cs.outlineWidth) > 0;
        const shadow = cs.boxShadow !== 'none';
        const ring = outlineOn ? window.__rgb(cs.outlineColor) : null;
        return {
          what: el.tagName.toLowerCase() + (typeof el.className === 'string' && el.className ? '.' + el.className.trim().split(/\s+/)[0] : ''),
          before, after: cs.borderRadius,
          outlineWidth: outlineOn ? parseFloat(cs.outlineWidth) : 0,
          shadow, ring, bg, native,
        };
      });
      if (!r) continue;
      if (r.native) { nativeStops++; continue; }
      stops++;
      if (r.before !== undefined && r.before !== r.after) {
        changedShape++;
        if (failures.length < 25) failures.push(`${theme} ${name}: ${r.what} changes shape on focus (${r.before} -> ${r.after})`);
      }
      if (r.ring) {
        const c = contrast(r.ring.slice(0, 3), r.bg.slice(0, 3));
        if (r.outlineWidth < 2 || c < 3) {
          weakRing++;
          if (failures.length < 25) failures.push(`${theme} ${name}: ${r.what} focus ring ${r.outlineWidth}px at ${c.toFixed(2)}:1`);
        }
      } else if (!r.shadow) {
        weakRing++;
        if (failures.length < 25) failures.push(`${theme} ${name}: ${r.what} shows no focus indicator`);
      }
    }
  }
  await ctx.close();
}
await browser.close();

console.log(`${stops} focus stops on ${ROUTES.length} routes x 2 themes: ${changedShape} change shape, ${weakRing} have a missing or weak ring (${nativeStops} more stops sat inside a browser media control bar and were not graded)`);
if (failures.length) console.log('  ' + failures.slice(0, 12).join('\n  '));
if (stops < 300) { console.error('focus: fewer than 300 stops reached, the keyboard did not move'); process.exit(1); }
process.exit(changedShape || weakRing ? 1 : 0);
