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

const browser = await chromium.launch(CHROMIUM_PATH ? { executablePath: CHROMIUM_PATH } : {});
const failures = [];
let graded = 0;
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
  await ctx.close();
}
await browser.close();

if (failures.length) {
  console.log(`\nspecies-colours: ${failures.length} finding(s)`);
  for (const f of failures) console.log(`  FAIL ${f}`);
  process.exit(1);
}
console.log(`\nspecies-colours: ${graded} species graded across ${ROUTES.length} pages and ${THEMES.length} themes, each in one colour`);
