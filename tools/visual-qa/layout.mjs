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
// Six checks, per route x theme x viewport:
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
//   widen     the document is wider than the viewport (scrollingElement.scrollWidth
//             > clientWidth), reported with the element that reaches furthest right
//             and is not clipped by a scrolling ancestor. A hint holding one 385px
//             identifier in a 292px box widened /station/alerts to 434px on a 390px
//             phone and took the fixed tab bar with it; qa.mjs counts the overflow
//             but names nothing, and none of the checks above looks at the page as a
//             whole. The tab bar and fixed elements are victims, never the cause.
//   svgtext   two SVG <text> labels whose turned boxes overlap by more than a
//             quarter of the shorter one's height (and 2px). The first four checks walk HTML text and skip every
//             <svg>, so a chart label written over its neighbour (the chord
//             diagram's Red-winged Blackbird across Black-capped Chickadee, on
//             the documentation's own picture) passed all of them. The boxes are
//             the labels' real ones: getBBox through the screen transform, so a
//             name written along a tilted arc is a tilted rectangle, not the
//             larger upright box that would flag every pair of neighbours.
//
// Env:
//   BASE      base url                       (default http://127.0.0.1:8502)
//   THEMES    csv of light,dark              (default light,dark)
//   VPS       csv of desktop,mobile          (default desktop,mobile)
//   ONLY      substring filter on route name
//   CSS_OVERRIDE  serve this file in place of /static/css/app.css (an older one, to
//             see a new check fail against the stylesheet it was written for)
//   MIN_ROUTES  fail if fewer routes were graded than this (default 30), so a run
//             that could not reach the fixture cannot pass by grading nothing
//
// Exits non-zero on any finding. Run from this directory:
//   BASE=http://127.0.0.1:8502 node layout.mjs
import { pathToFileURL } from 'node:url';
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

// Runs in the page. Returns { overlap, squeeze, escape, tiny, svgtext, widen } arrays of strings.
function inPage() {
  const res = { overlap: [], squeeze: [], escape: [], tiny: [], svgtext: [], widen: [] };
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

  // What of a text run is painted. `Range.getClientRects` reports the whole run
  // even where an `overflow: hidden` box cuts it off, so a species name too long
  // for its phone column (ellipsised, and nothing drawn past the box) measured as
  // running under the play button beside it. Clip each rect to every ancestor,
  // the text's own element included, that clips its overflow.
  const visible = (el, rects) => {
    let out = rects.map((r) => ({ left: r.left, right: r.right, top: r.top, bottom: r.bottom }));
    for (let n = el; n && n !== document.body; n = n.parentElement) {
      const cs = getComputedStyle(n);
      if (cs.overflowX === 'visible' && cs.overflowY === 'visible') continue;
      const c = n.getBoundingClientRect();
      out = out.map((r) => ({
        left: Math.max(r.left, cs.overflowX === 'visible' ? -Infinity : c.left),
        right: Math.min(r.right, cs.overflowX === 'visible' ? Infinity : c.right),
        top: Math.max(r.top, cs.overflowY === 'visible' ? -Infinity : c.top),
        bottom: Math.min(r.bottom, cs.overflowY === 'visible' ? Infinity : c.bottom),
      }));
    }
    return out
      .map((r) => ({ ...r, width: r.right - r.left, height: r.bottom - r.top }))
      .filter((r) => r.width > 0 && r.height > 0);
  };

  const items = [];
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    if (!n.nodeValue.trim()) continue;
    const el = n.parentElement;
    if (!el || !shown(el) || el.closest(SKIP)) continue;
    if (el.closest('details:not([open])') && !el.closest('summary')) continue;
    const rg = document.createRange();
    rg.selectNodeContents(n);
    const rects = visible(el, [...rg.getClientRects()]).filter((r) => r.width > 1 && r.height > 1);
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

  // svgtext: labels inside charts, as turned rectangles.
  const corners = (t) => {
    const m = t.getScreenCTM();
    const b = t.getBBox();
    if (!m || !(b.width > 1 && b.height > 1)) return null;
    return [
      [b.x, b.y],
      [b.x + b.width, b.y],
      [b.x + b.width, b.y + b.height],
      [b.x, b.y + b.height],
    ].map(([x, y]) => [m.a * x + m.c * y + m.e, m.b * x + m.d * y + m.f]);
  };
  // Smallest overlap, over the polygons' edge normals, of two convex quads; 0 if apart.
  const penetration = (pa, pb) => {
    let least = Infinity;
    for (const poly of [pa, pb]) {
      for (let i = 0; i < 4; i++) {
        const [x1, y1] = poly[i];
        const [x2, y2] = poly[(i + 1) % 4];
        const len = Math.hypot(x2 - x1, y2 - y1) || 1;
        const nx = -(y2 - y1) / len;
        const ny = (x2 - x1) / len;
        const span = (q) => {
          const d = q.map(([x, y]) => x * nx + y * ny);
          return [Math.min(...d), Math.max(...d)];
        };
        const [a0, a1] = span(pa);
        const [b0, b1] = span(pb);
        const o = Math.min(a1, b1) - Math.max(a0, b0);
        if (o <= 0) return 0;
        least = Math.min(least, o);
      }
    }
    return least;
  };
  for (const svg of document.querySelectorAll('svg')) {
    if (res.svgtext.length >= 10) break;
    if (!shown(svg) || svg.closest('[aria-hidden="true"], [hidden], details:not([open]), template, [class*="tabbar"]')) continue;
    const labels = [];
    for (const t of svg.querySelectorAll('text')) {
      if (t.closest('defs, symbol, clipPath, mask') || !(t.textContent || '').trim()) continue;
      const cs = getComputedStyle(t);
      if (cs.display === 'none' || cs.visibility === 'hidden' || cs.opacity === '0') continue;
      const q = corners(t);
      if (q) labels.push({ t, q, txt: t.textContent.trim().slice(0, 28) });
    }
    for (let i = 0; i < labels.length && res.svgtext.length < 10; i++) {
      for (let j = i + 1; j < labels.length; j++) {
        const a = labels[i];
        const b = labels[j];
        if (a.t.contains(b.t) || b.t.contains(a.t)) continue;
        // A label's box is its font's full ascent and descent, so two lines of
        // one caption overlap by a few pixels at ordinary leading. What reads as
        // a collision is a quarter of the shorter label's height, as in `overlap`.
        const height = (q) => Math.hypot(q[3][0] - q[0][0], q[3][1] - q[0][1]);
        const o = penetration(a.q, b.q);
        if (o > Math.max(2, 0.25 * Math.min(height(a.q), height(b.q)))) {
          res.svgtext.push(`${sel(svg)} "${a.txt}" x "${b.txt}" (${Math.round(o)}px)`);
        }
      }
    }
  }

  // widen: the page itself is wider than the screen.
  {
    const de = document.scrollingElement || document.documentElement;
    const vw = de.clientWidth;
    if (de.scrollWidth > vw + 1) {
      const reach = (el, r) => Math.max(r.right, el.scrollWidth > el.clientWidth && getComputedStyle(el).overflowX === 'visible' ? r.left + el.scrollWidth : 0);
      let worst = null;
      let depthOf = (el) => { let d = 0; for (let n = el; n; n = n.parentElement) d++; return d; };
      for (const el of document.body.querySelectorAll('*')) {
        if (el.closest('[class*="tabbar"], script, style, template, [hidden], details:not([open])')) continue;
        const r = el.getBoundingClientRect();
        if (r.width <= 0 || getComputedStyle(el).position === 'fixed') continue;
        const x = reach(el, r);
        if (x <= vw + 1) continue;
        // Sideways scroll inside a box that itself fits is the box's own business.
        let held = false;
        for (let n = el.parentElement; n && n !== document.body; n = n.parentElement) {
          const cs = getComputedStyle(n);
          if (cs.position === 'fixed') { held = true; break; }
          if (cs.overflowX !== 'visible' && n.getBoundingClientRect().right <= vw + 1) { held = true; break; }
        }
        if (held) continue;
        const d = depthOf(el);
        if (!worst || x > worst.x + 0.5 || (Math.abs(x - worst.x) <= 0.5 && d > worst.d)) worst = { el, x, d };
      }
      const text = worst ? ` ("${(worst.el.textContent || '').trim().slice(0, 36)}")` : '';
      res.widen.push(
        worst
          ? `document is ${de.scrollWidth}px wide in a ${vw}px viewport; ${sel(worst.el)} reaches ${Math.round(worst.x)}px${text}`
          : `document is ${de.scrollWidth}px wide in a ${vw}px viewport; no unclipped element reaches past it`,
      );
    }
  }

  for (const k of Object.keys(res)) res[k] = [...new Set(res[k])].slice(0, 10);
  return res;
}

async function main() {
  const browser = await chromium.launch(CHROMIUM_PATH ? { executablePath: CHROMIUM_PATH } : {});
  // CSS_OVERRIDE=old.css grades an earlier stylesheet on the running fixture (as in
  // tokens.mjs), to watch a check go red against the CSS it was written for.
  if (process.env.CSS_OVERRIDE) {
    const { readFileSync } = await import('node:fs');
    const body = readFileSync(process.env.CSS_OVERRIDE, 'utf8');
    const original = browser.newContext.bind(browser);
    browser.newContext = async (opts) => {
      const c = await original(opts);
      await c.route(/\/static\/css\/app\.css(\?.*)?$/, (r) => r.fulfill({ status: 200, contentType: 'text/css', body }));
      return c;
    };
  }
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
    console.log('layout: no overlaps, squeezed columns, escaping controls, sub-9px text, colliding chart labels or page wider than the screen.');
  }
}

export { inPage };

// Imported by layout-selftest.mjs for `inPage`; only a direct run grades routes.
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((e) => {
    console.error(e);
    process.exit(1);
  });
}
