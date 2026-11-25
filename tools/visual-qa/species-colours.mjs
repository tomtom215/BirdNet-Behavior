// Gate: one species, one colour, on every page (UIR-01).
//
// A species' chart colour used to be its rank within each chart (`kit::slot_class(i)`), so the
// Robin was green on Today's day strip and orange on the dawn rings, and a reader who had learnt
// "the green one is the Robin" read the next chart wrong. The station now gives its eight most
// heard species one colour each (`kit::SpeciesSlots`), and every other species the neutral
// `--chart-other`.
//
// This visits the pages that colour by species, reads every key entry it can name (a swatch
// beside a species name, a chord arc and the name in its reading, a ladder dot and its name, the
// Today's songs and seasons keys, the voices of the assembling chorus), and reads the colour the
// browser paints for it. It fails when
//
//   * one species is painted in two colours anywhere across the pages (the defect), or
//   * two species share a colour that is not the neutral (the counterpart: a discriminator, not
//     a blanket "everything is grey"), or
//   * a page it visits contributes no named swatch at all, or fewer than two species carry a
//     colour that is not the neutral (it would be grading nothing).
//
// The second half (N25-06) grades the marks that are NOT a species: the temperature line under
// Today's hours, the hours around sunrise and sunset on Patterns, the migration ridgeline's
// seasons, a trend's line, the life list's curve and a species page's one-series bars. Each is
// named by a selector below, and each fails when the browser paints it in a colour a species
// owns (any of --chart-1 .. --chart-8, or within 6 OKLab units of one), because "the green one
// is the Robin" is then false on that figure. A selector that matches nothing on its page fails
// too, so a renamed class cannot turn the check into a pass.
//
// Run against the fixture server (see .github/workflows/a11y.yml for how to start it):
//   BASE=http://127.0.0.1:8502 CHROMIUM_PATH=... node species-colours.mjs
// THEMES=light,dark (default both).
import { chromium } from 'playwright';

const BASE = process.env.BASE || 'http://127.0.0.1:8502';
const CHROMIUM_PATH = process.env.CHROMIUM_PATH || '';
const THEMES = (process.env.THEMES || 'light,dark').split(',').filter(Boolean);

// The pages, and the figures on them that colour by species.
const ROUTES = [
  ['/', 'the day strip, the songs of the day'],
  ['/patterns', 'the streamgraph'],
  ['/patterns?tab=dawn', 'the dawn rings, each bird\'s day'],
  ['/patterns?tab=together', 'the chord, the ladder'],
  ['/patterns?tab=behavior', 'the bursts timeline'],
  ['/patterns?tab=sequences', 'the running order, the assembling chorus'],
  ['/reports?tab=year', 'who led each month'],
  ['/species', 'the species list\'s sparklines'],
];

// The marks that are not a species, where they are drawn. Each selector must match on its page.
async function speciesDetail() {
  try {
    const r = await fetch(`${BASE}/api/v2/detections?limit=1`);
    const d = (await r.json()).detections[0];
    return `/species/detail?name=${encodeURIComponent(d.com_name)}`;
  } catch (e) {
    throw new Error(`species-colours: no detection to name a species page at ${BASE} (${e})`);
  }
}
const NOT_SPECIES = [
  ['/', 'svg[data-ck-id^="today-hourly"] path.ck-line', 'the temperature line under Today\'s hours'],
  ['/patterns', 'svg[data-ck-id="hourly-totals"] path.ck-grow:not(.ck-ov)', 'the hour bars on Patterns'],
  ['/patterns?tab=migration', 'g.rd-ridge path.rd-line, path.rd-bar', 'the ridgeline\'s seasons'],
  ['/patterns?tab=trends', 'svg path.ck-line', 'a trend\'s line'],
  ['/species?view=lifelist', 'svg path.ck-line, svg circle.ck-ring', 'the life list\'s curve'],
  [await speciesDetail(), 'svg[data-ck-id^="species-"] path.ck-grow:not(.ck-ov)', 'a species page\'s one-series bars'],
];

/** Load everything a reader would: scroll through, open the closed figures, wait for htmx. */
async function settle(page) {
  await page.waitForLoadState('networkidle', { timeout: 20000 }).catch(() => {});
  for (let pass = 0; pass < 3; pass++) {
    await page.evaluate(async () => {
      for (const d of document.querySelectorAll('details:not([open])')) {
        if (d.closest('nav, header, .cmdk')) continue;
        d.open = true;
        d.dispatchEvent(new Event('toggle'));
      }
      const step = Math.max(200, Math.floor(window.innerHeight * 0.8));
      for (let y = 0; y < document.body.scrollHeight; y += step) {
        window.scrollTo(0, y);
        await new Promise((r) => setTimeout(r, 60));
      }
      window.scrollTo(0, 0);
    });
    await page.waitForLoadState('networkidle', { timeout: 20000 }).catch(() => {});
    await page.waitForTimeout(400);
  }
}

/** Every (species name, painted colour, where) the page shows. */
async function readKeys(page) {
  return page.evaluate(() => {
    const out = [];
    const clean = (s) => (s || '').replace(/\s+/g, ' ').trim();
    const paint = (el) => {
      const cs = getComputedStyle(el);
      const c = cs.getPropertyValue('--c').trim();
      if (c) return c;
      if (el.matches('path.line')) return cs.stroke;
      if (cs.fill && cs.fill !== 'none') return cs.fill;
      return cs.backgroundColor;
    };
    // Resolve every colour through one probe so two spellings of one colour compare equal.
    const probe = document.createElement('i');
    document.body.appendChild(probe);
    const norm = (c) => {
      probe.style.color = '';
      probe.style.color = c;
      return getComputedStyle(probe).color;
    };
    const push = (name, el, where) => {
      name = clean(name);
      // "Other species", "52 other species": the band of everyone else, the neutral by design.
      if (!name || /other species$/i.test(name) || /^show all$/i.test(name)) return;
      out.push({ name, colour: norm(paint(el)), where });
    };
    // A swatch beside a name: the legends of the day strip, the streamgraph, the dawn rings,
    // the bursts, the terrain.
    for (const sw of document.querySelectorAll('.ck-sw[class*="ck-c"]')) {
      const item = sw.closest('.viz-legend-item, .ck-leg, th[scope="row"], li');
      if (!item) continue;
      const nameEl = item.querySelector('.dc-leg-name');
      let name = nameEl ? nameEl.textContent : item.textContent;
      if (!nameEl && item.matches('th')) name = item.textContent;
      push(name, sw, 'legend');
    }
    // Each bird's day: the square beside each name.
    for (const r of document.querySelectorAll('rect.ck-fill[class*="ck-c"]')) {
      const t = r.nextElementSibling;
      if (t && t.matches('text.dc-strip-name')) push(t.textContent, r, 'day strips');
    }
    // The chord: an arc's reading opens with its species.
    for (const a of document.querySelectorAll('path.chord-arc')) {
      push((a.getAttribute('data-tip') || '').split('\n')[0], a, 'chord');
    }
    // The ladder: a dot, then its name.
    for (const c of document.querySelectorAll('circle.ck-fill[class*="ck-c"]')) {
      const t = c.nextElementSibling;
      if (t && t.matches('text.ladder-name')) push(t.textContent, c, 'ladder');
    }
    // The assembling chorus: the voices' key.
    for (const chip of document.querySelectorAll('.as-order .as-chip')) {
      const li = chip.closest('li');
      const n = li && li.querySelector('.as-n');
      push(clean(li.textContent).replace(clean(n ? n.textContent : ''), ''), chip, 'chorus voices');
    }
    // The running order: one square and one name a lane, in lane order.
    for (const svg of document.querySelectorAll('svg.mo-svg')) {
      const squares = svg.querySelectorAll('rect.ck-fill[class*="ck-c"]');
      const names = svg.querySelectorAll('text.mo-name');
      if (squares.length === names.length) squares.forEach((r, i) => push(names[i].textContent, r, 'running order'));
    }
    // Sparklines beside a species' name (Today's top species, the species lists): the line's
    // stroke is the species' colour on the station.
    for (const sp of document.querySelectorAll('svg.bnb-spark')) {
      const row = sp.closest('a.x-top, tr, li');
      const line = sp.querySelector('.line');
      if (!row || !line) continue;
      // The name itself: a row's link also holds the avatar's code and the scientific name.
      const nameEl = row.querySelector('.t, .sp-nm, .dp-link');
      if (nameEl) push(nameEl.textContent, line, 'sparkline');
    }
    // Today's songs: the key's chips.
    for (const sw of document.querySelectorAll('.ss-chip .ss-sw')) {
      const n = sw.parentElement.querySelector('.ss-chip-name');
      if (n) push(n.textContent, sw, 'songs of the day');
    }
    // Who led each month: the rows of the key.
    for (const row of document.querySelectorAll('.ssn-row')) {
      const n = row.querySelector('.ssn-nm');
      const bar = row.querySelector('.ssn-bar') || row;
      if (n) push(n.textContent, bar, 'seasons');
    }
    probe.remove();
    const other = (() => {
      const p = document.createElement('i');
      p.style.color = 'var(--chart-other)';
      document.body.appendChild(p);
      const c = getComputedStyle(p).color;
      p.remove();
      return c;
    })();
    return { keys: out, other };
  });
}

/**
 * For every element `selector` matches, the colour it is painted in (a gradient's own colour for a
 * `url(#…)` fill), and those that are a species' colour: equal to `--chart-k`, or within 6 OKLab
 * units of it once both are drawn on a pixel (a near-copy reads as the same bird).
 */
async function readNonSpecies(page, selector) {
  return page.evaluate((sel) => {
    const cv = document.createElement('canvas');
    cv.width = cv.height = 1;
    const cx = cv.getContext('2d', { willReadFrequently: true });
    const lin = (c) => { c /= 255; return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4; };
    const lab = (css) => {
      cx.clearRect(0, 0, 1, 1);
      cx.fillStyle = '#000';
      cx.fillStyle = css;
      cx.fillRect(0, 0, 1, 1);
      const [r, g, b] = [...cx.getImageData(0, 0, 1, 1).data].map(lin);
      const l = Math.cbrt(0.4122214708 * r + 0.5363137492 * g + 0.0514459244 * b);
      const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
      const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
      return [0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s];
    };
    const probe = document.createElement('i');
    document.body.appendChild(probe);
    const resolve = (c) => { probe.style.color = ''; probe.style.color = c; return getComputedStyle(probe).color; };
    const owned = [];
    for (let k = 1; k <= 8; k++) owned.push(resolve(`var(--chart-${k})`));
    const paintOf = (el) => {
      const cs = getComputedStyle(el);
      const fill = cs.fill || '';
      const url = fill.match(/url\("?#([^")]+)"?\)/);
      if (url) {
        const grad = document.getElementById(url[1]);
        if (!grad) return '';
        const c = getComputedStyle(grad).getPropertyValue('--c').trim();
        if (c) return resolve(c);
        const stop = grad.querySelector('stop');
        return stop ? resolve(getComputedStyle(stop).stopColor) : '';
      }
      if (el.matches('path.ck-line, path.rd-line, path.line')) return resolve(cs.stroke);
      if (fill && fill !== 'none') return resolve(fill);
      return resolve(cs.stroke);
    };
    let marks = 0;
    const bad = [];
    for (const el of document.querySelectorAll(sel)) {
      const colour = paintOf(el);
      if (!colour) continue;
      marks++;
      const a = lab(colour);
      owned.forEach((o, i) => {
        const b = lab(o);
        const de = Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2]) * 100;
        if (colour === o || de < 6) {
          const where = el.tagName.toLowerCase() + '.' + [...el.classList].join('.');
          if (!bad.some((x) => x.where === where && x.slot === i + 1)) bad.push({ where, colour, slot: i + 1, de: de.toFixed(1) });
        }
      });
    }
    probe.remove();
    return { marks, owned: bad };
  }, selector);
}

const browser = await chromium.launch(CHROMIUM_PATH ? { executablePath: CHROMIUM_PATH } : {});
const failures = [];
let graded = 0;
let nonSpecies = 0;
for (const theme of THEMES) {
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 900 }, colorScheme: theme });
  await ctx.addInitScript((t) => {
    try { localStorage.setItem('theme', t); } catch { /* private window */ }
  }, theme);
  const page = await ctx.newPage();
  const seen = new Map(); // name -> Map(colour -> [where])
  let other = '';
  for (const [route, what] of ROUTES) {
    const resp = await page.goto(BASE + route, { waitUntil: 'domcontentloaded', timeout: 30000 });
    if (!resp || !resp.ok()) {
      failures.push(`${theme} ${route}: answered ${resp ? resp.status() : 'nothing'}`);
      continue;
    }
    await settle(page);
    const { keys, other: o } = await readKeys(page);
    other = o;
    if (!keys.length) {
      failures.push(`${theme} ${route}: no named species swatch was found (${what}); the gate grades nothing here`);
    }
    for (const k of keys) {
      if (!seen.has(k.name)) seen.set(k.name, new Map());
      const m = seen.get(k.name);
      if (!m.has(k.colour)) m.set(k.colour, []);
      m.get(k.colour).push(`${route} (${k.where})`);
    }
    console.log(`  ${theme} ${route}: ${keys.length} named swatches`);
  }
  // The defect: one species in two colours.
  for (const [name, colours] of seen) {
    if (colours.size > 1) {
      const list = [...colours].map(([c, w]) => `${c} on ${[...new Set(w)].join(', ')}`).join('; ');
      failures.push(`${theme}: ${name} is painted in ${colours.size} colours: ${list}`);
    }
  }
  // The counterpart: two species never share an owned colour.
  const owner = new Map();
  for (const [name, colours] of seen) {
    for (const c of colours.keys()) {
      if (c === other) continue;
      if (owner.has(c) && owner.get(c) !== name) {
        failures.push(`${theme}: ${owner.get(c)} and ${name} share the colour ${c}, which is not the neutral`);
      } else owner.set(c, name);
    }
  }
  if (owner.size < 2) {
    failures.push(`${theme}: fewer than two species carry a colour of their own (${owner.size}); the gate grades nothing`);
  }
  graded += seen.size;
  console.log(`  ${theme}: ${seen.size} species named, ${owner.size} with a colour of their own`);

  // The marks that are not a species never wear a species' colour (N25-06).
  for (const [route, selector, what] of NOT_SPECIES) {
    const resp = await page.goto(BASE + route, { waitUntil: 'domcontentloaded', timeout: 30000 });
    if (!resp || !resp.ok()) {
      failures.push(`${theme} ${route}: answered ${resp ? resp.status() : 'nothing'}`);
      continue;
    }
    await settle(page);
    const got = await readNonSpecies(page, selector);
    if (!got.marks) {
      failures.push(`${theme} ${route}: "${selector}" matched nothing (${what}); the check grades nothing here`);
      continue;
    }
    for (const bad of got.owned) {
      failures.push(`${theme} ${route}: ${what} (${bad.where}) is painted ${bad.colour}, within ${bad.de} of --chart-${bad.slot}, a colour a species owns`);
    }
    nonSpecies += got.marks;
    console.log(`  ${theme} ${route}: ${got.marks} marks that are not a species, ${got.owned.length} in an owned colour`);
  }
  await ctx.close();
}
await browser.close();

if (failures.length) {
  console.log(`\nspecies-colours: ${failures.length} finding(s)`);
  for (const f of failures) console.log(`  FAIL ${f}`);
  process.exit(1);
}
console.log(`\nspecies-colours: ${graded} species graded across ${ROUTES.length} pages and ${THEMES.length} themes, each in one colour; ${nonSpecies} marks that are not a species, none in a species' colour`);
