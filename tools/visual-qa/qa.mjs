// Comprehensive visual-QA capture + per-page diagnostics for birdnet-web.
//
// Captures full-page screenshots across themes x viewports x routes and
// records, per page: console errors, uncaught exceptions, failed requests,
// >=400 responses, horizontal overflow, stuck "loading..." text, and broken
// images. Writes <out>/report.json and prints a summary of pages with issues.
//
// Env:
//   BASE      base url (default http://127.0.0.1:8502)
//   OUT       output dir (default shots)
//   THEMES    csv of light,dark            (default light,dark)
//   VPS       csv of named viewports       (default desktop,mobile)
//             known: xl=1440 lg=1280 md=1024 sm=800 mobile=390
//   DENSITY   compact|comfy|regular        (default regular)
//   MOTION    reduced|"" (default "")
//   CONTRAST  high|"" (default "")
//   ONLY      substring filter on route name
//
// Run from this directory after `npm i playwright && npx playwright install chromium`.
import { chromium } from 'playwright';
import fs from 'node:fs';
import path from 'node:path';

const BASE = process.env.BASE || 'http://127.0.0.1:8502';
const OUT = process.env.OUT || 'shots';
const THEMES = (process.env.THEMES || 'light,dark').split(',').filter(Boolean);
const DENSITY = process.env.DENSITY || 'regular';
const MOTION = process.env.MOTION || '';
const CONTRAST = process.env.CONTRAST || '';
const ONLY = process.env.ONLY || '';

const VP_TABLE = {
  xl: { width: 1440, height: 900 },
  lg: { width: 1280, height: 860 },
  md: { width: 1024, height: 820 },
  sm: { width: 800, height: 900 },
  mobile: { width: 390, height: 844 },
  desktop: { width: 1440, height: 900 },
};
const VPS = (process.env.VPS || 'desktop,mobile')
  .split(',')
  .filter(Boolean)
  .map((n) => ({ name: n, ...VP_TABLE[n] }));

// today (UTC) — matches the server's system clock so the hero detection resolves.
const T = new Date();
const TODAY = `${T.getUTCFullYear()}-${String(T.getUTCMonth() + 1).padStart(2, '0')}-${String(T.getUTCDate()).padStart(2, '0')}`;
const enc = encodeURIComponent;

export const ROUTES = [
  ['dashboard', '/'],
  ['onboarding', '/onboarding'],
  ['today', '/today'],
  ['species', '/species'],
  ['species-detail', `/species/detail?name=${enc('European Robin')}`],
  ['detection-detail', `/detections/detail?date=${TODAY}&time=05:14:08&name=${enc('Eurasian Magpie')}`],
  ['heatmap', '/heatmap'],
  ['migration', '/migration'],
  ['correlation', '/correlation'],
  ['analytics', '/analytics'],
  ['dawn-chorus', '/analytics/dawn-chorus'],
  ['timeseries', '/timeseries'],
  ['life-list', '/life-list'],
  ['recordings', '/recordings'],
  ['gallery', '/gallery'],
  ['weekly', '/weekly'],
  ['year-in-review', '/year-in-review'],
  ['history', '/history'],
  ['notifications', '/notifications'],
  ['quarantine', '/quarantine'],
  ['system', '/system'],
  ['kiosk', '/kiosk'],
  ['live', '/live'],
  ['admin', '/admin'],
  ['admin-settings', '/admin/settings'],
  ['admin-audio', '/admin/audio'],
  ['admin-backups', '/admin/backups'],
  ['admin-species', '/admin/species'],
  ['admin-quality', '/admin/quality'],
  ['admin-rules', '/admin/rules'],
  ['admin-notifications', '/admin/notifications'],
  ['admin-system', '/admin/system'],
  ['admin-doctor', '/admin/doctor'],
  ['admin-images', '/admin/images'],
  ['admin-migrate', '/admin/migrate'],
  ['notfound', '/this-route-does-not-exist'],
];

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function diagnose(page) {
  return page.evaluate(() => {
    const de = document.documentElement;
    const overflowX = de.scrollWidth > de.clientWidth + 2;
    const stuck = [];
    const re = /loading[….]/i;
    document.querySelectorAll('*').forEach((el) => {
      if (el.children.length === 0 && el.textContent && re.test(el.textContent.trim()) && el.offsetParent !== null) {
        stuck.push(el.textContent.trim().slice(0, 40));
      }
    });
    const imgs = [...document.querySelectorAll('img')];
    const broken = imgs.filter((i) => i.complete && i.naturalWidth === 0 && (i.currentSrc || i.src)).map((i) => i.currentSrc || i.src);
    return {
      overflowX, scrollW: de.scrollWidth, clientW: de.clientWidth,
      stuck: [...new Set(stuck)].slice(0, 8),
      imgTotal: imgs.length, imgBroken: [...new Set(broken)].slice(0, 12),
      title: document.title,
    };
  });
}

async function main() {
  fs.mkdirSync(OUT, { recursive: true });
  const browser = await chromium.launch();
  const report = {};
  let n = 0;

  for (const vp of VPS) {
    for (const theme of THEMES) {
      const context = await browser.newContext({
        viewport: { width: vp.width, height: vp.height },
        deviceScaleFactor: 1,
        colorScheme: theme === 'dark' ? 'dark' : 'light',
        reducedMotion: MOTION === 'reduced' ? 'reduce' : 'no-preference',
        forcedColors: CONTRAST === 'high' ? 'active' : 'none',
      });
      await context.addInitScript(
        ([t, d, m, c]) => {
          try {
            localStorage.setItem('theme', t);
            localStorage.setItem('bnb-density', d);
            if (m) localStorage.setItem('bnb-motion', m);
            if (c) localStorage.setItem('bnb-contrast', c);
          } catch (e) {}
        },
        [theme, DENSITY, MOTION, CONTRAST]
      );
      const page = await context.newPage();

      for (const [name, route] of ROUTES) {
        if (ONLY && !name.includes(ONLY)) continue;
        const key = `${name}__${theme}__${vp.name}`;
        const consoleErrs = [];
        const pageErrs = [];
        const failed = [];
        const bad = [];
        const onConsole = (m) => { if (m.type() === 'error') consoleErrs.push(m.text().slice(0, 200)); };
        const onPageErr = (e) => pageErrs.push(String(e).slice(0, 200));
        const onFailed = (r) => failed.push(`${r.url()} :: ${r.failure()?.errorText}`);
        const onResp = (r) => { if (r.status() >= 400) bad.push(`${r.status()} ${r.url()}`); };
        page.on('console', onConsole);
        page.on('pageerror', onPageErr);
        page.on('requestfailed', onFailed);
        page.on('response', onResp);
        try {
          const resp = await page.goto(BASE + route, { waitUntil: 'domcontentloaded', timeout: 25000 });
          await page.waitForLoadState('networkidle', { timeout: 9000 }).catch(() => {});
          await sleep(1400);
          const diag = await diagnose(page);
          await page.screenshot({ path: path.join(OUT, `${key}.png`), fullPage: true });
          report[key] = {
            route, status: resp ? resp.status() : null,
            consoleErrs, pageErrs,
            failed: failed.filter((f) => !f.includes('favicon')),
            bad: bad.filter((b) => !b.includes('favicon')),
            ...diag,
          };
          const flag = (report[key].overflowX ? 'OVERFLOW ' : '') + (consoleErrs.length ? `ERR(${consoleErrs.length}) ` : '') + (report[key].imgBroken.length ? `IMG(${report[key].imgBroken.length}) ` : '') + (report[key].stuck.length ? 'STUCK ' : '');
          process.stdout.write(`${flag ? '! ' : '. '}${key} ${flag}\n`);
          n++;
        } catch (err) {
          report[key] = { route, error: String(err).slice(0, 200) };
          process.stdout.write(`x ${key}: ${err.message}\n`);
        }
        page.off('console', onConsole);
        page.off('pageerror', onPageErr);
        page.off('requestfailed', onFailed);
        page.off('response', onResp);
      }
      await context.close();
    }
  }
  await browser.close();
  fs.writeFileSync(path.join(OUT, 'report.json'), JSON.stringify(report, null, 2));

  const probs = Object.entries(report).filter(([, v]) =>
    v.error || v.overflowX || (v.consoleErrs && v.consoleErrs.length) ||
    (v.pageErrs && v.pageErrs.length) || (v.imgBroken && v.imgBroken.length) ||
    (v.stuck && v.stuck.length) || (v.bad && v.bad.length));
  console.log(`\nCaptured ${n} screenshots into ${OUT}/`);
  console.log(`\n=== ${probs.length} pages with issues ===`);
  for (const [k, v] of probs) {
    const parts = [];
    if (v.error) parts.push(`error=${v.error}`);
    if (v.overflowX) parts.push(`overflowX(${v.scrollW}>${v.clientW})`);
    if (v.consoleErrs?.length) parts.push(`console=${JSON.stringify(v.consoleErrs)}`);
    if (v.pageErrs?.length) parts.push(`pageerr=${JSON.stringify(v.pageErrs)}`);
    if (v.bad?.length) parts.push(`http=${JSON.stringify(v.bad)}`);
    if (v.imgBroken?.length) parts.push(`brokenImg=${JSON.stringify(v.imgBroken)}`);
    if (v.stuck?.length) parts.push(`stuck=${JSON.stringify(v.stuck)}`);
    console.log(`  ${k}: ${parts.join(' | ')}`);
  }
}

main().catch((e) => { console.error(e); process.exit(1); });
