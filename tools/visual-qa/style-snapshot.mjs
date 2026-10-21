// Proof that a CSS refactor changed nothing the reader sees.
//
//   node style-snapshot.mjs snapshot <out.json>      against BASE (the fixture)
//   CSS_OVERRIDE=old.css node style-snapshot.mjs snapshot <out.json>
//   node style-snapshot.mjs diff <before.json> <after.json>
//
// CSS_OVERRIDE serves the given file in place of the page's /static/css/app.css
// (through the browser's request interception, so the server is untouched).
// That is what makes the comparison honest: the *same* server process, with the
// same seeded rows and the same clock, answers both runs, so the only thing that
// differs between the snapshots is the stylesheet. Two snapshots taken against
// two builds of the fixture also differ in whatever the fixture generates (a
// meter's fill, a page's row count), and a diff of those cannot say which
// differences are the refactor's.
//
// For every route in qa.mjs x light/dark x desktop/mobile it records, per
// element, the computed value of every property a token substitution can move
// (gaps, margins, paddings, borders, radii, weights, tracking, sizes, colours).
// `diff` compares two snapshots element by element and prints what moved.
//
// Why: a token pass replaces `gap: 10px` with `gap: var(--space-10)` in a few
// hundred places. Reading the diff of the stylesheet cannot show that every
// one still computes to ten pixels (a typo in a token's value, a rule that
// stopped matching, a `var()` inside `calc()` that no longer resolves). The
// computed styles can: identical before and after means the refactor is pure.
//
// What it cannot see: anything that is not in the property list, a state only
// reachable by interaction, and content that differs between two runs (the
// fixture is time-dependent; elements whose tag and class path differ between
// the two snapshots are reported as "structure changed", not as a style diff).
import { chromium } from 'playwright';
import fs from 'node:fs';
import { ROUTES } from './qa.mjs';

const BASE = process.env.BASE || 'http://127.0.0.1:8502';
const CHROMIUM_PATH = process.env.CHROMIUM_PATH || '';
const CSS_OVERRIDE = process.env.CSS_OVERRIDE || '';
const [mode, a, b] = process.argv.slice(2);

const PROPS = [
  'display', 'gap', 'rowGap', 'columnGap',
  'marginTop', 'marginRight', 'marginBottom', 'marginLeft',
  'paddingTop', 'paddingRight', 'paddingBottom', 'paddingLeft',
  'borderTopWidth', 'borderRightWidth', 'borderBottomWidth', 'borderLeftWidth', 'borderTopStyle',
  'borderTopColor', 'borderTopLeftRadius', 'borderTopRightRadius', 'borderBottomLeftRadius', 'borderBottomRightRadius',
  'fontFamily', 'fontSize', 'fontWeight', 'lineHeight', 'letterSpacing',
  'color', 'backgroundColor', 'boxShadow', 'width', 'height', 'opacity', 'transitionDuration',
  // The motion, layer and effect tokens: a token pass over z-index, easing and
  // blur moves these and none of the properties above.
  'zIndex', 'transitionTimingFunction', 'transitionDelay', 'animationDuration', 'animationTimingFunction',
  'backdropFilter', 'textShadow', 'outlineWidth', 'outlineOffset', 'minHeight', 'maxWidth', 'transform',
];

const VIEWS = [
  { vp: 'desktop', width: 1440, height: 900, theme: 'light' },
  { vp: 'desktop', width: 1440, height: 900, theme: 'dark' },
  { vp: 'mobile', width: 390, height: 844, theme: 'light', touch: true },
];

async function snapshot(out) {
  const browser = await chromium.launch(CHROMIUM_PATH ? { executablePath: CHROMIUM_PATH } : {});
  const snap = {};
  for (const v of VIEWS) {
    const ctx = await browser.newContext({
      viewport: { width: v.width, height: v.height },
      hasTouch: Boolean(v.touch),
      isMobile: Boolean(v.touch),
      colorScheme: v.theme,
      reducedMotion: 'reduce', // a still page: nothing mid-animation is sampled
      serviceWorkers: 'block', // the page's own cache must not answer for the override
    });
    if (CSS_OVERRIDE) {
      const body = fs.readFileSync(CSS_OVERRIDE, 'utf8');
      await ctx.route(/\/static\/css\/app\.css(\?.*)?$/, (route) => route.fulfill({ status: 200, contentType: 'text/css', body }));
    }
    await ctx.addInitScript((t) => { try { localStorage.setItem('theme', t); } catch (e) {} }, v.theme);
    const page = await ctx.newPage();
    for (const [name, route] of ROUTES) {
      try {
        await page.goto(BASE + route, { waitUntil: 'domcontentloaded', timeout: 25000 });
        await page.waitForLoadState('networkidle', { timeout: 9000 }).catch(() => {});
        await page.waitForTimeout(900);
        snap[`${name}__${v.theme}__${v.vp}`] = await page.evaluate((props) => {
          const out = [];
          for (const el of document.body.querySelectorAll('*')) {
            if (el.closest('svg') && el.tagName !== 'svg') continue; // marks inside charts: presentation attributes
            const cs = getComputedStyle(el);
            out.push([
              el.tagName.toLowerCase() + (el.className && typeof el.className === 'string' ? '.' + el.className.trim().split(/\s+/).join('.') : ''),
              props.map((p) => cs[p]).join('|'),
            ]);
          }
          return out;
        }, PROPS);
      } catch (e) {
        snap[`${name}__${v.theme}__${v.vp}`] = { error: String(e).slice(0, 120) };
      }
    }
    await ctx.close();
    process.stderr.write(`snapshotted ${v.theme} ${v.vp}\n`);
  }
  await browser.close();
  fs.writeFileSync(out, JSON.stringify({ props: PROPS, snap }));
  console.log(`wrote ${out}: ${Object.keys(snap).length} pages`);
}

// Elements whose state is not the stylesheet's doing: an <img> is `display:none`
// after a failed photo load, the footer prints how long the station has been up.
const NOISE = /^(img|svg)\b|bnb-footer__|\bfresh\b/;
// Elements whose *width* is a live reading, not a style: a meter's fill is the
// CPU, memory or disk figure at the moment of the snapshot, and a counter's box
// is as wide as its digits. Their other properties are still compared.
const LIVE_WIDTH = /meter|^span#\d+$|^span\.mono#\d+$/;

// An element's key is its tag.class path plus how many identical ones came
// before it, so an extra row of live data cannot shift every later element out
// of alignment (it did, when this compared by position, and reported a hundred
// moved elements that were one feed row).
function keyed(page) {
  const seen = new Map();
  const out = new Map();
  for (const [name, props] of page) {
    const n = (seen.get(name) || 0) + 1;
    seen.set(name, n);
    out.set(`${name}#${n}`, props);
  }
  return out;
}

function diff(fa, fb) {
  const A = JSON.parse(fs.readFileSync(fa, 'utf8'));
  const B = JSON.parse(fs.readFileSync(fb, 'utf8'));
  let pages = 0, same = 0, elementsCompared = 0, moved = 0, onlyOne = 0, liveReadings = 0;
  const report = [];
  for (const key of Object.keys(A.snap)) {
    const pa = A.snap[key], pb = B.snap[key];
    if (!pb || pa.error || pb.error) { report.push(`${key}: not comparable (${pa.error || pb?.error || 'missing'})`); continue; }
    pages++;
    const ka = keyed(pa), kb = keyed(pb);
    let pageMoved = 0;
    for (const [k, va] of ka) {
      if (NOISE.test(k)) continue;
      const vb = kb.get(k);
      if (vb === undefined) { onlyOne++; continue; }
      elementsCompared++;
      if (va !== vb) {
        const xa = va.split('|'), xb = vb.split('|');
        const live = LIVE_WIDTH.test(k);
        const diffs = A.props.map((p, j) => (xa[j] !== xb[j] && !(live && p === 'width') ? `${p}: ${xa[j]} -> ${xb[j]}` : null)).filter(Boolean);
        if (!diffs.length) { liveReadings++; continue; }
        moved++; pageMoved++;
        if (pageMoved <= 3) report.push(`${key} ${k}: ${diffs.join('; ')}`);
      }
    }
    for (const k of kb.keys()) if (!ka.has(k) && !NOISE.test(k)) onlyOne++;
    if (!pageMoved) same++;
  }
  console.log(`${pages} pages comparable, ${same} identical, ${elementsCompared} elements compared, ${moved} elements moved, ${liveReadings} live readings differ (meter fills, counters), ${onlyOne} elements on one side only (live data)`);
  if (report.length) console.log(report.slice(0, 60).join('\n'));
  process.exitCode = moved ? 1 : 0;
}

if (mode === 'snapshot') await snapshot(a);
else if (mode === 'diff') diff(a, b);
else { console.error('usage: style-snapshot.mjs snapshot <out> | diff <a> <b>'); process.exit(2); }
