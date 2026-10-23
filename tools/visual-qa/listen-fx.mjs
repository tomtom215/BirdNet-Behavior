// Does the listening-aids chain (static/listen-fx.js) do what it says, and does
// it stay out of the way when it is not asked for?
//
// Measured, in the browser the reader would use, not read off the source:
//
//   1. OUTPUT PEAK. Real clips are decoded and rendered through the *same*
//      graph the page builds (`bnbListenFx.buildChain` on an
//      OfflineAudioContext, whose float output is not clipped, so an over is
//      visible) at a spread of settings, up to +24 dB with the compressor on.
//      The peak must stay at or under the limiter's ceiling. The counterpart
//      renders the same settings with the limiter left out and must go OVER
//      full scale on at least one clip, otherwise the first check could not
//      have seen a clip and proves nothing.
//   2. NEUTRAL IS TRANSPARENT. At the defaults the chain is not even built for
//      a played clip (checked below), and if it were, its output must match the
//      input to within a fraction of a dB.
//   3. THE LIVE PATH. With a setting on, a played `Audio` (detached, as the
//      shared quick-play button uses) is routed and sound reaches the chain's
//      output; with the defaults it is not routed; a cross-origin source is
//      never routed (it would be silenced).
//   4. THE CONTROLS. Operable from the keyboard alone, announce their value,
//      remember it across a reload, and still work when storage throws.
//   5. SCREENSHOTS of the controls, light and dark, 1440 and 390, when SHOTS is
//      a directory.
//
// Clips: LISTEN_FX_CLIPS is a comma-separated list of WAV paths (the real ones
// used for the figures in the changelog are BirdNET-Pi's `Pica pica_30s.wav`
// and birdnet-go's `tawnyowl.wav` and `soundscape.wav`); the fixture's own
// first clip is always added.
//
// Usage:  BASE=http://127.0.0.1:8516 CHROMIUM_PATH=... node listen-fx.mjs
//         MUTATE=1 node listen-fx.mjs   (self-test: the limiter stage is
//                                        replaced by a pass-through, so check 1
//                                        must go red)
import { chromium } from 'playwright';
import fs from 'node:fs';

const BASE = process.env.BASE || 'http://127.0.0.1:8502';
const CHROMIUM_PATH = process.env.CHROMIUM_PATH || '';
const MUTATE = Boolean(process.env.MUTATE);
const SHOTS = process.env.SHOTS || '';

const failures = [];
const ok = (cond, msg) => {
  if (!cond) { failures.push(msg); console.log('  FAIL', msg); }
};

const browser = await chromium.launch({
  ...(CHROMIUM_PATH ? { executablePath: CHROMIUM_PATH } : {}),
  args: ['--autoplay-policy=no-user-gesture-required'],
});
const context = await browser.newContext({ viewport: { width: 1440, height: 900 } });
const page = await context.newPage();
const consoleErrors = [];
page.on('console', (m) => { if (m.type() === 'error') consoleErrors.push(m.text()); });
page.on('pageerror', (e) => consoleErrors.push(String(e)));

await page.goto(`${BASE}/station/settings`, { waitUntil: 'networkidle' });
ok(await page.evaluate(() => typeof window.bnbListenFx === 'object'), 'listen-fx.js is loaded on the settings page');

if (MUTATE) {
  // A limiter that does nothing: what the chain would be without the last stage.
  await page.evaluate(() => {
    const real = window.bnbListenFx.buildChain;
    window.bnbListenFx.buildChain = (c, s, d, st) => real(c, s, d, st, { limiter: false });
  });
}

// ---- 1 + 2: the output peak on real clips -----------------------------------

const clips = [];
for (const p of (process.env.LISTEN_FX_CLIPS || '').split(',').filter(Boolean)) {
  clips.push({ name: p.split('/').pop(), b64: fs.readFileSync(p).toString('base64') });
}
const detections = await (await page.request.get(`${BASE}/api/v2/detections?limit=1`)).json();
const fixtureFile = JSON.stringify(detections).match(/[A-Za-z0-9_\-:.]+\.wav/)?.[0];
if (fixtureFile) {
  const res = await page.request.get(`${BASE}/api/v2/recordings/${encodeURIComponent(fixtureFile)}`);
  if (res.ok()) clips.push({ name: `fixture:${fixtureFile}`, b64: (await res.body()).toString('base64') });
}
ok(clips.length > 0, 'at least one clip to measure');

const SETTINGS = [
  { gain: 0, hp: 20, comp: false },
  { gain: 12, hp: 20, comp: false },
  { gain: 24, hp: 20, comp: false },
  { gain: 24, hp: 8000, comp: false },
  { gain: 12, hp: 2000, comp: true },
  { gain: 24, hp: 20, comp: true },
  { gain: 24, hp: 1000, comp: true },
];

const measure = (b64, settings, limiter) => page.evaluate(async ({ b64, settings, limiter }) => {
  const bytes = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
  const tmp = new AudioContext();
  const buf = await tmp.decodeAudioData(bytes.buffer.slice(0));
  await tmp.close();
  // Up to 30 s of the first channel, mono, at the clip's own rate.
  const rate = buf.sampleRate;
  const len = Math.min(buf.length, rate * 30);
  const input = buf.getChannelData(0).slice(0, len);
  const off = new OfflineAudioContext(1, len, rate);
  const src = off.createBufferSource();
  const mono = off.createBuffer(1, len, rate);
  mono.copyToChannel(input, 0);
  src.buffer = mono;
  window.bnbListenFx.buildChain(off, src, off.destination, settings, { limiter });
  src.start();
  const out = (await off.startRendering()).getChannelData(0);
  const stat = (a) => {
    let peak = 0, sq = 0, hot = 0;
    for (let i = 0; i < a.length; i++) {
      const v = Math.abs(a[i]);
      if (v > peak) peak = v;
      if (v > 0.9) hot++;
      sq += a[i] * a[i];
    }
    return { peak, rms: Math.sqrt(sq / a.length), hot: hot / a.length };
  };
  return { in: stat(input), out: stat(out), seconds: len / rate };
}, { b64, settings, limiter });

const db = (x) => (x > 0 ? (20 * Math.log10(x)).toFixed(1) : '-inf');
const ceiling = await page.evaluate(() => window.bnbListenFx.CEILING);
let sawOverWithoutLimiter = false;
console.log(`ceiling ${ceiling} (${db(ceiling)} dBFS)`);
for (const clip of clips) {
  console.log(`\n${clip.name}`);
  for (const s of SETTINGS) {
    const withL = await measure(clip.b64, s, true);
    const bare = await measure(clip.b64, s, false);
    const label = `gain ${String(s.gain).padStart(2)} dB, hp ${String(s.hp).padStart(4)} Hz, comp ${s.comp ? 'on ' : 'off'}`;
    console.log(`  ${label}  in peak ${db(withL.in.peak)} dBFS | out peak ${db(withL.out.peak)} dBFS rms ${db(withL.out.rms)} hot>0.9 ${(withL.out.hot * 100).toFixed(2)}% | without limiter peak ${db(bare.out.peak)} dBFS`);
    ok(withL.out.peak <= ceiling + 1e-6, `${clip.name} @ ${label}: output peak ${withL.out.peak.toFixed(4)} is over the ceiling ${ceiling}`);
    if (bare.out.peak > 1) sawOverWithoutLimiter = true;
    if (s.gain === 0 && s.hp === 20 && !s.comp) {
      // Transparent: the 20 Hz corner and the limiter's identity region.
      const dp = Math.abs(20 * Math.log10((withL.out.peak || 1e-9) / (withL.in.peak || 1e-9)));
      const dr = Math.abs(20 * Math.log10((withL.out.rms || 1e-9) / (withL.in.rms || 1e-9)));
      ok(dp < 0.5 && dr < 0.5, `${clip.name}: neutral settings changed the level (peak ${dp.toFixed(2)} dB, rms ${dr.toFixed(2)} dB)`);
    }
  }
}
ok(sawOverWithoutLimiter, 'the same settings WITHOUT the limiter must go over full scale on some clip, or the peak check could not see a clip');

// ---- 3: the live path --------------------------------------------------------

const clipUrl = fixtureFile ? `${BASE}/api/v2/recordings/${encodeURIComponent(fixtureFile)}` : null;
if (clipUrl) {
  const live = (settings, src) => page.evaluate(async ({ settings, src }) => {
    const fx = window.bnbListenFx;
    fx.set(settings, false);
    const el = new Audio(src);
    el.loop = true;
    el.volume = 1;
    await el.play().catch(() => {});
    await new Promise((r) => setTimeout(r, 700));
    const chain = fx.chainFor(el);
    let level = null;
    if (chain) {
      const ctx = fx.context();
      const an = ctx.createAnalyser();
      an.fftSize = 2048;
      chain.limiter.connect(an);
      const data = new Float32Array(an.fftSize);
      let peak = 0;
      for (let i = 0; i < 20; i++) {
        an.getFloatTimeDomainData(data);
        for (const v of data) peak = Math.max(peak, Math.abs(v));
        await new Promise((r) => setTimeout(r, 40));
      }
      level = peak;
    }
    const r = { routed: fx.isRouted(el), level, time: el.currentTime, state: fx.context() ? fx.context().state : null };
    el.pause();
    return r;
  }, { settings, src });

  const neutral = await live({ gain: 0, hp: 20, comp: false }, clipUrl);
  console.log('\nlive, neutral:', JSON.stringify(neutral));
  ok(!neutral.routed, 'with the defaults a played clip is not routed through Web Audio at all');
  ok(neutral.time > 0.2, 'a clip played with the defaults advances');

  const active = await live({ gain: 12, hp: 500, comp: true }, clipUrl);
  console.log('live, active :', JSON.stringify(active));
  ok(active.routed, 'with a setting on, a played detached Audio is routed');
  ok(active.time > 0.2, 'a routed clip advances');
  ok(active.level !== null && active.level > 0.001, `sound reaches the chain's output (peak ${active.level})`);
  ok(active.level === null || active.level <= ceiling + 1e-6, 'the live output stays under the ceiling');

  const foreign = await live({ gain: 12, hp: 20, comp: false }, 'https://cross-origin.invalid/clip.wav');
  ok(!foreign.routed, 'a cross-origin source is never routed (the browser would silence it)');
  await page.evaluate(() => window.bnbListenFx.set({ gain: 0, hp: 20, comp: false }, false));
} else {
  ok(false, 'no fixture clip to play');
}

// ---- 4: the controls ---------------------------------------------------------

await page.goto(`${BASE}/station/settings`, { waitUntil: 'networkidle' });
await page.evaluate(() => { try { localStorage.removeItem('bnb-listen'); } catch (e) { /* none */ } });
await page.reload({ waitUntil: 'networkidle' });
const box = page.locator('#display-prefs [data-listen-fx]');
await box.waitFor({ state: 'visible', timeout: 5000 });
const gainSlider = box.getByLabel('Boost');
await gainSlider.focus();
for (let i = 0; i < 3; i++) await page.keyboard.press('ArrowRight');
ok((await gainSlider.inputValue()) === '3', 'the boost slider moves 1 dB per arrow key');
ok((await gainSlider.getAttribute('aria-valuetext')) === 'plus 3 decibels', 'the boost slider announces its value');
ok((await box.locator('.lfx-out').first().textContent()) === '+3 dB', 'the boost readout shows the value');
const hpSlider = box.getByLabel('Cut low rumble');
await hpSlider.focus();
await page.keyboard.press('End');
ok((await hpSlider.getAttribute('aria-valuetext')) === '8 kilohertz', 'the high-pass slider announces kilohertz');
await page.keyboard.press('Home');
ok((await hpSlider.getAttribute('aria-valuetext')) === 'off', 'the high-pass slider at its lowest says off');
await box.getByLabel('Even out loud and quiet parts').check();
const stored = await page.evaluate(() => localStorage.getItem('bnb-listen'));
ok(stored && JSON.parse(stored).gain === 3 && JSON.parse(stored).comp === true, `the choice is stored (${stored})`);
await page.reload({ waitUntil: 'networkidle' });
ok((await page.locator('#display-prefs [data-listen-fx]').getByLabel('Boost').inputValue()) === '3', 'the choice survives a reload');
await page.locator('#display-prefs [data-listen-fx] .lfx-reset').click();
ok((await page.locator('#display-prefs [data-listen-fx]').getByLabel('Boost').inputValue()) === '0', 'Reset returns to the defaults');
ok((await page.evaluate(() => window.bnbListenFx.isNeutral(window.bnbListenFx.get()))), 'Reset leaves the chain neutral');

// Storage that throws (a private window, blocked site data) must not break the controls.
const blocked = await browser.newContext({ viewport: { width: 1440, height: 900 } });
await blocked.addInitScript(() => {
  const deny = () => { throw new DOMException('denied', 'SecurityError'); };
  Object.defineProperty(window, 'localStorage', { get: deny });
});
const bp = await blocked.newPage();
const blockedErrors = [];
bp.on('pageerror', (e) => blockedErrors.push(String(e)));
await bp.goto(`${BASE}/station/settings`, { waitUntil: 'networkidle' });
const bbox = bp.locator('#display-prefs [data-listen-fx]');
await bbox.waitFor({ state: 'visible', timeout: 5000 });
await bbox.getByLabel('Boost').focus();
await bp.keyboard.press('ArrowRight');
ok((await bbox.getByLabel('Boost').inputValue()) === '1', 'the controls work when localStorage throws');
ok(blockedErrors.length === 0, `no page error when localStorage throws (${blockedErrors.join('; ')})`);
await blocked.close();

// ---- 5: screenshots ----------------------------------------------------------

if (SHOTS) {
  fs.mkdirSync(SHOTS, { recursive: true });
  for (const scheme of ['light', 'dark']) {
    for (const [w, h] of [[1440, 900], [390, 844]]) {
      const c = await browser.newContext({ viewport: { width: w, height: h }, colorScheme: scheme });
      await c.addInitScript((s) => {
        try { localStorage.setItem('theme', s); localStorage.setItem('bnb-listen', JSON.stringify({ gain: 12, hp: 500, comp: true })); } catch (e) { /* none */ }
      }, scheme);
      const p = await c.newPage();
      await p.goto(`${BASE}/station/settings`, { waitUntil: 'networkidle' });
      const card = p.locator('#display-prefs');
      await card.scrollIntoViewIfNeeded();
      await card.screenshot({ path: `${SHOTS}/listen-fx-prefs-${scheme}-${w}.png` });
      if (clipUrl) {
        await p.goto(`${BASE}/player/${encodeURIComponent(fixtureFile)}`, { waitUntil: 'networkidle' });
        await p.locator('.lfx-fold > summary').click();
        await p.screenshot({ path: `${SHOTS}/listen-fx-player-${scheme}-${w}.png` });
      }
      await c.close();
    }
  }
}

ok(consoleErrors.filter((e) => !/cross-origin\.invalid|ERR_NAME_NOT_RESOLVED|net::ERR/.test(e)).length === 0,
  `console errors: ${consoleErrors.join(' | ')}`);

await browser.close();
if (failures.length) {
  console.log(`\n${failures.length} FAILED`);
  process.exit(1);
}
console.log('\nlisten-fx: all checks passed');
