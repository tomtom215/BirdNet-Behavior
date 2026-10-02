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
// Also flags duplicate element ids (see diagnose()).
//
// Run from this directory after `npm i playwright && npx playwright install chromium`.
import { chromium } from 'playwright';
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const BASE = process.env.BASE || 'http://127.0.0.1:8502';
const OUT = process.env.OUT || 'shots';
const THEMES = (process.env.THEMES || 'light,dark').split(',').filter(Boolean);
const DENSITY = process.env.DENSITY || 'regular';
const MOTION = process.env.MOTION || '';
const CONTRAST = process.env.CONTRAST || '';
const ONLY = process.env.ONLY || '';
// Explicit browser binary, for sandboxes that ship their own Chromium rather
// than the build this playwright pinned (same knob as axe.mjs / interactions.mjs).
const CHROMIUM_PATH = process.env.CHROMIUM_PATH || '';

// `touch: true` is load-bearing, not cosmetic. The phone layout is behind
// `@media (max-width: 720px) and (pointer: coarse)`, and a Playwright context
// given only a viewport reports `pointer: fine` — so the query never matched and
// every "mobile" run here rendered the *desktop* layout at phone width: bottom
// tab bar `display:none`, top nav links visible, 231px of chrome instead of
// 160px. The gate existed and was inert. `hasTouch` is what makes it real.
const VP_TABLE = {
  xl: { width: 1440, height: 900 },
  lg: { width: 1280, height: 860 },
  md: { width: 1024, height: 820 },
  sm: { width: 800, height: 900 },
  mobile: { width: 390, height: 844, touch: true },
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

// A detection that exists. The route below named a hard-coded time that the
// fixture never seeds, so the page answered "Detection not found" — with a
// 200, until that became an honest 404 — and both this sweep and axe (which
// imports ROUTES) graded the not-found page and never the detection page.
async function realDetectionPath() {
  try {
    const r = await fetch(`${BASE}/api/v2/detections?limit=1`);
    const d = (await r.json()).detections[0];
    return `/detections/detail?date=${d.date}&time=${d.time}&name=${enc(d.com_name)}`;
  } catch (e) {
    throw new Error(`qa: no detection to open at ${BASE}/api/v2/detections — is the fixture up? (${e})`);
  }
}
const DETECTION_DETAIL = await realDetectionPath();

export const ROUTES = [
  // ── The six homes of the v3 spine, by their real URLs ──────────────────
  //
  // This table used to be written in pre-spine URLs — `/heatmap`, `/weekly`,
  // `/system`, `/admin/audio`, … — which still resolve because
  // `routes::redirects` 308s them and Playwright follows redirects. The homes
  // were therefore being screenshotted under names that did not describe them
  // (`admin-audio` wrote a picture of the Station Capture tab), and the
  // coverage was a property of the redirect table rather than of this one:
  // retarget one redirect and a home would silently stop being gated with no
  // row changing.
  //
  // `crates/birdnet-web/tests/qa_routes_cover_the_navigation.rs` now fails if a
  // home or a Station tab is missing from here. The redirects have their own
  // Rust test (`routes::redirects`), so they do not need a browser to prove.
  ['dashboard', '/'],
  ['species', '/species'],
  ['species-lifelist', '/species?view=lifelist'],
  ['species-photos', '/species?view=photos'],
  ['patterns', '/patterns'],
  ['patterns-dawn', '/patterns?tab=dawn'],
  ['patterns-migration', '/patterns?tab=migration'],
  ['patterns-together', '/patterns?tab=together'],
  ['patterns-trends', '/patterns?tab=trends'],
  ['patterns-behavior', '/patterns?tab=behavior'],
  ['recordings', '/recordings'],
  ['recordings-live', '/recordings?view=live'],
  ['reports', '/reports'],
  ['reports-year', '/reports?tab=year'],
  ['reports-history', '/reports?tab=history'],
  ['reports-day', '/reports/day'],

  // ── Station: the public health tab plus the five gated management tabs ──
  ['station', '/station'],
  ['station-capture', '/station/capture'],
  ['station-alerts', '/station/alerts'],
  ['station-data', '/station/data'],
  ['station-settings', '/station/settings'],
  ['station-access', '/station/access'],

  // ── Screens that belong to no home ─────────────────────────────────────
  //
  // `/login` is the only page an unauthenticated visitor can reach on a
  // station with a password, and it was in neither this table nor any
  // redirect — so no gate had ever loaded it.
  ['login', '/login'],
  ['onboarding', '/onboarding'],
  ['detection-detail', DETECTION_DETAIL],
  ['species-detail', `/species/detail?name=${enc('European Robin')}`],
  ['detection-reviews', '/detection-reviews'],
  ['notifications', '/notifications'],
  ['quarantine', '/quarantine'],
  ['kiosk', '/kiosk'],

  // ── Admin pages that have not folded into a Station tab yet ────────────
  ['admin', '/admin'],
  ['admin-settings', '/admin/settings'],
  ['admin-system', '/admin/system'],
  ['admin-doctor', '/admin/doctor'],
  ['admin-images', '/admin/images'],
  ['admin-audit', '/admin/audit'],
  ['admin-overview', '/admin/overview'],

  ['notfound', '/this-route-does-not-exist'],
];


// Routes that are *expected* to return 404 — negative tests for the error
// page. Their own 404 status and the browser's "Failed to load resource …
// 404" console log are success, not regressions, so they are filtered out
// below (anything else on the page — overflow, other errors — still counts).
const EXPECT_404 = new Set(['notfound']);

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
    // A species photograph is optional by design. `/api/v2/species/image/
    // {sci}/file` answers 404 whenever the station has no picture for that
    // bird — Wikipedia has none on the species page, the lookup failed, an
    // admin blacklisted it, or the cache is switched off — and the avatar
    // opts into `data-hide-on-error` so the coloured banding-code tile shows
    // through. That is a designed state, not a broken asset, so it is counted
    // separately rather than reported as a page defect. Counted, not
    // discarded: `speciesImgBroken === speciesImgTotal` means the endpoint or
    // the URL is broken, and that still fails (see the floor below).
    const isSpeciesPhoto = (u) => /\/api\/v2\/species\/image\//.test(u || '');
    const brokenAll = imgs
      .filter((i) => i.complete && i.naturalWidth === 0 && (i.currentSrc || i.src))
      .map((i) => i.currentSrc || i.src);
    const speciesImgs = imgs.filter((i) => isSpeciesPhoto(i.currentSrc || i.src));
    const broken = brokenAll.filter((u) => !isSpeciesPhoto(u));
    // Two elements with one id: `getElementById` returns the first, so the
    // script that owns the second binds to the wrong element and does nothing.
    // /station/settings shipped exactly that — a heading and the Display card
    // both `id="display-prefs"` — and every Theme / Density / Motion button
    // was dead. axe no longer has a duplicate-id rule, so nothing else in the
    // suite can see it. Read after htmx has swapped the page's fragments in.
    const seen = new Map();
    document.querySelectorAll('[id]').forEach((el) => { seen.set(el.id, (seen.get(el.id) || 0) + 1); });
    const dupIds = [...seen].filter(([, c]) => c > 1).map(([id, c]) => `${id}x${c}`);
    return {
      dupIds,
      overflowX, scrollW: de.scrollWidth, clientW: de.clientWidth,
      stuck: [...new Set(stuck)].slice(0, 8),
      imgTotal: imgs.length, imgBroken: [...new Set(broken)].slice(0, 12),
      speciesImgTotal: speciesImgs.length,
      speciesImgBroken: [...new Set(brokenAll.filter(isSpeciesPhoto))].length,
      title: document.title,
    };
  });
}

async function main() {
  fs.mkdirSync(OUT, { recursive: true });
  const browser = await chromium.launch(CHROMIUM_PATH ? { executablePath: CHROMIUM_PATH } : {});
  const report = {};
  let n = 0;

  for (const vp of VPS) {
    for (const theme of THEMES) {
      const context = await browser.newContext({
        viewport: { width: vp.width, height: vp.height },
        hasTouch: Boolean(vp.touch),
        isMobile: Boolean(vp.touch),
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
          // For an expected-404 route, drop the route's own 404 (status + the
          // browser's 404 resource log); keep every other signal.
          const expect404 = EXPECT_404.has(name);
          const consoleErrsF = expect404
            ? consoleErrs.filter((m) => !/status of 404/i.test(m))
            : consoleErrs;
          // Optional species photos: drop their 404s from the HTTP list, and
          // drop exactly as many 404 console lines as they produced. The
          // console message carries no URL, so it is matched by count rather
          // than by text — dropping every 404 line would hide a genuinely
          // missing asset on the same page.
          const speciesPhoto404 = bad.filter(
            (b) => b.startsWith('404 ') && b.includes('/api/v2/species/image/'),
          ).length;
          let toDrop = speciesPhoto404;
          const consoleErrsS = consoleErrsF.filter((m) => {
            if (toDrop > 0 && /status of 404/i.test(m)) { toDrop -= 1; return false; }
            return true;
          });
          const badF = bad.filter(
            (b) =>
              !b.includes('favicon') &&
              !(b.startsWith('404 ') && b.includes('/api/v2/species/image/')) &&
              !(expect404 && b.startsWith('404 ') && b.includes(route)),
          );
          report[key] = {
            route, status: resp ? resp.status() : null,
            consoleErrs: consoleErrsS, pageErrs,
            failed: failed.filter((f) => !f.includes('favicon')),
            bad: badF,
            ...diag,
          };
          const flag = (report[key].overflowX ? 'OVERFLOW ' : '') + (report[key].consoleErrs.length ? `ERR(${report[key].consoleErrs.length}) ` : '') + (report[key].imgBroken.length ? `IMG(${report[key].imgBroken.length}) ` : '') + (report[key].stuck.length ? 'STUCK ' : '') + (report[key].dupIds.length ? `DUPID(${report[key].dupIds.length}) ` : '');
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

  // A page where EVERY species photo failed is not "these birds have no
  // picture" — it is a broken route or a malformed URL, and that is exactly
  // what this sweep exists to catch. One or two missing is ordinary.
  const allPhotosFailed = (v) =>
    (v.speciesImgTotal || 0) > 0 && v.speciesImgBroken === v.speciesImgTotal;
  const probs = Object.entries(report).filter(([, v]) =>
    v.error || v.overflowX || (v.consoleErrs && v.consoleErrs.length) ||
    (v.pageErrs && v.pageErrs.length) || (v.imgBroken && v.imgBroken.length) ||
    (v.stuck && v.stuck.length) || (v.bad && v.bad.length) || (v.dupIds && v.dupIds.length) || allPhotosFailed(v));
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
    if (allPhotosFailed(v)) parts.push(`allSpeciesPhotosFailed(${v.speciesImgBroken}/${v.speciesImgTotal})`);
    if (v.stuck?.length) parts.push(`stuck=${JSON.stringify(v.stuck)}`);
    if (v.dupIds?.length) parts.push(`duplicateIds=${JSON.stringify(v.dupIds)}`);
    console.log(`  ${k}: ${parts.join(' | ')}`);
  }

  // STRICT mode (CI gate): any structural regression — horizontal overflow,
  // console/page errors, >=400 responses, broken images, stuck loaders — fails
  // the run. Local exploratory runs (no STRICT) always exit 0 as before.
  if (process.env.STRICT && probs.length) {
    console.error(`\nSTRICT: failing — ${probs.length} page/state(s) with issues.`);
    process.exitCode = 1;
  }
}

// Only auto-run when executed directly (`node qa.mjs`); when another script
// imports { ROUTES } from this module it must not launch a capture run.
const invokedDirectly =
  process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href;
if (invokedDirectly) {
  main().catch((e) => {
    console.error(e);
    process.exit(1);
  });
}
