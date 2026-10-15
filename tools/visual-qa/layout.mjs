// Layout gate: the defects a screenshot shows and axe cannot.
//
// axe grades semantics and contrast; qa.mjs grades horizontal overflow of the
// whole page, console errors and broken images. Neither notices a line of text
// that is struck through by the input above it, a heading drawn on top of its
// eyebrow, an empty-state sentence squeezed into a 90px column inside a 1100px
// card, or a row of buttons that ends 30px past the card holding it. Every one
// of those shipped, in both themes, with every other gate green, and was found
// by looking at a rendered page. This reads the same facts from the DOM.
//
// Four checks, per route x theme x viewport:
//
//   overlap   two text boxes (not nested, not in a closed <details>, not under
//             the fixed tab bar) that intersect by more than 3px horizontally
//             and a quarter of the shorter box vertically; or a text box that
//             intersects a form control it is not part of (a hint whose margin
//             pulled it up into the input above it, with the input's border
//             running through the words).
//   squeeze   a block of 4+ lines of text narrower than 130px inside a parent
//             wider than 380px: a flex/grid child that shrank to nothing.
//   escape    a button, link, field or select whose right edge is more than 2px
//             beyond the card (or list item) that holds it, where that card does
//             not itself scroll sideways.
//   tiny      a visible text node set below 9px (the floor the legibility gate
//             holds SVG labels to; HTML text had none).
//
// Env:
//   BASE      base url                       (default http://127.0.0.1:8502)
//   THEMES    csv of light,dark              (default light,dark)
//   VPS       csv of desktop,mobile          (default desktop,mobile)
//   ONLY      substring filter on route name
//   MIN_ROUTES  fail if fewer routes were graded than this (default 30), so a run
//             that could not reach the fixture cannot pass by grading nothing
//
// Exits non-zero on any finding. Run from this directory:
//   BASE=http://127.0.0.1:8502 node layout.mjs
import { chromium } from 'playwright';
import { ROUTES } from './qa.mjs';

const BASE = process.env.BASE || 'http://127.0.0.1:8502';
const CHROMIUM_PATH = process.env.CHROMIUM_PATH || '';
const THEMES = (process.env.THEMES || 'light,dark').split(',').filter(Boolean);
const VP_TABLE = {
  desktop: { viewport: { width: 1440, height: 900 }, hasTouch: false },
  mobile: { viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true },
};
const VPS = (process.env.VPS || 'desktop,mobile').split(',').filter((v) => VP_TABLE[v]);
const ONLY = process.env.ONLY || '';
const MIN_ROUTES = Number(process.env.MIN_ROUTES || 30);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// Runs in the page. Returns { overlap, squeeze, escape, tiny } arrays of strings.
function inPage() {
  const res = { overlap: [], squeeze: [], escape: [], tiny: [] };
  const sel = (el) =>
    el.tagName.toLowerCase() +
    (el.id ? '#' + el.id : '') +
    (el.className && typeof el.className === 'string' ? '.' + el.className.trim().split(/\s+/).slice(0, 2).join('.') : '');
  const shown = (el) => {
    const r = el.getBoundingClientRect();
    const cs = getComputedStyle(el);
    return r.width > 0 && r.height > 0 && cs.visibility !== 'hidden' && cs.display !== 'none' && cs.opacity !== '0';
  };
  const SKIP =
    'script,style,noscript,svg,option,[aria-hidden="true"],.bnb-skel,.sr-only,[hidden],dialog:not([open]),template,[class*="tabbar"]';

  const items = [];
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    if (!n.nodeValue.trim()) continue;
    const el = n.parentElement;
    if (!el || !shown(el) || el.closest(SKIP)) continue;
    if (el.closest('details:not([open])') && !el.closest('summary')) continue;
    const rg = document.createRange();
    rg.selectNodeContents(n);
    const rects = [...rg.getClientRects()].filter((r) => r.width > 1 && r.height > 1);
    if (!rects.length) continue;
    items.push({ el, rects, txt: n.nodeValue.trim().slice(0, 36), size: parseFloat(getComputedStyle(el).fontSize) });
  }

  // overlap
  for (let i = 0; i < items.length && res.overlap.length < 10; i++) {
    for (let j = i + 1; j < items.length; j++) {
      const a = items[i];
      const b = items[j];
      if (a.el === b.el || a.el.contains(b.el) || b.el.contains(a.el)) continue;
      let hit = null;
      for (const ra of a.rects) {
        for (const rb of b.rects) {
          const ox = Math.min(ra.right, rb.right) - Math.max(ra.left, rb.left);
          const oy = Math.min(ra.bottom, rb.bottom) - Math.max(ra.top, rb.top);
          if (ox > 3 && oy > Math.min(ra.height, rb.height) * 0.25) {
            hit = [Math.round(ox), Math.round(oy)];
            break;
          }
        }
        if (hit) break;
      }
      if (hit) res.overlap.push(`${sel(a.el)} "${a.txt}" x ${sel(b.el)} "${b.txt}" (${hit[0]}x${hit[1]}px)`);
    }
  }

  // text over a form control it is not part of
  const controls = [...document.querySelectorAll('input:not([type=hidden]):not([type=checkbox]):not([type=radio]), select, textarea')].filter(
    (c) => shown(c) && !c.closest(SKIP) && !(c.closest('details:not([open])') && !c.closest('summary')),
  );
  for (const t of items) {
    if (res.overlap.length >= 10) break;
    if (t.el.closest('select, textarea, button, label, summary')) continue;
    for (const c of controls) {
      if (t.el.contains(c) || c.contains(t.el)) continue;
      const cr = c.getBoundingClientRect();
      for (const r of t.rects) {
        const ox = Math.min(r.right, cr.right) - Math.max(r.left, cr.left);
        const oy = Math.min(r.bottom, cr.bottom) - Math.max(r.top, cr.top);
        if (ox > 3 && oy > r.height * 0.25) {
          res.overlap.push(`${sel(t.el)} "${t.txt}" over ${sel(c)} (${Math.round(ox)}x${Math.round(oy)}px)`);
          break;
        }
      }
    }
  }

  // squeeze + tiny
  const seen = new Set();
  for (const t of items) {
    const r = t.el.getBoundingClientRect();
    const cs = getComputedStyle(t.el);
    const lh = parseFloat(cs.lineHeight) || t.size * 1.4;
    if (
      !seen.has(t.el) &&
      cs.display !== 'inline' &&
      t.el.textContent.trim().length > 40 &&
      r.width < 130 &&
      r.height / lh >= 4 &&
      t.el.parentElement &&
      t.el.parentElement.getBoundingClientRect().width > 380
    ) {
      seen.add(t.el);
      res.squeeze.push(`${sel(t.el)} is ${Math.round(r.width)}px wide and ${Math.round(r.height / lh)} lines tall: "${t.txt}"`);
    }
    if (t.size < 9 && !t.el.closest('[data-no-text-fit]')) res.tiny.push(`${sel(t.el)} ${t.size}px "${t.txt}"`);
  }

  // escape
  const holders = 'li, .bnb-card, .card, .audio-source, .audio-sources > *';
  for (const el of document.querySelectorAll('button, a[href], input:not([type=hidden]), select, textarea')) {
    if (res.escape.length >= 10) break;
    if (!shown(el) || el.closest(SKIP) || (el.closest('details:not([open])') && !el.closest('summary'))) continue;
    const holder = el.parentElement && el.parentElement.closest(holders);
    if (!holder || !shown(holder)) continue;
    let scrolls = false;
    for (let p = el.parentElement; p; p = p.parentElement) {
      const o = getComputedStyle(p).overflowX;
      if (o === 'auto' || o === 'scroll') scrolls = true;
      if (p === holder) break;
    }
    if (scrolls) continue;
    const r = el.getBoundingClientRect();
    const h = holder.getBoundingClientRect();
    if (r.right > h.right + 2 && r.left < h.right) {
      res.escape.push(`${sel(el)} "${(el.textContent || el.value || '').trim().slice(0, 24)}" ends ${Math.round(r.right - h.right)}px past ${sel(holder)}`);
    }
  }
  for (const k of Object.keys(res)) res[k] = [...new Set(res[k])].slice(0, 10);
  return res;
}

async function main() {
  const browser = await chromium.launch(CHROMIUM_PATH ? { executablePath: CHROMIUM_PATH } : {});
  const problems = [];
  let graded = 0;
  for (const theme of THEMES) {
    for (const vp of VPS) {
      const context = await browser.newContext({
        ...VP_TABLE[vp],
        deviceScaleFactor: 1,
        colorScheme: theme === 'dark' ? 'dark' : 'light',
      });
      await context.addInitScript((t) => {
        try {
          localStorage.setItem('theme', t);
        } catch (e) {
          /* ignore */
        }
      }, theme);
      const page = await context.newPage();
      for (const [name, route] of ROUTES) {
        if (ONLY && !name.includes(ONLY)) continue;
        if (route.includes('does-not-exist')) continue;
        const key = `${name}__${theme}__${vp}`;
        try {
          await page.goto(BASE + route, { waitUntil: 'domcontentloaded', timeout: 25000 });
          await page.waitForLoadState('networkidle', { timeout: 9000 }).catch(() => {});
          await sleep(900);
          const found = await page.evaluate(inPage);
          graded++;
          for (const kind of Object.keys(found)) {
            for (const f of found[kind]) problems.push(`${key}  ${kind}: ${f}`);
          }
          process.stdout.write(`${Object.values(found).some((a) => a.length) ? '! ' : '. '}${key}\n`);
        } catch (err) {
          problems.push(`${key}  error: ${String(err).slice(0, 160)}`);
        }
      }
      await context.close();
    }
  }
  await browser.close();
  console.log(`\nGraded ${graded} page states.`);
  if (graded < MIN_ROUTES * THEMES.length * VPS.length * (ONLY ? 0 : 1)) {
    console.error(`layout: graded ${graded}, expected at least ${MIN_ROUTES * THEMES.length * VPS.length}; is the fixture up at ${BASE}?`);
    process.exitCode = 1;
  }
  if (problems.length) {
    console.error(`\n=== ${problems.length} layout finding(s) ===`);
    for (const p of problems) console.error('  ' + p);
    process.exitCode = 1;
  } else {
    console.log('layout: no overlaps, squeezed columns, escaping controls or sub-9px text.');
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
