// Behavioural gate for the relational, radial and behavioural Patterns charts: the chord
// diagram and its phone ladder ("Who sings together"), the dawn-chorus rings, the migration
// ridgeline and the Behavior tab's four pictures.
//
// What each of them promises is something a reader does to it. A screenshot shows the chord at
// rest; it cannot show that pointing at a species leaves only its ribbons, that the phone gets a
// picture it can read, that a ring of the dawn chart answers a finger, or that the ridgeline's
// "today" is hatched ahead of itself. Nor can it show the thing that was wrong with the Migration
// tab's "Earliest vs last year -356 d": a number.
//
//   chord        pointing at a species leaves exactly the ribbons that end on it (and the species they
//                end on) in the foreground and the rest at --chart-dim; pointing at a ribbon leaves
//                it and its two species; the keyboard reads species then links; "Links shown"
//                changes the number of ribbons and says which is pressed; nothing is written over
//                the ribbons; a phone gets the ladder and no ring, with no sideways scroll
//   dawn rings   every ring-hour reads (name, hour span, count, share, busiest hour); a ring comes
//                forward from the pointer, from its key and from the keyboard (Up and Down move from
//                ring to ring); the centre says the time now and no "chorus 24 h"; the now hand is a
//                solid line
//   ridgeline    a column reads every species heard that week with its real count and how many kinds
//                were heard; the weeks that have not happened are hatched; the colour of a ridge is
//                a season, three kinds at most
//   migration    the Earliest-vs-last-year tile does not claim a number the station cannot know (the
//                fixture's station began on 26 October of last year)
//   behavior     the bursts timeline draws bars and its legend pins a species; the retention ledger
//                groups species by class; the neighbour flow is drawn at desktop width and ranked
//                lists at a phone's; the running order draws a dot per morning and one morning
//                comes forward from the pointer
//
// Run against the fixture server:
//   BASE=http://127.0.0.1:8502 CHROMIUM_PATH=... node relational.mjs
//
// Self-test (each must go red; this is how the gate was seen failing):
//   MUTATE=nofocus   chart-kit.js never applies a mark's data-ck-f focus
//   MUTATE=allthere  the stylesheet shows the ring on a phone as well as the ladder
//   MUTATE=nodim     the stylesheet stops dimming
import { chromium } from 'playwright';

const BASE = process.env.BASE || 'http://127.0.0.1:8502';
const CHROMIUM_PATH = process.env.CHROMIUM_PATH || '';
const MUTATE = process.env.MUTATE || '';
const failures = [];

function check(name, ok, detail = '') {
  if (ok) console.log(`  \x1b[32mPASS\x1b[0m ${name}`);
  else {
    failures.push({ name, detail });
    console.log(`  \x1b[31mFAIL\x1b[0m ${name}\n       ${detail}`);
  }
}

const browser = await chromium.launch(CHROMIUM_PATH ? { executablePath: CHROMIUM_PATH } : {});

async function open(route, { width = 1440, theme = 'light', touch = false } = {}) {
  const context = await browser.newContext({ viewport: { width, height: 1000 }, colorScheme: theme, hasTouch: touch, reducedMotion: 'reduce' });
  await context.addInitScript((t) => {
    try {
      localStorage.setItem('theme', t);
    } catch (e) { /* storage blocked */ }
  }, theme);
  if (MUTATE === 'nofocus') {
    await context.route('**/static/chart-kit.js*', async (r) => {
      const res = await r.fetch();
      const body = (await res.text()).replace('if (hasF) applyTokens(svg.querySelector', 'if (false) applyTokens(svg.querySelector');
      await r.fulfill({ response: res, body });
    });
  }
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', (e) => errors.push(e.message));
  await page.goto(BASE + route, { waitUntil: 'domcontentloaded', timeout: 30000 });
  await page.waitForLoadState('networkidle', { timeout: 12000 }).catch(() => {});
  if (MUTATE === 'allthere') await page.evaluate(() => document.styleSheets[0].insertRule('.co-chord { display: block !important; }', document.styleSheets[0].cssRules.length));
  if (MUTATE === 'nodim') await page.evaluate(() => document.styleSheets[0].insertRule('svg.ck-focus [data-ck-s] { opacity: 1 !important; }', document.styleSheets[0].cssRules.length));
  return { context, page, errors };
}

const opacities = (page, sel) => page.$$eval(sel, (els) => els.map((e) => Number(getComputedStyle(e).opacity)));
const dim = (page) => page.evaluate(() => Number(getComputedStyle(document.documentElement).getPropertyValue('--chart-dim')) || 0.35);

// ── chord ───────────────────────────────────────────────────────────────────
console.log('who sings together: the chord at desktop width');
{
  const { context, page, errors } = await open('/patterns?tab=together');
  await page.waitForSelector('.viz-svg-chord', { timeout: 15000 });
  const d = await dim(page);
  const arcs = await page.$$('.viz-svg-chord .chord-arc');
  const ribbons = await page.$$('.viz-svg-chord .chord-ribbon');
  check('the chord has arcs and ribbons', arcs.length >= 3 && ribbons.length >= 3, `${arcs.length} arcs ${ribbons.length} ribbons`);
  const written = await page.$$eval('.viz-svg-chord text', (els) => els.map((e) => e.textContent.trim()));
  const noCaption = !written.some((t) => /co-occurrence|5-minute|five-minute/i.test(t));
  check('nothing is written over the ribbons (no centre caption)', noCaption, written.join(' | '));

  // Point at the busiest species' arc.
  const tokens = await page.$$eval('.viz-svg-chord .chord-arc', (els) => els.map((e) => e.getAttribute('data-ck-s').split(' ')));
  const ribbonTokens = await page.$$eval('.viz-svg-chord .chord-ribbon', (els) => els.map((e) => e.getAttribute('data-ck-s').split(' ')));
  const first = tokens[0][0];
  const touching = ribbonTokens.filter((t) => t.includes(first)).length;
  // The middle of the arc's own path, not of its bounding box (a curved band's box centre is
  // over the ribbons, which would read a ribbon).
  await arcs[0].scrollIntoViewIfNeeded();
  const on = await arcs[0].evaluate((p) => {
    const m = p.getScreenCTM();
    const len = p.getTotalLength();
    for (let f = 0.02; f < 1; f += 0.02) {
      const pt = p.getPointAtLength(len * f);
      const x = pt.x * m.a + pt.y * m.c + m.e;
      const y = pt.x * m.b + pt.y * m.d + m.f;
      for (const [dx, dy] of [[3, 0], [-3, 0], [0, 3], [0, -3], [3, 3], [-3, -3], [3, -3], [-3, 3]]) {
        if (document.elementFromPoint(x + dx, y + dy) === p) return { x: x + dx, y: y + dy };
      }
    }
    return { x: 5, y: 5 };
  });
  await page.mouse.move(on.x, on.y);
  await page.waitForTimeout(150);
  const ribOp = await opacities(page, '.viz-svg-chord .chord-ribbon');
  const lit = ribOp.filter((o) => o > 0.99).length;
  const faded = ribOp.filter((o) => Math.abs(o - d) < 0.02).length;
  check('pointing at a species leaves exactly the ribbons that end on it', lit === touching && faded === ribbons.length - touching && touching > 0, `lit ${lit} faded ${faded} touching ${touching} of ${ribbons.length}`);
  const tip = await page.locator('.co-chord .ck-tip.on').textContent().catch(() => '');
  check('and reads it (name, partners, share)', /%/.test(tip) && tip.length > 8, tip);
  await page.mouse.move(5, 5);
  await page.waitForTimeout(150);
  const rest = await opacities(page, '.viz-svg-chord .chord-ribbon');
  check('moving away brings every ribbon back', rest.every((o) => o > 0.99), JSON.stringify(rest.slice(0, 4)));

  // Point at a ribbon.
  const rb = await ribbons[Math.floor(ribbons.length / 2)].evaluate((e) => {
    // The middle of the ribbon's bounding box can be empty ground; use a point of its own path.
    const len = e.getTotalLength();
    const p = e.getPointAtLength(len * 0.12);
    const m = e.getScreenCTM();
    return { x: m.a * p.x + m.c * p.y + m.e, y: m.b * p.x + m.d * p.y + m.f };
  });
  await page.mouse.move(rb.x, rb.y);
  await page.waitForTimeout(150);
  const one = await opacities(page, '.viz-svg-chord .chord-ribbon');
  check('pointing at a ribbon leaves that ribbon in the foreground', one.filter((o) => o > 0.99).length >= 1 && one.filter((o) => Math.abs(o - d) < 0.02).length >= ribbons.length - 2, JSON.stringify(one.map((o) => +o.toFixed(2))));
  const arcOp = await opacities(page, '.viz-svg-chord .chord-arc');
  check('and the two species at its ends', arcOp.filter((o) => o > 0.99).length === 2, JSON.stringify(arcOp.map((o) => +o.toFixed(2))));
  await page.mouse.move(5, 5);

  // Keyboard.
  await page.locator('.viz-svg-chord').focus();
  await page.keyboard.press('ArrowRight');
  const live = await page.locator('.co-chord .ck-live').textContent();
  check('the keyboard reads species first (aria-live)', live.length > 10 && /%/.test(live), live);
  const names = await page.$$eval('.viz-svg-chord .chord-name', (e) => e.length);
  for (let i = 0; i < names + 1; i++) await page.keyboard.press('ArrowRight');
  const live2 = await page.locator('.co-chord .ck-live').textContent();
  check('and then the links', /×/.test(live2), live2);

  // Links shown.
  const control = page.locator('.co-controls');
  if (await control.count()) {
    const before = (await page.$$('.viz-svg-chord .chord-ribbon')).length;
    await page.locator('.co-controls button', { hasText: 'All' }).click();
    await page.waitForFunction((n) => document.querySelectorAll('.viz-svg-chord .chord-ribbon').length > n, before, { timeout: 10000 });
    const pressed = await page.$eval('.co-controls button[aria-pressed="true"]', (b) => b.textContent.trim());
    check('"Links shown: All" draws more ribbons and says it is pressed', pressed === 'All', pressed);
  } else {
    console.log('  (this fixture has few enough links that no control is offered)');
  }

  // Theme.
  const fillLight = await page.$eval('.viz-svg-chord .chord-ribbon', (e) => getComputedStyle(e).fill);
  await page.evaluate(() => document.documentElement.setAttribute('data-theme', 'dark'));
  const fillDark = await page.$eval('.viz-svg-chord .chord-ribbon', (e) => getComputedStyle(e).fill);
  check('a theme switch repaints the ribbons (no reload)', fillLight !== fillDark, `${fillLight} ${fillDark}`);
  check('no script error', errors.length === 0, errors.join(' | '));
  await context.close();
}

console.log('\nwho sings together: a phone');
{
  const { context, page, errors } = await open('/patterns?tab=together', { width: 390, touch: true });
  await page.waitForSelector('.co-ladder svg', { timeout: 15000 });
  const r = await page.evaluate(() => ({
    ring: getComputedStyle(document.querySelector('.co-chord')).display,
    ladder: getComputedStyle(document.querySelector('.co-ladder')).display,
    over: document.documentElement.scrollWidth - window.innerWidth,
    cells: document.querySelectorAll('.co-ladder rect[data-tip]').length,
    rank: document.querySelectorAll('.co-rank-row').length,
  }));
  check('a phone gets the ladder and the ranked pairs, and no ring', r.ring === 'none' && r.ladder !== 'none' && r.cells > 5 && r.rank > 2, JSON.stringify(r));
  check('with no sideways scroll', r.over <= 0, `${r.over}px`);
  const fs = await page.$eval('.co-ladder .ladder-name', (e) => {
    const svg = e.closest('svg');
    const scale = svg.getScreenCTM().a;
    return parseFloat(getComputedStyle(e).fontSize) * scale;
  });
  check('the ladder\'s names are legible on a phone (>= 8.5 px)', fs >= 8.5, `${fs.toFixed(1)} px`);
  const cell = await page.$('.co-ladder rect[data-tip]');
  const b = await cell.boundingBox();
  await page.touchscreen.tap(b.x + b.width / 2, b.y + b.height / 2);
  await page.waitForTimeout(150);
  const tip = await page.locator('.co-ladder .ck-tip.on').textContent().catch(() => '');
  check('a tap on a cell reads the pair', tip.includes('×') && /%/.test(tip), tip);
  check('no script error on a phone', errors.length === 0, errors.join(' | '));
  await context.close();
}

// ── dawn rings ──────────────────────────────────────────────────────────────
console.log('\nthe dawn chorus rings');
{
  const { context, page, errors } = await open('/patterns?tab=dawn');
  await page.waitForSelector('.dc-polar-svg .dc-ring', { timeout: 15000 });
  const d = await dim(page);
  const rings = await page.$$eval('.dc-polar-svg .dc-ring', (e) => e.length);
  const wedges = await page.$$eval('.dc-polar-svg .ck-hit', (e) => e.length);
  check('every ring has a mark for each of the 24 hours', wedges === rings * 24 && rings >= 3, `${rings} rings, ${wedges} marks`);
  const centre = await page.$eval('.dc-polar-svg', (s) => s.textContent);
  check('the centre says the time now and the window, not "chorus 24 h"', /NOW/.test(centre) && /60 days/.test(centre) && !/chorus/i.test(centre.replace(/Dawn chorus by species/g, '')) && !/24 h/.test(centre), centre.slice(0, 200));
  const hand = await page.$eval('.dc-now', (l) => ({ dash: l.getAttribute('stroke-dasharray'), sw: getComputedStyle(l).strokeDasharray }));
  check('the now hand is a solid line, not dashes', !hand.dash && (hand.sw === 'none' || hand.sw === ''), JSON.stringify(hand));
  // A wedge of the outermost ring, at its busiest hour.
  const start = await page.$eval('.dc-polar-svg', (s) => Number(s.getAttribute('data-ck-start')));
  const wedge = (await page.$$('.dc-polar-svg .ck-hit'))[start];
  const wb = await wedge.boundingBox();
  await page.mouse.move(wb.x + wb.width / 2, wb.y + wb.height / 2);
  await page.waitForTimeout(150);
  const tip = await page.locator('.dc-polar-col .ck-tip.on').textContent().catch(() => '');
  check('pointing at a ring-hour reads its name, hours, count, share and busiest hour', /\d\d:\d\d to \d\d:\d\d/.test(tip) && /share of the day/i.test(tip) && /busiest hour/i.test(tip), tip);
  const ringOp = await opacities(page, '.dc-polar-svg .dc-ring');
  check('and brings that ring forward, the others back', ringOp.filter((o) => o > 0.99).length === 1 && ringOp.filter((o) => Math.abs(o - d) < 0.02).length === rings - 1, JSON.stringify(ringOp.map((o) => +o.toFixed(2))));
  await page.mouse.move(5, 5);
  // The key.
  const leg = page.locator('.dc-leg').nth(1);
  await leg.hover();
  await page.waitForTimeout(150);
  const viaKey = await opacities(page, '.dc-polar-svg .dc-ring');
  check('hovering a name in the key brings its ring forward', viaKey[1] > 0.99 && viaKey[0] < 0.5, JSON.stringify(viaKey.map((o) => +o.toFixed(2))));
  await page.mouse.move(5, 5);
  // Keyboard: Up and Down move between rings.
  await page.locator('.dc-polar-svg').focus();
  await page.keyboard.press('ArrowRight');
  const l1 = await page.locator('.dc-polar-col .ck-live').textContent();
  await page.keyboard.press('ArrowDown');
  const l2 = await page.locator('.dc-polar-col .ck-live').textContent();
  const nameOf = (t) => t.split(':')[0].split(',')[0];
  check('Down moves to the next ring (a different species is read)', l1 !== l2 && nameOf(l1) !== nameOf(l2), `${l1} | ${l2}`);
  // The table.
  const table = await page.$$eval('.dc-table tbody tr', (r) => r.length).catch(() => 0);
  await page.waitForSelector('#dawn-list svg', { timeout: 10000 });
  const strips = await page.$$eval('#dawn-list .dc-strips-svg .ck-hit', (e) => e.length);
  check('"Each bird\'s day" is drawn (not a blank card with a button)', strips === rings * 24, `${strips}`);
  const tr = await page.$$eval('#dawn-list .dc-table tbody tr', (r) => r.length);
  check('and its numbers are in a table', tr === rings, `${tr} rows`);
  check('no script error', errors.length === 0, errors.join(' | '));
  await context.close();
}

// ── migration ───────────────────────────────────────────────────────────────
console.log('\nmigration');
{
  const { context, page, errors } = await open('/patterns?tab=migration');
  await page.waitForSelector('.rd-svg, #migration-ridgeline .empty-state', { timeout: 15000 });
  await page.waitForSelector('.stat-tile', { timeout: 15000 });
  const tiles = await page.$$eval('.stat-tile', (t) => t.map((x) => x.textContent.replace(/\s+/g, ' ').trim()));
  const earliest = tiles.find((t) => t.startsWith('Earliest vs last year')) || '';
  check('Earliest vs last year does not print a number the station cannot know', !/[-−]\d{2,3} d/.test(earliest) && /partly recorded|no prior-year|no bird has arrived/.test(earliest), earliest);
  const expected = tiles.find((t) => t.startsWith('Still expected')) || '';
  check('Still expected says what its zero means', /needs a year|arriving within|none outstanding/.test(expected), expected);
  const missing = await page.locator('.mig-cards .bnb-card').nth(2).textContent();
  check('"Missing" is not a placeholder', !/Forecast model pending/.test(missing), missing.replace(/\s+/g, ' '));
  if (await page.$('.rd-svg')) {
    const ridges = await page.$$eval('.rd-ridge', (e) => e.length);
    check('the ridgeline draws one ridge per species', ridges >= 2 && ridges <= 12, `${ridges}`);
    const classes = await page.$$eval('.rd-ridge', (g) => [...new Set(g.map((x) => [...x.classList].find((c) => /^ck-c/.test(c))))]);
    check('ridge colour is a season (three kinds at most)', classes.length <= 3, classes.join(' '));
    const svg = await page.$('.rd-svg');
    const sb = await svg.boundingBox();
    await page.mouse.move(sb.x + sb.width * 0.45, sb.y + sb.height * 0.35);
    await page.waitForTimeout(200);
    const tip = await page.locator('.rd-fig .ck-tip.on').textContent().catch(() => '');
    check('a column reads the week, the species heard and how many kinds', /Week \d+/.test(tip) && /species heard|has not happened/.test(tip), tip);
    const hatched = await page.$$eval('.rd-svg rect[fill*="rd-nd"]', (e) => e.length);
    check('the weeks that have not happened are hatched', hatched === 1, `${hatched}`);
  } else {
    console.log('  (no ridgeline on this fixture: the empty state)');
  }
  check('no script error', errors.length === 0, errors.join(' | '));
  await context.close();
}

// ── behavior ────────────────────────────────────────────────────────────────
console.log('\nbehavior');
{
  const { context, page, errors } = await open('/patterns?tab=behavior');
  await page.waitForSelector('.bt-svg, .rt-table, .mo-svg, .fl-fig', { timeout: 20000 }).catch(() => {});
  await page.waitForTimeout(1200);
  const d = await dim(page);
  const bars = await page.$$eval('.bt-bar', (e) => e.length);
  check('the bursts timeline draws a bar per burst', bars >= 3, `${bars}`);
  if (bars) {
    const legend = page.locator('.bt-key .ck-leg').first();
    await legend.click();
    const pressed = await legend.getAttribute('aria-pressed');
    const ops = await opacities(page, '.bt-bar');
    check('the legend pins a species and the rest fall back', pressed === 'true' && ops.some((o) => Math.abs(o - d) < 0.02) && ops.some((o) => o > 0.99), `${pressed} ${JSON.stringify(ops.slice(0, 6))}`);
    await legend.click();
  }
  const groups = await page.$$eval('.rt-group', (e) => e.map((x) => x.textContent.replace(/\s+/g, ' ').trim()));
  check('retention is grouped by class (resident, regular, migrant)', groups.length >= 2 && groups.some((g) => /Resident/.test(g)), groups.join(' | '));
  const cells = await page.$$eval('.rt-cell', (e) => e.length);
  check('and each species has its six return rates', cells >= 6 * 5, `${cells}`);
  const flow = await page.evaluate(() => ({ sankey: getComputedStyle(document.querySelector('.fl-sankey')).display, lists: getComputedStyle(document.querySelector('.fl-lists')).display, flows: document.querySelectorAll('.fl-flow').length }));
  check('the neighbour flow is drawn at desktop width, with the lists hidden', flow.sankey !== 'none' && flow.lists === 'none' && flow.flows >= 2, JSON.stringify(flow));
  const dots = await page.$$eval('.mo-dot', (e) => e.length);
  check('the running order draws a dot per morning per voice', dots >= 6, `${dots}`);
  if (dots) {
    const hit = (await page.$$('.mo-svg .ck-hit'))[1];
    await hit.scrollIntoViewIfNeeded();
    const hb = await hit.boundingBox();
    await page.mouse.move(hb.x + hb.width / 2, hb.y + hb.height / 2);
    await page.waitForTimeout(150);
    const dotOps = await opacities(page, '.mo-dot');
    check('pointing at a morning brings its dots forward and the others back', dotOps.some((o) => o > 0.99) && dotOps.some((o) => Math.abs(o - d) < 0.02), JSON.stringify(dotOps.slice(0, 8)));
    const tip = await page.locator('.mo-fig .ck-tip.on').textContent().catch(() => '');
    check('and reads the time', /First heard/.test(tip) && /\d\d:\d\d/.test(tip), tip);
  }
  check('no script error', errors.length === 0, errors.join(' | '));
  await context.close();
}

console.log('\nbehavior on a phone');
{
  const { context, page, errors } = await open('/patterns?tab=behavior', { width: 390, touch: true });
  await page.waitForSelector('.fl-lists, .bt-svg', { timeout: 20000 }).catch(() => {});
  await page.waitForTimeout(1200);
  const r = await page.evaluate(() => ({
    sankey: getComputedStyle(document.querySelector('.fl-sankey')).display,
    lists: getComputedStyle(document.querySelector('.fl-lists')).display,
    over: document.documentElement.scrollWidth - window.innerWidth,
  }));
  check('a phone gets ranked lists in place of the flow', r.sankey === 'none' && r.lists !== 'none', JSON.stringify(r));
  check('with no sideways scroll', r.over <= 0, `${r.over}px`);
  check('no script error', errors.length === 0, errors.join(' | '));
  await context.close();
}

await browser.close();
console.log(failures.length ? `\n${failures.length} FAILED` : '\nall passed');
process.exit(failures.length ? 1 : 0);
