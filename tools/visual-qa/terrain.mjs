// Behavioural gate for the soundscape terrain (static/terrain.js, the Patterns "When active"
// lead card and the vendored three.js behind its 3D view).
//
// A screenshot shows the terrain at rest, and a 3D view that rendered once and then ignores its
// pointer, leaks its GPU memory or loads half a megabyte on every page looks identical in one.
// Each check below is something a reader does to it, or a promise the code makes:
//
//   geometry            the mesh as a pure function (run under node, no browser): the surface reaches
//                       every cell's height and never goes above its neighbours, a void cell is flat,
//                       normals are unit and point up, the cell under a point is the one it is over
//   lazy                the page does not request three.js until 3D terrain is chosen, and requests it
//                       once, with the hash in its URL and an immutable cache header
//   draws               the canvas holds lit pixels (read back from the GPU, not guessed from the DOM)
//   orbit               a drag, the arrow keys and the wheel (once the terrain has focus) each change
//                       the picture; Home puts it back exactly
//   reading             a pointer over the ground, and Shift + the arrows, put the cell's day, hour,
//                       count and top species into the aria-live line
//   csp                 no securitypolicyviolation event and no page error, from load through every
//                       interaction here
//   no webgl            a browser without WebGL keeps the grid, says so, disables 3D, and never fetches
//                       three.js
//   reduced motion      no entrance (the OS, and the operator's Motion setting)
//   hidden tab          nothing is drawn while the document is hidden
//   theme               a theme switch repaints the terrain, no reload
//   dpr cap             a device pixel ratio of 3 draws at no more than 2
//   context loss        a lost WebGL context falls back to the grid
//   dispose             removing the card frees the GPU resources
//   frame time          the cost of a frame, measured in headless Chromium's software GL (swiftshader):
//                       a lower bound for a weak device, printed, with a ceiling that only a real
//                       regression reaches
//
// Run against the fixture server:
//   BASE=http://127.0.0.1:8502 CHROMIUM_PATH=... node terrain.mjs
//
// Self-test (each must go red; this is how the gate was seen failing):
//   MUTATE=eager      terrain.js imports three.js as soon as the card boots
//   MUTATE=noorbit    a drag no longer turns the view
//   MUTATE=nofallback hasWebGL() answers true everywhere
//   MUTATE=noreading  the pointer reading is never written
//   MUTATE=leak       dispose() frees nothing
//   MUTATE=nofit      the camera goes back to one fixed distance for every shape of canvas
import { chromium } from 'playwright';
import * as T from '../../crates/birdnet-web/static/terrain.js';

const BASE = process.env.BASE || 'http://127.0.0.1:8502';
const CHROMIUM_PATH = process.env.CHROMIUM_PATH || '';
const MUTATE = process.env.MUTATE || '';
const FRAME_CEILING_MS = Number(process.env.FRAME_CEILING_MS || 250);
const failures = [];

function check(name, ok, detail = '') {
  if (ok) console.log(`  \x1b[32mPASS\x1b[0m ${name}`);
  else {
    failures.push({ name, detail });
    console.log(`  \x1b[31mFAIL\x1b[0m ${name}\n       ${detail}`);
  }
}

// ── geometry, under node ────────────────────────────────────────────────────
console.log('terrain geometry (no browser)');
{
  const rows = 6;
  const cols = 24;
  const heights = new Float32Array(rows * cols);
  heights[2 * cols + 6] = 1;
  heights[3 * cols + 7] = 0.4;
  const void_ = new Uint8Array(rows * cols);
  void_[5 * cols + 20] = 1;
  heights[5 * cols + 20] = 0.9; // a void cell must be flat whatever it was given
  const f = T.buildHeightField({ heights, void_, rows, cols, subX: 4, subZ: 2, width: 24, depth: 20, height: 7 });
  check('vertex and triangle counts follow the lattice', f.values.length === f.GX * f.GZ && f.index.length === (f.GX - 1) * (f.GZ - 1) * 6, `${f.GX}x${f.GZ} ${f.values.length} ${f.index.length}`);
  check('the lattice has the outer half-cell and the middle of every cell', f.GX === T.axis(cols, 4).length && T.axis(cols, 4)[0] === -0.5 && T.axis(cols, 4).at(-1) === cols - 0.5 && T.axis(cols, 1).length === cols + 2);
  let top = 0;
  for (const v of f.values) top = Math.max(top, v);
  check('the tallest cell is reached (a peak is not cut off)', Math.abs(top - 1) < 1e-6, `top ${top}`);
  // Never above the highest of the four cells around a vertex, never below the lowest: a surface
  // that overshot would draw a busier hour than there was.
  let overshoot = 0;
  const us = T.axis(cols, 4);
  const vs = T.axis(rows, 2);
  for (let gz = 0; gz < f.GZ; gz++) {
    for (let gx = 0; gx < f.GX; gx++) {
      const u = us[gx];
      const v = vs[gz];
      if (f.isVoid[gz * f.GX + gx]) continue;
      const near = [];
      for (const r of [Math.floor(v), Math.ceil(v)]) for (const c of [Math.floor(u), Math.ceil(u)]) near.push(heights[Math.min(rows - 1, Math.max(0, r)) * cols + Math.min(cols - 1, Math.max(0, c))]);
      const val = f.values[gz * f.GX + gx];
      if (val > Math.max(...near) + 1e-6 || val < Math.min(...near) - 1e-6) overshoot++;
    }
  }
  check('between cell middles the surface stays between its neighbours (cannot overshoot)', overshoot === 0, `${overshoot} vertices out of range`);
  let voidHigh = 0;
  for (let i = 0; i < f.values.length; i++) if (f.isVoid[i] && f.values[i] !== 0) voidHigh++;
  check('a day the station was off (void) is flat, whatever the count', voidHigh === 0 && [...f.isVoid].some((x) => x === 1), `${voidHigh} raised void vertices`);
  let badNormal = 0;
  for (let i = 0; i < f.normals.length; i += 3) {
    const len = Math.hypot(f.normals[i], f.normals[i + 1], f.normals[i + 2]);
    if (Math.abs(len - 1) > 1e-4 || f.normals[i + 1] <= 0) badNormal++;
  }
  check('normals are unit length and point up', badNormal === 0, `${badNormal} bad`);
  const flat = T.buildHeightField({ heights: new Float32Array(rows * cols), void_: null, rows, cols });
  check('an empty station is a flat plane', [...flat.values].every((v) => v === 0) && [...flat.normals].filter((_, i) => i % 3 === 1).every((y) => Math.abs(y - 1) < 1e-6));
  const c = T.cellAt(f, -11.9, -9.9);
  const d = T.cellAt(f, 11.9, 9.9);
  check('a point is the cell it is over, and none outside', c.day === 0 && c.hour === 0 && d.day === rows - 1 && d.hour === cols - 1 && T.cellAt(f, 12.5, 0) === null && T.cellAt(f, 0, -10.5) === null);
  // The newest day is at the front (+z): the last row.
  check('the newest day is nearest the viewer', f.positions[(f.GZ - 1) * f.GX * 3 + 2] > f.positions[2]);
  check('the ramp is square-root spaced and zero is empty', T.rampStep(0, 100) === 0 && T.rampStep(1, 100) === 1 && T.rampStep(100, 100) === 7 && T.rampT(25, 100) === 0.5);
  const rd = T.reading({ days: 2, dates: ['2026-10-20', '2026-10-21'], n: [...Array(48).fill(0)], top: Array(48).fill(-1), top_n: Array(48).fill(0), off: [1, 0], species: [], hour_now: 5 }, 0, 6);
  check('a day the station was off reads as off, not as zero', rd.note.includes('not listening') && rd.rows.length === 0, JSON.stringify(rd));
  const rn = T.reading({ days: 2, dates: ['2026-10-20', '2026-10-21'], n: Array(48).fill(0), top: Array(48).fill(-1), top_n: Array(48).fill(0), off: [0, 0], species: [], hour_now: 5 }, 1, 9);
  check('an hour not yet come reads as not yet', rn.note.includes('not happened'), JSON.stringify(rn));
}

// ── in the browser ──────────────────────────────────────────────────────────
const browser = await chromium.launch({
  ...(CHROMIUM_PATH ? { executablePath: CHROMIUM_PATH } : {}),
  // Software GL: the lower bound a weak device is measured against.
  args: ['--use-gl=angle', '--use-angle=swiftshader', '--enable-unsafe-swiftshader', '--ignore-gpu-blocklist'],
});

async function open({ reduced = 'no-preference', motionSetting = null, theme = 'light', width = 1280, dpr = 1, noWebGL = false } = {}) {
  const context = await browser.newContext({ viewport: { width, height: 1000 }, deviceScaleFactor: dpr, reducedMotion: reduced, colorScheme: theme });
  await context.addInitScript(([m, t]) => {
    try {
      if (m) localStorage.setItem('bnb-motion', m);
      localStorage.setItem('theme', t);
    } catch (e) { /* storage blocked */ }
    window.__csp = [];
    document.addEventListener('securitypolicyviolation', (e) => window.__csp.push(`${e.violatedDirective} ${e.blockedURI}`));
  }, [motionSetting, theme]);
  if (noWebGL) {
    await context.addInitScript(() => {
      const real = HTMLCanvasElement.prototype.getContext;
      HTMLCanvasElement.prototype.getContext = function (type, ...rest) {
        if (/webgl/i.test(String(type))) return null;
        return real.call(this, type, ...rest);
      };
    });
  }
  if (MUTATE) {
    await context.route('**/static/terrain.js*', async (route) => {
      const res = await route.fetch();
      let body = await res.text();
      if (MUTATE === 'eager') body = body.replace('card.__terrain = new Terrain(card);', 'card.__terrain = new Terrain(card); import(`./vendor/three/three.module.min.js?v=${THREE_HASH}`);');
      if (MUTATE === 'noorbit') body = body.replace('o().theta -= dx * 0.008;', '');
      if (MUTATE === 'nofallback') body = body.replace('function hasWebGL() {', 'function hasWebGL() { return true;');
      if (MUTATE === 'noreading') body = body.replace('if (this.readout) this.readout.textContent = readingText(this.data, cell.day, cell.hour);', '');
      if (MUTATE === 'nofit') body = body.replace('this.fitHome();', 'g.camera.aspect < 1.2 ? (this.orbitHome.r = 34 / Math.max(0.55, g.camera.aspect) ** 0.7) : (this.orbitHome.r = 31);');
      if (MUTATE === 'leak') body = body.replaceAll('g.mesh.geometry.dispose();', '').replace('g.renderer.forceContextLoss();', '');
      await route.fulfill({ response: res, body });
    });
  }
  const page = await context.newPage();
  const errors = [];
  const requests = [];
  page.on('pageerror', (e) => errors.push(e.message));
  page.on('console', (m) => {
    if (m.type() === 'error' && !/Failed to load resource/.test(m.text())) errors.push(m.text());
  });
  page.on('request', (r) => requests.push(r.url()));
  await page.goto(`${BASE}/patterns?tab=when`, { waitUntil: 'domcontentloaded', timeout: 30000 });
  await page.waitForSelector('.tr-card .tr-readout', { timeout: 15000 });
  await page.waitForFunction(() => document.querySelector('.tr-card')?.__terrain?.data, null, { timeout: 15000 }).catch(() => {});
  return { context, page, errors, requests };
}

const threeRequests = (reqs) => reqs.filter((u) => u.includes('three.module'));
const cardOf = (page) => page.locator('.tr-card');

async function openTerrain(page) {
  await page.locator('.tr-card [data-view="3d"]').click();
  await page.waitForFunction(() => document.querySelector('.tr-card')?.__terrain?.glState === 'ready' || document.querySelector('.tr-card')?.__terrain?.glState === 'failed', null, { timeout: 30000 });
  await page.waitForTimeout(1300); // the entrance, when there is one
}

const probe = (page) => page.evaluate(() => document.querySelector('.tr-card').__terrain.probe());
// Which axis labels are not wholly inside the stage that holds them.
const labelsOutside = (page) =>
  page.evaluate(() => {
    const st = document.querySelector('.tr-stage').getBoundingClientRect();
    return [...document.querySelectorAll('.tr-lbl')]
      .filter((l) => !l.hidden && l.getClientRects().length)
      .map((l) => ({ t: l.textContent, r: l.getBoundingClientRect() }))
      .filter(({ r }) => r.left < st.left - 1 || r.right > st.right + 1 || r.top < st.top - 1 || r.bottom > st.bottom + 1)
      .map(({ t }) => t);
  });

// 1. lazy, draws, orbit, reading, csp ----------------------------------------------------------
console.log('\nthe terrain in a browser with WebGL');
{
  const { context, page, errors, requests } = await open();
  const gridLit = await page.evaluate(() => {
    const c = document.querySelector('.tr-grid-canvas');
    const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
    let lit = 0;
    for (let i = 3; i < d.length; i += 4 * 9) if (d[i] > 0) lit++;
    return { lit, w: c.width, h: c.height };
  });
  check('the grid is drawn on load, not blank', gridLit.lit > 200, JSON.stringify(gridLit));
  check('three.js is not requested by loading the page', threeRequests(requests).length === 0, threeRequests(requests).join(' '));
  await openTerrain(page);
  const state = await page.evaluate(() => document.querySelector('.tr-card').__terrain.glState);
  check('choosing 3D terrain loads and starts it', state === 'ready', `state ${state}`);
  const three = threeRequests(requests);
  check('three.js is requested once, with the hash in its URL', three.length === 1 && /three\.module\.min\.js\?v=[0-9a-f]{8}$/.test(three[0]), three.join(' '));
  const headers = await page.evaluate(async (u) => {
    const r = await fetch(u);
    return { cache: r.headers.get('cache-control'), etag: r.headers.get('etag'), type: r.headers.get('content-type') };
  }, three[0] || '/static/vendor/three/three.module.min.js');
  check('it is served immutable with an ETag', /immutable/.test(headers.cache || '') && /^"[0-9a-f]{16}"$/.test(headers.etag || ''), JSON.stringify(headers));
  const first = await probe(page);
  check('the canvas holds lit pixels read back from the GPU', first && first.litShare > 0.08, JSON.stringify(first));
  check('the entrance has finished: the ground is full height', first && Math.abs(first.scaleY - 1) < 1e-6, `scaleY ${first && first.scaleY}`);
  // The camera is fitted to the canvas: nothing of the slab is cut off, and no label is outside it.
  const outside = await labelsOutside(page);
  check('the whole slab is inside the canvas at 1280 px (no lit pixel on its edge)', first && first.edgeLit === 0, `${first && first.edgeLit} lit pixel(s) on the edge of a ${first && first.width}x${first && first.height} canvas`);
  check('every axis label is inside the stage at 1280 px', outside.length === 0, outside.join(', '));

  // Orbit: a drag.
  const box = await page.locator('.tr-gl-canvas').boundingBox();
  const cx = box.x + box.width / 2;
  const cy = box.y + box.height / 2;
  await page.mouse.move(cx, cy);
  await page.mouse.down();
  await page.mouse.move(cx + 140, cy + 30, { steps: 8 });
  await page.mouse.up();
  const dragged = await probe(page);
  check('a drag turns the view', dragged.hash !== first.hash && dragged.orbit.theta !== first.orbit.theta, `${first.orbit.theta} -> ${dragged.orbit.theta}`);
  // Keyboard.
  await page.locator('.tr-gl-canvas').focus();
  await page.keyboard.press('ArrowLeft');
  const keyed = await probe(page);
  check('the arrow keys turn it', keyed.hash !== dragged.hash && keyed.orbit.theta > dragged.orbit.theta, `${dragged.orbit.theta} -> ${keyed.orbit.theta}`);
  await page.keyboard.press('ArrowUp');
  const tilted = await probe(page);
  check('Up and Down tilt it', tilted.orbit.phi < keyed.orbit.phi, `${keyed.orbit.phi} -> ${tilted.orbit.phi}`);
  await page.keyboard.press('+');
  const zoomed = await probe(page);
  check('plus zooms in and minus out', zoomed.orbit.r < tilted.orbit.r, `${tilted.orbit.r} -> ${zoomed.orbit.r}`);
  await page.keyboard.press('-');
  await page.keyboard.press('-');
  const out = await probe(page);
  check('minus zooms out', out.orbit.r > zoomed.orbit.r, `${zoomed.orbit.r} -> ${out.orbit.r}`);
  // The wheel works once the terrain has focus.
  await page.mouse.move(cx, cy);
  await page.mouse.wheel(0, -300);
  const wheeled = await probe(page);
  check('the wheel zooms once the terrain has focus', wheeled.orbit.r < out.orbit.r, `${out.orbit.r} -> ${wheeled.orbit.r}`);
  // The pointer has been resting on the ground (a hover marker is part of the picture); take it
  // away so the comparison is of the view and not of the marker.
  await page.mouse.move(2, 2);
  await page.locator('.tr-gl-canvas').focus();
  await page.keyboard.press('Escape'); // clears the reading cursor, which is drawn on the ground
  await page.keyboard.press('Home');
  const home = await probe(page);
  check('Home puts the view back exactly', home.hash === first.hash && Math.abs(home.orbit.theta - first.orbit.theta) < 1e-9, `${home.hash} vs ${first.hash} ${JSON.stringify(home.orbit)} ${JSON.stringify(first.orbit)} cursor ${home.cursor}/${first.cursor} scaleY ${home.scaleY}/${first.scaleY}`);

  // Reading.
  const readoutBefore = await page.locator('.tr-readout').textContent();
  await page.mouse.move(cx - 20, cy + 10);
  await page.mouse.move(cx + 5, cy + 15, { steps: 4 });
  await page.waitForTimeout(200);
  const readoutAfter = await page.locator('.tr-readout').textContent();
  check('pointing at the ground writes a reading (day, hour, count)', readoutAfter !== readoutBefore && /\d\d:00 to \d\d:59/.test(readoutAfter), readoutAfter);
  check('the reading says who was heard most, or why there is nothing', /Most heard|not listening|not happened|heard nothing/.test(readoutAfter), readoutAfter);
  await page.locator('.tr-gl-canvas').focus();
  await page.keyboard.press('Shift+ArrowLeft');
  const shifted = await page.locator('.tr-readout').textContent();
  check('Shift and the arrows move a cursor and read it from the keyboard', shifted !== readoutAfter, `${readoutAfter} -> ${shifted}`);
  const live = await page.locator('.tr-readout').getAttribute('aria-live');
  check('the reading is an aria-live region', live === 'polite', String(live));
  const names = await page.evaluate(() => [document.querySelector('.tr-grid-canvas').getAttribute('aria-label'), document.querySelector('.tr-gl-canvas').getAttribute('aria-label'), document.querySelector('.tr-gl-canvas').tabIndex]);
  check('both canvases are labelled and focusable', names[0].length > 20 && names[1].length > 20 && names[2] === 0, JSON.stringify(names));

  // Colour by species repaints.
  const before = await probe(page);
  await page.locator('.tr-card [data-colour="species"]').click();
  const bySpecies = await probe(page);
  check('colouring by species repaints the terrain', bySpecies.hash !== before.hash, `${before.hash} ${bySpecies.hash}`);
  await page.locator('.tr-card [data-colour="activity"]').click();

  // Ranges.
  for (const days of [30, 365]) {
    await page.locator(`.tr-card [data-days="${days}"]`).click();
    await page.waitForFunction((d) => document.querySelector('.tr-card').__terrain.data?.days === d && document.querySelector('.tr-card').__terrain.field?.rows === d, days, { timeout: 15000 });
    const p = await probe(page);
    check(`${days} days draws`, p && p.litShare > 0.05, JSON.stringify(p));
  }
  await page.locator('.tr-card [data-days="90"]').click();
  await page.waitForFunction(() => document.querySelector('.tr-card').__terrain.data?.days === 90 && document.querySelector('.tr-card').__terrain.field?.rows === 90, null, { timeout: 15000 });

  // Theme.
  const light = await probe(page);
  await page.evaluate(() => document.documentElement.setAttribute('data-theme', 'dark'));
  await page.waitForTimeout(150);
  const dark = await probe(page);
  check('a theme switch repaints the terrain without a reload', dark.hash !== light.hash, `${light.hash} ${dark.hash}`);
  await page.evaluate(() => document.documentElement.setAttribute('data-theme', 'light'));

  // Frame time.
  const bench = await page.evaluate(() => document.querySelector('.tr-card').__terrain.bench(90));
  console.log(`  frame time (software GL, ${bench.frames} frames orbiting): interval median ${bench.intervalMedian.toFixed(1)} ms, p95 ${bench.intervalP95.toFixed(1)} ms; CPU submit median ${bench.cpuMedian.toFixed(2)} ms, p95 ${bench.cpuP95.toFixed(2)} ms`);
  check(`a frame costs under ${FRAME_CEILING_MS} ms even on software GL`, bench.intervalMedian < FRAME_CEILING_MS, JSON.stringify(bench));

  // Hidden tab: nothing is drawn.
  await page.evaluate(() => {
    Object.defineProperty(document, 'hidden', { configurable: true, get: () => true });
    document.dispatchEvent(new Event('visibilitychange'));
  });
  const framesBefore = await page.evaluate(() => document.querySelector('.tr-card').__terrain.frames);
  await page.locator('.tr-gl-canvas').focus();
  await page.keyboard.press('ArrowLeft');
  await page.waitForTimeout(250);
  const framesHidden = await page.evaluate(() => document.querySelector('.tr-card').__terrain.frames);
  check('nothing is drawn while the tab is hidden', framesHidden === framesBefore, `${framesBefore} -> ${framesHidden}`);
  await page.evaluate(() => {
    delete document.hidden;
    document.dispatchEvent(new Event('visibilitychange'));
  });
  await page.waitForTimeout(250);
  const framesShown = await page.evaluate(() => document.querySelector('.tr-card').__terrain.frames);
  check('and drawing resumes when it is shown again', framesShown > framesHidden, `${framesHidden} -> ${framesShown}`);

  // Dispose.
  const geoms = await page.evaluate(() => document.querySelector('.tr-card').__terrain.gl.renderer.info.memory.geometries);
  await page.evaluate(() => {
    window.__renderer = document.querySelector('.tr-card').__terrain.gl.renderer;
    document.querySelector('.tr-card').remove();
  });
  await page.waitForTimeout(400);
  await page.evaluate(() => window.dispatchEvent(new Event('resize')));
  const after = await page.evaluate(() => ({ geoms: window.__renderer.info.memory.geometries, lost: window.__renderer.getContext().isContextLost() }));
  check('removing the card frees its GPU resources', geoms > 0 && after.geoms === 0 && after.lost, `before ${geoms}, after ${JSON.stringify(after)}`);

  check('no script error from load through every interaction', errors.length === 0, errors.join(' | '));
  const csp = await page.evaluate(() => window.__csp);
  check('no securitypolicyviolation event', csp.length === 0, csp.join(' | '));
  await context.close();
}

// 2. context loss ------------------------------------------------------------------------------
console.log('\ncontext loss');
{
  const { context, page } = await open();
  await openTerrain(page);
  await page.evaluate(() => {
    const gl = document.querySelector('.tr-card').__terrain.gl.renderer.getContext();
    gl.getExtension('WEBGL_lose_context')?.loseContext();
  });
  await page.waitForFunction(() => document.querySelector('.tr-card').__terrain.glState === 'failed', null, { timeout: 8000 }).catch(() => {});
  const r = await page.evaluate(() => ({ state: document.querySelector('.tr-card').__terrain.glState, grid: !document.querySelector('.tr-grid').hidden, glHidden: document.querySelector('.tr-3d').hidden, msg: document.querySelector('.tr-msg').textContent }));
  check('a lost context falls back to the grid and says so', r.state === 'failed' && r.grid && r.glHidden && /interrupted/.test(r.msg), JSON.stringify(r));
  await context.close();
}

// 3. no WebGL ----------------------------------------------------------------------------------
console.log('\nno WebGL');
{
  const { context, page, errors, requests } = await open({ noWebGL: true });
  await page.locator('.tr-card [data-view="3d"]').click();
  await page.waitForTimeout(600);
  const r = await page.evaluate(() => ({
    grid: !document.querySelector('.tr-grid').hidden,
    glHidden: document.querySelector('.tr-3d').hidden,
    msg: document.querySelector('.tr-msg').textContent,
    disabled: document.querySelector('.tr-card [data-view="3d"]').getAttribute('aria-disabled'),
    pressed: document.querySelector('.tr-card [data-view="grid"]').getAttribute('aria-pressed'),
  }));
  check('without WebGL the grid stays, says why, and 3D is disabled', r.grid && r.glHidden && /cannot draw/.test(r.msg) && r.disabled === 'true' && r.pressed === 'true', JSON.stringify(r));
  check('without WebGL three.js is never fetched', threeRequests(requests).length === 0, threeRequests(requests).join(' '));
  check('without WebGL there is no error', errors.length === 0, errors.join(' | '));
  await page.locator('.tr-grid-canvas').focus();
  await page.keyboard.press('ArrowLeft');
  const text = await page.locator('.tr-readout').textContent();
  check('the grid is still readable from the keyboard', /\d\d:00 to \d\d:59/.test(text), text);
  await context.close();
}

// 4. reduced motion, three ways -----------------------------------------------------------------
console.log('\nreduced motion');
for (const [label, opts, expectRise] of [
  ['with no preference the ground rises', {}, true],
  ['when the OS asks for reduced motion there is no entrance', { reduced: 'reduce' }, false],
  ['when the operator\'s Motion setting is reduced there is no entrance', { motionSetting: 'reduced' }, false],
]) {
  const { context, page } = await open(opts);
  await page.locator('.tr-card [data-view="3d"]').click();
  await page.waitForFunction(() => document.querySelector('.tr-card').__terrain.glState === 'ready', null, { timeout: 30000 });
  const intro = await page.evaluate(() => document.querySelector('.tr-card').__terrain.gl.intro !== 0);
  check(label, intro === expectRise, `intro running: ${intro}`);
  await context.close();
}

// 5. device pixel ratio -----------------------------------------------------------------------
console.log('\ndevice pixel ratio');
{
  const { context, page } = await open({ dpr: 3 });
  await openTerrain(page);
  const p = await probe(page);
  check('a pixel ratio of 3 draws at no more than 2', p.width <= p.cssWidth * 2 + 2 && p.width >= p.cssWidth * 2 - 2, JSON.stringify({ w: p.width, css: p.cssWidth }));
  await context.close();
}

// 6. the phone ----------------------------------------------------------------------------------
console.log('\na phone');
{
  const { context, page, errors } = await open({ width: 390 });
  await openTerrain(page);
  const p = await probe(page);
  const over = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
  check('the 3D view draws at 390 px and the page does not scroll sideways', p && p.litShare > 0.05 && over <= 0, JSON.stringify({ p: p && p.litShare, over }));
  const phoneOutside = await labelsOutside(page);
  check('the whole slab is inside the canvas at 390 px (no lit pixel on its edge)', p && p.edgeLit === 0, `${p && p.edgeLit} lit pixel(s) on the edge of a ${p && p.width}x${p && p.height} canvas`);
  check('every axis label is inside the stage at 390 px', phoneOutside.length === 0, phoneOutside.join(', '));
  check('no error on a phone', errors.length === 0, errors.join(' | '));
  await context.close();
}

await browser.close();
console.log(failures.length ? `\n${failures.length} FAILED` : '\nall passed');
process.exit(failures.length ? 1 : 0);
