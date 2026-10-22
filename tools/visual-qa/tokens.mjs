// Measured gate: the colour tokens do what their comments say, in both themes.
//
// Reads the computed value of real elements on the running fixture (so what is
// graded is what the browser paints, after `color-mix()`, `var()` and the
// theme cascade), resolves it to sRGB through a canvas, and computes WCAG
// contrast. Nothing here is eyeballed, and nothing is read from the CSS text.
//
// What it holds:
//   1. Confidence (`--conf-*`): a tier's bar fill is 3:1 against the bar's
//      track (WCAG 1.4.11, graphical object), its pill text 4.5:1 on its pill
//      fill (1.4.3), the key's text 4.5:1 on the card, and the three tiers
//      are told apart from each other. The scale never uses the alert hue.
//   2. Chart palette (`--chart-*`): see the second half of this file.
//
// Usage:  BASE=http://127.0.0.1:8611 CHROMIUM_PATH=... node tokens.mjs
//         MUTATE='--conf-low=oklch(68% 0.12 60)' node tokens.mjs   (self-test:
//         override a token before grading, to watch the gate go red)
import { chromium } from 'playwright';

const BASE = process.env.BASE || 'http://127.0.0.1:8502';
const CHROMIUM_PATH = process.env.CHROMIUM_PATH || '';
const MUTATE = process.env.MUTATE || '';

const browser = await chromium.launch(CHROMIUM_PATH ? { executablePath: CHROMIUM_PATH } : {});
// CSS_OVERRIDE=old.css grades an earlier stylesheet on the running fixture (the
// same trick as style-snapshot.mjs), to see a new check fail against the code it
// was written for without rebuilding anything.
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
const failures = [];
const rows = [];

// ── colour maths ───────────────────────────────────────────────────────────
const lin = (c) => { c /= 255; return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4; };
const lum = ([r, g, b]) => 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
export const contrast = (a, b) => {
  const [hi, lo] = [lum(a), lum(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
};

// Resolve any CSS colour string to [r, g, b] through a canvas, on the page.
async function resolver(page) {
  return page.evaluate(() => {
    const c = document.createElement('canvas');
    c.width = c.height = 1;
    const ctx = c.getContext('2d', { willReadFrequently: true });
    window.__resolve = (css) => {
      ctx.clearRect(0, 0, 1, 1);
      ctx.fillStyle = '#000';
      ctx.fillStyle = css;
      ctx.fillRect(0, 0, 1, 1);
      const d = ctx.getImageData(0, 0, 1, 1).data;
      return [d[0], d[1], d[2]];
    };
    return true;
  });
}

// A CSS colour string to [r,g,b] on a given page (resolver() must have run there).
const rgbOn = (page, css) => page.evaluate((c) => window.__resolve(c), css);

// Composite a possibly translucent computed colour over `under` ([r,g,b]).
const over = (css, under, page) => page.evaluate(([css, under]) => {
  const c = document.createElement('canvas');
  c.width = c.height = 1;
  const ctx = c.getContext('2d', { willReadFrequently: true });
  ctx.fillStyle = `rgb(${under.join(',')})`;
  ctx.fillRect(0, 0, 1, 1);
  ctx.fillStyle = css;
  ctx.fillRect(0, 0, 1, 1);
  const d = ctx.getImageData(0, 0, 1, 1).data;
  return [d[0], d[1], d[2]];
}, [css, under]);

function check(name, got, min, detail) {
  const ok = got >= min;
  rows.push(`${ok ? 'PASS' : 'FAIL'}  ${name.padEnd(58)} ${got.toFixed(2)}:1 (>= ${min})${detail ? '  ' + detail : ''}`);
  if (!ok) failures.push(`${name}: ${got.toFixed(2)}:1 < ${min}:1 ${detail || ''}`);
}

for (const theme of ['light', 'dark']) {
  const ctx = await browser.newContext({ viewport: { width: 1000, height: 700 }, colorScheme: theme });
  await ctx.addInitScript((t) => { try { localStorage.setItem('theme', t); } catch (e) {} }, theme);
  const page = await ctx.newPage();
  await page.goto(`${BASE}/login`, { waitUntil: 'domcontentloaded' });
  await page.waitForLoadState('networkidle').catch(() => {});
  await resolver(page);
  if (MUTATE) {
    // `--a=value;--b=value`. Only for watching the gate go red. Through the
    // CSSOM: the page's CSP refuses an injected <style>, as it should.
    await page.evaluate((m) => {
      for (const d of m.split(';')) {
        const at = d.indexOf('=');
        document.documentElement.style.setProperty(d.slice(0, at).trim(), d.slice(at + 1).trim(), 'important');
      }
    }, MUTATE);
  }

  // A probe card holding one of everything, drawn with the app's own classes.
  const probe = await page.evaluate(() => {
    const host = document.createElement('div');
    host.className = 'bnb-card pad';
    host.id = 'tokens-probe';
    host.innerHTML = ['high', 'mid', 'low'].map((t) =>
      `<div><span class="bnb-conf ${t}"><span class="track"><span class="fill"></span></span><span class="val">80%</span></span>` +
      `<span class="bnb-conf-pill ${t}">80%</span></div>`).join('') +
      `<p class="bnb-conf-legend"><span class="bnb-conf-key high"><i></i>90% and up very sure</span></p>`;
    document.body.appendChild(host);
    const css = (el, prop) => getComputedStyle(el)[prop];
    const out = { card: css(host, 'backgroundColor'), tiers: {} };
    for (const t of ['high', 'mid', 'low']) {
      const bar = host.querySelector(`.bnb-conf.${t}`);
      const pill = host.querySelector(`.bnb-conf-pill.${t}`);
      out.tiers[t] = {
        fill: css(bar.querySelector('.fill'), 'backgroundColor'),
        track: css(bar.querySelector('.track'), 'backgroundColor'),
        pillBg: css(pill, 'backgroundColor'),
        pillFg: css(pill, 'color'),
        val: css(bar.querySelector('.val'), 'color'),
      };
    }
    out.legend = css(host.querySelector('.bnb-conf-legend'), 'color');
    return out;
  });
  const rgb = (css) => page.evaluate((c) => window.__resolve(c), css);
  const card = await rgb(probe.card);

  const fills = {};
  for (const t of ['high', 'mid', 'low']) {
    const v = probe.tiers[t];
    const track = await over(v.track, card, page);
    const fill = await rgb(v.fill);
    const pillBg = await over(v.pillBg, card, page);
    const pillFg = await rgb(v.pillFg);
    fills[t] = fill;
    check(`${theme} conf-${t} fill on its track (graphic, 1.4.11)`, contrast(fill, track), 3);
    check(`${theme} conf-${t} pill text on pill fill (1.4.3)`, contrast(pillFg, pillBg), 4.5);
    check(`${theme} conf-${t} value text on the card (1.4.3)`, contrast(await rgb(v.val), card), 4.5);
  }
  check(`${theme} key text on the card (1.4.3)`, contrast(await rgb(probe.legend), card), 4.5);

  // The tiers must differ from each other by more than a shade, or the scale
  // is decoration. Euclidean distance in sRGB is a blunt instrument, but it is
  // enough to tell "three colours" from "one colour three times".
  const dist = (a, b) => Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2]);
  for (const [a, b] of [['high', 'mid'], ['mid', 'low'], ['high', 'low']]) {
    const d = dist(fills[a], fills[b]);
    rows.push(`${d >= 40 ? 'PASS' : 'FAIL'}  ${`${theme} ${a} and ${b} fills are distinct`.padEnd(58)} sRGB distance ${d.toFixed(0)} (>= 40)`);
    if (d < 40) failures.push(`${theme}: conf-${a} and conf-${b} are only ${d.toFixed(0)} apart in sRGB`);
  }

  // The scale never uses the alert hue: no tier's fill is within a small hue
  // distance of --rare.
  const rare = await rgb(await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--rare')));
  for (const t of ['high', 'mid', 'low']) {
    const d = dist(fills[t], rare);
    rows.push(`${d >= 60 ? 'PASS' : 'FAIL'}  ${`${theme} conf-${t} is not the alert colour`.padEnd(58)} sRGB distance ${d.toFixed(0)} (>= 60)`);
    if (d < 60) failures.push(`${theme}: conf-${t} is within ${d.toFixed(0)} of --rare`);
  }
  // A tint of a colour over a neutral is that colour, paler. `color-mix(in
  // oklch, …)` interpolated hue, and a neutral's is 0 (light) or 250 (dark), so
  // a 16 % tint of amber over white came out at hue 9.6 (pink) and a 16 % tint of
  // moss at 24 (a brown). Read from what the browser paints: the hue of each
  // tinted surface against the hue of the token it is a tint of.
  {
    const tints = await page.evaluate(() => {
      const make = (html) => {
        const host = document.createElement('div');
        host.innerHTML = html;
        document.body.appendChild(host);
        return host.firstElementChild;
      };
      const css = (el, p) => getComputedStyle(el)[p];
      const out = [];
      for (const [kind, token] of [['success', '--moss'], ['warn', '--dawn'], ['error', '--rare'], ['info', '--info']]) {
        const t = make(`<div class="bnb-toast" data-kind="${kind}"><span class="bnb-toast__dot"></span></div>`);
        out.push({ what: `${kind} toast fill`, color: css(t, 'backgroundColor'), token });
        out.push({ what: `${kind} toast edge`, color: css(t, 'borderTopColor'), token });
      }
      const st = make('<div class="st-status warn"><span class="ico">!</span></div>');
      out.push({ what: 'status icon disc (warn)', color: css(st.querySelector('.ico'), 'backgroundColor'), token: '--dawn' });
      const ok = make('<div class="st-status"><span class="ico">&#10003;</span></div>');
      out.push({ what: 'status icon disc (ok)', color: css(ok.querySelector('.ico'), 'backgroundColor'), token: '--moss' });
      const feed = make('<div class="feed-row fresh"></div>');
      out.push({ what: 'a fresh feed row', color: css(feed, 'backgroundColor'), token: '--moss' });
      const wf = make('<div class="waveform"><span></span></div>');
      out.push({ what: 'the waveform beside a detection', color: css(wf.firstElementChild, 'backgroundColor'), token: '--moss' });
      const tok = {};
      for (const t of ['--moss', '--dawn', '--rare', '--info']) tok[t] = getComputedStyle(document.documentElement).getPropertyValue(t);
      return { tints: out, tok };
    });
    const toOklchHue = ([r, g, b]) => {
      const f = (c) => { c /= 255; return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4; };
      const [R, G, B] = [f(r), f(g), f(b)];
      const l = Math.cbrt(0.4122214708 * R + 0.5363325363 * G + 0.0514459929 * B);
      const m = Math.cbrt(0.2119034982 * R + 0.6806995451 * G + 0.1073969566 * B);
      const s = Math.cbrt(0.0883024619 * R + 0.2817188376 * G + 0.6299787005 * B);
      const a = 1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s;
      const bb = 0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s;
      return { h: ((Math.atan2(bb, a) * 180) / Math.PI + 360) % 360, c: Math.hypot(a, bb) };
    };
    const tokenHue = {};
    for (const [k, v] of Object.entries(tints.tok)) tokenHue[k] = toOklchHue(await rgb(v)).h;
    for (const t of tints.tints) {
      const under = await rgb(await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--surface')));
      const px = await over(t.color, under, page);
      const { h, c } = toOklchHue(px);
      // Below this chroma the surface it was mixed over (itself faintly blue in
      // dark) is most of the colour and a hue says nothing about the tint; a 6 %
      // wash such as the fresh feed row is that case.
      if (c < 0.015) continue;
      const d = Math.abs(((h - tokenHue[t.token] + 540) % 360) - 180);
      rows.push(`${d <= 25 ? 'PASS' : 'FAIL'}  ${`${theme} ${t.what} keeps the hue of ${t.token}`.padEnd(58)} hue ${h.toFixed(0)} vs ${tokenHue[t.token].toFixed(0)} (off by ${d.toFixed(0)}, <= 25)`);
      if (d > 25) failures.push(`${theme}: ${t.what} is hue ${h.toFixed(0)}, a tint of ${t.token} (${tokenHue[t.token].toFixed(0)}) should be within 25`);
    }
  }

  // The browser's own bar (<meta name="theme-color">) is --bg of the theme the
  // page is in. It was a warm brown in dark for as long as the dark theme has
  // been near-black, so on a phone the bar and the page were two different
  // darks. Two things are held: what the HTML says (what a browser with no
  // script, and the first paint, see), and what the page does when the
  // operator's theme and the OS's disagree (the media query follows the OS).
  const bg = await rgb(await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--bg')));
  const near = (a, b) => Math.max(...a.map((v, i) => Math.abs(v - b[i])));
  const hex = (h) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16));
  const html = await (await page.request.get(`${BASE}/today`)).text();
  const metas = [...html.matchAll(/<meta name="theme-color" content="(#[0-9a-fA-F]{6})" media="\(prefers-color-scheme: (light|dark)\)">/g)];
  const mine = metas.find((m) => m[2] === theme);
  const gap = mine ? near(hex(mine[1]), bg) : 999;
  rows.push(`${gap <= 2 ? 'PASS' : 'FAIL'}  ${`${theme} theme-color meta in the HTML is --bg`.padEnd(58)} ${mine ? mine[1] : 'missing'} vs rgb(${bg}) off by ${gap} (<= 2)`);
  if (gap > 2) failures.push(`${theme}: <meta name="theme-color"> is ${mine ? mine[1] : 'missing'}, --bg is rgb(${bg})`);
  await ctx.close();

  // The operator chose `theme`; the OS says the other. After the page's own
  // script has run, every theme-color meta must name the chosen theme's --bg.
  const other = theme === 'light' ? 'dark' : 'light';
  const ctx2 = await browser.newContext({ viewport: { width: 1000, height: 700 }, colorScheme: other });
  await ctx2.addInitScript((t) => { try { localStorage.setItem('theme', t); } catch (e) {} }, theme);
  const p2 = await ctx2.newPage();
  await p2.goto(`${BASE}/today`, { waitUntil: 'domcontentloaded' });
  await p2.waitForLoadState('networkidle').catch(() => {});
  await resolver(p2);
  const bg2 = await p2.evaluate(() => window.__resolve(getComputedStyle(document.documentElement).getPropertyValue('--bg')));
  const live = await p2.evaluate(() => [...document.querySelectorAll('meta[name="theme-color"]')].map((m) => m.getAttribute('content')));
  const worst = live.length ? Math.max(...live.map((h) => near(hex(h), bg2))) : 999;
  rows.push(`${worst <= 2 ? 'PASS' : 'FAIL'}  ${`${theme} chosen over an OS ${other}: bar follows the page`.padEnd(58)} ${live.join(' ')} vs rgb(${bg2}) off by ${worst} (<= 2)`);
  if (worst > 2) failures.push(`${theme} chosen with the OS in ${other}: theme-color metas are ${live.join(', ')}, --bg is rgb(${bg2})`);
  await ctx2.close();

  // A text field has to look like one (WCAG 1.4.11: the boundary a person needs
  // to find the control is 3:1 against what it sits on). The sign-in fields were
  // filled a grey one step off the card with a hairline edge, and read as
  // disabled, which axe does not grade. Measured on the real controls: the
  // edge, composited over the card, against the card; or, if the edge is not
  // enough, the fill against the card.
  const ctx3 = await browser.newContext({ viewport: { width: 1000, height: 700 }, colorScheme: theme });
  await ctx3.addInitScript((t) => { try { localStorage.setItem('theme', t); } catch (e) {} }, theme);
  const p3 = await ctx3.newPage();
  await p3.goto(`${BASE}/login`, { waitUntil: 'domcontentloaded' });
  await p3.waitForLoadState('networkidle').catch(() => {});
  await resolver(p3);
  for (const id of ['username', 'password']) {
    const m = await p3.evaluate((sel) => {
      const el = document.querySelector(sel);
      const cs = getComputedStyle(el);
      // The card is a gradient in dark (no background-colour of its own), so
      // its lighter end is read from the token it is drawn with.
      const card = getComputedStyle(document.documentElement).getPropertyValue('--card-top');
      return { edge: cs.borderTopColor, width: parseFloat(cs.borderTopWidth), fill: cs.backgroundColor, card };
    }, `#${id}`);
    const cardRgb = await p3.evaluate((c) => window.__resolve(c), m.card);
    const edge = await over(m.edge, cardRgb, p3);
    const fill = await over(m.fill, cardRgb, p3);
    const best = Math.max(m.width > 0 ? contrast(edge, cardRgb) : 1, contrast(fill, cardRgb));
    check(`${theme} sign-in ${id} field is findable against the card (1.4.11)`, best, 3, `edge ${m.width}px`);
  }
  await ctx3.close();

  // A placeholder is text (WCAG 1.4.3, 4.5:1). Nothing in the stylesheet set one, so each
  // engine drew its own default: 2.35:1 in WebKit light, 3.8:1 in Firefox light and 4.08:1
  // in Chromium dark, on a field that holds the instruction ("Scientific name (e.g. ...")
  // until it is typed over. Graded on the real fields of real pages, as the browser paints
  // them: `::placeholder`'s own computed colour and opacity, composited over the field's fill.
  const ctx4 = await browser.newContext({ viewport: { width: 1000, height: 700 }, colorScheme: theme });
  await ctx4.addInitScript((t) => { try { localStorage.setItem('theme', t); } catch (e) {} }, theme);
  const p4 = await ctx4.newPage();
  const seenPlaceholders = new Set();
  let graded = 0;
  for (const route of ['/station/alerts', '/station/capture', '/station/data', '/admin/images', '/dashboard']) {
    await p4.goto(`${BASE}${route}`, { waitUntil: 'domcontentloaded' });
    await p4.waitForLoadState('networkidle').catch(() => {});
    await resolver(p4);
    const fields = await p4.evaluate(() => [...document.querySelectorAll('input[placeholder], textarea[placeholder]')]
      .filter((el) => el.getBoundingClientRect().width > 0 && el.placeholder.trim() && el.type !== 'hidden')
      .map((el) => {
        const ph = getComputedStyle(el, '::placeholder');
        return {
          what: `${el.tagName.toLowerCase()}[${el.type}]${el.className ? '.' + String(el.className).trim().split(/\s+/)[0] : ''}`,
          colour: ph.color,
          opacity: parseFloat(ph.opacity),
          fill: getComputedStyle(el).backgroundColor,
          surface: getComputedStyle(document.documentElement).getPropertyValue('--surface'),
        };
      }));
    for (const f of fields) {
      const key = `${f.what}|${f.colour}|${f.opacity}|${f.fill}`;
      if (seenPlaceholders.has(key)) continue;
      seenPlaceholders.add(key);
      const under = await rgbOn(p4, f.surface);
      const fill = await over(f.fill, under, p4);
      // The placeholder's colour at its own opacity, over the fill.
      const text = await p4.evaluate(([css, bgRgb, opacity]) => {
        const c = document.createElement('canvas');
        c.width = c.height = 1;
        const ctx = c.getContext('2d', { willReadFrequently: true });
        ctx.fillStyle = `rgb(${bgRgb.join(',')})`;
        ctx.fillRect(0, 0, 1, 1);
        ctx.globalAlpha = opacity;
        ctx.fillStyle = css;
        ctx.fillRect(0, 0, 1, 1);
        const d = ctx.getImageData(0, 0, 1, 1).data;
        return [d[0], d[1], d[2]];
      }, [f.colour, fill, f.opacity]);
      check(`${theme} placeholder in ${f.what} on ${route} (1.4.3)`, contrast(text, fill), 4.5, `opacity ${f.opacity}`);
      graded++;
    }
  }
  // A gate that found no field graded nothing.
  if (graded < 5) { failures.push(`${theme}: only ${graded} placeholder style(s) found on the fixture's pages`); rows.push(`FAIL  ${theme} placeholders found: ${graded}`); }
  await ctx4.close();
}

await browser.close();
console.log(rows.join('\n'));
if (failures.length) {
  console.error(`\ntokens: ${failures.length} failure(s)\n  ${failures.join('\n  ')}`);
  process.exit(1);
}
console.log(`\ntokens: ${rows.length} checks pass`);
